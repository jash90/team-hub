//! Who may see and change a project.
//!
//! An open project is visible to every signed-in person, and they may all work in it. A
//! private project exists only for its members. Viewers read; members edit issues;
//! project admins (and the lead, and instance administrators) change its configuration.

use hub_core::{Id, ProjectRole};
use rusqlite::types::Value;
use rusqlite::Connection;

use crate::auth::AuthUser;
use crate::db::query_one;
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectAccess {
    pub id: Id,
    pub key: String,
    pub role: ProjectRole,
}

impl ProjectAccess {
    pub fn can_edit(&self) -> bool {
        self.role != ProjectRole::Viewer
    }

    pub fn require_edit(&self) -> ApiResult<()> {
        if self.can_edit() {
            Ok(())
        } else {
            Err(ApiError::forbidden("You can only view this project"))
        }
    }

    pub fn require_admin(&self) -> ApiResult<()> {
        if self.role == ProjectRole::Admin {
            Ok(())
        } else {
            Err(ApiError::forbidden("Only a project administrator can do that"))
        }
    }
}

fn resolve(connection: &Connection, user: &AuthUser, found: Option<(Id, String, Option<Id>, bool)>) -> ApiResult<ProjectAccess> {
    let (id, key, lead_id, is_private) = found.ok_or_else(|| ApiError::not_found("Project"))?;
    let membership: Option<String> =
        query_one(connection, "SELECT role FROM project_member WHERE project_id = ?1 AND user_id = ?2", (id, user.id), |row| row.get(0))?;

    let role = if user.is_admin() || lead_id == Some(user.id) {
        ProjectRole::Admin
    } else if let Some(role) = membership.as_deref().and_then(ProjectRole::parse) {
        role
    } else if !is_private {
        ProjectRole::Member
    } else {
        // A private project is indistinguishable from a missing one for outsiders.
        return Err(ApiError::not_found("Project"));
    };

    Ok(ProjectAccess { id, key, role })
}

type ProjectRow = (Id, String, Option<Id>, bool);

fn project_row(row: &rusqlite::Row) -> rusqlite::Result<ProjectRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

pub fn project_access(connection: &Connection, user: &AuthUser, key: &str) -> ApiResult<ProjectAccess> {
    let found = query_one(connection, "SELECT id, key, lead_id, is_private FROM project WHERE key = ?1 COLLATE NOCASE", [key], project_row)?;

    resolve(connection, user, found)
}

pub fn project_access_by_id(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<ProjectAccess> {
    let found = query_one(connection, "SELECT id, key, lead_id, is_private FROM project WHERE id = ?1", [id], project_row)?;

    resolve(connection, user, found)
}

/// A condition limiting `column` (a project id) to the projects `user` may see.
pub fn visible_projects(user: &AuthUser, column: &str) -> (String, Vec<Value>) {
    if user.is_admin() {
        return ("1 = 1".to_string(), Vec::new());
    }

    (
        format!(
            "{column} IN (SELECT id FROM project WHERE is_private = 0 OR lead_id = ? OR id IN (SELECT project_id FROM project_member WHERE user_id = ?))"
        ),
        vec![Value::Integer(user.id), Value::Integer(user.id)],
    )
}
