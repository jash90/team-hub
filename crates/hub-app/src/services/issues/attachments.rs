//! Files attached to an issue.

use std::path::PathBuf;

use hub_core::{Attachment, Id};
use rusqlite::Connection;
use serde_json::json;

use super::current::load_current;
use crate::access::{self};
use crate::app::{App, Tx};
use crate::auth::AuthUser;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::history::{record_change, FieldChange};

const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

fn map_attachment(row: &rusqlite::Row) -> rusqlite::Result<Attachment> {
    Ok(Attachment {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        filename: row.get(2)?,
        mime: row.get(3)?,
        size: row.get(4)?,
        uploader_id: row.get(5)?,
        created_at: row.get(6)?,
    })
}

const ATTACHMENT_COLUMNS: &str = "id, issue_id, filename, mime, size, uploader_id, created_at";

pub fn load_attachments(connection: &Connection, issue_id: Id) -> ApiResult<Vec<Attachment>> {
    query_all(connection, &format!("SELECT {ATTACHMENT_COLUMNS} FROM attachment WHERE issue_id = ?1 ORDER BY id"), [issue_id], map_attachment)
}

fn safe_filename(name: &str) -> String {
    let name: String = name.rsplit(['/', '\\']).next().unwrap_or("").chars().filter(|character| !character.is_control()).take(120).collect();

    if name.trim_matches('.').trim().is_empty() {
        "file".to_string()
    } else {
        name
    }
}

/// Stores an uploaded file under the attachments directory and records it on the issue.
pub fn add_attachment(app: &App, tx: &Tx, issue_id: Id, filename: &str, mime: &str, bytes: &[u8]) -> ApiResult<Attachment> {
    if bytes.is_empty() {
        return Err(ApiError::bad_request("The file is empty"));
    }

    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(ApiError::bad_request("Attachments can be at most 25 MB"));
    }

    let current = load_current(tx, issue_id)?;
    let filename = safe_filename(filename);
    let stored = format!("{}-{}", &crate::auth::random_token()?[..16], issue_id);
    let directory = app.config.attachments_dir();

    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join(&stored), bytes)?;

    tx.execute(
        "INSERT INTO attachment (issue_id, filename, mime, size, path, uploader_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (issue_id, &filename, mime, bytes.len() as i64, &stored, tx.actor.user_id, &tx.now),
    )?;
    let id = tx.last_insert_rowid();

    tx.execute("UPDATE issue SET updated_at = ?1 WHERE id = ?2", (&tx.now, issue_id))?;
    record_change(tx, issue_id, &FieldChange::text("attachment", None, Some(filename)))?;
    tx.emit("issue.updated", Some(current.project_id), Some(issue_id), json!({ "key": current.key, "fields": ["attachments"] }))?;

    Ok(tx.query_row(&format!("SELECT {ATTACHMENT_COLUMNS} FROM attachment WHERE id = ?1"), [id], map_attachment)?)
}

/// An attachment and where its bytes are, for a caller who may see the issue.
pub fn attachment_file(app: &App, connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(Attachment, PathBuf)> {
    let (attachment, path): (Attachment, String) =
        query_one(connection, &format!("SELECT {ATTACHMENT_COLUMNS}, path FROM attachment WHERE id = ?1"), [id], |row| {
            Ok((map_attachment(row)?, row.get(7)?))
        })?
        .ok_or_else(|| ApiError::not_found("Attachment"))?;
    let current = load_current(connection, attachment.issue_id)?;
    access::project_access_by_id(connection, user, current.project_id).map_err(|_| ApiError::not_found("Attachment"))?;

    Ok((attachment, app.config.attachments_dir().join(path)))
}

pub fn delete_attachment(app: &App, tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<PathBuf> {
    let (attachment, path) = attachment_file(app, tx, user, id)?;
    let current = load_current(tx, attachment.issue_id)?;
    access::project_access_by_id(tx, user, current.project_id)?.require_edit()?;

    tx.execute("DELETE FROM attachment WHERE id = ?1", [id])?;
    record_change(tx, attachment.issue_id, &FieldChange::text("attachment", Some(attachment.filename), None))?;
    tx.emit("issue.updated", Some(current.project_id), Some(current.id), json!({ "key": current.key, "fields": ["attachments"] }))?;

    Ok(path)
}
