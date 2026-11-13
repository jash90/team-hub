//! Lines of JSON exchanged with the helper process that runs one agent thread: the core
//! writes `ToSidecar` to its standard input and reads `FromSidecar` from its standard output,
//! one message per line. The real helper (`apps/desktop/sidecar`) and the fake agent used by
//! tests speak exactly this; the TypeScript side imports the generated types.
//!
//! One process serves one turn of one thread: `start`, then events, then a `result` (or an
//! `error`), then the process ends. A process that ends without either has crashed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 1;

/// A read-only tool of the team-hub server, offered to the agent under its own name. The
/// helper only relays the call; the core checks the name again and makes the call itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
pub struct HubTool {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments object.
    pub input_schema: Value,
}

/// The operating system's sandbox for the thread's shell commands: no network, and writes
/// only inside the working directory plus `writable`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
pub struct SandboxPolicy {
    /// Folders outside the working directory that shell commands may write to.
    pub writable: Vec<String>,
    /// Paths shell commands may never write to, even inside a writable folder.
    pub protected: Vec<String>,
    /// Folders the thread may not read at all.
    pub unreadable: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
pub struct StartThread {
    pub thread: i64,
    pub prompt: String,
    /// The thread's working tree.
    pub cwd: String,
    pub model: String,
    /// The session to continue, when the thread ran before.
    pub resume: Option<String>,
    pub max_turns: u32,
    /// Added to the agent's instructions: how a thread works and what it may not do.
    pub rules: String,
    /// Tools the thread may use without asking; nobody is there to ask.
    pub allowed_tools: Vec<String>,
    /// Refused whatever else allows them.
    pub denied_tools: Vec<String>,
    /// None: this system has no sandbox, and the allowed tools then hold no shell unless the
    /// person chose otherwise in the settings.
    pub sandbox: Option<SandboxPolicy>,
    pub hub_tools: Vec<HubTool>,
    /// Offer the `commit_changes` tool: the thread writes files but has no shell to run git in,
    /// so the app commits for it.
    pub commit_tool: bool,
    /// Added to the environment of the thread's commands.
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
pub enum ToSidecar {
    Start(Box<StartThread>),
    /// Stop working now. The helper ends its process; a helper that does not is killed.
    Stop,
    /// The answer to a `request`.
    Reply {
        request: u64,
        ok: bool,
        result: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
pub enum AgentEvent {
    /// The session id to resume this conversation with.
    Session {
        session_id: String,
    },
    /// Something the agent said.
    Assistant {
        text: String,
    },
    /// A tool the agent called, with its input cut to a readable length.
    Tool {
        name: String,
        detail: String,
    },
    /// The turn is over.
    Result {
        ok: bool,
        summary: String,
        cost_usd: Option<f64>,
        turns: Option<u32>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
pub enum FromSidecar {
    Ready {
        protocol: u32,
    },
    Event {
        event: AgentEvent,
    },
    /// A tool call of the agent that the core answers with a `reply`: `read:<name>` for a
    /// tool of the server, or one of the `propose_*` tools, `ask_question` or `commit_changes`.
    Request {
        request: u64,
        tool: String,
        input: Value,
    },
}

/// Prefix of a request for one of the server's read-only tools.
pub const READ_PREFIX: &str = "read:";
pub const ASK_QUESTION: &str = "ask_question";
pub const PROPOSE_COMMENT: &str = "propose_comment";
pub const PROPOSE_STATUS: &str = "propose_status_change";
pub const PROPOSE_SUBTASKS: &str = "propose_subtasks";
pub const PROPOSE_PUSH: &str = "propose_push";
pub const PROPOSE_PULL_REQUEST: &str = "propose_pull_request";
pub const COMMIT_CHANGES: &str = "commit_changes";

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn messages_are_one_flat_object_with_a_type() {
        let reply = ToSidecar::Reply { request: 3, ok: true, result: json!("recorded") };

        assert_eq!(serde_json::to_value(&reply).unwrap(), json!({ "type": "reply", "request": 3, "ok": true, "result": "recorded" }));
        assert_eq!(serde_json::to_value(&ToSidecar::Stop).unwrap(), json!({ "type": "stop" }));

        let event: FromSidecar =
            serde_json::from_value(json!({ "type": "event", "event": { "kind": "tool", "name": "Bash", "detail": "cargo test" } })).unwrap();

        assert_eq!(event, FromSidecar::Event { event: AgentEvent::Tool { name: "Bash".into(), detail: "cargo test".into() } });
    }

    #[test]
    fn a_start_message_carries_its_fields_beside_the_type() {
        let start = ToSidecar::Start(Box::new(StartThread {
            thread: 1,
            prompt: "Do it".into(),
            cwd: "/w".into(),
            model: "sonnet".into(),
            resume: None,
            max_turns: 10,
            rules: String::new(),
            allowed_tools: vec!["Read".into()],
            denied_tools: vec![],
            sandbox: None,
            hub_tools: vec![],
            commit_tool: false,
            env: BTreeMap::new(),
        }));
        let value = serde_json::to_value(&start).unwrap();

        assert_eq!(value["type"], "start");
        assert_eq!(value["cwd"], "/w");
        assert_eq!(serde_json::from_value::<ToSidecar>(value).unwrap(), start);
    }
}
