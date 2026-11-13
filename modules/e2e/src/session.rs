//! What a result remembers of its session: from the report, per attempt, the steps with what
//! each did and saw, the app's log, the error and what was on screen when it landed, and the
//! files the report names. It is kept as one JSON text on the result's row, bounded in size,
//! and read back for the session view with each named file matched to its upload.

use std::collections::{BTreeMap, HashMap};

use hub_app::access::project_access_by_id;
use hub_app::db::{query_all, query_one};
use hub_app::{ApiError, ApiResult, AuthUser};
use hub_core::Id;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{run_by_id, run_result};
use crate::report::{clean, cut, whole, AttemptView, DocArtifact, DocError, DocFailure, DocLogLine, DocStep};
use crate::types::{
    E2eSession, E2eSessionArtifact, E2eSessionAttempt, E2eSessionError, E2eSessionEvent, E2eSessionFailure, E2eSessionLogLine, E2eSessionStep,
};

/// A report of a long agent-driven suite carries megabytes per test; a result keeps this much.
const MAX_SESSION_BYTES: usize = 1024 * 1024;
const MAX_STEPS: usize = 1500;
const MAX_EVENTS: usize = 30;
const MAX_CHANGES: usize = 12;
const MAX_LOG_LINES: usize = 400;
const MAX_ARTIFACTS: usize = 200;
const MAX_CANDIDATES: usize = 20;
const MAX_TEXT_CHARS: usize = 4000;
const MAX_LABEL_CHARS: usize = 500;
const MAX_LINE_CHARS: usize = 600;
/// How much of the report's result id names the test's directory under `results/`.
const RESULT_DIGEST_CHARS: usize = 16;
const TRACE_PAGE: &str = "trace.md";

/// The session of one result as it is stored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    /// The report's id of the result. Its first characters end the name of the test's
    /// directory, which is how an uploaded `trace.md` finds its result.
    pub report_id: Option<String>,
    pub attempts: Vec<E2eSessionAttempt>,
}

impl Session {
    fn count(&self, kind: &str) -> i64 {
        self.attempts.iter().flat_map(|attempt| attempt.artifacts.iter()).filter(|artifact| artifact.kind == kind).count() as i64
    }

    pub fn videos(&self) -> i64 {
        self.count("video")
    }

    pub fn screenshots(&self) -> i64 {
        self.count("screenshot")
    }

    /// The session as stored text, made to fit [`MAX_SESSION_BYTES`]: what a step did and saw
    /// goes first, then the later steps of each attempt.
    pub fn to_stored(&self) -> ApiResult<String> {
        let mut text = compact_json(self)?;

        if text.len() <= MAX_SESSION_BYTES {
            return Ok(text);
        }

        let mut slim = self.clone();

        for step in slim.attempts.iter_mut().flat_map(|attempt| attempt.steps.iter_mut()) {
            step.more_events += step.events.len() as i64;
            step.more_changes += step.changes.len() as i64;
            step.events.clear();
            step.changes.clear();
            step.explanation = None;
        }

        text = compact_json(&slim)?;

        while text.len() > MAX_SESSION_BYTES && slim.attempts.iter().any(|attempt| !attempt.steps.is_empty() || !attempt.app_log.is_empty()) {
            for attempt in &mut slim.attempts {
                let keep = attempt.steps.len() / 2;
                attempt.more_steps += (attempt.steps.len() - keep) as i64;
                attempt.steps.truncate(keep);

                let keep = attempt.app_log.len() / 2;
                attempt.more_app_log += (attempt.app_log.len() - keep) as i64;
                attempt.app_log.truncate(keep);
            }

            text = compact_json(&slim)?;
        }

        Ok(text)
    }
}

/// JSON without the fields that say nothing (null, false, zero, empty): every field of the
/// session has such a default, and a session is mostly those.
fn compact_json(session: &Session) -> ApiResult<String> {
    fn prune(value: &mut Value) -> bool {
        match value {
            Value::Null | Value::Bool(false) => false,
            Value::Number(number) => number.as_f64() != Some(0.0),
            Value::String(text) => !text.is_empty(),
            Value::Array(items) => {
                // Positions in a list mean something, so its items stay, pruned inside.
                items.iter_mut().for_each(|item| {
                    prune(item);
                });

                !items.is_empty()
            }
            Value::Object(fields) => {
                fields.retain(|_, field| prune(field));

                !fields.is_empty()
            }
            Value::Bool(true) => true,
        }
    }

    let mut value = serde_json::to_value(session)?;
    prune(&mut value);

    Ok(value.to_string())
}

fn error_of(error: &DocError) -> E2eSessionError {
    let details = error.details.as_ref();
    let source = error.source.as_ref();

    E2eSessionError {
        code: clean(error.code.as_deref(), 100),
        message: clean(error.message.as_deref(), MAX_TEXT_CHARS).unwrap_or_default(),
        locator: details.and_then(|details| clean(details.locator.as_deref(), MAX_LABEL_CHARS)),
        expected: details.and_then(|details| clean(details.expected.as_deref(), MAX_TEXT_CHARS)),
        observed: details.and_then(|details| clean(details.observed.as_deref(), MAX_TEXT_CHARS)),
        file: source.and_then(|source| clean(source.file.as_deref(), 300)),
        line: source.and_then(|source| source.line).map(|line| whole(Some(line))).filter(|line| *line > 0),
    }
}

fn step_of(step: &DocStep, position: usize) -> E2eSessionStep {
    // What a person reads under a step: what the engine did, where the app went, what an
    // assertion saw while it waited. Model turns and policy checks stay in the report.
    let told: Vec<_> = step
        .events
        .iter()
        .filter(|event| matches!(event.kind.as_deref(), Some("engine" | "navigation" | "poll")))
        .filter(|event| event.detail.as_deref().is_some_and(|detail| !detail.trim().is_empty()))
        .collect();
    let source = step.source.as_ref();
    let screen = step.screen.as_ref();
    let changes = screen.map(|screen| screen.changes.as_slice()).unwrap_or(&[]);

    E2eSessionStep {
        index: step.index.map(|index| whole(Some(index))).unwrap_or(position as i64),
        api: clean(step.api.as_deref(), 100).unwrap_or_default(),
        label: clean(step.label.as_deref(), MAX_LABEL_CHARS).unwrap_or_default(),
        status: clean(step.status.as_deref(), 30).unwrap_or_default(),
        started_at: step.started_at.as_deref().and_then(crate::report::normalize_time),
        duration_ms: whole(step.duration_ms),
        file: source.and_then(|source| clean(source.file.as_deref(), 300)),
        line: source.and_then(|source| source.line).map(|line| whole(Some(line))).filter(|line| *line > 0),
        phase: clean(step.phase.as_deref(), 30),
        error: step.error.as_ref().map(error_of),
        explanation: clean(step.explanation.as_deref(), MAX_TEXT_CHARS),
        events: told
            .iter()
            .take(MAX_EVENTS)
            .map(|event| E2eSessionEvent {
                kind: event.kind.clone().unwrap_or_default(),
                detail: cut(event.detail.as_deref().unwrap_or("").trim(), MAX_LINE_CHARS),
                failed: event.status.as_deref() == Some("failed"),
                duration_ms: whole(event.duration_ms),
            })
            .collect(),
        more_events: told.len().saturating_sub(MAX_EVENTS) as i64,
        location: screen.and_then(|screen| clean(screen.location.as_deref(), MAX_LABEL_CHARS)),
        changes: changes.iter().take(MAX_CHANGES).map(|line| cut(line, MAX_LINE_CHARS)).collect(),
        more_changes: changes.len().saturating_sub(MAX_CHANGES) as i64 + whole(screen.and_then(|screen| screen.more)),
        artifacts: step.artifacts.iter().take(MAX_ARTIFACTS).map(|id| cut(id, 200)).collect(),
    }
}

fn log_line_of(line: &DocLogLine) -> E2eSessionLogLine {
    E2eSessionLogLine {
        level: clean(line.level.as_deref(), 20).unwrap_or_else(|| "log".to_string()),
        text: cut(line.text.as_deref().unwrap_or(""), MAX_LINE_CHARS),
        at: line.at.as_deref().and_then(crate::report::normalize_time),
        step: line.step.map(|step| whole(Some(step))),
    }
}

/// The newest lines when the app logged more than is kept, but never at the cost of an error
/// or a warning: those are what a reader looks for.
fn app_log_of(lines: &[DocLogLine]) -> (Vec<E2eSessionLogLine>, i64) {
    if lines.len() <= MAX_LOG_LINES {
        return (lines.iter().map(log_line_of).collect(), 0);
    }

    let loud = |line: &DocLogLine| matches!(line.level.as_deref(), Some("error" | "warn" | "warning"));
    let mut room = MAX_LOG_LINES.saturating_sub(lines.iter().filter(|line| loud(line)).count());
    let mut keep = vec![false; lines.len()];

    for (position, line) in lines.iter().enumerate().rev() {
        if loud(line) {
            keep[position] = true;
        } else if room > 0 {
            keep[position] = true;
            room -= 1;
        }
    }

    let kept: Vec<E2eSessionLogLine> =
        lines.iter().zip(&keep).filter(|(_, keep)| **keep).map(|(line, _)| log_line_of(line)).take(MAX_LOG_LINES).collect();
    let dropped = (lines.len() - kept.len()) as i64;

    (kept, dropped)
}

fn artifact_of(artifact: &DocArtifact) -> Option<E2eSessionArtifact> {
    Some(E2eSessionArtifact {
        report_id: clean(artifact.id.as_deref(), 200)?,
        kind: clean(artifact.kind.as_deref(), 30).unwrap_or_else(|| "download".to_string()),
        path: clean(artifact.path.as_deref(), MAX_LABEL_CHARS),
        // Shown as a link, so nothing but a web address is kept.
        url: clean(artifact.url.as_deref(), 1000).filter(|url| url.starts_with("https://") || url.starts_with("http://")),
        started_at: artifact.started_at.as_deref().and_then(crate::report::normalize_time),
        artifact_id: None,
        size: None,
    })
}

fn failure_of(failure: &DocFailure) -> E2eSessionFailure {
    E2eSessionFailure {
        url: clean(failure.url.as_deref(), 1000),
        screenshot: clean(failure.screenshot.as_deref(), 200),
        screen: clean(failure.screen.as_deref(), 200),
        nodes: failure.nodes.map(|nodes| whole(Some(nodes))),
        candidates: failure.candidates.iter().take(MAX_CANDIDATES).map(|line| cut(line, MAX_LINE_CHARS)).collect(),
    }
}

fn environment_of(value: Option<&Value>) -> BTreeMap<String, String> {
    let Some(Value::Object(fields)) = value else { return BTreeMap::new() };

    fields.iter().take(20).filter_map(|(name, value)| Some((cut(name, 100), cut(value.as_str()?, MAX_LABEL_CHARS)))).collect()
}

fn attempt_of(view: &AttemptView) -> E2eSessionAttempt {
    let (app_log, more_app_log) = app_log_of(view.app_log);

    E2eSessionAttempt {
        index: view.index,
        status: cut(view.status, 30),
        started_at: view.started_at.and_then(crate::report::normalize_time),
        duration_ms: view.duration_ms,
        error: view.error.map(error_of),
        also_failed: view.secondary_errors.iter().take(10).map(|error| error_of(error)).collect(),
        steps: view.steps.iter().take(MAX_STEPS).enumerate().map(|(position, step)| step_of(step, position)).collect(),
        more_steps: view.steps.len().saturating_sub(MAX_STEPS) as i64,
        app_log,
        more_app_log,
        failure: view.failure.map(failure_of),
        artifacts: view.artifacts.iter().take(MAX_ARTIFACTS).filter_map(artifact_of).collect(),
        environment: environment_of(view.environment),
    }
}

/// The session of a result with these attempts; `report_id` is the report's id of the result.
pub(crate) fn build(report_id: Option<&str>, views: &[AttemptView]) -> Session {
    Session { report_id: clean(report_id, 200), attempts: views.iter().map(attempt_of).collect() }
}

/// Whether an uploaded file is the trace page of the result with this report id: the
/// framework writes it as `<words of the test>-<start of the id>/trace.md`.
fn is_trace_of(path: &str, report_id: &str) -> bool {
    let Some(digest) = report_id.get(..RESULT_DIGEST_CHARS) else { return false };
    let mut segments = path.rsplit('/');

    segments.next() == Some(TRACE_PAGE) && segments.next().is_some_and(|directory| directory.ends_with(digest))
}

/// Everything the session view shows of one result, for a caller who may see its project.
pub fn load(connection: &Connection, user: &AuthUser, result_id: Id) -> ApiResult<E2eSession> {
    let found: Option<(Id, Id, Option<String>, String)> = query_one(
        connection,
        "SELECT e.run_id, r.project_id, e.session, t.tags FROM e2e_result e JOIN e2e_run r ON r.id = e.run_id JOIN e2e_test t ON t.id = e.test_id
         WHERE e.id = ?1",
        [result_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let (run_id, project_id, stored, tags) = found.ok_or_else(|| ApiError::not_found("Result"))?;
    let access = project_access_by_id(connection, user, project_id).map_err(|_| ApiError::not_found("Result"))?;
    let (run, _) = run_by_id(connection, run_id)?.ok_or_else(|| ApiError::not_found("Result"))?;
    let result = run_result(connection, user, result_id)?.ok_or_else(|| ApiError::not_found("Result"))?;
    let mut session: Session = stored.as_deref().and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default();

    let uploaded: HashMap<String, (Id, i64)> =
        query_all(connection, "SELECT path, id, size FROM e2e_artifact WHERE run_id = ?1", [run_id], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?
            .into_iter()
            .collect();

    for artifact in session.attempts.iter_mut().flat_map(|attempt| attempt.artifacts.iter_mut()) {
        if let Some((id, size)) = artifact.path.as_ref().and_then(|path| uploaded.get(path)) {
            artifact.artifact_id = Some(*id);
            artifact.size = Some(*size);
        }
    }

    let trace_artifact_id = session
        .report_id
        .as_deref()
        .and_then(|report_id| uploaded.iter().find(|(path, _)| is_trace_of(path, report_id)))
        .map(|(_, (id, _))| *id);
    let job: Option<(Id, String)> =
        query_one(connection, "SELECT id, state FROM e2e_job WHERE run_id = ?1 ORDER BY id DESC LIMIT 1", [run_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let bug_type_id =
        query_one(connection, "SELECT id FROM issue_type WHERE project_id = ?1 AND name = 'Bug' AND level = 'standard'", [project_id], |row| row.get(0))?;

    Ok(E2eSession {
        run,
        result,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        attempts: session.attempts,
        trace_artifact_id,
        job_id: job.as_ref().map(|(id, _)| *id),
        job_active: job.is_some_and(|(_, state)| matches!(state.as_str(), "claimed" | "running")),
        can_edit: access.can_edit(),
        bug_type_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_page_finds_its_result_by_the_directory_name() {
        let id = "1ee7d924694b89412ddabb00f97503019c95bbcd1c4c247eae21c77a5b5e5423";

        assert!(is_trace_of("board-web-9-the-card-of-an-issue-opens-1ee7d924694b8941/trace.md", id));
        assert!(!is_trace_of("board-the-board-shows-the-active-659c9e2a94191104/trace.md", id));
        assert!(!is_trace_of("board-web-9-the-card-of-an-issue-opens-1ee7d924694b8941/attempt-1/trace.md", id));
        assert!(!is_trace_of("trace.md", id));
        assert!(!is_trace_of("x-1ee7/trace.md", "1ee7"));
    }

    #[test]
    fn a_session_too_large_to_keep_loses_detail_before_steps() {
        let step = |index: i64| E2eSessionStep {
            index,
            api: "agent.act".to_string(),
            label: "x".repeat(400),
            events: (0..20).map(|_| E2eSessionEvent { kind: "engine".to_string(), detail: "y".repeat(500), ..Default::default() }).collect(),
            ..Default::default()
        };
        let session = Session { report_id: None, attempts: vec![E2eSessionAttempt { steps: (0..1500).map(step).collect(), ..Default::default() }] };
        let stored = session.to_stored().unwrap();
        let back: Session = serde_json::from_str(&stored).unwrap();

        assert!(stored.len() <= MAX_SESSION_BYTES);
        // The detail went, and says how much; every step is still there.
        assert_eq!((back.attempts[0].steps.len(), back.attempts[0].more_steps), (1500, 0));
        assert_eq!((back.attempts[0].steps[7].events.len(), back.attempts[0].steps[7].more_events), (0, 20));
        assert_eq!(back.attempts[0].steps[7].index, 7);
    }

    #[test]
    fn stored_text_leaves_out_what_says_nothing() {
        let session = Session { report_id: Some("abc".to_string()), attempts: vec![E2eSessionAttempt { status: "passed".to_string(), ..Default::default() }] };
        let stored = session.to_stored().unwrap();

        assert_eq!(stored, r#"{"attempts":[{"status":"passed"}],"reportId":"abc"}"#);
        assert_eq!(serde_json::from_str::<Session>(&stored).unwrap(), session);
    }
}
