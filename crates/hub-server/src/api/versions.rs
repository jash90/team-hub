//! Versions: releases and their notes.

use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use hub_app::access::project_access;
use hub_app::services::versions::{self, VersionInput};
use hub_app::{App, AuthUser};
use hub_core::{Id, Version};
use serde::Deserialize;
use serde_json::{json, Value};

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/projects/{key}/versions", post(create_version))
        .route("/versions/{id}", patch(update_version).delete(delete_version))
        .route("/versions/{id}/release", post(release_version))
        .route("/versions/{id}/notes", get(version_notes))
}

async fn create_version(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<VersionInput>) -> Reply<Version> {
    app.run(move |app| app.write(user.actor(), |tx| versions::create_version(tx, &project_access(tx, &user, &key)?, &input))).await.map(Json)
}

async fn update_version(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(input): Json<VersionInput>) -> Reply<Version> {
    app.run(move |app| app.write(user.actor(), |tx| versions::update_version(tx, &user, id, &input))).await.map(Json)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseRequest {
    released: bool,
    move_to: Option<Id>,
}

async fn release_version(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<ReleaseRequest>) -> Reply<Version> {
    app.run(move |app| app.write(user.actor(), |tx| versions::set_released(tx, &user, id, request.released, request.move_to))).await.map(Json)
}

async fn delete_version(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| versions::delete_version(tx, &user, id))).await?;

    Ok(OK)
}

async fn version_notes(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    let notes = app.run(move |app| app.read(|connection| versions::version_notes(connection, &user, id))).await?;

    Ok(Json(json!({ "markdown": notes })))
}
