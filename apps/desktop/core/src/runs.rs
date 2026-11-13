//! The runner's view of `docs/agent-runs.md`. The server's own types live in its `agents`
//! module and only serialise, so these read the same JSON; unknown fields are ignored, and a
//! kind or state this version does not know reads as `Other` instead of failing the request.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::git::Committed;
use crate::model::{EventKind, ThreadKind, ThreadState};

macro_rules! open_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)+
            Other,
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text,)+ Self::Other => "other" }
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;

                Ok(match text.as_str() { $($text => Self::$variant,)+ _ => Self::Other })
            }
        }
    };
}

open_enum!(RunKind {
    Refine => "refine",
    Breakdown => "breakdown",
    Summarize => "summarize",
    Triage => "triage",
    Implement => "implement",
    Review => "review",
    Custom => "custom",
});
open_enum!(RunState {
    Queued => "queued",
    Claimed => "claimed",
    Running => "running",
    Waiting => "waiting",
    Succeeded => "succeeded",
    Failed => "failed",
    Cancelled => "cancelled",
});
open_enum!(RunEventKind { Message => "message", Tool => "tool", Status => "status", Question => "question", Answer => "answer", Error => "error" });
open_enum!(RunProposalState { Pending => "pending", Approved => "approved", Rejected => "rejected", Failed => "failed" });

/// The kinds a runner with a repository carries out.
pub const RUNNER_KINDS: [RunKind; 3] = [RunKind::Implement, RunKind::Review, RunKind::Custom];

impl RunState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// What a thread's state is called on its run. A queued thread has claimed its run and
    /// not started, which the run already says.
    pub fn of_thread(state: ThreadState) -> Option<Self> {
        match state {
            ThreadState::Queued => None,
            ThreadState::Running => Some(Self::Running),
            ThreadState::Waiting => Some(Self::Waiting),
            ThreadState::Done => Some(Self::Succeeded),
            ThreadState::Failed => Some(Self::Failed),
            ThreadState::Stopped => Some(Self::Cancelled),
        }
    }
}

impl RunKind {
    pub fn of_thread(kind: ThreadKind) -> Self {
        match kind {
            ThreadKind::Implement => Self::Implement,
            ThreadKind::Review => Self::Review,
            ThreadKind::Analyse | ThreadKind::Custom => Self::Custom,
        }
    }

    pub fn thread_kind(self) -> ThreadKind {
        match self {
            Self::Implement => ThreadKind::Implement,
            Self::Review => ThreadKind::Review,
            _ => ThreadKind::Custom,
        }
    }
}

impl RunEventKind {
    pub fn of_thread(kind: EventKind) -> Self {
        match kind {
            EventKind::Message => Self::Message,
            EventKind::Tool => Self::Tool,
            EventKind::Status => Self::Status,
            EventKind::Question => Self::Question,
            EventKind::Answer => Self::Answer,
            EventKind::Error => Self::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    pub id: i64,
    pub issue_key: String,
    #[serde(default)]
    pub issue_summary: String,
    #[serde(default)]
    pub project_key: String,
    pub kind: RunKind,
    #[serde(default)]
    pub instructions: String,
    pub state: RunState,
    #[serde(default)]
    pub requested_by: i64,
    #[serde(default)]
    pub shared: bool,
    #[serde(default)]
    pub runner_id: Option<String>,
    #[serde(default)]
    pub runner_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub pull_request_url: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(default)]
    pub pending_proposals: i64,
    #[serde(default)]
    pub pending_question: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub id: i64,
    #[serde(default)]
    pub run_id: i64,
    pub kind: RunEventKind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub actor_id: Option<i64>,
    #[serde(default)]
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProposal {
    pub id: i64,
    #[serde(default)]
    pub run_id: i64,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
    pub state: RunProposalState,
    #[serde(default)]
    pub decided_by: Option<i64>,
    #[serde(default)]
    pub decided_at: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunDetail {
    pub run: AgentRun,
    #[serde(default)]
    pub events: Vec<RunEvent>,
    #[serde(default)]
    pub proposals: Vec<RunProposal>,
}

/// `POST /issues/{key}/runs`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRun {
    pub kind: RunKind,
    pub instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub shared: bool,
    /// This app's runner: the run is created already claimed by it, so that no other
    /// runner of the same person can take it first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
}

/// `PATCH /runs/{id}`: only what changed is sent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<RunState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl RunPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The fields of `self` that differ from what was last sent.
    pub fn changed_since(&self, sent: &RunPatch) -> RunPatch {
        fn pick<T: PartialEq + Clone>(now: &Option<T>, before: &Option<T>) -> Option<T> {
            if now.is_some() && now != before {
                now.clone()
            } else {
                None
            }
        }

        RunPatch {
            state: pick(&self.state, &sent.state),
            summary: pick(&self.summary, &sent.summary),
            branch: pick(&self.branch, &sent.branch),
            pull_request_url: pick(&self.pull_request_url, &sent.pull_request_url),
            model: pick(&self.model, &sent.model),
            input_tokens: pick(&self.input_tokens, &sent.input_tokens),
            output_tokens: pick(&self.output_tokens, &sent.output_tokens),
            cost_usd: pick(&self.cost_usd, &sent.cost_usd),
        }
    }
}

/// The contract's limit on the diff sent with a run's changes.
pub const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangedCommit {
    pub sha: String,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangedFile {
    pub path: String,
    /// `added`, `modified`, `deleted` or `renamed`.
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

/// `PUT /runs/{id}/changes`: what a headless runner's thread has committed, for a person to
/// look at on the web before anything is pushed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunChanges {
    pub base: String,
    pub head: String,
    pub commits: Vec<ChangedCommit>,
    pub files: Vec<ChangedFile>,
    pub patch: String,
    pub truncated: bool,
}

impl RunChanges {
    /// The commits and the diff between `base` and `head`. A diff past the limit is cut at
    /// the end of a line; the list of files stays whole.
    pub fn of(base: &str, head: &str, committed: Committed) -> Self {
        let truncated = committed.patch.len() > MAX_PATCH_BYTES;
        let mut patch = committed.patch;

        if truncated {
            let mut end = MAX_PATCH_BYTES;

            while !patch.is_char_boundary(end) {
                end -= 1;
            }

            patch.truncate(patch[..end].rfind('\n').map_or(end, |line_end| line_end + 1));
        }

        Self {
            base: base.to_string(),
            head: head.to_string(),
            commits: committed
                .commits
                .into_iter()
                .map(|commit| ChangedCommit { sha: commit.sha, subject: commit.subject, author: Some(commit.author).filter(|author| !author.is_empty()) })
                .collect(),
            files: committed
                .files
                .into_iter()
                .map(|file| ChangedFile { path: file.path, status: file.status, additions: file.additions, deletions: file.deletions })
                .collect(),
            patch,
            truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_reads_from_the_contracts_json_and_survives_fields_and_kinds_it_does_not_know() {
        let json = serde_json::json!({
            "run": {
                "id": 7, "issueKey": "WEB-12", "issueSummary": "Login", "projectKey": "WEB", "kind": "translate", "instructions": "",
                "state": "claimed", "requestedBy": 1, "shared": false, "runnerId": "r1", "runnerName": "Desktop", "model": null, "repo": "acme/web",
                "branch": null, "pullRequestUrl": null, "summary": null, "inputTokens": null, "outputTokens": null, "costUsd": null,
                "cancelRequested": false, "pendingProposals": 0, "pendingQuestion": false, "createdAt": "2026-11-13T10:00:00Z",
                "startedAt": null, "finishedAt": null, "somethingNew": 1
            },
            "events": [{ "id": 1, "runId": 7, "kind": "answer", "text": "yes", "data": null, "actorId": 1, "at": "2026-11-13T10:01:00Z" }],
            "proposals": []
        });
        let detail: AgentRunDetail = serde_json::from_value(json).unwrap();

        assert_eq!(detail.run.kind, RunKind::Other);
        assert_eq!(detail.run.state, RunState::Claimed);
        assert_eq!(detail.events[0].kind, RunEventKind::Answer);
    }

    #[test]
    fn a_patch_sends_only_what_changed() {
        let sent = RunPatch { state: Some(RunState::Running), branch: Some("feature/WEB-1".into()), ..Default::default() };
        let now =
            RunPatch { state: Some(RunState::Succeeded), branch: Some("feature/WEB-1".into()), summary: Some("Done".into()), ..Default::default() };
        let patch = now.changed_since(&sent);

        assert_eq!(serde_json::to_value(&patch).unwrap(), serde_json::json!({ "state": "succeeded", "summary": "Done" }));
        assert!(now.changed_since(&now).is_empty());
    }

    #[test]
    fn a_long_diff_is_cut_at_a_line_and_says_so() {
        let line = "+żółw ".repeat(100) + "\n";
        let patch = line.repeat(MAX_PATCH_BYTES / line.len() + 10);
        let changes = RunChanges::of("a", "b", Committed { commits: Vec::new(), files: Vec::new(), patch: patch.clone() });

        assert!(changes.truncated);
        assert!(changes.patch.len() <= MAX_PATCH_BYTES && changes.patch.ends_with('\n'));
        assert!(patch.starts_with(&changes.patch));

        let short = RunChanges::of("a", "b", Committed { commits: Vec::new(), files: Vec::new(), patch: "diff\n".into() });

        assert_eq!((short.truncated, short.patch.as_str()), (false, "diff\n"));
    }
}
