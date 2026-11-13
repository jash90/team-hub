//! A stand-in for Jira on a port of this machine, in both flavours: Jira Cloud (REST v3,
//! ADF, `nextPageToken`, account ids) and Jira Server (REST v2, wiki markup, `startAt`,
//! user names, sprints as text). The tests use it, and `examples/fake_jira.rs` serves it
//! with a realistic project for trying the import by hand.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{json, Value};

pub const POINTS_FIELD: &str = "customfield_10016";
pub const SPRINT_FIELD: &str = "customfield_10020";
pub const EPIC_LINK_FIELD: &str = "customfield_10014";

/// A text in both of Jira's formats; the fake serves the one its flavour stores.
#[derive(Clone, Debug, Default)]
pub struct Text {
    pub adf: Value,
    pub wiki: String,
}

impl Text {
    /// Plain paragraphs, one per line.
    pub fn plain(text: &str) -> Self {
        let paragraphs: Vec<Value> = text
            .split('\n')
            .filter(|line| !line.is_empty())
            .map(|line| json!({ "type": "paragraph", "content": [{ "type": "text", "text": line }] }))
            .collect();

        Self { adf: json!({ "type": "doc", "version": 1, "content": paragraphs }), wiki: text.replace('\n', "\n\n") }
    }

    pub fn new(adf: Value, wiki: &str) -> Self {
        Self { adf, wiki: wiki.to_string() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FakeUser {
    /// The account id on Cloud, the user name on Server.
    pub account: String,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FakeSprint {
    pub id: i64,
    pub board: i64,
    pub name: String,
    /// `closed`, `active` or `future`.
    pub state: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub complete: Option<String>,
    pub goal: String,
}

#[derive(Clone, Debug, Default)]
pub struct FakeVersion {
    pub name: String,
    pub released: bool,
    pub archived: bool,
    pub start: Option<String>,
    pub release: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FakeBoard {
    pub id: i64,
    pub name: String,
    pub scrum: bool,
    /// Status names per column, left to right.
    pub columns: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Default)]
pub struct FakeProject {
    pub key: String,
    pub name: String,
    pub description: String,
    pub lead: Option<String>,
    /// Name, whether it is a subtask type, whether it is the epic type.
    pub types: Vec<(String, bool, bool)>,
    /// Name and status category key (`new`, `indeterminate`, `done`), in the order Jira lists them.
    pub statuses: Vec<(String, String)>,
    pub components: Vec<String>,
    pub versions: Vec<FakeVersion>,
    pub boards: Vec<FakeBoard>,
    pub sprints: Vec<FakeSprint>,
}

#[derive(Clone, Debug, Default)]
pub struct FakeComment {
    pub id: i64,
    pub author: String,
    pub body: Text,
    pub created: String,
    pub updated: String,
}

#[derive(Clone, Debug, Default)]
pub struct FakeWorklog {
    pub id: i64,
    pub author: String,
    pub seconds: i64,
    pub started: String,
    pub comment: Option<Text>,
}

/// One changelog item: field, from (id), fromString, to (id), toString.
pub type FakeChange = (String, Option<String>, Option<String>, Option<String>, Option<String>);

#[derive(Clone, Debug, Default)]
pub struct FakeHistory {
    pub id: i64,
    pub author: String,
    pub created: String,
    pub items: Vec<FakeChange>,
}

#[derive(Clone, Debug, Default)]
pub struct FakeIssue {
    pub key: String,
    pub kind: String,
    pub status: String,
    pub priority: Option<String>,
    pub summary: String,
    pub description: Option<Text>,
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub labels: Vec<String>,
    pub components: Vec<String>,
    pub versions: Vec<String>,
    /// Ids of every sprint the issue has been in, oldest first.
    pub sprints: Vec<i64>,
    pub points: Option<f64>,
    pub due: Option<String>,
    pub estimate: Option<i64>,
    pub parent: Option<String>,
    pub created: String,
    pub updated: String,
    pub resolved: Option<String>,
    pub comments: Vec<FakeComment>,
    pub worklogs: Vec<FakeWorklog>,
    /// Link type name, whether this issue is the outward side, the other issue's key.
    pub links: Vec<(String, bool, String)>,
    /// File name and size.
    pub attachments: Vec<(String, i64)>,
    pub history: Vec<FakeHistory>,
}

pub struct Upstream {
    pub cloud: bool,
    /// The only `Authorization` header the server accepts; anything else gets a 401.
    pub accepts: String,
    /// The most results the server returns at once, whatever the client asks for.
    pub page_size: usize,
    /// Whether the agile API exists (boards, sprints).
    pub agile: bool,
    /// How many comments a search result carries per issue; the rest must be asked for.
    pub embedded_comments: usize,
    /// Answers to give the coming requests instead of the proper ones: status and `Retry-After`.
    pub failures: Vec<(u16, Option<u64>)>,
    /// Every request, as `METHOD path?query`.
    pub requests: Vec<String>,
    /// Milliseconds every answer takes.
    pub delay: u64,
    pub base_url: String,
    pub users: Vec<FakeUser>,
    pub projects: Vec<FakeProject>,
    pub issues: Vec<FakeIssue>,
}

impl Upstream {
    pub fn new(cloud: bool, accepts: &str) -> Self {
        Self {
            cloud,
            accepts: accepts.to_string(),
            page_size: 50,
            agile: true,
            embedded_comments: 1000,
            failures: Vec::new(),
            requests: Vec::new(),
            delay: 0,
            base_url: String::new(),
            users: Vec::new(),
            projects: Vec::new(),
            issues: Vec::new(),
        }
    }

    pub fn issue(&mut self, key: &str) -> &mut FakeIssue {
        self.issues.iter_mut().find(|issue| issue.key == key).unwrap_or_else(|| panic!("the fake has no issue {key}"))
    }

    fn api(&self) -> &'static str {
        if self.cloud {
            "3"
        } else {
            "2"
        }
    }

    fn user(&self, account: &str) -> Value {
        let Some(user) = self.users.iter().find(|user| user.account == account) else { return Value::Null };
        let mut value = if self.cloud {
            json!({ "accountId": user.account, "displayName": user.name, "active": true, "accountType": "atlassian" })
        } else {
            json!({ "name": user.account, "key": user.account, "displayName": user.name, "active": true })
        };

        if let Some(email) = &user.email {
            value["emailAddress"] = json!(email);
        }

        value
    }

    fn text(&self, text: &Text) -> Value {
        if self.cloud {
            text.adf.clone()
        } else {
            json!(text.wiki)
        }
    }

    fn status_id(&self, name: &str) -> String {
        let all: Vec<&str> = self.projects.iter().flat_map(|project| project.statuses.iter().map(|(name, _)| name.as_str())).collect();

        (10000 + all.iter().position(|known| *known == name).unwrap_or(999)).to_string()
    }

    fn status(&self, name: &str) -> Value {
        let category = self
            .projects
            .iter()
            .flat_map(|project| &project.statuses)
            .find(|(known, _)| known == name)
            .map_or("indeterminate", |(_, key)| key.as_str());
        let category_name = match category {
            "new" => "To Do",
            "done" => "Done",
            _ => "In Progress",
        };

        json!({ "id": self.status_id(name), "name": name, "statusCategory": { "id": 2, "key": category, "name": category_name } })
    }

    fn sprint(&self, id: i64) -> Option<&FakeSprint> {
        self.projects.iter().flat_map(|project| &project.sprints).find(|sprint| sprint.id == id)
    }

    fn sprint_value(&self, sprint: &FakeSprint) -> Value {
        if self.cloud {
            json!({
                "id": sprint.id, "name": sprint.name, "state": sprint.state, "boardId": sprint.board, "goal": sprint.goal,
                "startDate": sprint.start, "endDate": sprint.end, "completeDate": sprint.complete,
            })
        } else {
            let date = |date: &Option<String>| date.clone().unwrap_or_else(|| "<null>".to_string());

            json!(format!(
                "com.atlassian.greenhopper.service.sprint.Sprint@1f2e3d[id={},rapidViewId={},state={},name={},goal={},startDate={},endDate={},completeDate={},sequence={}]",
                sprint.id,
                sprint.board,
                sprint.state.to_uppercase(),
                sprint.name,
                sprint.goal,
                date(&sprint.start),
                date(&sprint.end),
                date(&sprint.complete),
                sprint.id
            ))
        }
    }

    fn project_of(&self, key: &str) -> Option<&FakeProject> {
        self.projects.iter().find(|project| key.starts_with(&format!("{}-", project.key)))
    }

    fn comment(&self, key: &str, comment: &FakeComment) -> Value {
        json!({
            "self": format!("{}/rest/api/{}/issue/{key}/comment/{}", self.base_url, self.api(), comment.id),
            "id": comment.id.to_string(),
            "author": self.user(&comment.author),
            "body": self.text(&comment.body),
            "created": comment.created,
            "updated": comment.updated,
        })
    }

    fn worklog(&self, worklog: &FakeWorklog) -> Value {
        json!({
            "id": worklog.id.to_string(),
            "author": self.user(&worklog.author),
            "timeSpentSeconds": worklog.seconds,
            "started": worklog.started,
            "comment": worklog.comment.as_ref().map(|text| self.text(text)),
        })
    }

    fn history(&self, entry: &FakeHistory) -> Value {
        let items: Vec<Value> = entry
            .items
            .iter()
            .map(|(field, from, from_text, to, to_text)| {
                let mut item = json!({ "field": field, "fieldtype": "jira", "from": from, "fromString": from_text, "to": to, "toString": to_text });

                if self.cloud {
                    item["fieldId"] = json!(match field.as_str() {
                        "Sprint" => SPRINT_FIELD.to_string(),
                        "Story point estimate" | "Story Points" => POINTS_FIELD.to_string(),
                        other => other.to_string(),
                    });
                }

                item
            })
            .collect();

        json!({ "id": entry.id.to_string(), "author": self.user(&entry.author), "created": entry.created, "items": items })
    }

    /// An issue as a search result has it.
    fn render(&self, issue: &FakeIssue) -> Value {
        let id = 10_000 + issue.key.rsplit('-').next().and_then(|number| number.parse::<i64>().ok()).unwrap_or(0);
        let project = self.project_of(&issue.key);
        let kind = project.and_then(|project| project.types.iter().find(|(name, _, _)| *name == issue.kind));
        let named = |names: &[String]| names.iter().map(|name| json!({ "name": name })).collect::<Vec<_>>();
        let parent_is_epic = issue
            .parent
            .as_ref()
            .and_then(|parent| self.issues.iter().find(|other| other.key == *parent))
            .is_some_and(|parent| parent.kind == "Epic");
        let mut histories: Vec<Value> = issue.history.iter().map(|entry| self.history(entry)).collect();

        if self.cloud {
            histories.reverse();
        }

        let mut fields = json!({
            "summary": issue.summary,
            "description": issue.description.as_ref().map(|text| self.text(text)),
            "issuetype": { "name": issue.kind, "subtask": kind.is_some_and(|(_, subtask, _)| *subtask) },
            "status": self.status(&issue.status),
            "priority": issue.priority.as_ref().map(|name| json!({ "name": name })),
            "assignee": issue.assignee.as_ref().map(|account| self.user(account)),
            "reporter": issue.reporter.as_ref().map(|account| self.user(account)),
            "labels": issue.labels,
            "components": named(&issue.components),
            "fixVersions": named(&issue.versions),
            "duedate": issue.due,
            "timeoriginalestimate": issue.estimate,
            "created": issue.created,
            "updated": issue.updated,
            "resolutiondate": issue.resolved,
            "comment": {
                "comments": issue.comments.iter().take(self.embedded_comments).map(|comment| self.comment(&issue.key, comment)).collect::<Vec<_>>(),
                "maxResults": self.embedded_comments, "total": issue.comments.len(), "startAt": 0,
            },
            "worklog": { "worklogs": issue.worklogs.iter().map(|worklog| self.worklog(worklog)).collect::<Vec<_>>(), "maxResults": 20, "total": issue.worklogs.len(), "startAt": 0 },
            "issuelinks": issue.links.iter().enumerate().map(|(index, (kind, outward, other))| {
                let side = if *outward { "outwardIssue" } else { "inwardIssue" };
                let mut link = json!({ "id": (id * 10 + index as i64).to_string(), "type": { "name": kind, "inward": "is affected by", "outward": "affects" } });
                link[side] = json!({ "key": other, "fields": { "summary": "…" } });

                link
            }).collect::<Vec<_>>(),
            "attachment": issue.attachments.iter().enumerate().map(|(index, (name, size))| json!({
                "id": (id * 10 + index as i64).to_string(), "filename": name, "size": size, "mimeType": "application/octet-stream",
                "content": format!("{}/secure/attachment/{}/{}", self.base_url, id * 10 + index as i64, name),
            })).collect::<Vec<_>>(),
        });

        fields[POINTS_FIELD] = json!(issue.points);
        fields[SPRINT_FIELD] = match issue.sprints.as_slice() {
            [] => Value::Null,
            ids => json!(ids.iter().filter_map(|id| self.sprint(*id)).map(|sprint| self.sprint_value(sprint)).collect::<Vec<_>>()),
        };

        // Cloud names every parent in `parent`; Server keeps an epic in the "Epic Link" field.
        match &issue.parent {
            Some(parent) if !self.cloud && parent_is_epic => fields[EPIC_LINK_FIELD] = json!(parent),
            Some(parent) => fields["parent"] = json!({ "id": "1", "key": parent }),
            None => {}
        }

        json!({
            "id": id.to_string(),
            "key": issue.key,
            "self": format!("{}/rest/api/{}/issue/{id}", self.base_url, self.api()),
            "fields": fields,
            "changelog": { "startAt": 0, "maxResults": histories.len(), "total": histories.len(), "histories": histories },
        })
    }

    /// The issues a JQL query of the kind the import writes matches, ordered by key.
    fn search(&self, jql: &str) -> Result<Vec<&FakeIssue>, String> {
        let quoted: Vec<&str> = jql.split('"').collect();
        let (Some(project), true) = (quoted.get(1), jql.starts_with("project = \"") && jql.contains("ORDER BY key")) else {
            return Err(format!("The fake does not understand this JQL: {jql}"));
        };
        let since = jql.contains("updated >= ").then(|| quoted.get(3).map(|minute| minute.replace(' ', "T"))).flatten();
        let number = |issue: &FakeIssue| issue.key.rsplit('-').next().and_then(|number| number.parse::<i64>().ok()).unwrap_or(0);
        let mut found: Vec<&FakeIssue> = self
            .issues
            .iter()
            .filter(|issue| issue.key.starts_with(&format!("{project}-")))
            .filter(|issue| since.as_ref().is_none_or(|since| issue.updated.get(..16).is_some_and(|updated| updated >= since.as_str())))
            .collect();

        found.sort_by_key(|issue| number(issue));

        Ok(found)
    }
}

/// A stand-in for Jira on a port of this machine.
#[derive(Clone)]
pub struct Fake {
    pub state: Arc<Mutex<Upstream>>,
    pub base_url: String,
}

impl Fake {
    pub async fn start(upstream: Upstream) -> Self {
        Self::serve(upstream, "127.0.0.1:0").await
    }

    pub async fn serve(mut upstream: Upstream, address: &str) -> Self {
        let listener = tokio::net::TcpListener::bind(address).await.expect("a free port");
        let base_url = format!("http://{}", listener.local_addr().expect("an address"));

        upstream.base_url = base_url.clone();

        let state = Arc::new(Mutex::new(upstream));
        let routes = Router::new().fallback(handle).with_state(state.clone());

        tokio::spawn(async move {
            axum::serve(listener, routes).await.expect("the fake Jira serves");
        });

        Self { state, base_url }
    }

    pub fn with<T>(&self, change: impl FnOnce(&mut Upstream) -> T) -> T {
        change(&mut self.state.lock().expect("the fake's state"))
    }

    /// The requests made since the last call.
    pub fn take_requests(&self) -> Vec<String> {
        self.with(|upstream| std::mem::take(&mut upstream.requests))
    }
}

fn query_of(uri: &Uri) -> HashMap<String, String> {
    let decode = |text: &str| {
        let bytes = text.replace('+', " ").into_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;

        while index < bytes.len() {
            let escaped = (bytes[index] == b'%')
                .then(|| std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok().and_then(|digits| u8::from_str_radix(digits, 16).ok()))
                .flatten();

            decoded.push(escaped.unwrap_or(bytes[index]));
            index += if escaped.is_some() { 3 } else { 1 };
        }

        String::from_utf8_lossy(&decoded).into_owned()
    };

    uri.query().unwrap_or("").split('&').filter_map(|pair| pair.split_once('=')).map(|(name, value)| (decode(name), decode(value))).collect()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "errorMessages": ["No such thing here"], "errors": {} }))).into_response()
}

fn page(query: &HashMap<String, String>, limit: usize, all: Vec<Value>, list: &str) -> Value {
    let start: usize = query.get("startAt").and_then(|value| value.parse().ok()).unwrap_or(0);
    let size = query.get("maxResults").and_then(|value| value.parse().ok()).unwrap_or(50).min(limit);
    let values: Vec<Value> = all.iter().skip(start).take(size).cloned().collect();
    let mut answer = json!({ "startAt": start, "maxResults": size, "total": all.len(), "isLast": start + values.len() >= all.len() });

    answer[list] = json!(values);

    answer
}

async fn handle(State(state): State<Arc<Mutex<Upstream>>>, method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let delay = state.lock().expect("the fake's state").delay;

    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }

    let mut upstream = state.lock().expect("the fake's state");
    let query = query_of(&uri);
    let path = uri.path().to_string();
    let authorization = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).unwrap_or("").to_string();

    upstream.requests.push(format!("{method} {}", uri.path_and_query().map_or(path.as_str(), |target| target.as_str())));

    if !upstream.failures.is_empty() {
        let (status, retry_after) = upstream.failures.remove(0);
        let mut response = (StatusCode::from_u16(status).expect("a status"), "try later").into_response();

        if let Some(seconds) = retry_after {
            response.headers_mut().insert(header::RETRY_AFTER, seconds.into());
        }

        return response;
    }

    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    let api = upstream.api();

    // Like the real one, the fake says what it is to anyone.
    if segments == ["rest", "api", "2", "serverInfo"] {
        return Json(json!({
            "baseUrl": upstream.base_url,
            "version": if upstream.cloud { "1001.0.0-SNAPSHOT" } else { "9.12.4" },
            "deploymentType": if upstream.cloud { "Cloud" } else { "Server" },
            "serverTitle": if upstream.cloud { "Jira" } else { "Jira Data Center" },
        }))
        .into_response();
    }

    if authorization != upstream.accepts {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "errorMessages": ["You are not authenticated."], "errors": {} }))).into_response();
    }

    let limit = upstream.page_size;

    match (method.as_str(), segments.as_slice()) {
        ("GET", ["rest", "api", "2", "field"]) => Json(json!([
            { "id": "summary", "name": "Summary", "custom": false, "schema": { "type": "string", "system": "summary" } },
            { "id": POINTS_FIELD, "name": if upstream.cloud { "Story point estimate" } else { "Story Points" }, "custom": true, "schema": { "type": "number" } },
            { "id": SPRINT_FIELD, "name": "Sprint", "custom": true, "schema": { "type": "array", "custom": "com.pyxis.greenhopper.jira:gh-sprint" } },
            { "id": EPIC_LINK_FIELD, "name": "Epic Link", "custom": true, "schema": { "type": "any", "custom": "com.pyxis.greenhopper.jira:gh-epic-link" } },
        ]))
        .into_response(),
        ("GET", ["rest", "api", version, rest @ ..]) | ("POST", ["rest", "api", version, rest @ ..]) if *version == api => match (method.as_str(), rest) {
            ("GET", ["myself"]) => Json(json!({ "displayName": "The token's owner", "active": true })).into_response(),
            ("GET", ["project", "search"]) if upstream.cloud => {
                let all = upstream.projects.iter().map(|project| json!({ "id": "1", "key": project.key, "name": project.name })).collect();

                Json(page(&query, limit, all, "values")).into_response()
            }
            ("GET", ["project"]) if !upstream.cloud => {
                Json(json!(upstream.projects.iter().map(|project| json!({ "id": "1", "key": project.key, "name": project.name })).collect::<Vec<_>>())).into_response()
            }
            ("GET", ["project", key]) => match upstream.projects.iter().find(|project| project.key == *key) {
                Some(project) => Json(json!({
                    "id": "1",
                    "key": project.key,
                    "name": project.name,
                    "description": project.description,
                    "lead": project.lead.as_ref().map(|account| upstream.user(account)),
                    "issueTypes": project.types.iter().map(|(name, subtask, epic)| json!({
                        "id": "1", "name": name, "subtask": subtask, "hierarchyLevel": if *subtask { -1 } else if *epic { 1 } else { 0 },
                    })).collect::<Vec<_>>(),
                    "components": project.components.iter().map(|name| json!({ "id": "1", "name": name, "description": format!("Everything about {name}") })).collect::<Vec<_>>(),
                    "versions": project.versions.iter().map(|version| json!({
                        "id": "1", "name": version.name, "released": version.released, "archived": version.archived,
                        "startDate": version.start, "releaseDate": version.release, "description": "",
                    })).collect::<Vec<_>>(),
                }))
                .into_response(),
                None => not_found(),
            },
            ("GET", ["project", key, "statuses"]) => match upstream.projects.iter().find(|project| project.key == *key) {
                Some(project) => Json(json!(project
                    .types
                    .iter()
                    .map(|(name, _, _)| json!({ "name": name, "statuses": project.statuses.iter().map(|(status, _)| upstream.status(status)).collect::<Vec<_>>() }))
                    .collect::<Vec<_>>()))
                .into_response(),
                None => not_found(),
            },
            ("GET", ["priority"]) => {
                Json(json!(["Highest", "High", "Medium", "Low", "Lowest"].iter().map(|name| json!({ "name": name })).collect::<Vec<_>>())).into_response()
            }
            ("GET", ["user", "assignable", "search"]) => {
                let start: usize = query.get("startAt").and_then(|value| value.parse().ok()).unwrap_or(0);
                let all: Vec<Value> = upstream.users.iter().map(|user| upstream.user(&user.account)).collect();

                Json(json!(all.into_iter().skip(start).take(limit).collect::<Vec<_>>())).into_response()
            }
            ("GET", ["search", "jql"]) if upstream.cloud => match upstream.search(query.get("jql").map_or("", String::as_str)) {
                Ok(found) => {
                    let start: usize = query.get("nextPageToken").and_then(|token| token.strip_prefix("page-")).and_then(|start| start.parse().ok()).unwrap_or(0);
                    let size = query.get("maxResults").and_then(|value| value.parse().ok()).unwrap_or(50).min(limit);
                    let issues: Vec<Value> = found.iter().skip(start).take(size).map(|issue| upstream.render(issue)).collect();
                    let last = start + issues.len() >= found.len();
                    let mut answer = json!({ "issues": issues, "isLast": last });

                    if !last {
                        answer["nextPageToken"] = json!(format!("page-{}", start + size));
                    }

                    if query.get("expand").map(String::as_str) != Some("changelog") || !query.contains_key("fields") {
                        return (StatusCode::BAD_REQUEST, Json(json!({ "errorMessages": ["The import must ask for its fields and the changelog"] }))).into_response();
                    }

                    Json(answer).into_response()
                }
                Err(message) => (StatusCode::BAD_REQUEST, Json(json!({ "errorMessages": [message] }))).into_response(),
            },
            ("POST", ["search", "approximate-count"]) if upstream.cloud => {
                let jql = serde_json::from_slice::<Value>(&body).ok().and_then(|body| body["jql"].as_str().map(String::from)).unwrap_or_default();

                match upstream.search(&format!("{jql} ORDER BY key")) {
                    Ok(found) => Json(json!({ "count": found.len() })).into_response(),
                    Err(message) => (StatusCode::BAD_REQUEST, Json(json!({ "errorMessages": [message] }))).into_response(),
                }
            }
            ("GET", ["search"]) if !upstream.cloud => {
                let jql = query.get("jql").cloned().unwrap_or_default();
                let jql = if jql.contains("ORDER BY") { jql } else { format!("{jql} ORDER BY key") };

                match upstream.search(&jql) {
                    Ok(found) => Json(page(&query, limit, found.iter().map(|issue| upstream.render(issue)).collect(), "issues")).into_response(),
                    Err(message) => (StatusCode::BAD_REQUEST, Json(json!({ "errorMessages": [message] }))).into_response(),
                }
            }
            ("GET", ["issue", key, what @ ("comment" | "worklog" | "changelog")]) => match upstream.issues.iter().find(|issue| issue.key == *key) {
                Some(issue) => match *what {
                    "comment" => Json(page(&query, limit, issue.comments.iter().map(|comment| upstream.comment(key, comment)).collect(), "comments")).into_response(),
                    "worklog" => Json(page(&query, limit, issue.worklogs.iter().map(|worklog| upstream.worklog(worklog)).collect(), "worklogs")).into_response(),
                    _ => Json(page(&query, limit, issue.history.iter().map(|entry| upstream.history(entry)).collect(), "values")).into_response(),
                },
                None => not_found(),
            },
            _ => not_found(),
        },
        ("GET", ["rest", "agile", "1.0", rest @ ..]) if upstream.agile => match rest {
            ["board"] => {
                let wanted = query.get("projectKeyOrId").cloned().unwrap_or_default();
                let all = upstream
                    .projects
                    .iter()
                    .filter(|project| project.key == wanted)
                    .flat_map(|project| &project.boards)
                    .map(|board| json!({ "id": board.id, "name": board.name, "type": if board.scrum { "scrum" } else { "kanban" } }))
                    .collect();

                Json(page(&query, limit, all, "values")).into_response()
            }
            ["board", id, "sprint"] => {
                let id: i64 = id.parse().unwrap_or(0);
                let board = upstream.projects.iter().flat_map(|project| &project.boards).find(|board| board.id == id);

                match board {
                    Some(board) if board.scrum => {
                        let all = upstream
                            .projects
                            .iter()
                            .flat_map(|project| &project.sprints)
                            .filter(|sprint| sprint.board == id)
                            .map(|sprint| {
                                json!({
                                    "id": sprint.id, "name": sprint.name, "state": sprint.state, "originBoardId": sprint.board, "goal": sprint.goal,
                                    "startDate": sprint.start, "endDate": sprint.end, "completeDate": sprint.complete,
                                })
                            })
                            .collect();

                        Json(page(&query, limit, all, "values")).into_response()
                    }
                    Some(_) => (StatusCode::BAD_REQUEST, Json(json!({ "errorMessages": ["The board does not support sprints"] }))).into_response(),
                    None => not_found(),
                }
            }
            ["board", id, "configuration"] => {
                let id: i64 = id.parse().unwrap_or(0);

                match upstream.projects.iter().flat_map(|project| &project.boards).find(|board| board.id == id) {
                    Some(board) => Json(json!({
                        "id": board.id,
                        "name": board.name,
                        "columnConfig": { "columns": board.columns.iter().enumerate().map(|(index, statuses)| json!({
                            "name": format!("Column {}", index + 1),
                            "statuses": statuses.iter().map(|name| json!({ "id": upstream.status_id(name) })).collect::<Vec<_>>(),
                        })).collect::<Vec<_>>() },
                    }))
                    .into_response(),
                    None => not_found(),
                }
            }
            _ => not_found(),
        },
        _ => not_found(),
    }
}

// ---------- a realistic project ----------

fn change(field: &str, from: Option<&str>, from_text: Option<&str>, to: Option<&str>, to_text: Option<&str>) -> FakeChange {
    (field.to_string(), from.map(String::from), from_text.map(String::from), to.map(String::from), to_text.map(String::from))
}

/// A web shop's project, about two months into its life: four epics, stories, bugs and tasks
/// across three completed sprints, a running one and a planned one, subtasks, comments, work
/// logs, links, attachments and a status history for every issue that has moved.
pub fn demo(cloud: bool, accepts: &str) -> Upstream {
    let mut upstream = Upstream::new(cloud, accepts);
    // Jira's way of writing an instant, `days` days and `hours` hours from the start of this hour.
    let now = chrono::Utc::now().timestamp() / 3600 * 3600;
    let at = |days: i64, hours: i64| {
        chrono::DateTime::from_timestamp(now + days * 86_400 + hours * 3600, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%S%.3f+0000").to_string()
    };
    let people = [
        ("anna", "Anna Nowak", Some("anna.nowak@example.com")),
        ("jan", "Jan Kowalski", Some("jan.kowalski@example.com")),
        ("ola", "Aleksandra Wiśniewska", None),
        ("piotr", "Piotr Zieliński", Some("piotr.zielinski@example.com")),
        ("kasia", "Katarzyna Wójcik", None),
        ("marek", "Marek Lewandowski", None),
    ];

    upstream.users = people
        .iter()
        .enumerate()
        .map(|(index, (login, name, email))| FakeUser {
            account: if cloud { format!("5b10ac8d82e05b22cc7d4e{index:02}") } else { login.to_string() },
            name: name.to_string(),
            email: email.map(String::from),
        })
        .collect();

    let accounts: Vec<String> = upstream.users.iter().map(|user| user.account.clone()).collect();
    let account = |index: usize| accounts[index % accounts.len()].clone();
    // Sprints of two weeks; the fourth is running.
    let sprint_days: [(i64, i64); 5] = [(-49, -35), (-35, -21), (-21, -7), (-7, 7), (7, 21)];
    let sprints: Vec<FakeSprint> = sprint_days
        .iter()
        .enumerate()
        .map(|(index, (start, end))| {
            let state = match index {
                0..=2 => "closed",
                3 => "active",
                _ => "future",
            };

            FakeSprint {
                id: 41 + index as i64,
                board: 7,
                name: format!("WEB Sprint {}", index + 1),
                state: state.to_string(),
                start: (state != "future").then(|| at(*start, 0)),
                end: (state != "future").then(|| at(*end, 0)),
                complete: (state == "closed").then(|| at(*end, 2)),
                goal: ["Koszyk działa od początku do końca", "Płatności kartą", "Konto klienta", "BLIK i e-maile", ""][index].to_string(),
            }
        })
        .collect();
    let statuses = ["Backlog", "To Do", "In Progress", "In Review", "Done"];
    let status_id = |name: &str| (10000 + statuses.iter().position(|known| *known == name).unwrap_or(0)).to_string();

    upstream.projects.push(FakeProject {
        key: "WEB".to_string(),
        name: "Sklep internetowy".to_string(),
        description: "Sklep internetowy: koszyk, płatności, konto klienta.".to_string(),
        lead: Some(account(0)),
        types: vec![
            ("Epic".to_string(), false, true),
            ("Story".to_string(), false, false),
            ("Task".to_string(), false, false),
            ("Bug".to_string(), false, false),
            ("Sub-task".to_string(), true, false),
        ],
        statuses: vec![
            ("To Do".to_string(), "new".to_string()),
            ("In Progress".to_string(), "indeterminate".to_string()),
            ("Done".to_string(), "done".to_string()),
            ("In Review".to_string(), "indeterminate".to_string()),
            ("Backlog".to_string(), "new".to_string()),
        ],
        components: ["Koszyk", "Płatności", "Frontend", "Backend"].map(String::from).to_vec(),
        versions: vec![
            FakeVersion {
                name: "1.0".to_string(),
                released: true,
                archived: false,
                start: Some(at(-49, 0)[..10].to_string()),
                release: Some(at(-21, 0)[..10].to_string()),
            },
            FakeVersion {
                name: "1.1".to_string(),
                released: true,
                archived: false,
                start: Some(at(-21, 0)[..10].to_string()),
                release: Some(at(-7, 0)[..10].to_string()),
            },
            FakeVersion {
                name: "1.2".to_string(),
                released: false,
                archived: false,
                start: Some(at(-7, 0)[..10].to_string()),
                release: Some(at(14, 0)[..10].to_string()),
            },
        ],
        boards: vec![FakeBoard {
            id: 7,
            name: "WEB board".to_string(),
            scrum: true,
            columns: vec![vec!["To Do".to_string()], vec!["In Progress".to_string()], vec!["In Review".to_string()], vec!["Done".to_string()]],
        }],
        sprints: sprints.clone(),
    });

    let epics = ["Proces zakupowy", "Płatności", "Konto klienta", "Wydajność i SEO"];
    let work = [
        "Koszyk: dodawanie i usuwanie produktów",
        "Formularz adresu dostawy z walidacją",
        "Cena w koszyku nie uwzględnia rabatu",
        "Konfiguracja CI: testy i lint na każdym PR",
        "Podsumowanie zamówienia",
        "Płatność kartą przez bramkę",
        "Obsługa 3-D Secure",
        "Błąd 500 przy pustym koszyku",
        "Webhook potwierdzenia płatności",
        "Zwroty płatności",
        "Rejestracja i logowanie e-mailem",
        "Reset hasła",
        "Historia zamówień w profilu",
        "Walidacja kodu pocztowego odrzuca poprawne kody",
        "Migracja sesji do Redis",
        "Płatność BLIK",
        "E-mail z potwierdzeniem zamówienia",
        "Testy e2e ścieżki zakupowej",
        "Kody rabatowe",
        "Menu mobilne nie zamyka się po wyborze kategorii",
        "Lista życzeń",
        "Lazy loading obrazów produktów",
        "Porównywarka produktów",
        "Opinie i oceny produktów",
        "Mapa witryny i dane strukturalne",
        "Literówka w stopce",
        "Logowanie przez Google",
        "Faktury PDF do zamówień",
        "Powiadomienia o statusie przesyłki",
        "Filtrowanie listy produktów po cenie",
        "Cache listy kategorii",
        "Zdjęcia produktów w wysokiej rozdzielczości",
        "Dostawa do paczkomatu",
        "Eksport zamówień do CSV",
        "Panel zwrotów dla obsługi",
        "Tryb ciemny",
        "Strona 404 z wyszukiwarką",
        "Limit prób logowania",
        "Raport sprzedaży dziennej",
        "Newsletter: zapis i wypis",
        "Błędne zaokrąglenie VAT przy rabacie",
        "Wyszukiwarka z podpowiedziami",
    ];
    let mut number = 0;
    let mut history_id = 5000;
    let mut comment_id = 9000;
    let next = |counter: &mut i64| {
        *counter += 1;
        *counter
    };

    for (index, name) in epics.iter().enumerate() {
        number += 1;
        let started = index < 3;

        upstream.issues.push(FakeIssue {
            key: format!("WEB-{number}"),
            kind: "Epic".to_string(),
            status: if started { "In Progress" } else { "To Do" }.to_string(),
            priority: Some("Medium".to_string()),
            summary: name.to_string(),
            description: Some(Text::plain(&format!("Epik: {name}.\nZakres uzgodniony na warsztacie z zespołem sprzedaży."))),
            reporter: Some(account(0)),
            created: at(-56, index as i64),
            updated: at(if started { -48 + 14 * index as i64 } else { -56 }, 1),
            history: if started {
                vec![FakeHistory {
                    id: next(&mut history_id),
                    author: account(0),
                    created: at(-48 + 14 * index as i64, 1),
                    items: vec![change("status", Some(&status_id("To Do")), Some("To Do"), Some(&status_id("In Progress")), Some("In Progress"))],
                }]
            } else {
                Vec::new()
            },
            ..Default::default()
        });
    }

    for (index, summary) in work.iter().enumerate() {
        number += 1;

        // Jira's numbers have gaps: issues get deleted and moved.
        if index == 20 {
            number += 3;
        }

        // Where the issue is in its life: the first sprints are done, the running one is in
        // every state, the rest waits in the planned sprint or the backlog.
        let sprint = match index {
            0..=7 => Some(0),
            8..=15 => Some(1),
            16..=23 => Some(2),
            24..=33 => Some(3),
            34..=37 => Some(4),
            _ => None,
        };
        let kind = match index % 7 {
            2 | 6 => "Bug",
            3 => "Task",
            _ => "Story",
        };
        let assignee = account(index);
        let (start, end) = sprint.map_or((0, 0), |sprint| sprint_days[sprint]);
        let steps = match (sprint, index % 5) {
            (Some(0..=2), _) => 3,
            (Some(3), 0 | 1) => 3,
            (Some(3), 2) => 2,
            (Some(3), 3) => 1,
            _ => 0,
        };
        // Two issues of the second sprint were not finished in it and moved to the third.
        let carried = matches!(index, 14 | 15);
        let moves = ["To Do", "In Progress", "In Review", "Done"];
        let offset = 1 + (index as i64 % 4);
        // The running sprint is a week old: its issues move a day apart, the others two.
        let pace = if sprint == Some(3) { 1 } else { 2 };
        let moved_at = |step: usize| at(start + if carried { 14 } else { 0 } + offset + pace * step as i64, (index as i64 * 3) % 7);
        let mut history: Vec<FakeHistory> = Vec::new();
        let created = at(start.min(-7) - 6 - (index as i64 % 5), index as i64 % 6);

        if let Some(sprint) = sprint {
            let target = &sprints[sprint];

            history.push(FakeHistory {
                id: next(&mut history_id),
                author: account(0),
                created: at((start - 1).min(-1), 3),
                items: vec![change("Sprint", None, None, Some(&target.id.to_string()), Some(&target.name))],
            });
        }

        if carried {
            let (from, to) = (&sprints[1], &sprints[2]);

            history.push(FakeHistory {
                id: next(&mut history_id),
                author: account(0),
                // Moved on when the second sprint was completed, as Jira offers to.
                created: at(sprint_days[1].1, 2),
                items: vec![change(
                    "Sprint",
                    Some(&from.id.to_string()),
                    Some(&from.name),
                    Some(&format!("{}, {}", from.id, to.id)),
                    Some(&format!("{}, {}", from.name, to.name)),
                )],
            });
        }

        for step in 0..steps {
            history.push(FakeHistory {
                id: next(&mut history_id),
                author: assignee.clone(),
                created: moved_at(step),
                items: vec![change(
                    "status",
                    Some(&status_id(moves[step])),
                    Some(moves[step]),
                    Some(&status_id(moves[step + 1])),
                    Some(moves[step + 1]),
                )],
            });
        }

        if index % 9 == 4 && sprint.is_some() {
            history.push(FakeHistory {
                id: next(&mut history_id),
                author: account(0),
                created: at(start, 5),
                items: vec![change(if cloud { "Story point estimate" } else { "Story Points" }, None, Some("3"), None, Some("5"))],
            });
        }

        history.sort_by(|left, right| left.created.cmp(&right.created));

        let done = steps == 3;
        let status = if sprint.is_none() { "Backlog" } else { moves[steps] };
        let resolved = done.then(|| moved_at(2));
        let updated = history.last().map_or_else(|| created.clone(), |entry| entry.created.clone());
        let comments: Vec<FakeComment> = (0..(index % 4))
            .map(|turn| {
                let when = at(start.min(-7) + turn as i64 + 1, 2 + turn as i64);
                let text = [
                    "Zrobione na gałęzi, proszę o review.",
                    "Sprawdziłam na stagingu: działa, ale komunikat błędu jest po angielsku.",
                    "Poprawione, komunikaty są już po polsku.",
                ][turn];

                FakeComment {
                    id: next(&mut comment_id),
                    author: account(index + turn + 1),
                    body: Text::plain(text),
                    created: when.clone(),
                    updated: when,
                }
            })
            .collect();
        let description = if index % 6 == 2 {
            Text::new(
                json!({ "type": "doc", "version": 1, "content": [
                    { "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "Kroki" }] },
                    { "type": "orderedList", "content": [
                        { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Dodaj produkt objęty promocją" }] }] },
                        { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Otwórz " }, { "type": "text", "text": "koszyk", "marks": [{ "type": "strong" }] }] }] },
                    ] },
                    { "type": "panel", "attrs": { "panelType": "warning" }, "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Dotyczy tylko zalogowanych klientów." }] }] },
                    { "type": "codeBlock", "attrs": { "language": "json" }, "content": [{ "type": "text", "text": "{ \"total\": 199.99, \"discount\": 0 }" }] },
                ] }),
                "h2. Kroki\n\n# Dodaj produkt objęty promocją\n# Otwórz *koszyk*\n\n{panel:title=Uwaga}\nDotyczy tylko zalogowanych klientów.\n{panel}\n\n{code:json}\n{ \"total\": 199.99, \"discount\": 0 }\n{code}",
            )
        } else {
            Text::plain(&format!("{summary}.\nKryteria akceptacji ustalone z właścicielem produktu."))
        };

        upstream.issues.push(FakeIssue {
            key: format!("WEB-{number}"),
            kind: kind.to_string(),
            status: status.to_string(),
            priority: Some(["Medium", "High", "Medium", "Low", "Highest", "Medium", "Lowest"][index % 7].to_string()),
            summary: summary.to_string(),
            description: Some(description),
            assignee: sprint.map(|_| assignee.clone()),
            reporter: Some(account(index + 2)),
            labels: match index % 6 {
                0 => vec!["frontend".to_string()],
                2 => vec!["regresja".to_string(), "klient".to_string()],
                _ => Vec::new(),
            },
            components: vec![["Koszyk", "Płatności", "Frontend", "Backend"][index % 4].to_string()],
            versions: match sprint {
                Some(0 | 1) => vec!["1.0".to_string()],
                Some(2) => vec!["1.1".to_string()],
                Some(3) => vec!["1.2".to_string()],
                _ => Vec::new(),
            },
            sprints: match (sprint, carried) {
                (Some(_), true) => vec![sprints[1].id, sprints[2].id],
                (Some(sprint), false) => vec![sprints[sprint].id],
                (None, _) => Vec::new(),
            },
            points: (kind != "Task" || index % 2 == 0).then(|| [1.0, 2.0, 3.0, 5.0, 8.0][index % 5]),
            due: (index % 8 == 5).then(|| at(end, 0)[..10].to_string()),
            estimate: (index % 5 == 0).then_some(4 * 3600),
            parent: Some(format!("WEB-{}", 1 + index % 4)),
            created,
            updated,
            resolved,
            comments,
            worklogs: if done && index % 3 == 0 {
                vec![FakeWorklog {
                    id: 7000 + index as i64,
                    author: assignee.clone(),
                    seconds: 5400 + 1800 * (index as i64 % 3),
                    started: moved_at(1),
                    comment: Some(Text::plain("Implementacja i testy")),
                }]
            } else {
                Vec::new()
            },
            links: Vec::new(),
            attachments: if index % 10 == 2 { vec![("zrzut-ekranu.png".to_string(), 184_320)] } else { Vec::new() },
            history,
        });
    }

    // Links, each listed on both of its issues as Jira does.
    for (kind, source, target) in
        [("Blocks", "WEB-10", "WEB-11"), ("Blocks", "WEB-10", "WEB-13"), ("Duplicate", "WEB-45", "WEB-7"), ("Relates", "WEB-20", "WEB-21")]
    {
        upstream.issue(source).links.push((kind.to_string(), true, target.to_string()));
        upstream.issue(target).links.push((kind.to_string(), false, source.to_string()));
    }

    // Subtasks under stories of the running sprint.
    let parents =
        ["WEB-32", "WEB-32", "WEB-32", "WEB-33", "WEB-33", "WEB-35", "WEB-35", "WEB-36", "WEB-37", "WEB-37", "WEB-38", "WEB-39", "WEB-39", "WEB-40"];
    let parts = [
        "Formularz (frontend)",
        "Endpoint API",
        "Testy jednostkowe",
        "Migracja bazy",
        "Przegląd bezpieczeństwa",
        "Makieta",
        "Implementacja",
        "Tłumaczenia",
        "Obsługa błędów",
        "Dokumentacja",
        "Konfiguracja",
        "Monitoring",
        "Testy ręczne",
        "Wdrożenie na staging",
    ];

    for (index, (parent, part)) in parents.iter().zip(parts).enumerate() {
        number += 1;
        let parent_issue = upstream.issue(parent).clone();
        let assignee = parent_issue.assignee.clone().unwrap_or_else(|| account(index));
        let steps = [3, 1, 0, 3, 2][index % 5];
        let moves = ["To Do", "In Progress", "In Review", "Done"];
        let moved_at = |step: usize| at(-6 + step as i64 + (index as i64 % 3), index as i64 % 5);
        let history: Vec<FakeHistory> = (0..steps)
            .map(|step| FakeHistory {
                id: next(&mut history_id),
                author: assignee.clone(),
                created: moved_at(step),
                items: vec![change(
                    "status",
                    Some(&status_id(moves[step])),
                    Some(moves[step]),
                    Some(&status_id(moves[step + 1])),
                    Some(moves[step + 1]),
                )],
            })
            .collect();

        upstream.issues.push(FakeIssue {
            key: format!("WEB-{number}"),
            kind: "Sub-task".to_string(),
            status: moves[steps].to_string(),
            priority: Some("Medium".to_string()),
            summary: format!("{part}: {}", parent_issue.summary.to_lowercase()),
            assignee: Some(assignee),
            reporter: parent_issue.reporter.clone(),
            sprints: parent_issue.sprints.clone(),
            parent: Some(parent.to_string()),
            created: at(-8, index as i64 % 6),
            updated: history.last().map_or_else(|| at(-8, index as i64 % 6), |entry| entry.created.clone()),
            resolved: (steps == 3).then(|| moved_at(2)),
            history,
            ..Default::default()
        });
    }

    upstream
}
