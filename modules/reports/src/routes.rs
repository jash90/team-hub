//! The report routes. Each one checks that the caller may see the project, loads its
//! history on the worker pool and hands it to a pure function.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{NaiveDate, Utc};
use hub_app::access::project_access;
use hub_app::services::sprints::load_sprints;
use hub_app::{ApiError, ApiResult, App, AuthUser};
use hub_core::Id;
use rusqlite::Connection;
use serde::Deserialize;

use crate::history::ProjectHistory;
use crate::types::{ReportBurndown, ReportCreatedResolved, ReportCumulativeFlow, ReportCycleTime, ReportUnit, ReportVelocity, ReportWorkload};
use crate::{compute, load};

type Reply<T> = ApiResult<Json<T>>;

const MAX_DAYS: i64 = 366;
const MAX_SPRINTS: i64 = 50;

pub fn router() -> Router<App> {
    Router::new()
        .route("/projects/{key}/burndown", get(burndown))
        .route("/projects/{key}/velocity", get(velocity))
        .route("/projects/{key}/cumulative-flow", get(cumulative_flow))
        .route("/projects/{key}/created-vs-resolved", get(created_vs_resolved))
        .route("/projects/{key}/cycle-time", get(cycle_time))
        .route("/projects/{key}/workload", get(workload))
}

fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// The history of a project the caller may see, with whatever else `read` fetches for it.
fn read_project<T>(app: &App, user: &AuthUser, key: &str, read: impl FnOnce(&Connection, Id) -> ApiResult<T>) -> ApiResult<(ProjectHistory, T)> {
    app.read(|connection| {
        let access = project_access(connection, user, key)?;

        Ok((load::project_history(connection, access.id)?, read(connection, access.id)?))
    })
}

#[derive(Deserialize)]
struct BurndownQuery {
    sprint: Option<Id>,
    unit: Option<String>,
}

async fn burndown(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Query(query): Query<BurndownQuery>) -> Reply<ReportBurndown> {
    let unit = match query.unit.as_deref() {
        Some(text) => Some(ReportUnit::parse(text).ok_or_else(|| ApiError::bad_request("The unit is \"points\" or \"issues\""))?),
        None => None,
    };

    app.run(move |app| {
        let (history, sprints) = read_project(app, &user, &key, load_sprints)?;
        let sprint = match query.sprint {
            Some(id) => Some(sprints.iter().find(|sprint| sprint.id == id).ok_or_else(|| ApiError::not_found("Sprint"))?),
            None => compute::default_sprint(&sprints),
        };

        Ok(match sprint {
            Some(sprint) => compute::burndown(&history, sprint, unit, today()),
            None => ReportBurndown { sprint: None, unit: unit.unwrap_or(ReportUnit::Points), committed: 0.0, days: Vec::new() },
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct VelocityQuery {
    sprints: Option<i64>,
}

async fn velocity(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Query(query): Query<VelocityQuery>) -> Reply<ReportVelocity> {
    let count = query.sprints.unwrap_or(6).clamp(1, MAX_SPRINTS) as usize;

    app.run(move |app| {
        let (history, sprints) = read_project(app, &user, &key, load_sprints)?;

        Ok(compute::velocity(&history, &compute::recent_closed(&sprints, count)))
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct DaysQuery {
    days: Option<i64>,
}

impl DaysQuery {
    fn or(&self, default: i64) -> i64 {
        self.days.unwrap_or(default).clamp(1, MAX_DAYS)
    }
}

async fn cumulative_flow(
    State(app): State<App>,
    user: AuthUser,
    Path(key): Path<String>,
    Query(query): Query<DaysQuery>,
) -> Reply<ReportCumulativeFlow> {
    app.run(move |app| {
        let (history, ()) = read_project(app, &user, &key, |_, _| Ok(()))?;

        Ok(compute::cumulative_flow(&history, query.or(30), today()))
    })
    .await
    .map(Json)
}

async fn created_vs_resolved(
    State(app): State<App>,
    user: AuthUser,
    Path(key): Path<String>,
    Query(query): Query<DaysQuery>,
) -> Reply<ReportCreatedResolved> {
    app.run(move |app| {
        let (history, ()) = read_project(app, &user, &key, |_, _| Ok(()))?;

        Ok(compute::created_vs_resolved(&history, query.or(30), today()))
    })
    .await
    .map(Json)
}

async fn cycle_time(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Query(query): Query<DaysQuery>) -> Reply<ReportCycleTime> {
    app.run(move |app| {
        let (history, ()) = read_project(app, &user, &key, |_, _| Ok(()))?;

        Ok(compute::cycle_time(&history, query.or(60), today()))
    })
    .await
    .map(Json)
}

async fn workload(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<ReportWorkload> {
    app.run(move |app| {
        let (history, names) = read_project(app, &user, &key, load::assignee_names)?;

        Ok(compute::workload(&history, &names))
    })
    .await
    .map(Json)
}
