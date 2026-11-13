//! The routes of `docs/agent-runs.md`: for people, for runners, and the settings.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use hub_app::access::project_access;
use hub_app::{Actor, ApiResult, App, AuthUser};
use hub_core::Id;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::proposals::{self, Decision};
use crate::runs::{self, Claim, Heartbeat, NewEvent, NewRun, RunFilter, RunPatch, AI_LABEL};
use crate::settings::{self, SettingsInput};
use crate::types::{AgentProposal, AgentProposalKind, AgentRun, AgentRunDetail, AgentRunner, AgentSettings, AgentTrust};

type Reply<T> = ApiResult<Json<T>>;

pub fn router() -> Router<App> {
    Router::new()
        .route("/issues/{key}/runs", get(issue_runs).post(start_run))
        .route("/runs", get(list_runs))
        .route("/runs/claim", post(claim))
        .route("/runs/{id}", get(get_run).patch(patch_run))
        .route("/runs/{id}/cancel", post(cancel))
        .route("/runs/{id}/answer", post(answer))
        .route("/runs/{id}/events", post(add_event))
        .route("/runs/{id}/proposals", post(add_proposal))
        .route("/proposals/{id}/decide", post(decide))
        .route("/runners", get(runners))
        .route("/runners/heartbeat", post(heartbeat))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/projects/{key}/trust", get(get_trust).put(put_trust))
}

/// What a runner does is recorded under the agent's name, so the requester hears about it.
fn agent() -> Actor {
    Actor::system(AI_LABEL)
}

// ---------- for people ----------

async fn start_run(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(new): Json<NewRun>) -> Reply<AgentRun> {
    app.run(move |app| app.write(user.actor(), |tx| runs::create(tx, &user, &key, &new))).await.map(Json)
}

async fn issue_runs(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Vec<AgentRun>> {
    app.run(move |app| app.read(|connection| runs::list_for_issue(connection, &user, &key))).await.map(Json)
}

async fn list_runs(State(app): State<App>, user: AuthUser, Query(filter): Query<RunFilter>) -> Reply<Vec<AgentRun>> {
    app.run(move |app| app.read(|connection| runs::list(connection, &user, &filter))).await.map(Json)
}

async fn get_run(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<AgentRunDetail> {
    app.run(move |app| {
        app.read(|connection| {
            runs::run_access(connection, &user, id)?;

            runs::detail(connection, id)
        })
    })
    .await
    .map(Json)
}

async fn cancel(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<AgentRun> {
    app.run(move |app| app.write(user.actor(), |tx| runs::cancel(tx, &user, id))).await.map(Json)
}

#[derive(Deserialize)]
struct TextRequest {
    text: String,
}

async fn answer(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<TextRequest>) -> Reply<AgentRunDetail> {
    app.run(move |app| app.write(user.actor(), |tx| runs::answer(tx, &user, id, &request.text))).await.map(Json)
}

async fn decide(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(decision): Json<Decision>) -> Reply<AgentRunDetail> {
    app.run(move |app| proposals::decide(app, &user, id, &decision)).await.map(Json)
}

async fn runners(State(app): State<App>, user: AuthUser) -> Reply<Vec<AgentRunner>> {
    app.run(move |app| app.read(|connection| runs::runners(connection, &user, Utc::now()))).await.map(Json)
}

async fn get_settings(State(app): State<App>, user: AuthUser) -> Reply<AgentSettings> {
    app.run(move |app| app.read(|connection| settings::load(connection, user.is_admin(), Utc::now()))).await.map(Json)
}

async fn put_settings(State(app): State<App>, user: AuthUser, Json(input): Json<SettingsInput>) -> Reply<AgentSettings> {
    user.require_admin()?;
    app.run(move |app| {
        app.db.write(|tx| {
            settings::save(app, tx, &input)?;

            settings::load(tx, true, Utc::now())
        })
    })
    .await
    .map(Json)
}

async fn get_trust(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<AgentTrust> {
    app.run(move |app| app.read(|connection| proposals::trust(connection, project_access(connection, &user, &key)?.id))).await.map(Json)
}

async fn put_trust(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(trust): Json<AgentTrust>) -> Reply<AgentTrust> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let access = project_access(tx, &user, &key)?;
            access.require_admin()?;
            proposals::set_trust(tx, access.id, &trust)?;
            tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))?;

            proposals::trust(tx, access.id)
        })
    })
    .await
    .map(Json)
}

// ---------- for runners ----------

async fn heartbeat(State(app): State<App>, user: AuthUser, Json(beat): Json<Heartbeat>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| runs::heartbeat(tx, &user, &beat))).await?;

    Ok(Json(json!({ "ok": true })))
}

async fn claim(State(app): State<App>, user: AuthUser, Json(wanted): Json<Claim>) -> Reply<Option<AgentRunDetail>> {
    app.run(move |app| {
        app.write(agent(), |tx| match runs::claim(tx, &user, &wanted)? {
            Some(id) => Ok(Some(runs::detail(tx, id)?)),
            None => Ok(None),
        })
    })
    .await
    .map(Json)
}

async fn add_event(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(event): Json<NewEvent>) -> Reply<Value> {
    let event_id = app.run(move |app| app.write(agent(), |tx| runs::record(tx, &runs::held(tx, &user, id)?, &event))).await?;

    Ok(Json(json!({ "id": event_id })))
}

#[derive(Deserialize)]
struct NewProposal {
    kind: AgentProposalKind,
    payload: Value,
}

async fn add_proposal(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(new): Json<NewProposal>) -> Reply<AgentProposal> {
    app.run(move |app| {
        app.write(agent(), |tx| runs::held(tx, &user, id).map(|_| ()))?;

        proposals::submit(app, agent(), id, new.kind, &new.payload)
    })
    .await
    .map(Json)
}

async fn patch_run(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(patch): Json<RunPatch>) -> Reply<AgentRun> {
    app.run(move |app| app.write(agent(), |tx| runs::update(tx, &runs::held(tx, &user, id)?, &patch))).await.map(Json)
}
