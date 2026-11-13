//! Files attached to issues.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hub_app::services::issues;
use hub_app::{ApiError, ApiResult, App, AuthUser};
use hub_core::{Attachment, Id};
use serde_json::Value;

use super::shared::{Reply, OK};

const MAX_UPLOAD_BYTES: usize = 26 * 1024 * 1024;

pub fn routes() -> Router<App> {
    Router::new()
        .route("/issues/{key}/attachments", post(upload_attachment).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/attachments/{id}", get(download_attachment).delete(delete_attachment))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let escaped = (bytes[index] == b'%').then(|| text.get(index + 1..index + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok())).flatten();

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
    let name: String =
        attachment.filename.chars().filter(|character| character.is_ascii() && !matches!(character, '"' | '\\' | '\r' | '\n')).collect();
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
