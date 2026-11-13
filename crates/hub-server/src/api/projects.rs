//! Projects: the list, one with its configuration, and its own fields.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use hub_app::services::projects::{self, NewProject, ProjectPatch};
use hub_app::{App, AuthUser};
use hub_core::{Project, ProjectSummary};

use super::shared::{configure, Reply};

pub fn routes() -> Router<App> {
    Router::new().route("/projects", get(list_projects).post(create_project)).route("/projects/{key}", get(get_project).patch(update_project))
}

async fn list_projects(State(app): State<App>, user: AuthUser) -> Reply<Vec<ProjectSummary>> {
    app.run(move |app| app.read(|connection| projects::list_projects(connection, &user))).await.map(Json)
}

async fn create_project(State(app): State<App>, user: AuthUser, Json(new): Json<NewProject>) -> Reply<Project> {
    app.run(move |app| app.write(user.actor(), |tx| projects::create_project(tx, &user, &new))).await.map(Json)
}

async fn get_project(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Project> {
    app.run(move |app| app.read(|connection| projects::get_project(connection, &user, &key))).await.map(Json)
}

async fn update_project(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(patch): Json<ProjectPatch>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::update_project(tx, access, &patch)).await
}
