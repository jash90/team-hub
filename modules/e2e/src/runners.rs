//! The runners that take test jobs (`docs/runners.md`). A runner says it is there by asking
//! for a job; what it said then is kept, so that a job can be sent to one runner by name and
//! the list of runners can show who is connected and what they are carrying out.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hub_app::access::{visible_projects, ProjectAccess};
use hub_app::db::{query_all, query_one};
use hub_app::{ApiError, ApiResult, AuthUser, Tx};
use hub_core::runners::{Runner, RunnerActive, RunnerWork};
use hub_core::Id;
use rusqlite::{Connection, Row};

/// A runner is connected while it asked for work this recently.
const ONLINE_MINUTES: i64 = 2;

/// What a runner says about itself each time it asks for a job.
#[derive(Debug, Clone, PartialEq)]
pub struct Presence {
    pub id: String,
    pub name: String,
    /// Project keys it works for; none: every project its person can edit.
    pub projects: Option<Vec<String>>,
    /// Somebody's computer with the person at it, or a service.
    pub interactive: bool,
    /// It takes only jobs sent to it by name.
    pub pinned_only: bool,
    /// The team's runner: it works on anyone's jobs with that person's own keys.
    pub shared: bool,
    pub version: String,
}

struct Known {
    id: String,
    user_id: Id,
    name: String,
    projects: Option<Vec<String>>,
    interactive: bool,
    pinned_only: bool,
    shared: bool,
    version: String,
    last_seen_at: String,
}

const COLUMNS: &str = "id, user_id, name, projects, interactive, pinned_only, version, last_seen_at, shared";

fn map_known(row: &Row) -> rusqlite::Result<Known> {
    let projects: Option<String> = row.get(3)?;

    Ok(Known {
        id: row.get(0)?,
        user_id: row.get(1)?,
        name: row.get(2)?,
        projects: projects.and_then(|text| serde_json::from_str(&text).ok()),
        interactive: row.get(4)?,
        pinned_only: row.get(5)?,
        version: row.get(6)?,
        last_seen_at: row.get(7)?,
        shared: row.get(8)?,
    })
}

fn by_id(connection: &Connection, id: &str) -> ApiResult<Option<Known>> {
    query_one(connection, &format!("SELECT {COLUMNS} FROM e2e_runner WHERE id = ?1"), [id], map_known)
}

fn works_for(projects: Option<&[String]>, key: &str) -> bool {
    projects.is_none_or(|projects| projects.iter().any(|project| project.eq_ignore_ascii_case(key)))
}

/// Notes that a runner is there, with what it now says about itself. An id belongs to the
/// person whose token first used it. Only a service an instance administrator runs can be
/// the team's shared runner: it is handed other people's keys.
pub fn seen(tx: &Tx, user: &AuthUser, presence: &Presence) -> ApiResult<()> {
    if presence.shared && (presence.interactive || !user.is_admin()) {
        return Err(ApiError::forbidden("Only an administrator's runner can be shared"));
    }

    if by_id(tx, &presence.id)?.is_some_and(|known| known.user_id != user.id) {
        return Err(ApiError::conflict("That runner id belongs to somebody else's runner"));
    }

    tx.execute(
        "INSERT INTO e2e_runner (id, user_id, name, projects, interactive, pinned_only, version, last_seen_at, shared) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (id) DO UPDATE SET name = ?3, projects = ?4, interactive = ?5, pinned_only = ?6, version = ?7, last_seen_at = ?8, shared = ?9",
        rusqlite::params![
            presence.id,
            user.id,
            presence.name,
            presence.projects.as_ref().map(serde_json::to_string).transpose()?,
            presence.interactive,
            presence.pinned_only,
            presence.version,
            &tx.now,
            presence.shared,
        ],
    )?;

    Ok(())
}

/// The name of the runner a job of this project may be sent to by `user`. A service takes
/// work from anyone who may start it; somebody's computer only from its owner.
pub fn pinnable(connection: &Connection, user: &AuthUser, project: &ProjectAccess, runner_id: &str) -> ApiResult<String> {
    let runner = by_id(connection, runner_id)?.ok_or_else(|| ApiError::bad_request("That runner has not connected to this server"))?;

    if runner.interactive && runner.user_id != user.id {
        return Err(ApiError::forbidden("That runner is somebody's computer; only its owner sends work to it by name"));
    }

    if !works_for(runner.projects.as_deref(), &project.key) {
        return Err(ApiError::bad_request("That runner does not work for this project"));
    }

    Ok(runner.name)
}

fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The runners as the list every module contributes to shows them: the viewer's own, and
/// other people's while they are connected, each with the jobs it holds in projects the
/// viewer may see.
pub fn known(connection: &Connection, viewer: &AuthUser, now: DateTime<Utc>) -> ApiResult<Vec<Runner>> {
    let online_since = iso(now - Duration::minutes(ONLINE_MINUTES));
    let (visible, params) = visible_projects(viewer, "j.project_id");
    let held: Vec<(String, RunnerActive)> = query_all(
        connection,
        &format!(
            "SELECT j.runner_id, p.key, j.git_ref, j.id FROM e2e_job j JOIN project p ON p.id = j.project_id
             WHERE j.state IN ('claimed', 'running') AND j.runner_id IS NOT NULL AND {visible} ORDER BY j.id"
        ),
        rusqlite::params_from_iter(params),
        |row| {
            let (key, git_ref, id): (String, String, Id) = (row.get(1)?, row.get(2)?, row.get(3)?);

            Ok((row.get(0)?, RunnerActive { title: format!("{key} · tests · {git_ref}"), href: crate::jobs::job_path(&key, id) }))
        },
    )?;
    let runners = query_all(
        connection,
        &format!("SELECT {COLUMNS} FROM e2e_runner WHERE user_id = ?1 OR last_seen_at >= ?2 ORDER BY name, id"),
        (viewer.id, &online_since),
        map_known,
    )?;

    Ok(runners
        .into_iter()
        .map(|runner| {
            let online = runner.last_seen_at >= online_since;

            Runner {
                work: vec![RunnerWork {
                    module: "e2e".to_string(),
                    what: vec!["tests".to_string()],
                    projects: runner.projects,
                    pinned_only: runner.pinned_only,
                    online,
                    last_seen_at: runner.last_seen_at.clone(),
                    active: held.iter().filter(|(runner_id, _)| *runner_id == runner.id).map(|(_, active)| active.clone()).collect(),
                }],
                id: runner.id,
                name: runner.name,
                user_id: runner.user_id,
                interactive: runner.interactive,
                shared: runner.shared,
                online,
                last_seen_at: runner.last_seen_at,
                version: Some(runner.version).filter(|version| !version.is_empty()),
            }
        })
        .collect())
}
