//! Time logged on an issue.

use hub_core::{Id, Worklog};
use rusqlite::Connection;
use serde_json::json;

use super::current::load_current;
use crate::access::{self};
use crate::app::Tx;
use crate::auth::AuthUser;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};

fn map_worklog(row: &rusqlite::Row) -> rusqlite::Result<Worklog> {
    Ok(Worklog { id: row.get(0)?, issue_id: row.get(1)?, user_id: row.get(2)?, minutes: row.get(3)?, started_at: row.get(4)?, note: row.get(5)? })
}

pub fn load_worklogs(connection: &Connection, issue_id: Id) -> ApiResult<Vec<Worklog>> {
    query_all(
        connection,
        "SELECT id, issue_id, user_id, minutes, started_at, note FROM worklog WHERE issue_id = ?1 ORDER BY started_at, id",
        [issue_id],
        map_worklog,
    )
}

/// Logs time spent by the transaction's actor.
pub fn add_worklog(tx: &Tx, issue_id: Id, minutes: i64, started_at: Option<&str>, note: &str) -> ApiResult<Worklog> {
    if !(1..=24 * 60 * 31).contains(&minutes) {
        return Err(ApiError::bad_request("Time spent must be between a minute and a month"));
    }

    let current = load_current(tx, issue_id)?;
    let started_at = started_at.map(str::trim).filter(|text| !text.is_empty()).unwrap_or(&tx.now);

    tx.execute(
        "INSERT INTO worklog (issue_id, user_id, minutes, started_at, note) VALUES (?1, ?2, ?3, ?4, ?5)",
        (issue_id, tx.actor.user_id, minutes, started_at, note.trim()),
    )?;
    let id = tx.last_insert_rowid();
    tx.execute("UPDATE issue SET updated_at = ?1 WHERE id = ?2", (&tx.now, issue_id))?;
    tx.emit("worklog.added", Some(current.project_id), Some(issue_id), json!({ "key": current.key, "minutes": minutes }))?;

    Ok(tx.query_row("SELECT id, issue_id, user_id, minutes, started_at, note FROM worklog WHERE id = ?1", [id], map_worklog)?)
}

pub fn delete_worklog(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<()> {
    let (issue_id, owner): (Id, Option<Id>) =
        query_one(tx, "SELECT issue_id, user_id FROM worklog WHERE id = ?1", [id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .ok_or_else(|| ApiError::not_found("Work log"))?;
    let current = load_current(tx, issue_id)?;
    let access = access::project_access_by_id(tx, user, current.project_id)?;

    if owner != Some(user.id) && access.require_admin().is_err() {
        return Err(ApiError::forbidden("You can only remove your own work logs"));
    }

    tx.execute("DELETE FROM worklog WHERE id = ?1", [id])?;
    tx.emit("issue.updated", Some(current.project_id), Some(issue_id), json!({ "key": current.key, "fields": ["worklog"] }))
}
