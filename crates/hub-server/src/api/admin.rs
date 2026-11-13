//! Instance administration: switching modules.

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use hub_app::{App, AuthUser};
use serde::Deserialize;
use serde_json::{json, Value};

use super::shared::Reply;

pub fn routes() -> Router<App> {
    Router::new().route("/admin/modules/{id}", put(set_module))
}

#[derive(Deserialize)]
struct ModuleRequest {
    enabled: bool,
}

async fn set_module(State(app): State<App>, user: AuthUser, Path(id): Path<String>, Json(request): Json<ModuleRequest>) -> Reply<Value> {
    user.require_admin()?;
    app.run(move |app| {
        app.set_module_enabled(&id, request.enabled)?;

        // Tells every open browser to load or drop the module's screens.
        app.write(user.actor(), |tx| tx.emit("instance.modules", None, None, json!({ "module": id, "enabled": request.enabled })))
    })
    .await?;

    Ok(Json(json!({ "modules": app.module_infos() })))
}
