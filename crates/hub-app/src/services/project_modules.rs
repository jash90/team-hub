//! Modules per project. A module that is on for the instance can be switched off in one
//! project: its badges and its issue panel then stay away from that project's issues, and
//! the interface hides its navigation there.

use std::collections::{HashMap, HashSet};

use hub_core::{Id, IssueRow};
use rusqlite::Connection;
use serde_json::json;

use crate::access::ProjectAccess;
use crate::app::Tx;
use crate::db::query_all;
use crate::error::ApiResult;

/// Ids of the modules switched off in the project.
pub fn disabled_modules(connection: &Connection, project_id: Id) -> ApiResult<Vec<String>> {
    query_all(connection, "SELECT module_id FROM project_module_off WHERE project_id = ?1 ORDER BY module_id", [project_id], |row| row.get(0))
}

/// A condition that limits `column` (a project id) to the projects where the module is not
/// switched off. For what a module gathers across projects, such as its section of "My work".
pub fn on_in_project(module_id: &str, column: &str) -> String {
    debug_assert!(module_id.chars().all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_'));

    format!("{column} NOT IN (SELECT project_id FROM project_module_off WHERE module_id = '{module_id}')")
}

/// Switches a module on or off in one project. The caller has checked that the module exists.
pub fn set_project_module(tx: &Tx, access: &ProjectAccess, module_id: &str, enabled: bool) -> ApiResult<()> {
    access.require_admin()?;

    if enabled {
        tx.execute("DELETE FROM project_module_off WHERE project_id = ?1 AND module_id = ?2", (access.id, module_id))?;
    } else {
        tx.execute("INSERT OR IGNORE INTO project_module_off (project_id, module_id) VALUES (?1, ?2)", (access.id, module_id))?;
    }

    tx.emit("project.updated", Some(access.id), None, json!({ "key": access.key, "module": module_id, "enabled": enabled }))
}

/// Removes from each row what modules switched off in the row's project attached to it.
pub fn drop_disabled(connection: &Connection, rows: &mut [IssueRow]) -> ApiResult<()> {
    let pairs: Vec<(Id, String)> =
        query_all(connection, "SELECT project_id, module_id FROM project_module_off", [], |row| Ok((row.get(0)?, row.get(1)?)))?;

    if pairs.is_empty() {
        return Ok(());
    }

    let mut off: HashMap<Id, HashSet<String>> = HashMap::new();

    for (project_id, module_id) in pairs {
        off.entry(project_id).or_default().insert(module_id);
    }

    for row in rows {
        if let Some(modules) = off.get(&row.project_id) {
            row.ext.retain(|module_id, _| !modules.contains(module_id));
        }
    }

    Ok(())
}
