//! The shapes this module sends to the web interface and to agents.

use hub_core::Id;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How one test ended in one run. A test that timed out failed; one the run stopped before a
/// verdict, or that was skipped on purpose, is `skipped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum E2eStatus {
    Passed,
    Failed,
    /// Failed at least once, then passed on a retry.
    Flaky,
    Skipped,
}

impl E2eStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Flaky => "flaky",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Passed, Self::Failed, Self::Flaky, Self::Skipped].into_iter().find(|status| status.as_str() == text)
    }

    /// The worse of two results wins when a test ran on several targets in one run.
    pub fn severity(self) -> u8 {
        match self {
            Self::Skipped => 0,
            Self::Passed => 1,
            Self::Flaky => 2,
            Self::Failed => 3,
        }
    }
}

/// How a whole run ended, as its report says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum E2eRunStatus {
    Passed,
    Failed,
    /// Something other than a test failed: the app did not start, the report could not be written.
    Error,
    Interrupted,
    /// Credentials, the environment or a budget kept every failing test from a verdict.
    Blocked,
}

impl E2eRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Error => "error",
            Self::Interrupted => "interrupted",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Passed, Self::Failed, Self::Error, Self::Interrupted, Self::Blocked].into_iter().find(|status| status.as_str() == text)
    }
}

/// What an issue's tests say together: failing when any of them fails, else flaky when any
/// is flaky, else passing when any passed; `skipped` when none of them has run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum E2eIssueState {
    Passing,
    Failing,
    Flaky,
    Skipped,
}

impl E2eIssueState {
    pub const ALL: [E2eIssueState; 4] = [Self::Passing, Self::Failing, Self::Flaky, Self::Skipped];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passing => "passing",
            Self::Failing => "failing",
            Self::Flaky => "flaky",
            Self::Skipped => "skipped",
        }
    }

    pub fn of(passing: i64, failing: i64, flaky: i64) -> Self {
        if failing > 0 {
            Self::Failing
        } else if flaky > 0 {
            Self::Flaky
        } else if passing > 0 {
            Self::Passing
        } else {
            Self::Skipped
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eRun {
    pub id: Id,
    pub project_key: String,
    pub name: Option<String>,
    /// `json` (the framework's report) or `junit`.
    pub format: String,
    pub status: E2eRunStatus,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub pr_number: Option<i64>,
    pub ci_url: Option<String>,
    /// The runner and its version, e.g. `e2e 0.23.0`.
    pub runner: Option<String>,
    /// What kept the run from reaching its tests, when something did.
    pub error: Option<String>,
    pub started_at: String,
    pub duration_ms: i64,
    pub total: i64,
    pub passed: i64,
    pub failed: i64,
    pub flaky: i64,
    pub skipped: i64,
    /// Steps an agent carried out (`agent.act`, `agent.assert`, …) over the whole run.
    pub agent_steps: i64,
    pub model_calls: i64,
    pub model_tokens: i64,
    pub cost_usd: Option<f64>,
    pub created_by: Option<Id>,
    pub created_at: String,
}

/// An issue a test verifies.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eIssueRef {
    pub key: String,
    pub summary: String,
    pub status: String,
    /// The issue is in a "done" status.
    pub done: bool,
    /// Linked by hand rather than by a key in the test's title or tags.
    pub manual: bool,
}

/// A test's result in one of its recent runs.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eHistoryPoint {
    pub run_id: Id,
    pub status: E2eStatus,
    pub at: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eTest {
    pub id: Id,
    pub file: String,
    /// The describe titles and the test's own, joined with ` > `.
    pub title: String,
    pub tags: Vec<String>,
    pub line: Option<i64>,
    /// The result in the newest run that ran the test; none when it has not run yet.
    pub status: Option<E2eStatus>,
    pub last_run_id: Option<Id>,
    pub last_at: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub duration_ms: Option<i64>,
    pub attempts: Option<i64>,
    pub message: Option<String>,
    pub error_code: Option<String>,
    /// Flaky in its newest run, or passing and failing by turns in the recent ones.
    pub unstable: bool,
    /// Recent runs, oldest first.
    pub history: Vec<E2eHistoryPoint>,
    pub issues: Vec<E2eIssueRef>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eCounts {
    pub total: i64,
    pub passing: i64,
    pub failing: i64,
    pub flaky: i64,
    pub skipped: i64,
    /// Tests that verify no issue.
    pub unlinked: i64,
}

/// Everything the project's Tests page shows.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eProjectTests {
    pub project_key: String,
    pub latest_run: Option<E2eRun>,
    /// Newest first.
    pub runs: Vec<E2eRun>,
    pub tests: Vec<E2eTest>,
    pub counts: E2eCounts,
    pub can_edit: bool,
    pub can_admin: bool,
    /// Where CI posts run reports for this project.
    pub ingest_url: String,
    /// The address of this instance, for the reporter's configuration.
    pub base_url: String,
    /// The issue type "Create bug" uses, when the project has one called Bug.
    pub bug_type_id: Option<Id>,
}

/// One test in one run.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eRunResult {
    pub id: Id,
    pub test_id: Id,
    pub title: String,
    pub file: String,
    /// The line that failed, or where the test is declared.
    pub line: Option<i64>,
    /// The target (browser, device) or agent, when the run had several.
    pub variant: String,
    pub status: E2eStatus,
    pub duration_ms: i64,
    pub attempts: i64,
    pub message: Option<String>,
    pub error_code: Option<String>,
    pub agent_steps: i64,
    pub model_calls: i64,
    pub model_tokens: i64,
    pub cost_usd: Option<f64>,
    pub issues: Vec<E2eIssueRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eRunDetail {
    pub run: E2eRun,
    /// Failures first, then flaky, passed and skipped tests.
    pub results: Vec<E2eRunResult>,
}

/// A test as an issue sees it: its last result, and where that result came from.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eIssueTest {
    pub test_id: Id,
    /// The project the test's runs are posted to.
    pub project_key: String,
    pub title: String,
    pub file: String,
    pub line: Option<i64>,
    pub status: Option<E2eStatus>,
    pub at: Option<String>,
    pub run_id: Option<Id>,
    pub run_name: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub ci_url: Option<String>,
    pub duration_ms: Option<i64>,
    pub message: Option<String>,
    pub manual: bool,
    /// Recent runs, oldest first.
    pub history: Vec<E2eHistoryPoint>,
}

/// The "Tests" panel of an issue.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eIssuePanel {
    pub state: Option<E2eIssueState>,
    pub tests: Vec<E2eIssueTest>,
}

/// The mark on an issue's card or row.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eBadge {
    pub state: E2eIssueState,
    pub total: i64,
    pub passing: i64,
    pub failing: i64,
    pub flaky: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eDashboardIssue {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub done: bool,
    /// The failing tests only.
    pub tests: Vec<E2eIssueTest>,
}

/// "My issues with failing tests" on the dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eDashboard {
    pub issues: Vec<E2eDashboardIssue>,
}

/// The answer to a posted run report.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eIngestResult {
    pub run: E2eRun,
    /// False when a report with this run id had been stored already; nothing changed then.
    pub created: bool,
    /// Keys of the issues the run's tests verify.
    pub linked_issues: Vec<String>,
    /// The run's page in team-hub.
    pub url: String,
}
