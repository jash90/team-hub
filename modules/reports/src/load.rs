//! Reads a project's issues and their recorded changes into the plain structs the reports
//! are computed from.

use std::collections::HashMap;

use hub_app::db::query_all;
use hub_app::ApiResult;
use hub_core::{Id, StatusCategory, TypeLevel};
use rusqlite::Connection;

use crate::history::{IssueHistory, ProjectHistory, Step};

struct ChangeRow {
    issue_id: Id,
    field: String,
    old_value: Option<String>,
    new_value: Option<String>,
    old_ref: Option<String>,
    new_ref: Option<String>,
    at: String,
}

/// The id one side of a change refers to. The outer none means the row cannot be used: it
/// names something without saying which.
fn referred(value: &Option<String>, reference: &Option<String>) -> Option<Option<Id>> {
    match (reference, value) {
        (Some(reference), _) => reference.parse().ok().map(Some),
        (None, None) => Some(None),
        (None, Some(_)) => None,
    }
}

fn number(value: &Option<String>) -> Option<f64> {
    value.as_deref().and_then(|text| text.trim().parse().ok())
}

fn apply(issue: &mut IssueHistory, change: ChangeRow) {
    let ids = || Some((referred(&change.old_value, &change.old_ref)?, referred(&change.new_value, &change.new_ref)?));

    match change.field.as_str() {
        "status" => {
            if let Some((old, Some(new))) = ids() {
                issue.status_steps.push(Step { at: change.at, old, new: Some(new) });
            }
        }
        "sprint" => {
            if let Some((old, new)) = ids() {
                issue.sprint_steps.push(Step { at: change.at, old, new });
            }
        }
        "points" => issue.points_steps.push(Step { at: change.at, old: number(&change.old_value), new: number(&change.new_value) }),
        _ => {}
    }
}

/// Every issue of the project with the history of its status, sprint and points.
pub fn project_history(connection: &Connection, project_id: Id) -> ApiResult<ProjectHistory> {
    let categories: HashMap<Id, StatusCategory> =
        query_all(connection, "SELECT id, category FROM status WHERE project_id = ?1", [project_id], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?))
        })?
        .into_iter()
        .filter_map(|(id, category)| Some((id, StatusCategory::parse(&category)?)))
        .collect();

    let mut issues = query_all(
        connection,
        "SELECT i.id, i.key, i.summary, t.level, i.assignee_id, i.created_at, i.status_id, i.status_since, i.sprint_id, i.points
         FROM issue i JOIN issue_type t ON t.id = i.type_id
         WHERE i.project_id = ?1 ORDER BY i.id",
        [project_id],
        |row| {
            let level: String = row.get(3)?;

            Ok(IssueHistory {
                id: row.get(0)?,
                key: row.get(1)?,
                summary: row.get(2)?,
                level: TypeLevel::parse(&level).unwrap_or(TypeLevel::Standard),
                assignee_id: row.get(4)?,
                created_at: row.get(5)?,
                status_id: row.get(6)?,
                status_since: row.get(7)?,
                sprint_id: row.get(8)?,
                points: row.get(9)?,
                status_steps: Vec::new(),
                sprint_steps: Vec::new(),
                points_steps: Vec::new(),
            })
        },
    )?;
    let changes = query_all(
        connection,
        "SELECT c.issue_id, c.field, c.old_value, c.new_value, c.old_ref, c.new_ref, c.created_at
         FROM change c JOIN issue i ON i.id = c.issue_id
         WHERE i.project_id = ?1 AND c.field IN ('status', 'sprint', 'points') ORDER BY c.created_at, c.id",
        [project_id],
        |row| {
            Ok(ChangeRow {
                issue_id: row.get(0)?,
                field: row.get(1)?,
                old_value: row.get(2)?,
                new_value: row.get(3)?,
                old_ref: row.get(4)?,
                new_ref: row.get(5)?,
                at: row.get(6)?,
            })
        },
    )?;
    let positions: HashMap<Id, usize> = issues.iter().enumerate().map(|(position, issue)| (issue.id, position)).collect();

    for change in changes {
        if let Some(issue) = positions.get(&change.issue_id).and_then(|position| issues.get_mut(*position)) {
            apply(issue, change);
        }
    }

    for issue in &mut issues {
        issue.reconcile_status();
    }

    Ok(ProjectHistory { issues, categories })
}

/// The names of the people who have issues in the project.
pub fn assignee_names(connection: &Connection, project_id: Id) -> ApiResult<HashMap<Id, String>> {
    let names =
        query_all(connection, "SELECT id, name FROM user WHERE id IN (SELECT assignee_id FROM issue WHERE project_id = ?1)", [project_id], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?))
        })?;

    Ok(names.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    #[test]
    fn a_change_that_names_something_without_its_id_is_not_guessed_at() {
        assert_eq!(referred(&text("Sprint 4"), &text("12")), Some(Some(12)));
        assert_eq!(referred(&None, &None), Some(None));
        assert_eq!(referred(&text("Sprint 4"), &None), None);
        assert_eq!(referred(&text("Sprint 4"), &text("twelve")), None);
        assert_eq!(number(&text("2.5")), Some(2.5));
        assert_eq!(number(&None), None);
    }
}
