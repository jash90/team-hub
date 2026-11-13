use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::HeaderMap;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use hub_app::access::{project_access, project_access_by_id, visible_projects};
use hub_app::db::{query_all, query_one};
use hub_app::services::issues::issue_access;
use hub_app::{ApiError, ApiResult, App, AuthUser};
use hub_core::{Id, IssueRow};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;

use crate::ingest::{delete_run, delete_test, project_keys, set_manual_link, store_run, RunMeta};
use crate::model::{count_tests, project_runs, project_tests, run_by_id, run_results, state_of, tests_of_issues};
use crate::types::{E2eDashboard, E2eDashboardIssue, E2eIngestResult, E2eIssuePanel, E2eProjectTests, E2eRunDetail, E2eSession};
use crate::{artifacts, jobs, report, session, settings};

type Reply<T> = ApiResult<Json<T>>;

/// A report with every step of a long suite is large; screenshots and videos are not in it.
const MAX_REPORT_BYTES: usize = 64 * 1024 * 1024;
const RUNS_SHOWN: i64 = 40;

pub fn routes() -> Router<App> {
    Router::new()
        .route("/projects/{key}", get(project_page))
        .route("/projects/{key}/runs", post(post_run).layer(DefaultBodyLimit::max(MAX_REPORT_BYTES)))
        .route("/runs/{id}", get(run_detail).delete(remove_run))
        .route("/results/{id}", get(result_session))
        .route("/tests/{id}", delete(remove_test))
        .route("/tests/{id}/issues/{issue_key}", put(link_issue).delete(unlink_issue))
        .merge(settings::routes())
        .merge(jobs::routes())
        .merge(artifacts::routes())
}

fn base_url(app: &App) -> &str {
    app.config.base_url.trim_end_matches('/')
}

pub fn ingest_url(app: &App, project_key: &str) -> String {
    format!("{}/api/m/e2e/projects/{project_key}/runs", base_url(app))
}

fn run_url(app: &App, project_key: &str, run_id: Id) -> String {
    format!("{}/p/{project_key}/tests?run={run_id}", base_url(app))
}

fn load_project(app: &App, user: &AuthUser, key: &str) -> ApiResult<E2eProjectTests> {
    let access = app.read(|connection| project_access(connection, user, key))?;
    // Two independent reads, each on its own connection.
    let (runs, tests) = rayon::join(
        || app.read(|connection| project_runs(connection, access.id, RUNS_SHOWN)),
        || app.read(|connection| project_tests(connection, user, access.id)),
    );
    let (runs, tests) = (runs?, tests?);
    let bug_type_id = app.read(|connection| {
        query_one(connection, "SELECT id FROM issue_type WHERE project_id = ?1 AND name = 'Bug' AND level = 'standard'", [access.id], |row| {
            row.get(0)
        })
    })?;

    Ok(E2eProjectTests {
        latest_run: runs.first().cloned(),
        counts: count_tests(&tests),
        runs,
        tests,
        can_edit: access.can_edit(),
        can_admin: access.require_admin().is_ok(),
        ingest_url: ingest_url(app, &access.key),
        base_url: base_url(app).to_string(),
        bug_type_id,
        project_key: access.key,
    })
}

async fn project_page(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<E2eProjectTests> {
    app.run(move |app| load_project(app, &user, &key)).await.map(Json)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct WithMeta {
    meta: Option<Value>,
}

/// The job a posted report belongs to, when a runner posts it: `?job=<id>`.
#[derive(Deserialize)]
struct OfJob {
    job: Option<Id>,
}

/// Takes a run report: the framework's JSON document as it is (optionally with a `meta`
/// object beside `run`), or JUnit XML. Branch, commit, pull request, CI address, run name
/// and, for a report without one, a run id can also come as query parameters. A runner
/// names its job with `job`, and the stored run is tied to it.
async fn post_run(
    State(app): State<App>,
    user: AuthUser,
    Path(key): Path<String>,
    Query(query): Query<RunMeta>,
    Query(of_job): Query<OfJob>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply<E2eIngestResult> {
    let content_type = headers.get(CONTENT_TYPE).and_then(|value| value.to_str().ok()).map(String::from);

    app.run(move |app| {
        // Whether the caller may post is settled before the report is read.
        let projects = app.read(|connection| {
            project_access(connection, &user, &key)?.require_edit()?;
            project_keys(connection)
        })?;
        let mut run = report::parse(&body, content_type.as_deref())?;
        run.find_issue_keys(&projects);

        let in_body = match run.format {
            "json" => serde_json::from_slice::<WithMeta>(&body).ok().and_then(|document| document.meta),
            _ => None,
        };
        let in_body: RunMeta = match in_body {
            Some(meta) => serde_json::from_value(meta).map_err(|error| ApiError::bad_request(format!("\"meta\" is not understood: {error}")))?,
            None => RunMeta::default(),
        };
        let stored = app.write(user.actor(), |tx| {
            let access = project_access(tx, &user, &key)?;
            access.require_edit()?;

            if let Some(job_id) = of_job.job {
                jobs::check_report_job(tx, &user, job_id, access.id)?;
            }

            let stored = store_run(tx, Some(&user), access.id, &run, query.or(in_body))?;

            if let Some(job_id) = of_job.job {
                jobs::tie_run(tx, job_id, stored.run_id)?;
            }

            Ok(stored)
        })?;

        app.read(|connection| {
            let (run, _) = run_by_id(connection, stored.run_id)?.ok_or_else(|| ApiError::not_found("Run"))?;
            let linked_issues = match stored.issues.is_empty() {
                true => Vec::new(),
                false => {
                    let ids = stored.issues.iter().map(Id::to_string).collect::<Vec<_>>().join(", ");

                    query_all(connection, &format!("SELECT key FROM issue WHERE id IN ({ids}) ORDER BY project_id, number"), [], |row| row.get(0))?
                }
            };

            Ok(E2eIngestResult { url: run_url(app, &run.project_key, run.id), run, created: stored.created, linked_issues })
        })
    })
    .await
    .map(Json)
}

async fn run_detail(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<E2eRunDetail> {
    app.run(move |app| {
        app.read(|connection| {
            let (run, project_id) = run_by_id(connection, id)?.ok_or_else(|| ApiError::not_found("Run"))?;
            project_access_by_id(connection, &user, project_id).map_err(|_| ApiError::not_found("Run"))?;

            Ok(E2eRunDetail { results: run_results(connection, &user, id)?, job_id: jobs::job_of_run(connection, id)?, run })
        })
    })
    .await
    .map(Json)
}

/// One result of a run with what its report said happened: the session view's data.
async fn result_session(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<E2eSession> {
    app.run(move |app| app.read(|connection| session::load(connection, &user, id))).await.map(Json)
}

async fn remove_run(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (_, project_id) = run_by_id(tx, id)?.ok_or_else(|| ApiError::not_found("Run"))?;
            project_access_by_id(tx, &user, project_id).map_err(|_| ApiError::not_found("Run"))?.require_admin()?;

            delete_run(tx, id, project_id)
        })?;
        // The rows went with the run; its recordings and screenshots are files.
        artifacts::remove_run_files(app, id);

        Ok(())
    })
    .await?;

    Ok(Json(Value::Null))
}

/// The project a test belongs to, for a caller who may edit it.
fn editable_test(connection: &Connection, user: &AuthUser, test_id: Id) -> ApiResult<Id> {
    let project_id: Id = query_one(connection, "SELECT project_id FROM e2e_test WHERE id = ?1", [test_id], |row| row.get(0))?
        .ok_or_else(|| ApiError::not_found("Test"))?;
    project_access_by_id(connection, user, project_id).map_err(|_| ApiError::not_found("Test"))?.require_edit()?;

    Ok(project_id)
}

async fn remove_test(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let project_id = editable_test(tx, &user, id)?;
            delete_test(tx, id)?;

            tx.emit("e2e.run", Some(project_id), None, serde_json::json!({ "testId": id, "deleted": true }))
        })
    })
    .await?;

    Ok(Json(Value::Null))
}

async fn set_link(app: App, user: AuthUser, test_id: Id, issue_key: String, linked: bool) -> Reply<Value> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            editable_test(tx, &user, test_id)?;
            let (issue_id, _) = issue_access(tx, &user, &issue_key)?;

            set_manual_link(tx, test_id, issue_id, linked)
        })
    })
    .await?;

    Ok(Json(Value::Null))
}

/// Links a test to an issue by hand; the link survives later runs.
async fn link_issue(State(app): State<App>, user: AuthUser, Path((id, issue_key)): Path<(Id, String)>) -> Reply<Value> {
    set_link(app, user, id, issue_key, true).await
}

/// Removes a link, whether a run or a person made it; later runs do not bring it back.
async fn unlink_issue(State(app): State<App>, user: AuthUser, Path((id, issue_key)): Path<(Id, String)>) -> Reply<Value> {
    set_link(app, user, id, issue_key, false).await
}

/// The issue's tests. None when it has none and its project has never received a run, so the
/// panel only shows up where tests are in use.
pub fn issue_panel(connection: &Connection, issue: &IssueRow, viewer: &AuthUser) -> ApiResult<Option<E2eIssuePanel>> {
    let tests: Vec<_> =
        tests_of_issues(connection, viewer, "i.id = ?", vec![SqlValue::Integer(issue.id)])?.into_iter().map(|(_, test)| test).collect();

    if tests.is_empty() {
        let in_use: Option<Id> = query_one(connection, "SELECT id FROM e2e_run WHERE project_id = ?1 LIMIT 1", [issue.project_id], |row| row.get(0))?;

        if in_use.is_none() {
            return Ok(None);
        }
    }

    Ok(Some(E2eIssuePanel { state: state_of(&tests), tests }))
}

/// The viewer's issues that have a failing test, finished ones first: an issue that is done
/// while its test fails is the one to look at.
pub fn dashboard(connection: &Connection, viewer: &AuthUser) -> ApiResult<Option<E2eDashboard>> {
    let (visible, mut params) = visible_projects(viewer, "i.project_id");
    params.insert(0, SqlValue::Integer(viewer.id));

    // Projects that have this module switched off stay out of the section.
    let on = hub_app::services::project_modules::on_in_project("e2e", "i.project_id");
    let failing = tests_of_issues(connection, viewer, &format!("i.assignee_id = ? AND t.last_status = 'failed' AND {visible} AND {on}"), params)?;

    if failing.is_empty() {
        return Ok(None);
    }

    let ids = failing.iter().map(|(issue_id, _)| issue_id.to_string()).collect::<Vec<_>>().join(", ");
    let mut issues = query_all(
        connection,
        &format!(
            "SELECT i.id, i.key, i.summary, s.name, s.category = 'done' FROM issue i JOIN status s ON s.id = i.status_id
             WHERE i.id IN ({ids}) ORDER BY s.category = 'done' DESC, i.updated_at DESC LIMIT 20"
        ),
        [],
        |row| {
            Ok((
                row.get::<_, Id>(0)?,
                E2eDashboardIssue { key: row.get(1)?, summary: row.get(2)?, status: row.get(3)?, done: row.get(4)?, tests: Vec::new() },
            ))
        },
    )?;

    for (issue_id, test) in failing {
        if let Some((_, issue)) = issues.iter_mut().find(|(id, _)| *id == issue_id) {
            issue.tests.push(test);
        }
    }

    Ok(Some(E2eDashboard { issues: issues.into_iter().map(|(_, issue)| issue).collect() }))
}
