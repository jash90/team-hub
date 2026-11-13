//! Issues: creating, reading, changing (one or many), ranking, deleting, watching.

use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use hub_app::access::project_access;
use hub_app::services::issues;
use hub_app::{ApiError, App, AuthUser};
use hub_core::{Id, Issue, IssuePatch, IssueRow, NewIssue};
use serde::Deserialize;
use serde_json::{json, Value};

use super::shared::{Reply, OK};

pub fn routes() -> Router<App> {
    Router::new()
        .route("/issues", post(create_issue))
        .route("/issues/bulk", post(bulk_update))
        .route("/issues/{key}", get(get_issue).patch(update_issue).delete(delete_issue))
        .route("/issues/{key}/move", post(move_issue))
        .route("/issues/{key}/watch", put(set_watch))
}

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
                    issues::update_issue(tx, id, &request.patch)
                        .map_err(|error| ApiError::new(error.status, error.code, format!("{key}: {}", error.message)))?;
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
