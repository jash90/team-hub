//! Timeline: a project's epics as bars on a time axis. A bar runs from the epic's start date,
//! which this module keeps (`timeline_span`), to its due date, which is the tracker's own.
//! An end nobody set is estimated from the epic's children (`model`).

mod field;
pub mod model;
mod routes;
mod schema;
mod seed;
pub mod span;
pub mod types;

use axum::Router;
use hub_app::db::query_one;
use hub_app::{ApiError, ApiResult, App, AuthUser, FieldSql, Module, ModuleMeta, QueryScope, ToolSpec};
use hub_core::query::{Clause, QueryFieldInfo};
use hub_core::{IssueRow, TypeLevel};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::types::{TimelineEpic, TimelineProject};

pub struct TimelineModule;

/// The span of an issue that has a start of its own or is an epic; other issues get no panel.
fn issue_panel(connection: &Connection, issue: &IssueRow) -> ApiResult<Option<Value>> {
    let level: Option<String> = query_one(connection, "SELECT level FROM issue_type WHERE id = ?1", [issue.type_id], |row| row.get(0))?;
    let row = model::row(connection, issue.id)?;
    let own_start = row.span.start.is_some() && !row.span.derived.start;

    if !own_start && level.as_deref() != Some(TypeLevel::Epic.as_str()) {
        return Ok(None);
    }

    Ok(Some(serde_json::to_value(row.span)?))
}

/// An epic as an agent reads it: what marks an estimate is only there when something is one.
fn brief(epic: &TimelineEpic) -> Value {
    let estimated: Vec<&str> = [("start", epic.span.derived.start), ("due", epic.span.derived.due)]
        .into_iter()
        .filter(|(_, derived)| *derived)
        .map(|(end, _)| end)
        .collect();
    let mut brief = json!({
        "key": epic.key,
        "summary": epic.summary,
        "status": epic.status.name,
        "category": epic.status.category,
        "assigneeId": epic.assignee_id,
        "start": epic.span.start,
        "due": epic.span.due,
        "issues": { "todo": epic.counts.todo, "inProgress": epic.counts.in_progress, "done": epic.counts.done },
        "points": { "done": epic.points_done, "total": epic.points_total },
        "children": epic.children.iter().map(|child| json!({ "key": child.key, "summary": child.summary, "status": child.status.name, "start": child.start, "due": child.due })).collect::<Vec<_>>(),
    });

    if !estimated.is_empty() {
        brief["estimated"] = json!(estimated);
    }

    brief
}

fn compact(timeline: &TimelineProject) -> Value {
    json!({
        "projectKey": timeline.project_key,
        "epics": timeline.epics.iter().map(brief).collect::<Vec<_>>(),
        "sprints": timeline.sprints.iter().map(|sprint| json!({ "name": sprint.name, "state": sprint.state, "start": sprint.start, "end": sprint.end })).collect::<Vec<_>>(),
        "versions": timeline.versions.iter().map(|version| json!({ "name": version.name, "released": version.released, "releaseDate": version.release_date })).collect::<Vec<_>>(),
    })
}

impl Module for TimelineModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: "timeline",
            name: "Timeline",
            description: "A project's epics as bars on a time axis, from start date to due date, with sprints and releases behind them.",
            version: env!("CARGO_PKG_VERSION"),
            depends_on: &[],
            default_enabled: true,
        }
    }

    fn migrations(&self) -> &'static [&'static str] {
        schema::MIGRATIONS
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(routes::routes())
    }

    fn query_fields(&self) -> Vec<QueryFieldInfo> {
        vec![QueryFieldInfo::new("start", &["startDate"], "date", "start >= 2026-01-01")]
    }

    fn compile_field(&self, _connection: &Connection, clause: &Clause, scope: &QueryScope) -> Option<ApiResult<FieldSql>> {
        field::is_start(&clause.field).then(|| field::start_condition(clause, scope.now))
    }

    fn issue_panel(&self, _app: &App, connection: &Connection, issue: &IssueRow, _viewer: &AuthUser) -> ApiResult<Option<Value>> {
        issue_panel(connection, issue)
    }

    fn tools(&self) -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "get_timeline",
            description: "The roadmap of a project: its epics in rank order, each with status, start and due date (\"estimated\" names the ends nobody set, worked out from the epic's issues), how many of its issues are to do, in progress and done, story points done and in total, and the issues themselves; plus the project's sprints and versions with their dates. Use it to see what is planned when and how far along it is.",
            input_schema: json!({ "type": "object", "properties": { "project": { "type": "string", "description": "Project key, e.g. WEB" } }, "required": ["project"] }),
            read_only: true,
        }]
    }

    fn call_tool(&self, app: &App, user: &AuthUser, name: &str, arguments: &Value) -> Option<ApiResult<Value>> {
        (name == "get_timeline").then(|| {
            let key = arguments["project"].as_str().ok_or_else(|| ApiError::bad_request("\"project\" is required"))?;

            routes::load_project(app, user, key).map(|timeline| compact(&timeline))
        })
    }

    fn seed_demo(&self, app: &App) -> ApiResult<()> {
        seed::seed_demo(app)
    }
}
