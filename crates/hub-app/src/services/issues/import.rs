//! Issues, comments, history and worklogs as another tracker had them.

use hub_core::rank::rank_between;
use hub_core::{Id, StatusCategory, TypeLevel};
use rusqlite::Connection;

use super::current::{status_category, type_level};
use super::fields::{clean_labels, clean_points, clean_priority, clean_summary, set_components, set_labels};
use super::index::reindex_issue;
use crate::app::Tx;
use crate::db::query_one;
use crate::error::{ApiError, ApiResult};
use crate::services::history::{clean_date, FieldChange};

/// An issue as another tracker had it. Ids refer to rows of the issue's own project, and
/// instants are RFC 3339 in UTC, the way `now_iso` writes them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedIssue {
    pub project_id: Id,
    /// The number in the key (`WEB-123` → 123), kept so commits and pull requests still point at it.
    pub number: i64,
    pub type_id: Id,
    pub status_id: Id,
    pub priority: i64,
    pub summary: String,
    pub description: String,
    pub assignee_id: Option<Id>,
    pub reporter_id: Option<Id>,
    pub parent_id: Option<Id>,
    /// May be a completed sprint: that is where finished work of the past belongs.
    pub sprint_id: Option<Id>,
    pub version_id: Option<Id>,
    pub points: Option<f64>,
    pub due_date: Option<String>,
    pub estimate_minutes: Option<i64>,
    pub labels: Vec<String>,
    pub component_ids: Vec<Id>,
    pub created_at: String,
    pub updated_at: String,
    /// When the issue entered its current status.
    pub status_since: String,
    /// Only kept for a status that is done; then it defaults to `status_since`.
    pub resolved_at: Option<String>,
}

fn owned_by_project(connection: &Connection, table: &str, id: Option<Id>, project_id: Id, what: &str) -> ApiResult<()> {
    let Some(id) = id else { return Ok(()) };
    let found: Option<Id> =
        query_one(connection, &format!("SELECT id FROM {table} WHERE id = ?1 AND project_id = ?2"), (id, project_id), |row| row.get(0))?;

    found.map(|_| ()).ok_or_else(|| ApiError::bad_request(format!("That {what} does not belong to this project")))
}

/// Stores an imported issue under its original number: inserted when the project has no issue
/// with that number, otherwise overwritten (its place in the order is kept). The project's
/// counter is raised so that new issues continue after it. Unlike `create_issue` this takes
/// the timestamps as given, notifies nobody, makes nobody a watcher and emits no event; the
/// importer announces its own progress. Returns the issue's id.
pub fn import_issue(tx: &Tx, issue: &ImportedIssue) -> ApiResult<Id> {
    let project_id = issue.project_id;
    let project_key: String =
        query_one(tx, "SELECT key FROM project WHERE id = ?1", [project_id], |row| row.get(0))?.ok_or_else(|| ApiError::not_found("Project"))?;

    if !(1..=9_999_999).contains(&issue.number) {
        return Err(ApiError::bad_request("An issue number goes from 1 to 9999999"));
    }

    let level = type_level(tx, issue.type_id, project_id)?;
    let category = status_category(tx, issue.status_id, project_id)?;
    let existing: Option<Id> =
        query_one(tx, "SELECT id FROM issue WHERE project_id = ?1 AND number = ?2", (project_id, issue.number), |row| row.get(0))?;

    owned_by_project(tx, "sprint", issue.sprint_id, project_id, "sprint")?;
    owned_by_project(tx, "version", issue.version_id, project_id, "version")?;

    if let Some(parent_id) = issue.parent_id {
        let parent_type: Option<Id> =
            query_one(tx, "SELECT type_id FROM issue WHERE id = ?1 AND project_id = ?2", (parent_id, project_id), |row| row.get(0))?;
        let parent_level = type_level(tx, parent_type.ok_or_else(|| ApiError::bad_request("The parent must be in the same project"))?, project_id)?;
        let fits = match level {
            TypeLevel::Epic => false,
            TypeLevel::Standard => parent_level == TypeLevel::Epic,
            TypeLevel::Subtask => parent_level == TypeLevel::Standard,
        };

        if !fits || Some(parent_id) == existing {
            return Err(ApiError::bad_request("That parent does not fit: epics hold stories, tasks and bugs, and those hold subtasks"));
        }
    }

    let resolved_at = (category == StatusCategory::Done).then(|| issue.resolved_at.clone().unwrap_or_else(|| issue.status_since.clone()));
    // Everything is checked before the first write, so a refused issue leaves nothing behind.
    let (priority, summary) = (clean_priority(issue.priority)?, clean_summary(&issue.summary)?);
    let (points, due_date) = (clean_points(issue.points)?, clean_date(issue.due_date.as_deref())?);

    let id = match existing {
        Some(id) => id,
        None => {
            // A new issue goes to the end of the project's order; the fields follow below.
            let last_rank: Option<String> = tx.query_row("SELECT max(rank) FROM issue WHERE project_id = ?1", [project_id], |row| row.get(0))?;

            tx.execute(
                "INSERT INTO issue (project_id, number, key, type_id, status_id, summary, rank, status_since, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, '', ?6, ?7, ?7, ?7)",
                rusqlite::params![
                    project_id,
                    issue.number,
                    format!("{project_key}-{}", issue.number),
                    issue.type_id,
                    issue.status_id,
                    rank_between(last_rank.as_deref(), None),
                    issue.created_at,
                ],
            )?;

            tx.last_insert_rowid()
        }
    };

    tx.execute(
        "UPDATE issue SET type_id = ?1, status_id = ?2, priority = ?3, summary = ?4, description = ?5, assignee_id = ?6,
                reporter_id = ?7, parent_id = ?8, sprint_id = ?9, version_id = ?10, points = ?11, due_date = ?12,
                estimate_minutes = ?13, status_since = ?14, resolved_at = ?15, created_at = ?16, updated_at = ?17
         WHERE id = ?18",
        rusqlite::params![
            issue.type_id,
            issue.status_id,
            priority,
            summary,
            issue.description,
            issue.assignee_id,
            issue.reporter_id,
            issue.parent_id,
            issue.sprint_id,
            issue.version_id,
            points,
            due_date,
            issue.estimate_minutes.filter(|minutes| *minutes > 0),
            issue.status_since,
            resolved_at,
            issue.created_at,
            issue.updated_at,
            id,
        ],
    )?;
    tx.execute("UPDATE project SET issue_seq = max(issue_seq, ?1) WHERE id = ?2", (issue.number, project_id))?;
    set_labels(tx, id, &clean_labels(&issue.labels))?;
    set_components(tx, id, project_id, &issue.component_ids)?;
    reindex_issue(tx, id)?;

    Ok(id)
}

/// Adds a comment as `author` wrote it at `created_at`, without notifications or events.
/// The search index is not refreshed: call `reindex_issue` once after an issue's last comment.
pub fn import_comment(tx: &Tx, issue_id: Id, author: &crate::app::Actor, body: &str, created_at: &str, updated_at: &str) -> ApiResult<Id> {
    let body = body.trim();

    if body.is_empty() {
        return Err(ApiError::bad_request("The comment is empty"));
    }

    tx.execute(
        "INSERT INTO comment (issue_id, author_id, author_label, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (issue_id, author.user_id, &author.label, body, created_at, updated_at),
    )?;

    Ok(tx.last_insert_rowid())
}

/// Records a change that `actor` made at `at`, as `record_change` does for the transaction's
/// own actor and clock. Reports read these rows, so status and sprint changes carry their ids.
pub fn import_change(tx: &Tx, issue_id: Id, actor: &crate::app::Actor, change: &FieldChange, at: &str) -> ApiResult<()> {
    tx.execute(
        "INSERT INTO change (issue_id, actor_id, actor_label, field, old_value, new_value, old_ref, new_ref, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        (issue_id, actor.user_id, &actor.label, change.field, &change.old, &change.new, &change.old_ref, &change.new_ref, at),
    )?;

    Ok(())
}

/// Logs time `user_id` spent, starting at `started_at`, without touching the issue or emitting an event.
pub fn import_worklog(tx: &Tx, issue_id: Id, user_id: Option<Id>, minutes: i64, started_at: &str, note: &str) -> ApiResult<Id> {
    if minutes < 1 {
        return Err(ApiError::bad_request("Time spent must be at least a minute"));
    }

    tx.execute(
        "INSERT INTO worklog (issue_id, user_id, minutes, started_at, note) VALUES (?1, ?2, ?3, ?4, ?5)",
        (issue_id, user_id, minutes, started_at, note.trim()),
    )?;

    Ok(tx.last_insert_rowid())
}
