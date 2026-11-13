//! The rules for single fields: what a valid value is, and how labels and components are stored.

use hub_core::{Id, TypeLevel};
use rusqlite::Connection;

use super::current::type_level;
use super::rows::issue_id_by_key;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};

const MAX_SUMMARY: usize = 255;

pub(super) fn clean_summary(summary: &str) -> ApiResult<String> {
    let summary = summary.split_whitespace().collect::<Vec<_>>().join(" ");

    if summary.is_empty() {
        return Err(ApiError::bad_request("An issue needs a summary"));
    }

    if summary.chars().count() > MAX_SUMMARY {
        return Err(ApiError::bad_request(format!("The summary is longer than {MAX_SUMMARY} characters")));
    }

    Ok(summary)
}

pub(super) fn clean_priority(priority: i64) -> ApiResult<i64> {
    if (1..=5).contains(&priority) {
        Ok(priority)
    } else {
        Err(ApiError::bad_request("Priority goes from 1 (highest) to 5 (lowest)"))
    }
}

pub(super) fn clean_points(points: Option<f64>) -> ApiResult<Option<f64>> {
    match points {
        Some(points) if !(0.0..=1000.0).contains(&points) => Err(ApiError::bad_request("Story points go from 0 to 1000")),
        other => Ok(other),
    }
}

pub(super) fn clean_labels(labels: &[String]) -> Vec<String> {
    let mut cleaned: Vec<String> = Vec::new();

    for label in labels {
        let label: String = label.split_whitespace().collect::<Vec<_>>().join("-").chars().take(40).collect();

        if !label.is_empty() && !cleaned.iter().any(|existing| existing.eq_ignore_ascii_case(&label)) {
            cleaned.push(label);
        }
    }

    cleaned
}

pub(super) fn require_user(connection: &Connection, id: Id) -> ApiResult<()> {
    let found: Option<Id> = query_one(connection, "SELECT id FROM user WHERE id = ?1 AND active = 1", [id], |row| row.get(0))?;

    found.map(|_| ()).ok_or_else(|| ApiError::bad_request("That person does not exist or is no longer active"))
}

pub(super) fn require_sprint(connection: &Connection, sprint_id: Id, project_id: Id) -> ApiResult<()> {
    let state: Option<String> =
        query_one(connection, "SELECT state FROM sprint WHERE id = ?1 AND project_id = ?2", (sprint_id, project_id), |row| row.get(0))?;

    match state.as_deref() {
        None => Err(ApiError::bad_request("That sprint does not belong to this project")),
        Some("closed") => Err(ApiError::conflict("A completed sprint cannot take new issues")),
        Some(_) => Ok(()),
    }
}

pub(super) fn require_version(connection: &Connection, version_id: Id, project_id: Id) -> ApiResult<()> {
    let found: Option<Id> =
        query_one(connection, "SELECT id FROM version WHERE id = ?1 AND project_id = ?2", (version_id, project_id), |row| row.get(0))?;

    found.map(|_| ()).ok_or_else(|| ApiError::bad_request("That version does not belong to this project"))
}

/// Checks that an issue of `level` may sit under `parent`, and returns the parent's id.
///
/// Epics have no parent. A subtask needs a parent that is an ordinary issue. An ordinary
/// issue may only sit under an epic.
pub(super) fn resolve_parent(
    connection: &Connection,
    project_id: Id,
    own_id: Option<Id>,
    level: TypeLevel,
    parent_key: Option<&str>,
) -> ApiResult<Option<Id>> {
    let Some(parent_key) = parent_key.map(str::trim).filter(|key| !key.is_empty()) else {
        if level == TypeLevel::Subtask {
            return Err(ApiError::bad_request("A subtask needs a parent issue"));
        }

        return Ok(None);
    };
    let (parent_id, parent_project) =
        issue_id_by_key(connection, parent_key)?.ok_or_else(|| ApiError::bad_request(format!("{parent_key} does not exist")))?;

    if parent_project != project_id {
        return Err(ApiError::bad_request("The parent must be in the same project"));
    }

    if Some(parent_id) == own_id {
        return Err(ApiError::bad_request("An issue cannot be its own parent"));
    }

    let parent_type: Id = connection.query_row("SELECT type_id FROM issue WHERE id = ?1", [parent_id], |row| row.get(0))?;
    let parent_level = type_level(connection, parent_type, project_id)?;
    let fits = match level {
        TypeLevel::Epic => false,
        TypeLevel::Standard => parent_level == TypeLevel::Epic,
        TypeLevel::Subtask => parent_level == TypeLevel::Standard,
    };

    if !fits {
        return Err(ApiError::bad_request(match level {
            TypeLevel::Epic => "An epic cannot have a parent",
            TypeLevel::Standard => "Only an epic can be the parent of this issue",
            TypeLevel::Subtask => "A subtask belongs under a story, task or bug",
        }));
    }

    Ok(Some(parent_id))
}

pub(super) fn set_labels(connection: &Connection, issue_id: Id, labels: &[String]) -> ApiResult<()> {
    connection.execute("DELETE FROM issue_label WHERE issue_id = ?1", [issue_id])?;

    for label in labels {
        connection.execute("INSERT OR IGNORE INTO issue_label (issue_id, label) VALUES (?1, ?2)", (issue_id, label))?;
    }

    Ok(())
}

pub(super) fn set_components(connection: &Connection, issue_id: Id, project_id: Id, component_ids: &[Id]) -> ApiResult<()> {
    connection.execute("DELETE FROM issue_component WHERE issue_id = ?1", [issue_id])?;

    for component_id in component_ids {
        let owned: Option<Id> =
            query_one(connection, "SELECT id FROM component WHERE id = ?1 AND project_id = ?2", (component_id, project_id), |row| row.get(0))?;

        if owned.is_none() {
            return Err(ApiError::bad_request("That component does not belong to this project"));
        }

        connection.execute("INSERT OR IGNORE INTO issue_component (issue_id, component_id) VALUES (?1, ?2)", (issue_id, component_id))?;
    }

    Ok(())
}

pub(super) fn component_names(connection: &Connection, issue_id: Id) -> ApiResult<Option<String>> {
    let names: Vec<String> = query_all(
        connection,
        "SELECT c.name FROM issue_component ic JOIN component c ON c.id = ic.component_id WHERE ic.issue_id = ?1 ORDER BY c.name COLLATE NOCASE",
        [issue_id],
        |row| row.get(0),
    )?;

    Ok(Some(names.join(", ")).filter(|names| !names.is_empty()))
}

pub(super) fn label_names(connection: &Connection, issue_id: Id) -> ApiResult<Option<String>> {
    let names: Vec<String> =
        query_all(connection, "SELECT label FROM issue_label WHERE issue_id = ?1 ORDER BY label COLLATE NOCASE", [issue_id], |row| row.get(0))?;

    Ok(Some(names.join(", ")).filter(|names| !names.is_empty()))
}
