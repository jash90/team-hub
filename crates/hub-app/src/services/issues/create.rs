//! Creating an issue.

use hub_core::rank::rank_between;
use hub_core::{Id, NewIssue, StatusCategory, TypeLevel, DEFAULT_PRIORITY};
use serde_json::json;

use super::current::{status_category, type_level};
use super::fields::{
    clean_labels, clean_points, clean_priority, clean_summary, require_sprint, require_user, require_version, resolve_parent, set_components,
    set_labels,
};
use super::index::reindex_issue;
use super::watchers::watch;
use crate::app::Tx;
use crate::db::query_one;
use crate::error::{ApiError, ApiResult};
use crate::services::history::clean_date;
use crate::services::notifications::{notify, notify_mentions};

/// Creates an issue. The caller has already checked that the actor may do so.
pub fn create_issue(tx: &Tx, new: &NewIssue) -> ApiResult<Id> {
    let fields = &new.fields;
    let (project_id, project_key, sequence): (Id, String, i64) =
        query_one(tx, "SELECT id, key, issue_seq FROM project WHERE key = ?1 COLLATE NOCASE", [new.project_key.trim()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .ok_or_else(|| ApiError::not_found("Project"))?;
    let summary = clean_summary(fields.summary.as_deref().unwrap_or(""))?;
    let parent_key = fields.parent_key.clone().flatten();

    let type_id = match fields.type_id {
        Some(type_id) => type_id,
        None => {
            let wanted_level = match &parent_key {
                Some(parent) => {
                    let parent_type: Option<Id> =
                        query_one(tx, "SELECT type_id FROM issue WHERE key = ?1 COLLATE NOCASE", [parent.trim()], |row| row.get(0))?;
                    let parent_level = parent_type.map(|type_id| type_level(tx, type_id, project_id)).transpose()?;

                    if parent_level == Some(TypeLevel::Standard) {
                        TypeLevel::Subtask
                    } else {
                        TypeLevel::Standard
                    }
                }
                None => TypeLevel::Standard,
            };

            query_one(
                tx,
                "SELECT id FROM issue_type WHERE project_id = ?1 AND level = ?2 ORDER BY name = 'Task' DESC, position LIMIT 1",
                (project_id, wanted_level.as_str()),
                |row| row.get(0),
            )?
            .ok_or_else(|| ApiError::conflict("The project has no issue type for this"))?
        }
    };
    let level = type_level(tx, type_id, project_id)?;
    let parent_id = resolve_parent(tx, project_id, None, level, parent_key.as_deref())?;

    let status_id = match fields.status_id {
        Some(status_id) => status_id,
        None => tx.query_row("SELECT id FROM status WHERE project_id = ?1 ORDER BY position, id LIMIT 1", [project_id], |row| row.get(0))?,
    };
    let category = status_category(tx, status_id, project_id)?;
    let assignee_id = fields.assignee_id.flatten();
    let reporter_id = fields.reporter_id.flatten().or(tx.actor.user_id);
    // A subtask lives in its parent's sprint unless it is given one of its own.
    let sprint_id = match (fields.sprint_id, parent_id) {
        (Some(sprint_id), _) => sprint_id,
        (None, Some(parent_id)) if level == TypeLevel::Subtask => {
            tx.query_row("SELECT sprint_id FROM issue WHERE id = ?1", [parent_id], |row| row.get(0))?
        }
        _ => None,
    };
    let version_id = fields.version_id.flatten();

    if let Some(assignee_id) = assignee_id {
        require_user(tx, assignee_id)?;
    }

    if let Some(sprint_id) = sprint_id {
        require_sprint(tx, sprint_id, project_id)?;
    }

    if let Some(version_id) = version_id {
        require_version(tx, version_id, project_id)?;
    }

    let number = sequence + 1;
    let key = format!("{project_key}-{number}");
    let last_rank: Option<String> = tx.query_row("SELECT max(rank) FROM issue WHERE project_id = ?1", [project_id], |row| row.get(0))?;
    let description = fields.description.clone().unwrap_or_default();

    tx.execute("UPDATE project SET issue_seq = ?1 WHERE id = ?2", (number, project_id))?;
    tx.execute(
        "INSERT INTO issue (project_id, number, key, type_id, status_id, priority, summary, description, assignee_id, reporter_id,
                            parent_id, sprint_id, version_id, points, due_date, estimate_minutes, rank, status_since, resolved_at,
                            created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?18, ?18)",
        rusqlite::params![
            project_id,
            number,
            key,
            type_id,
            status_id,
            clean_priority(fields.priority.unwrap_or(DEFAULT_PRIORITY))?,
            summary,
            description,
            assignee_id,
            reporter_id,
            parent_id,
            sprint_id,
            version_id,
            clean_points(fields.points.flatten())?,
            clean_date(fields.due_date.clone().flatten().as_deref())?,
            fields.estimate_minutes.flatten(),
            rank_between(last_rank.as_deref(), None),
            tx.now,
            (category == StatusCategory::Done).then(|| tx.now.clone()),
        ],
    )?;
    let id = tx.last_insert_rowid();

    if let Some(labels) = &fields.labels {
        set_labels(tx, id, &clean_labels(labels))?;
    }

    if let Some(component_ids) = &fields.component_ids {
        set_components(tx, id, project_id, component_ids)?;
    }

    watch(tx, id, reporter_id)?;
    watch(tx, id, assignee_id)?;
    reindex_issue(tx, id)?;

    if let Some(assignee_id) = assignee_id {
        notify(tx, assignee_id, "assigned", id, &format!("{key} was assigned to you: {summary}"))?;
    }

    notify_mentions(tx, id, &description, &format!("You were mentioned in {key}: {summary}"))?;
    tx.emit("issue.created", Some(project_id), Some(id), json!({ "key": key, "summary": summary }))?;

    Ok(id)
}
