//! The routes under `/api/m/wiki`. Handlers move the request onto the worker pool, check
//! access and call the functions in `spaces`, `pages` and `search`.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use hub_app::access::project_access;
use hub_app::services::issues::issue_access;
use hub_app::{ApiError, ApiResult, App, AuthUser};
use hub_core::Id;
use serde::Deserialize;
use serde_json::Value;

use crate::pages::{self, PageEdit};
use crate::spaces::{self, NewSpace};
use crate::templates::{template_body, templates};
use crate::types::{
    WikiDiff, WikiMove, WikiNewPage, WikiNewSpace, WikiPageDetail, WikiPagePatch, WikiRestore, WikiRevision, WikiRevisionInfo, WikiSearchHit,
    WikiSpace, WikiSpacePatch, WikiTasksRequest, WikiTasksResult, WikiTemplate, WikiTreeNode,
};
use crate::{links, search};

type Reply<T> = ApiResult<Json<T>>;

const OK: Json<Value> = Json(Value::Null);

pub fn router() -> Router<App> {
    Router::new()
        .route("/spaces", get(list_spaces).post(create_space))
        .route("/spaces/{key}", get(get_space).patch(update_space).delete(delete_space))
        .route("/spaces/{key}/tree", get(space_tree))
        .route("/spaces/{key}/pages", post(create_page))
        .route("/pages/{id}", get(get_page).patch(update_page).delete(delete_page))
        .route("/pages/{id}/move", post(move_page))
        .route("/pages/{id}/detach", post(detach_page))
        .route("/pages/{id}/revisions", get(list_revisions))
        .route("/pages/{id}/revisions/{version}", get(get_revision))
        .route("/pages/{id}/diff", get(diff))
        .route("/pages/{id}/restore", post(restore_page))
        .route("/pages/{id}/issues", post(tasks_to_issues))
        .route("/pages/{id}/issues/{issue_key}", put(link_issue).delete(unlink_issue))
        .route("/search", get(run_search))
        .route("/templates", get(list_templates))
}

fn detail(app: &App, user: &AuthUser, id: Id) -> ApiResult<WikiPageDetail> {
    app.read(|connection| pages::page_detail(app, connection, user, id))
}

// ---------- spaces ----------

async fn list_spaces(State(app): State<App>, user: AuthUser) -> Reply<Vec<WikiSpace>> {
    app.run(move |app| app.read(|connection| spaces::list_spaces(connection, &user))).await.map(Json)
}

async fn get_space(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<WikiSpace> {
    app.run(move |app| app.read(|connection| spaces::describe(connection, &spaces::space_access(connection, &user, &key)?))).await.map(Json)
}

async fn create_space(State(app): State<App>, user: AuthUser, Json(request): Json<WikiNewSpace>) -> Reply<WikiSpace> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let project = match request.project_key.as_deref().map(str::trim).filter(|key| !key.is_empty()) {
                Some(key) => {
                    let project = project_access(tx, &user, key)?;
                    project.require_edit()?;

                    Some(project.id)
                }
                None => None,
            };
            let space = spaces::create_space(
                tx,
                &NewSpace {
                    key: &request.key,
                    name: &request.name,
                    description: request.description.as_deref().unwrap_or(""),
                    project_id: project,
                    source: "local",
                    source_ref: None,
                },
            )?;

            spaces::describe(tx, &spaces::access_to(tx, &user, space)?)
        })
    })
    .await
    .map(Json)
}

async fn update_space(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(patch): Json<WikiSpacePatch>) -> Reply<WikiSpace> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let access = spaces::space_access(tx, &user, &key)?;
            spaces::update_space(tx, &user, &access, &patch)?;

            spaces::describe(tx, &spaces::space_access(tx, &user, &key)?)
        })
    })
    .await
    .map(Json)
}

async fn delete_space(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Value> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let access = spaces::space_access(tx, &user, &key)?;
            access.require_admin()?;

            spaces::delete_space(tx, &access.space)
        })
    })
    .await?;

    Ok(OK)
}

async fn space_tree(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<Vec<WikiTreeNode>> {
    app.run(move |app| app.read(|connection| pages::tree(connection, spaces::space_access(connection, &user, &key)?.space.id))).await.map(Json)
}

// ---------- pages ----------

async fn create_page(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(request): Json<WikiNewPage>) -> Reply<WikiPageDetail> {
    app.run(move |app| {
        let body = match (&request.body, request.template_id.as_deref()) {
            (Some(body), _) => body.as_str(),
            (None, Some(template)) => template_body(template).ok_or_else(|| ApiError::bad_request("There is no such template"))?,
            (None, None) => "",
        };
        let id = app.write(user.actor(), |tx| {
            let access = spaces::space_access(tx, &user, &key)?;
            access.require_edit()?;

            pages::create_page(tx, &access.space, &request.title, body, request.parent_id)
        })?;

        detail(app, &user, id)
    })
    .await
    .map(Json)
}

async fn get_page(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<WikiPageDetail> {
    app.run(move |app| detail(app, &user, id)).await.map(Json)
}

async fn update_page(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(patch): Json<WikiPagePatch>) -> Reply<WikiPageDetail> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (page, access) = pages::page_access(tx, &user, id)?;
            access.require_edit()?;

            pages::update_page(
                tx,
                &access.space,
                &page,
                &PageEdit {
                    title: patch.title.as_deref(),
                    body: patch.body.as_deref(),
                    base_version: patch.base_version,
                    message: patch.message.as_deref().unwrap_or(""),
                },
            )
        })?;

        detail(app, &user, id)
    })
    .await
    .map(Json)
}

async fn move_page(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<WikiMove>) -> Reply<WikiPageDetail> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (page, access) = pages::page_access(tx, &user, id)?;
            access.require_edit()?;

            pages::move_page(tx, &access.space, &page, request.parent_id, request.position)
        })?;

        detail(app, &user, id)
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct DeleteQuery {
    #[serde(default)]
    recursive: bool,
}

async fn delete_page(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Query(query): Query<DeleteQuery>) -> Reply<Value> {
    let deleted = app
        .run(move |app| {
            app.write(user.actor(), |tx| {
                let (page, access) = pages::page_access(tx, &user, id)?;
                access.require_edit()?;

                pages::delete_page(tx, &access.space, &page, query.recursive)
            })
        })
        .await?;

    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

async fn detach_page(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<WikiPageDetail> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (page, access) = pages::page_access(tx, &user, id)?;
            access.require_edit()?;

            pages::detach_page(tx, &access.space, &page)
        })?;

        detail(app, &user, id)
    })
    .await
    .map(Json)
}

// ---------- history ----------

async fn list_revisions(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Vec<WikiRevisionInfo>> {
    app.run(move |app| {
        app.read(|connection| {
            pages::page_access(connection, &user, id)?;

            pages::list_revisions(connection, id)
        })
    })
    .await
    .map(Json)
}

async fn get_revision(State(app): State<App>, user: AuthUser, Path((id, version)): Path<(Id, i64)>) -> Reply<WikiRevision> {
    app.run(move |app| {
        app.read(|connection| {
            pages::page_access(connection, &user, id)?;

            pages::load_revision(connection, id, version)
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct DiffQuery {
    from: Option<i64>,
    to: Option<i64>,
}

async fn diff(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Query(query): Query<DiffQuery>) -> Reply<WikiDiff> {
    app.run(move |app| {
        app.read(|connection| {
            let (page, _) = pages::page_access(connection, &user, id)?;

            pages::diff(connection, &page, query.from, query.to)
        })
    })
    .await
    .map(Json)
}

async fn restore_page(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(request): Json<WikiRestore>) -> Reply<WikiPageDetail> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (page, access) = pages::page_access(tx, &user, id)?;
            access.require_edit()?;

            pages::restore_page(tx, &access.space, &page, request.version)
        })?;

        detail(app, &user, id)
    })
    .await
    .map(Json)
}

// ---------- issues ----------

async fn tasks_to_issues(
    State(app): State<App>,
    user: AuthUser,
    Path(id): Path<Id>,
    Json(request): Json<WikiTasksRequest>,
) -> Reply<WikiTasksResult> {
    app.run(move |app| {
        let keys = app.write(user.actor(), |tx| {
            let (page, access) = pages::page_access(tx, &user, id)?;
            access.require_edit()?;
            project_access(tx, &user, &request.project_key)?.require_edit()?;

            if request.base_version.is_some_and(|base| base != page.version) {
                return Err(ApiError::conflict(format!(
                    "Someone else saved version {} of this page; reload it and choose the tasks again",
                    page.version
                )));
            }

            pages::tasks_to_issues(tx, &access.space, &page, &request.project_key, request.type_id, &request.lines)
        })?;

        Ok(WikiTasksResult { keys, page: detail(app, &user, id)? })
    })
    .await
    .map(Json)
}

async fn set_issue_link(app: App, user: AuthUser, id: Id, issue_key: String, linked: bool) -> Reply<WikiPageDetail> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (_, access) = pages::page_access(tx, &user, id)?;
            access.require_edit()?;
            let (issue_id, _) = issue_access(tx, &user, &issue_key)?;

            links::set_documents(tx, id, issue_id, linked)
        })?;

        detail(app, &user, id)
    })
    .await
    .map(Json)
}

async fn link_issue(State(app): State<App>, user: AuthUser, Path((id, issue_key)): Path<(Id, String)>) -> Reply<WikiPageDetail> {
    set_issue_link(app, user, id, issue_key, true).await
}

async fn unlink_issue(State(app): State<App>, user: AuthUser, Path((id, issue_key)): Path<(Id, String)>) -> Reply<WikiPageDetail> {
    set_issue_link(app, user, id, issue_key, false).await
}

// ---------- search and templates ----------

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    space: Option<String>,
}

async fn run_search(State(app): State<App>, user: AuthUser, Query(query): Query<SearchQuery>) -> Reply<Vec<WikiSearchHit>> {
    app.run(move |app| app.read(|connection| search::search_pages(connection, &user, &query.q, query.space.as_deref()))).await.map(Json)
}

async fn list_templates(_user: AuthUser) -> Json<Vec<WikiTemplate>> {
    Json(templates())
}
