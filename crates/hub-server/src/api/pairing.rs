//! Connecting a desktop app to a person's account (`docs/desktop-pairing.md`): the app asks
//! and later collects, signed in as nobody and holding a secret; the person in between says
//! yes, signed in in a browser.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hub_app::services::pairing::{self, Claim};
use hub_app::{ApiResult, App, AuthUser};
use hub_core::DesktopPairing;
use serde::Deserialize;
use serde_json::{json, Value};

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/desktop/pairings", post(start))
        .route("/desktop/pairings/{code}", get(show))
        .route("/desktop/pairings/{code}/approve", post(approve))
        .route("/desktop/pairings/{code}/decline", post(decline))
        .route("/desktop/pairings/{id}/token", post(claim))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRequest {
    name: String,
    #[serde(default)]
    runner_id: String,
}

/// No caller is asked for: the app has no account yet. What it gets lets it do nothing
/// until a person has said yes.
async fn start(State(app): State<App>, Json(request): Json<StartRequest>) -> Reply<Value> {
    let started = app.run(move |app| app.db.write(|tx| pairing::start(tx, &request.name, &request.runner_id))).await?;

    Ok(Json(json!({ "id": started.id, "code": started.code, "secret": started.secret, "expiresAt": started.expires_at })))
}

async fn show(State(app): State<App>, user: AuthUser, Path(code): Path<String>) -> Reply<DesktopPairing> {
    app.run(move |app| app.read(|connection| pairing::show(connection, &user, &code))).await.map(Json)
}

async fn approve(State(app): State<App>, user: AuthUser, Path(code): Path<String>) -> Reply<DesktopPairing> {
    app.run(move |app| app.db.write(|tx| pairing::approve(tx, &user, &code))).await.map(Json)
}

async fn decline(State(app): State<App>, user: AuthUser, Path(code): Path<String>) -> Reply<Value> {
    app.run(move |app| app.db.write(|tx| pairing::decline(tx, &user, &code))).await?;

    Ok(OK)
}

#[derive(Deserialize)]
struct ClaimRequest {
    secret: String,
}

/// 202 while nobody has said yes; the token, once; 410 when the request is no more.
async fn claim(State(app): State<App>, Path(id): Path<String>, Json(request): Json<ClaimRequest>) -> ApiResult<Response> {
    let claim = app.run(move |app| pairing::claim(app, &id, &request.secret)).await?;

    Ok(match claim {
        Claim::Pending => (StatusCode::ACCEPTED, Json(json!({ "status": "pending" }))).into_response(),
        Claim::Granted { token, token_id } => Json(json!({ "status": "approved", "token": token, "tokenId": token_id })).into_response(),
    })
}
