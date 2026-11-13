//! Spaces, and who may see and change them.
//!
//! A space bound to a project follows that project: invisible when the project is, editable
//! by those who may edit the project, deletable by its administrators. A space bound to
//! nothing belongs to everyone signed in; only an instance administrator may delete it.

use axum::http::StatusCode;
use hub_app::access::{project_access, project_access_by_id, visible_projects, ProjectAccess};
use hub_app::db::{query_all, query_one};
use hub_app::{ApiError, ApiResult, AuthUser, Tx};
use hub_core::keys::is_project_key;
use hub_core::{Id, ProjectRole};
use rusqlite::types::Value;
use rusqlite::Connection;

use crate::links;
use crate::types::{WikiSpace, WikiSpacePatch};

const MAX_NAME: usize = 120;
const MAX_DESCRIPTION: usize = 2000;

#[derive(Debug, Clone, PartialEq)]
pub struct SpaceRecord {
    pub id: Id,
    pub key: String,
    pub name: String,
    pub description: String,
    pub project_id: Option<Id>,
    pub source: String,
    /// What the importing module needs to run the import again.
    pub source_ref: Option<serde_json::Value>,
    pub created_at: String,
}

const SPACE_SELECT: &str = "SELECT s.id, s.key, s.name, s.description, s.project_id, s.source, s.source_ref, s.created_at FROM wiki_space s";

fn map_space(row: &rusqlite::Row) -> rusqlite::Result<SpaceRecord> {
    let source_ref: Option<String> = row.get(6)?;

    Ok(SpaceRecord {
        id: row.get(0)?,
        key: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        project_id: row.get(4)?,
        source: row.get(5)?,
        source_ref: source_ref.and_then(|text| serde_json::from_str(&text).ok()),
        created_at: row.get(7)?,
    })
}

pub fn space_by_key(connection: &Connection, key: &str) -> ApiResult<Option<SpaceRecord>> {
    query_one(connection, &format!("{SPACE_SELECT} WHERE s.key = ?1 COLLATE NOCASE"), [key.trim()], map_space)
}

pub fn space_by_id(connection: &Connection, id: Id) -> ApiResult<Option<SpaceRecord>> {
    query_one(connection, &format!("{SPACE_SELECT} WHERE s.id = ?1"), [id], map_space)
}

pub fn spaces_by_source(connection: &Connection, source: &str) -> ApiResult<Vec<SpaceRecord>> {
    query_all(connection, &format!("{SPACE_SELECT} WHERE s.source = ?1 ORDER BY s.key"), [source], map_space)
}

/// A space the caller may see, with their role in the project it is bound to.
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceAccess {
    pub space: SpaceRecord,
    pub project: Option<ProjectAccess>,
    instance_admin: bool,
}

impl SpaceAccess {
    pub fn can_edit(&self) -> bool {
        self.project.as_ref().is_none_or(ProjectAccess::can_edit)
    }

    pub fn can_admin(&self) -> bool {
        match &self.project {
            Some(project) => project.role == ProjectRole::Admin,
            None => self.instance_admin,
        }
    }

    pub fn require_edit(&self) -> ApiResult<()> {
        match &self.project {
            Some(project) => project.require_edit(),
            None => Ok(()),
        }
    }

    pub fn require_admin(&self) -> ApiResult<()> {
        match &self.project {
            Some(project) => project.require_admin(),
            None if self.instance_admin => Ok(()),
            None => Err(ApiError::forbidden("Only an administrator can do that")),
        }
    }
}

/// The caller's access to `space`; a space of a project they cannot see is reported as missing.
pub fn access_to(connection: &Connection, user: &AuthUser, space: SpaceRecord) -> ApiResult<SpaceAccess> {
    let project = match space.project_id {
        Some(project_id) => Some(project_access_by_id(connection, user, project_id).map_err(|error| {
            if error.status == StatusCode::NOT_FOUND {
                ApiError::not_found("Space")
            } else {
                error
            }
        })?),
        None => None,
    };

    Ok(SpaceAccess { space, project, instance_admin: user.is_admin() })
}

pub fn space_access(connection: &Connection, user: &AuthUser, key: &str) -> ApiResult<SpaceAccess> {
    let space = space_by_key(connection, key)?.ok_or_else(|| ApiError::not_found("Space"))?;

    access_to(connection, user, space)
}

/// A condition limiting the space aliased `alias` to the ones `user` may see.
pub fn visible_spaces(user: &AuthUser, alias: &str) -> (String, Vec<Value>) {
    let (visible, params) = visible_projects(user, &format!("{alias}.project_id"));

    (format!("({alias}.project_id IS NULL OR {visible})"), params)
}

pub fn describe(connection: &Connection, access: &SpaceAccess) -> ApiResult<WikiSpace> {
    let space = &access.space;
    let page_count: i64 = connection.query_row("SELECT count(*) FROM wiki_page WHERE space_id = ?1", [space.id], |row| row.get(0))?;

    Ok(WikiSpace {
        id: space.id,
        key: space.key.clone(),
        name: space.name.clone(),
        description: space.description.clone(),
        project_id: space.project_id,
        project_key: access.project.as_ref().map(|project| project.key.clone()),
        source: space.source.clone(),
        created_at: space.created_at.clone(),
        page_count,
        can_edit: access.can_edit(),
        can_admin: access.can_admin(),
    })
}

pub fn list_spaces(connection: &Connection, user: &AuthUser) -> ApiResult<Vec<WikiSpace>> {
    let (visible, params) = visible_spaces(user, "s");
    let records = query_all(
        connection,
        &format!("{SPACE_SELECT} WHERE {visible} ORDER BY s.name COLLATE NOCASE, s.key"),
        rusqlite::params_from_iter(params),
        map_space,
    )?;

    records.into_iter().map(|space| describe(connection, &access_to(connection, user, space)?)).collect()
}

pub fn clean_key(key: &str) -> ApiResult<String> {
    let key = key.trim().to_uppercase();

    if is_project_key(&key) {
        Ok(key)
    } else {
        Err(ApiError::bad_request("A space key is 2 to 10 letters and digits, starting with a letter"))
    }
}

fn clean_name(name: &str) -> ApiResult<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");

    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(ApiError::bad_request(format!("A space needs a name of 1 to {MAX_NAME} characters")));
    }

    Ok(name)
}

fn clean_description(description: &str) -> ApiResult<String> {
    let description = description.trim();

    if description.chars().count() > MAX_DESCRIPTION {
        return Err(ApiError::bad_request(format!("The description is longer than {MAX_DESCRIPTION} characters")));
    }

    Ok(description.to_string())
}

pub struct NewSpace<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub project_id: Option<Id>,
    pub source: &'a str,
    pub source_ref: Option<&'a serde_json::Value>,
}

/// Creates a space. The caller has already checked that the actor may bind it to the project.
pub fn create_space(tx: &Tx, new: &NewSpace) -> ApiResult<SpaceRecord> {
    let key = clean_key(new.key)?;
    let name = clean_name(new.name)?;
    let description = clean_description(new.description)?;

    if space_by_key(tx, &key)?.is_some() {
        return Err(ApiError::conflict(format!("The space key {key} is already taken")));
    }

    tx.execute(
        "INSERT INTO wiki_space (key, name, description, project_id, source, source_ref, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (&key, &name, &description, new.project_id, new.source, new.source_ref.map(serde_json::Value::to_string), &tx.now),
    )?;

    space_by_id(tx, tx.last_insert_rowid())?.ok_or_else(|| ApiError::not_found("Space"))
}

/// Renames a space, or binds it to another project. Changing the binding changes who may see
/// the space, so it takes the right to delete it, and to administer the project it moves to.
pub fn update_space(tx: &Tx, user: &AuthUser, access: &SpaceAccess, patch: &WikiSpacePatch) -> ApiResult<()> {
    access.require_edit()?;
    let space = &access.space;

    if let Some(name) = &patch.name {
        tx.execute("UPDATE wiki_space SET name = ?1 WHERE id = ?2", (clean_name(name)?, space.id))?;
    }

    if let Some(description) = &patch.description {
        tx.execute("UPDATE wiki_space SET description = ?1 WHERE id = ?2", (clean_description(description)?, space.id))?;
    }

    if let Some(project_key) = &patch.project_key {
        let project_id = match project_key.as_deref().map(str::trim).filter(|key| !key.is_empty()) {
            Some(key) => Some(project_access(tx, user, key)?),
            None => None,
        };

        if project_id.as_ref().map(|project| project.id) != space.project_id {
            access.require_admin()?;

            match &project_id {
                Some(project) => project.require_admin()?,
                None => user.require_admin()?,
            }

            tx.execute("UPDATE wiki_space SET project_id = ?1 WHERE id = ?2", (project_id.map(|project| project.id), space.id))?;
        }
    }

    Ok(())
}

/// Deletes a space with its pages, their history and their links.
pub fn delete_space(tx: &Tx, space: &SpaceRecord) -> ApiResult<()> {
    links::forget_pages(tx, "SELECT id FROM wiki_page WHERE space_id = ?1", [space.id])?;
    tx.execute("DELETE FROM wiki_fts WHERE rowid IN (SELECT id FROM wiki_page WHERE space_id = ?1)", [space.id])?;
    tx.execute("DELETE FROM wiki_space WHERE id = ?1", [space.id])?;

    Ok(())
}
