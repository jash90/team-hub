//! The team-hub server, as the person whose API token this is. Every call blocks, so callers
//! give it a thread (`tokio::task::spawn_blocking`); nothing here runs on the Rayon pool.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hub_core::{Bootstrap, Comment, Dashboard, Issue, IssuePatch, IssueRow, NewIssue, Notification, Project, SearchResult, Worklog};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::Agent;

use crate::error::{CoreError, CoreResult};
use crate::jobs::{ClaimedJob, JobPatch, TestJob};
use crate::protocol::HubTool;
use crate::runs::{AgentRun, AgentRunDetail, ClaimedRun, NewRun, RunChanges, RunEventKind, RunKind, RunPatch, RunProposal};
use crate::secrets::Secret;

const TIMEOUT: Duration = Duration::from_secs(30);
/// A recording can be 200 MB; it gets longer than a request does.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const AGENTS: &str = "/api/m/agents";
const E2E: &str = "/api/m/e2e";
const MCP_PROTOCOL: &str = "2025-06-18";

/// A change announced on `GET /api/stream`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveEvent {
    #[serde(default)]
    pub id: i64,
    pub kind: String,
    #[serde(default)]
    pub actor_id: Option<i64>,
    #[serde(default)]
    pub payload: Value,
}

/// What the runner says about itself on every heartbeat and claim.
#[derive(Debug, Clone, PartialEq)]
pub struct RunnerIdentity {
    pub id: String,
    pub name: String,
    pub kinds: Vec<RunKind>,
    /// The projects it works for; none: every project the person can edit.
    pub projects: Option<Vec<String>>,
    pub version: String,
    /// False for a runner nobody sits at: its pushes and pull requests are decided on the web.
    pub interactive: bool,
    /// True: it takes only work that was sent to it by name (docs/runners.md).
    pub pinned_only: bool,
    /// True: the team's runner, which works with each person's own keys.
    pub shared: bool,
}

impl RunnerIdentity {
    /// The heartbeat's body. What the desktop app sends stays as it always was; `interactive`
    /// is said only when it is false, and `projects` is left out when there is no list.
    fn announcement(&self) -> Value {
        let mut body = json!({ "runnerId": self.id, "name": self.name, "kinds": self.kinds, "version": self.version });

        if let Some(projects) = &self.projects {
            body["projects"] = json!(projects);
        }

        if !self.interactive {
            body["interactive"] = json!(false);
        }

        if self.pinned_only {
            body["pinnedOnly"] = json!(true);
        }

        if self.shared {
            body["shared"] = json!(true);
        }

        body
    }

    /// What a claim for a test job carries: every claim also says the runner is there.
    fn job_claim(&self) -> Value {
        let mut body = json!({
            "runnerId": self.id, "runnerName": self.name, "interactive": self.interactive, "version": self.version, "pinnedOnly": self.pinned_only,
            "shared": self.shared,
        });

        if let Some(projects) = &self.projects {
            body["projects"] = json!(projects);
        }

        body
    }
}

/// What the server hands an app that asks to be connected (docs/desktop-pairing.md).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingStarted {
    pub id: String,
    /// Shown to the person, who compares it with what the server shows them.
    pub code: String,
    /// Kept by the app and sent back to collect the token; never shown.
    pub secret: Secret,
    pub expires_at: String,
}

/// The token an approved pairing ends in, and the id to remove it by when disconnecting.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingGrant {
    pub token: Secret,
    pub token_id: i64,
}

#[derive(Clone)]
pub struct HubClient {
    base: String,
    token: Secret,
    agent: Agent,
    stream_agent: Agent,
    upload_agent: Agent,
}

impl std::fmt::Debug for HubClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HubClient").field("base", &self.base).finish_non_exhaustive()
    }
}

fn is_loopback(base: &str) -> bool {
    let host = base.split("://").nth(1).unwrap_or_default();

    host.starts_with("127.") || host.starts_with("localhost") || host.starts_with("[::1]")
}

fn agent_for(base: &str, timeout: Option<Duration>) -> Agent {
    let mut config = Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(timeout)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .user_agent(concat!("team-hub-desktop/", env!("CARGO_PKG_VERSION")))
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build());

    // A server on this machine is never behind the proxy the environment names.
    if is_loopback(base) {
        config = config.proxy(None);
    }

    config.build().into()
}

fn offline(error: ureq::Error) -> CoreError {
    CoreError::Offline(error.to_string())
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

impl HubClient {
    pub fn new(base: &str, token: Secret) -> Self {
        Self {
            base: base.to_string(),
            token,
            agent: agent_for(base, Some(TIMEOUT)),
            stream_agent: agent_for(base, None),
            upload_agent: agent_for(base, Some(UPLOAD_TIMEOUT)),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.token.expose())
    }

    fn finish<T: DeserializeOwned>(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> CoreResult<T> {
        let mut response = response.map_err(offline)?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().map_err(offline)?;

        if !(200..300).contains(&status) {
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let message = body["error"]["message"].as_str().map(String::from).unwrap_or_else(|| format!("The server answered {status}"));
            let code = body["error"]["code"].as_str().unwrap_or_default().to_string();

            return Err(CoreError::Api { status, code, message });
        }

        let text = if text.trim().is_empty() { "null" } else { text.as_str() };

        serde_json::from_str(text).map_err(|error| CoreError::Api {
            status,
            code: "unreadable".into(),
            message: format!("The server's answer could not be read: {error}"),
        })
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> CoreResult<T> {
        Self::finish(self.agent.get(format!("{}{path}", self.base)).header("Authorization", self.authorization()).call())
    }

    fn send<T: DeserializeOwned>(&self, method: &str, path: &str, body: &Value) -> CoreResult<T> {
        let url = format!("{}{path}", self.base);
        let request = match method {
            "PATCH" => self.agent.patch(url),
            "PUT" => self.agent.put(url),
            _ => self.agent.post(url),
        };

        Self::finish(request.header("Authorization", self.authorization()).send_json(body))
    }

    pub fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> CoreResult<T> {
        self.send("POST", path, body)
    }

    pub fn patch<T: DeserializeOwned>(&self, path: &str, body: &Value) -> CoreResult<T> {
        self.send("PATCH", path, body)
    }

    // ---------- connecting without a token in anybody's hands: docs/desktop-pairing.md ----------

    /// Asks the server at `base` to connect this app. Nobody is signed in yet.
    pub fn start_pairing(base: &str, name: &str, runner_id: &str) -> CoreResult<PairingStarted> {
        let request = agent_for(base, Some(TIMEOUT)).post(format!("{base}/api/desktop/pairings"));

        Self::finish(request.send_json(json!({ "name": name, "runnerId": runner_id })))
    }

    /// Comes to collect: the token once a person has said yes, nothing while nobody has.
    /// A request that ran out of time or was declined is an error with status 410.
    pub fn claim_pairing(base: &str, id: &str, secret: &Secret) -> CoreResult<Option<PairingGrant>> {
        let request = agent_for(base, Some(TIMEOUT)).post(format!("{base}/api/desktop/pairings/{}/token", encode(id)));
        let answer: Value = Self::finish(request.send_json(json!({ "secret": secret.expose() })))?;

        if answer["token"].is_string() {
            Ok(Some(serde_json::from_value(answer)?))
        } else {
            Ok(None)
        }
    }

    /// Removes one of the person's API tokens on the server: the one this app was given.
    pub fn revoke_token(&self, id: i64) -> CoreResult<()> {
        let request = self.agent.delete(format!("{}/api/me/tokens/{id}", self.base));

        Self::finish::<Value>(request.header("Authorization", self.authorization()).call()).map(|_| ())
    }

    // ---------- the tracker ----------

    pub fn bootstrap(&self) -> CoreResult<Bootstrap> {
        self.get("/api/bootstrap")
    }

    pub fn dashboard(&self) -> CoreResult<Dashboard> {
        self.get("/api/dashboard")
    }

    pub fn project(&self, key: &str) -> CoreResult<Project> {
        self.get(&format!("/api/projects/{}", encode(key)))
    }

    pub fn issue(&self, key: &str) -> CoreResult<Issue> {
        self.get(&format!("/api/issues/{}", encode(key)))
    }

    pub fn search(&self, query: &str, limit: i64) -> CoreResult<SearchResult> {
        self.get(&format!("/api/search?q={}&limit={limit}", encode(query)))
    }

    pub fn notifications(&self) -> CoreResult<Vec<Notification>> {
        self.get("/api/notifications")
    }

    pub fn mark_notifications_read(&self, id: Option<i64>) -> CoreResult<()> {
        self.post::<Value>("/api/notifications/read", &json!({ "id": id })).map(|_| ())
    }

    pub fn add_comment(&self, key: &str, body: &str) -> CoreResult<Comment> {
        self.post(&format!("/api/issues/{}/comments", encode(key)), &json!({ "body": body }))
    }

    pub fn update_issue(&self, key: &str, patch: &IssuePatch) -> CoreResult<IssueRow> {
        self.patch(&format!("/api/issues/{}", encode(key)), &sparse(patch)?)
    }

    pub fn create_issue(&self, new: &NewIssue) -> CoreResult<IssueRow> {
        let mut body = sparse(&new.fields)?;

        body["projectKey"] = json!(new.project_key);
        self.post("/api/issues", &body)
    }

    pub fn log_work(&self, key: &str, spent: &str, note: &str) -> CoreResult<Worklog> {
        self.post(&format!("/api/issues/{}/worklogs", encode(key)), &json!({ "spent": spent, "note": note }))
    }

    /// A module's own data, e.g. `/api/m/github/projects/WEB`. Typed in the interface by the
    /// module's generated bindings; the core reads single fields from it.
    pub fn module_data(&self, path: &str) -> CoreResult<Value> {
        self.get(path)
    }

    // ---------- agent runs: docs/agent-runs.md ----------

    pub fn heartbeat(&self, runner: &RunnerIdentity) -> CoreResult<()> {
        self.post::<Value>(&format!("{AGENTS}/runners/heartbeat"), &runner.announcement()).map(|_| ())
    }

    /// The oldest queued run this runner may take, now claimed by it.
    pub fn claim(&self, runner: &RunnerIdentity) -> CoreResult<Option<ClaimedRun>> {
        let mut body = json!({ "runnerId": runner.id, "kinds": runner.kinds });

        if let Some(projects) = &runner.projects {
            body["projects"] = json!(projects);
        }

        self.post(&format!("{AGENTS}/runs/claim"), &body)
    }

    pub fn create_run(&self, issue_key: &str, new: &NewRun) -> CoreResult<AgentRun> {
        self.post(&format!("{AGENTS}/issues/{}/runs", encode(issue_key)), &serde_json::to_value(new)?)
    }

    pub fn runs(&self, mine: bool, limit: i64) -> CoreResult<Vec<AgentRun>> {
        self.get(&format!("{AGENTS}/runs?mine={mine}&limit={limit}"))
    }

    pub fn run(&self, id: i64) -> CoreResult<AgentRunDetail> {
        self.get(&format!("{AGENTS}/runs/{id}"))
    }

    pub fn patch_run(&self, id: i64, patch: &RunPatch) -> CoreResult<AgentRun> {
        self.patch(&format!("{AGENTS}/runs/{id}"), &serde_json::to_value(patch)?)
    }

    pub fn add_run_event(&self, id: i64, kind: RunEventKind, text: &str, data: &Value) -> CoreResult<i64> {
        let answer: Value = self.post(&format!("{AGENTS}/runs/{id}/events"), &json!({ "kind": kind, "text": text, "data": data }))?;

        Ok(answer["id"].as_i64().unwrap_or_default())
    }

    /// Asks the server to open a pull request for the issue with its own GitHub token.
    pub fn open_pull_request(&self, issue_key: &str, new: &Value) -> CoreResult<Value> {
        self.post(&format!("/api/m/github/issues/{}/pull-request", encode(issue_key)), new)
    }

    pub fn add_run_proposal(&self, id: i64, kind: &str, payload: &Value) -> CoreResult<RunProposal> {
        self.post(&format!("{AGENTS}/runs/{id}/proposals"), &json!({ "kind": kind, "payload": payload }))
    }

    pub fn answer_run(&self, id: i64, text: &str) -> CoreResult<AgentRunDetail> {
        self.post(&format!("{AGENTS}/runs/{id}/answer"), &json!({ "text": text }))
    }

    pub fn decide_proposal(&self, id: i64, approve: bool, note: Option<&str>) -> CoreResult<AgentRunDetail> {
        self.post(&format!("{AGENTS}/proposals/{id}/decide"), &json!({ "approve": approve, "note": note }))
    }

    /// Says how a push or a pull request the person decided here went.
    pub fn settle_proposal(&self, id: i64, approve: bool, note: Option<&str>, result: &Value) -> CoreResult<AgentRunDetail> {
        self.post(&format!("{AGENTS}/proposals/{id}/settle"), &json!({ "approve": approve, "note": note, "result": result }))
    }

    pub fn cancel_run(&self, id: i64) -> CoreResult<AgentRun> {
        self.post(&format!("{AGENTS}/runs/{id}/cancel"), &json!({}))
    }

    // ---------- headless runners: what is decided on the web ----------

    /// What the run's thread has committed, for the person who decides its push.
    pub fn put_changes(&self, run_id: i64, changes: &RunChanges) -> CoreResult<()> {
        self.send::<Value>("PUT", &format!("{AGENTS}/runs/{run_id}/changes"), &serde_json::to_value(changes)?).map(|_| ())
    }

    /// How a push or a pull request approved on the web went: what it produced, or why not.
    pub fn report_proposal(&self, id: i64, outcome: &Result<Value, String>) -> CoreResult<()> {
        let body = match outcome {
            Ok(result) => json!({ "result": result }),
            Err(reason) => json!({ "error": reason }),
        };

        self.post::<Value>(&format!("{AGENTS}/proposals/{id}/result"), &body).map(|_| ())
    }

    // ---------- test jobs: docs/e2e-jobs.md ----------

    /// The oldest queued job this person may run, now claimed, with the project's settings
    /// and the values of its secrets.
    pub fn claim_job(&self, runner: &RunnerIdentity) -> CoreResult<Option<ClaimedJob>> {
        self.post(&format!("{E2E}/jobs/claim"), &runner.job_claim())
    }

    pub fn patch_job(&self, id: i64, patch: &JobPatch) -> CoreResult<TestJob> {
        self.patch(&format!("{E2E}/jobs/{id}"), &serde_json::to_value(patch)?)
    }

    pub fn job_log(&self, id: i64, lines: &[String]) -> CoreResult<()> {
        self.post::<Value>(&format!("{E2E}/jobs/{id}/log"), &json!({ "lines": lines })).map(|_| ())
    }

    /// Sends the suite's report for a job; the id of the run the server stored it as.
    pub fn send_report(&self, project_key: &str, job_id: i64, report: Vec<u8>) -> CoreResult<i64> {
        let url = format!("{}{E2E}/projects/{}/runs?job={job_id}", self.base, encode(project_key));
        let request = self.upload_agent.post(url).header("Authorization", self.authorization()).header("Content-Type", "application/json");
        let answer: Value = Self::finish(request.send(&report[..]))?;

        answer["run"]["id"].as_i64().ok_or_else(|| CoreError::Api {
            status: 200,
            code: "unreadable".into(),
            message: "The server took the report but did not say which run it became".into(),
        })
    }

    /// One file of a run, under the path the report names it by.
    pub fn send_artifact(&self, run_id: i64, path: &str, content_type: &str, file: std::fs::File) -> CoreResult<()> {
        let url = format!("{}{E2E}/runs/{run_id}/artifacts?path={}", self.base, encode(path));
        let request = self.upload_agent.put(url).header("Authorization", self.authorization()).header("Content-Type", content_type);

        Self::finish::<Value>(request.send(file)).map(|_| ())
    }

    pub fn finish_job(&self, id: i64, error: Option<&str>) -> CoreResult<TestJob> {
        let body = match error {
            Some(error) => json!({ "error": error }),
            None => json!({}),
        };

        self.post(&format!("{E2E}/jobs/{id}/finish"), &body)
    }

    // ---------- the server's MCP endpoint ----------

    fn mcp(&self, method: &str, params: Value) -> CoreResult<Value> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let response = self
            .agent
            .post(format!("{}{AGENTS}/mcp", self.base))
            .header("Authorization", self.authorization())
            .header("MCP-Protocol-Version", MCP_PROTOCOL)
            .send_json(&body);
        let answer: Value = Self::finish(response)?;

        match answer.get("error") {
            Some(error) => Err(CoreError::Api {
                status: 200,
                code: error["code"].to_string(),
                message: error["message"].as_str().unwrap_or("The MCP endpoint refused the call").to_string(),
            }),
            None => Ok(answer["result"].clone()),
        }
    }

    /// The tools of the server that change nothing. Only these are ever offered to a thread.
    pub fn read_only_tools(&self) -> CoreResult<Vec<HubTool>> {
        let listing = self.mcp("tools/list", json!({}))?;
        let tools = listing["tools"].as_array().cloned().unwrap_or_default();

        Ok(tools
            .into_iter()
            .filter(|tool| tool["annotations"]["readOnlyHint"].as_bool() == Some(true))
            .filter_map(|tool| {
                Some(HubTool {
                    name: tool["name"].as_str()?.to_string(),
                    description: tool["description"].as_str().unwrap_or_default().to_string(),
                    input_schema: tool["inputSchema"].clone(),
                })
            })
            .collect())
    }

    /// Calls one tool; the text it produced, or the tool's own error as `Err`.
    pub fn call_tool(&self, name: &str, arguments: &Value) -> CoreResult<String> {
        let result = self.mcp("tools/call", json!({ "name": name, "arguments": arguments }))?;
        let text = result["content"]
            .as_array()
            .map(|parts| parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        if result["isError"].as_bool() == Some(true) {
            return Err(CoreError::not_possible(text));
        }

        Ok(text)
    }

    // ---------- the live stream ----------

    /// Reads `GET /api/stream` until the connection ends or `stop` is set, calling `on_event`
    /// for every change. The server's keep-alive comments wake the loop to look at `stop`.
    pub fn stream(&self, stop: &AtomicBool, mut on_event: impl FnMut(LiveEvent)) -> CoreResult<()> {
        let response = self
            .stream_agent
            .get(format!("{}/api/stream", self.base))
            .header("Authorization", self.authorization())
            .header("Accept", "text/event-stream")
            .call()
            .map_err(offline)?;
        let status = response.status().as_u16();

        if status != 200 {
            return Err(CoreError::Api { status, code: "stream".into(), message: format!("The live stream answered {status}") });
        }

        let mut data = String::new();

        for line in BufReader::new(response.into_body().into_reader()).lines() {
            if stop.load(Ordering::Relaxed) {
                break;
            }

            let line = line.map_err(|error| CoreError::Offline(error.to_string()))?;

            if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim_start());
            } else if line.is_empty() && !data.is_empty() {
                if let Ok(event) = serde_json::from_str::<LiveEvent>(&data) {
                    on_event(event);
                }

                data.clear();
            }
        }

        Ok(())
    }
}

/// A patch as JSON with only the fields that are set, so absent ones stay untouched.
fn sparse(patch: &IssuePatch) -> CoreResult<Value> {
    let full = serde_json::to_value(patch)?;
    let fields = full.as_object().cloned().unwrap_or_default();
    let cleared = |name: &str| match name {
        "assigneeId" => patch.assignee_id == Some(None),
        "reporterId" => patch.reporter_id == Some(None),
        "parentKey" => patch.parent_key == Some(None),
        "sprintId" => patch.sprint_id == Some(None),
        "versionId" => patch.version_id == Some(None),
        "points" => patch.points == Some(None),
        "dueDate" => patch.due_date == Some(None),
        "estimateMinutes" => patch.estimate_minutes == Some(None),
        _ => false,
    };

    Ok(Value::Object(fields.into_iter().filter(|(name, value)| !value.is_null() || cleared(name)).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_sends_set_fields_and_explicit_clears_only() {
        let patch = IssuePatch { status_id: Some(4), assignee_id: Some(None), ..Default::default() };

        assert_eq!(sparse(&patch).unwrap(), json!({ "statusId": 4, "assigneeId": null }));
        assert_eq!(sparse(&IssuePatch { assignee_id: Some(Some(2)), ..Default::default() }).unwrap(), json!({ "assigneeId": 2 }));
    }

    #[test]
    fn paths_and_queries_are_escaped() {
        assert_eq!(encode("project = WEB AND text ~ \"żółw\""), "project%20%3D%20WEB%20AND%20text%20~%20%22%C5%BC%C3%B3%C5%82w%22");
        assert_eq!(encode("WEB-12"), "WEB-12");
    }

    #[test]
    fn the_token_is_not_in_the_debug_output() {
        let client = HubClient::new("http://127.0.0.1:1", Secret::new("th_very_secret"));

        assert!(!format!("{client:?}").contains("very_secret"));
    }

    #[test]
    fn an_unreachable_server_is_offline_not_an_api_error() {
        let client = HubClient::new("http://127.0.0.1:1", Secret::new("th_x"));

        assert!(client.dashboard().unwrap_err().is_offline());
    }
}
