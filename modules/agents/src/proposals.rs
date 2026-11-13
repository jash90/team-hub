//! Proposals: what an agent would like to change. They are checked when they arrive, shown
//! to a person with a preview of their effect, and applied, through the ordinary services
//! and as the person who approved, only then.

use std::cell::Cell;

use hub_app::access::project_access;
use hub_app::app::{get_setting, set_setting};
use hub_app::db::{query_all, query_one};
use hub_app::services::history::clean_date;
use hub_app::services::issues;
use hub_app::services::projects::{load_transitions, strict_workflow};
use hub_app::services::users::{get_user, user_name};
use hub_app::{Actor, ApiError, ApiResult, App, AuthUser, Tx};
use hub_core::diff::diff_lines;
use hub_core::text::{format_duration, parse_duration};
use hub_core::workflow::can_transition;
use hub_core::{double_option, priority_name, Id, IssuePatch, NewIssue};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::names;
use crate::runs::{self, RunRow, AI_LABEL};
use crate::types::{
    AgentEventKind, AgentFieldChange, AgentProposal, AgentProposalKind, AgentProposalPreview, AgentProposalState, AgentRunDetail, AgentRunState,
    AgentTrust,
};

const MAX_SUBTASKS: usize = 30;

#[derive(Debug, Deserialize, Serialize)]
struct CommentPayload {
    body: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct TransitionPayload {
    status: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
struct FieldsPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<String>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    points: Option<Option<f64>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    assignee: Option<Option<String>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    due_date: Option<Option<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SubtaskItem {
    summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SubtasksPayload {
    items: Vec<SubtaskItem>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct IssuePayload {
    project_key: String,
    summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    issue_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct WorkPayload {
    spent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

fn parse<T: for<'de> Deserialize<'de>>(payload: &Value) -> ApiResult<T> {
    T::deserialize(payload).map_err(|error| ApiError::bad_request(format!("The proposal's payload is not what its kind takes: {error}")))
}

fn filled(text: &str, what: &str) -> ApiResult<String> {
    let text = text.trim();

    if text.is_empty() {
        return Err(ApiError::bad_request(format!("{what} is empty")));
    }

    Ok(text.to_string())
}

fn blank_as_none(text: &Option<Option<String>>) -> Option<Option<&str>> {
    text.as_ref().map(|text| text.as_deref().map(str::trim).filter(|text| !text.is_empty()))
}

/// The person a run was requested by, as a caller: proposals are checked against what they may do.
pub fn person(connection: &Connection, id: Id) -> ApiResult<AuthUser> {
    let user = get_user(connection, id)?;

    if !user.active {
        return Err(ApiError::forbidden("The person who asked for this run is no longer active"));
    }

    Ok(AuthUser { id: user.id, name: user.name, email: user.email, role: user.role, via_token: false })
}

fn issue_level(connection: &Connection, issue_id: Id) -> ApiResult<String> {
    Ok(connection.query_row("SELECT t.level FROM issue i JOIN issue_type t ON t.id = i.type_id WHERE i.id = ?1", [issue_id], |row| row.get(0))?)
}

/// The type a new issue gets when none is named: the one `create_issue` would choose.
fn default_type(connection: &Connection, project_id: Id, level: &str) -> ApiResult<Option<String>> {
    query_one(
        connection,
        "SELECT name FROM issue_type WHERE project_id = ?1 AND level = ?2 ORDER BY name = 'Task' DESC, position LIMIT 1",
        (project_id, level),
        |row| row.get(0),
    )
}

/// Checks a payload against the issue's project and returns it tidied: names as the tracker
/// spells them, blanks removed. Anything that could not be applied is refused here, with a
/// message the agent can act on.
fn check(connection: &Connection, row: &RunRow, kind: AgentProposalKind, payload: &Value) -> ApiResult<Value> {
    Ok(match kind {
        AgentProposalKind::Comment => json!(CommentPayload { body: filled(&parse::<CommentPayload>(payload)?.body, "The comment")? }),
        AgentProposalKind::Transition => {
            let status_id = names::status_id(connection, row.project_id, &parse::<TransitionPayload>(payload)?.status)?;
            let status: String = connection.query_row("SELECT name FROM status WHERE id = ?1", [status_id], |row| row.get(0))?;

            json!(TransitionPayload { status })
        }
        AgentProposalKind::UpdateFields => {
            let mut fields: FieldsPayload = parse(payload)?;

            if let Some(summary) = &fields.summary {
                fields.summary = Some(filled(summary, "The summary")?);
            }

            if let Some(priority) = &fields.priority {
                fields.priority = Some(json!(priority_name(names::priority(priority)?)));
            }

            if let Some(points) = fields.points.flatten() {
                if !(0.0..=1000.0).contains(&points) {
                    return Err(ApiError::bad_request("Story points go from 0 to 1000"));
                }
            }

            let assignee = match blank_as_none(&fields.assignee) {
                Some(Some(name)) => {
                    let requester = person(connection, row.run.requested_by)?;

                    Some(user_name(connection, Some(names::user_id(connection, &requester, name)?))?)
                }
                Some(None) => Some(None),
                None => None,
            };
            let due_date = blank_as_none(&fields.due_date).map(clean_date).transpose()?;
            fields.assignee = assignee;
            fields.due_date = due_date;

            let value = json!(fields);

            if value.as_object().is_none_or(|fields| fields.is_empty()) {
                return Err(ApiError::bad_request(
                    "The proposal changes no field (it may set summary, description, priority, labels, points, assignee, dueDate)",
                ));
            }

            value
        }
        AgentProposalKind::CreateSubtasks => {
            let mut subtasks: SubtasksPayload = parse(payload)?;

            if subtasks.items.is_empty() || subtasks.items.len() > MAX_SUBTASKS {
                return Err(ApiError::bad_request(format!("A breakdown has 1 to {MAX_SUBTASKS} items")));
            }

            if issue_level(connection, row.issue_id)? == "subtask" {
                return Err(ApiError::bad_request(format!("{} is a subtask itself and cannot have subtasks", row.run.issue_key)));
            }

            for item in &mut subtasks.items {
                item.summary = filled(&item.summary, "A subtask's summary")?;
            }

            json!(subtasks)
        }
        AgentProposalKind::CreateIssue => {
            let mut issue: IssuePayload = parse(payload)?;
            let requester = person(connection, row.run.requested_by)?;
            let access = project_access(connection, &requester, &issue.project_key)?;
            access.require_edit()?;
            issue.project_key = access.key.clone();
            issue.summary = filled(&issue.summary, "The summary")?;

            if let Some(name) = &issue.issue_type {
                names::type_id(connection, access.id, name)?;
            }

            if let Some(parent) = &issue.parent_key {
                issues::issue_id_by_key(connection, parent)?.ok_or_else(|| ApiError::bad_request(format!("{parent} does not exist")))?;
            }

            json!(issue)
        }
        AgentProposalKind::LogWork => {
            let work: WorkPayload = parse(payload)?;
            let minutes = parse_duration(&work.spent)
                .filter(|minutes| *minutes > 0)
                .ok_or_else(|| ApiError::bad_request("Time spent looks like \"1h 30m\""))?;

            json!(WorkPayload { spent: format_duration(minutes), note: work.note })
        }
    })
}

fn change(field: &str, before: Option<String>, after: Option<String>) -> Option<AgentFieldChange> {
    (before != after).then(|| AgentFieldChange { field: field.to_string(), before, after })
}

/// What the proposal would do to the issue as it is now.
fn preview(connection: &Connection, row: &RunRow, kind: AgentProposalKind, payload: &Value) -> ApiResult<AgentProposalPreview> {
    let mut preview = AgentProposalPreview::default();

    match kind {
        AgentProposalKind::Transition => {
            let wanted: TransitionPayload = parse(payload)?;
            let (current, name): (Id, String) = connection.query_row(
                "SELECT s.id, s.name FROM issue i JOIN status s ON s.id = i.status_id WHERE i.id = ?1",
                [row.issue_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let target = names::status_id(connection, row.project_id, &wanted.status)?;

            preview.allowed =
                Some(can_transition(strict_workflow(connection, row.project_id)?, &load_transitions(connection, row.project_id)?, current, target));
            preview.changes.extend(change("status", Some(name), Some(wanted.status)));
        }
        AgentProposalKind::UpdateFields => {
            let fields: FieldsPayload = parse(payload)?;
            let (summary, description, priority, points, assignee, due_date): (String, String, i64, Option<f64>, Option<Id>, Option<String>) =
                connection.query_row(
                    "SELECT summary, description, priority, points, assignee_id, due_date FROM issue WHERE id = ?1",
                    [row.issue_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
                )?;
            let labels: Vec<String> =
                query_all(connection, "SELECT label FROM issue_label WHERE issue_id = ?1 ORDER BY label COLLATE NOCASE", [row.issue_id], |row| {
                    row.get(0)
                })?;
            let number = |points: Option<f64>| points.map(|points| points.to_string());

            if let Some(after) = fields.summary {
                preview.changes.extend(change("summary", Some(summary), Some(after)));
            }

            if let Some(after) = fields.priority.as_ref().and_then(Value::as_str) {
                preview.changes.extend(change("priority", Some(priority_name(priority).to_string()), Some(after.to_string())));
            }

            if let Some(after) = fields.labels {
                preview.changes.extend(change(
                    "labels",
                    Some(labels.join(", ")).filter(|text| !text.is_empty()),
                    Some(after.join(", ")).filter(|text| !text.is_empty()),
                ));
            }

            if let Some(after) = fields.points {
                preview.changes.extend(change("points", number(points), number(after)));
            }

            if let Some(after) = fields.assignee {
                preview.changes.extend(change("assignee", user_name(connection, assignee)?, after));
            }

            if let Some(after) = fields.due_date {
                preview.changes.extend(change("dueDate", due_date, after));
            }

            if let Some(after) = fields.description.filter(|after| *after != description) {
                preview.description_diff = Some(diff_lines(&description, &after).into_iter().map(Into::into).collect());
            }
        }
        AgentProposalKind::CreateSubtasks => {
            let level = if issue_level(connection, row.issue_id)? == "epic" { "standard" } else { "subtask" };
            preview.issue_type = default_type(connection, row.project_id, level)?;
        }
        AgentProposalKind::CreateIssue => {
            let issue: IssuePayload = parse(payload)?;
            let project_id: Option<Id> = query_one(connection, "SELECT id FROM project WHERE key = ?1", [&issue.project_key], |row| row.get(0))?;

            preview.issue_type = match (issue.issue_type, project_id) {
                (Some(name), _) => Some(name),
                (None, Some(project_id)) => default_type(connection, project_id, "standard")?,
                (None, None) => None,
            };
        }
        AgentProposalKind::Comment | AgentProposalKind::LogWork => {}
    }

    Ok(preview)
}

/// Carries a proposal out inside the approver's transaction, and says what came of it.
fn apply(tx: &Tx, approver: &AuthUser, row: &RunRow, kind: AgentProposalKind, payload: &Value) -> ApiResult<Value> {
    let issue_id = row.issue_id;

    Ok(match kind {
        AgentProposalKind::Comment => json!({ "commentId": issues::add_comment(tx, issue_id, &parse::<CommentPayload>(payload)?.body)?.id }),
        AgentProposalKind::Transition => {
            let status_id = names::status_id(tx, row.project_id, &parse::<TransitionPayload>(payload)?.status)?;

            json!({ "changed": issues::update_issue(tx, issue_id, &IssuePatch { status_id: Some(status_id), ..Default::default() })? })
        }
        AgentProposalKind::UpdateFields => {
            let fields: FieldsPayload = parse(payload)?;
            let before: String = tx.query_row("SELECT description FROM issue WHERE id = ?1", [issue_id], |row| row.get(0))?;
            let patch = IssuePatch {
                summary: fields.summary.clone(),
                description: fields.description.clone(),
                priority: fields.priority.as_ref().map(names::priority).transpose()?,
                labels: fields.labels.clone(),
                points: fields.points,
                assignee_id: match blank_as_none(&fields.assignee) {
                    Some(Some(name)) => Some(Some(names::user_id(tx, approver, name)?)),
                    Some(None) => Some(None),
                    None => None,
                },
                due_date: fields.due_date.clone(),
                ..Default::default()
            };
            let changed = issues::update_issue(tx, issue_id, &patch)?;

            // The issue's history keeps only the start of an old description, so the whole
            // text stays here, where it can be read and put back.
            if changed.contains(&"description") {
                json!({ "changed": changed, "previousDescription": before })
            } else {
                json!({ "changed": changed })
            }
        }
        AgentProposalKind::CreateSubtasks => {
            let subtasks: SubtasksPayload = parse(payload)?;
            // Subtasks belong to their parent's sprint, as long as that sprint can still take issues.
            let sprint: Option<Id> = query_one(
                tx,
                "SELECT s.id FROM issue i JOIN sprint s ON s.id = i.sprint_id WHERE i.id = ?1 AND s.state != 'closed'",
                [issue_id],
                |row| row.get(0),
            )?;
            let mut keys = Vec::new();

            for item in subtasks.items {
                let fields = IssuePatch {
                    summary: Some(item.summary),
                    description: item.description,
                    parent_key: Some(Some(row.run.issue_key.clone())),
                    sprint_id: sprint.map(Some),
                    ..Default::default()
                };
                let id = issues::create_issue(tx, &NewIssue { project_key: row.run.project_key.clone(), fields })?;

                keys.push(tx.query_row("SELECT key FROM issue WHERE id = ?1", [id], |row| row.get::<_, String>(0))?);
            }

            json!({ "keys": keys })
        }
        AgentProposalKind::CreateIssue => {
            let issue: IssuePayload = parse(payload)?;
            let access = project_access(tx, approver, &issue.project_key)?;
            access.require_edit()?;
            let fields = IssuePatch {
                summary: Some(issue.summary),
                description: issue.description,
                type_id: issue.issue_type.as_deref().map(|name| names::type_id(tx, access.id, name)).transpose()?,
                parent_key: issue.parent_key.map(Some),
                ..Default::default()
            };
            let id = issues::create_issue(tx, &NewIssue { project_key: access.key, fields })?;

            json!({ "key": tx.query_row("SELECT key FROM issue WHERE id = ?1", [id], |row| row.get::<_, String>(0))? })
        }
        AgentProposalKind::LogWork => {
            let work: WorkPayload = parse(payload)?;
            let minutes = parse_duration(&work.spent).ok_or_else(|| ApiError::bad_request("Time spent looks like \"1h 30m\""))?;

            json!({ "worklogId": issues::add_worklog(tx, issue_id, minutes, None, work.note.as_deref().unwrap_or(""))?.id })
        }
    })
}

struct Stored {
    proposal: AgentProposal,
    preview: Option<String>,
}

fn stored(connection: &Connection, condition: &str, id: Id) -> ApiResult<Vec<Stored>> {
    query_all(
        connection,
        &format!("SELECT id, run_id, kind, payload, state, decided_by, decided_at, note, result, preview FROM agent_proposal WHERE {condition} ORDER BY id"),
        [id],
        |row| {
            let kind: String = row.get(2)?;
            let payload: String = row.get(3)?;
            let state: String = row.get(4)?;
            let result: Option<String> = row.get(8)?;

            Ok(Stored {
                proposal: AgentProposal {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    kind: AgentProposalKind::parse(&kind).unwrap_or(AgentProposalKind::Comment),
                    payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                    state: AgentProposalState::parse(&state).unwrap_or(AgentProposalState::Failed),
                    decided_by: row.get(5)?,
                    decided_at: row.get(6)?,
                    note: row.get(7)?,
                    result: result.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null),
                    preview: None,
                },
                preview: row.get(9)?,
            })
        },
    )
}

/// An undecided proposal is previewed against the issue as it is now; a decided one keeps
/// the picture it was decided on.
fn with_preview(connection: &Connection, row: &RunRow, stored: Stored) -> AgentProposal {
    let mut proposal = stored.proposal;
    let kept = stored.preview.and_then(|text| serde_json::from_str(&text).ok());

    proposal.preview = match proposal.state {
        AgentProposalState::Pending => preview(connection, row, proposal.kind, &proposal.payload).ok().or(kept),
        _ => kept,
    };

    proposal
}

pub fn list(connection: &Connection, row: &RunRow) -> ApiResult<Vec<AgentProposal>> {
    Ok(stored(connection, "run_id = ?1", row.run.id)?.into_iter().map(|stored| with_preview(connection, row, stored)).collect())
}

pub fn reject_pending(tx: &Tx, run_id: Id, note: &str) -> ApiResult<()> {
    tx.execute(
        "UPDATE agent_proposal SET state = 'rejected', note = ?1, decided_at = ?2 WHERE run_id = ?3 AND state = 'pending'",
        (note, &tx.now, run_id),
    )?;

    Ok(())
}

fn trust_setting(project_id: Id) -> String {
    format!("agents:trust:{project_id}")
}

/// The proposal kinds this project applies without waiting for a person. None by default.
pub fn trust(connection: &Connection, project_id: Id) -> ApiResult<AgentTrust> {
    let kinds: Vec<String> = get_setting(connection, &trust_setting(project_id))?.unwrap_or_default();

    Ok(AgentTrust { kinds: kinds.iter().filter_map(|kind| AgentProposalKind::parse(kind)).collect() })
}

pub fn set_trust(connection: &Connection, project_id: Id, trust: &AgentTrust) -> ApiResult<()> {
    let kinds: Vec<&str> = AgentProposalKind::ALL.iter().filter(|kind| trust.kinds.contains(kind)).map(|kind| kind.as_str()).collect();

    set_setting(connection, &trust_setting(project_id), &kinds)
}

fn kind_words(kind: AgentProposalKind) -> &'static str {
    match kind {
        AgentProposalKind::Comment => "a comment",
        AgentProposalKind::Transition => "a change of status",
        AgentProposalKind::UpdateFields => "changes to the issue's fields",
        AgentProposalKind::CreateSubtasks => "subtasks",
        AgentProposalKind::CreateIssue => "a new issue",
        AgentProposalKind::LogWork => "a work log",
    }
}

/// Records a proposal for the run in `tx`. It is refused, with the reason, when it could not
/// be applied as it stands.
pub fn add(tx: &Tx, row: &RunRow, kind: AgentProposalKind, payload: &Value) -> ApiResult<Id> {
    let payload = check(tx, row, kind, payload)?;
    let preview = preview(tx, row, kind, &payload)?;

    tx.execute(
        "INSERT INTO agent_proposal (run_id, kind, payload, preview, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        (row.run.id, kind.as_str(), payload.to_string(), serde_json::to_string(&preview)?, &tx.now),
    )?;
    let id = tx.last_insert_rowid();

    // A runner elsewhere now waits for the decision; the server's own run says so when it is through.
    if !row.on_server() && !row.run.state.is_finished() {
        tx.execute("UPDATE agent_run SET state = 'waiting' WHERE id = ?1", [row.run.id])?;
    }

    runs::add_event(tx, row.run.id, AgentEventKind::Tool, &format!("Proposed {}", kind_words(kind)), Some(&json!({ "proposalId": id })), None)?;
    runs::tell_requester(tx, row, &format!("the agent proposes {}", kind_words(kind)))?;
    runs::announce(tx, row.run.id)?;

    Ok(id)
}

/// Records a proposal and, when the project trusts its kind, applies it at once on behalf
/// of the person who asked for the run.
pub fn submit(app: &App, actor: Actor, run_id: Id, kind: AgentProposalKind, payload: &Value) -> ApiResult<AgentProposal> {
    let (id, row, trusted) = app.write(actor, |tx| {
        let row = runs::load(tx, run_id)?;
        let id = add(tx, &row, kind, payload)?;
        let trusted = trust(tx, row.project_id)?.kinds.contains(&kind);

        Ok((id, row, trusted))
    })?;

    if trusted {
        let requester = app.read(|connection| person(connection, row.run.requested_by));
        let decision = Decision { approve: true, note: Some("Applied at once: the project trusts this kind of proposal".to_string()), payload: None };

        match requester {
            Ok(requester) => {
                decide(app, &requester, id, &decision)?;
            }
            Err(error) => tracing::warn!("proposal {id} was not applied at once: {}", error.message),
        }
    }

    app.read(|connection| {
        let row = runs::load(connection, run_id)?;

        stored(connection, "id = ?1", id)?.pop().map(|stored| with_preview(connection, &row, stored)).ok_or_else(|| ApiError::not_found("Proposal"))
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Decision {
    pub approve: bool,
    pub note: Option<String>,
    /// The proposal as the person edited it before approving (not in the contract; optional).
    pub payload: Option<Value>,
}

fn decided(
    tx: &Tx,
    row: &RunRow,
    proposal: &AgentProposal,
    state: AgentProposalState,
    note: Option<&str>,
    result: Option<&Value>,
    words: &str,
) -> ApiResult<()> {
    tx.execute(
        "UPDATE agent_proposal SET state = ?1, decided_by = ?2, decided_at = ?3, note = ?4, result = ?5 WHERE id = ?6",
        (state.as_str(), tx.actor.user_id, &tx.now, note, result.map(Value::to_string), proposal.id),
    )?;
    runs::add_event(tx, row.run.id, AgentEventKind::Status, words, Some(&json!({ "proposalId": proposal.id, "state": state })), tx.actor.user_id)?;
    runs::settle(tx, row.run.id)?;

    runs::announce(tx, row.run.id)
}

/// A person's decision on a proposal. Approving applies it as that person, labelled as the
/// agent's work in the history; if it cannot be applied, nothing changes and the proposal
/// is marked failed with the reason.
pub fn decide(app: &App, user: &AuthUser, id: Id, decision: &Decision) -> ApiResult<AgentRunDetail> {
    let actor = || Actor::user_via(user.id, AI_LABEL);
    let note = decision.note.as_deref().map(str::trim).filter(|note| !note.is_empty());
    let applying = Cell::new(false);
    let found = Cell::new(None);

    let outcome = app.write(actor(), |tx| {
        let proposal = stored(tx, "id = ?1", id)?.pop().ok_or_else(|| ApiError::not_found("Proposal"))?.proposal;
        let (row, access) = runs::run_access(tx, user, proposal.run_id).map_err(|_| ApiError::not_found("Proposal"))?;
        access.require_edit()?;
        found.set(Some(row.run.id));

        if proposal.state != AgentProposalState::Pending {
            return Err(ApiError::conflict("This proposal has already been decided"));
        }

        if row.run.state == AgentRunState::Cancelled {
            return Err(ApiError::conflict("The run was cancelled"));
        }

        if !decision.approve {
            return decided(tx, &row, &proposal, AgentProposalState::Rejected, note, None, &format!("Rejected: {}", kind_words(proposal.kind)));
        }

        let payload = match &decision.payload {
            Some(edited) => check(tx, &row, proposal.kind, edited)?,
            None => proposal.payload.clone(),
        };
        let preview = preview(tx, &row, proposal.kind, &payload)?;

        tx.execute(
            "UPDATE agent_proposal SET payload = ?1, preview = ?2 WHERE id = ?3",
            (payload.to_string(), serde_json::to_string(&preview)?, id),
        )?;
        applying.set(true);
        let result = apply(tx, user, &row, proposal.kind, &payload)?;

        decided(tx, &row, &proposal, AgentProposalState::Approved, note, Some(&result), &format!("Approved: {}", kind_words(proposal.kind)))
    });

    match (outcome, found.get()) {
        (Ok(()), Some(run_id)) => app.read(|connection| runs::detail(connection, run_id)),
        // The transaction that tried to apply it was rolled back whole; this one records why.
        (Err(error), Some(run_id)) if applying.get() => app.write(actor(), |tx| {
            let proposal = stored(tx, "id = ?1", id)?.pop().ok_or_else(|| ApiError::not_found("Proposal"))?.proposal;
            let row = runs::load(tx, run_id)?;

            decided(
                tx,
                &row,
                &proposal,
                AgentProposalState::Failed,
                Some(&error.message),
                None,
                &format!("Could not apply {}: {}", kind_words(proposal.kind), error.message),
            )?;

            runs::detail(tx, run_id)
        }),
        (Err(error), _) => Err(error),
        (Ok(()), None) => Err(ApiError::not_found("Proposal")),
    }
}
