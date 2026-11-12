//! The core REST API. Handlers are thin: they move the request onto the worker pool, check
//! access, and call a service.

use std::convert::Infallible;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use futures_util::{Stream, StreamExt};
use hub_app::access::{project_access, project_access_by_id};
use hub_app::auth::{self, MaybeUser};
use hub_app::services::projects::{ComponentInput, NewProject, ProjectPatch, StatusInput, TransitionInput, TypeInput};
use hub_app::services::sprints::SprintInput;
use hub_app::services::users::{NewUser, UserPatch};
use hub_app::services::versions::VersionInput;
use hub_app::services::{board, issues, notifications, overview, projects, search, sprints, users, versions};
use hub_app::{seed, ApiError, ApiResult, App, AuthUser};
use hub_core::query::QueryFieldInfo;
use hub_core::{
    ActivityItem, ApiToken, Attachment, Backlog, Board, Bootstrap, Comment, Dashboard, Id, Issue, IssuePatch, IssueRow, LinkType, NewIssue,
    Notification, Project, ProjectRole, ProjectSummary, QuickHit, Role, SavedFilter, SearchResult, Sprint, User, Version, Worklog,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::wrappers::BroadcastStream;

type Reply<T> = ApiResult<Json<T>>;

const OK: Json<Value> = Json(Value::Null);
const MAX_UPLOAD_BYTES: usize = 26 * 1024 * 1024;

pub fn router() -> Router<App> {
    Router::new()
        .route("/bootstrap", get(bootstrap))
        .route("/setup", post(setup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me/tokens", get(list_tokens).post(create_token))
        .route("/me/tokens/{id}", delete(delete_token))
        .route("/users", get(list_users).post(create_user))
        .route("/users/{id}", patch(update_user))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{key}", get(get_project).patch(update_project))
        .route("/projects/{key}/statuses", post(add_status))
        .route("/projects/{key}/statuses/{id}", patch(update_status).delete(delete_status))
        .route("/projects/{key}/types", post(add_type))
        .route("/projects/{key}/types/{id}", patch(update_type).delete(delete_type))
        .route("/projects/{key}/components", post(add_component))
        .route("/projects/{key}/components/{id}", patch(update_component).delete(delete_component))
        .route("/projects/{key}/members/{user_id}", put(set_member))
        .route("/projects/{key}/transitions", put(replace_transitions))
        .route("/projects/{key}/board", get(get_board))
        .route("/projects/{key}/backlog", get(get_backlog))
        .route("/projects/{key}/sprints", post(create_sprint))
        .route("/projects/{key}/versions", post(create_version))
        .route("/sprints/{id}", patch(update_sprint).delete(delete_sprint))
        .route("/sprints/{id}/start", post(start_sprint))
        .route("/sprints/{id}/complete", post(complete_sprint))
        .route("/versions/{id}", patch(update_version).delete(delete_version))
        .route("/versions/{id}/release", post(release_version))
        .route("/versions/{id}/notes", get(version_notes))
        .route("/issues", post(create_issue))
        .route("/issues/bulk", post(bulk_update))
        .route("/issues/{key}", get(get_issue).patch(update_issue).delete(delete_issue))
        .route("/issues/{key}/move", post(move_issue))
        .route("/issues/{key}/comments", post(add_comment))
        .route("/issues/{key}/links", post(add_link))
        .route("/issues/{key}/watch", put(set_watch))
        .route("/issues/{key}/worklogs", post(add_worklog))
        .route("/issues/{key}/attachments", post(upload_attachment).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/comments/{id}", patch(update_comment).delete(delete_comment))
        .route("/links/{id}", delete(delete_link))
        .route("/worklogs/{id}", delete(delete_worklog))
        .route("/attachments/{id}", get(download_attachment).delete(delete_attachment))
        .route("/search", get(run_search))
        .route("/search/fields", get(search_fields))
        .route("/quick", get(quick))
        .route("/filters", get(list_filters).post(create_filter))
        .route("/filters/{id}", patch(update_filter).delete(delete_filter))
        .route("/dashboard", get(dashboard))
        .route("/activity", get(activity))
        .route("/notifications", get(list_notifications))
        .route("/notifications/read", post(mark_read))
        .route("/stream", get(stream))
        .route("/admin/modules/{id}", put(set_module))
}

// ---------- session ----------

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

                let new = NewUser { name: request.name, email: request.email, password: Some(request.password), role: Some(Role::Admin), github_login: None };

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

// ---------- people ----------

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

// ---------- projects ----------

async fn list_projects(State(app): State<App>, user: AuthUser) -> Reply<Vec<ProjectSummary>> {
    app.run(move |app| app.read(|connection| projects::list_projects(connection, &user))).await.map(Json)
}

async fn create_project(State(app): State<App>, user: AuthUser, Json(new): Json<NewProject>) -> Reply<Project> {
    app.run(move |app| app.write(user.actor(), |tx| projects::create_project(tx, &user, &new))).await.map(Json)
}

async fn get_project(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Project> {
    app.run(move |app| app.read(|connection| projects::get_project(connection, &user, &key))).await.map(Json)
}

/// Runs a change to a project's configuration and returns the project as it is afterwards.
async fn configure(
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

async fn update_project(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(patch): Json<ProjectPatch>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::update_project(tx, access, &patch)).await
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MoveTo {
    move_to: Option<Id>,
}

async fn add_status(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<StatusInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::add_status(tx, access, &input).map(|_| ())).await
}

async fn update_status(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>, Json(input): Json<StatusInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::update_status(tx, access, id, &input)).await
}

async fn delete_status(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>, Query(target): Query<MoveTo>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::delete_status(tx, access, id, target.move_to)).await
}

async fn add_type(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<TypeInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::add_type(tx, access, &input).map(|_| ())).await
}

async fn update_type(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>, Json(input): Json<TypeInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::update_type(tx, access, id, &input)).await
}

async fn delete_type(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>, Query(target): Query<MoveTo>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::delete_type(tx, access, id, target.move_to)).await
}

async fn add_component(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<ComponentInput>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::add_component(tx, access, &input).map(|_| ())).await
}

async fn update_component(
    State(app): State<App>,
    user: AuthUser,
    Path((key, id)): Path<(String, Id)>,
    Json(input): Json<ComponentInput>,
) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::update_component(tx, access, id, &input)).await
}

async fn delete_component(State(app): State<App>, user: AuthUser, Path((key, id)): Path<(String, Id)>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::delete_component(tx, access, id)).await
}

#[derive(Deserialize)]
struct MemberRequest {
    role: Option<ProjectRole>,
}

async fn set_member(State(app): State<App>, user: AuthUser, Path((key, user_id)): Path<(String, Id)>, Json(request): Json<MemberRequest>) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::set_member(tx, access, user_id, request.role)).await
}

async fn replace_transitions(
    State(app): State<App>,
    user: AuthUser,
    Path(key): Path<String>,
    Json(transitions): Json<Vec<TransitionInput>>,
) -> Reply<Project> {
    configure(app, user, key, move |tx, access| projects::replace_transitions(tx, access, &transitions)).await
}

// ---------- boards, sprints, versions ----------

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

async fn create_sprint(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<SprintInput>) -> Reply<Sprint> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::create_sprint(tx, &project_access(tx, &user, &key)?, &input))).await.map(Json)
}

async fn update_sprint(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(input): Json<SprintInput>) -> Reply<Sprint> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::update_sprint(tx, &user, id, &input))).await.map(Json)
}

async fn start_sprint(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(input): Json<SprintInput>) -> Reply<Sprint> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::start_sprint(tx, &user, id, &input))).await.map(Json)
}

async fn complete_sprint(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(target): Json<MoveTo>) -> Reply<sprints::SprintCompletion> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::complete_sprint(tx, &user, id, target.move_to))).await.map(Json)
}

async fn delete_sprint(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| sprints::delete_sprint(tx, &user, id))).await?;

    Ok(OK)
}

async fn create_version(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<VersionInput>) -> Reply<Version> {
    app.run(move |app| app.write(user.actor(), |tx| versions::create_version(tx, &project_access(tx, &user, &key)?, &input))).await.map(Json)
}

async fn update_version(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(input): Json<VersionInput>) -> Reply<Version> {
    app.run(move |app| app.write(user.actor(), |tx| versions::update_version(tx, &user, id, &input))).await.map(Json)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseRequest {
    released: bool,
    move_to: Option<Id>,
}

async fn release_version(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<ReleaseRequest>) -> Reply<Version> {
    app.run(move |app| app.write(user.actor(), |tx| versions::set_released(tx, &user, id, request.released, request.move_to))).await.map(Json)
}

async fn delete_version(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| versions::delete_version(tx, &user, id))).await?;

    Ok(OK)
}

async fn version_notes(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    let notes = app.run(move |app| app.read(|connection| versions::version_notes(connection, &user, id))).await?;

    Ok(Json(json!({ "markdown": notes })))
}

// ---------- issues ----------

async fn create_issue(State(app): State<App>, user: AuthUser, Json(new): Json<NewIssue>) -> Reply<IssueRow> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            project_access(tx, &user, &new.project_key)?.require_edit()?;
            let id = issues::create_issue(tx, &new)?;

            issues::row_by_id(app, tx, id)
        })
    })
    .await
    .map(Json)
}

async fn get_issue(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Issue> {
    app.run(move |app| issues::issue_detail(app, &user, &key)).await.map(Json)
}

async fn update_issue(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(patch): Json<IssuePatch>) -> Reply<IssueRow> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            access.require_edit()?;
            issues::update_issue(tx, id, &patch)?;

            issues::row_by_id(app, tx, id)
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct BulkRequest {
    keys: Vec<String>,
    patch: IssuePatch,
}

/// The same patch on many issues, all or nothing.
async fn bulk_update(State(app): State<App>, user: AuthUser, Json(request): Json<BulkRequest>) -> Reply<Value> {
    let count = app
        .run(move |app| {
            app.write(user.actor(), |tx| {
                for key in &request.keys {
                    let (id, access) = issues::issue_access(tx, &user, key)?;
                    access.require_edit()?;
                    issues::update_issue(tx, id, &request.patch).map_err(|error| ApiError::new(error.status, error.code, format!("{key}: {}", error.message)))?;
                }

                Ok(request.keys.len())
            })
        })
        .await?;

    Ok(Json(json!({ "updated": count })))
}

async fn delete_issue(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Value> {
    app.run(move |app| {
        let files = app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            let reporter: Option<Id> = tx.query_row("SELECT reporter_id FROM issue WHERE id = ?1", [id], |row| row.get(0))?;

            if reporter != Some(user.id) {
                access.require_admin()?;
            }

            access.require_edit()?;

            issues::delete_issue(tx, id)
        })?;

        for file in files {
            let _ = std::fs::remove_file(app.config.attachments_dir().join(file));
        }

        Ok(())
    })
    .await?;

    Ok(OK)
}

async fn move_issue(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<issues::MoveRequest>) -> Reply<IssueRow> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            access.require_edit()?;
            issues::move_issue(tx, id, &request)?;

            issues::row_by_id(app, tx, id)
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct BodyRequest {
    body: String,
}

async fn add_comment(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<BodyRequest>) -> Reply<Comment> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            access.require_edit()?;

            issues::add_comment(tx, id, &request.body)
        })
    })
    .await
    .map(Json)
}

async fn update_comment(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<BodyRequest>) -> Reply<Comment> {
    app.run(move |app| app.write(user.actor(), |tx| issues::update_comment(tx, &user, id, &request.body))).await.map(Json)
}

async fn delete_comment(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| issues::delete_comment(tx, &user, id))).await?;

    Ok(OK)
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

#[derive(Deserialize)]
struct WatchRequest {
    watching: bool,
}

async fn set_watch(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<WatchRequest>) -> Reply<Value> {
    app.run(move |app| {
        app.db.write(|tx| {
            let (id, _) = issues::issue_access(tx, &user, &key)?;

            issues::set_watching(tx, id, user.id, request.watching)
        })
    })
    .await?;

    Ok(OK)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorklogRequest {
    /// Minutes, or a duration such as "1h 30m".
    minutes: Option<i64>,
    spent: Option<String>,
    started_at: Option<String>,
    #[serde(default)]
    note: String,
}

async fn add_worklog(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<WorklogRequest>) -> Reply<Worklog> {
    let minutes = request
        .minutes
        .or_else(|| request.spent.as_deref().and_then(hub_core::text::parse_duration))
        .ok_or_else(|| ApiError::bad_request("Time spent looks like \"1h 30m\""))?;

    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            access.require_edit()?;

            issues::add_worklog(tx, id, minutes, request.started_at.as_deref(), &request.note)
        })
    })
    .await
    .map(Json)
}

async fn delete_worklog(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.write(user.actor(), |tx| issues::delete_worklog(tx, &user, id))).await?;

    Ok(OK)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok()))
            .flatten();

        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

/// The file is the request body; its name travels percent-encoded in `x-filename`.
async fn upload_attachment(State(app): State<App>, user: AuthUser, Path(key): Path<String>, headers: HeaderMap, body: Bytes) -> Reply<Attachment> {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).map(String::from);
    let filename = percent_decode(&header("x-filename").unwrap_or_default());
    let mime = header("content-type").unwrap_or_else(|| "application/octet-stream".to_string());

    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (id, access) = issues::issue_access(tx, &user, &key)?;
            access.require_edit()?;

            issues::add_attachment(app, tx, id, &filename, &mime, &body)
        })
    })
    .await
    .map(Json)
}

/// Serves an attachment. Only images are shown in place; everything else downloads, and
/// nothing may run script, whatever it claims to be.
async fn download_attachment(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> ApiResult<Response> {
    let (attachment, path) = app.run(move |app| app.read(|connection| issues::attachment_file(app, connection, &user, id))).await?;
    let bytes = tokio::fs::read(&path).await.map_err(|_| ApiError::not_found("Attachment"))?;
    let inline = attachment.mime.starts_with("image/") && attachment.mime != "image/svg+xml";
    let name: String = attachment.filename.chars().filter(|character| character.is_ascii() && !matches!(character, '"' | '\\' | '\r' | '\n')).collect();
    let disposition = format!("{}; filename=\"{}\"", if inline { "inline" } else { "attachment" }, if name.is_empty() { "file" } else { &name });
    let mime = if inline { attachment.mime.clone() } else { "application/octet-stream".to_string() };

    Ok(([(CONTENT_TYPE, mime), (CONTENT_DISPOSITION, disposition), (CONTENT_SECURITY_POLICY, "sandbox; default-src 'none'".to_string())], bytes)
        .into_response())
}

async fn delete_attachment(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| {
        let path = app.write(user.actor(), |tx| issues::delete_attachment(app, tx, &user, id))?;
        let _ = std::fs::remove_file(path);

        Ok(())
    })
    .await?;

    Ok(OK)
}

// ---------- search and overview ----------

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn run_search(State(app): State<App>, user: AuthUser, Query(query): Query<SearchQuery>) -> Reply<SearchResult> {
    app.run(move |app| search::search(app, &user, &query.q, query.limit.unwrap_or(50), query.offset.unwrap_or(0))).await.map(Json)
}

async fn search_fields(State(app): State<App>, _user: AuthUser) -> Reply<Vec<QueryFieldInfo>> {
    Ok(Json(search::query_fields(&app)))
}

async fn quick(State(app): State<App>, user: AuthUser, Query(query): Query<SearchQuery>) -> Reply<Vec<QuickHit>> {
    app.run(move |app| app.read(|connection| overview::quick_search(app, connection, &user, &query.q))).await.map(Json)
}

async fn list_filters(State(app): State<App>, user: AuthUser) -> Reply<Vec<SavedFilter>> {
    app.run(move |app| app.read(|connection| overview::list_filters(connection, &user))).await.map(Json)
}

async fn create_filter(State(app): State<App>, user: AuthUser, Json(input): Json<overview::FilterInput>) -> Reply<SavedFilter> {
    app.run(move |app| app.db.write(|tx| overview::save_filter(tx, &user, None, &input))).await.map(Json)
}

async fn update_filter(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(input): Json<overview::FilterInput>) -> Reply<SavedFilter> {
    app.run(move |app| app.db.write(|tx| overview::save_filter(tx, &user, Some(id), &input))).await.map(Json)
}

async fn delete_filter(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| app.db.write(|tx| overview::delete_filter(tx, &user, id))).await?;

    Ok(OK)
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

async fn list_notifications(State(app): State<App>, user: AuthUser) -> Reply<Vec<Notification>> {
    app.run(move |app| app.read(|connection| notifications::list_notifications(connection, user.id, 50))).await.map(Json)
}

#[derive(Deserialize)]
struct ReadRequest {
    id: Option<Id>,
}

async fn mark_read(State(app): State<App>, user: AuthUser, Json(request): Json<ReadRequest>) -> Reply<Value> {
    app.run(move |app| app.db.write(|tx| notifications::mark_read(tx, user.id, request.id))).await?;

    Ok(OK)
}

/// Server-sent events: every change the caller may see, as it commits.
async fn stream(State(app): State<App>, user: AuthUser) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let events = BroadcastStream::new(app.subscribe()).filter_map(move |event| {
        let app = app.clone();
        let user = user.clone();

        async move {
            let event = event.ok()?;

            if let Some(project_id) = event.project_id {
                app.read(|connection| project_access_by_id(connection, &user, project_id)).ok()?;
            }

            Some(Ok(SseEvent::default().event("change").data(serde_json::to_string(&event).ok()?)))
        }
    });

    Sse::new(events).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct ModuleRequest {
    enabled: bool,
}

async fn set_module(State(app): State<App>, user: AuthUser, Path(id): Path<String>, Json(request): Json<ModuleRequest>) -> Reply<Value> {
    user.require_admin()?;
    app.run(move |app| app.set_module_enabled(&id, request.enabled)).await?;

    Ok(Json(json!({ "modules": app.module_infos() })))
}

pub async fn not_found() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_FOUND, Json(json!({ "error": { "code": "not_found", "message": "No such endpoint" } })))
}
