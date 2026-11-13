//! Signing in and out, the first-run setup, and the caller's API tokens.

use axum::extract::{Path, State};
use axum::http::header::SET_COOKIE;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use hub_app::auth::{self, MaybeUser};
use hub_app::services::users::NewUser;
use hub_app::services::{notifications, projects, users};
use hub_app::{seed, ApiError, ApiResult, App, AuthUser};
use hub_core::{ApiToken, Bootstrap, Id, Role, User};
use serde::Deserialize;
use serde_json::{json, Value};

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/bootstrap", get(bootstrap))
        .route("/setup", post(setup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me/tokens", get(list_tokens).post(create_token))
        .route("/me/tokens/{id}", delete(delete_token))
}

async fn bootstrap(State(app): State<App>, caller: MaybeUser) -> Reply<Bootstrap> {
    app.run(move |app| {
        app.read(|connection| {
            let user = caller.user.as_ref().map(|user| users::get_user(connection, user.id)).transpose()?;
            let signed_in = caller.user.as_ref();

            Ok(Bootstrap {
                setup_required: users::setup_required(connection)?,
                csrf: caller.csrf.clone(),
                users: if signed_in.is_some() { users::list_users(connection)? } else { Vec::new() },
                projects: signed_in.map(|user| projects::list_projects(connection, user)).transpose()?.unwrap_or_default(),
                modules: if signed_in.is_some() { app.module_infos() } else { Vec::new() },
                version: env!("CARGO_PKG_VERSION").to_string(),
                unread_notifications: signed_in.map(|user| notifications::unread_count(connection, user.id)).transpose()?.unwrap_or(0),
                user,
            })
        })
    })
    .await
    .map(Json)
}

fn signed_in(app: &App, user: User, session: auth::Session) -> Response {
    let secure = app.config.base_url.starts_with("https://");

    ([(SET_COOKIE, auth::session_cookie(&session.token, secure))], Json(json!({ "user": user, "csrf": session.csrf }))).into_response()
}

#[derive(Deserialize)]
struct SetupRequest {
    name: String,
    email: String,
    password: String,
    #[serde(default)]
    demo: bool,
}

/// Creates the first account, an administrator. Works exactly once.
async fn setup(State(app): State<App>, Json(request): Json<SetupRequest>) -> ApiResult<Response> {
    let (user, session) = app
        .run(move |app| {
            let user = app.db.write(|tx| {
                if !users::setup_required(tx)? {
                    return Err(ApiError::conflict("This instance is already set up"));
                }

                let new = NewUser {
                    name: request.name,
                    email: request.email,
                    password: Some(request.password),
                    role: Some(Role::Admin),
                    github_login: None,
                };

                users::create_user(tx, &new)
            })?;

            if request.demo {
                seed::seed_demo(app, user.id)?;
            }

            let session = app.db.write(|tx| auth::create_session(tx, user.id))?;

            Ok((user, session))
        })
        .await?;

    Ok(signed_in(&app, user, session))
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

async fn login(State(app): State<App>, Json(request): Json<LoginRequest>) -> ApiResult<Response> {
    let (user, session) = app.run(move |app| users::login(app, &request.email, &request.password)).await?;

    Ok(signed_in(&app, user, session))
}

async fn logout(State(app): State<App>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = auth::cookie_value(&headers, auth::SESSION_COOKIE) {
        app.run(move |app| app.db.write(|tx| auth::destroy_session(tx, &token))).await?;
    }

    Ok(([(SET_COOKIE, auth::clear_session_cookie())], OK).into_response())
}

async fn list_tokens(State(app): State<App>, user: AuthUser) -> Reply<Vec<ApiToken>> {
    app.run(move |app| app.read(|connection| users::list_tokens(connection, user.id))).await.map(Json)
}

#[derive(Deserialize)]
struct NameRequest {
    name: String,
}

async fn create_token(State(app): State<App>, user: AuthUser, Json(request): Json<NameRequest>) -> Reply<Value> {
    let token = app.run(move |app| app.db.write(|tx| users::create_token(tx, user.id, &request.name))).await?;

    Ok(Json(json!({ "token": token })))
}

async fn delete_token(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.db.write(|tx| users::delete_token(tx, user.id, id))).await?;

    Ok(OK)
}
