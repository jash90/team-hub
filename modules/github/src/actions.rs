//! What happens to issues because of their code: workflow rules ("pull request opened →
//! In Review") and commands written in commit messages.

use hub_app::db::query_one;
use hub_app::services::issues::{add_comment, add_worklog, status_by_name, update_issue};
use hub_app::{Actor, ApiResult, App};
use hub_core::smart_commits::SmartCommand;
use hub_core::{Id, IssuePatch};
use rusqlite::Connection;

use crate::ingest::{Action, CommitAuthor};
use crate::model::GithubTrigger;

/// Runs each action as its own change. A failing action (say, a strict workflow refusing a
/// move) does not stop the others; what happened is returned as notes for the delivery log.
pub fn run_actions(app: &App, actions: Vec<Action>) -> Vec<String> {
    let mut notes = Vec::new();

    for action in actions {
        let outcome = match action {
            Action::Rule { issue_id, trigger, closes } => apply_rule(app, issue_id, trigger, closes),
            Action::Smart { issue_id, command, author } => apply_smart(app, issue_id, &command, &author).map(Some),
        };

        match outcome {
            Ok(Some(note)) => notes.push(note),
            Ok(None) => {}
            Err(error) => notes.push(format!("failed: {}", error.message)),
        }
    }

    notes
}

struct Standing {
    key: String,
    project_id: Id,
    status_id: Id,
    category: String,
    position: i64,
}

fn standing(connection: &Connection, issue_id: Id) -> ApiResult<Option<Standing>> {
    query_one(
        connection,
        "SELECT i.key, i.project_id, i.status_id, s.category, s.position FROM issue i JOIN status s ON s.id = i.status_id WHERE i.id = ?1",
        [issue_id],
        |row| Ok(Standing { key: row.get(0)?, project_id: row.get(1)?, status_id: row.get(2)?, category: row.get(3)?, position: row.get(4)? }),
    )
}

/// Moves an issue as its project's rule for `trigger` says, unless that would undo progress:
/// a new branch only starts work that has not started, nothing reopens finished work, and
/// an issue is only finished by a merge once none of its pull requests is still open.
fn apply_rule(app: &App, issue_id: Id, trigger: GithubTrigger, closes: bool) -> ApiResult<Option<String>> {
    app.write(Actor::system("GitHub"), |tx| {
        let Some(issue) = standing(tx, issue_id)? else { return Ok(None) };
        let ruled: Option<Id> = query_one(
            tx,
            "SELECT status_id FROM gh_rule WHERE project_id = ?1 AND trigger = ?2",
            (issue.project_id, trigger.as_str()),
            |row| row.get(0),
        )?;
        let target = match ruled {
            Some(status_id) => Some(status_id),
            None if trigger == GithubTrigger::PrMerged && closes => query_one(
                tx,
                "SELECT id FROM status WHERE project_id = ?1 AND category = 'done' ORDER BY position, id LIMIT 1",
                [issue.project_id],
                |row| row.get(0),
            )?,
            None => None,
        };
        let Some(target) = target.filter(|target| *target != issue.status_id) else { return Ok(None) };
        let target_position: i64 = tx.query_row("SELECT position FROM status WHERE id = ?1", [target], |row| row.get(0))?;
        let forward = target_position > issue.position;

        let allowed = match trigger {
            GithubTrigger::BranchCreated => issue.category == "todo",
            GithubTrigger::PrOpened | GithubTrigger::PrApproved => issue.category != "done" && forward,
            GithubTrigger::ChangesRequested => issue.category != "done",
            GithubTrigger::PrMerged => {
                let open: i64 = tx.query_row(
                    "SELECT count(*) FROM gh_ref f JOIN gh_pr p ON p.id = f.ref_id WHERE f.kind = 'pr' AND f.issue_id = ?1 AND p.state = 'open'",
                    [issue_id],
                    |row| row.get(0),
                )?;

                open == 0 && issue.category != "done"
            }
        };

        if !allowed {
            return Ok(None);
        }

        update_issue(tx, issue_id, &IssuePatch { status_id: Some(target), ..Default::default() })?;

        Ok(Some(format!("{} moved ({})", issue.key, trigger.as_str())))
    })
}

/// The user behind a commit: by GitHub login first, then by e-mail address.
fn actor_for(app: &App, author: &CommitAuthor) -> ApiResult<Actor> {
    let label = author.login.clone().or_else(|| author.name.clone()).unwrap_or_else(|| "unknown".to_string());
    let user: Option<Id> = app.read(|connection| {
        if let Some(login) = &author.login {
            let found = query_one(connection, "SELECT id FROM user WHERE github_login = ?1 AND active = 1", [login], |row| row.get(0))?;

            if found.is_some() {
                return Ok(found);
            }
        }

        match &author.email {
            Some(email) => query_one(connection, "SELECT id FROM user WHERE email = ?1 AND active = 1", [email], |row| row.get(0)),
            None => Ok(None),
        }
    })?;

    Ok(match user {
        Some(id) => Actor::user_via(id, "GitHub"),
        None => Actor::system(format!("GitHub · {label}")),
    })
}

fn apply_smart(app: &App, issue_id: Id, command: &SmartCommand, author: &CommitAuthor) -> ApiResult<String> {
    let actor = actor_for(app, author)?;

    app.write(actor, |tx| {
        let Some(issue) = standing(tx, issue_id)? else { return Ok("issue gone".to_string()) };

        match command {
            SmartCommand::Comment(body) => {
                add_comment(tx, issue_id, body)?;

                Ok(format!("{} commented", issue.key))
            }
            SmartCommand::Time(minutes) => {
                add_worklog(tx, issue_id, *minutes, None, "Logged from a commit")?;

                Ok(format!("{} logged {minutes}m", issue.key))
            }
            SmartCommand::Transition(name) => match status_by_name(tx, issue.project_id, name)? {
                Some(status_id) => {
                    update_issue(tx, issue_id, &IssuePatch { status_id: Some(status_id), ..Default::default() })?;

                    Ok(format!("{} moved by commit", issue.key))
                }
                None => Ok(format!("{}: no status called \"{name}\"", issue.key)),
            },
        }
    })
}
