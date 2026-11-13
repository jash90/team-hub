//! The board and the backlog of a project.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use hub_app::services::board;
use hub_app::{App, AuthUser};
use hub_core::{Backlog, Board, Id};
use serde::Deserialize;

use super::shared::Reply;

pub fn routes() -> Router<App> {
    Router::new().route("/projects/{key}/board", get(get_board)).route("/projects/{key}/backlog", get(get_backlog))
}

#[derive(Deserialize)]
struct BoardQuery {
    sprint: Option<Id>,
}

async fn get_board(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Query(query): Query<BoardQuery>) -> Reply<Board> {
    app.run(move |app| board::board(app, &user, &key, query.sprint)).await.map(Json)
}

async fn get_backlog(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Backlog> {
    app.run(move |app| board::backlog(app, &user, &key)).await.map(Json)
}
