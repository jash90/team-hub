//! Keeps `gh_drift` — where an issue's status and its code disagree — up to date.

use chrono::Utc;
use hub_app::db::query_all;
use hub_app::ApiResult;
use hub_core::dev::{classify_drift_many, DevSummary, DriftConfig, DriftInput};
use hub_core::{Id, StatusCategory};
use rusqlite::Connection;

use crate::model::{dev_summaries, id_list};

/// Recomputes drift for `issue_ids`. Only issues of projects with a connected repository
/// are judged: without one, "in progress with no code" would be true of everything. Epics
/// and subtasks are not judged either; the code belongs to the issue between them.
pub fn recompute_drift(connection: &Connection, issue_ids: &[Id]) -> ApiResult<()> {
    if issue_ids.is_empty() {
        return Ok(());
    }

    let ids = id_list(issue_ids);
    let issues = query_all(
        connection,
        &format!(
            "SELECT i.id, s.category, i.status_since FROM issue i JOIN status s ON s.id = i.status_id
             WHERE i.id IN ({ids}) AND i.project_id IN (SELECT project_id FROM gh_repo)
               AND i.type_id IN (SELECT id FROM issue_type WHERE level = 'standard')"
        ),
        [],
        |row| Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
    )?;
    let summaries = dev_summaries(connection, issue_ids)?;
    let nothing = DevSummary::default();
    let inputs: Vec<DriftInput> = issues
        .iter()
        .map(|(id, category, since)| DriftInput {
            category: StatusCategory::parse(category).unwrap_or(StatusCategory::Todo),
            in_status_since: since,
            dev: summaries.get(id).unwrap_or(&nothing),
        })
        .collect();
    let drift = classify_drift_many(&inputs, DriftConfig::default(), Utc::now());

    connection.execute(&format!("DELETE FROM gh_drift WHERE issue_id IN ({ids})"), [])?;

    for ((issue_id, _, _), kinds) in issues.iter().zip(drift) {
        for kind in kinds {
            connection.execute("INSERT OR IGNORE INTO gh_drift (issue_id, kind) VALUES (?1, ?2)", (issue_id, kind.as_str()))?;
        }
    }

    Ok(())
}

/// Recomputes drift for every open or recently changed issue of the projects that have a
/// repository; some kinds of drift appear with the mere passing of time.
pub fn recompute_all(connection: &Connection) -> ApiResult<usize> {
    let issue_ids: Vec<Id> = query_all(
        connection,
        "SELECT i.id FROM issue i JOIN status s ON s.id = i.status_id
         WHERE i.project_id IN (SELECT project_id FROM gh_repo)
           AND (s.category != 'done' OR i.id IN (SELECT issue_id FROM gh_ref) OR i.id IN (SELECT issue_id FROM gh_drift))",
        [],
        |row| row.get(0),
    )?;

    recompute_drift(connection, &issue_ids)?;

    Ok(issue_ids.len())
}
