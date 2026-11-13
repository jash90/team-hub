//! Sprints: planning, starting, completing.

use axum::extract::{Path, State};
use axum::routing::{patch, post};
use axum::{Json, Router};
use hub_app::access::project_access;
use hub_app::services::sprints::{self, SprintInput};
use hub_app::{App, AuthUser};
use hub_core::{Id, Sprint};
use serde_json::Value;

use super::shared::{MoveTo, Reply, OK};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/projects/{key}/sprints", post(create_sprint))
        .route("/sprints/{id}", patch(update_sprint).delete(delete_sprint))
        .route("/sprints/{id}/start", post(start_sprint))
        .route("/sprints/{id}/complete", post(complete_sprint))
}

async fn create_sprint(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<SprintInput>) -> Reply<Sprint> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::create_sprint(tx, &project_access(tx, &user, &key)?, &input))).await.map(Json)
}

async fn update_sprint(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(input): Json<SprintInput>) -> Reply<Sprint> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::update_sprint(tx, &user, id, &input))).await.map(Json)
}

async fn start_sprint(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(input): Json<SprintInput>) -> Reply<Sprint> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::start_sprint(tx, &user, id, &input))).await.map(Json)
}

async fn complete_sprint(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(target): Json<MoveTo>) -> Reply<sprints::SprintCompletion> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::complete_sprint(tx, &user, id, target.move_to))).await.map(Json)
}

async fn delete_sprint(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::delete_sprint(tx, &user, id))).await?;

    Ok(OK)
}
