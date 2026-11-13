//! The query language: running a query, and the fields it knows.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use hub_app::services::search;
use hub_app::{App, AuthUser};
use hub_core::query::QueryFieldInfo;
use hub_core::SearchResult;

use super::shared::{Reply, SearchQuery};

pub fn routes() -> Router<App> {
    Router::new().route("/search", get(run_search)).route("/search/fields", get(search_fields))
}

async fn run_search(State(app): State<App>, user: AuthUser, Query(query): Query<SearchQuery>) -> Reply<SearchResult> {
    app.run(move |app| search::search(app, &user, &query.q, query.limit.unwrap_or(50), query.offset.unwrap_or(0))).await.map(Json)
}

async fn search_fields(State(app): State<App>, _user: AuthUser) -> Reply<Vec<QueryFieldInfo>> {
    Ok(Json(search::query_fields(&app)))
}
