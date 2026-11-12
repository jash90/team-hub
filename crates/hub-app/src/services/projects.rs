use hub_core::keys::is_project_key;
use hub_core::workflow::{DEFAULT_TYPES, KANBAN_STATUSES, SCRUM_STATUSES};
use hub_core::{BoardMode, Component, Id, IssueType, Project, ProjectMember, ProjectRole, ProjectSummary, Status, StatusCategory, Transition, TypeLevel};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use crate::access::{self, visible_projects, ProjectAccess};
use crate::app::Tx;
use crate::auth::AuthUser;
use crate::db::{query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::{sprints, versions};

fn map_summary(row: &rusqlite::Row) -> rusqlite::Result<ProjectSummary> {
    let mode: String = row.get(5)?;

    Ok(ProjectSummary {
        id: row.get(0)?,
        key: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        lead_id: row.get(4)?,
        board_mode: BoardMode::parse(&mode).unwrap_or(BoardMode::Scrum),
        is_private: row.get(6)?,
        archived: row.get(7)?,
        role: None,
        issue_count: row.get(8)?,
        open_count: row.get(9)?,
    })
}

const SUMMARY_SELECT: &str = "SELECT p.id, p.key, p.name, p.description, p.lead_id, p.board_mode, p.is_private, p.archived,
        (SELECT count(*) FROM issue WHERE project_id = p.id),
        (SELECT count(*) FROM issue i JOIN status s ON s.id = i.status_id WHERE i.project_id = p.id AND s.category != 'done')
    FROM project p";

pub fn list_projects(connection: &Connection, user: &AuthUser) -> ApiResult<Vec<ProjectSummary>> {
    let (condition, params) = visible_projects(user, "p.id");
    let mut projects = query_all(
        connection,
        &format!("{SUMMARY_SELECT} WHERE {condition} ORDER BY p.archived, p.name COLLATE NOCASE"),
        rusqlite::params_from_iter(params),
        map_summary,
    )?;

    for project in &mut projects {
        project.role = Some(access::project_access_by_id(connection, user, project.id)?.role);
    }

    Ok(projects)
}

pub fn load_statuses(connection: &Connection, project_id: Id) -> ApiResult<Vec<Status>> {
    query_all(
        connection,
        "SELECT id, project_id, name, category, position, on_board, wip_limit FROM status WHERE project_id = ?1 ORDER BY position, id",
        [project_id],
        |row| {
            let category: String = row.get(3)?;

            Ok(Status {
                id: row.get(0)?,
                project_id: row.get(1)?,
                name: row.get(2)?,
                category: StatusCategory::parse(&category).unwrap_or(StatusCategory::Todo),
                position: row.get(4)?,
                on_board: row.get(5)?,
                wip_limit: row.get(6)?,
            })
        },
    )
}

pub fn load_types(connection: &Connection, project_id: Id) -> ApiResult<Vec<IssueType>> {
    query_all(
        connection,
        "SELECT id, project_id, name, level, icon, position FROM issue_type WHERE project_id = ?1 ORDER BY position, id",
        [project_id],
        |row| {
            let level: String = row.get(3)?;

            Ok(IssueType {
                id: row.get(0)?,
                project_id: row.get(1)?,
                name: row.get(2)?,
                level: TypeLevel::parse(&level).unwrap_or(TypeLevel::Standard),
                icon: row.get(4)?,
                position: row.get(5)?,
            })
        },
    )
}

pub fn load_transitions(connection: &Connection, project_id: Id) -> ApiResult<Vec<Transition>> {
    query_all(connection, "SELECT id, from_status_id, to_status_id, name FROM transition WHERE project_id = ?1 ORDER BY id", [project_id], |row| {
        Ok(Transition { id: row.get(0)?, from_status_id: row.get(1)?, to_status_id: row.get(2)?, name: row.get(3)? })
    })
}

pub fn load_components(connection: &Connection, project_id: Id) -> ApiResult<Vec<Component>> {
    query_all(
        connection,
        "SELECT id, project_id, name, description, lead_id FROM component WHERE project_id = ?1 ORDER BY name COLLATE NOCASE",
        [project_id],
        |row| Ok(Component { id: row.get(0)?, project_id: row.get(1)?, name: row.get(2)?, description: row.get(3)?, lead_id: row.get(4)? }),
    )
}

pub fn project_labels(connection: &Connection, project_id: Id) -> ApiResult<Vec<String>> {
    query_all(
        connection,
        "SELECT DISTINCT l.label FROM issue_label l JOIN issue i ON i.id = l.issue_id WHERE i.project_id = ?1 ORDER BY l.label COLLATE NOCASE",
        [project_id],
        |row| row.get(0),
    )
}

pub fn strict_workflow(connection: &Connection, project_id: Id) -> ApiResult<bool> {
    Ok(connection.query_row("SELECT strict_workflow FROM project WHERE id = ?1", [project_id], |row| row.get(0))?)
}

pub fn get_project(connection: &Connection, user: &AuthUser, key: &str) -> ApiResult<Project> {
    let access = access::project_access(connection, user, key)?;
    let mut summary = query_one(connection, &format!("{SUMMARY_SELECT} WHERE p.id = ?1"), [access.id], map_summary)?
        .ok_or_else(|| ApiError::not_found("Project"))?;
    summary.role = Some(access.role);

    let members = query_all(connection, "SELECT user_id, role FROM project_member WHERE project_id = ?1", [access.id], |row| {
        let role: String = row.get(1)?;

        Ok(ProjectMember { user_id: row.get(0)?, role: ProjectRole::parse(&role).unwrap_or(ProjectRole::Member) })
    })?;

    Ok(Project {
        strict_workflow: strict_workflow(connection, access.id)?,
        statuses: load_statuses(connection, access.id)?,
        types: load_types(connection, access.id)?,
        transitions: load_transitions(connection, access.id)?,
        components: load_components(connection, access.id)?,
        versions: versions::load_versions(connection, access.id)?,
        sprints: sprints::load_sprints(connection, access.id)?,
        members,
        labels: project_labels(connection, access.id)?,
        summary,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProject {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub board_mode: Option<BoardMode>,
    #[serde(default)]
    pub is_private: bool,
    pub lead_id: Option<Id>,
}

pub fn create_project(tx: &Tx, user: &AuthUser, new: &NewProject) -> ApiResult<Project> {
    let key = new.key.trim().to_uppercase();
    let name = new.name.trim();

    if !is_project_key(&key) {
        return Err(ApiError::bad_request("A project key is 2 to 10 letters and digits, starting with a letter"));
    }

    if name.is_empty() {
        return Err(ApiError::bad_request("The project needs a name"));
    }

    let taken: Option<Id> = query_one(tx, "SELECT id FROM project WHERE key = ?1", [&key], |row| row.get(0))?;

    if taken.is_some() {
        return Err(ApiError::conflict(format!("The key {key} is already taken")));
    }

    let mode = new.board_mode.unwrap_or(BoardMode::Scrum);

    tx.execute(
        "INSERT INTO project (key, name, description, lead_id, board_mode, is_private, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (&key, name, new.description.trim(), new.lead_id.unwrap_or(user.id), mode.as_str(), new.is_private, &tx.now),
    )?;
    let project_id = tx.last_insert_rowid();

    for (position, template) in DEFAULT_TYPES.iter().enumerate() {
        tx.execute(
            "INSERT INTO issue_type (project_id, name, level, icon, position) VALUES (?1, ?2, ?3, ?4, ?5)",
            (project_id, template.name, template.level.as_str(), template.icon, position as i64),
        )?;
    }

    let statuses: &[_] = if mode == BoardMode::Kanban { &KANBAN_STATUSES } else { &SCRUM_STATUSES };

    for (position, template) in statuses.iter().enumerate() {
        tx.execute(
            "INSERT INTO status (project_id, name, category, position, on_board) VALUES (?1, ?2, ?3, ?4, ?5)",
            (project_id, template.name, template.category.as_str(), position as i64, template.on_board),
        )?;
    }

    tx.execute("INSERT INTO project_member (project_id, user_id, role) VALUES (?1, ?2, 'admin')", (project_id, user.id))?;
    tx.emit("project.created", Some(project_id), None, json!({ "key": key, "name": name }))?;

    get_project(tx, user, &key)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub lead_id: Option<Id>,
    pub board_mode: Option<BoardMode>,
    pub is_private: Option<bool>,
    pub archived: Option<bool>,
    pub strict_workflow: Option<bool>,
}

pub fn update_project(tx: &Tx, access: &ProjectAccess, patch: &ProjectPatch) -> ApiResult<()> {
    access.require_admin()?;

    if let Some(name) = patch.name.as_deref().map(str::trim) {
        if name.is_empty() {
            return Err(ApiError::bad_request("The project needs a name"));
        }

        tx.execute("UPDATE project SET name = ?1 WHERE id = ?2", (name, access.id))?;
    }

    if let Some(description) = &patch.description {
        tx.execute("UPDATE project SET description = ?1 WHERE id = ?2", (description.trim(), access.id))?;
    }

    if let Some(lead_id) = patch.lead_id {
        tx.execute("UPDATE project SET lead_id = ?1 WHERE id = ?2", (lead_id, access.id))?;
    }

    if let Some(mode) = patch.board_mode {
        tx.execute("UPDATE project SET board_mode = ?1 WHERE id = ?2", (mode.as_str(), access.id))?;
    }

    if let Some(is_private) = patch.is_private {
        tx.execute("UPDATE project SET is_private = ?1 WHERE id = ?2", (is_private, access.id))?;
    }

    if let Some(archived) = patch.archived {
        tx.execute("UPDATE project SET archived = ?1 WHERE id = ?2", (archived, access.id))?;
    }

    if let Some(strict) = patch.strict_workflow {
        tx.execute("UPDATE project SET strict_workflow = ?1 WHERE id = ?2", (strict, access.id))?;
    }

    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}

fn clean_label(name: &str, what: &str) -> ApiResult<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");

    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad_request(format!("{what} needs a name of 1 to 60 characters")));
    }

    Ok(name)
}

fn next_position(connection: &Connection, table: &str, project_id: Id) -> ApiResult<i64> {
    Ok(connection.query_row(&format!("SELECT coalesce(max(position), -1) + 1 FROM {table} WHERE project_id = ?1"), [project_id], |row| row.get(0))?)
}

fn owned_by(connection: &Connection, table: &str, id: Id, project_id: Id, what: &str) -> ApiResult<()> {
    let found: Option<Id> =
        query_one(connection, &format!("SELECT id FROM {table} WHERE id = ?1 AND project_id = ?2"), (id, project_id), |row| row.get(0))?;

    found.map(|_| ()).ok_or_else(|| ApiError::not_found(what))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatusInput {
    pub name: Option<String>,
    pub category: Option<StatusCategory>,
    pub on_board: Option<bool>,
    #[serde(deserialize_with = "hub_core::double_option")]
    pub wip_limit: Option<Option<i64>>,
    /// Where to put it among the project's statuses, counted from zero.
    pub position: Option<i64>,
}

fn reposition(connection: &Connection, table: &str, project_id: Id, id: Id, position: i64) -> ApiResult<()> {
    let mut ids: Vec<Id> =
        query_all(connection, &format!("SELECT id FROM {table} WHERE project_id = ?1 AND id != ?2 ORDER BY position, id"), (project_id, id), |row| row.get(0))?;
    let index = (position.max(0) as usize).min(ids.len());
    ids.insert(index, id);

    for (position, id) in ids.into_iter().enumerate() {
        connection.execute(&format!("UPDATE {table} SET position = ?1 WHERE id = ?2"), (position as i64, id))?;
    }

    Ok(())
}

pub fn add_status(tx: &Tx, access: &ProjectAccess, input: &StatusInput) -> ApiResult<Id> {
    access.require_admin()?;
    let name = clean_label(input.name.as_deref().unwrap_or(""), "A status")?;

    tx.execute(
        "INSERT INTO status (project_id, name, category, position, on_board, wip_limit) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            access.id,
            name,
            input.category.unwrap_or(StatusCategory::InProgress).as_str(),
            next_position(tx, "status", access.id)?,
            input.on_board.unwrap_or(true),
            input.wip_limit.flatten(),
        ),
    )?;
    let id = tx.last_insert_rowid();

    if let Some(position) = input.position {
        reposition(tx, "status", access.id, id, position)?;
    }

    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))?;

    Ok(id)
}

pub fn update_status(tx: &Tx, access: &ProjectAccess, id: Id, input: &StatusInput) -> ApiResult<()> {
    access.require_admin()?;
    owned_by(tx, "status", id, access.id, "Status")?;

    if let Some(name) = &input.name {
        tx.execute("UPDATE status SET name = ?1 WHERE id = ?2", (clean_label(name, "A status")?, id))?;
    }

    if let Some(category) = input.category {
        tx.execute("UPDATE status SET category = ?1 WHERE id = ?2", (category.as_str(), id))?;

        // Issues already in this status follow its new meaning.
        if category == StatusCategory::Done {
            tx.execute("UPDATE issue SET resolved_at = ?1 WHERE status_id = ?2 AND resolved_at IS NULL", (&tx.now, id))?;
        } else {
            tx.execute("UPDATE issue SET resolved_at = NULL WHERE status_id = ?1", [id])?;
        }
    }

    if let Some(on_board) = input.on_board {
        tx.execute("UPDATE status SET on_board = ?1 WHERE id = ?2", (on_board, id))?;
    }

    if let Some(limit) = input.wip_limit {
        tx.execute("UPDATE status SET wip_limit = ?1 WHERE id = ?2", (limit.filter(|limit| *limit > 0), id))?;
    }

    if let Some(position) = input.position {
        reposition(tx, "status", access.id, id, position)?;
    }

    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}

/// Deletes a status; its issues move to `move_to`.
pub fn delete_status(tx: &Tx, access: &ProjectAccess, id: Id, move_to: Option<Id>) -> ApiResult<()> {
    access.require_admin()?;
    owned_by(tx, "status", id, access.id, "Status")?;

    let in_use: i64 = tx.query_row("SELECT count(*) FROM issue WHERE status_id = ?1", [id], |row| row.get(0))?;

    if in_use > 0 {
        let target = move_to.filter(|target| *target != id).ok_or_else(|| ApiError::conflict("Choose a status for the issues that are in this one"))?;
        owned_by(tx, "status", target, access.id, "Status")?;
        let done: bool = tx.query_row("SELECT category = 'done' FROM status WHERE id = ?1", [target], |row| row.get(0))?;

        tx.execute("UPDATE issue SET status_id = ?1, status_since = ?2, updated_at = ?2 WHERE status_id = ?3", (target, &tx.now, id))?;

        if done {
            tx.execute("UPDATE issue SET resolved_at = ?1 WHERE status_id = ?2 AND resolved_at IS NULL", (&tx.now, target))?;
        } else {
            tx.execute("UPDATE issue SET resolved_at = NULL WHERE status_id = ?1", [target])?;
        }
    }

    let remaining: i64 = tx.query_row("SELECT count(*) FROM status WHERE project_id = ?1", [access.id], |row| row.get(0))?;

    if remaining <= 1 {
        return Err(ApiError::conflict("A project needs at least one status"));
    }

    tx.execute("DELETE FROM status WHERE id = ?1", [id])?;
    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TypeInput {
    pub name: Option<String>,
    pub level: Option<TypeLevel>,
    pub icon: Option<String>,
}

pub fn add_type(tx: &Tx, access: &ProjectAccess, input: &TypeInput) -> ApiResult<Id> {
    access.require_admin()?;
    let name = clean_label(input.name.as_deref().unwrap_or(""), "An issue type")?;

    tx.execute(
        "INSERT INTO issue_type (project_id, name, level, icon, position) VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            access.id,
            name,
            input.level.unwrap_or(TypeLevel::Standard).as_str(),
            input.icon.as_deref().unwrap_or("task"),
            next_position(tx, "issue_type", access.id)?,
        ),
    )?;
    let id = tx.last_insert_rowid();
    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))?;

    Ok(id)
}

pub fn update_type(tx: &Tx, access: &ProjectAccess, id: Id, input: &TypeInput) -> ApiResult<()> {
    access.require_admin()?;
    owned_by(tx, "issue_type", id, access.id, "Issue type")?;

    if let Some(name) = &input.name {
        tx.execute("UPDATE issue_type SET name = ?1 WHERE id = ?2", (clean_label(name, "An issue type")?, id))?;
    }

    if let Some(icon) = &input.icon {
        tx.execute("UPDATE issue_type SET icon = ?1 WHERE id = ?2", (icon, id))?;
    }

    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}

pub fn delete_type(tx: &Tx, access: &ProjectAccess, id: Id, move_to: Option<Id>) -> ApiResult<()> {
    access.require_admin()?;
    owned_by(tx, "issue_type", id, access.id, "Issue type")?;

    let in_use: i64 = tx.query_row("SELECT count(*) FROM issue WHERE type_id = ?1", [id], |row| row.get(0))?;

    if in_use > 0 {
        let target = move_to.filter(|target| *target != id).ok_or_else(|| ApiError::conflict("Choose a type for the issues that have this one"))?;
        owned_by(tx, "issue_type", target, access.id, "Issue type")?;
        tx.execute("UPDATE issue SET type_id = ?1 WHERE type_id = ?2", (target, id))?;
    }

    tx.execute("DELETE FROM issue_type WHERE id = ?1", [id])?;
    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComponentInput {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(deserialize_with = "hub_core::double_option")]
    pub lead_id: Option<Option<Id>>,
}

pub fn add_component(tx: &Tx, access: &ProjectAccess, input: &ComponentInput) -> ApiResult<Id> {
    access.require_admin()?;
    let name = clean_label(input.name.as_deref().unwrap_or(""), "A component")?;

    tx.execute(
        "INSERT INTO component (project_id, name, description, lead_id) VALUES (?1, ?2, ?3, ?4)",
        (access.id, name, input.description.as_deref().unwrap_or("").trim(), input.lead_id.flatten()),
    )?;
    let id = tx.last_insert_rowid();
    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))?;

    Ok(id)
}

pub fn update_component(tx: &Tx, access: &ProjectAccess, id: Id, input: &ComponentInput) -> ApiResult<()> {
    access.require_admin()?;
    owned_by(tx, "component", id, access.id, "Component")?;

    if let Some(name) = &input.name {
        tx.execute("UPDATE component SET name = ?1 WHERE id = ?2", (clean_label(name, "A component")?, id))?;
    }

    if let Some(description) = &input.description {
        tx.execute("UPDATE component SET description = ?1 WHERE id = ?2", (description.trim(), id))?;
    }

    if let Some(lead_id) = input.lead_id {
        tx.execute("UPDATE component SET lead_id = ?1 WHERE id = ?2", (lead_id, id))?;
    }

    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}

pub fn delete_component(tx: &Tx, access: &ProjectAccess, id: Id) -> ApiResult<()> {
    access.require_admin()?;
    owned_by(tx, "component", id, access.id, "Component")?;
    tx.execute("DELETE FROM component WHERE id = ?1", [id])?;
    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}

/// Sets a person's role in the project; no role removes them.
pub fn set_member(tx: &Tx, access: &ProjectAccess, user_id: Id, role: Option<ProjectRole>) -> ApiResult<()> {
    access.require_admin()?;

    match role {
        Some(role) => {
            tx.execute(
                "INSERT INTO project_member (project_id, user_id, role) VALUES (?1, ?2, ?3)
                 ON CONFLICT (project_id, user_id) DO UPDATE SET role = ?3",
                (access.id, user_id, role.as_str()),
            )?;
        }
        None => {
            tx.execute("DELETE FROM project_member WHERE project_id = ?1 AND user_id = ?2", (access.id, user_id))?;
        }
    }

    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionInput {
    pub from_status_id: Option<Id>,
    pub to_status_id: Id,
    #[serde(default)]
    pub name: String,
}

/// Replaces the project's whole list of allowed transitions.
pub fn replace_transitions(tx: &Tx, access: &ProjectAccess, transitions: &[TransitionInput]) -> ApiResult<()> {
    access.require_admin()?;
    tx.execute("DELETE FROM transition WHERE project_id = ?1", [access.id])?;

    for transition in transitions {
        owned_by(tx, "status", transition.to_status_id, access.id, "Status")?;

        if let Some(from) = transition.from_status_id {
            owned_by(tx, "status", from, access.id, "Status")?;
        }

        tx.execute(
            "INSERT INTO transition (project_id, from_status_id, to_status_id, name) VALUES (?1, ?2, ?3, ?4)",
            (access.id, transition.from_status_id, transition.to_status_id, transition.name.trim()),
        )?;
    }

    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key }))
}
