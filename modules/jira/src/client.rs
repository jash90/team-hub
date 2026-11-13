//! Talking to Jira's REST API: v3 on Jira Cloud, v2 on Jira Server and Data Center, and the
//! agile API for boards and sprints. Everything here blocks on the network: call it from a
//! plain thread or `spawn_blocking`, never from the worker pool.

use std::time::Duration;

use base64::Engine;
use hub_app::app::get_setting;
use hub_app::secrets::get_secret;
use hub_app::{ApiError, ApiResult};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::remote::{self, Fields, RemoteComment, RemoteHistory, RemoteSprint, RemoteStatus, RemoteType, RemoteUser, RemoteVersion, RemoteWorklog};

pub const BASE_URL_SETTING: &str = "jira:base_url";
pub const EMAIL_SETTING: &str = "jira:email";
pub const TOKEN_SECRET: &str = "jira.token";

pub const PAGE_SIZE: usize = 50;
const TRIES: u32 = 4;
const BACKOFF: Duration = Duration::from_millis(300);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ANSWER_BYTES: u64 = 128 * 1024 * 1024;
/// More pages than any listing has; ends the loop should a server page in circles.
const MAX_PAGES: usize = 2000;

pub const REJECTED: &str = "Jira rejected the token";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub base_url: String,
    pub email: String,
    pub token: Option<String>,
}

pub fn load_settings(connection: &Connection, key: &[u8; 32]) -> ApiResult<Settings> {
    Ok(Settings {
        base_url: get_setting::<String>(connection, BASE_URL_SETTING)?.unwrap_or_default(),
        email: get_setting::<String>(connection, EMAIL_SETTING)?.unwrap_or_default(),
        token: get_secret(connection, key, TOKEN_SECRET)?,
    })
}

fn host_of(authority: &str) -> &str {
    match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    }
}

fn is_loopback(base_url: &str) -> bool {
    let authority = base_url.split_once("://").map_or("", |(_, rest)| rest).split('/').next().unwrap_or("");

    matches!(host_of(authority), "127.0.0.1" | "localhost")
}

/// Scheme, host and port: what decides which server a token is sent to.
pub fn origin(base_url: &str) -> String {
    let (scheme, rest) = base_url.split_once("://").unwrap_or(("", base_url));

    format!("{}://{}", scheme.to_lowercase(), rest.split('/').next().unwrap_or("").to_lowercase())
}

/// The address without a trailing slash. Only `https://` is accepted, so the token never
/// crosses the network in the clear; plain `http://` is for this machine only.
pub fn clean_base_url(text: &str) -> ApiResult<String> {
    let url = text.trim().trim_end_matches('/');
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(ApiError::bad_request("The Jira address must start with https://"));
    };
    let authority = rest.split('/').next().unwrap_or("");
    let malformed = authority.is_empty()
        || authority.contains('@')
        || url.contains(|character: char| character.is_whitespace() || character.is_control() || matches!(character, '?' | '#'));

    if malformed {
        return Err(ApiError::bad_request("That does not look like a Jira address, e.g. https://example.atlassian.net"));
    }

    match scheme.to_lowercase().as_str() {
        "https" => Ok(url.to_string()),
        "http" if is_loopback(url) => Ok(url.to_string()),
        _ => Err(ApiError::bad_request("The Jira address must start with https://")),
    }
}

/// `Basic` with the e-mail for Jira Cloud, `Bearer` for a personal access token.
pub fn authorization(email: &str, token: &str) -> String {
    if email.trim().is_empty() {
        format!("Bearer {token}")
    } else {
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{}:{token}", email.trim())))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerInfo {
    pub cloud: bool,
    pub version: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteProject {
    pub key: String,
    pub name: String,
}

/// A project's configuration: what an import creates before the first issue.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteProjectDetail {
    pub key: String,
    pub name: String,
    pub description: String,
    pub lead: Option<RemoteUser>,
    pub types: Vec<RemoteType>,
    pub components: Vec<(String, String)>,
    pub versions: Vec<RemoteVersion>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteBoard {
    pub id: String,
    pub name: String,
    pub scrum: bool,
}

/// Where a search continues: a token on Cloud, an offset on Server.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchCursor {
    pub token: Option<String>,
    pub start: usize,
    pub done: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SearchPage {
    pub issues: Vec<Value>,
    /// How many issues the search matches, where the server says.
    pub total: Option<i64>,
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    authorization: String,
    pub cloud: bool,
}

impl Client {
    pub fn new(settings: &Settings) -> ApiResult<Self> {
        let token = settings.token.as_deref().filter(|token| !token.is_empty());
        let (false, Some(token)) = (settings.base_url.is_empty(), token) else {
            return Err(ApiError::conflict("Set the Jira address and token first"));
        };
        let base = clean_base_url(&settings.base_url)?;
        let mut config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(base.starts_with("https://"))
            .timeout_global(Some(TIMEOUT))
            .user_agent("team-hub");

        if is_loopback(&base) {
            config = config.proxy(None);
        }

        // An e-mail means Jira Cloud; `detect` settles it by asking the server.
        Ok(Self {
            agent: config.build().new_agent(),
            authorization: authorization(&settings.email, token),
            cloud: !settings.email.trim().is_empty(),
            base,
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn api(&self) -> &'static str {
        if self.cloud {
            "/rest/api/3"
        } else {
            "/rest/api/2"
        }
    }

    /// One request, answered with JSON. 429 and 5xx are tried again after a pause (the one
    /// Jira asks for, when it does); a 401 is final.
    fn request(&self, path: &str, query: &[(&str, String)], body: Option<&Value>) -> ApiResult<Value> {
        let url = format!("{}{path}", self.base);
        let mut pause = BACKOFF;
        let mut last_status = 0;

        for attempt in 0..TRIES {
            if attempt > 0 {
                std::thread::sleep(pause);
                pause = BACKOFF * 2u32.pow(attempt);
            }

            let sent = match body {
                Some(body) => self.agent.post(&url).header("Authorization", &self.authorization).header("Accept", "application/json").send_json(body),
                None => {
                    let mut request = self.agent.get(&url).header("Authorization", &self.authorization).header("Accept", "application/json");

                    for (name, value) in query {
                        request = request.query(*name, value);
                    }

                    request.call()
                }
            };
            let mut response = sent.map_err(|error| ApiError::upstream(format!("Could not reach Jira: {error}")))?;
            let status = response.status().as_u16();

            match status {
                200..=299 => {
                    return response
                        .body_mut()
                        .with_config()
                        .limit(MAX_ANSWER_BYTES)
                        .read_json::<Value>()
                        .map_err(|error| ApiError::upstream(format!("Jira sent an answer that is not JSON ({error}); check the address")));
                }
                401 => return Err(ApiError::upstream(REJECTED)),
                403 => return Err(ApiError::upstream("Jira refused access; the token's account may not be allowed to see this")),
                404 => return Err(ApiError::upstream("Jira has nothing at that address; check the address and the project key")),
                429 | 500..=599 => {
                    last_status = status;

                    let asked = response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<u64>().ok());

                    if let Some(seconds) = asked {
                        pause = Duration::from_secs(seconds).min(MAX_RETRY_AFTER);
                    }
                }
                400 => {
                    let answer = response.body_mut().with_config().limit(64 * 1024).read_json::<Value>().unwrap_or(Value::Null);
                    let message = items(&answer["errorMessages"]).first().and_then(Value::as_str).unwrap_or("the request was not understood");

                    return Err(ApiError::upstream(format!("Jira answered: {message}")));
                }
                other => return Err(ApiError::upstream(format!("Jira answered with status {other}"))),
            }
        }

        Err(ApiError::upstream(format!("Jira kept answering with status {last_status}; try again later")))
    }

    fn get(&self, path: &str, query: &[(&str, String)]) -> ApiResult<Value> {
        self.request(path, query, None)
    }

    /// Walks a listing paged with `startAt` and `maxResults`, whose entries are under `list`.
    fn paged(&self, path: &str, query: &[(&str, String)], list: &str, mut each: impl FnMut(&Value)) -> ApiResult<()> {
        let mut start = 0;

        for _ in 0..MAX_PAGES {
            let mut query = query.to_vec();
            query.push(("startAt", start.to_string()));
            query.push(("maxResults", PAGE_SIZE.to_string()));

            let answer = self.get(path, &query)?;
            // Some listings (people) are a bare array, paged all the same.
            let entries = match &answer {
                Value::Array(entries) => entries.as_slice(),
                other => items(&other[list]),
            };

            entries.iter().for_each(&mut each);
            start += entries.len();

            let total = answer["total"].as_u64().map(|total| total as usize);
            let last = answer["isLast"].as_bool().unwrap_or(false) || total.is_some_and(|total| start >= total);

            // A bare array says nothing about what follows: only an empty page ends it.
            let short = !answer.is_array() && total.is_none() && answer["isLast"].is_null() && entries.len() < PAGE_SIZE;

            if entries.is_empty() || last || short {
                return Ok(());
            }
        }

        Err(ApiError::upstream("Jira keeps sending more pages than a listing can have"))
    }

    /// Asks the server what it is, which also proves the address and the token: Cloud speaks
    /// REST v3 with ADF, Server and Data Center v2 with wiki markup.
    pub fn detect(&mut self) -> ApiResult<ServerInfo> {
        let info = self.get("/rest/api/2/serverInfo", &[])?;
        let cloud = info["deploymentType"].as_str().is_some_and(|kind| kind.eq_ignore_ascii_case("cloud"));

        self.cloud = cloud;
        // serverInfo answers anonymous callers too; this one needs the token to be right.
        self.get(&format!("{}/myself", self.api()), &[])?;

        Ok(ServerInfo {
            cloud,
            version: info["version"].as_str().unwrap_or("").to_string(),
            title: info["serverTitle"].as_str().unwrap_or("Jira").to_string(),
        })
    }

    pub fn projects(&self) -> ApiResult<Vec<RemoteProject>> {
        let mut projects = Vec::new();
        let mut add = |project: &Value| {
            if let Some(key) = project["key"].as_str() {
                projects.push(RemoteProject { key: key.to_string(), name: project["name"].as_str().unwrap_or(key).to_string() });
            }
        };

        if self.cloud {
            self.paged("/rest/api/3/project/search", &[("orderBy", "key".to_string())], "values", &mut add)?;
        } else {
            items(&self.get("/rest/api/2/project", &[])?).iter().for_each(&mut add);
        }

        projects.sort_by(|left, right| left.key.cmp(&right.key));

        Ok(projects)
    }

    /// How many issues a search matches; none when the server will not say.
    pub fn count(&self, jql: &str) -> Option<i64> {
        if self.cloud {
            self.request("/rest/api/3/search/approximate-count", &[], Some(&json!({ "jql": jql }))).ok()?["count"].as_i64()
        } else {
            self.get("/rest/api/2/search", &[("jql", jql.to_string()), ("maxResults", "0".to_string())]).ok()?["total"].as_i64()
        }
    }

    pub fn project(&self, key: &str) -> ApiResult<RemoteProjectDetail> {
        let project = self.get(&format!("{}/project/{key}", self.api()), &[])?;

        Ok(RemoteProjectDetail {
            key: project["key"].as_str().unwrap_or(key).to_string(),
            name: project["name"].as_str().unwrap_or(key).to_string(),
            description: match &project["description"] {
                Value::String(text) => text.clone(),
                _ => String::new(),
            },
            lead: remote::user(&project["lead"]),
            types: items(&project["issueTypes"]).iter().filter_map(remote::issue_type).collect(),
            components: items(&project["components"])
                .iter()
                .filter_map(|component| Some((text_of(&component["name"])?, component["description"].as_str().unwrap_or("").to_string())))
                .collect(),
            versions: items(&project["versions"]).iter().filter_map(remote::version).collect(),
        })
    }

    /// The statuses the project's workflows use, each once, in the order Jira lists them.
    pub fn statuses(&self, key: &str) -> ApiResult<Vec<RemoteStatus>> {
        let by_type = self.get(&format!("{}/project/{key}/statuses", self.api()), &[])?;
        let mut statuses: Vec<RemoteStatus> = Vec::new();

        for status in items(&by_type).iter().flat_map(|issue_type| items(&issue_type["statuses"])).filter_map(remote::status) {
            if !statuses.iter().any(|known| known.name.eq_ignore_ascii_case(&status.name)) {
                statuses.push(status);
            }
        }

        Ok(statuses)
    }

    /// Priority names, highest first.
    pub fn priorities(&self) -> ApiResult<Vec<String>> {
        Ok(items(&self.get(&format!("{}/priority", self.api()), &[])?).iter().filter_map(|priority| text_of(&priority["name"])).collect())
    }

    pub fn fields(&self) -> ApiResult<Fields> {
        Ok(Fields::discover(&self.get("/rest/api/2/field", &[])?))
    }

    /// The people who can be assigned issues of the project: the best list of "who works here"
    /// Jira offers without reading every issue.
    pub fn people(&self, key: &str) -> ApiResult<Vec<RemoteUser>> {
        let mut people: Vec<RemoteUser> = Vec::new();

        self.paged(&format!("{}/user/assignable/search", self.api()), &[("project", key.to_string())], "values", |person| {
            if let Some(person) = remote::user(person).filter(|person| !people.iter().any(|known| known.account == person.account)) {
                people.push(person);
            }
        })?;

        Ok(people)
    }

    pub fn boards(&self, key: &str) -> ApiResult<Vec<RemoteBoard>> {
        let mut boards = Vec::new();

        self.paged("/rest/agile/1.0/board", &[("projectKeyOrId", key.to_string())], "values", |board| {
            if let Some(id) = text_of(&board["id"]) {
                boards.push(RemoteBoard { id, name: board["name"].as_str().unwrap_or("").to_string(), scrum: board["type"] == "scrum" });
            }
        })?;

        Ok(boards)
    }

    pub fn sprints(&self, board_id: &str) -> ApiResult<Vec<RemoteSprint>> {
        let mut sprints = Vec::new();

        self.paged(&format!("/rest/agile/1.0/board/{board_id}/sprint"), &[], "values", |sprint| sprints.extend(remote::sprint(sprint)))?;

        Ok(sprints)
    }

    /// The board's columns from left to right, each with the ids of its statuses.
    pub fn columns(&self, board_id: &str) -> ApiResult<Vec<Vec<String>>> {
        let configuration = self.get(&format!("/rest/agile/1.0/board/{board_id}/configuration"), &[])?;

        Ok(items(&configuration["columnConfig"]["columns"])
            .iter()
            .map(|column| items(&column["statuses"]).iter().filter_map(|status| text_of(&status["id"])).collect())
            .collect())
    }

    /// The next page of a search, with every field the import reads and the changelog.
    /// Cloud pages with `nextPageToken` (`/search/jql`), Server with `startAt` (`/search`).
    pub fn search(&self, jql: &str, fields: &Fields, cursor: &mut SearchCursor) -> ApiResult<SearchPage> {
        let mut wanted = "summary,description,issuetype,status,priority,assignee,reporter,labels,components,fixVersions,duedate,timeoriginalestimate,parent,created,updated,resolutiondate,comment,worklog,issuelinks,attachment".to_string();

        for id in fields.points.iter().chain(&fields.sprint).chain(&fields.epic_link) {
            wanted.push(',');
            wanted.push_str(id);
        }

        let mut query =
            vec![("jql", jql.to_string()), ("maxResults", PAGE_SIZE.to_string()), ("fields", wanted), ("expand", "changelog".to_string())];

        let answer = if self.cloud {
            query.extend(cursor.token.clone().map(|token| ("nextPageToken", token)));

            self.get("/rest/api/3/search/jql", &query)?
        } else {
            query.push(("startAt", cursor.start.to_string()));

            self.get("/rest/api/2/search", &query)?
        };
        let issues = items(&answer["issues"]).to_vec();
        let total = answer["total"].as_i64();

        cursor.start += issues.len();
        cursor.token = answer["nextPageToken"].as_str().map(String::from);
        cursor.done = issues.is_empty() || if self.cloud { cursor.token.is_none() } else { total.is_none_or(|total| cursor.start as i64 >= total) };

        Ok(SearchPage { issues, total })
    }

    /// All comments of an issue, for when the search result carried only the first ones.
    pub fn comments(&self, key: &str) -> ApiResult<Vec<RemoteComment>> {
        let mut comments = Vec::new();

        self.paged(&format!("{}/issue/{key}/comment", self.api()), &[], "comments", |comment| comments.extend(remote::comment(comment)))?;

        Ok(comments)
    }

    pub fn worklogs(&self, key: &str) -> ApiResult<Vec<RemoteWorklog>> {
        let mut worklogs = Vec::new();

        self.paged(&format!("{}/issue/{key}/worklog", self.api()), &[], "worklogs", |worklog| worklogs.extend(remote::worklog(worklog)))?;

        Ok(worklogs)
    }

    /// The whole changelog of an issue (Cloud cuts the one in search results at 100 entries).
    pub fn changelog(&self, key: &str) -> ApiResult<Vec<RemoteHistory>> {
        let mut history = Vec::new();

        self.paged(&format!("{}/issue/{key}/changelog", self.api()), &[], "values", |entry| history.extend(remote::history(entry)))?;
        history.sort_by(|left: &RemoteHistory, right| left.created.cmp(&right.created).then(left.id.cmp(&right.id)));

        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_addresses_are_accepted_except_on_this_machine() {
        assert_eq!(clean_base_url(" https://example.atlassian.net/ ").unwrap(), "https://example.atlassian.net");
        assert_eq!(clean_base_url("https://jira.example.com:8443/jira").unwrap(), "https://jira.example.com:8443/jira");
        assert_eq!(clean_base_url("http://127.0.0.1:8080/jira").unwrap(), "http://127.0.0.1:8080/jira");
        assert_eq!(clean_base_url("http://localhost").unwrap(), "http://localhost");

        for refused in [
            "http://jira.example.com",
            "http://127.0.0.1.example.com",
            "http://localhost.example.com/jira",
            "http://user@127.0.0.1",
            "ftp://example.com",
            "example.atlassian.net",
            "https://",
            "https://exa mple.com",
            "https://example.com/jira?x=1",
            "https://user:secret@example.com",
            "",
        ] {
            assert!(clean_base_url(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_origin_is_what_a_token_is_bound_to() {
        assert_eq!(origin("https://Example.atlassian.net/jira"), "https://example.atlassian.net");
        assert_ne!(origin("https://example.atlassian.net"), origin("https://example.atlassian.net:8443"));
        assert_ne!(origin("http://localhost:8090"), origin("https://localhost:8090"));
    }

    #[test]
    fn the_authorization_header_depends_on_the_e_mail() {
        assert_eq!(authorization("", "pat-123"), "Bearer pat-123");
        assert_eq!(authorization(" anna@example.com ", "api-token"), "Basic YW5uYUBleGFtcGxlLmNvbTphcGktdG9rZW4=");
    }
}
