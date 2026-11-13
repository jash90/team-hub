//! People.

use axum::extract::{Path, State};
use axum::routing::{get, patch};
use axum::{Json, Router};
use hub_app::services::users::{self, NewUser, UserPatch};
use hub_app::{App, AuthUser};
use hub_core::{Id, User};

use super::shared::Reply;

pub fn routes() -> Router<App> {
    Router::new().route("/users", get(list_users).post(create_user)).route("/users/{id}", patch(update_user))
}

async fn list_users(State(app): State<App>, _user: AuthUser) -> Reply<Vec<User>> {
    app.run(|app| app.read(users::list_users)).await.map(Json)
}

async fn create_user(State(app): State<App>, user: AuthUser, Json(new): Json<NewUser>) -> Reply<User> {
    user.require_admin()?;
    app.run(move |app| app.db.write(|tx| users::create_user(tx, &new))).await.map(Json)
}

async fn update_user(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(patch): Json<UserPatch>) -> Reply<User> {
    app.run(move |app| app.write(user.actor(), |tx| users::update_user(tx, &user, id, &patch))).await.map(Json)
}
