//! The routes under `/api/m/confluence`. Settings and imports use an instance-wide token,
//! so they are for administrators; the list of imports is for everyone who may see the spaces.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hub_app::app::set_setting;
use hub_app::secrets::{clear_secret, secret_hint, set_secret};
use hub_app::{ApiResult, App, AuthUser};

use crate::client::{clean_base_url, load_settings, Client, BASE_URL_SETTING, EMAIL_SETTING, TOKEN_SECRET};
use crate::import::{self, ImportConfig};
use crate::types::{
    ConfluenceImport, ConfluenceImportReport, ConfluenceImportRequest, ConfluenceRemoteSpace, ConfluenceSettings, ConfluenceSettingsUpdate,
    ConfluenceTestResult,
};

type Reply<T> = ApiResult<Json<T>>;

pub fn router() -> Router<App> {
    Router::new()
        .route("/settings", get(get_settings).put(put_settings))
        .route("/test", post(test_connection))
        .route("/spaces", get(remote_spaces))
        .route("/import", post(start_import))
        .route("/imports", get(list_imports))
        .route("/imports/{wiki_space_key}/sync", post(sync_import))
}

fn read_settings(app: &App) -> ApiResult<ConfluenceSettings> {
    app.read(|connection| {
        let settings = load_settings(connection, &app.config.secret_key)?;

        Ok(ConfluenceSettings {
            configured: !settings.base_url.is_empty() && settings.token.is_some(),
            base_url: settings.base_url,
            email: settings.email,
            token_hint: secret_hint(connection, TOKEN_SECRET)?,
        })
    })
}

async fn get_settings(State(app): State<App>, user: AuthUser) -> Reply<ConfluenceSettings> {
    user.require_admin()?;

    app.run(read_settings).await.map(Json)
}

async fn put_settings(State(app): State<App>, user: AuthUser, Json(update): Json<ConfluenceSettingsUpdate>) -> Reply<ConfluenceSettings> {
    user.require_admin()?;

    app.run(move |app| {
        let base_url = if update.base_url.trim().is_empty() {
            String::new()
        } else {
            clean_base_url(&update.base_url)?
        };
        let email = update.email.as_deref().unwrap_or("").trim().to_string();
        let token = update.token.as_deref().map(str::trim).filter(|token| !token.is_empty());

        app.db.write(|tx| {
            set_setting(tx, BASE_URL_SETTING, &base_url)?;
            set_setting(tx, EMAIL_SETTING, &email)?;

            if let Some(token) = token {
                set_secret(tx, &app.config.secret_key, TOKEN_SECRET, token)?;
            } else if update.clear_token == Some(true) {
                clear_secret(tx, TOKEN_SECRET)?;
            }

            Ok(())
        })?;

        read_settings(app)
    })
    .await
    .map(Json)
}

async fn client(app: &App) -> ApiResult<Client> {
    app.run(|app| app.read(|connection| Client::new(&load_settings(connection, &app.config.secret_key)?)))
        .await
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> ApiResult<T> + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| hub_app::ApiError::internal(format!("the request to Confluence stopped: {error}")))?
}

/// Tries the stored address and token. A refusal by Confluence is an answer, not an error.
async fn test_connection(State(app): State<App>, user: AuthUser) -> Reply<ConfluenceTestResult> {
    user.require_admin()?;

    let client = client(&app).await?;
    let base = client.base().to_string();
    let result = match blocking(move || client.check()).await {
        Ok(()) => ConfluenceTestResult {
            ok: true,
            message: format!("Connected to {base}"),
        },
        Err(error) => ConfluenceTestResult {
            ok: false,
            message: error.message,
        },
    };

    Ok(Json(result))
}

async fn remote_spaces(State(app): State<App>, user: AuthUser) -> Reply<Vec<ConfluenceRemoteSpace>> {
    user.require_admin()?;

    let client = client(&app).await?;
    let spaces = blocking(move || client.spaces()).await?;

    Ok(Json(
        spaces
            .into_iter()
            .map(|space| ConfluenceRemoteSpace {
                key: space.key,
                name: space.name,
            })
            .collect(),
    ))
}

async fn start_import(State(app): State<App>, user: AuthUser, Json(request): Json<ConfluenceImportRequest>) -> Reply<ConfluenceImportReport> {
    user.require_admin()?;

    let target = request
        .target_space_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .unwrap_or(&request.space_key)
        .to_string();
    let config = ImportConfig {
        space_key: request.space_key,
        root_page_id: request.root_page_id.map(|root| root.trim().to_string()).filter(|root| !root.is_empty()),
        exclude: request
            .exclude
            .iter()
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect(),
        base_url: String::new(),
    };

    import::run(&app, config, target).await.map(Json)
}

async fn list_imports(State(app): State<App>, user: AuthUser) -> Reply<Vec<ConfluenceImport>> {
    app.run(move |app| app.read(|connection| import::list(connection, &user))).await.map(Json)
}

async fn sync_import(State(app): State<App>, user: AuthUser, Path(wiki_space_key): Path<String>) -> Reply<ConfluenceImportReport> {
    user.require_admin()?;

    let (target, config) = app
        .run(move |app| app.read(|connection| import::stored_config(connection, &wiki_space_key)))
        .await?;

    import::run(&app, config, target).await.map(Json)
}
