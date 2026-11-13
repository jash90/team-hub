//! The files a run's report names: recordings, screenshots, trace pages, downloads. A runner
//! uploads each under the path the report gives it; the bytes go to disk under the data
//! directory, in a folder per run, under a name made here. They are served with `Range`
//! support (a video is seeked, not downloaded whole) and so that nothing in them can run.

use std::path::{Path as FilePath, PathBuf};
use std::time::{Duration, SystemTime};

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::header::{HeaderValue, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use http_body_util::BodyExt;
use hub_app::access::project_access_by_id;
use hub_app::auth::random_token;
use hub_app::db::{query_all, query_one};
use hub_app::{ApiError, ApiResult, App, AuthUser};
use hub_core::Id;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::jobs::heard_for_run;

pub const MAX_ARTIFACT_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_RUN_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_PATH_CHARS: usize = 500;
/// A file on disk that no row names is an upload that never finished, once it is this old.
const ABANDONED_AFTER: Duration = Duration::from_secs(3600);

pub fn routes() -> Router<App> {
    Router::new().route("/runs/{id}/artifacts", put(upload)).route("/artifacts/{id}", get(serve))
}

/// What a file is, by its extension: that decides how it is shown and how it is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Video,
    Screenshot,
    Text,
    Download,
}

impl Kind {
    fn of(path: &str) -> (Kind, &'static str) {
        let name = path.rsplit('/').next().unwrap_or(path);
        let extension = name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).unwrap_or_default();

        match extension.as_str() {
            "webm" => (Kind::Video, "video/webm"),
            "mp4" => (Kind::Video, "video/mp4"),
            "png" => (Kind::Screenshot, "image/png"),
            "jpg" | "jpeg" => (Kind::Screenshot, "image/jpeg"),
            "md" | "txt" | "json" | "log" => (Kind::Text, "text/plain; charset=utf-8"),
            _ => (Kind::Download, "application/octet-stream"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Kind::Video => "video",
            Kind::Screenshot => "screenshot",
            Kind::Text => "text",
            Kind::Download => "download",
        }
    }

    /// Recordings and screenshots are shown in the page; everything else is only ever saved.
    fn inline(self) -> bool {
        matches!(self, Kind::Video | Kind::Screenshot)
    }
}

fn artifacts_dir(app: &App) -> PathBuf {
    app.config.data_dir.join("e2e-artifacts")
}

fn run_dir(app: &App, run_id: Id) -> PathBuf {
    artifacts_dir(app).join(run_id.to_string())
}

fn too_large(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", message)
}

/// The path as the report names it: relative, with `/` between its parts. It is a name to
/// match the report by and never a place on this machine's disk, but one that points out of
/// its folder is refused all the same, so that nothing downstream can be fooled by it.
fn checked_path(path: &str) -> ApiResult<String> {
    let refuse = || ApiError::bad_request("\"path\" is the artifact's path as the report names it: relative, with / between its parts, and no ..");
    let drive = path.as_bytes().get(1) == Some(&b':');

    if path.is_empty() || path.chars().count() > MAX_PATH_CHARS || path.starts_with('/') || drive || path.contains('\\') {
        return Err(refuse());
    }

    if path.chars().any(char::is_control) || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(refuse());
    }

    Ok(path.to_string())
}

/// The run's project, for a caller who may post to it. A run they cannot see does not exist.
fn writable_run(connection: &Connection, user: &AuthUser, run_id: Id) -> ApiResult<Id> {
    let project_id: Id = query_one(connection, "SELECT project_id FROM e2e_run WHERE id = ?1", [run_id], |row| row.get(0))?
        .ok_or_else(|| ApiError::not_found("Run"))?;
    project_access_by_id(connection, user, project_id).map_err(|_| ApiError::not_found("Run"))?.require_edit()?;

    Ok(project_id)
}

#[derive(Deserialize)]
struct Named {
    path: Option<String>,
}

#[derive(Serialize)]
struct Uploaded {
    id: Id,
}

/// Writes the request body to `file`, stopping as soon as it is larger than an artifact may be.
async fn receive(body: Body, file: &FilePath) -> ApiResult<u64> {
    let mut body = body;
    let mut out = tokio::fs::File::create(file).await?;
    let mut size: u64 = 0;

    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| ApiError::bad_request(format!("The upload broke off: {error}")))?;
        let Ok(bytes) = frame.into_data() else { continue };
        size += bytes.len() as u64;

        if size > MAX_ARTIFACT_BYTES {
            return Err(too_large("An artifact can be at most 200 MB"));
        }

        out.write_all(&bytes).await?;
    }

    out.flush().await?;

    Ok(size)
}

/// Records a received file as the run's artifact at `path`, in place of an earlier upload
/// of the same path. Returns the new artifact's id and the file the earlier one leaves behind.
fn record(app: &App, user: &AuthUser, run_id: Id, path: &str, stored: &str, size: u64) -> ApiResult<(Id, Option<PathBuf>)> {
    app.write(user.actor(), |tx| {
        let project_id = writable_run(tx, user, run_id)?;
        let before: Option<(Id, String)> =
            query_one(tx, "SELECT id, stored FROM e2e_artifact WHERE run_id = ?1 AND path = ?2", (run_id, path), |row| Ok((row.get(0)?, row.get(1)?)))?;

        if let Some((id, _)) = &before {
            tx.execute("DELETE FROM e2e_artifact WHERE id = ?1", [id])?;
        }

        let held: i64 = tx.query_row("SELECT coalesce(sum(size), 0) FROM e2e_artifact WHERE run_id = ?1", [run_id], |row| row.get(0))?;

        if held as u64 + size > MAX_RUN_BYTES {
            return Err(too_large("The artifacts of one run can take at most 2 GB together"));
        }

        tx.execute(
            "INSERT INTO e2e_artifact (run_id, path, kind, size, stored, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (run_id, path, Kind::of(path).0.as_str(), size as i64, stored, &tx.now),
        )?;
        let id = tx.last_insert_rowid();

        heard_for_run(tx, run_id)?;
        tx.emit("e2e.artifact", Some(project_id), None, json!({ "runId": run_id, "artifactId": id, "path": path }))?;

        Ok((id, before.map(|(_, stored)| run_dir(app, run_id).join(stored))))
    })
}

/// Takes one file of a run: the raw request body, named by `?path=` as the report names it.
/// The body is written to disk as it arrives, so a recording never sits in memory.
async fn upload(State(app): State<App>, user: AuthUser, Path(run_id): Path<Id>, Query(named): Query<Named>, headers: HeaderMap, body: Body) -> ApiResult<Json<Uploaded>> {
    let path = checked_path(named.path.as_deref().unwrap_or(""))?;
    let declared = headers.get(CONTENT_LENGTH).and_then(|value| value.to_str().ok()).and_then(|value| value.parse::<u64>().ok());

    if declared.is_some_and(|size| size > MAX_ARTIFACT_BYTES) {
        return Err(too_large("An artifact can be at most 200 MB"));
    }

    // Whether the caller may post to the run is settled before a byte is taken; the runner
    // behind the run is heard from now, since a large file takes a while to arrive.
    app.run({
        let user = user.clone();

        move |app| {
            app.write(user.actor(), |tx| {
                writable_run(tx, &user, run_id)?;
                heard_for_run(tx, run_id)
            })
        }
    })
    .await?;

    let directory = run_dir(&app, run_id);
    tokio::fs::create_dir_all(&directory).await?;

    // The name on disk is made here; nothing of the client's path reaches the file system.
    let stored = random_token()?[..24].to_string();
    let file = directory.join(&stored);
    let received = receive(body, &file).await;
    let recorded = match received {
        Ok(size) => app.run(move |app| record(app, &user, run_id, &path, &stored, size)).await,
        Err(error) => Err(error),
    };

    match recorded {
        Ok((id, replaced)) => {
            if let Some(replaced) = replaced {
                let _ = tokio::fs::remove_file(replaced).await;
            }

            Ok(Json(Uploaded { id }))
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&file).await;

            Err(error)
        }
    }
}

struct Stored {
    path: String,
    file: PathBuf,
}

/// An artifact and where its bytes are, for a caller who may see the run's project.
fn readable(app: &App, connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<Stored> {
    let found: Option<(Id, Id, String, String)> = query_one(
        connection,
        "SELECT a.run_id, r.project_id, a.path, a.stored FROM e2e_artifact a JOIN e2e_run r ON r.id = a.run_id WHERE a.id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let (run_id, project_id, path, stored) = found.ok_or_else(|| ApiError::not_found("Artifact"))?;
    project_access_by_id(connection, user, project_id).map_err(|_| ApiError::not_found("Artifact"))?;

    Ok(Stored { file: run_dir(app, run_id).join(stored), path })
}

/// The name a browser saves the file under: the last part of its path, in plain characters.
fn download_name(path: &str) -> String {
    let name: String = path
        .rsplit('/')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|character| character.is_ascii_graphic() && !matches!(character, '"' | '\\' | ';' | '%'))
        .take(120)
        .collect();

    if name.trim_matches('.').is_empty() {
        "file".to_string()
    } else {
        name
    }
}

/// Serves an artifact, whole or the byte range asked for. Recordings and screenshots are
/// shown in place, under a media type decided here from the extension; everything else is a
/// download. Nothing may run script, whatever it holds.
async fn serve(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, request: Request) -> ApiResult<Response> {
    let stored = app.run(move |app| app.read(|connection| readable(app, connection, &user, id))).await?;
    let (kind, media_type) = Kind::of(&stored.path);
    let mime: mime::Mime = media_type.parse().map_err(|_| ApiError::internal("the media type of an artifact does not parse"))?;

    // `ServeFile` answers `Range`, `If-Range` and `If-Modified-Since` and streams from disk.
    let answer = ServeFile::new_with_mime(&stored.file, &mime).oneshot(request).await.map_err(|error| ApiError::internal(format!("artifact: {error}")))?;

    if answer.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found("Artifact"));
    }

    let mut answer = answer.map(Body::new).into_response();
    let disposition = format!("{}; filename=\"{}\"", if kind.inline() { "inline" } else { "attachment" }, download_name(&stored.path));
    let headers = answer.headers_mut();

    headers.insert(CONTENT_TYPE, HeaderValue::from_static(media_type));
    headers.insert(CONTENT_DISPOSITION, HeaderValue::from_str(&disposition).map_err(|_| ApiError::internal("the name of an artifact is not a header value"))?);
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox; default-src 'none'"));
    // The bytes behind an id never change (a new upload of the path is a new artifact), but
    // who may read them can: a browser keeps them for itself, and not for long.
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));

    Ok(answer)
}

/// Removes the files of a run that is gone. The rows went with the run.
pub fn remove_run_files(app: &App, run_id: Id) {
    let directory = run_dir(app, run_id);

    if directory.exists() {
        if let Err(error) = std::fs::remove_dir_all(&directory) {
            tracing::warn!("could not remove the artifacts of run {run_id}: {error}");
        }
    }
}

fn older_than(file: &FilePath, age: Duration) -> bool {
    let modified = std::fs::metadata(file).and_then(|metadata| metadata.modified()).ok();

    modified.and_then(|modified| SystemTime::now().duration_since(modified).ok()).is_some_and(|since| since > age)
}

/// Removes what is on disk and in no row: the folders of runs that are gone (a project
/// deleted with its runs takes no files with it by itself) and uploads that never finished.
pub fn sweep(app: &App) -> ApiResult<()> {
    let Ok(folders) = std::fs::read_dir(artifacts_dir(app)) else { return Ok(()) };

    for folder in folders.flatten() {
        let Some(run_id) = folder.file_name().to_str().and_then(|name| name.parse::<Id>().ok()) else { continue };
        let known: Vec<String> = app.read(|connection| query_all(connection, "SELECT stored FROM e2e_artifact WHERE run_id = ?1", [run_id], |row| row.get(0)))?;
        let run_exists = app.read(|connection| query_one(connection, "SELECT id FROM e2e_run WHERE id = ?1", [run_id], |row| row.get::<_, Id>(0)))?.is_some();

        if !run_exists {
            remove_run_files(app, run_id);
            continue;
        }

        for file in std::fs::read_dir(folder.path())?.flatten() {
            let named = file.file_name().to_str().is_some_and(|name| known.iter().any(|stored| stored == name));

            if !named && older_than(&file.path(), ABANDONED_AFTER) {
                let _ = std::fs::remove_file(file.path());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_relative_and_stays_inside() {
        assert_eq!(checked_path("board-opens-1ee7d924694b8941/attempt-1/video/video.webm").unwrap(), "board-opens-1ee7d924694b8941/attempt-1/video/video.webm");

        for refused in ["", "/etc/passwd", "../secret.key", "a/../../b", "a/./b", "a//b", "a\\b", "C:/x", "a/b/", "a\nb"] {
            assert_eq!(checked_path(refused).unwrap_err().status, StatusCode::BAD_REQUEST, "{refused:?}");
        }
    }

    #[test]
    fn the_extension_decides_the_kind_and_the_media_type() {
        assert_eq!(Kind::of("a/video.WEBM"), (Kind::Video, "video/webm"));
        assert_eq!(Kind::of("a/clip.mp4").0, Kind::Video);
        assert_eq!(Kind::of("a/failure.png"), (Kind::Screenshot, "image/png"));
        assert_eq!(Kind::of("a/trace.md").0, Kind::Text);
        // What could run in a browser is only ever a download.
        assert_eq!(Kind::of("a/page.html"), (Kind::Download, "application/octet-stream"));
        assert_eq!(Kind::of("a/logo.svg").0, Kind::Download);
        assert_eq!(Kind::of("noextension").0, Kind::Download);
    }

    #[test]
    fn a_saved_file_gets_a_plain_name() {
        assert_eq!(download_name("a/b/video.webm"), "video.webm");
        assert_eq!(download_name("a/zrzut \"ekranu\"; x.png"), "zrzutekranux.png");
        assert_eq!(download_name("a/…"), "file");
    }
}
