//! The core REST API, one file per resource. Handlers are thin: they move the request onto
//! the worker pool, check access, and call a service.

mod admin;
mod attachments;
mod boards;
mod comments;
mod filters;
mod issues;
mod links;
mod notifications;
mod overview;
mod project_config;
mod projects;
mod runners;
mod search;
mod session;
mod shared;
mod sprints;
mod stream;
mod tasks;
mod users;
mod versions;
mod worklogs;

use axum::http::StatusCode;
use axum::{Json, Router};
use hub_app::App;
use serde_json::{json, Value};

pub fn router() -> Router<App> {
    Router::new()
        .merge(session::routes())
        .merge(users::routes())
        .merge(projects::routes())
        .merge(project_config::routes())
        .merge(boards::routes())
        .merge(sprints::routes())
        .merge(versions::routes())
        .merge(issues::routes())
        .merge(comments::routes())
        .merge(links::routes())
        .merge(attachments::routes())
        .merge(worklogs::routes())
        .merge(search::routes())
        .merge(filters::routes())
        .merge(overview::routes())
        .merge(notifications::routes())
        .merge(runners::routes())
        .merge(tasks::routes())
        .merge(stream::routes())
        .merge(admin::routes())
}

pub async fn not_found() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_FOUND, Json(json!({ "error": { "code": "not_found", "message": "No such endpoint" } })))
}
