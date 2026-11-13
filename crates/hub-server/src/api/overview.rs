//! What spans projects: "My work", the activity feed, the quick search.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use hub_app::services::overview;
use hub_app::{App, AuthUser};
use hub_core::{ActivityItem, Dashboard, Id, QuickHit};
use serde::Deserialize;

use super::shared::{Reply, SearchQuery};

pub fn routes() -> Router<App> {
    Router::new().route("/dashboard", get(dashboard)).route("/activity", get(activity)).route("/quick", get(quick))
}

async fn dashboard(State(app): State<App>, user: AuthUser) -> Reply<Dashboard> {
    app.run(move |app| overview::dashboard(app, &user)).await.map(Json)
}

#[derive(Deserialize)]
struct ActivityQuery {
    project: Option<String>,
    before: Option<Id>,
    limit: Option<i64>,
}

async fn activity(State(app): State<App>, user: AuthUser, Query(query): Query<ActivityQuery>) -> Reply<Vec<ActivityItem>> {
    app.run(move |app| {
        app.read(|connection| overview::activity(connection, &user, query.project.as_deref(), query.before, query.limit.unwrap_or(50)))
    })
    .await
    .map(Json)
}

async fn quick(State(app): State<App>, user: AuthUser, Query(query): Query<SearchQuery>) -> Reply<Vec<QuickHit>> {
    app.run(move |app| app.read(|connection| overview::quick_search(app, connection, &user, &query.q))).await.map(Json)
}
