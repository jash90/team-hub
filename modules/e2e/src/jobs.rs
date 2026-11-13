//! Test jobs: somebody asks for a project's suite to be run, a runner claims the job, says
//! what it is doing and sends its output, and ends it with the report or with a reason.
//! `docs/e2e-jobs.md` is the contract runners are written against.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, SecondsFormat, Utc};
use hub_app::access::{project_access, project_access_by_id};
use hub_app::db::{query_all, query_one};
use hub_app::services::personal_keys;
use hub_app::{Actor, ApiError, ApiResult, App, AuthUser, Tx};
use hub_core::Id;
use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::runners::{self, Presence};
use crate::settings;
use crate::types::{E2eJob, E2eJobDetail, E2eJobState, E2eLogLine, E2eSettings};

/// Lines of a job's log that are kept; older ones go as new ones arrive.
pub const LOG_LINES_KEPT: i64 = 2000;
const MAX_LINE_CHARS: usize = 2000;
/// Open pages hear of new log lines at most this often.
const LOG_ANNOUNCE_EVERY: Duration = Duration::from_secs(1);
/// A claimed or running job whose runner has been quiet this long has lost its runner.
const RUNNER_QUIET_MINUTES: i64 = 10;
const RUNNER_GONE: &str = "the runner stopped answering";
const NO_REPORT: &str = "the runner finished without sending a report";
const RUNNER_FAILED: &str = "the runner could not run the suite";
const JOBS_SHOWN: i64 = 20;
const MAX_JOBS_SHOWN: i64 = 100;

type Reply<T> = ApiResult<Json<T>>;

pub fn routes() -> Router<App> {
    Router::new()
        .route("/projects/{key}/jobs", get(list_jobs).post(request_job))
        .route("/jobs/claim", post(claim_job))
        .route("/jobs/{id}", get(job_detail).patch(patch_job))
        .route("/jobs/{id}/cancel", post(cancel_job))
        .route("/jobs/{id}/log", post(append_log))
        .route("/jobs/{id}/finish", post(finish_job))
}

/// What somebody asks for when they queue a job; what is left out comes from the project's settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewJob {
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    /// Test files or a `--grep` pattern, as typed.
    pub filter: Option<String>,
    pub video: Option<String>,
    pub trace: Option<String>,
    /// The scheduled task this job runs for, when the schedule started it.
    pub task_id: Option<Id>,
    /// The id of the runner the job is sent to; any runner that fits when left out.
    pub on_runner: Option<String>,
}

const JOB_COLUMNS: &str = "j.id, p.key, j.state, j.repo, j.git_ref, j.commit_sha, j.filter, j.video, j.trace, j.requested_by, j.runner_name, j.run_id,
    j.error, j.cancel_requested, j.step, j.created_at, j.started_at, j.finished_at, j.on_runner, j.on_runner_name";

fn map_job(row: &Row) -> rusqlite::Result<E2eJob> {
    let state: String = row.get(2)?;

    Ok(E2eJob {
        id: row.get(0)?,
        project_key: row.get(1)?,
        state: E2eJobState::parse(&state).unwrap_or(E2eJobState::Failed),
        repo: row.get(3)?,
        git_ref: row.get(4)?,
        commit: row.get(5)?,
        filter: row.get(6)?,
        video: row.get(7)?,
        trace: row.get(8)?,
        requested_by: row.get(9)?,
        on_runner: row.get(18)?,
        on_runner_name: row.get(19)?,
        runner_name: row.get(10)?,
        run_id: row.get(11)?,
        error: row.get(12)?,
        cancel_requested: row.get(13)?,
        step: row.get(14)?,
        created_at: row.get(15)?,
        started_at: row.get(16)?,
        finished_at: row.get(17)?,
    })
}

/// A job with what the routes need to decide about it: its project, and whose token claimed it.
struct Held {
    job: E2eJob,
    project_id: Id,
    runner_user_id: Option<Id>,
}

fn held(connection: &Connection, id: Id) -> ApiResult<Held> {
    query_one(
        connection,
        &format!("SELECT {JOB_COLUMNS}, j.project_id, j.runner_user_id FROM e2e_job j JOIN project p ON p.id = j.project_id WHERE j.id = ?1"),
        [id],
        |row| Ok(Held { job: map_job(row)?, project_id: row.get(20)?, runner_user_id: row.get(21)? }),
    )?
    .ok_or_else(|| ApiError::not_found("Job"))
}

fn job_by_id(connection: &Connection, id: Id) -> ApiResult<E2eJob> {
    held(connection, id).map(|held| held.job)
}

fn announce(tx: &Tx, project_id: Id, job_id: Id, state: E2eJobState) -> ApiResult<()> {
    tx.emit("e2e.job", Some(project_id), None, json!({ "jobId": job_id, "state": state.as_str() }))
}

fn tidy(text: Option<&str>, max_chars: usize) -> Option<String> {
    text.map(str::trim).filter(|text| !text.is_empty()).map(|text| text.chars().take(max_chars).collect())
}

/// The page that shows a job.
pub fn job_path(project_key: &str, id: Id) -> String {
    format!("/p/{project_key}/tests/jobs/{id}")
}

/// Queues a job for the project on behalf of `user`, who must be able to edit it. Also the
/// way in for scheduled tasks: `new.task_id` names the task, and the job remembers it. With
/// `new.on_runner` the job waits for that runner and is offered to no other.
pub fn create(tx: &Tx, user: &AuthUser, project_key: &str, new: &NewJob) -> ApiResult<E2eJob> {
    let access = project_access(tx, user, project_key)?;
    access.require_edit()?;

    let settings = settings::load(tx, access.id)?;
    let repo = settings
        .repo
        .ok_or_else(|| ApiError::conflict("This project has no repository to test: a project administrator sets it in Project settings → Tests"))?;
    let git_ref = match tidy(new.git_ref.as_deref(), 200) {
        Some(git_ref) => settings::checked_ref(&git_ref)?,
        None => settings.git_ref,
    };
    let filter = tidy(new.filter.as_deref(), 500);

    if filter.as_deref().is_some_and(|filter| filter.chars().any(char::is_control)) {
        return Err(ApiError::bad_request("The filter must be one line: test files, or a --grep pattern"));
    }

    let video = new.video.as_deref().map(settings::checked_video).transpose()?.unwrap_or(settings.video);
    let trace = new.trace.as_deref().map(settings::checked_trace).transpose()?.unwrap_or(settings.trace);

    let on_runner = tidy(new.on_runner.as_deref(), 200);
    let on_runner_name = on_runner.as_deref().map(|runner_id| runners::pinnable(tx, user, &access, runner_id)).transpose()?;

    tx.execute(
        "INSERT INTO e2e_job (project_id, state, repo, git_ref, filter, video, trace, requested_by, task_id, on_runner, on_runner_name, created_at)
         VALUES (?1, 'queued', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![access.id, repo, git_ref, filter, video.as_str(), trace.as_str(), user.id, new.task_id, on_runner, on_runner_name, &tx.now],
    )?;
    let id = tx.last_insert_rowid();
    announce(tx, access.id, id, E2eJobState::Queued)?;

    job_by_id(tx, id)
}

/// Ends a job: every way a job stops being queued, claimed or running comes through here.
fn end(tx: &Tx, held: &Held, state: E2eJobState, error: Option<&str>) -> ApiResult<()> {
    tx.execute(
        "UPDATE e2e_job SET state = ?1, error = coalesce(?2, error), finished_at = ?3, step = NULL WHERE id = ?4",
        (state.as_str(), error, &tx.now, held.job.id),
    )?;

    // A job started by a scheduled task (`task_id`) reports back here once the schedule
    // service exists: `hub_app::services::schedule::finish(tx, task_id, outcome)` with the
    // run's summary and link when `state` is succeeded, and with `error` otherwise.

    announce(tx, held.project_id, held.job.id, state)
}

/// A project's jobs, newest first.
pub fn project_jobs(connection: &Connection, project_id: Id, limit: i64) -> ApiResult<Vec<E2eJob>> {
    query_all(
        connection,
        &format!("SELECT {JOB_COLUMNS} FROM e2e_job j JOIN project p ON p.id = j.project_id WHERE j.project_id = ?1 ORDER BY j.id DESC LIMIT ?2"),
        (project_id, limit),
        map_job,
    )
}

fn minutes_ago(minutes: i64) -> String {
    (Utc::now() - chrono::Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Fails the claimed and running jobs whose runner has gone quiet. Cheap when there are none,
/// so reads call it before they answer, and the module's background thread calls it for the
/// jobs nobody is looking at.
pub fn fail_stale(app: &App) -> ApiResult<()> {
    let quiet_since = minutes_ago(RUNNER_QUIET_MINUTES);
    let stale = |connection: &Connection| {
        query_all(
            connection,
            "SELECT id FROM e2e_job WHERE state IN ('claimed', 'running') AND coalesce(heard_at, started_at, created_at) < ?1",
            [&quiet_since],
            |row| row.get::<_, Id>(0),
        )
    };

    if app.read(stale)?.is_empty() {
        return Ok(());
    }

    app.write(Actor::system("Tests"), |tx| {
        for id in stale(tx)? {
            end(tx, &held(tx, id)?, E2eJobState::Failed, Some(RUNNER_GONE))?;
        }

        Ok(())
    })
}

/// The job for a caller who may see its project; a job of a project they cannot see does not exist.
fn visible(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(Held, hub_app::access::ProjectAccess)> {
    let held = held(connection, id)?;
    let access = project_access_by_id(connection, user, held.project_id).map_err(|_| ApiError::not_found("Job"))?;

    Ok((held, access))
}

/// The job for the runner that claimed it, while it can still be written to.
fn for_its_runner(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<Held> {
    let (held, access) = visible(tx, user, id)?;
    access.require_edit()?;

    if held.runner_user_id != Some(user.id) {
        return Err(ApiError::forbidden("Only the runner that claimed this job can write to it"));
    }

    Ok(held)
}

fn still_open(held: &Held) -> ApiResult<()> {
    if held.job.state.ended() {
        return Err(ApiError::conflict(format!("This job has ended ({})", held.job.state.as_str())));
    }

    Ok(())
}

fn heard(tx: &Tx, job_id: Id) -> ApiResult<()> {
    tx.execute("UPDATE e2e_job SET heard_at = ?1 WHERE id = ?2", (&tx.now, job_id))?;

    Ok(())
}

#[derive(Deserialize)]
struct Shown {
    limit: Option<i64>,
}

async fn list_jobs(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Query(shown): Query<Shown>) -> Reply<Vec<E2eJob>> {
    app.run(move |app| {
        fail_stale(app)?;

        app.read(|connection| {
            let access = project_access(connection, &user, &key)?;

            project_jobs(connection, access.id, shown.limit.unwrap_or(JOBS_SHOWN).clamp(1, MAX_JOBS_SHOWN))
        })
    })
    .await
    .map(Json)
}

async fn request_job(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(new): Json<NewJob>) -> Reply<E2eJob> {
    app.run(move |app| app.write(user.actor(), |tx| create(tx, &user, &key, &new))).await.map(Json)
}

#[derive(Deserialize)]
struct After {
    after: Option<i64>,
}

async fn job_detail(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Query(after): Query<After>) -> Reply<E2eJobDetail> {
    app.run(move |app| {
        fail_stale(app)?;

        app.read(|connection| {
            let (held, _) = visible(connection, &user, id)?;
            let log = query_all(
                connection,
                "SELECT seq, text FROM e2e_job_log WHERE job_id = ?1 AND seq > ?2 ORDER BY seq",
                (id, after.after.unwrap_or(0)),
                |row| Ok(E2eLogLine { seq: row.get(0)?, text: row.get(1)? }),
            )?;
            let oldest: Option<i64> = connection.query_row("SELECT min(seq) FROM e2e_job_log WHERE job_id = ?1", [id], |row| row.get(0))?;

            Ok(E2eJobDetail { job: held.job, log, log_truncated: oldest.is_some_and(|oldest| oldest > 1) })
        })
    })
    .await
    .map(Json)
}

/// A queued job is cancelled at once. One a runner holds is marked, and ends when the runner
/// next writes to it and so learns that it should stop.
async fn cancel_job(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<E2eJob> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let (held, access) = visible(tx, &user, id)?;
            access.require_edit()?;

            match held.job.state {
                E2eJobState::Queued => end(tx, &held, E2eJobState::Cancelled, None)?,
                E2eJobState::Claimed | E2eJobState::Running if !held.job.cancel_requested => {
                    tx.execute("UPDATE e2e_job SET cancel_requested = 1 WHERE id = ?1", [id])?;
                    announce(tx, held.project_id, id, held.job.state)?;
                }
                _ => {}
            }

            job_by_id(tx, id)
        })
    })
    .await
    .map(Json)
}

/// A runner asking for a job. Each time it asks it also says it is there, and what it is.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Claim {
    runner_id: String,
    #[serde(default)]
    runner_name: Option<String>,
    /// Project keys the runner works for; every project its person can edit when left out.
    #[serde(default)]
    projects: Option<Vec<String>>,
    /// Somebody's computer with the person at it; false for a service. True when left out.
    #[serde(default)]
    interactive: Option<bool>,
    #[serde(default)]
    version: Option<String>,
    /// Take only jobs sent to this runner by name.
    #[serde(default)]
    pinned_only: bool,
    /// The team's runner (`docs/runners.md`): it works on anyone's job with that person's keys.
    #[serde(default)]
    shared: bool,
}

/// What a runner is handed with a job: how to run the suite, secret values included. The
/// one place those leave the server.
#[derive(Serialize)]
struct RunnerSettings {
    #[serde(flatten)]
    settings: E2eSettings,
    secrets: std::collections::BTreeMap<String, String>,
}

/// The person's key a job on a shared runner is cloned with (`docs/runners.md`).
pub const GITHUB_TOKEN_KEY: &str = "github_token";

/// The keys of the person who asked for the job, for the shared runner that claimed it and
/// for that job only. They are read when the claim is answered and kept nowhere else.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Credentials {
    #[serde(skip_serializing_if = "Option::is_none")]
    github_token: Option<String>,
}

#[derive(Serialize)]
struct Claimed {
    job: E2eJob,
    settings: RunnerSettings,
    /// Only for a shared runner: a runner of one's own works with what it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    credentials: Option<Credentials>,
}

/// The job this runner takes next, in a project its person may edit (and that it names,
/// when it names any): one sent to it by name before an older one anybody may take, and
/// never one sent to another runner.
fn next_for(tx: &Tx, user: &AuthUser, runner: &Presence) -> ApiResult<Option<(Id, Id)>> {
    let queued: Vec<(Id, Id, String)> = query_all(
        tx,
        "SELECT j.id, j.project_id, p.key FROM e2e_job j JOIN project p ON p.id = j.project_id
         WHERE j.state = 'queued' AND (j.on_runner = ?1 OR (j.on_runner IS NULL AND NOT ?2))
         ORDER BY j.on_runner IS NULL, j.id",
        (&runner.id, runner.pinned_only),
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    Ok(queued
        .into_iter()
        .filter(|(_, _, key)| runner.projects.as_ref().is_none_or(|wanted| wanted.iter().any(|name| name.eq_ignore_ascii_case(key))))
        .find(|(_, project_id, _)| project_access_by_id(tx, user, *project_id).is_ok_and(|access| access.can_edit()))
        .map(|(id, project_id, _)| (id, project_id)))
}

async fn claim_job(State(app): State<App>, user: AuthUser, Json(claim): Json<Claim>) -> Reply<Option<Claimed>> {
    app.run(move |app| {
        let id = tidy(Some(&claim.runner_id), 200).ok_or_else(|| ApiError::bad_request("\"runnerId\" is required: a name that stays the same for this runner"))?;
        let runner = Presence {
            name: tidy(claim.runner_name.as_deref(), 200).unwrap_or_else(|| id.clone()),
            id,
            projects: claim.projects.clone(),
            interactive: claim.interactive.unwrap_or(true),
            pinned_only: claim.pinned_only,
            shared: claim.shared,
            version: tidy(claim.version.as_deref(), 100).unwrap_or_default(),
        };

        app.write(user.actor(), |tx| {
            runners::seen(tx, &user, &runner)?;

            let Some((id, project_id)) = next_for(tx, &user, &runner)? else { return Ok(None) };

            tx.execute(
                "UPDATE e2e_job SET state = 'claimed', runner_id = ?1, runner_name = ?2, runner_user_id = ?3, started_at = ?4, heard_at = ?4 WHERE id = ?5",
                (&runner.id, &runner.name, user.id, &tx.now, id),
            )?;
            announce(tx, project_id, id, E2eJobState::Claimed)?;

            let mut settings = settings::load(tx, project_id)?;
            let secrets = settings::secret_values(app, tx, project_id)?;
            settings.secret_names = secrets.keys().cloned().collect();

            let job = job_by_id(tx, id)?;
            let credentials = match runner.shared {
                true => Some(Credentials { github_token: personal_keys::reveal(app, tx, job.requested_by, GITHUB_TOKEN_KEY)? }),
                false => None,
            };

            Ok(Some(Claimed { job, settings: RunnerSettings { settings, secrets }, credentials }))
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Patch {
    state: Option<String>,
    step: Option<String>,
    commit: Option<String>,
    error: Option<String>,
}

/// The runner says what it is doing; the answer tells it whether to stop (`cancelRequested`).
/// A job somebody asked to cancel ends here, the moment its runner learns of it.
async fn patch_job(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(patch): Json<Patch>) -> Reply<E2eJob> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let held = for_its_runner(tx, &user, id)?;
            still_open(&held)?;
            heard(tx, id)?;

            if let Some(commit) = tidy(patch.commit.as_deref(), 100) {
                tx.execute("UPDATE e2e_job SET commit_sha = ?1 WHERE id = ?2", (commit, id))?;
            }

            if held.job.cancel_requested {
                end(tx, &held, E2eJobState::Cancelled, None)?;

                return job_by_id(tx, id);
            }

            match patch.state.as_deref() {
                None => {}
                Some("running") => {
                    tx.execute("UPDATE e2e_job SET state = 'running' WHERE id = ?1", [id])?;
                }
                Some("failed") => {
                    end(tx, &held, E2eJobState::Failed, Some(tidy(patch.error.as_deref(), 4000).as_deref().unwrap_or(RUNNER_FAILED)))?;

                    return job_by_id(tx, id);
                }
                Some(_) => return Err(ApiError::bad_request("A runner sets a job's state to running or failed; finish ends it")),
            }

            if let Some(step) = tidy(patch.step.as_deref(), 100) {
                tx.execute("UPDATE e2e_job SET step = ?1 WHERE id = ?2", (step, id))?;
            }

            let job = job_by_id(tx, id)?;
            announce(tx, held.project_id, id, job.state)?;

            Ok(job)
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct Lines {
    lines: Vec<String>,
}

#[derive(Serialize)]
struct Logged {
    seq: i64,
}

fn since(earlier: &str) -> Option<chrono::Duration> {
    Some(Utc::now() - DateTime::parse_from_rfc3339(earlier).ok()?.with_timezone(&Utc))
}

/// Tells open pages about log lines they have not heard of, unless they were told a moment ago.
/// True when there is nothing left untold.
fn announce_log(tx: &Tx, job_id: Id) -> ApiResult<bool> {
    let (project_id, seq, told_seq, told_at): (Id, i64, i64, Option<String>) = tx.query_row(
        "SELECT project_id, log_seq, log_announced_seq, log_announced_at FROM e2e_job WHERE id = ?1",
        [job_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;

    if seq <= told_seq {
        return Ok(true);
    }

    let too_soon = told_at.as_deref().and_then(since).is_some_and(|ago| ago.to_std().is_ok_and(|ago| ago < LOG_ANNOUNCE_EVERY));

    if too_soon {
        return Ok(false);
    }

    tx.execute("UPDATE e2e_job SET log_announced_seq = ?1, log_announced_at = ?2 WHERE id = ?3", (seq, &tx.now, job_id))?;
    tx.emit("e2e.job.log", Some(project_id), None, json!({ "jobId": job_id, "seq": seq }))?;

    Ok(true)
}

/// A line of a terminal's output as a page shows it: without colours, cursor moves and other
/// control sequences, and no longer than a line may be.
fn plain(line: &str) -> String {
    let mut text = String::new();
    let mut characters = line.chars().peekable();

    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            // `ESC [ … letter` is a control sequence; any other escape is two characters.
            if characters.next() == Some('[') {
                while characters.next().is_some_and(|next| !('@'..='~').contains(&next)) {}
            }
        } else if !character.is_control() || character == '\t' {
            text.push(character);
        }
    }

    text.chars().take(MAX_LINE_CHARS).collect()
}

/// Adds lines to a job's log and drops the ones that fall out of what is kept.
fn add_lines(tx: &Tx, job_id: Id, lines: &[String]) -> ApiResult<i64> {
    let mut seq: i64 = tx.query_row("SELECT log_seq FROM e2e_job WHERE id = ?1", [job_id], |row| row.get(0))?;
    let mut insert = tx.prepare("INSERT INTO e2e_job_log (job_id, seq, text) VALUES (?1, ?2, ?3)")?;

    for line in lines {
        seq += 1;
        insert.execute((job_id, seq, plain(line)))?;
    }

    tx.execute("UPDATE e2e_job SET log_seq = ?1 WHERE id = ?2", (seq, job_id))?;
    tx.execute("DELETE FROM e2e_job_log WHERE job_id = ?1 AND seq <= ?2", (job_id, seq - LOG_LINES_KEPT))?;

    Ok(seq)
}

async fn append_log(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, Json(lines): Json<Lines>) -> Reply<Logged> {
    let (seq, told) = app
        .run({
            let user = user.clone();

            move |app| {
                app.write(user.actor(), |tx| {
                    let held = for_its_runner(tx, &user, id)?;
                    still_open(&held)?;
                    heard(tx, id)?;

                    Ok((add_lines(tx, id, &lines.lines)?, announce_log(tx, id)?))
                })
            }
        })
        .await?;

    if !told {
        // Pages were told less than a moment ago. They still hear of these lines, once the
        // moment has passed, even if the runner sends nothing more.
        tokio::spawn(async move {
            tokio::time::sleep(LOG_ANNOUNCE_EVERY + Duration::from_millis(50)).await;

            let _ = app.run(move |app| app.write(user.actor(), |tx| announce_log(tx, id))).await;
        });
    }

    Ok(Json(Logged { seq }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Finish {
    error: Option<String>,
}

/// The runner is done: succeeded when the report arrived and it names no error, failed
/// otherwise. A job that has already ended is answered as it ended, so a runner may always
/// finish what it claimed.
async fn finish_job(State(app): State<App>, user: AuthUser, Path(id): Path<Id>, body: Option<Json<Finish>>) -> Reply<E2eJob> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let held = for_its_runner(tx, &user, id)?;

            if held.job.state.ended() {
                return Ok(held.job);
            }

            heard(tx, id)?;

            let error = body.as_ref().and_then(|body| tidy(body.error.as_deref(), 4000));

            match (held.job.cancel_requested, error, held.job.run_id) {
                (true, _, _) => end(tx, &held, E2eJobState::Cancelled, None)?,
                (false, Some(error), _) => end(tx, &held, E2eJobState::Failed, Some(&error))?,
                (false, None, Some(_)) => end(tx, &held, E2eJobState::Succeeded, None)?,
                (false, None, None) => end(tx, &held, E2eJobState::Failed, Some(NO_REPORT))?,
            }

            job_by_id(tx, id)
        })
    })
    .await
    .map(Json)
}

/// Checks that `user` is the runner of job `job_id` and may tie a run of this project to it.
pub fn check_report_job(tx: &Tx, user: &AuthUser, job_id: Id, project_id: Id) -> ApiResult<()> {
    let held = for_its_runner(tx, user, job_id)?;

    if held.project_id != project_id {
        return Err(ApiError::bad_request("That job belongs to another project"));
    }

    still_open(&held)
}

/// Ties a stored run to the job that produced it. The run takes the job's commit and ref
/// where its report named none.
pub fn tie_run(tx: &Tx, job_id: Id, run_id: Id) -> ApiResult<()> {
    let held = held(tx, job_id)?;

    tx.execute("UPDATE e2e_job SET run_id = ?1, heard_at = ?2 WHERE id = ?3", (run_id, &tx.now, job_id))?;
    tx.execute(
        "UPDATE e2e_run SET commit_sha = coalesce(commit_sha, ?1), branch = coalesce(branch, ?2) WHERE id = ?3",
        (&held.job.commit, &held.job.git_ref, run_id),
    )?;

    announce(tx, held.project_id, job_id, held.job.state)
}

/// Notes that the runner of the job behind a run is still at work: an upload can take a while.
pub fn heard_for_run(tx: &Tx, run_id: Id) -> ApiResult<()> {
    tx.execute("UPDATE e2e_job SET heard_at = ?1 WHERE run_id = ?2 AND state IN ('claimed', 'running')", (&tx.now, run_id))?;

    Ok(())
}

/// The job that produced a run, when one did.
pub fn job_of_run(connection: &Connection, run_id: Id) -> ApiResult<Option<Id>> {
    query_one(connection, "SELECT id FROM e2e_job WHERE run_id = ?1 ORDER BY id DESC LIMIT 1", [run_id], |row| row.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_line_loses_what_only_a_terminal_understands() {
        assert_eq!(plain("\u{1b}[32m✓\u{1b}[0m \u{1b}[1;90mtests/cart.e2e.ts\u{1b}[39m\r"), "✓ tests/cart.e2e.ts");
        assert_eq!(plain("\u{1b}[2K\u{1b}[1Gprogress\t50%\u{7}"), "progress\t50%");
        assert_eq!(plain(&"x".repeat(MAX_LINE_CHARS + 10)).chars().count(), MAX_LINE_CHARS);
    }
}
