//! The `jira_job` table: one row per import, updated as the import goes, so its progress
//! survives the page that started it.

use hub_app::db::{now_iso, query_all, query_one};
use hub_app::{ApiError, ApiResult, App, Tx};
use hub_core::Id;
use rusqlite::Connection;
use serde_json::json;

use crate::types::{JiraJob, JiraJobState, JiraJobStep, JiraReport};

const COLUMNS: &str = "id, kind, project_key, state, step, total, processed, error, started_by, created_at, finished_at, report";
pub const EVENT: &str = "jira.job";

fn map_job(row: &rusqlite::Row) -> rusqlite::Result<JiraJob> {
    let state: String = row.get(3)?;
    let step: String = row.get(4)?;
    let report: String = row.get(11)?;

    Ok(JiraJob {
        id: row.get(0)?,
        kind: row.get(1)?,
        project_key: row.get(2)?,
        state: JiraJobState::parse(&state),
        step: JiraJobStep::parse(&step),
        total: row.get(5)?,
        processed: row.get(6)?,
        error: row.get(7)?,
        started_by: row.get(8)?,
        created_at: row.get(9)?,
        finished_at: row.get(10)?,
        report: serde_json::from_str(&report).unwrap_or_default(),
    })
}

pub fn get(connection: &Connection, id: Id) -> ApiResult<JiraJob> {
    query_one(connection, &format!("SELECT {COLUMNS} FROM jira_job WHERE id = ?1"), [id], map_job)?.ok_or_else(|| ApiError::not_found("Import"))
}

pub fn list(connection: &Connection, limit: i64) -> ApiResult<Vec<JiraJob>> {
    query_all(connection, &format!("SELECT {COLUMNS} FROM jira_job ORDER BY id DESC LIMIT ?1"), [limit], map_job)
}

/// Queues a job. One import runs at a time: two would wait for the same writer anyway, and
/// two of the same project would trip over each other.
pub fn create(tx: &Tx, kind: &str, project_key: &str, started_by: Id) -> ApiResult<JiraJob> {
    let busy: Option<String> =
        query_one(tx, "SELECT project_key FROM jira_job WHERE state IN ('queued', 'running') LIMIT 1", [], |row| row.get(0))?;

    if let Some(key) = busy {
        return Err(ApiError::conflict(format!("An import of {key} is still running; wait for it or cancel it")));
    }

    tx.execute(
        "INSERT INTO jira_job (kind, project_key, state, step, started_by, created_at) VALUES (?1, ?2, 'queued', 'waiting', ?3, ?4)",
        (kind, project_key, started_by, &tx.now),
    )?;

    let job = get(tx, tx.last_insert_rowid())?;
    announce(tx, &job)?;

    Ok(job)
}

/// Tells the open pages that a job moved. The event names no project: the import page is for
/// administrators, who ask for the details themselves.
fn announce(tx: &Tx, job: &JiraJob) -> ApiResult<()> {
    tx.emit(
        EVENT,
        None,
        None,
        json!({ "jobId": job.id, "projectKey": job.project_key, "state": job.state, "step": job.step, "processed": job.processed, "total": job.total }),
    )
}

/// Records where a running job is, and announces it.
pub fn progress(tx: &Tx, id: Id, step: JiraJobStep, total: Option<i64>, processed: i64, report: &JiraReport) -> ApiResult<()> {
    tx.execute(
        "UPDATE jira_job SET state = 'running', step = ?1, total = ?2, processed = ?3, created = ?4, updated = ?5, unchanged = ?6,
                skipped = ?7, failed = ?8, report = ?9
         WHERE id = ?10 AND state IN ('queued', 'running')",
        rusqlite::params![
            step.as_str(),
            total,
            processed,
            report.created,
            report.updated,
            report.unchanged,
            report.skipped.len() as i64,
            report.failed.len() as i64,
            serde_json::to_string(report)?,
            id
        ],
    )?;

    announce(tx, &get(tx, id)?)
}

/// Ends a job: done, failed with a reason, or cancelled.
pub fn finish(app: &App, id: Id, state: JiraJobState, error: Option<&str>) -> ApiResult<()> {
    app.write(hub_app::Actor::system("Jira"), |tx| {
        tx.execute(
            "UPDATE jira_job SET state = ?1, step = 'finished', error = ?2, finished_at = ?3 WHERE id = ?4 AND state IN ('queued', 'running')",
            (state.as_str(), error, &tx.now, id),
        )?;

        announce(tx, &get(tx, id)?)
    })
}

pub fn cancel_requested(connection: &Connection, id: Id) -> ApiResult<bool> {
    Ok(query_one(connection, "SELECT cancel FROM jira_job WHERE id = ?1", [id], |row| row.get::<_, bool>(0))?.unwrap_or(true))
}

/// Asks a job to stop after the page of issues it is on. What it has imported stays.
pub fn request_cancel(connection: &Connection, id: Id) -> ApiResult<()> {
    let changed = connection.execute("UPDATE jira_job SET cancel = 1 WHERE id = ?1 AND state IN ('queued', 'running')", [id])?;

    if changed == 0 {
        get(connection, id)?;

        return Err(ApiError::conflict("That import has already finished"));
    }

    Ok(())
}

/// Jobs that were running when the server stopped did not finish, and nothing will finish
/// them. Importing again continues where they left off.
pub fn fail_interrupted(connection: &Connection) -> ApiResult<usize> {
    Ok(connection.execute(
        "UPDATE jira_job SET state = 'failed', step = 'finished', finished_at = ?1,
                error = 'The server stopped while this import was running. Run it again: what was already imported is kept.'
         WHERE state IN ('queued', 'running')",
        [now_iso()],
    )?)
}
