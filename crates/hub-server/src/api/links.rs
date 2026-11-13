//! Links between issues.

use axum::extract::{Path, State};
use axum::routing::{delete, post};
use axum::{Json, Router};
use hub_app::services::issues;
use hub_app::{App, AuthUser};
use hub_core::{Id, LinkType};
use serde::Deserialize;
use serde_json::Value;

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new().route("/issues/{key}/links", post(add_link)).route("/links/{id}", delete(delete_link))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkRequest {
    #[serde(rename = "type")]
    link_type: LinkType,
    target_key: String,
}

async fn add_link(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<LinkRequest>) -> Reply<Value> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            access.require_edit()?;

            issues::add_link(tx, &user, id, request.link_type, &request.target_key)
        })
    })
    .await?;

    Ok(OK)
}

async fn delete_link(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| issues::delete_link(tx, &user, id))).await?;

    Ok(OK)
}
