//! The shapes the desktop app adds to those of `hub-core`. `cargo test` writes their
//! TypeScript counterparts into `apps/desktop/ui/src/bindings`.

use hub_core::{Bootstrap, Dashboard, Issue, Project, SearchResult, User};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
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

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

/// A struct that crosses to the interface: camelCase, with a generated TypeScript type.
macro_rules! shared {
    ($(#[$meta:meta])* pub struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
        #[serde(rename_all = "camelCase")]
        #[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
        pub struct $name { $($body)* }
    };
}

pub(crate) use shared;
pub(crate) use text_enum;

text_enum!(
    /// What a thread is asked to do. `analyse` reads and reports; it is a `custom` run on the server.
    ThreadKind { Implement => "implement", Review => "review", Analyse => "analyse", Custom => "custom" }
);
text_enum!(
    /// `queued` waits for a free slot; `waiting` needs a person (a question to answer).
    ThreadState { Queued => "queued", Running => "running", Waiting => "waiting", Done => "done", Failed => "failed", Stopped => "stopped" }
);
text_enum!(ThreadOrigin { App => "app", Delegated => "delegated" });
text_enum!(EventKind { Message => "message", Tool => "tool", Status => "status", Question => "question", Answer => "answer", Error => "error" });
text_enum!(
    /// The first three become proposals of the run on the server; pushing and opening a pull
    /// request are actions of this machine and are decided here.
    ProposalKind {
        Comment => "comment",
        Transition => "transition",
        CreateSubtasks => "create_subtasks",
        Push => "push",
        PullRequest => "pull_request",
    }
);
text_enum!(ProposalState { Pending => "pending", Approved => "approved", Rejected => "rejected", Failed => "failed" });
text_enum!(
    /// Where the copy of the server's data lives between runs of the app.
    CacheMode { Encrypted => "encrypted", MemoryOnly => "memory_only" }
);
text_enum!(SandboxState { Available => "available", Unavailable => "unavailable" });

impl ThreadState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

impl ProposalKind {
    /// Decided through the server's run, so the web interface shows and can decide it too.
    pub fn is_remote(self) -> bool {
        matches!(self, Self::Comment | Self::Transition | Self::CreateSubtasks)
    }
}

shared!(
    /// What the person can change. Never holds the token.
    pub struct Settings {
        pub server_url: Option<String>,
        /// Stable for this installation; the server knows the runner by it.
        pub runner_id: String,
        pub runner_name: String,
        pub runner_enabled: bool,
        /// Threads that may work at the same moment; the rest wait their turn.
        pub max_running: u32,
        pub max_turns: u32,
        pub default_model: String,
        pub node_command: String,
        /// Where there is no OS sandbox a thread gets no shell at all, unless this is set.
        pub allow_unsandboxed_shell: bool,
    }
);

shared!(
    pub struct RunnerStatus {
        pub id: String,
        pub name: String,
        pub enabled: bool,
        /// False when the server has no runs API (an older server, or the module is off).
        pub runs_api: Option<bool>,
        pub last_heartbeat: Option<String>,
        pub error: Option<String>,
    }
);

shared!(
    pub struct ConnectionStatus {
        pub server_url: Option<String>,
        pub has_token: bool,
        pub online: bool,
        pub user: Option<User>,
        pub server_version: Option<String>,
        pub error: Option<String>,
        pub cache: CacheMode,
        pub sandbox: SandboxState,
        pub runner: RunnerStatus,
        pub app_version: String,
    }
);

shared!(
    pub struct WorkspaceView {
        pub bootstrap: Bootstrap,
        pub fetched_at: String,
        /// True when this is the stored copy because the server could not be reached.
        pub offline: bool,
    }
);

shared!(
    pub struct DashboardView {
        pub dashboard: Dashboard,
        /// The configuration of every project that has an issue on the dashboard.
        pub projects: Vec<Project>,
        pub fetched_at: String,
        pub offline: bool,
    }
);

shared!(
    pub struct IssueView {
        pub issue: Issue,
        pub project: Option<Project>,
        pub fetched_at: String,
        pub offline: bool,
    }
);

shared!(
    pub struct SearchView {
        pub result: SearchResult,
        pub projects: Vec<Project>,
    }
);

shared!(
    /// A team-hub project (and, when known, one of its GitHub repositories) tied to a local clone.
    pub struct RepoMapping {
        pub id: i64,
        pub project_key: String,
        /// "owner/name".
        pub github_repo: Option<String>,
        pub path: String,
    }
);

shared!(
    pub struct WorktreeInfo {
        pub path: String,
        pub branch: Option<String>,
        pub head: String,
        /// The thread working in it, when it is one of ours.
        pub thread_id: Option<i64>,
    }
);

shared!(
    pub struct RepoStatus {
        pub mapping: RepoMapping,
        pub exists: bool,
        pub default_branch: Option<String>,
        pub current_branch: Option<String>,
        pub changed_files: u32,
        pub remote_url: Option<String>,
        /// Whether `origin` is the mapped GitHub repository; none when nothing is mapped.
        pub remote_matches: Option<bool>,
        pub worktrees: Vec<WorktreeInfo>,
        pub error: Option<String>,
    }
);

shared!(
    /// A folder looked at before it is mapped.
    pub struct RepoCheck {
        pub path: String,
        pub is_repository: bool,
        pub remote_url: Option<String>,
        pub remote_repo: Option<String>,
        pub matches: bool,
        pub default_branch: Option<String>,
        pub problem: Option<String>,
    }
);

shared!(
    pub struct NewThread {
        pub issue_key: String,
        pub kind: ThreadKind,
        #[serde(default)]
        pub instructions: String,
        #[serde(default)]
        pub model: Option<String>,
        /// Which mapped repository to work in; the project's only one when left out.
        #[serde(default)]
        pub repo_id: Option<i64>,
        /// Continue this existing branch instead of the one suggested for the issue.
        #[serde(default)]
        pub branch: Option<String>,
    }
);

shared!(
    pub struct Thread {
        pub id: i64,
        pub issue_key: String,
        pub issue_summary: String,
        pub project_key: String,
        pub kind: ThreadKind,
        pub instructions: String,
        pub model: String,
        pub repo_id: i64,
        pub repo_path: String,
        pub worktree: Option<String>,
        pub branch: Option<String>,
        /// The ref the branch started from, e.g. `origin/main`, and the commit it was at.
        pub base: Option<String>,
        pub base_commit: Option<String>,
        pub state: ThreadState,
        pub origin: ThreadOrigin,
        pub session_id: Option<String>,
        /// The run on the server that mirrors this thread; none while it could not be created.
        pub run_id: Option<i64>,
        pub summary: Option<String>,
        pub pull_request_url: Option<String>,
        pub input_tokens: i64,
        pub output_tokens: i64,
        pub cost_usd: f64,
        pub turns: i64,
        pub pending_question: bool,
        pub pending_proposals: i64,
        /// The last thing the agent said, for lists.
        pub last_text: Option<String>,
        pub created_at: String,
        pub started_at: Option<String>,
        pub finished_at: Option<String>,
    }
);

shared!(
    pub struct ThreadEvent {
        pub id: i64,
        pub thread_id: i64,
        pub kind: EventKind,
        pub text: String,
        pub data: Value,
        pub at: String,
    }
);

shared!(
    pub struct Proposal {
        pub id: i64,
        pub thread_id: i64,
        pub issue_key: String,
        pub kind: ProposalKind,
        pub payload: Value,
        pub state: ProposalState,
        /// The proposal's id on the server, for the kinds decided through the run.
        pub remote_id: Option<i64>,
        pub note: Option<String>,
        pub result: Value,
        pub created_at: String,
        pub decided_at: Option<String>,
    }
);

shared!(
    pub struct ThreadDetail {
        pub thread: Thread,
        pub events: Vec<ThreadEvent>,
        pub proposals: Vec<Proposal>,
    }
);

shared!(
    pub struct PatchLine {
        /// "context", "add" or "del".
        pub kind: String,
        pub text: String,
        pub old_line: Option<u32>,
        pub new_line: Option<u32>,
    }
);

shared!(
    pub struct PatchHunk {
        pub header: String,
        pub lines: Vec<PatchLine>,
    }
);

shared!(
    pub struct PatchFile {
        pub path: String,
        pub old_path: Option<String>,
        /// "added", "modified", "deleted", "renamed" or "untracked".
        pub status: String,
        pub additions: u32,
        pub deletions: u32,
        pub binary: bool,
        pub hunks: Vec<PatchHunk>,
    }
);

shared!(
    pub struct CommitInfo {
        pub sha: String,
        pub short: String,
        pub subject: String,
        pub author: String,
        pub at: String,
    }
);

shared!(
    /// Everything a thread changed against the point it started from, committed or not.
    pub struct ThreadDiff {
        pub base: String,
        pub files: Vec<PatchFile>,
        pub commits: Vec<CommitInfo>,
        /// Files changed or new in the working tree but not committed.
        pub uncommitted: u32,
    }
);

shared!(
    /// A reason a push waits for an explicit confirmation.
    pub struct PushFinding {
        /// "ci", "secret", "binary", "mode", "link" or "unreadable".
        pub kind: String,
        pub path: Option<String>,
        pub commit: Option<String>,
        pub detail: String,
    }
);

shared!(
    /// Exactly what a push would send: these commits, this diff, up to this commit.
    pub struct PushPreview {
        pub thread_id: i64,
        pub branch: String,
        pub remote_url: String,
        /// The commit that would be pushed; the push is given this id back.
        pub commit: String,
        /// What the server is taken to have: its branch as last seen, or the fork point.
        pub base: String,
        pub commits: Vec<CommitInfo>,
        pub files: Vec<PatchFile>,
        pub findings: Vec<PushFinding>,
        /// Set when the push cannot go at all (nothing to push, history rewritten, a merge commit).
        pub blocked: Option<String>,
        pub uncommitted: u32,
    }
);

shared!(
    pub struct PushResult {
        pub branch: String,
        pub commit: String,
        pub remote_url: String,
    }
);

shared!(
    pub struct PullRequestPreview {
        pub thread_id: i64,
        pub repo: Option<String>,
        pub base: String,
        pub head: String,
        pub title: String,
        pub body: String,
        /// Whether the branch on the server is at the thread's last commit.
        pub pushed: bool,
        /// True when the `gh` command is there to create it; otherwise the browser opens.
        pub can_create: bool,
        pub compare_url: Option<String>,
        pub existing_url: Option<String>,
    }
);

shared!(
    pub struct PullRequestResult {
        /// The created pull request.
        pub url: Option<String>,
        /// The page to finish creating it on, when it could not be created from here.
        pub browser_url: Option<String>,
    }
);

/// What the core tells whoever shows it: the window, the command line, a test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
pub enum AppEvent {
    /// A thread's state, summary or counters changed.
    ThreadChanged { thread_id: i64 },
    ThreadEvent { thread_id: i64, event: ThreadEvent },
    /// Something on the server changed: `scope` is "dashboard", "issue" or "runs".
    DataChanged { scope: String, issue_key: Option<String> },
    Connection { online: bool },
    /// Something needs the person: shown as a system notification.
    Notify { title: String, body: String, thread_id: Option<i64>, issue_key: Option<String> },
}
