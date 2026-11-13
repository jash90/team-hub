//! The shapes this module sends to the web interface and to agents.

use std::collections::BTreeMap;

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
    /// Recordings and screenshots the report names for this result.
    pub videos: i64,
    pub screenshots: i64,
    /// The report told the steps of this result, so it has a session to open.
    pub has_session: bool,
    pub issues: Vec<E2eIssueRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eRunDetail {
    pub run: E2eRun,
    /// The job that produced the run, when one did.
    pub job_id: Option<Id>,
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

// ---------- test jobs ----------

/// Which attempts are recorded as video. The names are the framework's `--video` modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum E2eVideoMode {
    On,
    RetainOnFailure,
    OnFirstRetry,
    Off,
}

impl E2eVideoMode {
    pub const ALL: [E2eVideoMode; 4] = [Self::On, Self::RetainOnFailure, Self::OnFirstRetry, Self::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::RetainOnFailure => "retain-on-failure",
            Self::OnFirstRetry => "on-first-retry",
            Self::Off => "off",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == text)
    }
}

/// Which tests keep a trace page (`trace.md`). The names are the framework's `--trace` modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum E2eTraceMode {
    On,
    RetainOnFailure,
    Off,
}

impl E2eTraceMode {
    pub const ALL: [E2eTraceMode; 3] = [Self::On, Self::RetainOnFailure, Self::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::RetainOnFailure => "retain-on-failure",
            Self::Off => "off",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == text)
    }
}

/// How a project's suite is run by a runner.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eSettings {
    /// `owner/name`; none: no jobs for this project.
    pub repo: Option<String>,
    /// The branch, tag or commit a job runs unless it names another.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// Where the suite's `package.json` is, from the repository root; empty for the root.
    pub directory: String,
    pub install_command: String,
    /// Extra arguments for `e2e run`.
    pub run_args: String,
    pub video: E2eVideoMode,
    pub trace: E2eTraceMode,
    /// Plain variables for the suite.
    pub env: BTreeMap<String, String>,
    /// Names of the stored secrets; their values leave the server only for a runner that claims a job.
    pub secret_names: Vec<String>,
    pub timeout_minutes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum E2eJobState {
    Queued,
    Claimed,
    Running,
    /// The suite ran and its report arrived; its tests may have failed.
    Succeeded,
    /// The suite could not be run: clone, install, no report, timeout, a runner that went quiet.
    Failed,
    Cancelled,
}

impl E2eJobState {
    pub const ALL: [E2eJobState; 6] = [Self::Queued, Self::Claimed, Self::Running, Self::Succeeded, Self::Failed, Self::Cancelled];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }

    /// Nothing more happens to the job.
    pub fn ended(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eJob {
    pub id: Id,
    pub project_key: String,
    pub state: E2eJobState,
    pub repo: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// The commit the runner checked out, once it has said.
    pub commit: Option<String>,
    /// Test files or a `--grep` pattern, as typed.
    pub filter: Option<String>,
    pub video: String,
    pub trace: String,
    pub requested_by: Id,
    /// The runner the job was sent to by name (`docs/runners.md`), and the name it had then;
    /// none for a job any runner that fits may take.
    pub on_runner: Option<String>,
    pub on_runner_name: Option<String>,
    pub runner_name: Option<String>,
    /// The stored run, once the report has arrived.
    pub run_id: Option<Id>,
    pub error: Option<String>,
    pub cancel_requested: bool,
    /// What the runner is doing: "cloning", "installing", "running", "uploading".
    pub step: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eLogLine {
    pub seq: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eJobDetail {
    pub job: E2eJob,
    /// The lines after `after`; every line still kept when the request names none.
    pub log: Vec<E2eLogLine>,
    /// Earlier lines of the job were dropped: only the newest are kept.
    pub log_truncated: bool,
}

// ---------- the session of one test result ----------

/// An error as the report tells it: the code, the message, and what an assertion expected and saw.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct E2eSessionError {
    pub code: Option<String>,
    pub message: String,
    pub locator: Option<String>,
    pub expected: Option<String>,
    pub observed: Option<String>,
    pub file: Option<String>,
    pub line: Option<i64>,
}

/// Something a step did or saw: an engine action, a navigation of the app, a poll of an assertion.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct E2eSessionEvent {
    /// `engine` (an action), `navigation` (where the app went) or `poll` (what an assertion read).
    pub kind: String,
    pub detail: String,
    pub failed: bool,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct E2eSessionStep {
    pub index: i64,
    /// `locator.tap`, `expect.toHaveText`, `agent.act`, …
    pub api: String,
    pub label: String,
    /// `passed`, `failed` or `cancelled`.
    pub status: String,
    pub started_at: Option<String>,
    pub duration_ms: i64,
    pub file: Option<String>,
    pub line: Option<i64>,
    /// The hook the step ran in (`beforeEach`, `afterEach`); none for a step of the test body.
    pub phase: Option<String>,
    pub error: Option<E2eSessionError>,
    /// What an agent step said about what it did or judged.
    pub explanation: Option<String>,
    pub events: Vec<E2eSessionEvent>,
    /// Events the report had beyond the ones kept.
    pub more_events: i64,
    /// Where the app was after the step.
    pub location: Option<String>,
    /// How the screen differs from the step before's, first lines.
    pub changes: Vec<String>,
    pub more_changes: i64,
    /// Ids (the report's) of the artifacts the step produced, such as a screenshot.
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct E2eSessionLogLine {
    /// `error`, `warn`, `info`, `log`, `debug`.
    pub level: String,
    pub text: String,
    pub at: Option<String>,
    /// The index of the step that was running; none before the first step.
    pub step: Option<i64>,
}

/// A file the report names for an attempt.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct E2eSessionArtifact {
    /// The report's id of the artifact; steps and the failure refer to it.
    #[serde(rename = "ref")]
    pub report_id: String,
    /// The report's kind: `video`, `screenshot`, `log` or `download`.
    pub kind: String,
    /// As the report names it; none for a recording a hosted service keeps (see `url`).
    pub path: Option<String>,
    pub url: Option<String>,
    /// When a recording began: a step is at `step.startedAt − startedAt` in it.
    pub started_at: Option<String>,
    /// The uploaded file (`GET /artifacts/{id}`), once the runner has sent it.
    #[serde(skip_deserializing)]
    pub artifact_id: Option<Id>,
    #[serde(skip_deserializing)]
    pub size: Option<i64>,
}

/// What the runner saw when the failure landed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct E2eSessionFailure {
    pub url: Option<String>,
    /// The report's ids of the screenshot and of the screen's text, both among the attempt's artifacts.
    pub screenshot: Option<String>,
    pub screen: Option<String>,
    pub nodes: Option<i64>,
    /// Screen lines of the nodes closest to what a failed locator asked for.
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct E2eSessionAttempt {
    pub index: i64,
    /// `passed`, `failed`, `timed-out`, `interrupted`, `skipped`.
    pub status: String,
    pub started_at: Option<String>,
    pub duration_ms: i64,
    pub error: Option<E2eSessionError>,
    /// What else failed, such as a teardown after the test's own failure.
    pub also_failed: Vec<E2eSessionError>,
    pub steps: Vec<E2eSessionStep>,
    /// Steps the report had beyond the ones kept.
    pub more_steps: i64,
    pub app_log: Vec<E2eSessionLogLine>,
    pub more_app_log: i64,
    pub failure: Option<E2eSessionFailure>,
    pub artifacts: Vec<E2eSessionArtifact>,
    /// What the engine said it ran on, such as `browser`.
    pub environment: BTreeMap<String, String>,
}

/// One test result with everything its report said happened, for the session view.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct E2eSession {
    pub run: E2eRun,
    pub result: E2eRunResult,
    pub tags: Vec<String>,
    /// Empty for a result whose report carried no steps: JUnit, or a test that never started.
    pub attempts: Vec<E2eSessionAttempt>,
    /// The uploaded `trace.md` of the test, when the runner has sent one.
    pub trace_artifact_id: Option<Id>,
    /// The job that produced the run, when one did, and whether it is still at work (so a
    /// missing file may yet arrive).
    pub job_id: Option<Id>,
    pub job_active: bool,
    pub can_edit: bool,
    /// The issue type "Create bug" uses, when the project has one called Bug.
    pub bug_type_id: Option<Id>,
}
