//! Time logged on issues.

use axum::extract::{Path, State};
use axum::routing::{delete, post};
use axum::{Json, Router};
use hub_app::services::issues;
use hub_app::{ApiError, App, AuthUser};
use hub_core::{Id, Worklog};
use serde::Deserialize;
use serde_json::Value;

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new().route("/issues/{key}/worklogs", post(add_worklog)).route("/worklogs/{id}", delete(delete_worklog))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorklogRequest {
    /// Minutes, or a duration such as "1h 30m".
    minutes: Option<i64>,
    spent: Option<String>,
    started_at: Option<String>,
    #[serde(default)]
    note: String,
}

async fn add_worklog(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<WorklogRequest>) -> Reply<Worklog> {
    let minutes = request
        .minutes
        .or_else(|| request.spent.as_deref().and_then(hub_core::text::parse_duration))
        .ok_or_else(|| ApiError::bad_request("Time spent looks like \"1h 30m\""))?;

    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            access.require_edit()?;

            issues::add_worklog(tx, id, minutes, request.started_at.as_deref(), &request.note)
        })
    })
    .await
    .map(Json)
}

async fn delete_worklog(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| issues::delete_worklog(tx, &user, id))).await?;

    Ok(OK)
}
