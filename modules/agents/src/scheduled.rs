//! What this module runs on a schedule (`docs/schedule.md`): `agents.run`, an agent run on
//! each issue a query finds. The schedule decides when; this file says what the form holds
//! and turns a task into runs, as the person the task runs as.

use hub_app::access::{project_access, visible_projects};
use hub_app::db::{query_all, query_one};
use hub_app::services::schedule::{self, NewTask, Outcome};
use hub_app::services::search;
use hub_app::services::users::get_user;
use hub_app::{Actor, ApiError, ApiResult, App, AuthUser, Tx};
use hub_core::demo::{DemoLanguage, DemoText};
use hub_core::schedule::{ScheduledTask, TaskField, TaskFieldType, TaskKind, TaskOption, TaskSlot, TaskStarted};
use hub_core::Id;
use rusqlite::types::Value as SqlValue;
use serde_json::{json, Map, Value};

use crate::catalog::OWN_MODULE;
use crate::runs::{self, NewRun};
use crate::types::{AgentRunKind, AgentRunState};

pub const RUN_KIND: &str = "agents.run";
const DEFAULT_LIMIT: i64 = 5;
const MAX_LIMIT: i64 = 20;
/// Where the runs a task started are followed.
const RUNS_PAGE: &str = "/agents";
/// How the journal labels runs that a task asked for, next to the person it runs as.
const LABEL: &str = "Schedule";

fn kind_label(kind: AgentRunKind) -> &'static str {
    match kind {
        AgentRunKind::Refine => "Refine description",
        AgentRunKind::Breakdown => "Break down into subtasks",
        AgentRunKind::Summarize => "Summarise",
        AgentRunKind::Triage => "Triage",
        AgentRunKind::Implement => "Implement in code",
        AgentRunKind::Review => "Review the pull request",
        AgentRunKind::Custom => "Custom task",
    }
}

fn kind_options() -> Vec<TaskOption> {
    AgentRunKind::ALL.iter().map(|kind| TaskOption { value: kind.as_str().to_string(), label: kind_label(*kind).to_string() }).collect()
}

pub fn kinds() -> Vec<TaskKind> {
    vec![TaskKind {
        id: RUN_KIND.to_string(),
        module: OWN_MODULE.to_string(),
        name: "Run an agent on issues".to_string(),
        description: "Starts an agent run on each issue a query finds in this project. An issue that already has an unfinished run of the same kind is left alone."
            .to_string(),
        fields: vec![
            TaskField {
                name: "query".to_string(),
                label: "Query".to_string(),
                hint: Some("Which issues, in the query language. Only issues of this project are looked at.".to_string()),
                required: true,
                kind: TaskFieldType::Query,
                ..TaskField::default()
            },
            TaskField {
                name: "limit".to_string(),
                label: "Limit".to_string(),
                hint: Some("How many issues at most, from 1 to 20: the first ones the query finds.".to_string()),
                kind: TaskFieldType::Number,
                default: Some(json!(DEFAULT_LIMIT)),
                ..TaskField::default()
            },
            TaskField {
                name: "kind".to_string(),
                label: "What the agent does".to_string(),
                required: true,
                kind: TaskFieldType::Select,
                options: kind_options(),
                default: Some(json!(AgentRunKind::Triage.as_str())),
                ..TaskField::default()
            },
            TaskField {
                name: "instructions".to_string(),
                label: "Instructions".to_string(),
                hint: Some("Extra guidance for the agent, the same for every issue.".to_string()),
                kind: TaskFieldType::Textarea,
                ..TaskField::default()
            },
            TaskField {
                name: "repo".to_string(),
                label: "Code repository".to_string(),
                hint: Some("For work in code, as owner/name.".to_string()),
                kind: TaskFieldType::Text,
                ..TaskField::default()
            },
            TaskField {
                name: "shared".to_string(),
                label: "Any runner of the project may take the runs".to_string(),
                hint: Some("Otherwise only the runners of the person the task runs as.".to_string()),
                kind: TaskFieldType::Boolean,
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

/// What a task's settings ask for.
struct Wanted {
    query: String,
    limit: i64,
    run: NewRun,
}

fn text(config: &Map<String, Value>, name: &str) -> String {
    config.get(name).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

/// The schedule has checked the form's shape; what the values mean is checked here.
fn wanted(config: &Map<String, Value>) -> ApiResult<Wanted> {
    let limit = match config.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(value) => value
            .as_i64()
            .filter(|limit| (1..=MAX_LIMIT).contains(limit))
            .ok_or_else(|| ApiError::bad_request(format!("The limit has to be a whole number from 1 to {MAX_LIMIT}")))?,
    };
    let kind = AgentRunKind::parse(&text(config, "kind")).ok_or_else(|| ApiError::bad_request("Choose what the agent does"))?;
    let query = text(config, "query");

    if query.is_empty() {
        return Err(ApiError::bad_request("The task needs a query that finds its issues"));
    }

    Ok(Wanted {
        query,
        limit,
        run: NewRun {
            kind,
            instructions: text(config, "instructions"),
            model: None,
            repo: Some(text(config, "repo")).filter(|repo| !repo.is_empty()),
            shared: config.get("shared").and_then(Value::as_bool).unwrap_or(false),
            runner_id: None,
            on_runner: Some(text(config, "onRunner")).filter(|runner| !runner.is_empty()),
        },
    })
}

/// The keys of the first `limit` issues the query finds in the task's project. The project
/// comes from the task, not from the query's text, and the actor's access decides the rest.
fn find(app: &App, task: &ScheduledTask, actor: &AuthUser, wanted: &Wanted) -> ApiResult<Vec<String>> {
    app.read(|connection| {
        let project = project_access(connection, actor, &task.project_key)?;
        let compiled = search::compile(app, connection, actor, &wanted.query)
            .map_err(|error| ApiError::bad_request(format!("The query cannot be understood: {}", error.message)))?;
        let (visible, mut params) = visible_projects(actor, "i.project_id");

        params.push(SqlValue::Integer(project.id));
        params.extend(compiled.params);
        params.push(SqlValue::Integer(wanted.limit));

        query_all(
            connection,
            &format!(
                "SELECT i.key FROM issue i JOIN project p ON p.id = i.project_id
                 WHERE {visible} AND i.project_id = ? AND ({}) ORDER BY {} LIMIT ?",
                compiled.filter, compiled.order
            ),
            rusqlite::params_from_iter(params),
            |row| row.get(0),
        )
    })
}

/// Whether the issue has a run of this kind that has not ended yet.
fn has_unfinished(tx: &Tx, key: &str, kind: AgentRunKind) -> ApiResult<bool> {
    let states: Vec<String> = query_all(
        tx,
        "SELECT r.state FROM agent_run r JOIN issue i ON i.id = r.issue_id WHERE i.key = ?1 AND r.kind = ?2",
        (key, kind.as_str()),
        |row| row.get(0),
    )?;

    Ok(states.iter().any(|state| !AgentRunState::parse(state).is_some_and(AgentRunState::is_finished)))
}

/// Starts the task: one run per issue found, all of them or none.
pub fn start(app: &App, task: &ScheduledTask, actor: &AuthUser) -> ApiResult<TaskStarted> {
    let wanted = wanted(&task.config)?;
    let found = find(app, task, actor, &wanted)?;
    let started = app.write(Actor::user_via(actor.id, LABEL), |tx| {
        let mut started = Vec::new();

        for key in &found {
            if !has_unfinished(tx, key, wanted.run.kind)? {
                started.push(runs::create(tx, actor, key, &wanted.run)?.issue_key);
            }
        }

        Ok(started)
    })?;
    let skipped = found.len() - started.len();
    let keys = started.join(", ");
    let text = match (started.len(), skipped) {
        (0, 0) => "Nothing matched".to_string(),
        (0, _) => format!("Nothing started: {skipped} of the issues found already have an unfinished run"),
        (1, 0) => format!("Started 1 run: {keys}"),
        (count, 0) => format!("Started {count} runs: {keys}"),
        (1, _) => format!("Started 1 run: {keys}; skipped {skipped} with an unfinished run"),
        (count, _) => format!("Started {count} runs: {keys}; skipped {skipped} with an unfinished run"),
    };

    Ok(TaskStarted::Done { summary: text, link: Some(RUNS_PAGE.to_string()) })
}

// ---------- example data ----------

struct Demo {
    name: DemoText,
    slot: TaskSlot,
    query: &'static str,
    kind: AgentRunKind,
    instructions: DemoText,
    /// Days ago the task last ran, finding nothing; none for a task that has not run yet.
    ran: Option<i64>,
}

const DEMO: &[Demo] = &[
    Demo {
        name: DemoText::new("Triage new issues", "Triage nowych zgłoszeń"),
        slot: TaskSlot::Morning,
        query: "created >= -1d AND category = todo ORDER BY created",
        kind: AgentRunKind::Triage,
        instructions: DemoText::new(
            "Suggest a priority and labels. Point out what is missing before work can start.",
            "Zaproponuj priorytet i etykiety. Zaznacz, czego brakuje do rozpoczęcia pracy.",
        ),
        ran: Some(1),
    },
    Demo {
        name: DemoText::new("Summarise issues waiting for review", "Podsumowanie zadań czekających na review"),
        slot: TaskSlot::Afternoon,
        query: "status = \"In Review\" AND updated <= -3d ORDER BY updated",
        kind: AgentRunKind::Summarize,
        instructions: DemoText::new(
            "Write in three sentences where the issue stands and who should take the next step.",
            "Napisz w trzech zdaniach, na czym stoi zadanie i kto powinien zrobić następny krok.",
        ),
        ran: None,
    },
    Demo {
        name: DemoText::new("Refine stories before planning", "Doprecyzuj historie przed planowaniem"),
        slot: TaskSlot::OnDemand,
        query: "type = Story AND sprint is empty AND category = todo ORDER BY rank",
        kind: AgentRunKind::Refine,
        instructions: DemoText::EMPTY,
        ran: Some(4),
    },
];

/// A few tasks in the demo's first project, made by its administrator, so the schedule's
/// page has something to show. They are real: the next slot runs them. Their names and
/// instructions are written in the language of the demo.
pub fn seed_demo(app: &App, language: DemoLanguage) -> ApiResult<()> {
    let found: Option<(String, Id)> = app.read(|connection| {
        query_one(
            connection,
            "SELECT p.key, u.id FROM project p, user u WHERE p.key = 'WEB' AND u.role = 'admin' AND u.active = 1 ORDER BY u.id LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    })?;
    let Some((key, admin_id)) = found else { return Ok(()) };
    let person = app.read(|connection| get_user(connection, admin_id))?;
    let admin = AuthUser { id: person.id, name: person.name, email: person.email, role: person.role, via_token: false, through: Vec::new() };

    for demo in DEMO {
        let config = json!({ "query": demo.query, "kind": demo.kind.as_str(), "instructions": demo.instructions.pick(language), "limit": 3 });
        let new = NewTask {
            kind: RUN_KIND.to_string(),
            name: demo.name.pick(language).to_string(),
            config: config.as_object().cloned().unwrap_or_default(),
            slot: demo.slot,
            enabled: None,
        };
        let task = app.write(admin.actor(), |tx| schedule::create(app, tx, &admin, &project_access(tx, &admin, &key)?, &new))?;

        if let Some(days) = demo.ran {
            let at = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let nothing = Outcome { summary: "Nothing matched".to_string(), link: Some(RUNS_PAGE.to_string()) };

            app.write_at(Actor::system(LABEL), at, |tx| schedule::finish(tx, task.id, Ok(nothing)))?;
        }
    }

    Ok(())
}
