//! What Jira has, in one shape whichever way it arrived: REST v3 with ADF (Cloud), REST v2
//! with wiki markup (Server and Data Center), or a CSV export. Parsing only; no network.

use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use serde_json::Value;

/// A description or comment as Jira stored it; converted to Markdown later, in parallel.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Body {
    #[default]
    None,
    Adf(Value),
    Wiki(String),
}

impl Body {
    pub fn of(value: &Value) -> Self {
        match value {
            Value::String(text) if !text.trim().is_empty() => Self::Wiki(text.clone()),
            Value::Object(_) => Self::Adf(value.clone()),
            _ => Self::None,
        }
    }

    pub fn to_markdown(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Adf(document) => crate::adf::adf_to_markdown(document),
            Self::Wiki(text) => crate::wiki::wiki_to_markdown(text),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteUser {
    /// What identifies the person in Jira: the account id (Cloud) or the user key or name (Server).
    pub account: String,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteType {
    pub name: String,
    pub subtask: bool,
    pub epic: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteStatus {
    pub id: String,
    pub name: String,
    /// Jira's status category key: `new`, `indeterminate` or `done`; empty when unknown.
    pub category: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteVersion {
    pub name: String,
    pub description: String,
    pub released: bool,
    pub archived: bool,
    pub start_date: Option<String>,
    pub release_date: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteSprint {
    pub id: String,
    pub name: String,
    /// `future`, `active` or `closed`.
    pub state: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub completed: Option<String>,
    pub goal: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteComment {
    pub id: String,
    pub author: Option<RemoteUser>,
    pub body: Body,
    pub created: String,
    pub updated: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteWorklog {
    pub id: String,
    pub author: Option<RemoteUser>,
    pub seconds: i64,
    pub started: String,
    pub comment: Body,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteLink {
    pub type_name: String,
    /// True when this issue is the one that blocks, duplicates or relates to `other_key`.
    pub outward: bool,
    pub other_key: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteAttachment {
    pub filename: String,
    pub url: String,
    pub size: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteChange {
    /// The field's name as the changelog has it (`status`, `Sprint`, `assignee`, `Story Points`).
    pub field: String,
    pub field_id: String,
    pub from: Option<String>,
    pub from_text: Option<String>,
    pub to: Option<String>,
    pub to_text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteHistory {
    /// Jira's id of the changelog entry; they grow, which is what makes a second import cheap.
    pub id: i64,
    pub author: Option<RemoteUser>,
    pub created: String,
    pub items: Vec<RemoteChange>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteIssue {
    pub id: String,
    pub key: String,
    pub number: i64,
    /// The REST address of the issue; empty for a CSV row.
    pub self_url: String,
    pub summary: String,
    pub description: Body,
    pub type_name: String,
    pub status_name: String,
    pub priority: Option<String>,
    pub assignee: Option<RemoteUser>,
    pub reporter: Option<RemoteUser>,
    pub labels: Vec<String>,
    pub components: Vec<String>,
    pub fix_versions: Vec<String>,
    /// Every sprint the issue has been in, in the order Jira lists them.
    pub sprints: Vec<RemoteSprint>,
    pub points: Option<f64>,
    pub due_date: Option<String>,
    pub estimate_seconds: Option<i64>,
    pub parent_key: Option<String>,
    pub created: String,
    pub updated: String,
    /// `updated` exactly as Jira wrote it, in the time zone of the account that asked.
    pub updated_raw: String,
    pub resolved: Option<String>,
    pub comments: Vec<RemoteComment>,
    pub worklogs: Vec<RemoteWorklog>,
    pub links: Vec<RemoteLink>,
    pub attachments: Vec<RemoteAttachment>,
    pub history: Vec<RemoteHistory>,
    /// How many comments, work logs and changelog entries Jira says there are; when more
    /// than came with the search result, the rest is fetched per issue.
    pub comment_total: usize,
    pub worklog_total: usize,
    pub history_total: usize,
}

/// The ids of the custom fields that matter, which differ from one Jira to the next.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fields {
    pub points: Vec<String>,
    pub sprint: Option<String>,
    pub epic_link: Option<String>,
}

impl Fields {
    /// From the answer of `/rest/api/2/field`.
    pub fn discover(fields: &Value) -> Self {
        let mut found = Self::default();

        for field in fields.as_array().map(Vec::as_slice).unwrap_or_default() {
            let Some(id) = field["id"].as_str() else { continue };
            let name = field["name"].as_str().unwrap_or("").trim().to_lowercase();
            let custom = field["schema"]["custom"].as_str().unwrap_or("");

            if name == "story points" || name == "story point estimate" {
                // The classic field first: where both exist it is the one boards use.
                if name == "story points" {
                    found.points.insert(0, id.to_string());
                } else {
                    found.points.push(id.to_string());
                }
            } else if custom.ends_with(":gh-sprint") || (name == "sprint" && found.sprint.is_none()) {
                found.sprint = Some(id.to_string());
            } else if custom.ends_with(":gh-epic-link") || (name == "epic link" && found.epic_link.is_none()) {
                found.epic_link = Some(id.to_string());
            }
        }

        found
    }

    pub fn is_points(&self, change: &RemoteChange) -> bool {
        let name = change.field.to_lowercase();

        self.points.contains(&change.field_id) || name == "story points" || name == "story point estimate"
    }
}

/// An instant in UTC the way team-hub stores them. Jira writes `2024-03-12T09:15:30.000+0100`.
pub fn instant(text: &str) -> Option<String> {
    let text = text.trim();
    let parsed = DateTime::parse_from_rfc3339(text)
        .or_else(|_| DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .map(|instant| instant.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M"]
                .iter()
                .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
                .or_else(|| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().and_then(|date| date.and_hms_opt(0, 0, 0)))
                .map(|naive| naive.and_utc())
        })?;

    Some(parsed.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The `YYYY-MM-DD` at the start of a date or instant.
pub fn day(text: &str) -> Option<String> {
    let day = text.trim().get(..10)?;

    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok().map(|date| date.format("%Y-%m-%d").to_string())
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn names(value: &Value) -> Vec<String> {
    items(value).iter().filter_map(|entry| text_of(&entry["name"]).or_else(|| text_of(entry))).collect()
}

pub fn user(value: &Value) -> Option<RemoteUser> {
    let account = text_of(&value["accountId"]).or_else(|| text_of(&value["key"])).or_else(|| text_of(&value["name"]))?;
    let name = text_of(&value["displayName"]).or_else(|| text_of(&value["name"])).unwrap_or_else(|| account.clone());

    Some(RemoteUser { account, name, email: text_of(&value["emailAddress"]).filter(|email| email.contains('@')) })
}

pub fn status(value: &Value) -> Option<RemoteStatus> {
    Some(RemoteStatus {
        id: text_of(&value["id"]).unwrap_or_default(),
        name: text_of(&value["name"])?,
        category: text_of(&value["statusCategory"]["key"]).unwrap_or_default(),
    })
}

pub fn version(value: &Value) -> Option<RemoteVersion> {
    Some(RemoteVersion {
        name: text_of(&value["name"])?,
        description: text_of(&value["description"]).unwrap_or_default(),
        released: value["released"].as_bool().unwrap_or(false),
        archived: value["archived"].as_bool().unwrap_or(false),
        start_date: value["startDate"].as_str().and_then(day),
        release_date: value["releaseDate"].as_str().and_then(day),
    })
}

pub fn issue_type(value: &Value) -> Option<RemoteType> {
    let name = text_of(&value["name"])?;
    let epic = value["hierarchyLevel"].as_i64() == Some(1) || name.eq_ignore_ascii_case("epic");

    Some(RemoteType { subtask: value["subtask"].as_bool().unwrap_or(false), epic, name })
}

/// Jira Server writes a sprint as the `toString` of a Java object:
/// `com.atlassian.greenhopper.service.sprint.Sprint@1f[id=7,rapidViewId=2,state=CLOSED,name=Sprint 3,…]`.
fn sprint_from_text(text: &str) -> Option<RemoteSprint> {
    const KEYS: [&str; 12] =
        ["id", "rapidViewId", "state", "name", "goal", "startDate", "endDate", "completeDate", "activatedDate", "sequence", "autoStartStop", "synced"];
    let inside = &text[text.find('[')? + 1..text.rfind(']')?];
    let value = |key: &str| -> Option<String> {
        let marker = format!("{key}=");
        let at = if inside.starts_with(&marker) { 0 } else { inside.find(&format!(",{marker}"))? + 1 };
        let start = at + marker.len();
        let rest = &inside[start..];
        let end = KEYS.iter().filter_map(|next| rest.find(&format!(",{next}="))).min().unwrap_or(rest.len());
        let value = rest[..end].trim();

        (!value.is_empty() && value != "<null>").then(|| value.to_string())
    };

    Some(RemoteSprint {
        id: value("id")?,
        name: value("name")?,
        state: value("state").unwrap_or_default().to_lowercase(),
        start: value("startDate").as_deref().and_then(instant),
        end: value("endDate").as_deref().and_then(instant),
        completed: value("completeDate").as_deref().and_then(instant),
        goal: value("goal").unwrap_or_default(),
    })
}

pub fn sprint(value: &Value) -> Option<RemoteSprint> {
    if let Some(text) = value.as_str() {
        return sprint_from_text(text);
    }

    Some(RemoteSprint {
        id: text_of(&value["id"])?,
        name: text_of(&value["name"])?,
        state: text_of(&value["state"]).unwrap_or_default().to_lowercase(),
        start: value["startDate"].as_str().and_then(instant),
        end: value["endDate"].as_str().and_then(instant),
        completed: value["completeDate"].as_str().and_then(instant),
        goal: text_of(&value["goal"]).unwrap_or_default(),
    })
}

pub fn comment(value: &Value) -> Option<RemoteComment> {
    let created = value["created"].as_str().and_then(instant)?;

    Some(RemoteComment {
        id: text_of(&value["id"])?,
        author: user(&value["author"]),
        body: Body::of(&value["body"]),
        updated: value["updated"].as_str().and_then(instant).unwrap_or_else(|| created.clone()),
        created,
    })
}

pub fn worklog(value: &Value) -> Option<RemoteWorklog> {
    Some(RemoteWorklog {
        id: text_of(&value["id"])?,
        author: user(&value["author"]),
        seconds: value["timeSpentSeconds"].as_i64()?,
        started: value["started"].as_str().and_then(instant)?,
        comment: Body::of(&value["comment"]),
    })
}

pub fn history(value: &Value) -> Option<RemoteHistory> {
    Some(RemoteHistory {
        id: text_of(&value["id"]).and_then(|id| id.parse().ok()).unwrap_or(0),
        author: user(&value["author"]),
        created: value["created"].as_str().and_then(instant)?,
        items: items(&value["items"])
            .iter()
            .map(|item| RemoteChange {
                field: text_of(&item["field"]).unwrap_or_default(),
                field_id: text_of(&item["fieldId"]).unwrap_or_default(),
                from: text_of(&item["from"]),
                from_text: text_of(&item["fromString"]),
                to: text_of(&item["to"]),
                to_text: text_of(&item["toString"]),
            })
            .collect(),
    })
}

/// `WEB-123` → 123.
pub fn number_of(key: &str) -> Option<i64> {
    hub_core::keys::parse_issue_key(key).map(|(_, number)| number)
}

/// One issue of a search result. None when it lacks what an issue cannot do without.
pub fn issue(value: &Value, fields: &Fields) -> Option<RemoteIssue> {
    let key = text_of(&value["key"])?;
    let number = number_of(&key)?;
    let data = &value["fields"];
    let created = data["created"].as_str().and_then(instant)?;
    let updated_raw = data["updated"].as_str().unwrap_or("").to_string();
    let points = fields.points.iter().find_map(|id| data[id.as_str()].as_f64());
    let sprints = fields.sprint.as_deref().map(|id| items(&data[id]).iter().filter_map(sprint).collect()).unwrap_or_default();
    let epic_link = fields.epic_link.as_deref().and_then(|id| text_of(&data[id]));
    let links = items(&data["issuelinks"])
        .iter()
        .filter_map(|link| {
            let type_name = text_of(&link["type"]["name"]).unwrap_or_default();

            match (text_of(&link["outwardIssue"]["key"]), text_of(&link["inwardIssue"]["key"])) {
                (Some(other_key), _) => Some(RemoteLink { type_name, outward: true, other_key }),
                (None, Some(other_key)) => Some(RemoteLink { type_name, outward: false, other_key }),
                _ => None,
            }
        })
        .collect();
    let attachments = items(&data["attachment"])
        .iter()
        .filter_map(|file| {
            Some(RemoteAttachment { filename: text_of(&file["filename"])?, url: text_of(&file["content"]).unwrap_or_default(), size: file["size"].as_i64() })
        })
        .collect();
    let comments: Vec<RemoteComment> = items(&data["comment"]["comments"]).iter().filter_map(comment).collect();
    let worklogs: Vec<RemoteWorklog> = items(&data["worklog"]["worklogs"]).iter().filter_map(worklog).collect();
    let mut history: Vec<RemoteHistory> = items(&value["changelog"]["histories"]).iter().filter_map(history).collect();
    let total = |value: &Value, have: usize| value.as_u64().map_or(have, |total| total as usize);

    // Cloud lists the newest change first; the import reads them oldest first.
    history.sort_by(|left, right| left.created.cmp(&right.created).then(left.id.cmp(&right.id)));

    Some(RemoteIssue {
        id: text_of(&value["id"]).unwrap_or_default(),
        self_url: text_of(&value["self"]).unwrap_or_default(),
        summary: text_of(&data["summary"]).unwrap_or_default(),
        description: Body::of(&data["description"]),
        type_name: text_of(&data["issuetype"]["name"]).unwrap_or_default(),
        status_name: text_of(&data["status"]["name"]).unwrap_or_default(),
        priority: text_of(&data["priority"]["name"]),
        assignee: user(&data["assignee"]),
        reporter: user(&data["reporter"]),
        labels: names(&data["labels"]),
        components: names(&data["components"]),
        fix_versions: names(&data["fixVersions"]),
        sprints,
        points,
        due_date: data["duedate"].as_str().and_then(day),
        estimate_seconds: data["timeoriginalestimate"].as_i64().filter(|seconds| *seconds > 0),
        parent_key: text_of(&data["parent"]["key"]).or(epic_link),
        updated: instant(&updated_raw).unwrap_or_else(|| created.clone()),
        updated_raw,
        resolved: data["resolutiondate"].as_str().and_then(instant),
        comment_total: total(&data["comment"]["total"], comments.len()),
        worklog_total: total(&data["worklog"]["total"], worklogs.len()),
        history_total: total(&value["changelog"]["total"], history.len()),
        comments,
        worklogs,
        links,
        attachments,
        history,
        created,
        key,
        number,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn instants_are_stored_in_utc() {
        assert_eq!(instant("2024-03-12T09:15:30.000+0100").as_deref(), Some("2024-03-12T08:15:30.000Z"));
        assert_eq!(instant("2024-03-12T09:15:30.123Z").as_deref(), Some("2024-03-12T09:15:30.123Z"));
        assert_eq!(instant("2024-03-12T09:15:30.000-05:00").as_deref(), Some("2024-03-12T14:15:30.000Z"));
        assert_eq!(instant("2024-03-12 09:15").as_deref(), Some("2024-03-12T09:15:00.000Z"));
        assert_eq!(instant("2024-03-12").as_deref(), Some("2024-03-12T00:00:00.000Z"));
        assert_eq!(instant("yesterday"), None);
        assert_eq!(day("2024-03-12T09:15:30.000+0100").as_deref(), Some("2024-03-12"));
        assert_eq!(day("12/03/2024"), None);
    }

    #[test]
    fn the_custom_fields_are_found_by_what_they_are() {
        let fields = Fields::discover(&json!([
            { "id": "summary", "name": "Summary", "custom": false },
            { "id": "customfield_10016", "name": "Story point estimate", "custom": true, "schema": { "type": "number" } },
            { "id": "customfield_10026", "name": "Story Points", "custom": true, "schema": { "type": "number" } },
            { "id": "customfield_10020", "name": "Sprint", "custom": true, "schema": { "custom": "com.pyxis.greenhopper.jira:gh-sprint" } },
            { "id": "customfield_10014", "name": "Epic Link", "custom": true, "schema": { "custom": "com.pyxis.greenhopper.jira:gh-epic-link" } },
        ]));

        assert_eq!(fields.points, vec!["customfield_10026", "customfield_10016"]);
        assert_eq!(fields.sprint.as_deref(), Some("customfield_10020"));
        assert_eq!(fields.epic_link.as_deref(), Some("customfield_10014"));
        assert_eq!(Fields::discover(&json!({ "not": "a list" })), Fields::default());
    }

    #[test]
    fn a_server_sprint_is_read_from_its_text() {
        let text = "com.atlassian.greenhopper.service.sprint.Sprint@1f2e[id=7,rapidViewId=2,state=CLOSED,name=Sprint 3, the long one,goal=Ship, then rest,startDate=2024-03-01T10:00:00.000+01:00,endDate=2024-03-15T10:00:00.000+01:00,completeDate=2024-03-15T12:30:00.000+01:00,activatedDate=<null>,sequence=7]";
        let sprint = sprint(&json!(text)).unwrap();

        assert_eq!((sprint.id.as_str(), sprint.name.as_str(), sprint.state.as_str()), ("7", "Sprint 3, the long one", "closed"));
        assert_eq!(sprint.goal, "Ship, then rest");
        assert_eq!(sprint.start.as_deref(), Some("2024-03-01T09:00:00.000Z"));
        assert_eq!(sprint.completed.as_deref(), Some("2024-03-15T11:30:00.000Z"));
        assert_eq!(super::sprint(&json!("Sprint@1[id=9,state=FUTURE,name=Next,startDate=<null>,endDate=<null>]")).unwrap().start, None);
        assert_eq!(super::sprint(&json!("garbage")), None);
    }
}
