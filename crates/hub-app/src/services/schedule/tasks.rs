//! The tasks of a project's schedule: reading them, and making, changing and deleting them.
//! Every change announces `task.updated` with the project.

use hub_core::schedule::{ScheduledTask, TaskField, TaskFieldType, TaskKind, TaskResult, TaskSlot, TaskState};
use hub_core::Id;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::access::{project_access_by_id, ProjectAccess};
use crate::app::{App, Tx};
use crate::auth::AuthUser;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::{project_modules, search};

const SELECT: &str = "SELECT t.id, p.key, t.kind, t.name, t.config, t.slot, t.enabled, t.run_as, t.state, t.last_started_at, t.last_finished_at,
        t.last_error, t.progress, t.result_at, t.result_summary, t.result_link, t.project_id
    FROM scheduled_task t JOIN project p ON p.id = t.project_id";
const MAX_NAME: usize = 120;
const MAX_TEXT: usize = 8000;

/// A task with what the contract's shape leaves out.
pub struct TaskRow {
    pub task: ScheduledTask,
    pub project_id: Id,
}

fn map_row(row: &rusqlite::Row) -> rusqlite::Result<TaskRow> {
    let config: String = row.get(4)?;
    let slot: String = row.get(5)?;
    let state: String = row.get(8)?;
    let result_at: Option<String> = row.get(13)?;

    Ok(TaskRow {
        task: ScheduledTask {
            id: row.get(0)?,
            project_key: row.get(1)?,
            kind: row.get(2)?,
            name: row.get(3)?,
            config: serde_json::from_str(&config).unwrap_or_default(),
            slot: TaskSlot::parse(&slot).unwrap_or(TaskSlot::OnDemand),
            enabled: row.get(6)?,
            run_as: row.get(7)?,
            state: TaskState::parse(&state).unwrap_or(TaskState::Idle),
            last_started_at: row.get(9)?,
            last_finished_at: row.get(10)?,
            last_error: row.get(11)?,
            progress: row.get(12)?,
            result: match result_at {
                Some(at) => Some(TaskResult { at, summary: row.get::<_, Option<String>>(14)?.unwrap_or_default(), link: row.get(15)? }),
                None => None,
            },
            // Decided by whoever reads the row, who knows the project's modules.
            kind_available: false,
        },
        project_id: row.get(16)?,
    })
}

/// Whether the kind "<module>.<name>" is the module's.
pub(super) fn owns(module_id: &str, kind_id: &str) -> bool {
    kind_id.strip_prefix(module_id).is_some_and(|rest| rest.starts_with('.'))
}

/// The kinds of every module that is on for the instance. A kind that does not carry its
/// module's id is left out: the id is how a task finds its module again.
pub fn task_kinds(app: &App) -> Vec<TaskKind> {
    app.enabled_modules()
        .flat_map(|module| {
            let id = module.meta().id;

            module.task_kinds().into_iter().filter(move |kind| kind.module == id && owns(id, &kind.id))
        })
        .collect()
}

/// The kinds a project can run: those of the modules it has not switched off.
pub(super) fn kinds_in_project(app: &App, connection: &Connection, project_id: Id) -> ApiResult<Vec<TaskKind>> {
    let off = project_modules::disabled_modules(connection, project_id)?;

    Ok(task_kinds(app).into_iter().filter(|kind| !off.contains(&kind.module)).collect())
}

fn rows(app: &App, connection: &Connection, tail: &str, id: Id) -> ApiResult<Vec<TaskRow>> {
    let mut rows = query_all(connection, &format!("{SELECT} {tail}"), [id], map_row)?;
    let mut kinds: Option<(Id, Vec<TaskKind>)> = None;

    for row in &mut rows {
        if kinds.as_ref().is_none_or(|(project_id, _)| *project_id != row.project_id) {
            kinds = Some((row.project_id, kinds_in_project(app, connection, row.project_id)?));
        }

        row.task.kind_available = kinds.as_ref().is_some_and(|(_, kinds)| kinds.iter().any(|kind| kind.id == row.task.kind));
    }

    Ok(rows)
}

/// A project's tasks, in the order they were made.
pub fn list(app: &App, connection: &Connection, project_id: Id) -> ApiResult<Vec<ScheduledTask>> {
    Ok(rows(app, connection, "WHERE t.project_id = ?1 ORDER BY t.id", project_id)?.into_iter().map(|row| row.task).collect())
}

pub(super) fn load(app: &App, connection: &Connection, id: Id) -> ApiResult<TaskRow> {
    rows(app, connection, "WHERE t.id = ?1", id)?.pop().ok_or_else(|| ApiError::not_found("Scheduled task"))
}

/// The task and the caller's access to its project. A task the caller may not see is missing.
pub(super) fn task_access(app: &App, connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(TaskRow, ProjectAccess)> {
    let row = load(app, connection, id)?;
    let access = project_access_by_id(connection, user, row.project_id).map_err(|_| ApiError::not_found("Scheduled task"))?;

    Ok((row, access))
}

/// Tells open pages that the task changed. `change` is set for what people did to it
/// ("created", "changed", "deleted"), which is what the activity feed tells apart.
pub(super) fn announce(tx: &Tx, id: Id, change: Option<&str>) -> ApiResult<()> {
    let (project_id, name, state): (Id, String, String) =
        tx.query_row("SELECT project_id, name, state FROM scheduled_task WHERE id = ?1", [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;

    tx.emit("task.updated", Some(project_id), None, json!({ "taskId": id, "state": state, "name": name, "change": change }))
}

fn clean_name(name: &str) -> ApiResult<&str> {
    let name = name.trim();

    if name.is_empty() {
        return Err(ApiError::bad_request("The task needs a name"));
    }

    if name.chars().count() > MAX_NAME {
        return Err(ApiError::bad_request(format!("The name is longer than {MAX_NAME} characters")));
    }

    Ok(name)
}

/// The text of a field, checked: there when it has to be, not endless, and for a query one
/// the tracker understands, tried as the person saving it.
fn clean_text(app: &App, connection: &Connection, user: &AuthUser, field: &TaskField, text: &str) -> ApiResult<Value> {
    let label = &field.label;
    let text = text.trim();

    if text.is_empty() && field.required {
        return Err(ApiError::bad_request(format!("\"{label}\" is required")));
    }

    if text.chars().count() > MAX_TEXT {
        return Err(ApiError::bad_request(format!("\"{label}\" is longer than {MAX_TEXT} characters")));
    }

    if field.kind == TaskFieldType::Select && !text.is_empty() && !field.options.iter().any(|option| option.value == text) {
        return Err(ApiError::bad_request(format!("\"{label}\" has to be one of its options")));
    }

    if field.kind == TaskFieldType::Query && !text.is_empty() {
        search::compile(app, connection, user, text).map_err(|error| ApiError::bad_request(format!("\"{label}\": {}", error.message)))?;
    }

    Ok(Value::String(text.to_string()))
}

/// One field of a task's settings as it is stored: the value given, else the kind's default.
fn clean_field(app: &App, connection: &Connection, user: &AuthUser, field: &TaskField, given: Option<&Value>) -> ApiResult<Option<Value>> {
    let label = &field.label;
    let value = given.filter(|value| !value.is_null()).or(field.default.as_ref());

    match (field.kind, value) {
        (TaskFieldType::Boolean, None) => Ok(Some(Value::Bool(false))),
        (TaskFieldType::Boolean, Some(Value::Bool(on))) => Ok(Some(Value::Bool(*on))),
        (TaskFieldType::Boolean, Some(_)) => Err(ApiError::bad_request(format!("\"{label}\" has to be yes or no"))),
        (TaskFieldType::Number, None) if field.required => Err(ApiError::bad_request(format!("\"{label}\" is required"))),
        (TaskFieldType::Number, None) => Ok(None),
        (TaskFieldType::Number, Some(Value::Number(number))) => Ok(Some(Value::Number(number.clone()))),
        (TaskFieldType::Number, Some(_)) => Err(ApiError::bad_request(format!("\"{label}\" has to be a number"))),
        (_, None) => clean_text(app, connection, user, field, "").map(Some),
        (_, Some(Value::String(text))) => clean_text(app, connection, user, field, text).map(Some),
        (_, Some(_)) => Err(ApiError::bad_request(format!("\"{label}\" has to be text"))),
    }
}

/// A task's settings as its kind's form defines them: every field checked, anything the form
/// does not have left out. What the values mean is checked by the module when the task runs.
fn clean_config(app: &App, connection: &Connection, user: &AuthUser, kind: &TaskKind, given: &Map<String, Value>) -> ApiResult<Map<String, Value>> {
    let mut config = Map::new();

    for field in &kind.fields {
        if let Some(value) = clean_field(app, connection, user, field, given.get(&field.name))? {
            config.insert(field.name.clone(), value);
        }
    }

    Ok(config)
}

fn kind_in_project(app: &App, connection: &Connection, project_id: Id, id: &str) -> ApiResult<TaskKind> {
    kinds_in_project(app, connection, project_id)?
        .into_iter()
        .find(|kind| kind.id == id)
        .ok_or_else(|| ApiError::bad_request("That kind of task is not available in this project; its module may be switched off"))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTask {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub config: Map<String, Value>,
    pub slot: TaskSlot,
    pub enabled: Option<bool>,
}

/// Adds a task to the project's schedule. It will run as whoever makes it.
pub fn create(app: &App, tx: &Tx, user: &AuthUser, access: &ProjectAccess, new: &NewTask) -> ApiResult<ScheduledTask> {
    access.require_admin()?;

    let kind = kind_in_project(app, tx, access.id, &new.kind)?;
    let config = clean_config(app, tx, user, &kind, &new.config)?;

    tx.execute(
        "INSERT INTO scheduled_task (project_id, kind, name, config, slot, enabled, run_as, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        (
            access.id,
            &kind.id,
            clean_name(&new.name)?,
            Value::Object(config).to_string(),
            new.slot.as_str(),
            new.enabled.unwrap_or(true),
            user.id,
            &tx.now,
        ),
    )?;
    let id = tx.last_insert_rowid();
    announce(tx, id, Some("created"))?;

    Ok(load(app, tx, id)?.task)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskPatch {
    pub name: Option<String>,
    pub config: Option<Map<String, Value>>,
    pub slot: Option<TaskSlot>,
    pub enabled: Option<bool>,
}

/// Changes a task. Whoever saves its settings becomes the person it runs as: they are the
/// one who last decided what it does, and it is how a task outlives the person who made it.
pub fn update(app: &App, tx: &Tx, user: &AuthUser, id: Id, patch: &TaskPatch) -> ApiResult<ScheduledTask> {
    let (row, access) = task_access(app, tx, user, id)?;
    access.require_admin()?;

    if let Some(name) = &patch.name {
        tx.execute("UPDATE scheduled_task SET name = ?1 WHERE id = ?2", (clean_name(name)?, id))?;
    }

    if let Some(config) = &patch.config {
        let kind = kind_in_project(app, tx, row.project_id, &row.task.kind)?;
        let config = clean_config(app, tx, user, &kind, config)?;

        tx.execute("UPDATE scheduled_task SET config = ?1, run_as = ?2 WHERE id = ?3", (Value::Object(config).to_string(), user.id, id))?;
    }

    if let Some(slot) = patch.slot {
        tx.execute("UPDATE scheduled_task SET slot = ?1 WHERE id = ?2", (slot.as_str(), id))?;
    }

    if let Some(enabled) = patch.enabled {
        tx.execute("UPDATE scheduled_task SET enabled = ?1 WHERE id = ?2", (enabled, id))?;
    }

    announce(tx, id, Some("changed"))?;

    Ok(load(app, tx, id)?.task)
}

/// Deletes a task. Work it started goes on; there is just nobody left to tell how it ended.
pub fn delete(app: &App, tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<()> {
    let (_, access) = task_access(app, tx, user, id)?;
    access.require_admin()?;

    announce(tx, id, Some("deleted"))?;
    tx.execute("DELETE FROM scheduled_task WHERE id = ?1", [id])?;

    Ok(())
}

/// The ids of a project's tasks that a run of `slot` starts: switched on, of a kind the
/// project can run, and not still running from the time before.
pub(super) fn startable(app: &App, connection: &Connection, project_id: Id, slot: TaskSlot) -> ApiResult<Vec<Id>> {
    Ok(list(app, connection, project_id)?
        .into_iter()
        .filter(|task| task.slot == slot && task.enabled && task.kind_available && task.state != TaskState::Running)
        .map(|task| task.id)
        .collect())
}

/// Whether the task exists; a module may report the end of one that was deleted meanwhile.
pub(super) fn exists(connection: &Connection, id: Id) -> ApiResult<bool> {
    Ok(query_one(connection, "SELECT 1 FROM scheduled_task WHERE id = ?1", [id], |row| row.get::<_, i64>(0))?.is_some())
}
