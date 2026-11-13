//! Running tasks: from their button, as a group, or by the minute-by-minute check.
//!
//! A run has three steps. The task is marked `running` in a transaction; its module is asked
//! to start it, outside any transaction and never on the worker pool, because what a module
//! does there is its own business; and what the module answered is written down. A failure
//! sets `failed` and the reason, and leaves the last good result where it was.

use std::panic::AssertUnwindSafe;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hub_core::schedule::{ScheduledTask, TaskSlot, TaskStarted, TaskState};
use hub_core::Id;
use rusqlite::Connection;

use super::tasks::{announce, exists, list, load, owns, startable, task_access, TaskRow};
use crate::access::{project_access, project_access_by_id};
use crate::app::{Actor, App, Tx};
use crate::auth::AuthUser;
use crate::db::query_all;
use crate::error::{ApiError, ApiResult};
use crate::services::users::get_user;

/// Who the journal names for what the schedule does by itself.
pub(super) const ACTOR: &str = "Schedule";
const STUCK_AFTER_HOURS: i64 = 6;
const STUCK: &str = "It was still running after six hours and was given up on";
const INTERRUPTED: &str = "The server stopped while the task was starting";
const NO_THREAD: &str = "The server could not start the task; try again";

/// How work that a module reported as `TaskStarted::Running` ended well.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub summary: String,
    pub link: Option<String>,
}

pub(super) fn iso(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Records how a task's attempt ended; for the module that answered `TaskStarted::Running`,
/// in the transaction in which its work ends. A good outcome becomes the task's result. A
/// reason for failing is kept beside the older result, which stays on show. For a task that
/// was deleted in the meantime nothing happens, and nothing fails.
pub fn finish(tx: &Tx, task_id: Id, outcome: Result<Outcome, String>) -> ApiResult<()> {
    if !exists(tx, task_id)? {
        return Ok(());
    }

    match outcome {
        Ok(outcome) => tx.execute(
            "UPDATE scheduled_task SET state = 'ok', last_finished_at = ?1, last_error = NULL, progress = NULL,
                result_at = ?1, result_summary = ?2, result_link = ?3 WHERE id = ?4",
            (&tx.now, outcome.summary.trim(), outcome.link, task_id),
        )?,
        Err(reason) => tx.execute(
            "UPDATE scheduled_task SET state = 'failed', last_finished_at = ?1, last_error = ?2, progress = NULL WHERE id = ?3",
            (&tx.now, reason.trim(), task_id),
        )?,
    };

    announce(tx, task_id, None)
}

fn begin(tx: &Tx, id: Id) -> ApiResult<()> {
    tx.execute("UPDATE scheduled_task SET state = 'running', last_started_at = ?1, progress = NULL WHERE id = ?2", (&tx.now, id))?;

    announce(tx, id, None)
}

/// Marks as running every task of the project that a run of `slot` starts, and returns them
/// in the order they are to be started: the order they were made in.
pub(super) fn begin_slot(app: &App, tx: &Tx, project_id: Id, slot: TaskSlot) -> ApiResult<Vec<ScheduledTask>> {
    let ids = startable(app, tx, project_id, slot)?;

    for id in &ids {
        begin(tx, *id)?;
    }

    Ok(list(app, tx, project_id)?.into_iter().filter(|task| ids.contains(&task.id)).collect())
}

/// What the module said about work that goes on. Nothing changes when the work has already
/// ended: a module may finish before its answer is written down.
fn note_progress(tx: &Tx, id: Id, summary: &str) -> ApiResult<()> {
    let noted = tx.execute("UPDATE scheduled_task SET progress = ?1 WHERE id = ?2 AND state = 'running'", (summary.trim(), id))?;

    if noted > 0 {
        announce(tx, id, None)?;
    }

    Ok(())
}

fn unavailable() -> ApiError {
    ApiError::conflict("This task cannot run: the module it belongs to is switched off")
}

/// The person the task runs as, acting through the task's module. A task never runs as
/// nobody: when that person is gone or can no longer see the project, the attempt fails and
/// says what to do about it.
fn acting_person(connection: &Connection, row: &TaskRow, module_id: &'static str) -> ApiResult<AuthUser> {
    let person = get_user(connection, row.task.run_as)?;
    let name = &person.name;

    if !person.active {
        return Err(ApiError::forbidden(format!(
            "{name} has been deactivated, and the task runs with their permissions; save the task again to run it as yourself"
        )));
    }

    let actor = AuthUser {
        id: person.id,
        name: person.name.clone(),
        email: person.email.clone(),
        role: person.role,
        via_token: false,
        through: vec![module_id],
    };

    project_access_by_id(connection, &actor, row.project_id).map_err(|_| {
        ApiError::forbidden(format!(
            "{name} can no longer see this project, and the task runs with their permissions; save the task again to run it as yourself"
        ))
    })?;

    Ok(actor)
}

/// Asks the task's module to start it. The module is only asked while it is on, for the
/// instance and for the task's project, and a panic in it is this attempt's failure only.
fn start(app: &App, id: Id) -> ApiResult<TaskStarted> {
    let row = app.read(|connection| load(app, connection, id))?;
    let module = app.enabled_modules().find(|module| row.task.kind_available && owns(module.meta().id, &row.task.kind)).ok_or_else(unavailable)?;
    let actor = app.read(|connection| acting_person(connection, &row, module.meta().id))?;

    match std::panic::catch_unwind(AssertUnwindSafe(|| module.start_task(app, &row.task, &actor))) {
        Ok(Some(answer)) => answer,
        Ok(None) => Err(unavailable()),
        Err(_) => {
            tracing::error!(module = module.meta().id, task = id, "module panicked while starting a task");

            Err(ApiError::conflict("The task's module broke while starting it; the server's log has the details"))
        }
    }
}

/// Starts a task that has been marked as running, and writes down how that went. `clock`
/// dates the outcome. Nothing is returned: whatever goes wrong is the task's own failure.
pub(super) fn carry_out(app: &App, id: Id, clock: &dyn Fn() -> DateTime<Utc>) {
    let answer = start(app, id);
    let written = app.write_at(Actor::system(ACTOR), iso(clock()), |tx| match answer {
        Ok(TaskStarted::Done { summary, link }) => finish(tx, id, Ok(Outcome { summary, link })),
        Ok(TaskStarted::Running { summary }) => note_progress(tx, id, &summary),
        Err(error) => finish(tx, id, Err(error.message)),
    });

    if let Err(error) = written {
        tracing::error!(task = id, "the outcome of a task could not be recorded: {}", error.message);
    }
}

fn fail(app: &App, ids: &[Id], now: DateTime<Utc>, reason: &str) -> ApiResult<()> {
    app.write_at(Actor::system(ACTOR), iso(now), |tx| ids.iter().try_for_each(|id| finish(tx, *id, Err(reason.to_string()))))
}

/// Starts tasks that have been marked as running, one after another, on a thread of their
/// own: the request that asked for them answers at once, and the worker pool is not held up
/// by whatever a module does to start.
fn in_background(app: &App, ids: Vec<Id>) {
    if ids.is_empty() {
        return;
    }

    let worker = app.clone();
    let queued = ids.clone();
    let spawned = std::thread::Builder::new().name("schedule-run".to_string()).spawn(move || {
        for id in queued {
            carry_out(&worker, id, &Utc::now);
        }
    });

    if let Err(error) = spawned {
        tracing::error!("could not start a thread for scheduled tasks: {error}");

        if let Err(error) = fail(app, &ids, Utc::now(), NO_THREAD) {
            tracing::error!("and could not record that: {}", error.message);
        }
    }
}

/// Runs one task now, for a member who can edit the project. Answers with the task as it is
/// once marked `running`; how it ends is announced as `task.updated`.
pub fn run_task(app: &App, user: &AuthUser, id: Id) -> ApiResult<ScheduledTask> {
    let task = app.write(user.actor(), |tx| {
        let (row, access) = task_access(app, tx, user, id)?;
        access.require_edit()?;

        if row.task.state == TaskState::Running {
            return Err(ApiError::conflict("The task is already running"));
        }

        if !row.task.kind_available {
            return Err(unavailable());
        }

        begin(tx, id)?;

        Ok(load(app, tx, id)?.task)
    })?;

    in_background(app, vec![task.id]);

    Ok(task)
}

/// Runs a project's group now: every task of `slot` that is switched on, can run and is not
/// running already. Answers with those tasks, marked `running`.
pub fn run_slot(app: &App, user: &AuthUser, key: &str, slot: TaskSlot) -> ApiResult<Vec<ScheduledTask>> {
    let tasks = app.write(user.actor(), |tx| {
        let access = project_access(tx, user, key)?;
        access.require_edit()?;

        begin_slot(app, tx, access.id, slot)
    })?;

    in_background(app, tasks.iter().map(|task| task.id).collect());

    Ok(tasks)
}

fn running_since(connection: &Connection, condition: &str, before: &str) -> ApiResult<Vec<Id>> {
    query_all(connection, &format!("SELECT id FROM scheduled_task WHERE state = 'running' AND {condition} ORDER BY id"), [before], |row| row.get(0))
}

/// Marks as failed every task that has been running for six hours: whatever it started is
/// not going to report back, and a task that stays `running` can never be run again.
pub(super) fn give_up_on_stuck(app: &App, now: DateTime<Utc>) -> ApiResult<()> {
    let before = iso(now - Duration::hours(STUCK_AFTER_HOURS));
    let stuck = app.read(|connection| running_since(connection, "last_started_at <= ?1", &before))?;

    if stuck.is_empty() {
        return Ok(());
    }

    fail(app, &stuck, now, STUCK)
}

/// For when the server comes up: a task that was marked as running, and whose module never
/// got to answer, was cut off by the server stopping. Nobody is working on it.
pub(super) fn fail_interrupted(app: &App, now: DateTime<Utc>) -> ApiResult<()> {
    let cut_off = app.read(|connection| running_since(connection, "progress IS NULL AND last_started_at <= ?1", &iso(now)))?;

    if cut_off.is_empty() {
        return Ok(());
    }

    fail(app, &cut_off, now, INTERRUPTED)
}
