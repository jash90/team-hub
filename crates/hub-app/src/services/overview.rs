//! Views across projects: my work, the activity feed, quick search, saved filters.

use chrono::{Duration, SecondsFormat, Utc};
use hub_core::keys::parse_issue_key;
use hub_core::{ActivityItem, Dashboard, Id, QuickHit, SavedFilter, SprintProgress};
use rusqlite::types::Value;
use rusqlite::Connection;
use serde::Deserialize;

use crate::access::{self, visible_projects};
use crate::app::App;
use crate::auth::AuthUser;
use crate::db::{now_iso, query_all, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::issues::load_rows;
use crate::services::search::fts_match;
use crate::services::sprints::get_sprint;

const OPEN: &str = "i.status_id IN (SELECT id FROM status WHERE category != 'done')";

fn my_rows(app: &App, user: &AuthUser, condition: &str, order: &str, limit: i64) -> ApiResult<Vec<hub_core::IssueRow>> {
    let (visible, mut params) = visible_projects(user, "i.project_id");
    let tail = format!("WHERE {visible} AND {} ORDER BY {order} LIMIT {limit}", condition.replace("{me}", &user.id.to_string()));
    params.retain(|_| true);

    app.read(|connection| load_rows(app, connection, &tail, params))
}

/// "My work": the four lists are independent, so they are read at the same time.
pub fn dashboard(app: &App, user: &AuthUser) -> ApiResult<Dashboard> {
    let recent = (Utc::now() - Duration::days(14)).to_rfc3339_opts(SecondsFormat::Millis, true);
    let done_condition = format!("i.assignee_id = {{me}} AND i.resolved_at >= '{recent}'");

    let ((assigned, reported), (watching, recently_done)) = rayon::join(
        || {
            rayon::join(
                || my_rows(app, user, &format!("i.assignee_id = {{me}} AND {OPEN}"), "i.priority, i.updated_at DESC", 100),
                || {
                    my_rows(
                        app,
                        user,
                        &format!("i.reporter_id = {{me}} AND (i.assignee_id IS NULL OR i.assignee_id != {{me}}) AND {OPEN}"),
                        "i.updated_at DESC",
                        25,
                    )
                },
            )
        },
        || {
            rayon::join(
                || {
                    my_rows(
                        app,
                        user,
                        &format!(
                            "EXISTS (SELECT 1 FROM watcher w WHERE w.issue_id = i.id AND w.user_id = {{me}})
                             AND (i.assignee_id IS NULL OR i.assignee_id != {{me}}) AND (i.reporter_id IS NULL OR i.reporter_id != {{me}}) AND {OPEN}"
                        ),
                        "i.updated_at DESC",
                        25,
                    )
                },
                || my_rows(app, user, &done_condition, "i.resolved_at DESC", 15),
            )
        },
    );

    let (active_sprints, ext) = app.read(|connection| {
        let (visible, params) = visible_projects(user, "s.project_id");
        let rows = query_all(
            connection,
            &format!(
                "SELECT s.id, p.key,
                    (SELECT count(*) FROM issue i WHERE i.sprint_id = s.id),
                    (SELECT count(*) FROM issue i JOIN status st ON st.id = i.status_id WHERE i.sprint_id = s.id AND st.category = 'done'),
                    (SELECT coalesce(sum(i.points), 0) FROM issue i WHERE i.sprint_id = s.id),
                    (SELECT coalesce(sum(i.points), 0) FROM issue i JOIN status st ON st.id = i.status_id WHERE i.sprint_id = s.id AND st.category = 'done')
                 FROM sprint s JOIN project p ON p.id = s.project_id
                 WHERE s.state = 'active' AND p.archived = 0 AND {visible} ORDER BY p.key"
            ),
            rusqlite::params_from_iter(params),
            |row| {
                Ok((
                    row.get::<_, Id>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                ))
            },
        )?;
        let mut sprints = Vec::new();

        for (sprint_id, project_key, total, done, points, points_done) in rows {
            sprints.push(SprintProgress { project_key, sprint: get_sprint(connection, sprint_id)?, total, done, points, points_done });
        }

        let mut ext = std::collections::BTreeMap::new();

        for module in app.enabled_modules() {
            if let Some(section) = module.dashboard(connection, &user.through(module.meta().id))? {
                ext.insert(module.meta().id.to_string(), section);
            }
        }

        Ok((sprints, ext))
    })?;

    Ok(Dashboard { assigned: assigned?, reported: reported?, watching: watching?, recently_done: recently_done?, active_sprints, ext })
}

/// Recent events the caller may see, newest first, optionally for one project or one issue.
pub fn activity(connection: &Connection, user: &AuthUser, project_key: Option<&str>, before: Option<Id>, limit: i64) -> ApiResult<Vec<ActivityItem>> {
    let (visible, mut params) = visible_projects(user, "e.project_id");
    let mut condition = format!("e.project_id IS NOT NULL AND {visible} AND e.kind NOT IN ('issue.ranked', 'project.updated')");

    if let Some(key) = project_key {
        let access = access::project_access(connection, user, key)?;
        condition.push_str(" AND e.project_id = ?");
        params.push(Value::Integer(access.id));
    }

    if let Some(before) = before {
        condition.push_str(" AND e.id < ?");
        params.push(Value::Integer(before));
    }

    params.push(Value::Integer(limit.clamp(1, 200)));

    query_all(
        connection,
        &format!(
            "SELECT e.id, e.at, e.actor_id, e.actor_label, e.kind, p.key, i.key, i.summary, e.payload
             FROM event e LEFT JOIN project p ON p.id = e.project_id LEFT JOIN issue i ON i.id = e.issue_id
             WHERE {condition} ORDER BY e.id DESC LIMIT ?"
        ),
        rusqlite::params_from_iter(params),
        |row| {
            let payload: String = row.get(8)?;

            Ok(ActivityItem {
                id: row.get(0)?,
                at: row.get(1)?,
                actor_id: row.get(2)?,
                actor_label: row.get(3)?,
                kind: row.get(4)?,
                project_key: row.get(5)?,
                issue_key: row.get(6)?,
                issue_summary: row.get(7)?,
                payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
            })
        },
    )
}

/// A handful of things matching what is being typed: issues by key or words, then whatever
/// the modules find (pages, pull requests).
pub fn quick_search(app: &App, connection: &Connection, user: &AuthUser, text: &str) -> ApiResult<Vec<QuickHit>> {
    let text = text.trim();

    if text.is_empty() {
        return Ok(Vec::new());
    }

    let (visible, visible_params) = visible_projects(user, "i.project_id");
    let hit = |row: &rusqlite::Row| -> rusqlite::Result<QuickHit> {
        let key: String = row.get(0)?;

        Ok(QuickHit { kind: "issue".to_string(), url: format!("/issue/{key}"), id: key.clone(), title: row.get(1)?, subtitle: key })
    };
    let mut hits = Vec::new();

    if let Some((project, number)) = parse_issue_key(text) {
        let mut params = visible_params.clone();
        params.push(Value::Text(format!("{project}-{number}")));
        hits.extend(query_all(
            connection,
            &format!("SELECT i.key, i.summary FROM issue i WHERE {visible} AND i.key = ?"),
            rusqlite::params_from_iter(params),
            hit,
        )?);
    } else if text.chars().all(|character| character.is_ascii_digit()) {
        let mut params = visible_params.clone();
        params.push(Value::Text(text.to_string()));
        hits.extend(query_all(
            connection,
            &format!("SELECT i.key, i.summary FROM issue i WHERE {visible} AND CAST(i.number AS TEXT) = ? ORDER BY i.updated_at DESC LIMIT 5"),
            rusqlite::params_from_iter(params),
            hit,
        )?);
    }

    if let Some(expression) = fts_match(text, None) {
        let mut params = visible_params;
        params.push(Value::Text(expression));
        let found = query_all(
            connection,
            &format!(
                "SELECT i.key, i.summary FROM issue_fts f JOIN issue i ON i.id = f.rowid
                 WHERE {visible} AND issue_fts MATCH ? ORDER BY bm25(issue_fts, 10.0, 5.0, 1.0, 0.5) LIMIT 8"
            ),
            rusqlite::params_from_iter(params),
            hit,
        )?;

        for found in found {
            if !hits.iter().any(|existing: &QuickHit| existing.id == found.id) {
                hits.push(found);
            }
        }
    }

    for module in app.enabled_modules() {
        hits.extend(module.quick_search(connection, &user.through(module.meta().id), text)?);
    }

    Ok(hits)
}

fn map_filter(row: &rusqlite::Row) -> rusqlite::Result<SavedFilter> {
    Ok(SavedFilter { id: row.get(0)?, owner_id: row.get(1)?, name: row.get(2)?, query: row.get(3)?, shared: row.get(4)? })
}

pub fn list_filters(connection: &Connection, user: &AuthUser) -> ApiResult<Vec<SavedFilter>> {
    query_all(
        connection,
        "SELECT id, owner_id, name, query, shared FROM filter WHERE owner_id = ?1 OR shared = 1 ORDER BY name COLLATE NOCASE",
        [user.id],
        map_filter,
    )
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterInput {
    pub name: Option<String>,
    pub query: Option<String>,
    pub shared: Option<bool>,
}

pub fn save_filter(connection: &Connection, user: &AuthUser, id: Option<Id>, input: &FilterInput) -> ApiResult<SavedFilter> {
    let id = match id {
        Some(id) => {
            let owner: Option<Id> = query_one(connection, "SELECT owner_id FROM filter WHERE id = ?1", [id], |row| row.get(0))?;

            if owner != Some(user.id) {
                return Err(ApiError::not_found("Filter"));
            }

            if let Some(name) = input.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
                connection.execute("UPDATE filter SET name = ?1 WHERE id = ?2", (name, id))?;
            }

            if let Some(query) = &input.query {
                connection.execute("UPDATE filter SET query = ?1 WHERE id = ?2", (query.trim(), id))?;
            }

            if let Some(shared) = input.shared {
                connection.execute("UPDATE filter SET shared = ?1 WHERE id = ?2", (shared, id))?;
            }

            id
        }
        None => {
            let name = input
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| ApiError::bad_request("The filter needs a name"))?;

            connection.execute(
                "INSERT INTO filter (owner_id, name, query, shared, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                (user.id, name, input.query.as_deref().unwrap_or("").trim(), input.shared.unwrap_or(false), now_iso()),
            )?;

            connection.last_insert_rowid()
        }
    };

    Ok(connection.query_row("SELECT id, owner_id, name, query, shared FROM filter WHERE id = ?1", [id], map_filter)?)
}

pub fn delete_filter(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<()> {
    let deleted = connection.execute("DELETE FROM filter WHERE id = ?1 AND owner_id = ?2", (id, user.id))?;

    if deleted == 0 {
        return Err(ApiError::not_found("Filter"));
    }

    Ok(())
}
