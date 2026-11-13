//! Turns what GitHub reports (webhook payloads, and the same shapes read through the API)
//! into rows and links, and says what should happen to the issues involved.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use hub_app::db::{query_all, query_one};
use hub_app::{ApiResult, Tx};
use hub_core::keys::{extract_issue_keys, extract_issue_keys_many, KeyCase};
use hub_core::smart_commits::{closing_keys, parse_smart_commits, SmartCommand};
use hub_core::Id;
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::drift::recompute_drift;
use crate::model::GithubTrigger;

/// Who wrote a commit, as far as GitHub told us.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommitAuthor {
    pub name: Option<String>,
    pub login: Option<String>,
    pub email: Option<String>,
}

/// Something to do to an issue because of what happened to its code. Run after the
/// ingesting transaction, each as its own change by its own actor.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Rule { issue_id: Id, trigger: GithubTrigger, closes: bool },
    Smart { issue_id: Id, command: SmartCommand, author: CommitAuthor },
}

#[derive(Debug, Default)]
pub struct Ingested {
    pub outcome: String,
    pub actions: Vec<Action>,
    /// Issues whose development information changed.
    pub touched: Vec<Id>,
}

impl Ingested {
    fn ignored(reason: &str) -> Self {
        Self { outcome: format!("ignored: {reason}"), ..Default::default() }
    }

    fn touch(&mut self, issue_ids: &[Id]) {
        for issue_id in issue_ids {
            if !self.touched.contains(issue_id) {
                self.touched.push(*issue_id);
            }
        }
    }
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |value, key| value.get(key)).and_then(Value::as_str)
}

fn owned(value: &Value, path: &[&str]) -> Option<String> {
    text(value, path).map(String::from)
}

/// Any timestamp GitHub sends, as UTC in the format the rest of the database uses.
pub fn normalize_time(text: &str) -> String {
    DateTime::parse_from_rfc3339(text)
        .map(|at| at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_else(|_| text.to_string())
}

pub fn project_keys(connection: &Connection) -> ApiResult<HashSet<String>> {
    Ok(query_all(connection, "SELECT key FROM project", [], |row| row.get::<_, String>(0))?.into_iter().collect())
}

pub fn repo_id(connection: &Connection, full_name: &str) -> ApiResult<Option<Id>> {
    let Some((owner, name)) = full_name.split_once('/') else { return Ok(None) };

    // A repository of a project that has switched this module off is, for a delivery, not connected.
    let on = hub_app::services::project_modules::on_in_project("github", "project_id");

    query_one(connection, &format!("SELECT id FROM gh_repo WHERE owner = ?1 AND name = ?2 AND {on}"), (owner, name), |row| row.get(0))
}

/// Replaces the issues one pull request, branch or commit is linked to; returns the issues
/// linked before or after.
fn set_refs(connection: &Connection, kind: &str, ref_id: Id, keys: &[String]) -> ApiResult<Vec<Id>> {
    let mut affected: Vec<Id> =
        query_all(connection, "SELECT issue_id FROM gh_ref WHERE kind = ?1 AND ref_id = ?2", (kind, ref_id), |row| row.get(0))?;
    connection.execute("DELETE FROM gh_ref WHERE kind = ?1 AND ref_id = ?2", (kind, ref_id))?;

    for key in keys {
        let issue_id: Option<Id> = query_one(connection, "SELECT id FROM issue WHERE key = ?1", [key], |row| row.get(0))?;

        if let Some(issue_id) = issue_id {
            connection.execute("INSERT OR IGNORE INTO gh_ref (issue_id, kind, ref_id) VALUES (?1, ?2, ?3)", (issue_id, kind, ref_id))?;

            if !affected.contains(&issue_id) {
                affected.push(issue_id);
            }
        }
    }

    Ok(affected)
}

fn linked_issues(connection: &Connection, kind: &str, ref_id: Id) -> ApiResult<Vec<Id>> {
    query_all(connection, "SELECT issue_id FROM gh_ref WHERE kind = ?1 AND ref_id = ?2", (kind, ref_id), |row| row.get(0))
}

pub struct UpsertedPr {
    pub id: Id,
    pub issues: Vec<Id>,
    /// Issues the pull request promises to close ("Fixes WEB-12").
    pub closes: Vec<Id>,
    pub was_open: bool,
    pub is_new: bool,
}

/// Stores a pull request as the API or a webhook describes it and links it to the issues
/// named in its title, its branch, or a closing phrase in its description.
pub fn upsert_pr(connection: &Connection, repo: Id, repo_name: &str, pr: &Value, projects: &HashSet<String>) -> ApiResult<Option<UpsertedPr>> {
    let (Some(number), Some(title)) = (pr.get("number").and_then(Value::as_i64), text(pr, &["title"])) else { return Ok(None) };
    let body = text(pr, &["body"]).unwrap_or("");
    let head_ref = text(pr, &["head", "ref"]).unwrap_or("");
    let merged = pr.get("merged").and_then(Value::as_bool).unwrap_or(false) || text(pr, &["merged_at"]).is_some();
    let state = match (merged, text(pr, &["state"])) {
        (true, _) => "merged",
        (false, Some("closed")) => "closed",
        _ => "open",
    };
    let time = |path: &[&str]| text(pr, path).map(normalize_time);
    let url = owned(pr, &["html_url"]).unwrap_or_else(|| format!("https://github.com/{repo_name}/pull/{number}"));
    let previous: Option<(Id, String)> =
        query_one(connection, "SELECT id, state FROM gh_pr WHERE repo_id = ?1 AND number = ?2", (repo, number), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;

    connection.execute(
        "INSERT INTO gh_pr (repo_id, number, title, body, state, draft, author, head_ref, head_sha, base_ref, url, additions, deletions,
                            created_at, updated_at, merged_at, closed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT (repo_id, number) DO UPDATE SET
            title = excluded.title, body = excluded.body, state = excluded.state, draft = excluded.draft,
            author = coalesce(excluded.author, gh_pr.author), head_ref = excluded.head_ref, base_ref = excluded.base_ref, url = excluded.url,
            checks = CASE WHEN excluded.head_sha IS NOT NULL AND gh_pr.head_sha IS NOT excluded.head_sha THEN NULL ELSE gh_pr.checks END,
            head_sha = coalesce(excluded.head_sha, gh_pr.head_sha),
            additions = coalesce(excluded.additions, gh_pr.additions), deletions = coalesce(excluded.deletions, gh_pr.deletions),
            created_at = coalesce(excluded.created_at, gh_pr.created_at), updated_at = excluded.updated_at,
            merged_at = excluded.merged_at, closed_at = excluded.closed_at",
        rusqlite::params![
            repo,
            number,
            title,
            body,
            state,
            pr.get("draft").and_then(Value::as_bool).unwrap_or(false),
            text(pr, &["user", "login"]),
            head_ref,
            text(pr, &["head", "sha"]),
            text(pr, &["base", "ref"]).unwrap_or(""),
            url,
            pr.get("additions").and_then(Value::as_i64),
            pr.get("deletions").and_then(Value::as_i64),
            time(&["created_at"]),
            time(&["updated_at"]).unwrap_or_else(hub_app::db::now_iso),
            time(&["merged_at"]),
            time(&["closed_at"]),
        ],
    )?;
    let id: Id = connection.query_row("SELECT id FROM gh_pr WHERE repo_id = ?1 AND number = ?2", (repo, number), |row| row.get(0))?;

    let closing = closing_keys(&format!("{title}\n{body}"), projects);
    let mut keys = extract_issue_keys(title, projects, KeyCase::Exact);

    for key in extract_issue_keys(head_ref, projects, KeyCase::Any).into_iter().chain(closing.iter().cloned()) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }

    let issues = set_refs(connection, "pr", id, &keys)?;
    let mut closes = Vec::new();

    for key in &closing {
        if let Some(issue_id) = query_one(connection, "SELECT id FROM issue WHERE key = ?1", [key], |row| row.get::<_, Id>(0))? {
            closes.push(issue_id);
        }
    }

    Ok(Some(UpsertedPr { id, issues, closes, was_open: previous.as_ref().is_some_and(|(_, state)| state == "open"), is_new: previous.is_none() }))
}

/// Stores a branch and links it to the issues its name mentions. Returns the linked issues
/// and whether the branch is new.
pub fn upsert_branch(
    connection: &Connection,
    repo: Id,
    repo_name: &str,
    name: &str,
    sha: Option<&str>,
    at: &str,
    projects: &HashSet<String>,
) -> ApiResult<(Vec<Id>, bool)> {
    let existing: Option<(Id, bool)> =
        query_one(connection, "SELECT id, deleted FROM gh_branch WHERE repo_id = ?1 AND name = ?2", (repo, name), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    let url = format!("https://github.com/{repo_name}/tree/{name}");

    connection.execute(
        "INSERT INTO gh_branch (repo_id, name, sha, url, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (repo_id, name) DO UPDATE SET sha = coalesce(excluded.sha, gh_branch.sha), updated_at = excluded.updated_at, deleted = 0",
        (repo, name, sha, url, at),
    )?;
    let id: Id = connection.query_row("SELECT id FROM gh_branch WHERE repo_id = ?1 AND name = ?2", (repo, name), |row| row.get(0))?;
    let issues = set_refs(connection, "branch", id, &extract_issue_keys(name, projects, KeyCase::Any))?;

    Ok((issues, existing.is_none_or(|(_, deleted)| deleted)))
}

pub struct NewCommit<'a> {
    pub sha: &'a str,
    pub message: &'a str,
    pub url: Option<&'a str>,
    pub at: String,
    pub author: CommitAuthor,
}

/// Stores commits and links them to the issues their messages mention. Keys are extracted
/// for all messages at once. Returns the linked issues and, for commits not seen before, the
/// commands their messages carry.
pub fn upsert_commits(
    connection: &Connection,
    repo: Id,
    repo_name: &str,
    commits: &[NewCommit],
    projects: &HashSet<String>,
) -> ApiResult<(Vec<Id>, Vec<Action>, i64)> {
    let messages: Vec<&str> = commits.iter().map(|commit| commit.message).collect();
    let keys = extract_issue_keys_many(&messages, projects, KeyCase::Exact);
    let mut touched = Vec::new();
    let mut actions = Vec::new();
    let mut stored = 0;

    for (commit, keys) in commits.iter().zip(keys) {
        if keys.is_empty() {
            continue;
        }

        let known: Option<Id> =
            query_one(connection, "SELECT id FROM gh_commit WHERE repo_id = ?1 AND sha = ?2", (repo, commit.sha), |row| row.get(0))?;
        let url = commit.url.map(String::from).unwrap_or_else(|| format!("https://github.com/{repo_name}/commit/{}", commit.sha));

        connection.execute(
            "INSERT OR IGNORE INTO gh_commit (repo_id, sha, message, author_name, author_login, author_email, url, committed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (repo, commit.sha, commit.message, &commit.author.name, &commit.author.login, &commit.author.email, url, &commit.at),
        )?;
        let id: Id = connection.query_row("SELECT id FROM gh_commit WHERE repo_id = ?1 AND sha = ?2", (repo, commit.sha), |row| row.get(0))?;

        for issue_id in set_refs(connection, "commit", id, &keys)? {
            if !touched.contains(&issue_id) {
                touched.push(issue_id);
            }
        }

        if known.is_some() {
            continue;
        }

        stored += 1;

        for smart in parse_smart_commits(commit.message, projects) {
            let issue_id: Option<Id> = query_one(connection, "SELECT id FROM issue WHERE key = ?1", [&smart.key], |row| row.get(0))?;
            let Some(issue_id) = issue_id else { continue };

            for command in smart.commands {
                actions.push(Action::Smart { issue_id, command, author: commit.author.clone() });
            }
        }
    }

    Ok((touched, actions, stored))
}

fn check_state(status: Option<&str>, conclusion: Option<&str>) -> Option<&'static str> {
    match (status, conclusion) {
        (_, Some("success" | "neutral" | "skipped")) => Some("passing"),
        (_, Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure" | "error")) => Some("failing"),
        (Some("queued" | "in_progress" | "requested" | "waiting" | "pending"), _) => Some("running"),
        _ => None,
    }
}

/// Sets the check state of the open pull requests whose head is `sha`. A failure is not
/// overwritten by a later success of another suite on the same commit.
fn set_checks(connection: &Connection, repo: Id, sha: &str, state: &str) -> ApiResult<Vec<Id>> {
    let pulls: Vec<(Id, Option<String>)> =
        query_all(connection, "SELECT id, checks FROM gh_pr WHERE repo_id = ?1 AND head_sha = ?2", (repo, sha), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    let mut issues = Vec::new();

    for (id, current) in pulls {
        if current.as_deref() == Some("failing") && state == "passing" {
            continue;
        }

        connection.execute("UPDATE gh_pr SET checks = ?1 WHERE id = ?2", (state, id))?;
        issues.extend(linked_issues(connection, "pr", id)?);
    }

    Ok(issues)
}

fn announce(tx: &Tx, issue_id: Id, action: &str, repo_name: &str, pr: &Value) -> ApiResult<()> {
    let project_id: Id = tx.query_row("SELECT project_id FROM issue WHERE id = ?1", [issue_id], |row| row.get(0))?;

    tx.emit(
        "github.pr",
        Some(project_id),
        Some(issue_id),
        json!({ "action": action, "repo": repo_name, "number": pr.get("number"), "title": pr.get("title"), "url": pr.get("html_url") }),
    )
}

/// Handles one webhook delivery. Everything it stores is idempotent, so a redelivery
/// changes nothing.
pub fn ingest(tx: &Tx, event: &str, payload: &Value) -> ApiResult<Ingested> {
    if event == "ping" {
        return Ok(Ingested { outcome: "pong".to_string(), ..Default::default() });
    }

    let Some(repo_name) = text(payload, &["repository", "full_name"]) else { return Ok(Ingested::ignored("no repository in the payload")) };
    let Some(repo) = repo_id(tx, repo_name)? else { return Ok(Ingested::ignored("this repository is not connected to a project")) };
    let projects = project_keys(tx)?;
    let mut result = Ingested::default();

    tx.execute("UPDATE gh_repo SET last_event_at = ?1 WHERE id = ?2", (&tx.now, repo))?;

    match event {
        "push" => {
            let Some(branch) = text(payload, &["ref"]).and_then(|name| name.strip_prefix("refs/heads/")) else {
                return Ok(Ingested::ignored("not a branch push"));
            };

            if payload.get("deleted").and_then(Value::as_bool).unwrap_or(false) {
                tx.execute("UPDATE gh_branch SET deleted = 1, updated_at = ?1 WHERE repo_id = ?2 AND name = ?3", (&tx.now, repo, branch))?;
                let id: Option<Id> = query_one(tx, "SELECT id FROM gh_branch WHERE repo_id = ?1 AND name = ?2", (repo, branch), |row| row.get(0))?;

                if let Some(id) = id {
                    result.touch(&linked_issues(tx, "branch", id)?);
                }

                result.outcome = format!("branch {branch} deleted");
            } else {
                let (issues, is_new) = upsert_branch(tx, repo, repo_name, branch, text(payload, &["after"]), &tx.now, &projects)?;

                if is_new {
                    result.actions.extend(issues.iter().map(|issue_id| Action::Rule {
                        issue_id: *issue_id,
                        trigger: GithubTrigger::BranchCreated,
                        closes: false,
                    }));
                }

                result.touch(&issues);

                let empty = Vec::new();
                let commits: Vec<NewCommit> = payload
                    .get("commits")
                    .and_then(Value::as_array)
                    .unwrap_or(&empty)
                    .iter()
                    .filter_map(|commit| {
                        Some(NewCommit {
                            sha: text(commit, &["id"])?,
                            message: text(commit, &["message"])?,
                            url: text(commit, &["url"]),
                            at: text(commit, &["timestamp"]).map(normalize_time).unwrap_or_else(|| tx.now.clone()),
                            author: CommitAuthor {
                                name: owned(commit, &["author", "name"]),
                                login: owned(commit, &["author", "username"]),
                                email: owned(commit, &["author", "email"]),
                            },
                        })
                    })
                    .collect();
                let (issues, actions, stored) = upsert_commits(tx, repo, repo_name, &commits, &projects)?;

                result.touch(&issues);
                result.actions.extend(actions);
                result.outcome = format!("branch {branch}: {stored} new commits linked");
            }
        }
        "create" | "delete" => {
            if text(payload, &["ref_type"]) != Some("branch") {
                return Ok(Ingested::ignored("not a branch"));
            }

            let Some(branch) = text(payload, &["ref"]) else { return Ok(Ingested::ignored("no branch name")) };

            if event == "create" {
                let (issues, is_new) = upsert_branch(tx, repo, repo_name, branch, None, &tx.now, &projects)?;

                if is_new {
                    result.actions.extend(issues.iter().map(|issue_id| Action::Rule {
                        issue_id: *issue_id,
                        trigger: GithubTrigger::BranchCreated,
                        closes: false,
                    }));
                }

                result.touch(&issues);
                result.outcome = format!("branch {branch} created");
            } else {
                tx.execute("UPDATE gh_branch SET deleted = 1, updated_at = ?1 WHERE repo_id = ?2 AND name = ?3", (&tx.now, repo, branch))?;
                let id: Option<Id> = query_one(tx, "SELECT id FROM gh_branch WHERE repo_id = ?1 AND name = ?2", (repo, branch), |row| row.get(0))?;

                if let Some(id) = id {
                    result.touch(&linked_issues(tx, "branch", id)?);
                }

                result.outcome = format!("branch {branch} deleted");
            }
        }
        "pull_request" => {
            let action = text(payload, &["action"]).unwrap_or("");
            let Some(pr) = payload.get("pull_request") else { return Ok(Ingested::ignored("no pull request in the payload")) };
            let Some(stored) = upsert_pr(tx, repo, repo_name, pr, &projects)? else { return Ok(Ingested::ignored("incomplete pull request")) };
            let merged = pr.get("merged").and_then(Value::as_bool).unwrap_or(false);
            let draft = pr.get("draft").and_then(Value::as_bool).unwrap_or(false);
            let trigger = match action {
                "opened" | "reopened" if !draft => Some(GithubTrigger::PrOpened),
                "ready_for_review" => Some(GithubTrigger::PrOpened),
                "closed" if merged => Some(GithubTrigger::PrMerged),
                _ => None,
            };

            if action == "converted_to_draft" || action == "synchronize" {
                // New commits invalidate an earlier verdict only on GitHub's say-so; leave the review as it is.
            }

            if let Some(trigger) = trigger {
                for issue_id in &stored.issues {
                    result.actions.push(Action::Rule { issue_id: *issue_id, trigger, closes: stored.closes.contains(issue_id) });
                    announce(tx, *issue_id, if trigger == GithubTrigger::PrMerged { "merged" } else { "opened" }, repo_name, pr)?;
                }
            }

            result.touch(&stored.issues);
            result.outcome = format!("pull request #{} {action}", pr.get("number").and_then(Value::as_i64).unwrap_or(0));
        }
        "pull_request_review" => {
            let Some(pr) = payload.get("pull_request") else { return Ok(Ingested::ignored("no pull request in the payload")) };
            let Some(stored) = upsert_pr(tx, repo, repo_name, pr, &projects)? else { return Ok(Ingested::ignored("incomplete pull request")) };
            let verdict = match (text(payload, &["action"]), text(payload, &["review", "state"]).map(str::to_lowercase).as_deref()) {
                (Some("submitted"), Some("approved")) => Some(("approved", GithubTrigger::PrApproved)),
                (Some("submitted"), Some("changes_requested")) => Some(("changes_requested", GithubTrigger::ChangesRequested)),
                (Some("dismissed"), _) => {
                    tx.execute("UPDATE gh_pr SET review = 'pending' WHERE id = ?1", [stored.id])?;
                    None
                }
                _ => None,
            };

            if let Some((review, trigger)) = verdict {
                tx.execute("UPDATE gh_pr SET review = ?1 WHERE id = ?2", (review, stored.id))?;
                result.actions.extend(stored.issues.iter().map(|issue_id| Action::Rule { issue_id: *issue_id, trigger, closes: false }));
            }

            result.touch(&stored.issues);
            result.outcome = "review recorded".to_string();
        }
        "check_suite" | "workflow_run" | "check_run" => {
            let subject = &payload[event];
            let Some(sha) = text(subject, &["head_sha"]) else { return Ok(Ingested::ignored("no commit in the payload")) };

            match check_state(text(subject, &["status"]), text(subject, &["conclusion"])) {
                Some(state) => {
                    result.touched = set_checks(tx, repo, sha, state)?;
                    result.outcome = format!("checks {state}");
                }
                None => return Ok(Ingested::ignored("check state not understood")),
            }
        }
        "status" => {
            let Some(sha) = text(payload, &["sha"]) else { return Ok(Ingested::ignored("no commit in the payload")) };
            let state = match text(payload, &["state"]) {
                Some("success") => "passing",
                Some("failure" | "error") => "failing",
                _ => "running",
            };

            result.touched = set_checks(tx, repo, sha, state)?;
            result.outcome = format!("checks {state}");
        }
        _ => return Ok(Ingested::ignored("event not handled")),
    }

    recompute_drift(tx, &result.touched)?;

    for issue_id in &result.touched {
        let project_id: Id = tx.query_row("SELECT project_id FROM issue WHERE id = ?1", [issue_id], |row| row.get(0))?;
        tx.emit("github.updated", Some(project_id), Some(*issue_id), json!({ "repo": repo_name }))?;
    }

    Ok(result)
}
