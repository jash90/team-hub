//! Tools that let a plain MCP client carry out runs: an agent working in a repository
//! (Claude Code, say) can pick up what was asked of it, report as it goes, ask, propose and
//! finish, without speaking the runner routes.

use hub_app::access::visible_projects;
use hub_app::{Actor, ApiError, ApiResult, App, AuthUser, ToolSpec, Tx};
use hub_core::Id;
use rusqlite::types::Value as SqlValue;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::proposals;
use crate::runs::{self, NewEvent, RunPatch, RunRow, AI_LABEL};
use crate::types::{AgentEventKind, AgentProposalKind, AgentRun, AgentRunState};

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

fn run_id() -> Value {
    json!({ "type": "integer", "description": "The run's id, from list_my_runs" })
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list_my_runs",
            description: "List the agent runs waiting for you: work on issues that someone asked an AI agent to do and that you can take, and \
                          runs you have taken and not finished. Each has the issue key, the kind of work and the requester's instructions.",
            input_schema: schema(json!({}), &[]),
            read_only: true,
        },
        ToolSpec {
            name: "get_run",
            description: "Read one agent run: what was asked, everything reported so far, the person's answers to questions, and each \
                          proposal with the person's decision. Read the issue itself with get_issue.",
            input_schema: schema(json!({ "runId": run_id() }), &["runId"]),
            read_only: true,
        },
        ToolSpec {
            name: "report_progress",
            description: "Tell the people following a run what you are doing or have found, in Markdown. The first report on a queued run takes \
                          the run for you.",
            input_schema: schema(json!({ "runId": run_id(), "text": { "type": "string" } }), &["runId", "text"]),
            read_only: false,
        },
        ToolSpec {
            name: "ask_question",
            description: "Ask the person who requested the run a question and make the run wait for them. Their answer appears in get_run.",
            input_schema: schema(json!({ "runId": run_id(), "text": { "type": "string" } }), &["runId", "text"]),
            read_only: false,
        },
        ToolSpec {
            name: "propose",
            description: "Propose a change to the run's issue for a person to approve. Kinds and payloads: comment { body }; transition { status }; \
                          update_fields with any of { summary, description, priority, labels, points, assignee, dueDate }; \
                          create_subtasks { items: [{ summary, description? }] }; create_issue { projectKey, summary, description?, type?, parentKey? }; \
                          log_work { spent, note? }. Names, not ids. The decision appears in get_run.",
            input_schema: schema(
                json!({
                    "runId": run_id(),
                    "kind": { "type": "string", "enum": ["comment", "transition", "update_fields", "create_subtasks", "create_issue", "log_work", "push", "pull_request"] },
                    "payload": { "type": "object" },
                }),
                &["runId", "kind", "payload"],
            ),
            read_only: false,
        },
        ToolSpec {
            name: "finish_run",
            description: "End a run you have taken, with a short summary of the result in Markdown. For work in code, give the branch and the pull \
                          request address.",
            input_schema: schema(
                json!({
                    "runId": run_id(),
                    "summary": { "type": "string" },
                    "state": { "type": "string", "enum": ["succeeded", "failed"], "description": "succeeded unless given" },
                    "branch": { "type": "string" },
                    "pullRequestUrl": { "type": "string" },
                }),
                &["runId", "summary"],
            ),
            read_only: false,
        },
    ]
}

fn parse<T: DeserializeOwned>(arguments: &Value) -> ApiResult<T> {
    T::deserialize(arguments).map_err(|error| ApiError::bad_request(format!("Invalid arguments: {error}")))
}

fn brief(run: &AgentRun, caller: &AuthUser) -> Value {
    json!({
        "runId": run.id,
        "issueKey": run.issue_key,
        "issueSummary": run.issue_summary,
        "kind": run.kind,
        "state": run.state,
        "instructions": run.instructions,
        "repo": run.repo,
        "takenBy": run.runner_name,
        "requestedByYou": run.requested_by == caller.id,
        "cancelRequested": run.cancel_requested,
    })
}

/// The run, held by the caller: a queued run the caller may take is taken on the way.
fn mine(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<RunRow> {
    let (row, _) = runs::run_access(tx, user, id)?;

    if row.run.state == AgentRunState::Queued && (row.run.requested_by == user.id || row.run.shared) {
        runs::hold(tx, id, &format!("mcp-{}", user.id), &format!("MCP · {}", user.name), Some(user.id))?;
    }

    runs::held(tx, user, id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunInput {
    run_id: Id,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextInput {
    run_id: Id,
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProposeInput {
    run_id: Id,
    kind: AgentProposalKind,
    payload: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinishInput {
    run_id: Id,
    summary: String,
    state: Option<AgentRunState>,
    branch: Option<String>,
    pull_request_url: Option<String>,
}

fn list_my_runs(app: &App, user: &AuthUser) -> ApiResult<Value> {
    let (visible, mut params) = visible_projects(user, "r.project_id");
    params.extend([SqlValue::Integer(user.id), SqlValue::Integer(user.id)]);
    let tail = format!(
        "WHERE {visible} AND ((r.state = 'queued' AND (r.requested_by = ? OR r.shared = 1)) OR (r.runner_user_id = ? AND r.state IN ('claimed', 'running', 'waiting')))
         ORDER BY r.id LIMIT 50"
    );

    app.read(|connection| Ok(runs::rows(connection, &tail, params)?.iter().map(|row| brief(&row.run, user)).collect()))
}

fn get_run(app: &App, user: &AuthUser, input: RunInput) -> ApiResult<Value> {
    app.read(|connection| {
        runs::run_access(connection, user, input.run_id)?;
        let detail = runs::detail(connection, input.run_id)?;
        let events: Vec<Value> = detail.events.iter().map(|event| json!({ "kind": event.kind, "text": event.text, "at": event.at })).collect();
        let proposals: Vec<Value> = detail
            .proposals
            .iter()
            .map(|proposal| json!({ "proposalId": proposal.id, "kind": proposal.kind, "state": proposal.state, "note": proposal.note, "payload": proposal.payload, "result": proposal.result }))
            .collect();
        let mut run = brief(&detail.run, user);
        run["summary"] = json!(detail.run.summary);
        run["events"] = json!(events);
        run["proposals"] = json!(proposals);

        Ok(run)
    })
}

fn report(app: &App, user: &AuthUser, input: TextInput, kind: AgentEventKind) -> ApiResult<Value> {
    app.write(Actor::system(AI_LABEL), |tx| {
        let row = mine(tx, user, input.run_id)?;
        runs::record(tx, &row, &NewEvent { kind, text: input.text.clone(), data: Value::Null })?;
        let run = runs::load(tx, input.run_id)?.run;

        Ok(json!({ "runId": run.id, "state": run.state, "cancelRequested": run.cancel_requested }))
    })
}

fn propose(app: &App, user: &AuthUser, input: ProposeInput) -> ApiResult<Value> {
    app.write(Actor::system(AI_LABEL), |tx| mine(tx, user, input.run_id).map(|_| ()))?;
    let proposal = proposals::submit(app, Actor::system(AI_LABEL), input.run_id, input.kind, &input.payload)?;

    Ok(json!({ "proposalId": proposal.id, "state": proposal.state, "note": proposal.note }))
}

fn finish_run(app: &App, user: &AuthUser, input: FinishInput) -> ApiResult<Value> {
    let state = match input.state.unwrap_or(AgentRunState::Succeeded) {
        state @ (AgentRunState::Succeeded | AgentRunState::Failed) => state,
        _ => return Err(ApiError::bad_request("A run ends as \"succeeded\" or \"failed\"")),
    };

    app.write(Actor::system(AI_LABEL), |tx| {
        let row = mine(tx, user, input.run_id)?;
        let patch = RunPatch {
            state: Some(state),
            summary: Some(input.summary.clone()),
            branch: input.branch.clone(),
            pull_request_url: input.pull_request_url.clone(),
            ..Default::default()
        };
        let run = runs::update(tx, &row, &patch)?;

        Ok(json!({ "runId": run.id, "state": run.state, "pendingProposals": run.pending_proposals }))
    })
}

/// Runs one of the tools above; none when `name` is not one of them.
pub fn call(app: &App, user: &AuthUser, name: &str, arguments: &Value) -> Option<ApiResult<Value>> {
    Some(match name {
        "list_my_runs" => list_my_runs(app, user),
        "get_run" => parse(arguments).and_then(|input| get_run(app, user, input)),
        "report_progress" => parse(arguments).and_then(|input| report(app, user, input, AgentEventKind::Message)),
        "ask_question" => parse(arguments).and_then(|input| report(app, user, input, AgentEventKind::Question)),
        "propose" => parse(arguments).and_then(|input| propose(app, user, input)),
        "finish_run" => parse(arguments).and_then(|input| finish_run(app, user, input)),
        _ => return None,
    })
}
