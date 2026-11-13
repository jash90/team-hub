//! Comments on an issue.

use hub_core::text::truncate;
use hub_core::{Comment, Id};
use rusqlite::Connection;
use serde_json::json;

use super::current::{load_current, Current};
use super::index::reindex_issue;
use super::watchers::watch;
use crate::access::{self};
use crate::app::Tx;
use crate::auth::AuthUser;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::notifications::{notify_mentions, notify_watchers};
use crate::services::users::user_name;

fn map_comment(row: &rusqlite::Row) -> rusqlite::Result<Comment> {
    Ok(Comment {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        author_id: row.get(2)?,
        author_label: row.get(3)?,
        body: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

const COMMENT_COLUMNS: &str = "id, issue_id, author_id, author_label, body, created_at, updated_at";

pub fn load_comments(connection: &Connection, issue_id: Id) -> ApiResult<Vec<Comment>> {
    query_all(connection, &format!("SELECT {COMMENT_COLUMNS} FROM comment WHERE issue_id = ?1 ORDER BY id"), [issue_id], map_comment)
}

fn get_comment(connection: &Connection, id: Id) -> ApiResult<Comment> {
    query_one(connection, &format!("SELECT {COMMENT_COLUMNS} FROM comment WHERE id = ?1"), [id], map_comment)?
        .ok_or_else(|| ApiError::not_found("Comment"))
}

/// Adds a comment written by the transaction's actor.
pub fn add_comment(tx: &Tx, issue_id: Id, body: &str) -> ApiResult<Comment> {
    let body = body.trim();

    if body.is_empty() {
        return Err(ApiError::bad_request("The comment is empty"));
    }

    let current = load_current(tx, issue_id)?;

    tx.execute(
        "INSERT INTO comment (issue_id, author_id, author_label, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        (issue_id, tx.actor.user_id, &tx.actor.label, body, &tx.now),
    )?;
    let comment = get_comment(tx, tx.last_insert_rowid())?;
    let author = user_name(tx, tx.actor.user_id)?.or_else(|| tx.actor.label.clone()).unwrap_or_else(|| "Someone".to_string());

    tx.execute("UPDATE issue SET updated_at = ?1 WHERE id = ?2", (&tx.now, issue_id))?;
    let mentioned = notify_mentions(tx, issue_id, body, &format!("{author} mentioned you in {}: {}", current.key, truncate(body, 120)))?;
    notify_watchers(tx, "comment", issue_id, &format!("{author} commented on {}: {}", current.key, truncate(body, 120)), &mentioned)?;
    watch(tx, issue_id, tx.actor.user_id)?;
    reindex_issue(tx, issue_id)?;
    tx.emit("comment.created", Some(current.project_id), Some(issue_id), json!({ "key": current.key, "commentId": comment.id }))?;

    Ok(comment)
}

fn own_comment(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(Comment, Current)> {
    let comment = get_comment(connection, id)?;
    let current = load_current(connection, comment.issue_id)?;
    let access = access::project_access_by_id(connection, user, current.project_id)?;

    if comment.author_id != Some(user.id) && access.require_admin().is_err() {
        return Err(ApiError::forbidden("You can only change your own comments"));
    }

    Ok((comment, current))
}

pub fn update_comment(tx: &Tx, user: &AuthUser, id: Id, body: &str) -> ApiResult<Comment> {
    let (comment, current) = own_comment(tx, user, id)?;
    let body = body.trim();

    if body.is_empty() {
        return Err(ApiError::bad_request("The comment is empty"));
    }

    tx.execute("UPDATE comment SET body = ?1, updated_at = ?2 WHERE id = ?3", (body, &tx.now, id))?;
    reindex_issue(tx, comment.issue_id)?;
    tx.emit("comment.updated", Some(current.project_id), Some(current.id), json!({ "key": current.key, "commentId": id }))?;

    get_comment(tx, id)
}

pub fn delete_comment(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<()> {
    let (comment, current) = own_comment(tx, user, id)?;

    tx.execute("DELETE FROM comment WHERE id = ?1", [id])?;
    reindex_issue(tx, comment.issue_id)?;
    tx.emit("comment.deleted", Some(current.project_id), Some(current.id), json!({ "key": current.key, "commentId": id }))
}
