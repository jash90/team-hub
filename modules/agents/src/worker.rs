//! The server as a runner, for runs that need no repository: it reads the issue, asks the
//! model, and turns what the model says into run events and proposals. It has no way to
//! change anything else. Model calls block for a long time, so this lives on threads of its
//! own, never on the worker pool.

use std::time::Duration;

use chrono::Utc;
use hub_app::{Actor, ApiError, ApiResult, App};
use hub_core::Id;
use rusqlite::types::Value as SqlValue;
use serde_json::{json, Value};

use crate::builtin;
use crate::model::{self, ModelClient};
use crate::proposals;
use crate::runs::{self, RunPatch, RunRow, AI_LABEL, SERVER_RUNNER};
use crate::settings::{self, ModelInfo, ServerSetup};
use crate::types::{AgentEventKind, AgentRunState};

const SERVER_NAME: &str = "Server";
/// The most times the model is asked within one run, and the most tokens a run may use.
const MAX_STEPS: usize = 6;
const MAX_RUN_TOKENS: i64 = 400_000;
const IDLE: Duration = Duration::from_secs(2);
const SWEEP_EVERY: Duration = Duration::from_secs(30);

fn actor() -> Actor {
    Actor::system(AI_LABEL)
}

fn status(app: &App, id: Id, kind: AgentEventKind, text: &str) -> ApiResult<()> {
    app.write(actor(), |tx| {
        runs::add_event(tx, id, kind, text, None, None)?;

        runs::announce(tx, id)
    })
}

/// Takes the oldest queued run the server may carry out, if it is set up to.
fn take(app: &App) -> ApiResult<Option<(RunRow, ServerSetup)>> {
    let Some(setup) = settings::server_setup(app, Utc::now())? else { return Ok(None) };

    if setup.kinds.is_empty() {
        return Ok(None);
    }

    let kinds: Vec<SqlValue> = setup.kinds.iter().map(|kind| SqlValue::Text(kind.as_str().to_string())).collect();
    let tail =
        format!("WHERE r.state = 'queued' AND r.repo IS NULL AND r.kind IN ({}) ORDER BY r.id LIMIT 1", hub_app::db::placeholders(kinds.len()));
    let taken = app.write(actor(), |tx| {
        let Some(row) = runs::rows(tx, &tail, kinds)?.pop() else { return Ok(None) };

        runs::hold(tx, row.run.id, SERVER_RUNNER, SERVER_NAME, None)?;

        Ok(Some(runs::load(tx, row.run.id)?))
    })?;

    Ok(taken.map(|row| (row, setup)))
}

/// A person asked for the run to stop: nothing more is asked of the model or proposed.
fn stop_if_cancelled(app: &App, id: Id) -> ApiResult<bool> {
    if !app.read(|connection| runs::load(connection, id))?.run.cancel_requested {
        return Ok(false);
    }

    app.write(actor(), |tx| {
        runs::add_event(tx, id, AgentEventKind::Status, "Cancelled", None, None)?;
        runs::finish(tx, id, AgentRunState::Cancelled)?;

        runs::announce(tx, id)
    })?;

    Ok(true)
}

/// What the model is given to read: the issue as the requester sees it, with what the other
/// modules know about it, and the names the project uses.
fn context(app: &App, row: &RunRow) -> ApiResult<(Value, Value)> {
    let requester = app.read(|connection| proposals::person(connection, row.run.requested_by))?;
    let read = |tool: &str, arguments: Value| {
        builtin::call(app, &requester, tool, &arguments).unwrap_or_else(|| Err(ApiError::internal("a built-in tool is missing")))
    };
    let issue = read("get_issue", json!({ "key": row.run.issue_key }))?;
    let mut project = read("get_project", json!({ "key": row.run.project_key }))?;

    if let Some(project) = project.as_object_mut() {
        project.retain(|field, _| {
            matches!(field.as_str(), "key" | "name" | "statuses" | "issueTypes" | "priorities" | "labels" | "people" | "components")
        });
    }

    Ok((issue, project))
}

struct Usage {
    input: i64,
    output: i64,
    cost: f64,
}

fn carry_out(app: &App, row: &RunRow, setup: &ServerSetup) -> ApiResult<()> {
    let id = row.run.id;
    let kind = row.run.kind;
    let chosen: &ModelInfo = row.run.model.as_deref().and_then(settings::model).unwrap_or(setup.default_model);
    let client = ModelClient::new(&setup.api_url, &setup.api_key);

    app.write(actor(), |tx| {
        tx.execute("UPDATE agent_run SET model = ?1 WHERE id = ?2", (chosen.id, id))?;
        runs::record(tx, row, &runs::NewEvent { kind: AgentEventKind::Status, text: "Reading the issue".to_string(), data: Value::Null })?;

        Ok(())
    })?;

    let (issue, project) = context(app, row)?;
    let system = model::system_prompt(kind);
    let tools = model::tools(kind);
    let mut messages = vec![model::first_message(kind, &row.run.instructions, &issue, &project)];
    let mut usage = Usage { input: 0, output: 0, cost: 0.0 };
    let mut summary = String::new();
    let mut reminded = false;

    for step in 1..=MAX_STEPS {
        if stop_if_cancelled(app, id)? {
            return Ok(());
        }

        status(
            app,
            id,
            AgentEventKind::Status,
            &if step == 1 { format!("Asking {}", chosen.name) } else { format!("Asking {} again (step {step})", chosen.name) },
        )?;
        let reply = client.send(chosen, &model::request(chosen, &system, &tools, &messages))?;

        usage.input += reply.input_tokens;
        usage.output += reply.output_tokens;
        usage.cost += settings::cost(chosen, reply.input_tokens, reply.output_tokens);
        app.write(actor(), |tx| {
            tx.execute(
                "UPDATE agent_run SET input_tokens = ?1, output_tokens = ?2, cost_usd = ?3 WHERE id = ?4",
                (usage.input, usage.output, usage.cost, id),
            )?;

            runs::announce(tx, id)
        })?;

        if stop_if_cancelled(app, id)? {
            return Ok(());
        }

        if reply.stop_reason == "refusal" {
            return Err(ApiError::upstream("The model declined to work on this request"));
        }

        let cut_off = reply.stop_reason == "max_tokens";
        let mut results = Vec::new();
        let mut said = Vec::new();

        for block in &reply.content {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str().map(str::trim).filter(|text| !text.is_empty()) {
                        status(app, id, AgentEventKind::Message, text)?;
                        said.push(text.to_string());
                    }
                }
                // An answer that was cut off may hold half a proposal; none of it is used.
                Some("tool_use") if !cut_off => {
                    let name = block["name"].as_str().unwrap_or("");
                    let outcome = match model::proposal_of(name).filter(|proposal| model::proposal_kinds(kind).contains(proposal)) {
                        Some(proposal) => proposals::submit(app, actor(), id, proposal, &block["input"]).map(|proposal| {
                            format!("Recorded as proposal {}. A person will decide on it; do not propose the same thing again.", proposal.id)
                        }),
                        None => Err(ApiError::bad_request(format!("There is no tool called \"{name}\""))),
                    };
                    let (content, failed) = match outcome {
                        Ok(text) => (text, false),
                        Err(error) => {
                            status(app, id, AgentEventKind::Tool, &format!("A proposal was refused: {}", error.message))?;

                            (error.message, true)
                        }
                    };

                    results.push(json!({ "type": "tool_result", "tool_use_id": block["id"], "content": content, "is_error": failed }));
                }
                _ => {}
            }
        }

        if !said.is_empty() {
            summary = said.join("\n\n");
        }

        if cut_off {
            return Err(ApiError::upstream("The model's answer was cut off before it was complete"));
        }

        if usage.input + usage.output > MAX_RUN_TOKENS {
            return Err(ApiError::conflict(format!("The run used more than {MAX_RUN_TOKENS} tokens and was stopped")));
        }

        messages.push(json!({ "role": "assistant", "content": reply.content }));

        if !results.is_empty() {
            messages.push(json!({ "role": "user", "content": results }));
            continue;
        }

        let proposed: i64 =
            app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM agent_proposal WHERE run_id = ?1", [id], |row| row.get(0))?))?;

        // The model is free not to call a tool; a run whose whole point is a proposal asks once more.
        if proposed == 0 && model::must_propose(kind) && !reminded {
            reminded = true;
            let names: Vec<&str> = model::proposal_kinds(kind).iter().map(|kind| model::tool_name(*kind)).collect();
            messages.push(json!({ "role": "user", "content": format!("You have not made a proposal yet. Call {} now with what you suggest.", names.join(" or ")) }));
            continue;
        }

        break;
    }

    if stop_if_cancelled(app, id)? {
        return Ok(());
    }

    app.write(actor(), |tx| {
        let row = runs::load(tx, id)?;
        let state = if row.run.pending_proposals > 0 { AgentRunState::Waiting } else { AgentRunState::Succeeded };
        let summary = if summary.is_empty() { "The model gave no summary.".to_string() } else { summary.clone() };

        runs::update(tx, &row, &RunPatch { state: Some(state), summary: Some(summary), ..Default::default() }).map(|_| ())
    })
}

/// Carries out the next queued run the server may take. False when there was nothing to do.
pub fn run_next(app: &App) -> ApiResult<bool> {
    let Some((row, setup)) = take(app)? else { return Ok(false) };
    let id = row.run.id;

    if let Err(error) = carry_out(app, &row, &setup) {
        tracing::warn!("agent run {id} failed: {}", error.message);
        app.write(actor(), |tx| {
            let row = runs::load(tx, id)?;

            if row.run.state.is_finished() {
                return Ok(());
            }

            runs::add_event(tx, id, AgentEventKind::Error, &error.message, None, None)?;

            runs::update(tx, &row, &RunPatch { state: Some(AgentRunState::Failed), ..Default::default() }).map(|_| ())
        })?;
    }

    Ok(true)
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new().name(name.to_string()).spawn(work) {
        tracing::error!("could not start {name}: {error}");
    }
}

/// Starts the two background threads: one carries out runs, one returns runs to the queue
/// when their runner has gone silent.
pub fn start(app: &App) {
    let (worker, sweeper) = (app.clone(), app.clone());

    spawn("agents-worker", move || {
        if let Err(error) = worker.write(actor(), |tx| runs::requeue_interrupted(tx).map(|_| ())) {
            tracing::warn!("could not requeue interrupted runs: {}", error.message);
        }

        loop {
            let busy = worker.module_enabled(crate::catalog::OWN_MODULE)
                && run_next(&worker).unwrap_or_else(|error| {
                    tracing::warn!("the agent worker: {}", error.message);

                    false
                });

            if !busy {
                std::thread::sleep(IDLE);
            }
        }
    });
    spawn("agents-sweeper", move || loop {
        std::thread::sleep(SWEEP_EVERY);

        if let Err(error) = sweeper.write(actor(), |tx| runs::requeue_stale(tx, Utc::now()).map(|_| ())) {
            tracing::warn!("could not check for silent runners: {}", error.message);
        }
    });
}
