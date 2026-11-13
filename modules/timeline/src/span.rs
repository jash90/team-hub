//! Setting when an issue's work runs: the start in this module's table, the due date through
//! the tracker, so that it lands in the issue's history like any other change of it.

use hub_app::db::query_one;
use hub_app::services::history::clean_date;
use hub_app::services::issues::update_issue;
use hub_app::{ApiError, ApiResult, Tx};
use hub_core::{Id, IssuePatch};
use serde_json::json;

/// Sets both ends of an issue's span; none clears one. Announces `timeline.updated` and says
/// whether anything changed. The caller has checked that the actor may edit the issue.
pub fn set_span(tx: &Tx, issue_id: Id, start: Option<&str>, due: Option<&str>) -> ApiResult<bool> {
    let start = clean_date(start)?;
    let due = clean_date(due)?;

    if start.as_ref().zip(due.as_ref()).is_some_and(|(start, due)| start > due) {
        return Err(ApiError::bad_request("The start date is after the due date"));
    }

    let (key, project_id): (String, Id) =
        query_one(tx, "SELECT key, project_id FROM issue WHERE id = ?1", [issue_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .ok_or_else(|| ApiError::not_found("Issue"))?;
    let stored: Option<String> = query_one(tx, "SELECT start_date FROM timeline_span WHERE issue_id = ?1", [issue_id], |row| row.get(0))?;

    match &start {
        Some(start) => tx.execute(
            "INSERT INTO timeline_span (issue_id, start_date) VALUES (?1, ?2) ON CONFLICT (issue_id) DO UPDATE SET start_date = excluded.start_date",
            (issue_id, start),
        )?,
        None => tx.execute("DELETE FROM timeline_span WHERE issue_id = ?1", [issue_id])?,
    };

    let due_changed = !update_issue(tx, issue_id, &IssuePatch { due_date: Some(due.clone()), ..Default::default() })?.is_empty();

    if stored == start && !due_changed {
        return Ok(false);
    }

    tx.emit("timeline.updated", Some(project_id), Some(issue_id), json!({ "key": key, "start": start, "due": due }))?;

    Ok(true)
}
