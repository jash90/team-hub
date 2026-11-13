//! Everything on an issue's page.

use hub_core::{Id, Issue};
use rusqlite::types::Value;

use super::attachments::load_attachments;
use super::comments::load_comments;
use super::links::load_links;
use super::rows::{issue_access, load_rows, row_by_id};
use super::worklogs::load_worklogs;
use crate::app::App;
use crate::auth::AuthUser;
use crate::db::query_all;
use crate::error::ApiResult;
use crate::services::history::load_changes;

/// Everything on an issue's page. The independent parts are read in parallel, each on its
/// own connection.
pub fn issue_detail(app: &App, user: &AuthUser, key: &str) -> ApiResult<Issue> {
    let (id, row) = app.read(|connection| {
        let (id, _) = issue_access(connection, user, key)?;

        Ok((id, row_by_id(app, connection, id)?))
    })?;

    let ((comments, changes), ((links, children), (rest, panels))) = rayon::join(
        || rayon::join(|| app.read(|connection| load_comments(connection, id)), || app.read(|connection| load_changes(connection, id))),
        || {
            rayon::join(
                || {
                    rayon::join(
                        || app.read(|connection| load_links(app, connection, id)),
                        || app.read(|connection| load_rows(app, connection, "WHERE i.parent_id = ? ORDER BY i.rank", vec![Value::Integer(id)])),
                    )
                },
                || {
                    rayon::join(
                        || {
                            app.read(|connection| {
                                let (description, estimate): (String, Option<i64>) =
                                    connection.query_row("SELECT description, estimate_minutes FROM issue WHERE id = ?1", [id], |row| {
                                        Ok((row.get(0)?, row.get(1)?))
                                    })?;
                                let watchers: Vec<Id> =
                                    query_all(connection, "SELECT user_id FROM watcher WHERE issue_id = ?1", [id], |row| row.get(0))?;

                                Ok((description, estimate, watchers, load_attachments(connection, id)?, load_worklogs(connection, id)?))
                            })
                        },
                        || {
                            app.read(|connection| {
                                let mut panels = std::collections::BTreeMap::new();
                                let off = crate::services::project_modules::disabled_modules(connection, row.project_id)?;

                                for module in app.enabled_modules().filter(|module| !off.iter().any(|id| id == module.meta().id)) {
                                    if let Some(panel) = module.issue_panel(app, connection, &row, user)? {
                                        panels.insert(module.meta().id.to_string(), panel);
                                    }
                                }

                                Ok(panels)
                            })
                        },
                    )
                },
            )
        },
    );
    let (description, estimate_minutes, watchers, attachments, worklogs) = rest?;

    Ok(Issue {
        row,
        description,
        estimate_minutes,
        spent_minutes: worklogs.iter().map(|worklog| worklog.minutes).sum(),
        comments: comments?,
        changes: changes?,
        links: links?,
        children: children?,
        attachments,
        worklogs,
        watchers,
        panels: panels?,
    })
}
