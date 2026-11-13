//! The caller's notifications.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use hub_app::services::notifications;
use hub_app::{App, AuthUser};
use hub_core::{Id, Notification};
use serde::Deserialize;
use serde_json::Value;

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new().route("/notifications", get(list_notifications)).route("/notifications/read", post(mark_read))
}

async fn list_notifications(State(app): State<App>, user: AuthUser) -> Reply<Vec<Notification>> {
    app.run(move |app| app.read(|connection| notifications::list_notifications(connection, user.id, 50))).await.map(Json)
}

#[derive(Deserialize)]
struct ReadRequest {
    id: Option<Id>,
}

async fn mark_read(State(app): State<App>, user: AuthUser, Json(request): Json<ReadRequest>) -> Reply<Value> {
    app.run(move |app| app.db.write(|tx| notifications::mark_read(tx, user.id, request.id))).await?;

    Ok(OK)
}
