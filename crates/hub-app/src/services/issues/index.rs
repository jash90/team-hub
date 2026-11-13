//! The full-text index of an issue.

use hub_core::text::fold;
use hub_core::Id;
use rusqlite::Connection;

use crate::db::query_all;
use crate::error::ApiResult;

pub fn reindex_issue(connection: &Connection, id: Id) -> ApiResult<()> {
    let (key, summary, description): (String, String, String) =
        connection.query_row("SELECT key, summary, description FROM issue WHERE id = ?1", [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let comments: Vec<String> = query_all(connection, "SELECT body FROM comment WHERE issue_id = ?1", [id], |row| row.get(0))?;

    connection.execute("DELETE FROM issue_fts WHERE rowid = ?1", [id])?;
    connection.execute(
        "INSERT INTO issue_fts (rowid, key, summary, description, comments) VALUES (?1, ?2, ?3, ?4, ?5)",
        (id, key, fold(&summary), fold(&description), fold(&comments.join("\n"))),
    )?;

    Ok(())
}
