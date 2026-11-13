//! Issue rows: the shape lists, boards and searches show, with the badges modules add.

use hub_core::keys::parse_issue_key;
use hub_core::{Id, IssueRow, SubtaskTally};
use rusqlite::types::Value;
use rusqlite::Connection;

use crate::access::{self, ProjectAccess};
use crate::app::App;
use crate::auth::AuthUser;
use crate::db::{placeholders, query_all, query_one};
use crate::error::{ApiError, ApiResult};

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

/// Lets every enabled module add its badges to the rows, except where a project has the
/// module switched off.
pub fn decorate(app: &App, connection: &Connection, rows: &mut [IssueRow]) -> ApiResult<()> {
    if rows.is_empty() {
        return Ok(());
    }

    for module in app.enabled_modules() {
        module.decorate_issues(connection, rows)?;
    }

    crate::services::project_modules::drop_disabled(connection, rows)
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
