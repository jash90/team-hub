//! Dragging an issue: its place in the order, and with it its sprint or status.

use hub_core::rank::rank_between;
use hub_core::{Id, IssuePatch};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use super::current::load_current;
use super::update::update_issue;
use crate::app::Tx;
use crate::db::query_one;
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MoveRequest {
    /// The issue that ends up directly above this one.
    pub after_key: Option<String>,
    /// The issue that ends up directly below this one.
    pub before_key: Option<String>,
    #[serde(deserialize_with = "hub_core::double_option")]
    pub sprint_id: Option<Option<Id>>,
    pub status_id: Option<Id>,
}

fn rank_of(connection: &Connection, key: &str, project_id: Id) -> ApiResult<String> {
    query_one(connection, "SELECT rank FROM issue WHERE key = ?1 COLLATE NOCASE AND project_id = ?2", (key.trim(), project_id), |row| row.get(0))?
        .ok_or_else(|| ApiError::bad_request(format!("{key} is not in this project")))
}

/// A drag on a board or backlog: a new place in the order, and possibly a new sprint or status.
pub fn move_issue(tx: &Tx, id: Id, request: &MoveRequest) -> ApiResult<()> {
    let current = load_current(tx, id)?;
    let patch = IssuePatch { sprint_id: request.sprint_id, status_id: request.status_id, ..Default::default() };
    update_issue(tx, id, &patch)?;

    let above = request.after_key.as_deref().map(|key| rank_of(tx, key, current.project_id)).transpose()?;
    let below = request.before_key.as_deref().map(|key| rank_of(tx, key, current.project_id)).transpose()?;

    let (above, below) = match (above, below) {
        (None, None) => return Ok(()),
        (Some(above), Some(below)) if above < below => (Some(above), Some(below)),
        (Some(above), _) => {
            let next: Option<String> = tx.query_row(
                "SELECT min(rank) FROM issue WHERE project_id = ?1 AND rank > ?2 AND id != ?3",
                (current.project_id, &above, id),
                |row| row.get(0),
            )?;

            (Some(above), next)
        }
        (None, Some(below)) => {
            let previous: Option<String> = tx.query_row(
                "SELECT max(rank) FROM issue WHERE project_id = ?1 AND rank < ?2 AND id != ?3",
                (current.project_id, &below, id),
                |row| row.get(0),
            )?;

            (previous, Some(below))
        }
    };

    tx.execute("UPDATE issue SET rank = ?1 WHERE id = ?2", (rank_between(above.as_deref(), below.as_deref()), id))?;
    tx.emit("issue.ranked", Some(current.project_id), Some(id), json!({ "key": current.key }))
}
