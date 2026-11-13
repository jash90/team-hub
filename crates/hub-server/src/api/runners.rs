//! The machines that carry out agent runs and test jobs, as the modules know them.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use hub_app::services::runners;
use hub_app::{App, AuthUser};
use hub_core::runners::Runner;

use super::shared::Reply;

pub fn routes() -> Router<App> {
    Router::new().route("/runners", get(list_runners))
}

async fn list_runners(State(app): State<App>, user: AuthUser) -> Reply<Vec<Runner>> {
    app.run(move |app| runners::list(app, &user, Utc::now())).await.map(Json)
}
