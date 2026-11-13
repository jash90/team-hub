//! The shapes of `docs/agent-runs.md`. The contract's `RunKind`, `RunEvent`, … carry an
//! `Agent` prefix here so their TypeScript bindings do not collide with other modules'.

use hub_core::diff::DiffLine;
use hub_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[ts(export)]
        pub enum $name {
            $(#[serde(rename = $text)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            pub fn parse(text: &str) -> Option<Self> {
                match text { $($text => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

text_enum!(
    /// What a run is asked to do.
    AgentRunKind {
        Refine => "refine",
        Breakdown => "breakdown",
        Summarize => "summarize",
        Triage => "triage",
        Implement => "implement",
        Review => "review",
        Custom => "custom",
    }
);
text_enum!(
    /// `waiting` means the run needs a person: a question to answer or a proposal to decide.
    AgentRunState {
        Queued => "queued",
        Claimed => "claimed",
        Running => "running",
        Waiting => "waiting",
        Succeeded => "succeeded",
        Failed => "failed",
        Cancelled => "cancelled",
    }
);
text_enum!(AgentEventKind { Message => "message", Tool => "tool", Status => "status", Question => "question", Answer => "answer", Error => "error" });
text_enum!(AgentProposalKind {
    Comment => "comment",
    Transition => "transition",
    UpdateFields => "update_fields",
    CreateSubtasks => "create_subtasks",
    CreateIssue => "create_issue",
    LogWork => "log_work",
    Push => "push",
    PullRequest => "pull_request",
});
text_enum!(AgentProposalState { Pending => "pending", Approved => "approved", Rejected => "rejected", Failed => "failed" });

impl AgentProposalKind {
    /// Pushing a branch and opening a pull request happen where the code is. Such a proposal
    /// is shown here and decided in the runner's own app, by a person who has seen the
    /// changes; the runner then says how it went.
    pub fn decided_by_runner(self) -> bool {
        matches!(self, Self::Push | Self::PullRequest)
    }
}

impl AgentRunState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

impl AgentRunKind {
    /// Kinds that need no repository, so the server itself can carry them out.
    pub fn server_can_run(self) -> bool {
        matches!(self, Self::Refine | Self::Breakdown | Self::Summarize | Self::Triage | Self::Custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentRun {
    pub id: Id,
    pub issue_key: String,
    pub issue_summary: String,
    pub project_key: String,
    pub kind: AgentRunKind,
    /// Extra guidance from the requester; may be empty.
    pub instructions: String,
    pub state: AgentRunState,
    pub requested_by: Id,
    /// False: only the requester's own runners may claim it.
    pub shared: bool,
    /// Who carries it out: "server", or a runner's id.
    pub runner_id: Option<String>,
    pub runner_name: Option<String>,
    pub model: Option<String>,
    /// "owner/name", for work in code.
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub pull_request_url: Option<String>,
    /// The result in a few sentences (Markdown).
    pub summary: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub cancel_requested: bool,
    pub pending_proposals: i64,
    pub pending_question: bool,
    /// False when the run's runner has nobody at it: its pushes and pull requests are then
    /// decided here rather than in the runner's app. True for the server and for no runner.
    pub runner_interactive: bool,
    /// Whether the runner has uploaded what the thread committed (`GET /runs/{id}/changes`).
    pub has_changes: bool,
    /// The runner this run was sent to by name and waits for; null: any runner that fits.
    pub on_runner: Option<String>,
    pub on_runner_name: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

/// One commit of a run's thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentCommit {
    pub sha: String,
    pub subject: String,
    #[serde(default)]
    pub author: Option<String>,
}

/// One file a run's thread changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentChangedFile {
    pub path: String,
    /// "added", "modified", "deleted" or "renamed".
    pub status: String,
    #[serde(default)]
    pub additions: i64,
    #[serde(default)]
    pub deletions: i64,
}

/// What a run's thread has committed, uploaded by its runner so that a person can look
/// before anything is pushed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentRunChanges {
    /// The commit the thread started from.
    pub base: String,
    /// The thread's last commit.
    pub head: String,
    #[serde(default)]
    pub commits: Vec<AgentCommit>,
    #[serde(default)]
    pub files: Vec<AgentChangedFile>,
    /// A unified diff from `base` to `head`.
    #[serde(default)]
    pub patch: String,
    /// True when the runner cut the diff short.
    #[serde(default)]
    pub truncated: bool,
    /// When it was uploaded; set by the server.
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentRunEvent {
    pub id: Id,
    pub run_id: Id,
    pub kind: AgentEventKind,
    /// Markdown for message, question and answer; one line for tool and status.
    pub text: String,
    /// Optional details (a command, a file list).
    pub data: Value,
    /// The person, for answers.
    pub actor_id: Option<Id>,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentProposal {
    pub id: Id,
    pub run_id: Id,
    pub kind: AgentProposalKind,
    pub payload: Value,
    pub state: AgentProposalState,
    pub decided_by: Option<Id>,
    pub decided_at: Option<String>,
    /// The person's reason, or why applying it failed.
    pub note: Option<String>,
    /// What applying it produced, e.g. `{ keys: ["WEB-31", "WEB-32"] }`.
    pub result: Value,
    /// What the proposal changes, for showing it before it is decided (not part of the
    /// contract's type): see `AgentProposalPreview`. Null when the issue is gone.
    pub preview: Option<AgentProposalPreview>,
}

/// One field a proposal would change, with its value now and after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentFieldChange {
    pub field: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// What a proposal would do, against the issue as it is now (or as it was when decided).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentProposalPreview {
    /// Fields that change, the description excepted.
    pub changes: Vec<AgentFieldChange>,
    /// The description line by line, when it changes.
    pub description_diff: Option<Vec<AgentDiffLine>>,
    /// For a transition: whether the project's workflow allows the move.
    pub allowed: Option<bool>,
    /// The type new issues would get.
    pub issue_type: Option<String>,
}

/// A line of a description diff (`hub_core::diff::DiffLine` with the fields readable back).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentDiffLine {
    /// "same", "added" or "removed".
    pub kind: String,
    pub text: String,
}

impl From<DiffLine> for AgentDiffLine {
    fn from(line: DiffLine) -> Self {
        Self { kind: line.kind.as_str().to_string(), text: line.text }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentRunDetail {
    pub run: AgentRun,
    pub events: Vec<AgentRunEvent>,
    pub proposals: Vec<AgentProposal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentRunner {
    /// Stable per installation.
    pub id: String,
    pub name: String,
    pub user_id: Id,
    pub kinds: Vec<AgentRunKind>,
    /// Project keys it has repositories for; null means any.
    pub projects: Option<Vec<String>>,
    pub version: String,
    pub last_seen_at: String,
    /// Seen in the last two minutes.
    pub online: bool,
    /// False for a runner with nobody at it (a container): see `AgentRun::runner_interactive`.
    pub interactive: bool,
    /// Takes only runs sent to it by name.
    pub pinned_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentModel {
    pub id: String,
    pub name: String,
    /// US dollars per million tokens.
    pub input_price: f64,
    pub output_price: f64,
}

/// How the server carries out runs. Everyone may read it (the key itself is never returned,
/// and its hint only to administrators); administrators change it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentSettings {
    pub key_configured: bool,
    pub key_hint: Option<String>,
    pub api_url: String,
    pub default_model: String,
    pub models: Vec<AgentModel>,
    /// The kinds the server carries out itself when a key is configured.
    pub server_kinds: Vec<AgentRunKind>,
    pub monthly_cap_usd: Option<f64>,
    pub spent_this_month_usd: f64,
    /// "ready", "no_key" or "cap_reached": why the server does or does not take runs.
    pub server_status: String,
}

/// The proposal kinds a project applies without waiting for a person.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentTrust {
    pub kinds: Vec<AgentProposalKind>,
}
