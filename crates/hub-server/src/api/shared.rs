//! What several resources of the API use: the reply type and the request shapes they share.

use axum::Json;
use hub_app::access::project_access;
use hub_app::services::projects;
use hub_app::{ApiResult, App, AuthUser};
use hub_core::{Id, Project};
use serde::Deserialize;
use serde_json::Value;

pub type Reply<T> = ApiResult<Json<T>>;

pub const OK: Json<Value> = Json(Value::Null);

/// Runs a change to a project's configuration and returns the project as it is afterwards.
pub async fn configure(
    app: App,
    user: AuthUser,
    key: String,
    change: impl FnOnce(&hub_app::Tx, &hub_app::access::ProjectAccess) -> ApiResult<()> + Send + 'static,
) -> Reply<Project> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let access = project_access(tx, &user, &key)?;
            change(tx, &access)?;

            projects::get_project(tx, &user, &key)
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MoveTo {
    pub move_to: Option<Id>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
