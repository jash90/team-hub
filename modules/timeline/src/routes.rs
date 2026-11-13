//! Routes under `/api/m/timeline`. Handlers are thin: the work runs on the worker pool.

use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use hub_app::access::project_access;
use hub_app::services::issues::issue_access;
use hub_app::services::sprints::load_sprints;
use hub_app::services::versions::load_versions;
use hub_app::{ApiResult, App, AuthUser};
use hub_core::{Sprint, Version};
use serde::{Deserialize, Deserializer};

use crate::model::{assemble, children, parents, row, Scope};
use crate::span::set_span;
use crate::types::{TimelineEpic, TimelineProject, TimelineSprint, TimelineVersion};

type Reply<T> = ApiResult<Json<T>>;

pub fn routes() -> Router<App> {
    Router::new().route("/projects/{key}", get(project_timeline)).route("/issues/{key}/span", put(put_span))
}

fn day(instant: Option<String>) -> Option<String> {
    instant.map(|instant| instant.chars().take(10).collect())
}

fn sprint(sprint: Sprint) -> TimelineSprint {
    TimelineSprint {
        id: sprint.id,
        name: sprint.name,
        state: sprint.state,
        start: day(sprint.start_date),
        end: day(sprint.end_date.or(sprint.completed_at)),
    }
}

fn version(version: Version) -> TimelineVersion {
    TimelineVersion {
        id: version.id,
        name: version.name,
        released: version.released,
        release_date: day(version.release_date.or(version.released_at)),
    }
}

/// The timeline of a project the caller may see.
pub fn load_project(app: &App, user: &AuthUser, key: &str) -> ApiResult<TimelineProject> {
    let access = app.read(|connection| project_access(connection, user, key))?;
    let scope = Scope::Epics { project_id: access.id };
    // Four independent reads, each on its own connection.
    let ((epics, under), (sprints, versions)) = rayon::join(
        || rayon::join(|| app.read(|connection| parents(connection, scope)), || app.read(|connection| children(connection, scope))),
        || rayon::join(|| app.read(|connection| load_sprints(connection, access.id)), || app.read(|connection| load_versions(connection, access.id))),
    );

    Ok(TimelineProject {
        epics: assemble(epics?, under?),
        sprints: sprints?.into_iter().map(sprint).collect(),
        versions: versions?.into_iter().filter(|version| !version.archived).map(version).collect(),
        can_edit: access.can_edit(),
        project_key: access.key,
    })
}

async fn project_timeline(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<TimelineProject> {
    app.run(move |app| load_project(app, &user, &key)).await.map(Json)
}

/// A field that has to be in the body, where null is a value: a span is set as a whole, and
/// a date that was merely left out must not be taken for one to clear.
fn stated<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Deserialize)]
struct SpanInput {
    #[serde(deserialize_with = "stated")]
    start: Option<String>,
    #[serde(deserialize_with = "stated")]
    due: Option<String>,
}

/// Sets when an issue's work runs and answers with its row of the timeline.
async fn put_span(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<SpanInput>) -> Reply<TimelineEpic> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (issue_id, access) = issue_access(tx, &user, &key)?;
            access.require_edit()?;
            set_span(tx, issue_id, input.start.as_deref(), input.due.as_deref())?;

            row(tx, issue_id)
        })
    })
    .await
    .map(Json)
}
