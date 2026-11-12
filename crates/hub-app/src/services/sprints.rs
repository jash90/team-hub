use chrono::{Duration, Utc};
use hub_core::{Id, Sprint, SprintState};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::access::{self, ProjectAccess};
use crate::app::Tx;
use crate::auth::AuthUser;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::history::{clean_date, record_change, FieldChange};

const COLUMNS: &str = "id, project_id, name, goal, state, start_date, end_date, completed_at, position";
const DEFAULT_SPRINT_DAYS: i64 = 14;

fn map_sprint(row: &rusqlite::Row) -> rusqlite::Result<Sprint> {
    let state: String = row.get(4)?;

    Ok(Sprint {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        goal: row.get(3)?,
        state: SprintState::parse(&state).unwrap_or(SprintState::Future),
        start_date: row.get(5)?,
        end_date: row.get(6)?,
        completed_at: row.get(7)?,
        position: row.get(8)?,
    })
}

pub fn load_sprints(connection: &Connection, project_id: Id) -> ApiResult<Vec<Sprint>> {
    query_all(connection, &format!("SELECT {COLUMNS} FROM sprint WHERE project_id = ?1 ORDER BY position, id"), [project_id], map_sprint)
}

pub fn get_sprint(connection: &Connection, id: Id) -> ApiResult<Sprint> {
    query_one(connection, &format!("SELECT {COLUMNS} FROM sprint WHERE id = ?1"), [id], map_sprint)?.ok_or_else(|| ApiError::not_found("Sprint"))
}

pub fn active_sprint(connection: &Connection, project_id: Id) -> ApiResult<Option<Sprint>> {
    query_one(
        connection,
        &format!("SELECT {COLUMNS} FROM sprint WHERE project_id = ?1 AND state = 'active' ORDER BY position, id LIMIT 1"),
        [project_id],
        map_sprint,
    )
}

/// The sprint and the caller's access to its project.
pub fn sprint_access(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(Sprint, ProjectAccess)> {
    let sprint = get_sprint(connection, id)?;
    let access = access::project_access_by_id(connection, user, sprint.project_id)?;

    Ok((sprint, access))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SprintInput {
    pub name: Option<String>,
    pub goal: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

pub fn create_sprint(tx: &Tx, access: &ProjectAccess, input: &SprintInput) -> ApiResult<Sprint> {
    access.require_edit()?;

    let count: i64 = tx.query_row("SELECT count(*) FROM sprint WHERE project_id = ?1", [access.id], |row| row.get(0))?;
    let name = input.name.as_deref().map(str::trim).filter(|name| !name.is_empty()).map(String::from).unwrap_or_else(|| format!("{} Sprint {}", access.key, count + 1));
    let position: i64 = tx.query_row("SELECT coalesce(max(position), 0) + 1 FROM sprint WHERE project_id = ?1", [access.id], |row| row.get(0))?;

    tx.execute(
        "INSERT INTO sprint (project_id, name, goal, state, start_date, end_date, position) VALUES (?1, ?2, ?3, 'future', ?4, ?5, ?6)",
        (
            access.id,
            &name,
            input.goal.as_deref().unwrap_or("").trim(),
            clean_date(input.start_date.as_deref())?,
            clean_date(input.end_date.as_deref())?,
            position,
        ),
    )?;
    let sprint = get_sprint(tx, tx.last_insert_rowid())?;
    tx.emit("sprint.created", Some(access.id), None, json!({ "sprintId": sprint.id, "name": sprint.name }))?;

    Ok(sprint)
}

pub fn update_sprint(tx: &Tx, user: &AuthUser, id: Id, input: &SprintInput) -> ApiResult<Sprint> {
    let (_, access) = sprint_access(tx, user, id)?;
    access.require_edit()?;

    if let Some(name) = input.name.as_deref().map(str::trim) {
        if name.is_empty() {
            return Err(ApiError::bad_request("The sprint needs a name"));
        }

        tx.execute("UPDATE sprint SET name = ?1 WHERE id = ?2", (name, id))?;
    }

    if let Some(goal) = &input.goal {
        tx.execute("UPDATE sprint SET goal = ?1 WHERE id = ?2", (goal.trim(), id))?;
    }

    if let Some(date) = &input.start_date {
        tx.execute("UPDATE sprint SET start_date = ?1 WHERE id = ?2", (clean_date(Some(date))?, id))?;
    }

    if let Some(date) = &input.end_date {
        tx.execute("UPDATE sprint SET end_date = ?1 WHERE id = ?2", (clean_date(Some(date))?, id))?;
    }

    tx.emit("sprint.updated", Some(access.id), None, json!({ "sprintId": id }))?;

    get_sprint(tx, id)
}

/// Starts a sprint. A project runs one sprint at a time.
pub fn start_sprint(tx: &Tx, user: &AuthUser, id: Id, input: &SprintInput) -> ApiResult<Sprint> {
    let (sprint, access) = sprint_access(tx, user, id)?;
    access.require_edit()?;

    if sprint.state != SprintState::Future {
        return Err(ApiError::conflict("Only a sprint that has not started can be started"));
    }

    if let Some(active) = active_sprint(tx, access.id)? {
        return Err(ApiError::conflict(format!("\"{}\" is still running; complete it first", active.name)));
    }

    let today = Utc::now().date_naive();
    let start = clean_date(input.start_date.as_deref())?.or(sprint.start_date).unwrap_or_else(|| today.format("%Y-%m-%d").to_string());
    let end = clean_date(input.end_date.as_deref())?
        .or(sprint.end_date)
        .unwrap_or_else(|| (today + Duration::days(DEFAULT_SPRINT_DAYS)).format("%Y-%m-%d").to_string());

    if end < start {
        return Err(ApiError::bad_request("The sprint cannot end before it starts"));
    }

    tx.execute("UPDATE sprint SET state = 'active', start_date = ?1, end_date = ?2 WHERE id = ?3", (&start, &end, id))?;

    if let Some(goal) = &input.goal {
        tx.execute("UPDATE sprint SET goal = ?1 WHERE id = ?2", (goal.trim(), id))?;
    }

    tx.emit("sprint.started", Some(access.id), None, json!({ "sprintId": id, "name": sprint.name }))?;

    get_sprint(tx, id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SprintCompletion {
    pub sprint: Sprint,
    pub done: i64,
    pub moved: i64,
}

/// Completes a sprint. Unfinished issues move to `move_to` (another sprint that has not been
/// closed) or back to the backlog.
pub fn complete_sprint(tx: &Tx, user: &AuthUser, id: Id, move_to: Option<Id>) -> ApiResult<SprintCompletion> {
    let (sprint, access) = sprint_access(tx, user, id)?;
    access.require_edit()?;

    if sprint.state != SprintState::Active {
        return Err(ApiError::conflict("Only a running sprint can be completed"));
    }

    let target = match move_to {
        Some(target_id) => {
            let target = get_sprint(tx, target_id)?;

            if target.project_id != access.id || target.state == SprintState::Closed || target.id == id {
                return Err(ApiError::bad_request("Unfinished issues can only move to an open sprint of the same project"));
            }

            Some(target)
        }
        None => None,
    };

    let open: Vec<Id> = query_all(
        tx,
        "SELECT i.id FROM issue i JOIN status s ON s.id = i.status_id WHERE i.sprint_id = ?1 AND s.category != 'done'",
        [id],
        |row| row.get(0),
    )?;
    let total: i64 = tx.query_row("SELECT count(*) FROM issue WHERE sprint_id = ?1", [id], |row| row.get(0))?;

    for issue_id in &open {
        tx.execute("UPDATE issue SET sprint_id = ?1, updated_at = ?2 WHERE id = ?3", (target.as_ref().map(|sprint| sprint.id), &tx.now, issue_id))?;
        record_change(
            tx,
            *issue_id,
            &FieldChange::reference("sprint", Some((sprint.id, sprint.name.clone())), target.as_ref().map(|sprint| (sprint.id, sprint.name.clone()))),
        )?;
    }

    tx.execute("UPDATE sprint SET state = 'closed', completed_at = ?1 WHERE id = ?2", (&tx.now, id))?;
    tx.emit(
        "sprint.completed",
        Some(access.id),
        None,
        json!({ "sprintId": id, "name": sprint.name, "done": total - open.len() as i64, "moved": open.len() }),
    )?;

    Ok(SprintCompletion { sprint: get_sprint(tx, id)?, done: total - open.len() as i64, moved: open.len() as i64 })
}

/// Deletes a sprint that has not been closed; its issues return to the backlog.
pub fn delete_sprint(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<()> {
    let (sprint, access) = sprint_access(tx, user, id)?;
    access.require_edit()?;

    if sprint.state == SprintState::Closed {
        return Err(ApiError::conflict("A completed sprint is part of the project's history and cannot be deleted"));
    }

    let issues: Vec<Id> = query_all(tx, "SELECT id FROM issue WHERE sprint_id = ?1", [id], |row| row.get(0))?;

    for issue_id in issues {
        record_change(tx, issue_id, &FieldChange::reference("sprint", Some((sprint.id, sprint.name.clone())), None))?;
    }

    tx.execute("DELETE FROM sprint WHERE id = ?1", [id])?;
    tx.emit("sprint.deleted", Some(access.id), None, json!({ "sprintId": id }))
}
