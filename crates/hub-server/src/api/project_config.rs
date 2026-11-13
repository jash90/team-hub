//! A project's configuration: statuses, issue types, components, members, transitions, and
//! which modules are on in it. Every route answers with the project as it is afterwards.

use axum::extract::{Path, Query, State};
use axum::routing::{patch, post, put};
use axum::{Json, Router};
use hub_app::services::project_modules;
use hub_app::services::projects::{self, ComponentInput, StatusInput, TransitionInput, TypeInput};
use hub_app::{ApiError, App, AuthUser};
use hub_core::{Id, Project, ProjectRole};
use serde::Deserialize;

use super::shared::{configure, MoveTo, Reply};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/projects/{key}/statuses", post(add_status))
        .route("/projects/{key}/statuses/{id}", patch(update_status).delete(delete_status))
        .route("/projects/{key}/types", post(add_type))
        .route("/projects/{key}/types/{id}", patch(update_type).delete(delete_type))
        .route("/projects/{key}/components", post(add_component))
        .route("/projects/{key}/components/{id}", patch(update_component).delete(delete_component))
        .route("/projects/{key}/members/{user_id}", put(set_member))
        .route("/projects/{key}/transitions", put(replace_transitions))
        .route("/projects/{key}/modules/{id}", put(set_module))
}

async fn add_status(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<StatusInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::add_status(tx, access, &input).map(|_| ())).await
}

async fn update_status(
    State(app): State<App>,
    user: AuthUser,
    Path((key, id)): Path<(String, Id)>,
    Json(input): Json<StatusInput>,
) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::update_status(tx, access, id, &input)).await
}

async fn delete_status(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>, Query(target): Query<MoveTo>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::delete_status(tx, access, id, target.move_to)).await
}

async fn add_type(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<TypeInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::add_type(tx, access, &input).map(|_| ())).await
}

async fn update_type(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>, Json(input): Json<TypeInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::update_type(tx, access, id, &input)).await
}

async fn delete_type(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>, Query(target): Query<MoveTo>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::delete_type(tx, access, id, target.move_to)).await
}

async fn add_component(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<ComponentInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::add_component(tx, access, &input).map(|_| ())).await
}

async fn update_component(
    State(app): State<App>,
    user: AuthUser,
    Path((key, id)): Path<(String, Id)>,
    Json(input): Json<ComponentInput>,
) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::update_component(tx, access, id, &input)).await
}

async fn delete_component(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::delete_component(tx, access, id)).await
}

#[derive(Deserialize)]
struct MemberRequest {
    role: Option<ProjectRole>,
}

async fn set_member(
    State(app): State<App>,
    user: AuthUser,
    Path((key, user_id)): Path<(String, Id)>,
    Json(request): Json<MemberRequest>,
) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::set_member(tx, access, user_id, request.role)).await
}

async fn replace_transitions(
    State(app): State<App>,
    user: AuthUser,
    Path(key): Path<String>,
    Json(transitions): Json<Vec<TransitionInput>>,
) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::replace_transitions(tx, access, &transitions)).await
}

#[derive(Deserialize)]
struct ModuleRequest {
    enabled: bool,
}

/// Switches a module on or off in this project only.
async fn set_module(
    State(app): State<App>,
    user: AuthUser,
    Path((key, id)): Path<(String, String)>,
    Json(request): Json<ModuleRequest>,
) -> Reply<Project> {
    if !app.modules.iter().any(|module| module.meta().id == id) {
        return Err(ApiError::not_found("Module"));
    }

    configure(app, user, key, move |tx, access| project_modules::set_project_module(tx, access, &id, request.enabled)).await
}
