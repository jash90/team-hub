//! The schedule: a project's recurring tasks, running them, and the instance's settings
//! (`docs/schedule.md`).

use axum::extract::{Path, State};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use hub_app::access::project_access;
use hub_app::services::schedule::{self, NewTask, TaskPatch};
use hub_app::{App, AuthUser};
use hub_core::schedule::{ScheduleSettings, ScheduledTask, TaskKind, TaskSlot};
use hub_core::Id;
use serde::Deserialize;
use serde_json::{json, Value};

use super::shared::Reply;

pub fn routes() -> Router<App> {
    Router::new()
        .route("/task-kinds", get(task_kinds))
        .route("/projects/{key}/tasks", get(list_tasks).post(create_task))
        .route("/projects/{key}/tasks/run", post(run_slot))
        .route("/tasks/{id}", patch(update_task).delete(delete_task))
        .route("/tasks/{id}/run", post(run_task))
        .route("/schedule", get(settings))
        .route("/admin/schedule", put(save_settings))
}

async fn task_kinds(State(app): State<App>, _user: AuthUser) -> Reply<Vec<TaskKind>> {
    Ok(Json(schedule::task_kinds(&app)))
}

async fn list_tasks(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Vec<ScheduledTask>> {
    app.run(move |app| app.read(|connection| schedule::list(app, connection, project_access(connection, &user, &key)?.id))).await.map(Json)
}

async fn create_task(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(new): Json<NewTask>) -> Reply<ScheduledTask> {
    app.run(move |app| app.write(user.actor(), |tx| schedule::create(app, tx, &user, &project_access(tx, &user, &key)?, &new))).await.map(Json)
}

async fn update_task(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(patch): Json<TaskPatch>) -> Reply<ScheduledTask> {
    app.run(move |app| app.write(user.actor(), |tx| schedule::update(app, tx, &user, id, &patch))).await.map(Json)
}

async fn delete_task(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| schedule::delete(app, tx, &user, id))).await?;

    Ok(Json(json!({ "ok": true })))
}

async fn run_task(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<ScheduledTask> {
    app.run(move |app| schedule::run_task(app, &user, id)).await.map(Json)
}

#[derive(Deserialize)]
struct SlotRequest {
    slot: TaskSlot,
}

async fn run_slot(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<SlotRequest>) -> Reply<Vec<ScheduledTask>> {
    app.run(move |app| schedule::run_slot(app, &user, &key, request.slot)).await.map(Json)
}

async fn settings(State(app): State<App>, _user: AuthUser) -> Reply<ScheduleSettings> {
    app.run(move |app| app.read(schedule::settings)).await.map(Json)
}

async fn save_settings(State(app): State<App>, user: AuthUser, Json(wanted): Json<ScheduleSettings>) -> Reply<ScheduleSettings> {
    app.run(move |app| app.write(user.actor(), |tx| schedule::save_settings(tx, &user, &wanted))).await.map(Json)
}
