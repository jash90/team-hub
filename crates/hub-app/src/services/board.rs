use chrono::{Duration, SecondsFormat, Utc};
use hub_core::{Backlog, Board, BoardColumn, BoardMode, Id, IssueRow, SprintIssues, SprintState};
use rayon::prelude::*;
use rusqlite::types::Value;

use crate::access;
use crate::app::App;
use crate::auth::AuthUser;
use crate::error::ApiResult;
use crate::services::issues::load_rows;
use crate::services::projects::load_statuses;
use crate::services::sprints::{active_sprint, get_sprint, load_sprints};

/// Finished work leaves a kanban board after this many days.
const KANBAN_DONE_DAYS: i64 = 14;
const NOT_EPIC: &str = "i.type_id IN (SELECT id FROM issue_type WHERE level != 'epic')";
const PLANNABLE: &str = "i.type_id IN (SELECT id FROM issue_type WHERE level = 'standard')";

fn board_mode(app: &App, project_id: Id) -> ApiResult<BoardMode> {
    app.read(|connection| {
        let mode: String = connection.query_row("SELECT board_mode FROM project WHERE id = ?1", [project_id], |row| row.get(0))?;

        Ok(BoardMode::parse(&mode).unwrap_or(BoardMode::Scrum))
    })
}

/// The board of a project: the running sprint (or the one asked for) in scrum mode, all
/// current work in kanban mode.
pub fn board(app: &App, user: &AuthUser, project_key: &str, sprint_id: Option<Id>) -> ApiResult<Board> {
    let access = app.read(|connection| access::project_access(connection, user, project_key))?;
    let mode = board_mode(app, access.id)?;

    let (statuses, loaded) = rayon::join(
        || app.read(|connection| load_statuses(connection, access.id)),
        || {
            app.read(|connection| {
                if mode == BoardMode::Kanban {
                    let cutoff = (Utc::now() - Duration::days(KANBAN_DONE_DAYS)).to_rfc3339_opts(SecondsFormat::Millis, true);
                    let tail = format!("WHERE i.project_id = ? AND {NOT_EPIC} AND (i.resolved_at IS NULL OR i.resolved_at >= ?) ORDER BY i.rank");

                    return Ok((None, load_rows(app, connection, &tail, vec![Value::Integer(access.id), Value::Text(cutoff)])?));
                }

                let sprint = match sprint_id {
                    Some(id) => Some(get_sprint(connection, id)?).filter(|sprint| sprint.project_id == access.id),
                    None => active_sprint(connection, access.id)?,
                };
                let issues = match &sprint {
                    Some(sprint) => {
                        load_rows(app, connection, &format!("WHERE i.sprint_id = ? AND {NOT_EPIC} ORDER BY i.rank"), vec![Value::Integer(sprint.id)])?
                    }
                    None => Vec::new(),
                };

                Ok((sprint, issues))
            })
        },
    );
    let (sprint, issues) = loaded?;
    let columns = statuses?
        .into_iter()
        .filter(|status| status.on_board)
        .map(|status| {
            let issues: Vec<IssueRow> = issues.iter().filter(|issue| issue.status_id == status.id).cloned().collect();

            BoardColumn { status, issues }
        })
        .collect();

    Ok(Board { mode, sprint, columns })
}

/// What is planned and what is waiting: the open sprints with their issues, and the backlog.
pub fn backlog(app: &App, user: &AuthUser, project_key: &str) -> ApiResult<Backlog> {
    let access = app.read(|connection| access::project_access(connection, user, project_key))?;
    let mode = board_mode(app, access.id)?;

    if mode == BoardMode::Kanban {
        let tail = format!(
            "WHERE i.project_id = ? AND {PLANNABLE} AND i.status_id IN (SELECT id FROM status WHERE on_board = 0 AND category != 'done') ORDER BY i.rank"
        );
        let backlog = app.read(|connection| load_rows(app, connection, &tail, vec![Value::Integer(access.id)]))?;

        return Ok(Backlog { sprints: Vec::new(), backlog });
    }

    let mut open: Vec<_> =
        app.read(|connection| load_sprints(connection, access.id))?.into_iter().filter(|sprint| sprint.state != SprintState::Closed).collect();
    open.sort_by_key(|sprint| (sprint.state != SprintState::Active, sprint.position, sprint.id));

    let (sprints, backlog) = rayon::join(
        || {
            open.into_par_iter()
                .map(|sprint| {
                    let tail = format!("WHERE i.sprint_id = ? AND {PLANNABLE} ORDER BY i.rank");
                    let issues = app.read(|connection| load_rows(app, connection, &tail, vec![Value::Integer(sprint.id)]))?;

                    Ok(SprintIssues { sprint, issues })
                })
                .collect::<ApiResult<Vec<_>>>()
        },
        || {
            let tail = format!(
                "WHERE i.project_id = ? AND i.sprint_id IS NULL AND {PLANNABLE}
                 AND i.status_id IN (SELECT id FROM status WHERE category != 'done') ORDER BY i.rank"
            );

            app.read(|connection| load_rows(app, connection, &tail, vec![Value::Integer(access.id)]))
        },
    );

    Ok(Backlog { sprints: sprints?, backlog: backlog? })
}
