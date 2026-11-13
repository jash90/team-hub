//! The tools the tracker itself offers to agents. Each goes through the same services as
//! the web interface, as the person whose token the agent carries: their permissions apply,
//! and history, notifications and events come out the same.

use hub_app::access::{project_access, visible_projects};
use hub_app::services::{issues, projects, search, users};
use hub_app::{ApiError, ApiResult, App, AuthUser, ToolSpec};
use hub_core::text::{format_duration, parse_duration};
use hub_core::{
    double_option, priority_name, Id, Issue, IssueLink, IssuePatch, LinkDirection, LinkType, NewIssue, SprintState, StatusCategory, PRIORITY_NAMES,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::names::{self, Names};

const DEFAULT_SEARCH_LIMIT: i64 = 20;
const MAX_SEARCH_LIMIT: i64 = 100;
const MAX_MY_WORK: i64 = 100;

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

fn issue_key() -> Value {
    json!({ "type": "string", "description": "Issue key, e.g. WEB-12" })
}

fn priority_schema() -> Value {
    json!({ "type": "string", "enum": PRIORITY_NAMES, "description": "Defaults to Medium on a new issue" })
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list_projects",
            description: "List the projects you can see, with their keys and how many issues are open.",
            input_schema: schema(json!({}), &[]),
            read_only: true,
        },
        ToolSpec {
            name: "get_project",
            description: "Describe one project: its statuses, issue types, open sprints, versions, components, labels and the people issues can be \
                          assigned to. Call this before creating or changing issues to learn the names the other tools accept.",
            input_schema: schema(json!({ "key": { "type": "string", "description": "Project key, e.g. WEB" } }), &["key"]),
            read_only: true,
        },
        ToolSpec {
            name: "search_issues",
            description: "Find issues with the TQL query language, or with plain words for a full-text search. \
                          Example: project = WEB AND status in (\"To Do\", \"In Progress\") AND assignee = currentUser() ORDER BY priority DESC. \
                          Fields: project, key, type, status, category (todo, in_progress, done), priority, assignee, reporter, label, component, sprint, \
                          version, parent, points, text, summary, description, comment, created, updated, resolved, due. \
                          Operators: = != ~ !~ < <= > >= in, not in, is empty, is not empty, combined with AND, OR, NOT and parentheses. \
                          Functions: currentUser(), openSprints(), now(), startOfDay(), startOfWeek(), startOfMonth(); dates also accept 2026-11-30 or -7d.",
            input_schema: schema(
                json!({
                    "query": { "type": "string", "description": "TQL, or plain words" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_SEARCH_LIMIT, "description": "How many issues to return; 20 unless given" },
                }),
                &["query"],
            ),
            read_only: true,
        },
        ToolSpec {
            name: "get_issue",
            description: "Read one issue in full: its fields, description, comments, links, subtasks and what other modules know about it \
                          (for example pull requests).",
            input_schema: schema(json!({ "key": issue_key() }), &["key"]),
            read_only: true,
        },
        ToolSpec {
            name: "create_issue",
            description: "Create an issue. Without a type it becomes a task, or a subtask when parentKey names a story, task or bug. \
                          A story, task or bug can have an epic as its parent.",
            input_schema: schema(
                json!({
                    "projectKey": { "type": "string", "description": "Project key, e.g. WEB" },
                    "summary": { "type": "string", "description": "One line saying what this is about" },
                    "description": { "type": "string", "description": "Details, in Markdown" },
                    "type": { "type": "string", "description": "Issue type by name, e.g. Bug; get_project lists them" },
                    "priority": priority_schema(),
                    "assignee": { "type": "string", "description": "A person's full name or e-mail, or \"me\"" },
                    "labels": { "type": "array", "items": { "type": "string" } },
                    "parentKey": { "type": "string", "description": "Key of the epic this belongs to, or of the issue this is a subtask of" },
                    "points": { "type": "number", "minimum": 0, "description": "Story points" },
                }),
                &["projectKey", "summary"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: "update_issue",
            description: "Change fields of an issue; fields you leave out stay as they are. Use transition_issue to change the status. \
                          assignee, points, dueDate, sprint and version can be null to clear them.",
            input_schema: schema(
                json!({
                    "key": issue_key(),
                    "summary": { "type": "string" },
                    "description": { "type": "string", "description": "Replaces the description; Markdown" },
                    "priority": priority_schema(),
                    "assignee": { "type": ["string", "null"], "description": "A person's full name or e-mail, or \"me\"; null to unassign" },
                    "labels": { "type": "array", "items": { "type": "string" }, "description": "Replaces all labels" },
                    "points": { "type": ["number", "null"], "minimum": 0 },
                    "dueDate": { "type": ["string", "null"], "description": "YYYY-MM-DD" },
                    "sprint": { "type": ["string", "null"], "description": "An open sprint by name, or \"active\" for the running one; null for the backlog" },
                    "version": { "type": ["string", "null"], "description": "A version by name" },
                }),
                &["key"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: "transition_issue",
            description: "Move an issue to another status, by the status's name. The words \"start\", \"done\" and \"reopen\" pick the first status that is \
                          in progress, done or to do.",
            input_schema: schema(json!({ "key": issue_key(), "status": { "type": "string", "description": "Status name, e.g. In Review" } }), &["key", "status"]),
            read_only: false,
        },
        ToolSpec {
            name: "add_comment",
            description: "Comment on an issue. Markdown; @first.last mentions notify that person.",
            input_schema: schema(json!({ "key": issue_key(), "body": { "type": "string" } }), &["key", "body"]),
            read_only: false,
        },
        ToolSpec {
            name: "log_work",
            description: "Record time you spent on an issue.",
            input_schema: schema(
                json!({
                    "key": issue_key(),
                    "spent": { "type": "string", "description": "A duration such as \"1h 30m\", \"45m\" or \"2d\" (a day is 8 hours)" },
                    "note": { "type": "string", "description": "What the time went into" },
                }),
                &["key", "spent"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: "my_work",
            description: "The open issues assigned to you, in progress first, most important first.",
            input_schema: schema(json!({}), &[]),
            read_only: true,
        },
    ]
}

fn parse<T: DeserializeOwned>(arguments: &Value) -> ApiResult<T> {
    T::deserialize(arguments).map_err(|error| ApiError::bad_request(format!("Invalid arguments: {error}")))
}

/// Runs one of the tools above; none when `name` is not one of them.
pub fn call(app: &App, user: &AuthUser, name: &str, arguments: &Value) -> Option<ApiResult<Value>> {
    Some(match name {
        "list_projects" => list_projects(app, user),
        "get_project" => parse(arguments).and_then(|input| get_project(app, user, input)),
        "search_issues" => parse(arguments).and_then(|input| search_issues(app, user, input)),
        "get_issue" => parse(arguments).and_then(|input| get_issue(app, user, input)),
        "create_issue" => parse(arguments).and_then(|input| create_issue(app, user, input)),
        "update_issue" => parse(arguments).and_then(|input| update_issue(app, user, input)),
        "transition_issue" => parse(arguments).and_then(|input| transition_issue(app, user, input)),
        "add_comment" => parse(arguments).and_then(|input| add_comment(app, user, input)),
        "log_work" => parse(arguments).and_then(|input| log_work(app, user, input)),
        "my_work" => my_work(app, user),
        _ => return None,
    })
}

#[derive(Deserialize)]
struct KeyInput {
    key: String,
}

fn list_projects(app: &App, user: &AuthUser) -> ApiResult<Value> {
    let projects = app.read(|connection| projects::list_projects(connection, user))?;

    Ok(projects
        .iter()
        .map(|project| {
            json!({
                "key": project.key,
                "name": project.name,
                "description": project.description,
                "boardMode": project.board_mode,
                "role": project.role,
                "openIssues": project.open_count,
                "totalIssues": project.issue_count,
                "archived": project.archived,
            })
        })
        .collect())
}

fn get_project(app: &App, user: &AuthUser, input: KeyInput) -> ApiResult<Value> {
    app.read(|connection| {
        let project = projects::get_project(connection, user, &input.key)?;
        let lead = users::user_name(connection, project.summary.lead_id)?;
        let open_sprints: Vec<Value> = project
            .sprints
            .iter()
            .filter(|sprint| sprint.state != SprintState::Closed)
            .map(|sprint| json!({ "name": sprint.name, "state": sprint.state, "startDate": sprint.start_date, "endDate": sprint.end_date, "goal": sprint.goal }))
            .collect();
        let versions: Vec<Value> = project
            .versions
            .iter()
            .filter(|version| !version.archived)
            .map(|version| json!({ "name": version.name, "released": version.released, "releaseDate": version.release_date }))
            .collect();

        Ok(json!({
            "key": project.summary.key,
            "name": project.summary.name,
            "description": project.summary.description,
            "boardMode": project.summary.board_mode,
            "lead": lead,
            "strictWorkflow": project.strict_workflow,
            "statuses": project.statuses.iter().map(|status| json!({ "name": status.name, "category": status.category })).collect::<Vec<_>>(),
            "issueTypes": project.types.iter().map(|issue_type| json!({ "name": issue_type.name, "level": issue_type.level })).collect::<Vec<_>>(),
            "priorities": PRIORITY_NAMES,
            "openSprints": open_sprints,
            "versions": versions,
            "components": project.components.iter().map(|component| component.name.as_str()).collect::<Vec<_>>(),
            "labels": project.labels,
            "people": names::active_people(connection)?,
        }))
    })
}

#[derive(Deserialize)]
struct SearchInput {
    query: String,
    limit: Option<i64>,
}

fn search_issues(app: &App, user: &AuthUser, input: SearchInput) -> ApiResult<Value> {
    let limit = input.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, MAX_SEARCH_LIMIT);
    let result = search::search(app, user, &input.query, limit, 0)?;

    if let Some(error) = result.error {
        return Err(ApiError::bad_request(format!("The query could not be understood at character {}: {}", error.position, error.message)));
    }

    let names = app.read(Names::load)?;

    Ok(json!({ "total": result.total, "issues": result.issues.iter().map(|row| names.row(row)).collect::<Vec<_>>() }))
}

fn relation(link: &IssueLink) -> &'static str {
    match (link.link_type, link.direction) {
        (LinkType::Blocks, LinkDirection::Outward) => "blocks",
        (LinkType::Blocks, LinkDirection::Inward) => "is blocked by",
        (LinkType::Duplicates, LinkDirection::Outward) => "duplicates",
        (LinkType::Duplicates, LinkDirection::Inward) => "is duplicated by",
        (LinkType::Relates, _) => "relates to",
    }
}

fn issue_json(app: &App, names: &Names, issue: &Issue) -> Value {
    let row = &issue.row;
    let duration = |minutes: Option<i64>| minutes.filter(|minutes| *minutes > 0).map(format_duration);
    let parent = row.parent_key.as_ref().map(|key| json!({ "key": key, "summary": row.parent_summary }));
    let comments: Vec<Value> = issue
        .comments
        .iter()
        .map(|comment| json!({ "author": names.user(comment.author_id).or(comment.author_label.as_deref()), "createdAt": comment.created_at, "body": comment.body }))
        .collect();
    let links: Vec<Value> = issue
        .links
        .iter()
        .map(|link| json!({ "relation": relation(link), "key": link.issue.key, "summary": link.issue.summary, "status": names.status(link.issue.status_id) }))
        .collect();
    let subtasks: Vec<Value> = issue
        .children
        .iter()
        .map(|child| json!({ "key": child.key, "summary": child.summary, "status": names.status(child.status_id), "assignee": names.user(child.assignee_id) }))
        .collect();

    json!({
        "key": row.key,
        "url": format!("{}/issue/{}", app.config.base_url.trim_end_matches('/'), row.key),
        "summary": row.summary,
        "type": names.issue_type(row.type_id),
        "status": names.status(row.status_id),
        "statusCategory": names.category(row.status_id),
        "priority": priority_name(row.priority),
        "assignee": names.user(row.assignee_id),
        "reporter": names.user(row.reporter_id),
        "sprint": names.sprint(row.sprint_id),
        "version": names.version(row.version_id),
        "points": row.points,
        "dueDate": row.due_date,
        "labels": row.labels,
        "components": names.components(&row.component_ids),
        "parent": parent,
        "estimate": duration(issue.estimate_minutes),
        "timeSpent": duration(Some(issue.spent_minutes)),
        "createdAt": row.created_at,
        "updatedAt": row.updated_at,
        "resolvedAt": row.resolved_at,
        "description": issue.description,
        "subtasks": subtasks,
        "links": links,
        "comments": comments,
        "panels": issue.panels.iter().filter(|(module, _)| module.as_str() != crate::catalog::OWN_MODULE).collect::<std::collections::BTreeMap<_, _>>(),
    })
}

fn get_issue(app: &App, user: &AuthUser, input: KeyInput) -> ApiResult<Value> {
    let issue = issues::issue_detail(app, user, &input.key)?;
    let names = app.read(Names::load)?;

    Ok(issue_json(app, &names, &issue))
}

/// The issue as a list row, read inside the transaction that changed it.
fn changed_row(app: &App, connection: &Connection, id: Id) -> ApiResult<Value> {
    Ok(Names::load(connection)?.row(&issues::row_by_id(app, connection, id)?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateInput {
    project_key: String,
    summary: String,
    description: Option<String>,
    #[serde(rename = "type")]
    issue_type: Option<String>,
    priority: Option<Value>,
    assignee: Option<String>,
    labels: Option<Vec<String>>,
    parent_key: Option<String>,
    points: Option<f64>,
}

fn create_issue(app: &App, user: &AuthUser, input: CreateInput) -> ApiResult<Value> {
    app.write(user.actor(), |tx| {
        let access = project_access(tx, user, &input.project_key)?;
        access.require_edit()?;

        let fields = IssuePatch {
            summary: Some(input.summary.clone()),
            description: input.description.clone(),
            type_id: input.issue_type.as_deref().map(|name| names::type_id(tx, access.id, name)).transpose()?,
            priority: input.priority.as_ref().map(names::priority).transpose()?,
            assignee_id: input.assignee.as_deref().map(|name| names::user_id(tx, user, name)).transpose()?.map(Some),
            labels: input.labels.clone(),
            parent_key: input.parent_key.clone().map(Some),
            points: input.points.map(Some),
            ..Default::default()
        };
        let id = issues::create_issue(tx, &NewIssue { project_key: access.key.clone(), fields })?;

        changed_row(app, tx, id)
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInput {
    key: String,
    summary: Option<String>,
    description: Option<String>,
    priority: Option<Value>,
    #[serde(default, deserialize_with = "double_option")]
    assignee: Option<Option<String>>,
    labels: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double_option")]
    points: Option<Option<f64>>,
    #[serde(default, deserialize_with = "double_option")]
    due_date: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    sprint: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    version: Option<Option<String>>,
}

/// A field that may be left alone, cleared (null or blank) or set by name.
fn named<T>(field: &Option<Option<String>>, resolve: impl FnOnce(&str) -> ApiResult<T>) -> ApiResult<Option<Option<T>>> {
    let Some(name) = field else { return Ok(None) };

    match name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => Ok(Some(Some(resolve(name)?))),
        None => Ok(Some(None)),
    }
}

fn update_issue(app: &App, user: &AuthUser, input: UpdateInput) -> ApiResult<Value> {
    app.write(user.actor(), |tx| {
        let (id, access) = issues::issue_access(tx, user, &input.key)?;
        access.require_edit()?;

        let patch = IssuePatch {
            summary: input.summary.clone(),
            description: input.description.clone(),
            priority: input.priority.as_ref().map(names::priority).transpose()?,
            assignee_id: named(&input.assignee, |name| names::user_id(tx, user, name))?,
            labels: input.labels.clone(),
            points: input.points,
            due_date: input.due_date.clone(),
            sprint_id: named(&input.sprint, |name| names::sprint_id(tx, access.id, name))?,
            version_id: named(&input.version, |name| names::version_id(tx, access.id, name))?,
            ..Default::default()
        };
        let changed = issues::update_issue(tx, id, &patch)?;
        let mut row = changed_row(app, tx, id)?;
        row["changed"] = json!(changed);

        Ok(row)
    })
}

#[derive(Deserialize)]
struct TransitionInput {
    key: String,
    status: String,
}

fn transition_issue(app: &App, user: &AuthUser, input: TransitionInput) -> ApiResult<Value> {
    app.write(user.actor(), |tx| {
        let (id, access) = issues::issue_access(tx, user, &input.key)?;
        access.require_edit()?;

        let status_id = names::status_id(tx, access.id, &input.status)?;
        issues::update_issue(tx, id, &IssuePatch { status_id: Some(status_id), ..Default::default() })?;

        changed_row(app, tx, id)
    })
}

#[derive(Deserialize)]
struct CommentInput {
    key: String,
    body: String,
}

fn add_comment(app: &App, user: &AuthUser, input: CommentInput) -> ApiResult<Value> {
    app.write(user.actor(), |tx| {
        let (id, access) = issues::issue_access(tx, user, &input.key)?;
        access.require_edit()?;

        let comment = issues::add_comment(tx, id, &input.body)?;
        let row = issues::row_by_id(app, tx, id)?;

        Ok(json!({ "key": row.key, "commentId": comment.id, "createdAt": comment.created_at, "comments": row.comment_count }))
    })
}

#[derive(Deserialize)]
struct WorkInput {
    key: String,
    spent: String,
    note: Option<String>,
}

fn log_work(app: &App, user: &AuthUser, input: WorkInput) -> ApiResult<Value> {
    let minutes = parse_duration(&input.spent).ok_or_else(|| ApiError::bad_request("Time spent looks like \"1h 30m\", \"45m\" or \"2d\""))?;

    app.write(user.actor(), |tx| {
        let (id, access) = issues::issue_access(tx, user, &input.key)?;
        access.require_edit()?;

        let worklog = issues::add_worklog(tx, id, minutes, None, input.note.as_deref().unwrap_or(""))?;
        let total: i64 = issues::load_worklogs(tx, id)?.iter().map(|worklog| worklog.minutes).sum();
        let row = issues::row_by_id(app, tx, id)?;

        Ok(json!({ "key": row.key, "logged": format_duration(worklog.minutes), "totalSpent": format_duration(total) }))
    })
}

fn my_work(app: &App, user: &AuthUser) -> ApiResult<Value> {
    let (visible, mut params) = visible_projects(user, "i.project_id");
    params.push(SqlValue::Integer(user.id));
    params.push(SqlValue::Integer(MAX_MY_WORK));
    let tail = format!(
        "WHERE {visible} AND i.assignee_id = ? AND i.status_id IN (SELECT id FROM status WHERE category != 'done')
         ORDER BY i.priority, i.updated_at DESC LIMIT ?"
    );

    app.read(|connection| {
        let rows = issues::load_rows(app, connection, &tail, params)?;
        let names = Names::load(connection)?;
        let in_category = |category: StatusCategory| {
            rows.iter().filter(|row| names.category(row.status_id) == category).map(|row| names.row(row)).collect::<Vec<_>>()
        };

        Ok(json!({ "inProgress": in_category(StatusCategory::InProgress), "todo": in_category(StatusCategory::Todo) }))
    })
}
