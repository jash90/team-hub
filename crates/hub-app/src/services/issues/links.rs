//! Links between issues.

use hub_core::{Id, IssueLink, LinkDirection, LinkType};
use rusqlite::Connection;
use serde_json::json;

use super::current::load_current;
use super::rows::{issue_access, rows_by_ids};
use crate::access::{self};
use crate::app::{App, Tx};
use crate::auth::AuthUser;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::history::{record_change, FieldChange};

pub fn load_links(app: &App, connection: &Connection, issue_id: Id) -> ApiResult<Vec<IssueLink>> {
    let edges: Vec<(Id, String, Id, bool)> = query_all(
        connection,
        "SELECT id, type, target_id, 1 FROM issue_link WHERE source_id = ?1
         UNION ALL SELECT id, type, source_id, 0 FROM issue_link WHERE target_id = ?1",
        [issue_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let others: Vec<Id> = edges.iter().map(|edge| edge.2).collect();
    let rows = rows_by_ids(app, connection, &others)?;

    Ok(edges
        .into_iter()
        .filter_map(|(id, link_type, other, outward)| {
            Some(IssueLink {
                id,
                link_type: LinkType::parse(&link_type)?,
                direction: if outward { LinkDirection::Outward } else { LinkDirection::Inward },
                issue: rows.iter().find(|row| row.id == other)?.clone(),
            })
        })
        .collect())
}

pub fn add_link(tx: &Tx, user: &AuthUser, source_id: Id, link_type: LinkType, target_key: &str) -> ApiResult<()> {
    let source = load_current(tx, source_id)?;
    let (target_id, _) = issue_access(tx, user, target_key).map_err(|_| ApiError::bad_request(format!("{target_key} does not exist")))?;
    let target = load_current(tx, target_id)?;

    if target_id == source_id {
        return Err(ApiError::bad_request("An issue cannot be linked to itself"));
    }

    let inserted = tx.execute(
        "INSERT OR IGNORE INTO issue_link (source_id, target_id, type, created_at) VALUES (?1, ?2, ?3, ?4)",
        (source_id, target_id, link_type.as_str(), &tx.now),
    )?;

    if inserted == 0 {
        return Err(ApiError::conflict("These issues are already linked that way"));
    }

    record_change(tx, source_id, &FieldChange::text("link", None, Some(format!("{} {}", link_type.as_str(), target.key))))?;
    record_change(tx, target_id, &FieldChange::text("link", None, Some(format!("{} (from {})", link_type.as_str(), source.key))))?;
    tx.emit("issue.updated", Some(source.project_id), Some(source_id), json!({ "key": source.key, "fields": ["links"] }))?;
    tx.emit("issue.updated", Some(target.project_id), Some(target_id), json!({ "key": target.key, "fields": ["links"] }))
}

pub fn delete_link(tx: &Tx, user: &AuthUser, link_id: Id) -> ApiResult<()> {
    let (source_id, target_id, link_type): (Id, Id, String) =
        query_one(tx, "SELECT source_id, target_id, type FROM issue_link WHERE id = ?1", [link_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .ok_or_else(|| ApiError::not_found("Link"))?;
    let source = load_current(tx, source_id)?;
    let target = load_current(tx, target_id)?;
    access::project_access_by_id(tx, user, source.project_id)?.require_edit()?;

    tx.execute("DELETE FROM issue_link WHERE id = ?1", [link_id])?;
    record_change(tx, source_id, &FieldChange::text("link", Some(format!("{link_type} {}", target.key)), None))?;
    tx.emit("issue.updated", Some(source.project_id), Some(source_id), json!({ "key": source.key, "fields": ["links"] }))?;
    tx.emit("issue.updated", Some(target.project_id), Some(target_id), json!({ "key": target.key, "fields": ["links"] }))
}
