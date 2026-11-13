//! Who watches an issue.

use hub_core::Id;
use rusqlite::Connection;

use crate::error::ApiResult;

pub(super) fn watch(connection: &Connection, issue_id: Id, user_id: Option<Id>) -> ApiResult<()> {
    if let Some(user_id) = user_id {
        connection.execute("INSERT OR IGNORE INTO watcher (issue_id, user_id) VALUES (?1, ?2)", (issue_id, user_id))?;
    }

    Ok(())
}

pub fn set_watching(connection: &Connection, issue_id: Id, user_id: Id, watching: bool) -> ApiResult<()> {
    if watching {
        watch(connection, issue_id, Some(user_id))
    } else {
        connection.execute("DELETE FROM watcher WHERE issue_id = ?1 AND user_id = ?2", (issue_id, user_id))?;

        Ok(())
    }
}
