//! An MCP (Model Context Protocol) server, so coding agents can read and change issues from
//! inside a repository. It speaks the Streamable HTTP transport in its stateless form: every
//! POST carries one JSON-RPC message and is answered with one JSON body. No sessions, no
//! streams. The caller is whoever the API token belongs to, and every tool acts as them.
//!
//! On top of that: agent runs (`docs/agent-runs.md`). People hand work on an issue to an
//! agent, runners carry it out (the server itself for work that needs no repository), and
//! whatever the agent wants to change comes back as proposals for a person to decide.

mod builtin;
pub mod catalog;
pub mod model;
mod names;
pub mod proposals;
mod routes;
pub mod rpc;
mod runner_tools;
pub mod runs;
mod schema;
pub mod settings;
pub mod types;
pub mod worker;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hub_app::access::visible_projects;
use hub_app::db::{placeholders, query_all};
use hub_app::{ApiResult, App, AuthUser, Module, ModuleMeta};
use hub_core::{Id, IssueRow};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};
use ts_rs::TS;

use crate::catalog::AgentToolInfo;
use crate::rpc::{Incoming, RpcError};

/// The revisions of the protocol this server speaks, newest first.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const PROTOCOL_HEADER: &str = "mcp-protocol-version";
const SERVER_NAME: &str = "team-hub";
const INSTRUCTIONS: &str = "team-hub is this team's issue tracker. Issues have keys such as WEB-12; when work relates to an issue, read it with \
     get_issue first. Tools take names, not ids: call get_project to learn a project's statuses, issue types, sprints and people before creating \
     or changing issues. search_issues takes the TQL query language, e.g. `project = WEB AND assignee = currentUser() AND category != done`. \
     Everything you do is recorded under the name of the person whose token you carry.";

pub struct AgentsModule;

impl Module for AgentsModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: catalog::OWN_MODULE,
            name: "AI agents (MCP)",
            description: "Lets coding agents such as Claude Code read and update issues through the Model Context Protocol.",
            version: env!("CARGO_PKG_VERSION"),
            depends_on: &[],
            default_enabled: true,
        }
    }

    fn migrations(&self) -> &'static [&'static str] {
        schema::MIGRATIONS
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(Router::new().route("/mcp", post(mcp)).route("/tools", get(tools)).route("/info", get(info)).merge(routes::router()))
    }

    /// Marks issues an agent is working on, or where an agent waits for a person.
    fn decorate_issues(&self, connection: &Connection, rows: &mut [IssueRow]) -> ApiResult<()> {
        let ids: Vec<SqlValue> = rows.iter().map(|row| SqlValue::Integer(row.id)).collect();
        let busy = query_all(
            connection,
            &format!(
                "SELECT r.issue_id, sum(r.state IN ('queued', 'claimed', 'running')),
                        sum(r.state = 'waiting' OR EXISTS (SELECT 1 FROM agent_proposal a WHERE a.run_id = r.id AND a.state = 'pending'))
                 FROM agent_run r WHERE r.issue_id IN ({}) AND r.state != 'cancelled' GROUP BY r.issue_id",
                placeholders(ids.len())
            ),
            rusqlite::params_from_iter(ids),
            |row| Ok((row.get::<_, Id>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        )?;

        for (issue_id, working, waiting) in busy.into_iter().filter(|(_, working, waiting)| working + waiting > 0) {
            if let Some(row) = rows.iter_mut().find(|row| row.id == issue_id) {
                row.ext.insert(catalog::OWN_MODULE.to_string(), json!({ "working": working, "waiting": waiting }));
            }
        }

        Ok(())
    }

    /// The issue's runs, newest first.
    fn issue_panel(&self, _app: &App, connection: &Connection, issue: &IssueRow, _viewer: &AuthUser) -> ApiResult<Option<Value>> {
        let runs: Vec<_> = runs::rows(connection, "WHERE r.issue_id = ? ORDER BY r.id DESC LIMIT 30", vec![SqlValue::Integer(issue.id)])?
            .into_iter()
            .map(|row| row.run)
            .collect();

        Ok(Some(json!({ "runs": runs })))
    }

    /// Runs the viewer asked for that wait for a person: a question, or proposals to decide.
    fn dashboard(&self, connection: &Connection, viewer: &AuthUser) -> ApiResult<Option<Value>> {
        let (visible, mut params) = visible_projects(viewer, "r.project_id");
        params.push(SqlValue::Integer(viewer.id));
        let tail = format!(
            "WHERE {visible} AND r.requested_by = ? AND r.state != 'cancelled'
               AND (r.question_pending = 1 OR EXISTS (SELECT 1 FROM agent_proposal a WHERE a.run_id = r.id AND a.state = 'pending'))
             ORDER BY r.id DESC LIMIT 20"
        );
        let waiting: Vec<_> = runs::rows(connection, &tail, params)?.into_iter().map(|row| row.run).collect();

        Ok(if waiting.is_empty() { None } else { Some(json!({ "waiting": waiting })) })
    }

    fn start(&self, app: &App) {
        worker::start(app);
    }
}

/// How to connect an agent to this instance.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentInfo {
    /// The address of the MCP endpoint.
    pub endpoint: String,
    /// The Claude Code command that registers it; `<token>` stands for an API token.
    pub example_command: String,
}

async fn tools(State(app): State<App>, _user: AuthUser) -> Json<Vec<AgentToolInfo>> {
    Json(catalog::infos(&app))
}

async fn info(State(app): State<App>, _user: AuthUser) -> Json<AgentInfo> {
    let endpoint = format!("{}/api/m/{}/mcp", app.config.base_url.trim_end_matches('/'), catalog::OWN_MODULE);
    let example_command = format!("claude mcp add --transport http {SERVER_NAME} {endpoint} --header \"Authorization: Bearer <token>\"");

    Json(AgentInfo { endpoint, example_command })
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested.filter(|version| PROTOCOL_VERSIONS.contains(version)).unwrap_or(PROTOCOL_VERSIONS[0]);

    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

/// What a tool produced, as text for the model: a string as it is, anything else as JSON.
fn tool_result(output: &Value, is_error: bool) -> Value {
    let text = match output {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };

    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// A tool that fails has still been called: the model reads why in the result and can try
/// again. Only a name no tool answers to is an error of the protocol.
async fn call_tool(app: &App, user: AuthUser, params: &Value) -> Result<Value, RpcError> {
    let invalid = |message: &str| RpcError::new(rpc::INVALID_PARAMS, message);
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| invalid("tools/call needs the \"name\" of a tool"))?.to_string();
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(arguments) if arguments.is_object() => arguments.clone(),
        Some(_) => return Err(invalid("\"arguments\" must be an object")),
    };
    let tool = name.clone();
    let outcome = app
        .run(move |app| Ok(catalog::call(app, &user, &tool, &arguments)))
        .await
        .map_err(|error| RpcError::new(rpc::INTERNAL_ERROR, error.message))?;

    match outcome {
        Some(Ok(output)) => Ok(tool_result(&output, false)),
        Some(Err(error)) => Ok(tool_result(&Value::String(error.message), true)),
        None => Err(RpcError::new(rpc::INVALID_PARAMS, format!("Unknown tool: {name}"))),
    }
}

async fn answer(app: &App, user: AuthUser, method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": catalog::listing(app) })),
        "tools/call" => call_tool(app, user, params).await,
        other => Err(RpcError::new(rpc::METHOD_NOT_FOUND, format!("Method not found: {other}"))),
    }
}

fn rejected(id: Value, error: &RpcError) -> Response {
    (StatusCode::BAD_REQUEST, Json(rpc::failure(id, error))).into_response()
}

async fn mcp(State(app): State<App>, user: AuthUser, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return rejected(Value::Null, &RpcError::new(rpc::PARSE_ERROR, "Parse error: the request body is not JSON"));
    };

    if let Some(version) = headers.get(PROTOCOL_HEADER) {
        if !version.to_str().is_ok_and(|version| PROTOCOL_VERSIONS.contains(&version)) {
            let message = format!("Unsupported MCP-Protocol-Version; this server speaks {}", PROTOCOL_VERSIONS.join(", "));

            return rejected(Value::Null, &RpcError::new(rpc::INVALID_REQUEST, message));
        }
    }

    match rpc::classify(message) {
        Incoming::Notification => StatusCode::ACCEPTED.into_response(),
        Incoming::Invalid { id, error } => rejected(id, &error),
        Incoming::Call { id, method, params } => match answer(&app, user, &method, &params).await {
            Ok(result) => Json(rpc::success(id, result)).into_response(),
            Err(error) => Json(rpc::failure(id, &error)).into_response(),
        },
    }
}
