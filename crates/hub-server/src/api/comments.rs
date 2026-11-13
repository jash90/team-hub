//! Comments on issues.

use axum::extract::{Path, State};
use axum::routing::{patch, post};
use axum::{Json, Router};
use hub_app::services::issues;
use hub_app::{App, AuthUser};
use hub_core::{Comment, Id};
use serde::Deserialize;
use serde_json::Value;

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new().route("/issues/{key}/comments", post(add_comment)).route("/comments/{id}", patch(update_comment).delete(delete_comment))
}

#[derive(Deserialize)]
struct BodyRequest {
    body: String,
}

async fn add_comment(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<BodyRequest>) -> Reply<Comment> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            access.require_edit()?;

            issues::add_comment(tx, id, &request.body)
        })
    })
    .await
    .map(Json)
}

async fn update_comment(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<BodyRequest>) -> Reply<Comment> {
    app.run(move |app| app.write(user.actor(), |tx| issues::update_comment(tx, &user, id, &request.body))).await.map(Json)
}

async fn delete_comment(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| issues::delete_comment(tx, &user, id))).await?;

    Ok(OK)
}
