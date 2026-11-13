//! Agent runs: starting them, following them, and the runner's side of carrying them out.
//! Everything here runs inside a transaction and announces `agent.run` when a run changes.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hub_app::access::{project_access_by_id, visible_projects, ProjectAccess};
use hub_app::db::{placeholders, query_all, query_one};
use hub_app::services::issues::issue_access;
use hub_app::services::notifications::notify;
use hub_app::{ApiError, ApiResult, AuthUser, Tx};
use hub_core::text::truncate;
use hub_core::Id;
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::proposals;
use crate::types::{AgentEventKind, AgentRun, AgentRunDetail, AgentRunEvent, AgentRunKind, AgentRunState, AgentRunner};

/// The runner id of the server itself.
pub const SERVER_RUNNER: &str = "server";
/// How changes made on an agent's behalf are labelled in histories and journals.
pub const AI_LABEL: &str = "AI agent";
const STALE_AFTER_MINUTES: i64 = 10;
const ONLINE_FOR_SECONDS: i64 = 120;
const MAX_INSTRUCTIONS: usize = 8000;
const MAX_EVENT_TEXT: usize = 100_000;

const SELECT: &str = "SELECT r.id, i.key, i.summary, p.key, r.kind, r.instructions, r.state, r.requested_by, r.shared, r.runner_id, r.runner_name,
        r.model, r.repo, r.branch, r.pull_request_url, r.summary, r.input_tokens, r.output_tokens, r.cost_usd, r.cancel_requested,
        (SELECT count(*) FROM agent_proposal a WHERE a.run_id = r.id AND a.state = 'pending'),
        r.question_pending, r.created_at, r.started_at, r.finished_at, r.issue_id, r.project_id, r.runner_user_id
    FROM agent_run r JOIN issue i ON i.id = r.issue_id JOIN project p ON p.id = r.project_id";

/// A run with what the contract's shape leaves out.
pub struct RunRow {
    pub run: AgentRun,
    pub issue_id: Id,
    pub project_id: Id,
    /// Whose token the runner that holds the run uses; none for the server.
    pub runner_user_id: Option<Id>,
}

impl RunRow {
    pub fn on_server(&self) -> bool {
        self.run.runner_id.as_deref() == Some(SERVER_RUNNER)
    }
}

fn iso(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn map_row(row: &rusqlite::Row) -> rusqlite::Result<RunRow> {
    let kind: String = row.get(4)?;
    let state: String = row.get(6)?;

    Ok(RunRow {
        run: AgentRun {
            id: row.get(0)?,
            issue_key: row.get(1)?,
            issue_summary: row.get(2)?,
            project_key: row.get(3)?,
            kind: AgentRunKind::parse(&kind).unwrap_or(AgentRunKind::Custom),
            instructions: row.get(5)?,
            state: AgentRunState::parse(&state).unwrap_or(AgentRunState::Failed),
            requested_by: row.get(7)?,
            shared: row.get(8)?,
            runner_id: row.get(9)?,
            runner_name: row.get(10)?,
            model: row.get(11)?,
            repo: row.get(12)?,
            branch: row.get(13)?,
            pull_request_url: row.get(14)?,
            summary: row.get(15)?,
            input_tokens: row.get(16)?,
            output_tokens: row.get(17)?,
            cost_usd: row.get(18)?,
            cancel_requested: row.get(19)?,
            pending_proposals: row.get(20)?,
            pending_question: row.get(21)?,
            created_at: row.get(22)?,
            started_at: row.get(23)?,
            finished_at: row.get(24)?,
        },
        issue_id: row.get(25)?,
        project_id: row.get(26)?,
        runner_user_id: row.get(27)?,
    })
}

/// Runs for `tail`, the part of the statement after the FROM clause.
pub fn rows(connection: &Connection, tail: &str, params: Vec<SqlValue>) -> ApiResult<Vec<RunRow>> {
    query_all(connection, &format!("{SELECT} {tail}"), rusqlite::params_from_iter(params), map_row)
}

pub fn load(connection: &Connection, id: Id) -> ApiResult<RunRow> {
    rows(connection, "WHERE r.id = ?", vec![SqlValue::Integer(id)])?.pop().ok_or_else(|| ApiError::not_found("Run"))
}

/// The run and the caller's access to its project. A run the caller may not see is missing.
pub fn run_access(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(RunRow, ProjectAccess)> {
    let row = load(connection, id)?;
    let access = project_access_by_id(connection, user, row.project_id).map_err(|_| ApiError::not_found("Run"))?;

    Ok((row, access))
}

fn events(connection: &Connection, run_id: Id) -> ApiResult<Vec<AgentRunEvent>> {
    query_all(connection, "SELECT id, run_id, kind, text, data, actor_id, at FROM agent_run_event WHERE run_id = ?1 ORDER BY id", [run_id], |row| {
        let kind: String = row.get(2)?;
        let data: Option<String> = row.get(4)?;

        Ok(AgentRunEvent {
            id: row.get(0)?,
            run_id: row.get(1)?,
            kind: AgentEventKind::parse(&kind).unwrap_or(AgentEventKind::Status),
            text: row.get(3)?,
            data: data.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null),
            actor_id: row.get(5)?,
            at: row.get(6)?,
        })
    })
}

pub fn detail(connection: &Connection, id: Id) -> ApiResult<AgentRunDetail> {
    let row = load(connection, id)?;

    Ok(AgentRunDetail { events: events(connection, id)?, proposals: proposals::list(connection, &row)?, run: row.run })
}

/// Tells open pages and runners that the run changed.
pub fn announce(tx: &Tx, id: Id) -> ApiResult<()> {
    let row = load(tx, id)?;

    tx.emit("agent.run", Some(row.project_id), Some(row.issue_id), json!({ "runId": id, "state": row.run.state, "key": row.run.issue_key }))
}

pub fn add_event(tx: &Tx, run_id: Id, kind: AgentEventKind, text: &str, data: Option<&Value>, actor_id: Option<Id>) -> ApiResult<Id> {
    tx.execute(
        "INSERT INTO agent_run_event (run_id, kind, text, data, actor_id, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (run_id, kind.as_str(), text, data.filter(|data| !data.is_null()).map(Value::to_string), actor_id, &tx.now),
    )?;

    Ok(tx.last_insert_rowid())
}

/// A notification for whoever asked for the run.
pub fn tell_requester(tx: &Tx, row: &RunRow, text: &str) -> ApiResult<()> {
    notify(tx, row.run.requested_by, "agent", row.issue_id, &format!("{}: {text}", row.run.issue_key))
}

fn set_state(tx: &Tx, id: Id, state: AgentRunState) -> ApiResult<()> {
    tx.execute("UPDATE agent_run SET state = ?1 WHERE id = ?2", (state.as_str(), id))?;

    if state == AgentRunState::Running {
        tx.execute("UPDATE agent_run SET started_at = coalesce(started_at, ?1) WHERE id = ?2", (&tx.now, id))?;
    }

    if state.is_finished() {
        tx.execute("UPDATE agent_run SET finished_at = ?1, question_pending = 0 WHERE id = ?2", (&tx.now, id))?;
    }

    if state == AgentRunState::Cancelled {
        proposals::reject_pending(tx, id, "The run was cancelled")?;
    }

    Ok(())
}

/// Ends a run and tells its requester how it went.
pub fn finish(tx: &Tx, id: Id, state: AgentRunState) -> ApiResult<()> {
    set_state(tx, id, state)?;
    let row = load(tx, id)?;

    match state {
        AgentRunState::Succeeded => {
            let outcome = row.run.summary.as_deref().and_then(|summary| summary.lines().find(|line| !line.trim().is_empty())).unwrap_or("no summary");

            tell_requester(tx, &row, &format!("the agent finished: {}", truncate(outcome, 140)))
        }
        AgentRunState::Failed => tell_requester(tx, &row, "the agent run failed"),
        _ => Ok(()),
    }
}

/// A run that waited for a person and no longer does goes on: back to its runner, or, when
/// the server carried it out and has nothing left to do, to its end.
pub fn settle(tx: &Tx, id: Id) -> ApiResult<()> {
    let row = load(tx, id)?;

    if row.run.state != AgentRunState::Waiting || row.run.pending_proposals > 0 || row.run.pending_question {
        return Ok(());
    }

    if row.on_server() {
        finish(tx, id, AgentRunState::Succeeded)
    } else {
        set_state(tx, id, AgentRunState::Running)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRun {
    pub kind: AgentRunKind,
    #[serde(default)]
    pub instructions: String,
    pub model: Option<String>,
    pub repo: Option<String>,
    #[serde(default)]
    pub shared: bool,
    /// The caller's own runner that carries the run out. The run is created already claimed
    /// by it, so no other runner can take it in between.
    pub runner_id: Option<String>,
}

fn optional(text: Option<&str>, what: &str, max: usize) -> ApiResult<Option<String>> {
    match text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) if text.chars().count() > max || text.chars().any(char::is_control) => {
            Err(ApiError::bad_request(format!("That is not a valid {what}")))
        }
        other => Ok(other.map(String::from)),
    }
}

/// Queues a run on an issue the caller may edit.
pub fn create(tx: &Tx, user: &AuthUser, key: &str, new: &NewRun) -> ApiResult<AgentRun> {
    let (issue_id, access) = issue_access(tx, user, key)?;
    access.require_edit()?;
    let instructions = new.instructions.trim();

    if instructions.chars().count() > MAX_INSTRUCTIONS {
        return Err(ApiError::bad_request(format!("The instructions are longer than {MAX_INSTRUCTIONS} characters")));
    }

    tx.execute(
        "INSERT INTO agent_run (issue_id, project_id, kind, instructions, requested_by, shared, model, repo, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        (
            issue_id,
            access.id,
            new.kind.as_str(),
            instructions,
            user.id,
            new.shared,
            optional(new.model.as_deref(), "model", 80)?,
            optional(new.repo.as_deref(), "repository", 140)?,
            &tx.now,
        ),
    )?;
    let id = tx.last_insert_rowid();

    match &new.runner_id {
        Some(runner) => {
            let runner = runner_id(runner)?;
            let name = own_runner(tx, user, runner)?.ok_or_else(|| ApiError::bad_request("That runner has not announced itself yet; send a heartbeat first"))?;

            hold(tx, id, runner, &name, Some(user.id))?;
        }
        None => announce(tx, id)?,
    }

    Ok(load(tx, id)?.run)
}

pub fn list_for_issue(connection: &Connection, user: &AuthUser, key: &str) -> ApiResult<Vec<AgentRun>> {
    let (issue_id, _) = issue_access(connection, user, key)?;

    Ok(rows(connection, "WHERE r.issue_id = ? ORDER BY r.id DESC LIMIT 100", vec![SqlValue::Integer(issue_id)])?
        .into_iter()
        .map(|row| row.run)
        .collect())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RunFilter {
    /// One state, or several separated by commas.
    pub state: Option<String>,
    pub mine: bool,
    pub project: Option<String>,
    pub limit: Option<i64>,
}

/// Runs in the projects the caller may see, newest first.
pub fn list(connection: &Connection, user: &AuthUser, filter: &RunFilter) -> ApiResult<Vec<AgentRun>> {
    let (visible, mut params) = visible_projects(user, "r.project_id");
    let mut conditions = vec![visible];

    if let Some(states) = filter.state.as_deref().map(str::trim).filter(|states| !states.is_empty()) {
        let states: Vec<&str> = states.split(',').map(str::trim).collect();

        if states.iter().any(|state| AgentRunState::parse(state).is_none()) {
            return Err(ApiError::bad_request("That is not a run state"));
        }

        conditions.push(format!("r.state IN ({})", placeholders(states.len())));
        params.extend(states.into_iter().map(|state| SqlValue::Text(state.to_string())));
    }

    if filter.mine {
        conditions.push("r.requested_by = ?".to_string());
        params.push(SqlValue::Integer(user.id));
    }

    if let Some(project) = filter.project.as_deref().map(str::trim).filter(|project| !project.is_empty()) {
        conditions.push("p.key = ? COLLATE NOCASE".to_string());
        params.push(SqlValue::Text(project.to_string()));
    }

    params.push(SqlValue::Integer(filter.limit.unwrap_or(100).clamp(1, 500)));

    Ok(rows(connection, &format!("WHERE {} ORDER BY r.id DESC LIMIT ?", conditions.join(" AND ")), params)?.into_iter().map(|row| row.run).collect())
}

/// Stops a run. Nobody is working on a queued run, or on one that waits for a person on
/// the server's behalf, so those end at once; a runner at work is asked to stop.
pub fn cancel(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<AgentRun> {
    let (row, access) = run_access(tx, user, id)?;
    access.require_edit()?;

    if row.run.state.is_finished() {
        return Ok(row.run);
    }

    if row.run.state == AgentRunState::Queued || (row.run.state == AgentRunState::Waiting && row.on_server()) {
        add_event(tx, id, AgentEventKind::Status, "Cancelled", None, Some(user.id))?;
        finish(tx, id, AgentRunState::Cancelled)?;
    } else {
        tx.execute("UPDATE agent_run SET cancel_requested = 1 WHERE id = ?1", [id])?;
        add_event(tx, id, AgentEventKind::Status, "Asked to stop", None, Some(user.id))?;
    }

    announce(tx, id)?;

    Ok(load(tx, id)?.run)
}

/// A person's answer to the run's question.
pub fn answer(tx: &Tx, user: &AuthUser, id: Id, text: &str) -> ApiResult<AgentRunDetail> {
    let (row, access) = run_access(tx, user, id)?;
    access.require_edit()?;
    let text = text.trim();

    if text.is_empty() {
        return Err(ApiError::bad_request("The answer is empty"));
    }

    if row.run.state.is_finished() {
        return Err(ApiError::conflict("The run has already ended"));
    }

    add_event(tx, id, AgentEventKind::Answer, text, None, Some(user.id))?;
    tx.execute("UPDATE agent_run SET question_pending = 0 WHERE id = ?1", [id])?;
    settle(tx, id)?;
    announce(tx, id)?;

    detail(tx, id)
}

// ---------- the runner's side ----------

/// The run, for the runner that claimed it, whether or not it has ended.
fn holder(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<RunRow> {
    let (row, _) = run_access(tx, user, id)?;

    if row.on_server() || row.runner_user_id != Some(user.id) {
        return Err(ApiError::forbidden("Only the runner that claimed this run may write to it"));
    }

    Ok(row)
}

/// The run, for the runner that claimed it; nobody else may write to it.
pub fn held(tx: &Tx, user: &AuthUser, id: Id) -> ApiResult<RunRow> {
    let row = holder(tx, user, id)?;

    if row.run.state.is_finished() {
        return Err(ApiError::conflict("The run has already ended"));
    }

    tx.execute("UPDATE agent_run SET runner_seen_at = ?1 WHERE id = ?2", (&tx.now, id))?;

    Ok(row)
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewEvent {
    pub kind: AgentEventKind,
    pub text: String,
    #[serde(default)]
    pub data: Value,
}

/// Something the runner reports. A question makes the run wait for a person.
pub fn record(tx: &Tx, row: &RunRow, event: &NewEvent) -> ApiResult<Id> {
    let id = row.run.id;
    let text = event.text.trim();

    if event.kind == AgentEventKind::Answer {
        return Err(ApiError::bad_request("Answers come from people, not from runners"));
    }

    if text.is_empty() || text.len() > MAX_EVENT_TEXT {
        return Err(ApiError::bad_request("An event needs a text of reasonable length"));
    }

    let event_id = add_event(tx, id, event.kind, text, Some(&event.data), None)?;

    if event.kind == AgentEventKind::Question {
        tx.execute("UPDATE agent_run SET question_pending = 1 WHERE id = ?1", [id])?;
        set_state(tx, id, AgentRunState::Waiting)?;
        tell_requester(tx, row, &format!("the agent asks: {}", truncate(text, 140)))?;
    } else if row.run.state == AgentRunState::Claimed {
        set_state(tx, id, AgentRunState::Running)?;
    }

    announce(tx, id)?;

    Ok(event_id)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunPatch {
    pub state: Option<AgentRunState>,
    pub summary: Option<String>,
    pub branch: Option<String>,
    pub pull_request_url: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
}

impl RunPatch {
    /// True when the patch only says where the code went, which often becomes known after
    /// the run has ended: the person pushes and opens the pull request afterwards.
    fn only_names_the_code(&self) -> bool {
        self.state.is_none()
            && self.summary.is_none()
            && self.model.is_none()
            && self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cost_usd.is_none()
    }
}

/// What the runner reports about a run it holds. A run that has ended still takes its
/// branch and its pull request, and nothing else.
pub fn report(tx: &Tx, user: &AuthUser, id: Id, patch: &RunPatch) -> ApiResult<AgentRun> {
    let row = holder(tx, user, id)?;

    if row.run.state.is_finished() && !patch.only_names_the_code() {
        return Err(ApiError::conflict("The run has already ended; only its branch and pull request can still be recorded"));
    }

    if !row.run.state.is_finished() {
        tx.execute("UPDATE agent_run SET runner_seen_at = ?1 WHERE id = ?2", (&tx.now, id))?;
    }

    update(tx, &row, patch)
}

/// What the runner reports about the run as a whole: its state, its result, what it cost.
pub fn update(tx: &Tx, row: &RunRow, patch: &RunPatch) -> ApiResult<AgentRun> {
    let id = row.run.id;

    if let Some(summary) = &patch.summary {
        tx.execute("UPDATE agent_run SET summary = ?1 WHERE id = ?2", (summary.trim(), id))?;
    }

    if let Some(branch) = &patch.branch {
        tx.execute("UPDATE agent_run SET branch = ?1 WHERE id = ?2", (optional(Some(branch), "branch name", 250)?, id))?;
    }

    if let Some(url) = &patch.pull_request_url {
        let url = optional(Some(url), "pull request address", 500)?;

        if url.as_deref().is_some_and(|url| !url.starts_with("https://") && !url.starts_with("http://")) {
            return Err(ApiError::bad_request("The pull request address must start with https://"));
        }

        tx.execute("UPDATE agent_run SET pull_request_url = ?1 WHERE id = ?2", (url, id))?;
    }

    if let Some(model) = &patch.model {
        tx.execute("UPDATE agent_run SET model = ?1 WHERE id = ?2", (optional(Some(model), "model", 80)?, id))?;
    }

    if patch.input_tokens.is_some_and(|tokens| tokens < 0)
        || patch.output_tokens.is_some_and(|tokens| tokens < 0)
        || patch.cost_usd.is_some_and(|cost| !(0.0..1e9).contains(&cost))
    {
        return Err(ApiError::bad_request("Tokens and cost cannot be negative"));
    }

    tx.execute(
        "UPDATE agent_run SET input_tokens = coalesce(?1, input_tokens), output_tokens = coalesce(?2, output_tokens), cost_usd = coalesce(?3, cost_usd) WHERE id = ?4",
        (patch.input_tokens, patch.output_tokens, patch.cost_usd, id),
    )?;

    match patch.state {
        Some(AgentRunState::Queued | AgentRunState::Claimed) => return Err(ApiError::bad_request("A runner cannot put a run back in the queue")),
        Some(state) if state.is_finished() => finish(tx, id, state)?,
        Some(state) => set_state(tx, id, state)?,
        None => {}
    }

    announce(tx, id)?;

    Ok(load(tx, id)?.run)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Heartbeat {
    pub runner_id: String,
    pub name: String,
    #[serde(default)]
    pub kinds: Vec<AgentRunKind>,
    pub projects: Option<Vec<String>>,
    #[serde(default)]
    pub version: String,
}

fn runner_id(text: &str) -> ApiResult<&str> {
    let id = text.trim();

    if id.is_empty() || id.len() > 100 || id == SERVER_RUNNER || id.chars().any(|character| character.is_control() || character.is_whitespace()) {
        return Err(ApiError::bad_request("A runner id is 1 to 100 characters without spaces"));
    }

    Ok(id)
}

/// A runner saying it is there, and what it can do.
pub fn heartbeat(tx: &Tx, user: &AuthUser, beat: &Heartbeat) -> ApiResult<()> {
    let id = runner_id(&beat.runner_id)?;
    let name = optional(Some(&beat.name), "runner name", 120)?.unwrap_or_else(|| id.to_string());
    let owner: Option<Id> = query_one(tx, "SELECT user_id FROM agent_runner WHERE id = ?1", [id], |row| row.get(0))?;

    if owner.is_some_and(|owner| owner != user.id) {
        return Err(ApiError::conflict("That runner id belongs to somebody else's runner"));
    }

    let kinds: Vec<&str> = beat.kinds.iter().map(|kind| kind.as_str()).collect();

    tx.execute(
        "INSERT INTO agent_runner (id, name, user_id, kinds, projects, version, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (id) DO UPDATE SET name = ?2, kinds = ?4, projects = ?5, version = ?6, last_seen_at = ?7",
        (
            id,
            name,
            user.id,
            json!(kinds).to_string(),
            beat.projects.as_ref().map(|projects| json!(projects).to_string()),
            beat.version.trim(),
            &tx.now,
        ),
    )?;

    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub runner_id: String,
    pub kinds: Vec<AgentRunKind>,
    pub projects: Option<Vec<String>>,
    /// Claim this run and no other.
    pub run_id: Option<Id>,
}

/// The name of the caller's runner with this id; none when it has not announced itself.
fn own_runner(tx: &Tx, user: &AuthUser, runner: &str) -> ApiResult<Option<String>> {
    let owner: Option<(Id, String)> =
        query_one(tx, "SELECT user_id, name FROM agent_runner WHERE id = ?1", [runner], |row| Ok((row.get(0)?, row.get(1)?)))?;

    match owner {
        Some((owner, _)) if owner != user.id => Err(ApiError::conflict("That runner id belongs to somebody else's runner")),
        other => Ok(other.map(|(_, name)| name)),
    }
}

/// Hands the oldest queued run that fits to this runner, or the one it names. A run is
/// offered to its requester's own runners, or to anyone who can see the project when it is
/// shared. Transactions are serialised, so two runners never get the same run.
pub fn claim(tx: &Tx, user: &AuthUser, wanted: &Claim) -> ApiResult<Option<Id>> {
    let runner = runner_id(&wanted.runner_id)?;
    let owner = own_runner(tx, user, runner)?;

    if wanted.kinds.is_empty() {
        return Ok(None);
    }

    tx.execute("UPDATE agent_runner SET last_seen_at = ?1 WHERE id = ?2", (&tx.now, runner))?;

    let (visible, mut params) = visible_projects(user, "r.project_id");
    let mut conditions = vec![
        visible,
        "r.state = 'queued'".to_string(),
        "(r.requested_by = ? OR r.shared = 1)".to_string(),
        format!("r.kind IN ({})", placeholders(wanted.kinds.len())),
    ];
    params.push(SqlValue::Integer(user.id));
    params.extend(wanted.kinds.iter().map(|kind| SqlValue::Text(kind.as_str().to_string())));

    if let Some(projects) = wanted.projects.as_ref().filter(|projects| !projects.is_empty()) {
        conditions.push(format!("p.key COLLATE NOCASE IN ({})", placeholders(projects.len())));
        params.extend(projects.iter().map(|key| SqlValue::Text(key.trim().to_string())));
    }

    if let Some(run_id) = wanted.run_id {
        conditions.push("r.id = ?".to_string());
        params.push(SqlValue::Integer(run_id));
    }

    let Some(row) = rows(tx, &format!("WHERE {} ORDER BY r.id LIMIT 1", conditions.join(" AND ")), params)?.pop() else { return Ok(None) };
    let name = owner.unwrap_or_else(|| runner.to_string());

    hold(tx, row.run.id, runner, &name, Some(user.id))?;

    Ok(Some(row.run.id))
}

/// Marks a queued run as taken by a runner.
pub fn hold(tx: &Tx, id: Id, runner: &str, name: &str, user_id: Option<Id>) -> ApiResult<()> {
    tx.execute(
        "UPDATE agent_run SET state = 'claimed', runner_id = ?1, runner_name = ?2, runner_user_id = ?3, runner_seen_at = ?4 WHERE id = ?5",
        (runner, name, user_id, &tx.now, id),
    )?;
    add_event(tx, id, AgentEventKind::Status, &format!("Taken by {name}"), None, None)?;

    announce(tx, id)
}

fn requeue(tx: &Tx, id: Id, why: &str) -> ApiResult<()> {
    let cancelled: bool = tx.query_row("SELECT cancel_requested FROM agent_run WHERE id = ?1", [id], |row| row.get(0))?;

    add_event(tx, id, AgentEventKind::Status, why, None, None)?;
    tx.execute("UPDATE agent_run SET runner_id = NULL, runner_name = NULL, runner_user_id = NULL, runner_seen_at = NULL WHERE id = ?1", [id])?;

    if cancelled {
        finish(tx, id, AgentRunState::Cancelled)?;
    } else {
        set_state(tx, id, AgentRunState::Queued)?;
    }

    announce(tx, id)
}

/// Runs whose runner has not been heard from for ten minutes, neither on the run nor with a
/// heartbeat, go back to the queue for another runner. `now` is a parameter so that tests
/// need not wait.
pub fn requeue_stale(tx: &Tx, now: DateTime<Utc>) -> ApiResult<usize> {
    let cutoff = iso(now - Duration::minutes(STALE_AFTER_MINUTES));
    let stale: Vec<Id> = query_all(
        tx,
        "SELECT r.id FROM agent_run r LEFT JOIN agent_runner n ON n.id = r.runner_id
         WHERE r.state IN ('claimed', 'running') AND r.runner_id IS NOT NULL AND r.runner_id != ?1
           AND max(coalesce(r.runner_seen_at, ''), coalesce(n.last_seen_at, '')) < ?2",
        (SERVER_RUNNER, cutoff),
        |row| row.get(0),
    )?;

    for id in &stale {
        requeue(tx, *id, "The runner stopped answering; the run is queued again")?;
    }

    Ok(stale.len())
}

/// Runs the server was carrying out when it stopped start over.
pub fn requeue_interrupted(tx: &Tx) -> ApiResult<usize> {
    let interrupted: Vec<Id> =
        query_all(tx, "SELECT id FROM agent_run WHERE state IN ('claimed', 'running') AND runner_id = ?1", [SERVER_RUNNER], |row| row.get(0))?;

    for id in &interrupted {
        requeue(tx, *id, "The server restarted; the run is queued again")?;
    }

    Ok(interrupted.len())
}

/// The caller's own runners, and other people's that are online (those can take shared runs).
pub fn runners(connection: &Connection, user: &AuthUser, now: DateTime<Utc>) -> ApiResult<Vec<AgentRunner>> {
    let online_since = iso(now - Duration::seconds(ONLINE_FOR_SECONDS));
    let all = query_all(
        connection,
        "SELECT id, name, user_id, kinds, projects, version, last_seen_at FROM agent_runner ORDER BY last_seen_at DESC",
        [],
        |row| {
            let kinds: String = row.get(3)?;
            let projects: Option<String> = row.get(4)?;
            let last_seen_at: String = row.get(6)?;

            Ok(AgentRunner {
                id: row.get(0)?,
                name: row.get(1)?,
                user_id: row.get(2)?,
                kinds: serde_json::from_str::<Vec<String>>(&kinds).unwrap_or_default().iter().filter_map(|kind| AgentRunKind::parse(kind)).collect(),
                projects: projects.and_then(|projects| serde_json::from_str(&projects).ok()),
                version: row.get(5)?,
                online: last_seen_at >= online_since,
                last_seen_at,
            })
        },
    )?;

    Ok(all.into_iter().filter(|runner| runner.user_id == user.id || runner.online).collect())
}
