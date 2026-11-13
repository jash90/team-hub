//! Deleting an issue.

use hub_core::Id;
use serde_json::json;

use super::current::load_current;
use crate::app::Tx;
use crate::db::query_all;
use crate::error::ApiResult;

/// Deletes an issue and returns the files of its attachments, for the caller to remove once
/// the transaction has committed.
pub fn delete_issue(tx: &Tx, id: Id) -> ApiResult<Vec<String>> {
    let current = load_current(tx, id)?;
    let files: Vec<String> = query_all(tx, "SELECT path FROM attachment WHERE issue_id = ?1", [id], |row| row.get(0))?;

    tx.execute("DELETE FROM issue_fts WHERE rowid = ?1", [id])?;
    tx.execute("DELETE FROM issue WHERE id = ?1", [id])?;
    tx.emit("issue.deleted", Some(current.project_id), None, json!({ "key": current.key, "summary": current.summary }))?;

    Ok(files)
}
