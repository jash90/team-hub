//! The routes under `/api/m/jira`. Everything here uses the instance-wide Jira token or
//! writes whole projects, so it is for administrators.
//!
//! | Route | |
//! |---|---|
//! | `GET/PUT /settings` | address, e-mail, token (only its hint is ever returned) |
//! | `POST /test` | tries the stored connection and says which Jira answered |
//! | `GET /projects` | Jira's projects with their issue counts |
//! | `GET /projects/{key}/preview` | what the project consists of, and the proposed mapping |
//! | `POST /import` | starts an import job |
//! | `GET /jobs`, `GET /jobs/{id}`, `POST /jobs/{id}/cancel` | progress and reports |
//! | `POST /csv` | uploads a CSV export (the file is the body, its name in `x-filename`) |
//! | `POST /csv/{id}/preview` | the same preview for a file, with the chosen columns |
//! | `POST /csv/{id}/import` | starts an import job from the file |

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use hub_app::app::set_setting;
use hub_app::db::{now_iso, query_all, query_one};
use hub_app::secrets::{clear_secret, secret_hint, set_secret};
use hub_app::{ApiError, ApiResult, App, AuthUser};
use hub_core::Id;
use rusqlite::Connection;

use crate::client::{clean_base_url, load_settings, origin, Client, BASE_URL_SETTING, EMAIL_SETTING, TOKEN_SECRET};
use crate::csv::{self, Parsed};
use crate::import::{self, Options, Request, Source};
use crate::jobs;
use crate::mapping::{self, Structure};
use crate::types::{
    JiraCsvPreview, JiraCsvProject, JiraCsvRequest, JiraImportRequest, JiraJob, JiraLocalProject, JiraMapping, JiraPreview, JiraRemoteProject, JiraSettings,
    JiraSettingsUpdate, JiraTestResult,
};

type Reply<T> = ApiResult<Json<T>>;

const MAX_CSV_BYTES: usize = 64 * 1024 * 1024;
/// Beyond this many projects the list comes without issue counts: one request each adds up.
const MAX_COUNTED_PROJECTS: usize = 100;
const KEPT_JOBS: i64 = 30;

pub fn router() -> Router<App> {
    Router::new()
        .route("/settings", get(get_settings).put(put_settings))
        .route("/test", post(test_connection))
        .route("/projects", get(remote_projects))
        .route("/projects/{key}/preview", get(preview))
        .route("/import", post(start_import))
        .route("/jobs", get(list_jobs))
        .route("/jobs/{id}", get(get_job))
        .route("/jobs/{id}/cancel", post(cancel_job))
        .route("/csv", post(upload_csv).layer(DefaultBodyLimit::max(MAX_CSV_BYTES)))
        .route("/csv/{id}/preview", post(preview_csv))
        .route("/csv/{id}/import", post(import_csv))
}

fn read_settings(app: &App) -> ApiResult<JiraSettings> {
    app.read(|connection| {
        let settings = load_settings(connection, &app.config.secret_key)?;

        Ok(JiraSettings {
            configured: !settings.base_url.is_empty() && settings.token.is_some(),
            base_url: settings.base_url,
            email: settings.email,
            token_hint: secret_hint(connection, TOKEN_SECRET)?,
        })
    })
}

async fn get_settings(State(app): State<App>, user: AuthUser) -> Reply<JiraSettings> {
    user.require_admin()?;

    app.run(read_settings).await.map(Json)
}

async fn put_settings(State(app): State<App>, user: AuthUser, Json(update): Json<JiraSettingsUpdate>) -> Reply<JiraSettings> {
    user.require_admin()?;

    app.run(move |app| {
        let base_url = if update.base_url.trim().is_empty() { String::new() } else { clean_base_url(&update.base_url)? };
        let email = update.email.as_deref().unwrap_or("").trim().to_string();
        let token = update.token.as_deref().map(str::trim).filter(|token| !token.is_empty());

        app.db.write(|tx| {
            // The stored token was entered for one server; nobody gets it sent to another by
            // changing the address.
            let stored = load_settings(tx, &app.config.secret_key)?;
            let moves = stored.token.is_some() && !base_url.is_empty() && origin(&stored.base_url) != origin(&base_url);

            if moves && token.is_none() && update.clear_token != Some(true) {
                return Err(ApiError::bad_request("The address points to another server; enter the token for it"));
            }

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
    app.run(|app| app.read(|connection| Client::new(&load_settings(connection, &app.config.secret_key)?))).await
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> ApiResult<T> + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(work).await.map_err(|error| ApiError::internal(format!("the request to Jira stopped: {error}")))?
}

/// A project key as Jira allows them, safe to put into an address and a query.
fn remote_key(key: &str) -> ApiResult<String> {
    let key = key.trim().to_uppercase();
    let valid = key.chars().next().is_some_and(|first| first.is_ascii_alphabetic())
        && key.len() <= 32
        && key.chars().all(|character| character.is_ascii_alphanumeric() || character == '_');

    if valid {
        Ok(key)
    } else {
        Err(ApiError::bad_request("Name the Jira project by its key, e.g. WEB"))
    }
}

/// Tries the stored address and token. A refusal by Jira is an answer, not an error.
async fn test_connection(State(app): State<App>, user: AuthUser) -> Reply<JiraTestResult> {
    user.require_admin()?;

    let mut client = client(&app).await?;
    let base = client.base().to_string();
    let result = match blocking(move || client.detect()).await {
        Ok(info) => JiraTestResult {
            ok: true,
            message: format!("Connected to {} at {base}", if info.title.is_empty() { "Jira" } else { &info.title }),
            deployment: Some(if info.cloud { "cloud" } else { "server" }.to_string()),
            version: Some(info.version).filter(|version| !version.is_empty()),
        },
        Err(error) => JiraTestResult { ok: false, message: error.message, deployment: None, version: None },
    };

    Ok(Json(result))
}

fn local_project(connection: &Connection, key: &str) -> ApiResult<Option<JiraLocalProject>> {
    query_one(
        connection,
        "SELECT p.name, (SELECT count(*) FROM issue WHERE project_id = p.id), j.last_import_at
         FROM project p LEFT JOIN jira_project j ON j.project_id = p.id WHERE p.key = ?1 COLLATE NOCASE",
        [key],
        |row| Ok(JiraLocalProject { name: row.get(0)?, issue_count: row.get(1)?, imported_at: row.get(2)? }),
    )
}

async fn remote_projects(State(app): State<App>, user: AuthUser) -> Reply<Vec<JiraRemoteProject>> {
    user.require_admin()?;

    let mut client = client(&app).await?;
    let remote = blocking(move || {
        client.detect()?;

        let projects = client.projects()?;
        let counted = projects.len() <= MAX_COUNTED_PROJECTS;

        Ok(projects
            .into_iter()
            .map(|project| {
                let count = if counted { client.count(&format!("project = \"{}\"", project.key)) } else { None };

                (project, count)
            })
            .collect::<Vec<_>>())
    })
    .await?;

    app.run(move |app| {
        app.read(|connection| {
            remote
                .into_iter()
                .map(|(project, issue_count)| {
                    let local = local_project(connection, &project.key)?;

                    Ok(JiraRemoteProject {
                        key: project.key,
                        name: project.name,
                        issue_count,
                        exists: local.is_some(),
                        imported_at: local.and_then(|local| local.imported_at),
                    })
                })
                .collect()
        })
    })
    .await
    .map(Json)
}

/// The mapping an import of this structure would use, and the project it would write into.
fn describe(connection: &Connection, structure: &Structure, chosen: Option<&JiraMapping>) -> ApiResult<(JiraMapping, Option<JiraLocalProject>)> {
    let users: Vec<(Id, String)> = query_all(connection, "SELECT id, email FROM user", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let stored = import::stored(connection, &structure.key)?.and_then(|stored| stored.mapping);

    Ok((mapping::resolve(structure, &users, stored.as_ref(), chosen), local_project(connection, &structure.key)?))
}

async fn preview(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<JiraPreview> {
    user.require_admin()?;

    let key = remote_key(&key)?;
    let mut client = client(&app).await?;
    let (cloud, structure, issue_count, story_points) = blocking(move || {
        let info = client.detect()?;
        let structure = mapping::fetch(&client, &key)?;
        let issue_count = client.count(&format!("project = \"{}\"", structure.key));
        let story_points = client.fields().is_ok_and(|fields| !fields.points.is_empty());

        Ok((info.cloud, structure, issue_count, story_points))
    })
    .await?;

    app.run(move |app| {
        app.read(|connection| {
            let (mapping, local) = describe(connection, &structure, None)?;

            Ok(JiraPreview {
                project_key: structure.key.clone(),
                project_name: structure.name.clone(),
                issue_count,
                sprints: structure.sprints.len() as i64,
                versions: structure.versions.len() as i64,
                components: structure.components.len() as i64,
                deployment: if cloud { "cloud" } else { "server" }.to_string(),
                agile: structure.agile,
                story_points,
                local,
                mapping,
            })
        })
    })
    .await
    .map(Json)
}

async fn start_import(State(app): State<App>, user: AuthUser, Json(request): Json<JiraImportRequest>) -> Reply<JiraJob> {
    user.require_admin()?;

    let key = remote_key(&request.project_key)?;
    let project_key = key.clone();
    let admin = user.clone();
    let job = app
        .run(move |app| {
            // Fails here, with a message for the form, when nothing is configured yet.
            app.read(|connection| Client::new(&load_settings(connection, &app.config.secret_key)?))?;
            app.write(user.actor(), |tx| jobs::create(tx, "api", &key, user.id))
        })
        .await?;
    let options = Options {
        into_existing: request.into_existing.unwrap_or(false),
        overwrite_local: request.overwrite_local.unwrap_or(false),
        full: request.full.unwrap_or(false),
    };

    import::spawn(&app, Request { job_id: job.id, admin, project_key, options, mapping: request.mapping }, Source::Api)?;

    Ok(Json(job))
}

async fn list_jobs(State(app): State<App>, user: AuthUser) -> Reply<Vec<JiraJob>> {
    user.require_admin()?;

    app.run(|app| app.read(|connection| jobs::list(connection, KEPT_JOBS))).await.map(Json)
}

async fn get_job(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<JiraJob> {
    user.require_admin()?;

    app.run(move |app| app.read(|connection| jobs::get(connection, id))).await.map(Json)
}

async fn cancel_job(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<JiraJob> {
    user.require_admin()?;

    app.run(move |app| {
        app.db.write(|tx| {
            jobs::request_cancel(tx, id)?;
            jobs::get(tx, id)
        })
    })
    .await
    .map(Json)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let escaped = (bytes[index] == b'%').then(|| text.get(index + 1..index + 3).and_then(|digits| u8::from_str_radix(digits, 16).ok())).flatten();

        decoded.push(escaped.unwrap_or(bytes[index]));
        index += if escaped.is_some() { 3 } else { 1 };
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

/// Reads a stored file with the columns and project the request chooses: the preview, and
/// the issues an import of it would write.
fn read_upload(app: &App, upload_id: Id, request: &JiraCsvRequest) -> ApiResult<(JiraCsvPreview, Parsed)> {
    let (filename, content): (String, String) = app
        .read(|connection| query_one(connection, "SELECT filename, content FROM jira_upload WHERE id = ?1", [upload_id], |row| Ok((row.get(0)?, row.get(1)?))))?
        .ok_or_else(|| ApiError::not_found("Upload"))?;
    let table = csv::parse(&content);
    let columns = csv::columns(&table, request.columns.as_deref());

    if !columns.iter().any(|column| column.target == "key") {
        return Err(ApiError::bad_request(
            "The file has no column with issue keys. Export from Jira with “Export CSV (all fields)”, or choose which column holds the keys.",
        ));
    }

    let projects = csv::projects(&table, &columns);
    let project_key = match request.project_key.as_deref().map(str::trim).filter(|key| !key.is_empty()) {
        Some(key) => key.to_uppercase(),
        None => projects.first().map(|(key, _)| key.clone()).ok_or_else(|| ApiError::bad_request("No row of the file has an issue key such as WEB-12"))?,
    };
    let parsed = csv::read(&table, &columns, &project_key, request.project_name.as_deref(), &now_iso());
    let (mapping, local) = app.read(|connection| describe(connection, &parsed.structure, request.mapping.as_ref()))?;
    let preview = JiraCsvPreview {
        upload_id,
        filename,
        rows: table.rows.len() as i64,
        columns,
        targets: csv::TARGETS.iter().map(|target| target.to_string()).collect(),
        projects: projects.into_iter().map(|(key, issues)| JiraCsvProject { key, issues }).collect(),
        project_key,
        project_name: parsed.structure.name.clone(),
        issue_count: parsed.issues.len() as i64,
        sprints: parsed.structure.sprints.len() as i64,
        versions: parsed.structure.versions.len() as i64,
        components: parsed.structure.components.len() as i64,
        local,
        problems: parsed.problems.iter().take(50).cloned().collect(),
        mapping,
    };

    Ok((preview, parsed))
}

/// The file is the request body; its name travels percent-encoded in `x-filename`.
async fn upload_csv(State(app): State<App>, user: AuthUser, headers: HeaderMap, body: Bytes) -> Reply<JiraCsvPreview> {
    user.require_admin()?;

    let filename = headers.get("x-filename").and_then(|value| value.to_str().ok()).map(percent_decode).unwrap_or_default();

    app.run(move |app| {
        let content = String::from_utf8_lossy(&body).into_owned();

        if content.trim().is_empty() {
            return Err(ApiError::bad_request("The file is empty"));
        }

        let filename: String = filename.rsplit(['/', '\\']).next().unwrap_or("").chars().filter(|character| !character.is_control()).take(120).collect();
        let upload_id = app.db.write(|tx| {
            // Files nobody came back to are not kept.
            let stale = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

            tx.execute("DELETE FROM jira_upload WHERE created_at < ?1", [stale])?;
            tx.execute(
                "INSERT INTO jira_upload (filename, content, uploaded_by, created_at) VALUES (?1, ?2, ?3, ?4)",
                (if filename.is_empty() { "export.csv" } else { &filename }, &content, user.id, now_iso()),
            )?;

            Ok(tx.last_insert_rowid())
        })?;

        match read_upload(app, upload_id, &JiraCsvRequest::default()) {
            Ok((preview, _)) => Ok(preview),
            Err(error) => {
                // A file that cannot be read at all is not worth keeping.
                app.db.write(|tx| Ok(tx.execute("DELETE FROM jira_upload WHERE id = ?1", [upload_id])?))?;

                Err(error)
            }
        }
    })
    .await
    .map(Json)
}

async fn preview_csv(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<JiraCsvRequest>) -> Reply<JiraCsvPreview> {
    user.require_admin()?;

    app.run(move |app| read_upload(app, id, &request).map(|(preview, _)| preview)).await.map(Json)
}

async fn import_csv(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<JiraCsvRequest>) -> Reply<JiraJob> {
    user.require_admin()?;

    let admin = user.clone();
    let options = Options { into_existing: request.into_existing.unwrap_or(false), overwrite_local: request.overwrite_local.unwrap_or(false), full: true };
    let (job, parsed, mapping) = app
        .run(move |app| {
            let (preview, parsed) = read_upload(app, id, &request)?;

            if parsed.issues.is_empty() {
                return Err(ApiError::bad_request(format!("The file has no issues of {}", preview.project_key)));
            }

            let job = app.write(user.actor(), |tx| jobs::create(tx, "csv", &preview.project_key, user.id))?;

            Ok((job, parsed, request.mapping))
        })
        .await?;

    import::spawn(&app, Request { job_id: job.id, admin, project_key: job.project_key.clone(), options, mapping }, Source::Csv(Box::new(parsed)))?;

    Ok(Json(job))
}
