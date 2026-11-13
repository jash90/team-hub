//! Agents speak in names ("In Progress", "Anna Nowak"), the tracker in ids. This is the
//! translation in both directions; a name nothing answers to is an error listing the ones
//! that exist, so the agent can correct itself.

use std::collections::HashMap;

use hub_app::db::{query_all, query_one};
use hub_app::services::issues::status_by_name;
use hub_app::services::sprints::active_sprint;
use hub_app::services::users::resolve_user;
use hub_app::{ApiError, ApiResult, AuthUser};
use hub_core::{priority_from_name, priority_name, Id, IssueRow, StatusCategory, PRIORITY_NAMES};
use rusqlite::Connection;
use serde_json::{json, Value};

fn id_names(connection: &Connection, sql: &str) -> ApiResult<HashMap<Id, String>> {
    Ok(query_all(connection, sql, [], |row| Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?)))?.into_iter().collect())
}

/// The names behind the ids on an issue.
pub struct Names {
    statuses: HashMap<Id, (String, StatusCategory)>,
    types: HashMap<Id, String>,
    users: HashMap<Id, String>,
    sprints: HashMap<Id, String>,
    versions: HashMap<Id, String>,
    components: HashMap<Id, String>,
}

impl Names {
    pub fn load(connection: &Connection) -> ApiResult<Self> {
        let statuses = query_all(connection, "SELECT id, name, category FROM status", [], |row| {
            Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?
        .into_iter()
        .map(|(id, name, category)| (id, (name, StatusCategory::parse(&category).unwrap_or(StatusCategory::Todo))))
        .collect();

        Ok(Self {
            statuses,
            types: id_names(connection, "SELECT id, name FROM issue_type")?,
            users: id_names(connection, "SELECT id, name FROM user")?,
            sprints: id_names(connection, "SELECT id, name FROM sprint")?,
            versions: id_names(connection, "SELECT id, name FROM version")?,
            components: id_names(connection, "SELECT id, name FROM component")?,
        })
    }

    pub fn status(&self, id: Id) -> Option<&str> {
        self.statuses.get(&id).map(|(name, _)| name.as_str())
    }

    pub fn category(&self, status_id: Id) -> StatusCategory {
        self.statuses.get(&status_id).map_or(StatusCategory::Todo, |(_, category)| *category)
    }

    pub fn issue_type(&self, id: Id) -> Option<&str> {
        self.types.get(&id).map(String::as_str)
    }

    pub fn user(&self, id: Option<Id>) -> Option<&str> {
        self.users.get(&id?).map(String::as_str)
    }

    pub fn sprint(&self, id: Option<Id>) -> Option<&str> {
        self.sprints.get(&id?).map(String::as_str)
    }

    pub fn version(&self, id: Option<Id>) -> Option<&str> {
        self.versions.get(&id?).map(String::as_str)
    }

    pub fn components(&self, ids: &[Id]) -> Vec<&str> {
        ids.iter().filter_map(|id| self.components.get(id)).map(String::as_str).collect()
    }

    /// An issue in a few fields, for lists.
    pub fn row(&self, row: &IssueRow) -> Value {
        json!({
            "key": row.key,
            "summary": row.summary,
            "status": self.status(row.status_id),
            "type": self.issue_type(row.type_id),
            "priority": priority_name(row.priority),
            "assignee": self.user(row.assignee_id),
            "sprint": self.sprint(row.sprint_id),
            "points": row.points,
        })
    }
}

fn unknown(what: &str, given: &str, valid: &[String]) -> ApiError {
    let valid = if valid.is_empty() { "there are none".to_string() } else { format!("valid: {}", valid.join(", ")) };

    ApiError::bad_request(format!("Unknown {what} \"{given}\" ({valid})"))
}

fn names_where(connection: &Connection, sql: &str, project_id: Id) -> ApiResult<Vec<String>> {
    query_all(connection, sql, [project_id], |row| row.get(0))
}

pub fn status_id(connection: &Connection, project_id: Id, name: &str) -> ApiResult<Id> {
    match status_by_name(connection, project_id, name)? {
        Some(id) => Ok(id),
        None => {
            Err(unknown("status", name, &names_where(connection, "SELECT name FROM status WHERE project_id = ?1 ORDER BY position, id", project_id)?))
        }
    }
}

pub fn type_id(connection: &Connection, project_id: Id, name: &str) -> ApiResult<Id> {
    let found =
        query_one(connection, "SELECT id FROM issue_type WHERE project_id = ?1 AND name = ?2 COLLATE NOCASE", (project_id, name.trim()), |row| {
            row.get(0)
        })?;

    match found {
        Some(id) => Ok(id),
        None => Err(unknown(
            "issue type",
            name,
            &names_where(connection, "SELECT name FROM issue_type WHERE project_id = ?1 ORDER BY position, id", project_id)?,
        )),
    }
}

/// A sprint that can still take issues, by name; "active" is the running one.
pub fn sprint_id(connection: &Connection, project_id: Id, name: &str) -> ApiResult<Id> {
    let name = name.trim();
    let found = if name.eq_ignore_ascii_case("active") || name.eq_ignore_ascii_case("current") {
        active_sprint(connection, project_id)?.map(|sprint| sprint.id)
    } else {
        query_one(
            connection,
            "SELECT id FROM sprint WHERE project_id = ?1 AND state != 'closed' AND name = ?2 COLLATE NOCASE ORDER BY position, id LIMIT 1",
            (project_id, name),
            |row| row.get(0),
        )?
    };

    match found {
        Some(id) => Ok(id),
        None => Err(unknown(
            "sprint",
            name,
            &names_where(connection, "SELECT name FROM sprint WHERE project_id = ?1 AND state != 'closed' ORDER BY position, id", project_id)?,
        )),
    }
}

pub fn version_id(connection: &Connection, project_id: Id, name: &str) -> ApiResult<Id> {
    let found =
        query_one(connection, "SELECT id FROM version WHERE project_id = ?1 AND name = ?2 COLLATE NOCASE", (project_id, name.trim()), |row| {
            row.get(0)
        })?;

    match found {
        Some(id) => Ok(id),
        None => Err(unknown(
            "version",
            name,
            &names_where(connection, "SELECT name FROM version WHERE project_id = ?1 AND archived = 0 ORDER BY id", project_id)?,
        )),
    }
}

/// A person by full name, e-mail or @handle; "me" is the caller.
pub fn user_id(connection: &Connection, caller: &AuthUser, name: &str) -> ApiResult<Id> {
    if name.trim().eq_ignore_ascii_case("me") {
        return Ok(caller.id);
    }

    match resolve_user(connection, name)? {
        Some(id) => Ok(id),
        None => Err(unknown("person", name, &active_people(connection)?)),
    }
}

pub fn active_people(connection: &Connection) -> ApiResult<Vec<String>> {
    query_all(connection, "SELECT name FROM user WHERE active = 1 ORDER BY name COLLATE NOCASE", [], |row| row.get(0))
}

/// A priority given as its name ("High") or its number (1 is the highest, 5 the lowest).
pub fn priority(value: &Value) -> ApiResult<i64> {
    let found = match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => priority_from_name(text.trim()).or_else(|| text.trim().parse().ok()),
        _ => None,
    };

    found.filter(|priority| (1..=5).contains(priority)).ok_or_else(|| {
        let given = value.as_str().map(String::from).unwrap_or_else(|| value.to_string());

        unknown("priority", &given, &PRIORITY_NAMES.map(String::from))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_priority_is_a_name_or_a_number_from_one_to_five() {
        assert_eq!(priority(&json!("high")).ok(), Some(2));
        assert_eq!(priority(&json!(5)).ok(), Some(5));
        assert_eq!(priority(&json!("3")).ok(), Some(3));
        assert_eq!(
            priority(&json!("urgent")).map_err(|error| error.message),
            Err("Unknown priority \"urgent\" (valid: Highest, High, Medium, Low, Lowest)".to_string())
        );
        assert!(priority(&json!(9)).is_err());
    }
}
