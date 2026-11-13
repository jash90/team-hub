//! Saved filters.

use axum::extract::{Path, State};
use axum::routing::{get, patch};
use axum::{Json, Router};
use hub_app::services::overview;
use hub_app::{App, AuthUser};
use hub_core::{Id, SavedFilter};
use serde_json::Value;

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new().route("/filters", get(list_filters).post(create_filter)).route("/filters/{id}", patch(update_filter).delete(delete_filter))
}

async fn list_filters(State(app): State<App>, user: AuthUser) -> Reply<Vec<SavedFilter>> {
    app.run(move |app| app.read(|connection| overview::list_filters(connection, &user))).await.map(Json)
}

async fn create_filter(State(app): State<App>, user: AuthUser, Json(input): Json<overview::FilterInput>) -> Reply<SavedFilter> {
    app.run(move |app| app.db.write(|tx| overview::save_filter(tx, &user, None, &input))).await.map(Json)
}

async fn update_filter(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(input): Json<overview::FilterInput>) -> Reply<SavedFilter> {
    app.run(move |app| app.db.write(|tx| overview::save_filter(tx, &user, Some(id), &input))).await.map(Json)
}

async fn delete_filter(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.db.write(|tx| overview::delete_filter(tx, &user, id))).await?;

    Ok(OK)
}
