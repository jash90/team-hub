use hub_core::release_notes::{release_notes, NotesIssue};
use hub_core::{Id, Version, WorkCounts};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use crate::access::{self, ProjectAccess};
use crate::app::Tx;
use crate::auth::AuthUser;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::history::{clean_date, record_change, FieldChange};

const SELECT: &str = "SELECT v.id, v.project_id, v.name, v.description, v.start_date, v.release_date, v.released, v.released_at, v.archived,
        count(i.id),
        coalesce(sum(s.category = 'todo'), 0),
        coalesce(sum(s.category = 'in_progress'), 0),
        coalesce(sum(s.category = 'done'), 0)
    FROM version v
    LEFT JOIN issue i ON i.version_id = v.id
    LEFT JOIN status s ON s.id = i.status_id";

fn map_version(row: &rusqlite::Row) -> rusqlite::Result<Version> {
    Ok(Version {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        start_date: row.get(4)?,
        release_date: row.get(5)?,
        released: row.get(6)?,
        released_at: row.get(7)?,
        archived: row.get(8)?,
        counts: WorkCounts { total: row.get(9)?, todo: row.get(10)?, in_progress: row.get(11)?, done: row.get(12)? },
    })
}

pub fn load_versions(connection: &Connection, project_id: Id) -> ApiResult<Vec<Version>> {
    query_all(
        connection,
        &format!("{SELECT} WHERE v.project_id = ?1 GROUP BY v.id ORDER BY v.released, coalesce(v.release_date, '9999'), v.id"),
        [project_id],
        map_version,
    )
}

pub fn get_version(connection: &Connection, id: Id) -> ApiResult<Version> {
    query_one(connection, &format!("{SELECT} WHERE v.id = ?1 GROUP BY v.id"), [id], map_version)?.ok_or_else(|| ApiError::not_found("Version"))
}

pub fn version_access(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(Version, ProjectAccess)> {
    let version = get_version(connection, id)?;
    let access = access::project_access_by_id(connection, user, version.project_id)?;

    Ok((version, access))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VersionInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub start_date: Option<String>,
    pub release_date: Option<String>,
    pub archived: Option<bool>,
}

fn clean_name(name: Option<&str>) -> ApiResult<String> {
    name.map(str::trim).filter(|name| !name.is_empty()).map(String::from).ok_or_else(|| ApiError::bad_request("The version needs a name"))
}

pub fn create_version(tx: &Tx, access: &ProjectAccess, input: &VersionInput) -> ApiResult<Version> {
    access.require_edit()?;
    let name = clean_name(input.name.as_deref())?;

    tx.execute(
        "INSERT INTO version (project_id, name, description, start_date, release_date) VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            access.id,
            &name,
            input.description.as_deref().unwrap_or("").trim(),
            clean_date(input.start_date.as_deref())?,
            clean_date(input.release_date.as_deref())?,
        ),
    )?;
    let version = get_version(tx, tx.last_insert_rowid())?;
    tx.emit("version.created", Some(access.id), None, json!({ "versionId": version.id, "name": name }))?;

    Ok(version)
}

pub fn update_version(tx: &Tx, user: &AuthUser, id: Id, input: &VersionInput) -> ApiResult<Version> {
    let (_, access) = version_access(tx, user, id)?;
    access.require_edit()?;

    if input.name.is_some() {
        tx.execute("UPDATE version SET name = ?1 WHERE id = ?2", (clean_name(input.name.as_deref())?, id))?;
    }

    if let Some(description) = &input.description {
        tx.execute("UPDATE version SET description = ?1 WHERE id = ?2", (description.trim(), id))?;
    }

    if let Some(date) = &input.start_date {
        tx.execute("UPDATE version SET start_date = ?1 WHERE id = ?2", (clean_date(Some(date))?, id))?;
    }

    if let Some(date) = &input.release_date {
        tx.execute("UPDATE version SET release_date = ?1 WHERE id = ?2", (clean_date(Some(date))?, id))?;
    }

    if let Some(archived) = input.archived {
        tx.execute("UPDATE version SET archived = ?1 WHERE id = ?2", (archived, id))?;
    }

    tx.emit("version.updated", Some(access.id), None, json!({ "versionId": id }))?;

    get_version(tx, id)
}

/// Marks a version released (or takes that back). Unfinished issues may move to another
/// version; without one they stay, and the release notes say they are not finished.
pub fn set_released(tx: &Tx, user: &AuthUser, id: Id, released: bool, move_open_to: Option<Id>) -> ApiResult<Version> {
    let (version, access) = version_access(tx, user, id)?;
    access.require_edit()?;

    if released {
        if let Some(target_id) = move_open_to {
            let target = get_version(tx, target_id)?;

            if target.project_id != access.id || target.released || target.id == id {
                return Err(ApiError::bad_request("Unfinished issues can only move to an unreleased version of the same project"));
            }

            let open: Vec<Id> = query_all(
                tx,
                "SELECT i.id FROM issue i JOIN status s ON s.id = i.status_id WHERE i.version_id = ?1 AND s.category != 'done'",
                [id],
                |row| row.get(0),
            )?;

            for issue_id in open {
                tx.execute("UPDATE issue SET version_id = ?1, updated_at = ?2 WHERE id = ?3", (target.id, &tx.now, issue_id))?;
                record_change(
                    tx,
                    issue_id,
                    &FieldChange::reference("version", Some((version.id, version.name.clone())), Some((target.id, target.name.clone()))),
                )?;
            }
        }

        tx.execute(
            "UPDATE version SET released = 1, released_at = ?1, release_date = coalesce(release_date, substr(?1, 1, 10)) WHERE id = ?2",
            (&tx.now, id),
        )?;
        tx.emit("version.released", Some(access.id), None, json!({ "versionId": id, "name": version.name }))?;
    } else {
        tx.execute("UPDATE version SET released = 0, released_at = NULL WHERE id = ?1", [id])?;
        tx.emit("version.updated", Some(access.id), None, json!({ "versionId": id }))?;
    }

    get_version(tx, id)
}

pub fn delete_version(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<()> {
    let (version, access) = version_access(tx, user, id)?;
    access.require_admin()?;

    let issues: Vec<Id> = query_all(tx, "SELECT id FROM issue WHERE version_id = ?1", [id], |row| row.get(0))?;

    for issue_id in issues {
        record_change(tx, issue_id, &FieldChange::reference("version", Some((version.id, version.name.clone())), None))?;
    }

    tx.execute("DELETE FROM version WHERE id = ?1", [id])?;
    tx.emit("version.deleted", Some(access.id), None, json!({ "versionId": id }))
}

/// Release notes in Markdown: finished issues by type, unfinished ones named as such.
pub fn version_notes(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<String> {
    let (version, _) = version_access(connection, user, id)?;
    let rows = query_all(
        connection,
        "SELECT i.key, i.summary, t.name, s.category = 'done' FROM issue i
         JOIN issue_type t ON t.id = i.type_id JOIN status s ON s.id = i.status_id
         WHERE i.version_id = ?1 AND t.level != 'subtask' ORDER BY i.number",
        [id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, bool>(3)?)),
    )?;
    let types: Vec<String> =
        query_all(connection, "SELECT name FROM issue_type WHERE project_id = ?1 ORDER BY position, id", [version.project_id], |row| row.get(0))?;
    let type_order: Vec<&str> = types.iter().map(String::as_str).collect();
    let issues: Vec<NotesIssue> = rows.iter().map(|(key, summary, type_name, done)| NotesIssue { key, summary, type_name, done: *done }).collect();

    Ok(release_notes(&version.name, version.release_date.as_deref(), &type_order, &issues))
}
