use hub_core::text::mentioned_handles;
use hub_core::{Id, Notification};
use rusqlite::Connection;

use crate::app::Tx;
use crate::db::{now_iso, query_all};
use crate::error::ApiResult;
use crate::services::users::resolve_user;

/// Tells `user_id` about something on an issue, unless they did it themselves.
pub fn notify(tx: &Tx, user_id: Id, kind: &str, issue_id: Id, text: &str) -> ApiResult<()> {
    if tx.actor.user_id == Some(user_id) {
        return Ok(());
    }

    tx.execute(
        "INSERT INTO notification (user_id, kind, issue_id, actor_id, text, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (user_id, kind, issue_id, tx.actor.user_id, text, &tx.now),
    )?;

    Ok(())
}

pub fn notify_watchers(tx: &Tx, kind: &str, issue_id: Id, text: &str, except: &[Id]) -> ApiResult<()> {
    let watchers: Vec<Id> = query_all(tx, "SELECT user_id FROM watcher WHERE issue_id = ?1", [issue_id], |row| row.get(0))?;

    for user_id in watchers {
        if !except.contains(&user_id) {
            notify(tx, user_id, kind, issue_id, text)?;
        }
    }

    Ok(())
}

/// Notifies the people `@mentioned` in `text` and returns who they are.
pub fn notify_mentions(tx: &Tx, issue_id: Id, text: &str, notice: &str) -> ApiResult<Vec<Id>> {
    let mut mentioned = Vec::new();

    for handle in mentioned_handles(text) {
        if let Some(user_id) = resolve_user(tx, &handle)? {
            if !mentioned.contains(&user_id) {
                notify(tx, user_id, "mention", issue_id, notice)?;
                mentioned.push(user_id);
            }
        }
    }

    Ok(mentioned)
}

pub fn list_notifications(connection: &Connection, user_id: Id, limit: i64) -> ApiResult<Vec<Notification>> {
    query_all(
        connection,
        "SELECT n.id, n.kind, i.key, n.actor_id, n.text, n.created_at, n.read_at
         FROM notification n LEFT JOIN issue i ON i.id = n.issue_id
         WHERE n.user_id = ?1 ORDER BY n.id DESC LIMIT ?2",
        (user_id, limit),
        |row| {
            Ok(Notification {
                id: row.get(0)?,
                kind: row.get(1)?,
                issue_key: row.get(2)?,
                actor_id: row.get(3)?,
                text: row.get(4)?,
                created_at: row.get(5)?,
                read_at: row.get(6)?,
            })
        },
    )
}

pub fn unread_count(connection: &Connection, user_id: Id) -> ApiResult<i64> {
    Ok(connection.query_row("SELECT count(*) FROM notification WHERE user_id = ?1 AND read_at IS NULL", [user_id], |row| row.get(0))?)
}

/// Marks one notification read, or all of the user's when no id is given.
pub fn mark_read(connection: &Connection, user_id: Id, id: Option<Id>) -> ApiResult<()> {
    match id {
        Some(id) => connection.execute("UPDATE notification SET read_at = ?1 WHERE user_id = ?2 AND id = ?3 AND read_at IS NULL", (now_iso(), user_id, id))?,
        None => connection.execute("UPDATE notification SET read_at = ?1 WHERE user_id = ?2 AND read_at IS NULL", (now_iso(), user_id))?,
    };

    Ok(())
}
