//! What an issue is right now, read before changing it, and what its fields refer to.

use hub_core::{Id, StatusCategory, TypeLevel};
use rusqlite::Connection;

use crate::db::query_one;
use crate::error::{ApiError, ApiResult};

pub(super) struct Current {
    pub(super) id: Id,
    pub(super) project_id: Id,
    pub(super) key: String,
    pub(super) type_id: Id,
    pub(super) status_id: Id,
    pub(super) priority: i64,
    pub(super) summary: String,
    pub(super) description: String,
    pub(super) assignee_id: Option<Id>,
    pub(super) reporter_id: Option<Id>,
    pub(super) parent_id: Option<Id>,
    pub(super) sprint_id: Option<Id>,
    pub(super) version_id: Option<Id>,
    pub(super) points: Option<f64>,
    pub(super) due_date: Option<String>,
    pub(super) estimate_minutes: Option<i64>,
}

pub(super) fn load_current(connection: &Connection, id: Id) -> ApiResult<Current> {
    query_one(
        connection,
        "SELECT id, project_id, key, type_id, status_id, priority, summary, description, assignee_id, reporter_id, parent_id,
                sprint_id, version_id, points, due_date, estimate_minutes
         FROM issue WHERE id = ?1",
        [id],
        |row| {
            Ok(Current {
                id: row.get(0)?,
                project_id: row.get(1)?,
                key: row.get(2)?,
                type_id: row.get(3)?,
                status_id: row.get(4)?,
                priority: row.get(5)?,
                summary: row.get(6)?,
                description: row.get(7)?,
                assignee_id: row.get(8)?,
                reporter_id: row.get(9)?,
                parent_id: row.get(10)?,
                sprint_id: row.get(11)?,
                version_id: row.get(12)?,
                points: row.get(13)?,
                due_date: row.get(14)?,
                estimate_minutes: row.get(15)?,
            })
        },
    )?
    .ok_or_else(|| ApiError::not_found("Issue"))
}

pub(super) fn name_in(connection: &Connection, table: &str, id: Option<Id>) -> ApiResult<Option<(Id, String)>> {
    let Some(id) = id else { return Ok(None) };
    let name: Option<String> = query_one(connection, &format!("SELECT name FROM {table} WHERE id = ?1"), [id], |row| row.get(0))?;

    Ok(name.map(|name| (id, name)))
}

pub(super) fn type_level(connection: &Connection, type_id: Id, project_id: Id) -> ApiResult<TypeLevel> {
    let level: Option<String> =
        query_one(connection, "SELECT level FROM issue_type WHERE id = ?1 AND project_id = ?2", (type_id, project_id), |row| row.get(0))?;

    level.as_deref().and_then(TypeLevel::parse).ok_or_else(|| ApiError::bad_request("That issue type does not belong to this project"))
}

pub(super) fn status_category(connection: &Connection, status_id: Id, project_id: Id) -> ApiResult<StatusCategory> {
    let category: Option<String> =
        query_one(connection, "SELECT category FROM status WHERE id = ?1 AND project_id = ?2", (status_id, project_id), |row| row.get(0))?;

    category.as_deref().and_then(StatusCategory::parse).ok_or_else(|| ApiError::bad_request("That status does not belong to this project"))
}

/// The status of `project_id` a word refers to: an exact status name, or failing that a word
/// for a whole category ("done", "start"), which picks that category's first status.
pub fn status_by_name(connection: &Connection, project_id: Id, name: &str) -> ApiResult<Option<Id>> {
    let name = name.trim();
    let exact: Option<Id> =
        query_one(connection, "SELECT id FROM status WHERE project_id = ?1 AND name = ?2 COLLATE NOCASE", (project_id, name), |row| row.get(0))?;

    if exact.is_some() {
        return Ok(exact);
    }

    let category = match name.to_lowercase().as_str() {
        "done" | "close" | "closed" | "resolve" | "resolved" | "fixed" | "finish" => StatusCategory::Done,
        "start" | "started" | "progress" | "in progress" | "doing" | "wip" => StatusCategory::InProgress,
        "todo" | "to do" | "reopen" | "open" => StatusCategory::Todo,
        _ => return Ok(None),
    };

    query_one(
        connection,
        "SELECT id FROM status WHERE project_id = ?1 AND category = ?2 ORDER BY position, id LIMIT 1",
        (project_id, category.as_str()),
        |row| row.get(0),
    )
}
