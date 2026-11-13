//! Issues: reading them, and every way they change. Each change is recorded in the issue's
//! history and announced as an event.

use std::path::PathBuf;

use hub_core::keys::parse_issue_key;
use hub_core::rank::rank_between;
use hub_core::text::{fold, truncate};
use hub_core::workflow::can_transition;
use hub_core::{
    Attachment, Comment, Id, Issue, IssueLink, IssuePatch, IssueRow, LinkDirection, LinkType, NewIssue, StatusCategory, SubtaskTally, TypeLevel,
    Worklog, DEFAULT_PRIORITY,
};
use rusqlite::types::Value;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use crate::access::{self, ProjectAccess};
use crate::app::{App, Tx};
use crate::auth::AuthUser;
use crate::db::{placeholders, query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::history::{clean_date, load_changes, record_change, FieldChange};
use crate::services::notifications::{notify, notify_mentions, notify_watchers};
use crate::services::projects::{load_transitions, strict_workflow};
use crate::services::users::user_name;

const MAX_SUMMARY: usize = 255;
const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
const LABEL_SEPARATOR: char = '\u{1f}';

pub const ISSUE_SELECT: &str = "SELECT i.id, i.key, i.project_id, p.key, i.number, i.summary, i.type_id, i.status_id, i.priority,
        i.assignee_id, i.reporter_id, i.parent_id, par.key, par.summary, i.sprint_id, i.version_id, i.points, i.due_date,
        i.rank, i.created_at, i.updated_at, i.resolved_at,
        (SELECT group_concat(label, char(31)) FROM (SELECT label FROM issue_label WHERE issue_id = i.id ORDER BY label COLLATE NOCASE)),
        (SELECT group_concat(component_id) FROM issue_component WHERE issue_id = i.id),
        (SELECT count(*) FROM issue c WHERE c.parent_id = i.id),
        (SELECT count(*) FROM issue c JOIN status cs ON cs.id = c.status_id WHERE c.parent_id = i.id AND cs.category = 'done'),
        (SELECT count(*) FROM comment WHERE issue_id = i.id)
    FROM issue i
    JOIN project p ON p.id = i.project_id
    LEFT JOIN issue par ON par.id = i.parent_id";

fn map_issue_row(row: &rusqlite::Row) -> rusqlite::Result<IssueRow> {
    let labels: Option<String> = row.get(22)?;
    let components: Option<String> = row.get(23)?;

    Ok(IssueRow {
        id: row.get(0)?,
        key: row.get(1)?,
        project_id: row.get(2)?,
        project_key: row.get(3)?,
        number: row.get(4)?,
        summary: row.get(5)?,
        type_id: row.get(6)?,
        status_id: row.get(7)?,
        priority: row.get(8)?,
        assignee_id: row.get(9)?,
        reporter_id: row.get(10)?,
        parent_id: row.get(11)?,
        parent_key: row.get(12)?,
        parent_summary: row.get(13)?,
        sprint_id: row.get(14)?,
        version_id: row.get(15)?,
        points: row.get(16)?,
        due_date: row.get(17)?,
        rank: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        resolved_at: row.get(21)?,
        labels: labels.map(|text| text.split(LABEL_SEPARATOR).map(String::from).collect()).unwrap_or_default(),
        component_ids: components.map(|text| text.split(',').filter_map(|id| id.parse().ok()).collect()).unwrap_or_default(),
        subtasks: SubtaskTally { total: row.get(24)?, done: row.get(25)? },
        comment_count: row.get(26)?,
        ext: Default::default(),
    })
}

/// Lets every enabled module add its badges to the rows.
pub fn decorate(app: &App, connection: &Connection, rows: &mut [IssueRow]) -> ApiResult<()> {
    if rows.is_empty() {
        return Ok(());
    }

    for module in app.enabled_modules() {
        module.decorate_issues(connection, rows)?;
    }

    Ok(())
}

/// Issue rows for `tail`, the part of the statement after the FROM clause (`WHERE ... ORDER BY ...`).
pub fn load_rows(app: &App, connection: &Connection, tail: &str, params: Vec<Value>) -> ApiResult<Vec<IssueRow>> {
    let mut rows = query_all(connection, &format!("{ISSUE_SELECT} {tail}"), rusqlite::params_from_iter(params), map_issue_row)?;
    decorate(app, connection, &mut rows)?;

    Ok(rows)
}

pub fn rows_by_ids(app: &App, connection: &Connection, ids: &[Id]) -> ApiResult<Vec<IssueRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let tail = format!("WHERE i.id IN ({}) ORDER BY i.rank", placeholders(ids.len()));

    load_rows(app, connection, &tail, ids.iter().map(|id| Value::Integer(*id)).collect())
}

pub fn row_by_id(app: &App, connection: &Connection, id: Id) -> ApiResult<IssueRow> {
    load_rows(app, connection, "WHERE i.id = ?", vec![Value::Integer(id)])?.pop().ok_or_else(|| ApiError::not_found("Issue"))
}

pub fn issue_id_by_key(connection: &Connection, key: &str) -> ApiResult<Option<(Id, Id)>> {
    let Some((project, number)) = parse_issue_key(key) else { return Ok(None) };

    query_one(connection, "SELECT id, project_id FROM issue WHERE key = ?1", [format!("{project}-{number}")], |row| Ok((row.get(0)?, row.get(1)?)))
}

/// The issue's id and the caller's access to its project. A key the caller may not see is
/// reported as missing.
pub fn issue_access(connection: &Connection, user: &AuthUser, key: &str) -> ApiResult<(Id, ProjectAccess)> {
    let (id, project_id) = issue_id_by_key(connection, key)?.ok_or_else(|| ApiError::not_found("Issue"))?;
    let access = access::project_access_by_id(connection, user, project_id).map_err(|_| ApiError::not_found("Issue"))?;

    Ok((id, access))
}

struct Current {
    id: Id,
    project_id: Id,
    key: String,
    type_id: Id,
    status_id: Id,
    priority: i64,
    summary: String,
    description: String,
    assignee_id: Option<Id>,
    reporter_id: Option<Id>,
    parent_id: Option<Id>,
    sprint_id: Option<Id>,
    version_id: Option<Id>,
    points: Option<f64>,
    due_date: Option<String>,
    estimate_minutes: Option<i64>,
}

fn load_current(connection: &Connection, id: Id) -> ApiResult<Current> {
    query_one(
        connection,
        "SELECT id, project_id, key, type_id, status_id, priority, summary, description, assignee_id, reporter_id, parent_id,
                sprint_id, version_id, points, due_date, estimate_minutes
         FROM issue WHERE id = ?1",
        [id],
        |row| {
            Ok(Current {
                id: row.get(0)?,
                project_id: row.get(1)?,
                key: row.get(2)?,
                type_id: row.get(3)?,
                status_id: row.get(4)?,
                priority: row.get(5)?,
                summary: row.get(6)?,
                description: row.get(7)?,
                assignee_id: row.get(8)?,
                reporter_id: row.get(9)?,
                parent_id: row.get(10)?,
                sprint_id: row.get(11)?,
                version_id: row.get(12)?,
                points: row.get(13)?,
                due_date: row.get(14)?,
                estimate_minutes: row.get(15)?,
            })
        },
    )?
    .ok_or_else(|| ApiError::not_found("Issue"))
}

fn name_in(connection: &Connection, table: &str, id: Option<Id>) -> ApiResult<Option<(Id, String)>> {
    let Some(id) = id else { return Ok(None) };
    let name: Option<String> = query_one(connection, &format!("SELECT name FROM {table} WHERE id = ?1"), [id], |row| row.get(0))?;

    Ok(name.map(|name| (id, name)))
}

fn type_level(connection: &Connection, type_id: Id, project_id: Id) -> ApiResult<TypeLevel> {
    let level: Option<String> =
        query_one(connection, "SELECT level FROM issue_type WHERE id = ?1 AND project_id = ?2", (type_id, project_id), |row| row.get(0))?;

    level.as_deref().and_then(TypeLevel::parse).ok_or_else(|| ApiError::bad_request("That issue type does not belong to this project"))
}

fn status_category(connection: &Connection, status_id: Id, project_id: Id) -> ApiResult<StatusCategory> {
    let category: Option<String> =
        query_one(connection, "SELECT category FROM status WHERE id = ?1 AND project_id = ?2", (status_id, project_id), |row| row.get(0))?;

    category.as_deref().and_then(StatusCategory::parse).ok_or_else(|| ApiError::bad_request("That status does not belong to this project"))
}

fn clean_summary(summary: &str) -> ApiResult<String> {
    let summary = summary.split_whitespace().collect::<Vec<_>>().join(" ");

    if summary.is_empty() {
        return Err(ApiError::bad_request("An issue needs a summary"));
    }

    if summary.chars().count() > MAX_SUMMARY {
        return Err(ApiError::bad_request(format!("The summary is longer than {MAX_SUMMARY} characters")));
    }

    Ok(summary)
}

fn clean_priority(priority: i64) -> ApiResult<i64> {
    if (1..=5).contains(&priority) {
        Ok(priority)
    } else {
        Err(ApiError::bad_request("Priority goes from 1 (highest) to 5 (lowest)"))
    }
}

fn clean_points(points: Option<f64>) -> ApiResult<Option<f64>> {
    match points {
        Some(points) if !(0.0..=1000.0).contains(&points) => Err(ApiError::bad_request("Story points go from 0 to 1000")),
        other => Ok(other),
    }
}

fn clean_labels(labels: &[String]) -> Vec<String> {
    let mut cleaned: Vec<String> = Vec::new();

    for label in labels {
        let label: String = label.split_whitespace().collect::<Vec<_>>().join("-").chars().take(40).collect();

        if !label.is_empty() && !cleaned.iter().any(|existing| existing.eq_ignore_ascii_case(&label)) {
            cleaned.push(label);
        }
    }

    cleaned
}

fn require_user(connection: &Connection, id: Id) -> ApiResult<()> {
    let found: Option<Id> = query_one(connection, "SELECT id FROM user WHERE id = ?1 AND active = 1", [id], |row| row.get(0))?;

    found.map(|_| ()).ok_or_else(|| ApiError::bad_request("That person does not exist or is no longer active"))
}

fn require_sprint(connection: &Connection, sprint_id: Id, project_id: Id) -> ApiResult<()> {
    let state: Option<String> =
        query_one(connection, "SELECT state FROM sprint WHERE id = ?1 AND project_id = ?2", (sprint_id, project_id), |row| row.get(0))?;

    match state.as_deref() {
        None => Err(ApiError::bad_request("That sprint does not belong to this project")),
        Some("closed") => Err(ApiError::conflict("A completed sprint cannot take new issues")),
        Some(_) => Ok(()),
    }
}

fn require_version(connection: &Connection, version_id: Id, project_id: Id) -> ApiResult<()> {
    let found: Option<Id> =
        query_one(connection, "SELECT id FROM version WHERE id = ?1 AND project_id = ?2", (version_id, project_id), |row| row.get(0))?;

    found.map(|_| ()).ok_or_else(|| ApiError::bad_request("That version does not belong to this project"))
}

/// Checks that an issue of `level` may sit under `parent`, and returns the parent's id.
///
/// Epics have no parent. A subtask needs a parent that is an ordinary issue. An ordinary
/// issue may only sit under an epic.
fn resolve_parent(connection: &Connection, project_id: Id, own_id: Option<Id>, level: TypeLevel, parent_key: Option<&str>) -> ApiResult<Option<Id>> {
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

fn watch(connection: &Connection, issue_id: Id, user_id: Option<Id>) -> ApiResult<()> {
    if let Some(user_id) = user_id {
        connection.execute("INSERT OR IGNORE INTO watcher (issue_id, user_id) VALUES (?1, ?2)", (issue_id, user_id))?;
    }

    Ok(())
}

fn set_labels(connection: &Connection, issue_id: Id, labels: &[String]) -> ApiResult<()> {
    connection.execute("DELETE FROM issue_label WHERE issue_id = ?1", [issue_id])?;

    for label in labels {
        connection.execute("INSERT OR IGNORE INTO issue_label (issue_id, label) VALUES (?1, ?2)", (issue_id, label))?;
    }

    Ok(())
}

fn set_components(connection: &Connection, issue_id: Id, project_id: Id, component_ids: &[Id]) -> ApiResult<()> {
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

fn component_names(connection: &Connection, issue_id: Id) -> ApiResult<Option<String>> {
    let names: Vec<String> = query_all(
        connection,
        "SELECT c.name FROM issue_component ic JOIN component c ON c.id = ic.component_id WHERE ic.issue_id = ?1 ORDER BY c.name COLLATE NOCASE",
        [issue_id],
        |row| row.get(0),
    )?;

    Ok(Some(names.join(", ")).filter(|names| !names.is_empty()))
}

fn label_names(connection: &Connection, issue_id: Id) -> ApiResult<Option<String>> {
    let names: Vec<String> =
        query_all(connection, "SELECT label FROM issue_label WHERE issue_id = ?1 ORDER BY label COLLATE NOCASE", [issue_id], |row| row.get(0))?;

    Ok(Some(names.join(", ")).filter(|names| !names.is_empty()))
}

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
    let sprint_id = fields.sprint_id.flatten();
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

/// Applies a patch to an issue and returns the names of the fields that actually changed.
/// The caller has already checked that the actor may edit the issue.
pub fn update_issue(tx: &Tx, id: Id, patch: &IssuePatch) -> ApiResult<Vec<&'static str>> {
    let current = load_current(tx, id)?;
    let project_id = current.project_id;
    let mut changed: Vec<&'static str> = Vec::new();
    let mut transition = None;

    if let Some(summary) = &patch.summary {
        let summary = clean_summary(summary)?;

        if summary != current.summary {
            tx.execute("UPDATE issue SET summary = ?1 WHERE id = ?2", (&summary, id))?;
            record_change(tx, id, &FieldChange::text("summary", Some(current.summary.clone()), Some(summary)))?;
            changed.push("summary");
        }
    }

    if let Some(description) = &patch.description {
        if *description != current.description {
            tx.execute("UPDATE issue SET description = ?1 WHERE id = ?2", (description, id))?;
            record_change(tx, id, &FieldChange::text("description", Some(truncate(&current.description, 200)), Some(truncate(description, 200))))?;
            notify_mentions(tx, id, description, &format!("You were mentioned in {}: {}", current.key, current.summary))?;
            changed.push("description");
        }
    }

    let new_type = patch.type_id.filter(|type_id| *type_id != current.type_id);
    let level = type_level(tx, new_type.unwrap_or(current.type_id), project_id)?;

    if let Some(type_id) = new_type {
        let old_level = type_level(tx, current.type_id, project_id)?;

        if level != old_level && patch.parent_key.is_none() {
            // The existing parent (or lack of one) must still make sense for the new kind of issue.
            let parent_key: Option<String> = match current.parent_id {
                Some(parent_id) => Some(tx.query_row("SELECT key FROM issue WHERE id = ?1", [parent_id], |row| row.get(0))?),
                None => None,
            };
            resolve_parent(tx, project_id, Some(id), level, parent_key.as_deref())?;
        }

        if level != TypeLevel::Standard {
            let children: i64 = tx.query_row("SELECT count(*) FROM issue WHERE parent_id = ?1", [id], |row| row.get(0))?;

            if children > 0 && level == TypeLevel::Subtask {
                return Err(ApiError::conflict("An issue with subtasks cannot become a subtask"));
            }
        }

        tx.execute("UPDATE issue SET type_id = ?1 WHERE id = ?2", (type_id, id))?;
        record_change(
            tx,
            id,
            &FieldChange::reference("type", name_in(tx, "issue_type", Some(current.type_id))?, name_in(tx, "issue_type", Some(type_id))?),
        )?;
        changed.push("type");
    }

    if let Some(parent_key) = &patch.parent_key {
        let parent_id = resolve_parent(tx, project_id, Some(id), level, parent_key.as_deref())?;

        if parent_id != current.parent_id {
            let key_of = |issue_id: Option<Id>| -> ApiResult<Option<String>> {
                match issue_id {
                    Some(issue_id) => Ok(Some(tx.query_row("SELECT key FROM issue WHERE id = ?1", [issue_id], |row| row.get(0))?)),
                    None => Ok(None),
                }
            };

            tx.execute("UPDATE issue SET parent_id = ?1 WHERE id = ?2", (parent_id, id))?;
            record_change(tx, id, &FieldChange::text("parent", key_of(current.parent_id)?, key_of(parent_id)?))?;
            changed.push("parent");
        }
    }

    if let Some(status_id) = patch.status_id.filter(|status_id| *status_id != current.status_id) {
        let category = status_category(tx, status_id, project_id)?;
        let old_category = status_category(tx, current.status_id, project_id)?;

        if !can_transition(strict_workflow(tx, project_id)?, &load_transitions(tx, project_id)?, current.status_id, status_id) {
            let from = name_in(tx, "status", Some(current.status_id))?.map(|(_, name)| name).unwrap_or_default();
            let to = name_in(tx, "status", Some(status_id))?.map(|(_, name)| name).unwrap_or_default();

            return Err(ApiError::conflict(format!("The workflow does not allow moving from \"{from}\" to \"{to}\"")));
        }

        let resolved = (category == StatusCategory::Done).then(|| tx.now.clone());
        let old = name_in(tx, "status", Some(current.status_id))?;
        let new = name_in(tx, "status", Some(status_id))?;

        tx.execute("UPDATE issue SET status_id = ?1, status_since = ?2, resolved_at = ?3 WHERE id = ?4", (status_id, &tx.now, resolved, id))?;
        transition = Some(json!({
            "key": current.key,
            "from": old.as_ref().map(|(_, name)| name),
            "to": new.as_ref().map(|(_, name)| name),
            "fromCategory": old_category.as_str(),
            "toCategory": category.as_str(),
        }));
        notify_watchers(
            tx,
            "status",
            id,
            &format!("{} moved to {}: {}", current.key, new.as_ref().map(|(_, name)| name.as_str()).unwrap_or("?"), current.summary),
            &[],
        )?;
        record_change(tx, id, &FieldChange::reference("status", old, new))?;
        changed.push("status");
    }

    if let Some(priority) = patch.priority.filter(|priority| *priority != current.priority) {
        tx.execute("UPDATE issue SET priority = ?1 WHERE id = ?2", (clean_priority(priority)?, id))?;
        record_change(
            tx,
            id,
            &FieldChange::text(
                "priority",
                Some(hub_core::priority_name(current.priority).to_string()),
                Some(hub_core::priority_name(priority).to_string()),
            ),
        )?;
        changed.push("priority");
    }

    if let Some(assignee_id) = patch.assignee_id.filter(|assignee_id| *assignee_id != current.assignee_id) {
        if let Some(assignee_id) = assignee_id {
            require_user(tx, assignee_id)?;
            notify(tx, assignee_id, "assigned", id, &format!("{} was assigned to you: {}", current.key, current.summary))?;
        }

        tx.execute("UPDATE issue SET assignee_id = ?1 WHERE id = ?2", (assignee_id, id))?;
        watch(tx, id, assignee_id)?;
        record_change(tx, id, &FieldChange::reference("assignee", name_in(tx, "user", current.assignee_id)?, name_in(tx, "user", assignee_id)?))?;
        changed.push("assignee");
    }

    if let Some(reporter_id) = patch.reporter_id.filter(|reporter_id| *reporter_id != current.reporter_id) {
        tx.execute("UPDATE issue SET reporter_id = ?1 WHERE id = ?2", (reporter_id, id))?;
        record_change(tx, id, &FieldChange::reference("reporter", name_in(tx, "user", current.reporter_id)?, name_in(tx, "user", reporter_id)?))?;
        changed.push("reporter");
    }

    if let Some(sprint_id) = patch.sprint_id.filter(|sprint_id| *sprint_id != current.sprint_id) {
        if let Some(sprint_id) = sprint_id {
            require_sprint(tx, sprint_id, project_id)?;
        }

        tx.execute("UPDATE issue SET sprint_id = ?1 WHERE id = ?2", (sprint_id, id))?;
        record_change(tx, id, &FieldChange::reference("sprint", name_in(tx, "sprint", current.sprint_id)?, name_in(tx, "sprint", sprint_id)?))?;

        // Subtasks travel with their parent, and their history says so.
        let subtasks: Vec<(Id, Option<Id>)> = query_all(
            tx,
            "SELECT id, sprint_id FROM issue WHERE parent_id = ?1 AND type_id IN (SELECT id FROM issue_type WHERE level = 'subtask')",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        for (subtask_id, subtask_sprint) in subtasks {
            if subtask_sprint == sprint_id {
                continue;
            }

            tx.execute("UPDATE issue SET sprint_id = ?1, updated_at = ?2 WHERE id = ?3", (sprint_id, &tx.now, subtask_id))?;
            record_change(
                tx,
                subtask_id,
                &FieldChange::reference("sprint", name_in(tx, "sprint", subtask_sprint)?, name_in(tx, "sprint", sprint_id)?),
            )?;
        }
        changed.push("sprint");
    }

    if let Some(version_id) = patch.version_id.filter(|version_id| *version_id != current.version_id) {
        if let Some(version_id) = version_id {
            require_version(tx, version_id, project_id)?;
        }

        tx.execute("UPDATE issue SET version_id = ?1 WHERE id = ?2", (version_id, id))?;
        record_change(tx, id, &FieldChange::reference("version", name_in(tx, "version", current.version_id)?, name_in(tx, "version", version_id)?))?;
        changed.push("version");
    }

    if let Some(points) = patch.points.filter(|points| *points != current.points) {
        tx.execute("UPDATE issue SET points = ?1 WHERE id = ?2", (clean_points(points)?, id))?;
        record_change(
            tx,
            id,
            &FieldChange::text("points", current.points.map(|points| points.to_string()), points.map(|points| points.to_string())),
        )?;
        changed.push("points");
    }

    if let Some(due_date) = &patch.due_date {
        let due_date = clean_date(due_date.as_deref())?;

        if due_date != current.due_date {
            tx.execute("UPDATE issue SET due_date = ?1 WHERE id = ?2", (&due_date, id))?;
            record_change(tx, id, &FieldChange::text("due", current.due_date.clone(), due_date))?;
            changed.push("due");
        }
    }

    if let Some(estimate) = patch.estimate_minutes.filter(|estimate| *estimate != current.estimate_minutes) {
        let format = |minutes: Option<i64>| minutes.map(hub_core::text::format_duration);

        tx.execute("UPDATE issue SET estimate_minutes = ?1 WHERE id = ?2", (estimate.filter(|minutes| *minutes > 0), id))?;
        record_change(tx, id, &FieldChange::text("estimate", format(current.estimate_minutes), format(estimate)))?;
        changed.push("estimate");
    }

    if let Some(labels) = &patch.labels {
        let old = label_names(tx, id)?;
        set_labels(tx, id, &clean_labels(labels))?;
        let new = label_names(tx, id)?;

        if old != new {
            record_change(tx, id, &FieldChange::text("labels", old, new))?;
            changed.push("labels");
        }
    }

    if let Some(component_ids) = &patch.component_ids {
        let old = component_names(tx, id)?;
        set_components(tx, id, project_id, component_ids)?;
        let new = component_names(tx, id)?;

        if old != new {
            record_change(tx, id, &FieldChange::text("components", old, new))?;
            changed.push("components");
        }
    }

    if changed.is_empty() {
        return Ok(changed);
    }

    tx.execute("UPDATE issue SET updated_at = ?1 WHERE id = ?2", (&tx.now, id))?;

    if changed.contains(&"summary") || changed.contains(&"description") {
        reindex_issue(tx, id)?;
    }

    if let Some(payload) = transition {
        tx.emit("issue.transitioned", Some(project_id), Some(id), payload)?;
    }

    if changed.iter().any(|field| *field != "status") {
        let fields: Vec<&str> = changed.iter().copied().filter(|field| *field != "status").collect();
        tx.emit("issue.updated", Some(project_id), Some(id), json!({ "key": current.key, "fields": fields }))?;
    }

    Ok(changed)
}

/// The status of `project_id` a word refers to: an exact status name, or failing that a word
/// for a whole category ("done", "start"), which picks that category's first status.
pub fn status_by_name(connection: &Connection, project_id: Id, name: &str) -> ApiResult<Option<Id>> {
    let name = name.trim();
    let exact: Option<Id> =
        query_one(connection, "SELECT id FROM status WHERE project_id = ?1 AND name = ?2 COLLATE NOCASE", (project_id, name), |row| row.get(0))?;

    if exact.is_some() {
        return Ok(exact);
    }

    let category = match name.to_lowercase().as_str() {
        "done" | "close" | "closed" | "resolve" | "resolved" | "fixed" | "finish" => StatusCategory::Done,
        "start" | "started" | "progress" | "in progress" | "doing" | "wip" => StatusCategory::InProgress,
        "todo" | "to do" | "reopen" | "open" => StatusCategory::Todo,
        _ => return Ok(None),
    };

    query_one(
        connection,
        "SELECT id FROM status WHERE project_id = ?1 AND category = ?2 ORDER BY position, id LIMIT 1",
        (project_id, category.as_str()),
        |row| row.get(0),
    )
}

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

fn map_comment(row: &rusqlite::Row) -> rusqlite::Result<Comment> {
    Ok(Comment {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        author_id: row.get(2)?,
        author_label: row.get(3)?,
        body: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

const COMMENT_COLUMNS: &str = "id, issue_id, author_id, author_label, body, created_at, updated_at";

pub fn load_comments(connection: &Connection, issue_id: Id) -> ApiResult<Vec<Comment>> {
    query_all(connection, &format!("SELECT {COMMENT_COLUMNS} FROM comment WHERE issue_id = ?1 ORDER BY id"), [issue_id], map_comment)
}

fn get_comment(connection: &Connection, id: Id) -> ApiResult<Comment> {
    query_one(connection, &format!("SELECT {COMMENT_COLUMNS} FROM comment WHERE id = ?1"), [id], map_comment)?
        .ok_or_else(|| ApiError::not_found("Comment"))
}

/// Adds a comment written by the transaction's actor.
pub fn add_comment(tx: &Tx, issue_id: Id, body: &str) -> ApiResult<Comment> {
    let body = body.trim();

    if body.is_empty() {
        return Err(ApiError::bad_request("The comment is empty"));
    }

    let current = load_current(tx, issue_id)?;

    tx.execute(
        "INSERT INTO comment (issue_id, author_id, author_label, body, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        (issue_id, tx.actor.user_id, &tx.actor.label, body, &tx.now),
    )?;
    let comment = get_comment(tx, tx.last_insert_rowid())?;
    let author = user_name(tx, tx.actor.user_id)?.or_else(|| tx.actor.label.clone()).unwrap_or_else(|| "Someone".to_string());

    tx.execute("UPDATE issue SET updated_at = ?1 WHERE id = ?2", (&tx.now, issue_id))?;
    let mentioned = notify_mentions(tx, issue_id, body, &format!("{author} mentioned you in {}: {}", current.key, truncate(body, 120)))?;
    notify_watchers(tx, "comment", issue_id, &format!("{author} commented on {}: {}", current.key, truncate(body, 120)), &mentioned)?;
    watch(tx, issue_id, tx.actor.user_id)?;
    reindex_issue(tx, issue_id)?;
    tx.emit("comment.created", Some(current.project_id), Some(issue_id), json!({ "key": current.key, "commentId": comment.id }))?;

    Ok(comment)
}

fn own_comment(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(Comment, Current)> {
    let comment = get_comment(connection, id)?;
    let current = load_current(connection, comment.issue_id)?;
    let access = access::project_access_by_id(connection, user, current.project_id)?;

    if comment.author_id != Some(user.id) && access.require_admin().is_err() {
        return Err(ApiError::forbidden("You can only change your own comments"));
    }

    Ok((comment, current))
}

pub fn update_comment(tx: &Tx, user: &AuthUser, id: Id, body: &str) -> ApiResult<Comment> {
    let (comment, current) = own_comment(tx, user, id)?;
    let body = body.trim();

    if body.is_empty() {
        return Err(ApiError::bad_request("The comment is empty"));
    }

    tx.execute("UPDATE comment SET body = ?1, updated_at = ?2 WHERE id = ?3", (body, &tx.now, id))?;
    reindex_issue(tx, comment.issue_id)?;
    tx.emit("comment.updated", Some(current.project_id), Some(current.id), json!({ "key": current.key, "commentId": id }))?;

    get_comment(tx, id)
}

pub fn delete_comment(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<()> {
    let (comment, current) = own_comment(tx, user, id)?;

    tx.execute("DELETE FROM comment WHERE id = ?1", [id])?;
    reindex_issue(tx, comment.issue_id)?;
    tx.emit("comment.deleted", Some(current.project_id), Some(current.id), json!({ "key": current.key, "commentId": id }))
}

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

pub fn set_watching(connection: &Connection, issue_id: Id, user_id: Id, watching: bool) -> ApiResult<()> {
    if watching {
        watch(connection, issue_id, Some(user_id))
    } else {
        connection.execute("DELETE FROM watcher WHERE issue_id = ?1 AND user_id = ?2", (issue_id, user_id))?;

        Ok(())
    }
}

fn map_worklog(row: &rusqlite::Row) -> rusqlite::Result<Worklog> {
    Ok(Worklog { id: row.get(0)?, issue_id: row.get(1)?, user_id: row.get(2)?, minutes: row.get(3)?, started_at: row.get(4)?, note: row.get(5)? })
}

pub fn load_worklogs(connection: &Connection, issue_id: Id) -> ApiResult<Vec<Worklog>> {
    query_all(
        connection,
        "SELECT id, issue_id, user_id, minutes, started_at, note FROM worklog WHERE issue_id = ?1 ORDER BY started_at, id",
        [issue_id],
        map_worklog,
    )
}

/// Logs time spent by the transaction's actor.
pub fn add_worklog(tx: &Tx, issue_id: Id, minutes: i64, started_at: Option<&str>, note: &str) -> ApiResult<Worklog> {
    if !(1..=24 * 60 * 31).contains(&minutes) {
        return Err(ApiError::bad_request("Time spent must be between a minute and a month"));
    }

    let current = load_current(tx, issue_id)?;
    let started_at = started_at.map(str::trim).filter(|text| !text.is_empty()).unwrap_or(&tx.now);

    tx.execute(
        "INSERT INTO worklog (issue_id, user_id, minutes, started_at, note) VALUES (?1, ?2, ?3, ?4, ?5)",
        (issue_id, tx.actor.user_id, minutes, started_at, note.trim()),
    )?;
    let id = tx.last_insert_rowid();
    tx.execute("UPDATE issue SET updated_at = ?1 WHERE id = ?2", (&tx.now, issue_id))?;
    tx.emit("worklog.added", Some(current.project_id), Some(issue_id), json!({ "key": current.key, "minutes": minutes }))?;

    Ok(tx.query_row("SELECT id, issue_id, user_id, minutes, started_at, note FROM worklog WHERE id = ?1", [id], map_worklog)?)
}

pub fn delete_worklog(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<()> {
    let (issue_id, owner): (Id, Option<Id>) =
        query_one(tx, "SELECT issue_id, user_id FROM worklog WHERE id = ?1", [id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .ok_or_else(|| ApiError::not_found("Work log"))?;
    let current = load_current(tx, issue_id)?;
    let access = access::project_access_by_id(tx, user, current.project_id)?;

    if owner != Some(user.id) && access.require_admin().is_err() {
        return Err(ApiError::forbidden("You can only remove your own work logs"));
    }

    tx.execute("DELETE FROM worklog WHERE id = ?1", [id])?;
    tx.emit("issue.updated", Some(current.project_id), Some(issue_id), json!({ "key": current.key, "fields": ["worklog"] }))
}

fn map_attachment(row: &rusqlite::Row) -> rusqlite::Result<Attachment> {
    Ok(Attachment {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        filename: row.get(2)?,
        mime: row.get(3)?,
        size: row.get(4)?,
        uploader_id: row.get(5)?,
        created_at: row.get(6)?,
    })
}

const ATTACHMENT_COLUMNS: &str = "id, issue_id, filename, mime, size, uploader_id, created_at";

pub fn load_attachments(connection: &Connection, issue_id: Id) -> ApiResult<Vec<Attachment>> {
    query_all(connection, &format!("SELECT {ATTACHMENT_COLUMNS} FROM attachment WHERE issue_id = ?1 ORDER BY id"), [issue_id], map_attachment)
}

fn safe_filename(name: &str) -> String {
    let name: String = name.rsplit(['/', '\\']).next().unwrap_or("").chars().filter(|character| !character.is_control()).take(120).collect();

    if name.trim_matches('.').trim().is_empty() {
        "file".to_string()
    } else {
        name
    }
}

/// Stores an uploaded file under the attachments directory and records it on the issue.
pub fn add_attachment(app: &App, tx: &Tx, issue_id: Id, filename: &str, mime: &str, bytes: &[u8]) -> ApiResult<Attachment> {
    if bytes.is_empty() {
        return Err(ApiError::bad_request("The file is empty"));
    }

    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(ApiError::bad_request("Attachments can be at most 25 MB"));
    }

    let current = load_current(tx, issue_id)?;
    let filename = safe_filename(filename);
    let stored = format!("{}-{}", &crate::auth::random_token()?[..16], issue_id);
    let directory = app.config.attachments_dir();

    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join(&stored), bytes)?;

    tx.execute(
        "INSERT INTO attachment (issue_id, filename, mime, size, path, uploader_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (issue_id, &filename, mime, bytes.len() as i64, &stored, tx.actor.user_id, &tx.now),
    )?;
    let id = tx.last_insert_rowid();

    tx.execute("UPDATE issue SET updated_at = ?1 WHERE id = ?2", (&tx.now, issue_id))?;
    record_change(tx, issue_id, &FieldChange::text("attachment", None, Some(filename)))?;
    tx.emit("issue.updated", Some(current.project_id), Some(issue_id), json!({ "key": current.key, "fields": ["attachments"] }))?;

    Ok(tx.query_row(&format!("SELECT {ATTACHMENT_COLUMNS} FROM attachment WHERE id = ?1"), [id], map_attachment)?)
}

/// An attachment and where its bytes are, for a caller who may see the issue.
pub fn attachment_file(app: &App, connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(Attachment, PathBuf)> {
    let (attachment, path): (Attachment, String) =
        query_one(connection, &format!("SELECT {ATTACHMENT_COLUMNS}, path FROM attachment WHERE id = ?1"), [id], |row| {
            Ok((map_attachment(row)?, row.get(7)?))
        })?
        .ok_or_else(|| ApiError::not_found("Attachment"))?;
    let current = load_current(connection, attachment.issue_id)?;
    access::project_access_by_id(connection, user, current.project_id).map_err(|_| ApiError::not_found("Attachment"))?;

    Ok((attachment, app.config.attachments_dir().join(path)))
}

pub fn delete_attachment(app: &App, tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<PathBuf> {
    let (attachment, path) = attachment_file(app, tx, user, id)?;
    let current = load_current(tx, attachment.issue_id)?;
    access::project_access_by_id(tx, user, current.project_id)?.require_edit()?;

    tx.execute("DELETE FROM attachment WHERE id = ?1", [id])?;
    record_change(tx, attachment.issue_id, &FieldChange::text("attachment", Some(attachment.filename), None))?;
    tx.emit("issue.updated", Some(current.project_id), Some(current.id), json!({ "key": current.key, "fields": ["attachments"] }))?;

    Ok(path)
}

/// Everything on an issue's page. The independent parts are read in parallel, each on its
/// own connection.
pub fn issue_detail(app: &App, user: &AuthUser, key: &str) -> ApiResult<Issue> {
    let (id, row) = app.read(|connection| {
        let (id, _) = issue_access(connection, user, key)?;

        Ok((id, row_by_id(app, connection, id)?))
    })?;

    let ((comments, changes), ((links, children), (rest, panels))) = rayon::join(
        || rayon::join(|| app.read(|connection| load_comments(connection, id)), || app.read(|connection| load_changes(connection, id))),
        || {
            rayon::join(
                || {
                    rayon::join(
                        || app.read(|connection| load_links(app, connection, id)),
                        || app.read(|connection| load_rows(app, connection, "WHERE i.parent_id = ? ORDER BY i.rank", vec![Value::Integer(id)])),
                    )
                },
                || {
                    rayon::join(
                        || {
                            app.read(|connection| {
                                let (description, estimate): (String, Option<i64>) =
                                    connection.query_row("SELECT description, estimate_minutes FROM issue WHERE id = ?1", [id], |row| {
                                        Ok((row.get(0)?, row.get(1)?))
                                    })?;
                                let watchers: Vec<Id> =
                                    query_all(connection, "SELECT user_id FROM watcher WHERE issue_id = ?1", [id], |row| row.get(0))?;

                                Ok((description, estimate, watchers, load_attachments(connection, id)?, load_worklogs(connection, id)?))
                            })
                        },
                        || {
                            app.read(|connection| {
                                let mut panels = std::collections::BTreeMap::new();

                                for module in app.enabled_modules() {
                                    if let Some(panel) = module.issue_panel(app, connection, &row, user)? {
                                        panels.insert(module.meta().id.to_string(), panel);
                                    }
                                }

                                Ok(panels)
                            })
                        },
                    )
                },
            )
        },
    );
    let (description, estimate_minutes, watchers, attachments, worklogs) = rest?;

    Ok(Issue {
        row,
        description,
        estimate_minutes,
        spent_minutes: worklogs.iter().map(|worklog| worklog.minutes).sum(),
        comments: comments?,
        changes: changes?,
        links: links?,
        children: children?,
        attachments,
        worklogs,
        watchers,
        panels: panels?,
    })
}

// ---------- imports from other trackers ----------

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
    let project_key: String = query_one(tx, "SELECT key FROM project WHERE id = ?1", [project_id], |row| row.get(0))?
        .ok_or_else(|| ApiError::not_found("Project"))?;

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
