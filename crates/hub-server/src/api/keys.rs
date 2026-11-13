//! The caller's own keys (`docs/runners.md` → Shared runners): set, replaced and removed,
//! never read back.

use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use hub_app::services::personal_keys;
use hub_app::{App, AuthUser};
use hub_core::runners::PersonalKeyState;
use serde::Deserialize;

use super::shared::Reply;

pub fn routes() -> Router<App> {
    Router::new().route("/me/keys", get(list_keys)).route("/me/keys/{name}", put(set_key).delete(clear_key))
}

async fn list_keys(State(app): State<App>, user: AuthUser) -> Reply<Vec<PersonalKeyState>> {
    app.run(move |app| app.read(|connection| personal_keys::list(app, connection, &user))).await.map(Json)
}

#[derive(Deserialize)]
struct NewValue {
    value: String,
}

async fn set_key(State(app): State<App>, user: AuthUser, Path(name): Path<String>, Json(new): Json<NewValue>) -> Reply<Vec<PersonalKeyState>> {
    app.run(move |app| {
        app.db.write(|tx| {
            personal_keys::set(app, tx, &user, &name, &new.value)?;
            personal_keys::list(app, tx, &user)
        })
    })
    .await
    .map(Json)
}

async fn clear_key(State(app): State<App>, user: AuthUser, Path(name): Path<String>) -> Reply<Vec<PersonalKeyState>> {
    app.run(move |app| {
        app.db.write(|tx| {
            personal_keys::clear(app, tx, &user, &name)?;
            personal_keys::list(app, tx, &user)
        })
    })
    .await
    .map(Json)
}
