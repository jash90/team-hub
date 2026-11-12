//! The record of what changed on an issue, and small helpers the services share.

use chrono::NaiveDate;
use hub_core::{Change, Id};
use rusqlite::Connection;

use crate::app::Tx;
use crate::db::query_all;
use crate::error::{ApiError, ApiResult};

/// One changed field. `old`/`new` are what a person reads (a status name, a person's name);
/// the refs are the ids behind them, which reports use.
pub struct FieldChange<'a> {
    pub field: &'a str,
    pub old: Option<String>,
    pub new: Option<String>,
    pub old_ref: Option<String>,
    pub new_ref: Option<String>,
}

impl<'a> FieldChange<'a> {
    pub fn text(field: &'a str, old: Option<String>, new: Option<String>) -> Self {
        Self { field, old, new, old_ref: None, new_ref: None }
    }

    pub fn reference(field: &'a str, old: Option<(Id, String)>, new: Option<(Id, String)>) -> Self {
        Self {
            field,
            old_ref: old.as_ref().map(|(id, _)| id.to_string()),
            new_ref: new.as_ref().map(|(id, _)| id.to_string()),
            old: old.map(|(_, name)| name),
            new: new.map(|(_, name)| name),
        }
    }
}

pub fn record_change(tx: &Tx, issue_id: Id, change: &FieldChange) -> ApiResult<()> {
    tx.execute(
        "INSERT INTO change (issue_id, actor_id, actor_label, field, old_value, new_value, old_ref, new_ref, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        (issue_id, tx.actor.user_id, &tx.actor.label, change.field, &change.old, &change.new, &change.old_ref, &change.new_ref, &tx.now),
    )?;

    Ok(())
}

pub fn load_changes(connection: &Connection, issue_id: Id) -> ApiResult<Vec<Change>> {
    query_all(
        connection,
        "SELECT id, issue_id, actor_id, actor_label, field, old_value, new_value, created_at FROM change WHERE issue_id = ?1 ORDER BY id",
        [issue_id],
        |row| {
            Ok(Change {
                id: row.get(0)?,
                issue_id: row.get(1)?,
                actor_id: row.get(2)?,
                actor_label: row.get(3)?,
                field: row.get(4)?,
                old_value: row.get(5)?,
                new_value: row.get(6)?,
                created_at: row.get(7)?,
            })
        },
    )
}

/// A `YYYY-MM-DD` date, or none for blank input.
pub fn clean_date(text: Option<&str>) -> ApiResult<Option<String>> {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else { return Ok(None) };
    let date = NaiveDate::parse_from_str(&text[..text.len().min(10)], "%Y-%m-%d")
        .map_err(|_| ApiError::bad_request(format!("\"{text}\" is not a date (expected YYYY-MM-DD)")))?;

    Ok(Some(date.format("%Y-%m-%d").to_string()))
}
