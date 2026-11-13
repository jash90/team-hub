//! Tests (e2e): runs of the e2e testing framework arrive as reports, each test is linked to
//! the issues it verifies, and the issue, the board and a project page show whether the
//! behaviour an issue describes actually works.

pub mod artifacts;
pub mod ingest;
pub mod jobs;
pub mod model;
pub mod report;
pub mod routes;
pub mod runners;
mod schema;
mod seed;
pub mod session;
pub mod settings;
pub mod types;

use std::time::{Duration, Instant};

use axum::Router;
use hub_app::access::project_access;
use hub_app::services::issues::{issue_access, row_by_id};
use hub_app::{ApiError, ApiResult, App, AuthUser, Event, FieldSql, Module, ModuleMeta, QueryScope, ToolSpec};
use hub_core::query::{Clause, ClauseOp, QueryFieldInfo, QueryValue};
use hub_core::{Id, IssueRow};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::model::{badges, issue_state_sql, project_runs, project_tests};
use crate::types::{E2eIssuePanel, E2eIssueState, E2eStatus, E2eTest};

pub struct E2eModule;

/// How often the background thread looks for jobs whose runner went quiet.
const CHECK_JOBS_EVERY: Duration = Duration::from_secs(30);
const SWEEP_EVERY: Duration = Duration::from_secs(3600);

/// `tests = failing`, `tests in (failing, flaky)`, `tests != passing`, `tests is empty`.
/// The value is what the issue's tests say together; `none` is an issue without tests.
fn tests_condition(clause: &Clause) -> ApiResult<FieldSql> {
    let state = issue_state_sql();

    match clause.op {
        ClauseOp::IsEmpty => Ok(FieldSql { sql: format!("{state} = 'none'"), params: Vec::new() }),
        ClauseOp::IsNotEmpty => Ok(FieldSql { sql: format!("{state} != 'none'"), params: Vec::new() }),
        ClauseOp::Eq | ClauseOp::NotEq | ClauseOp::In | ClauseOp::NotIn => {
            let allowed: Vec<&str> = E2eIssueState::ALL.iter().map(|state| state.as_str()).chain(["none"]).collect();
            let words = clause
                .values
                .iter()
                .map(|value| match value {
                    QueryValue::Text(text) => match text.to_lowercase().as_str() {
                        "passed" | "pass" => Ok("passing".to_string()),
                        "failed" | "fail" => Ok("failing".to_string()),
                        word if allowed.contains(&word) => Ok(word.to_string()),
                        word => Err(ApiError::bad_request(format!("\"{word}\" is not one of: {}", allowed.join(", ")))),
                    },
                    _ => Err(ApiError::bad_request(format!("\"{}\" takes a word such as failing or passing", clause.field))),
                })
                .collect::<ApiResult<Vec<String>>>()?;
            let negation = if clause.op.is_negative() { "NOT " } else { "" };
            let list = vec!["?"; words.len().max(1)].join(", ");

            Ok(FieldSql { sql: format!("{state} {negation}IN ({list})"), params: words.into_iter().map(SqlValue::Text).collect() })
        }
        _ => Err(ApiError::bad_request(format!("That operator cannot be used with \"{}\"", clause.field))),
    }
}

/// A test as an agent reads it: what failed, where, and which issues it verifies.
fn brief(test: &E2eTest) -> Value {
    json!({
        "id": test.id,
        "title": test.title,
        "file": test.file,
        "line": test.line,
        "status": test.status,
        "message": test.message,
        "errorCode": test.error_code,
        "lastRunId": test.last_run_id,
        "lastAt": test.last_at,
        "branch": test.branch,
        "commit": test.commit,
        "attempts": test.attempts,
        "recent": test.history.iter().map(|point| point.status).collect::<Vec<_>>(),
        "issues": test.issues.iter().map(|issue| json!({ "key": issue.key, "summary": issue.summary, "status": issue.status, "done": issue.done })).collect::<Vec<_>>(),
    })
}

impl Module for E2eModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: "e2e",
            name: "Tests (e2e)",
            description:
                "Runs of the e2e testing framework, each test linked to the issues it verifies: results on issues, cards and a project page.",
            version: env!("CARGO_PKG_VERSION"),
            depends_on: &[],
            default_enabled: true,
        }
    }

    fn migrations(&self) -> &'static [&'static str] {
        schema::MIGRATIONS
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(routes::routes())
    }

    /// A deleted issue takes its test links with it.
    fn on_event(&self, app: &App, event: &Event) {
        if event.kind != "issue.deleted" {
            return;
        }

        if let Err(error) = app.db.write(|tx| ingest::forget_orphans(tx)) {
            tracing::warn!("could not drop test links of a deleted issue: {}", error.message);
        }
    }

    fn query_fields(&self) -> Vec<QueryFieldInfo> {
        vec![QueryFieldInfo::new("tests", &[], "module", "tests = failing")]
    }

    fn compile_field(&self, _connection: &Connection, clause: &Clause, _scope: &QueryScope) -> Option<ApiResult<FieldSql>> {
        match clause.field.to_lowercase().as_str() {
            "tests" => Some(tests_condition(clause)),
            _ => None,
        }
    }

    fn decorate_issues(&self, connection: &Connection, rows: &mut [IssueRow]) -> ApiResult<()> {
        let ids: Vec<Id> = rows.iter().map(|row| row.id).collect();
        let mut marks = badges(connection, &ids)?;

        for row in rows {
            if let Some(badge) = marks.remove(&row.id) {
                row.ext.insert("e2e".to_string(), serde_json::to_value(badge)?);
            }
        }

        Ok(())
    }

    fn issue_panel(&self, _app: &App, connection: &Connection, issue: &IssueRow, viewer: &AuthUser) -> ApiResult<Option<Value>> {
        Ok(routes::issue_panel(connection, issue, viewer)?.map(serde_json::to_value).transpose()?)
    }

    /// My issues with failing tests.
    fn dashboard(&self, connection: &Connection, viewer: &AuthUser) -> ApiResult<Option<Value>> {
        Ok(routes::dashboard(connection, viewer)?.map(serde_json::to_value).transpose()?)
    }

    fn tools(&self) -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "get_issue_tests",
                description: "The end-to-end tests that verify an issue: each test's last result (passed, failed, flaky, skipped), when it ran, on which branch and commit, the failure message, and its recent history. Use it to check whether the behaviour an issue describes actually works.",
                input_schema: json!({ "type": "object", "properties": { "key": { "type": "string", "description": "Issue key, e.g. WEB-12" } }, "required": ["key"] }),
                read_only: true,
            },
            ToolSpec {
                name: "list_failing_tests",
                description: "The end-to-end tests of a project that fail or are flaky in their latest run, with file and line, failure message, branch and commit, and the issues each one verifies; plus the project's latest run.",
                input_schema: json!({ "type": "object", "properties": { "projectKey": { "type": "string", "description": "Project key, e.g. WEB" } }, "required": ["projectKey"] }),
                read_only: true,
            },
        ]
    }

    fn call_tool(&self, app: &App, user: &AuthUser, name: &str, arguments: &Value) -> Option<ApiResult<Value>> {
        match name {
            "get_issue_tests" => Some(app.read(|connection| {
                let key = arguments["key"].as_str().ok_or_else(|| ApiError::bad_request("\"key\" is required"))?;
                let (issue_id, _) = issue_access(connection, user, key)?;
                let issue = row_by_id(app, connection, issue_id)?;
                let panel = routes::issue_panel(connection, &issue, user)?.unwrap_or(E2eIssuePanel { state: None, tests: Vec::new() });

                Ok(json!({ "key": issue.key, "summary": issue.summary, "state": panel.state, "tests": panel.tests }))
            })),
            "list_failing_tests" => Some(app.read(|connection| {
                let key = arguments["projectKey"].as_str().ok_or_else(|| ApiError::bad_request("\"projectKey\" is required"))?;
                let access = project_access(connection, user, key)?;
                let tests = project_tests(connection, user, access.id)?;
                let with = |status: E2eStatus| tests.iter().filter(|test| test.status == Some(status)).map(brief).collect::<Vec<_>>();

                Ok(json!({
                    "projectKey": access.key,
                    "latestRun": project_runs(connection, access.id, 1)?.into_iter().next(),
                    "tests": tests.len(),
                    "failing": with(E2eStatus::Failed),
                    "flaky": with(E2eStatus::Flaky),
                }))
            })),
            _ => None,
        }
    }

    fn runners(&self, connection: &Connection, viewer: &AuthUser, now: chrono::DateTime<chrono::Utc>) -> ApiResult<Vec<hub_core::runners::Runner>> {
        runners::known(connection, viewer, now)
    }

    /// Jobs whose runner went quiet are failed here when nobody is looking at them, and
    /// files no run names any more are removed.
    fn start(&self, app: &App) {
        let app = app.clone();

        let spawned = std::thread::Builder::new().name("e2e-jobs".to_string()).spawn(move || {
            let mut swept: Option<Instant> = None;

            loop {
                if app.module_enabled("e2e") {
                    if let Err(error) = jobs::fail_stale(&app) {
                        tracing::warn!("could not check for jobs without a runner: {}", error.message);
                    }

                    if swept.is_none_or(|at| at.elapsed() > SWEEP_EVERY) {
                        swept = Some(Instant::now());

                        if let Err(error) = artifacts::sweep(&app) {
                            tracing::warn!("could not sweep test artifacts: {}", error.message);
                        }
                    }
                }

                std::thread::sleep(CHECK_JOBS_EVERY);
            }
        });

        if let Err(error) = spawned {
            tracing::error!("could not start the test jobs thread: {error}");
        }
    }

    fn seed_demo(&self, app: &App) -> ApiResult<()> {
        seed::seed_demo(app)
    }
}
