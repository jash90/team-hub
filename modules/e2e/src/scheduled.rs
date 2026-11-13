//! What this module runs on a schedule (`docs/schedule.md`): `e2e.job`, the project's test
//! suite. The schedule decides when; this file says what the form holds, queues the job as
//! the person the task runs as, and tells the schedule how the job ended.

use hub_app::db::query_one;
use hub_app::services::schedule::{self, Outcome};
use hub_app::{Actor, ApiResult, App, AuthUser, Tx};
use hub_core::schedule::{ScheduledTask, TaskField, TaskFieldType, TaskKind, TaskOption, TaskStarted};
use hub_core::Id;
use serde_json::{Map, Value};

use crate::jobs::{self, NewJob};
use crate::types::{E2eJob, E2eJobState};

pub const JOB_KIND: &str = "e2e.job";
const MODULE: &str = "e2e";
/// How the journal labels jobs that a task asked for, next to the person it runs as.
const LABEL: &str = "Schedule";

fn option(value: &str, label: &str) -> TaskOption {
    TaskOption { value: value.to_string(), label: label.to_string() }
}

pub fn kinds() -> Vec<TaskKind> {
    vec![TaskKind {
        id: JOB_KIND.to_string(),
        module: MODULE.to_string(),
        name: "Run the test suite".to_string(),
        description: "Runs the project's e2e suite on a runner, with the repository and the settings from Project settings → Tests, and keeps the recording of the session."
            .to_string(),
        fields: vec![
            TaskField {
                name: "ref".to_string(),
                label: "Branch, tag or commit".to_string(),
                hint: Some("Leave empty for the one in the project's test settings.".to_string()),
                kind: TaskFieldType::Text,
                ..TaskField::default()
            },
            TaskField {
                name: "filter".to_string(),
                label: "Only these tests".to_string(),
                hint: Some("Test files, or a pattern for test names. Leave empty for the whole suite.".to_string()),
                kind: TaskFieldType::Text,
                ..TaskField::default()
            },
            TaskField {
                name: "video".to_string(),
                label: "Recording".to_string(),
                kind: TaskFieldType::Select,
                options: vec![
                    option("", "As in the project's test settings"),
                    option("on", "Record every test"),
                    option("retain-on-failure", "Keep recordings of failed tests"),
                    option("on-first-retry", "Record the first retry"),
                    option("off", "No recording"),
                ],
                ..TaskField::default()
            },
            TaskField {
                name: "onRunner".to_string(),
                label: "Runner".to_string(),
                hint: Some("The id of the runner that carries it out, as the Runners page shows it. Leave empty for any runner that fits.".to_string()),
                kind: TaskFieldType::Text,
                ..TaskField::default()
            },
        ],
    }]
}

fn text(config: &Map<String, Value>, name: &str) -> Option<String> {
    config.get(name).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(String::from)
}

/// Queues the job. The task stays "running" until the job ends (`ended`).
pub fn start(app: &App, task: &ScheduledTask, actor: &AuthUser) -> ApiResult<TaskStarted> {
    let new = NewJob {
        git_ref: text(&task.config, "ref"),
        filter: text(&task.config, "filter"),
        video: text(&task.config, "video"),
        trace: None,
        task_id: Some(task.id),
        on_runner: text(&task.config, "onRunner"),
    };
    let job = app.write(Actor::user_via(actor.id, LABEL), |tx| jobs::create(tx, actor, &task.project_key, &new))?;

    Ok(TaskStarted::Running {
        summary: match &job.on_runner_name {
            Some(runner) => format!("Tests of {} queued for {runner}", job.git_ref),
            None => format!("Tests of {} queued for a runner", job.git_ref),
        },
    })
}

/// Tells the schedule how a job that a task started has ended. A suite that ran is a result
/// even when tests failed: the summary says how many. A job that could not be run, or was
/// cancelled, is a failure of the task.
pub fn ended(tx: &Tx, job: &E2eJob, state: E2eJobState, error: Option<&str>) -> ApiResult<()> {
    let Some(task_id) = query_one(tx, "SELECT task_id FROM e2e_job WHERE id = ?1", [job.id], |row| row.get::<_, Option<Id>>(0))?.flatten() else {
        return Ok(());
    };
    let link = Some(format!("/p/{}/tests/jobs/{}", job.project_key, job.id));
    let outcome = match state {
        E2eJobState::Succeeded => {
            let counts: Option<(i64, i64, i64, i64)> = match job.run_id {
                Some(run) => query_one(tx, "SELECT total, passed, failed, flaky FROM e2e_run WHERE id = ?1", [run], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?,
                None => None,
            };

            Ok(Outcome {
                summary: match counts {
                    Some((total, passed, 0, 0)) if passed == total => format!("All {total} tests passed"),
                    Some((total, _, failed, flaky)) => format!("{failed} of {total} tests failed, {flaky} flaky"),
                    None => "The suite ran".to_string(),
                },
                link,
            })
        }
        E2eJobState::Cancelled => Err("The test job was cancelled".to_string()),
        _ => Err(error.map(String::from).or_else(|| job.error.clone()).unwrap_or_else(|| "The test job failed".to_string())),
    };

    schedule::finish(tx, task_id, outcome)
}
