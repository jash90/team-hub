use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use hub_app::access::{project_access, project_access_by_id};
use hub_app::app::set_setting;
use hub_app::auth::random_token;
use hub_app::db::{now_iso, query_all, query_one};
use hub_app::secrets::{clear_secret, get_secret, secret_hint, set_secret};
use hub_app::services::issues::{issue_access, rows_by_ids};
use hub_app::{Actor, ApiError, ApiResult, App, AuthUser};
use hub_core::keys::suggest_branch_name;
use hub_core::{Id, IssueRow};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::actions::run_actions;
use crate::client::{api_url, clean_api_url, sync_and_record, GithubClient, NewPullRequest, API_URL_SETTING, TOKEN_SECRET, WEBHOOK_SECRET};
use crate::drift::recompute_drift;
use crate::ingest::{ingest, project_keys, upsert_branch, Action};
use crate::model::{
    id_list, issue_branches, issue_commits, issue_pulls, project_repos, project_rules, pulls_where, repo_by_id, stored_drift, GithubDelivery,
    GithubDriftItem, GithubIssuePanel, GithubProjectCode, GithubRepoRef, GithubRule, GithubSettings, GithubSyncReport, GithubTrigger,
};

type Reply<T> = ApiResult<Json<T>>;

pub fn routes() -> Router<App> {
    Router::new()
        .route("/settings", get(get_settings).put(put_settings))
        .route("/settings/webhook-secret", post(generate_webhook_secret))
        .route("/deliveries", get(deliveries))
        .route("/projects/{key}", get(project_code))
        .route("/projects/{key}/repos", post(add_repo))
        .route("/projects/{key}/rules", put(set_rules))
        .route("/repos/{id}", delete(remove_repo))
        .route("/repos/{id}/sync", post(sync_repo))
        .route("/issues/{key}/branch", post(create_branch))
        .route("/issues/{key}/pull-request", post(create_pull_request))
}

pub fn hooks() -> Router<App> {
    Router::new().route("/", post(webhook))
}

fn webhook_url(app: &App) -> String {
    format!("{}/hooks/github", app.config.base_url.trim_end_matches('/'))
}

fn settings(app: &App, connection: &Connection) -> ApiResult<GithubSettings> {
    Ok(GithubSettings {
        api_url: hub_app::app::get_setting::<String>(connection, API_URL_SETTING)?.unwrap_or_else(|| crate::client::DEFAULT_API_URL.to_string()),
        token_hint: secret_hint(connection, TOKEN_SECRET)?,
        webhook_secret_hint: secret_hint(connection, WEBHOOK_SECRET)?,
        webhook_url: webhook_url(app),
    })
}

async fn get_settings(State(app): State<App>, user: AuthUser) -> Reply<GithubSettings> {
    user.require_admin()?;
    app.run(|app| app.read(|connection| settings(app, connection))).await.map(Json)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct SettingsInput {
    api_url: Option<String>,
    /// Blank keeps the stored token: the form cannot show what it holds.
    token: Option<String>,
    clear_token: bool,
    webhook_secret: Option<String>,
}

async fn put_settings(State(app): State<App>, user: AuthUser, Json(input): Json<SettingsInput>) -> Reply<GithubSettings> {
    user.require_admin()?;

    app.run(move |app| {
        app.db.write(|tx| {
            if let Some(url) = input.api_url.as_deref().filter(|url| !url.trim().is_empty()) {
                set_setting(tx, API_URL_SETTING, &clean_api_url(url)?)?;
            }

            if input.clear_token {
                clear_secret(tx, TOKEN_SECRET)?;
            } else if let Some(token) = input.token.as_deref().map(str::trim).filter(|token| !token.is_empty()) {
                set_secret(tx, &app.config.secret_key, TOKEN_SECRET, token)?;
            }

            if let Some(secret) = input.webhook_secret.as_deref().map(str::trim).filter(|secret| !secret.is_empty()) {
                set_secret(tx, &app.config.secret_key, WEBHOOK_SECRET, secret)?;
            }

            settings(app, tx)
        })
    })
    .await
    .map(Json)
}

/// Makes a new webhook secret and shows it once, to be pasted into GitHub.
async fn generate_webhook_secret(State(app): State<App>, user: AuthUser) -> Reply<Value> {
    user.require_admin()?;

    let secret = app
        .run(|app| {
            let secret = random_token()?;
            app.db.write(|tx| set_secret(tx, &app.config.secret_key, WEBHOOK_SECRET, &secret))?;

            Ok(secret)
        })
        .await?;

    Ok(Json(json!({ "secret": secret, "webhookUrl": webhook_url(&app) })))
}

async fn deliveries(State(app): State<App>, user: AuthUser) -> Reply<Vec<GithubDelivery>> {
    user.require_admin()?;

    app.run(|app| {
        app.read(|connection| {
            query_all(connection, "SELECT id, event, repo, received_at, outcome, detail FROM gh_delivery ORDER BY id DESC LIMIT 50", [], |row| {
                Ok(GithubDelivery {
                    id: row.get(0)?,
                    event: row.get(1)?,
                    repo: row.get(2)?,
                    received_at: row.get(3)?,
                    outcome: row.get(4)?,
                    detail: row.get(5)?,
                })
            })
        })
    })
    .await
    .map(Json)
}

fn load_project_code(app: &App, connection: &Connection, user: &AuthUser, key: &str) -> ApiResult<GithubProjectCode> {
    let access = project_access(connection, user, key)?;
    let project_id = access.id;
    let pulls = pulls_where(
        connection,
        &format!(
            "(r.project_id = {project_id} OR p.id IN (SELECT f.ref_id FROM gh_ref f JOIN issue i ON i.id = f.issue_id
                                                       WHERE f.kind = 'pr' AND i.project_id = {project_id}))
             AND (p.state = 'open' OR p.updated_at >= datetime('now', '-30 days'))"
        ),
        200,
    )?;
    let drifting: Vec<Id> = query_all(
        connection,
        "SELECT DISTINCT d.issue_id FROM gh_drift d JOIN issue i ON i.id = d.issue_id WHERE i.project_id = ?1",
        [project_id],
        |row| row.get(0),
    )?;
    let kinds = stored_drift(connection, &drifting)?;
    let rows: Vec<IssueRow> = rows_by_ids(app, connection, &drifting)?;

    Ok(GithubProjectCode {
        repos: project_repos(connection, project_id)?,
        rules: project_rules(connection, project_id)?,
        pulls,
        drift: rows.into_iter().map(|issue| GithubDriftItem { kinds: kinds.get(&issue.id).cloned().unwrap_or_default(), issue }).collect(),
        can_admin: access.require_admin().is_ok(),
        token_configured: secret_hint(connection, TOKEN_SECRET)?.is_some(),
        webhook_configured: secret_hint(connection, WEBHOOK_SECRET)?.is_some(),
        webhook_url: webhook_url(app),
    })
}

async fn project_code(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> Reply<GithubProjectCode> {
    app.run(move |app| app.read(|connection| load_project_code(app, connection, &user, &key))).await.map(Json)
}

/// Rules a project starts with when its first repository is connected: a branch starts the
/// work, a pull request puts it in review, a merge finishes it.
pub fn default_rules(connection: &Connection, project_id: Id) -> ApiResult<()> {
    let existing: i64 = connection.query_row("SELECT count(*) FROM gh_rule WHERE project_id = ?1", [project_id], |row| row.get(0))?;

    if existing > 0 {
        return Ok(());
    }

    let pick = |sql: &str| -> ApiResult<Option<Id>> { query_one(connection, sql, [project_id], |row| row.get(0)) };
    let rules = [
        (
            GithubTrigger::BranchCreated,
            pick("SELECT id FROM status WHERE project_id = ?1 AND category = 'in_progress' ORDER BY name = 'In Progress' DESC, position LIMIT 1")?,
        ),
        (GithubTrigger::PrOpened, pick("SELECT id FROM status WHERE project_id = ?1 AND name IN ('In Review', 'Review', 'Code Review') LIMIT 1")?),
        (GithubTrigger::PrMerged, pick("SELECT id FROM status WHERE project_id = ?1 AND category = 'done' ORDER BY position LIMIT 1")?),
    ];

    for (trigger, status_id) in rules {
        if let Some(status_id) = status_id {
            connection
                .execute("INSERT INTO gh_rule (project_id, trigger, status_id) VALUES (?1, ?2, ?3)", (project_id, trigger.as_str(), status_id))?;
        }
    }

    Ok(())
}

pub fn connect_repo(connection: &Connection, project_id: Id, full_name: &str) -> ApiResult<Id> {
    let valid_part = |part: &str| !part.is_empty() && part.chars().all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character));
    let (owner, name) = full_name
        .trim()
        .split_once('/')
        .filter(|(owner, name)| valid_part(owner) && valid_part(name))
        .ok_or_else(|| ApiError::bad_request("A repository is written owner/name, for example acme/web-shop"))?;
    let taken: Option<String> = query_one(
        connection,
        "SELECT p.key FROM gh_repo r JOIN project p ON p.id = r.project_id WHERE r.owner = ?1 AND r.name = ?2",
        (owner, name),
        |row| row.get(0),
    )?;

    if let Some(project) = taken {
        return Err(ApiError::conflict(format!("{owner}/{name} is already connected to project {project}")));
    }

    connection.execute("INSERT INTO gh_repo (project_id, owner, name, created_at) VALUES (?1, ?2, ?3, ?4)", (project_id, owner, name, now_iso()))?;
    let id = connection.last_insert_rowid();
    default_rules(connection, project_id)?;

    Ok(id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoInput {
    full_name: String,
}

async fn add_repo(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<RepoInput>) -> Reply<GithubProjectCode> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let access = project_access(tx, &user, &key)?;
            access.require_admin()?;
            connect_repo(tx, access.id, &input.full_name)?;
            tx.emit("github.synced", Some(access.id), None, json!({ "repo": input.full_name }))?;

            load_project_code(app, tx, &user, &key)
        })
    })
    .await
    .map(Json)
}

async fn remove_repo(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<Value> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let repo = repo_by_id(tx, id)?.ok_or_else(|| ApiError::not_found("Repository"))?;
            project_access_by_id(tx, &user, repo.project_id)?.require_admin()?;

            let issues: Vec<Id> = query_all(
                tx,
                "SELECT DISTINCT issue_id FROM gh_ref WHERE (kind = 'pr' AND ref_id IN (SELECT id FROM gh_pr WHERE repo_id = ?1))
                    OR (kind = 'branch' AND ref_id IN (SELECT id FROM gh_branch WHERE repo_id = ?1))
                    OR (kind = 'commit' AND ref_id IN (SELECT id FROM gh_commit WHERE repo_id = ?1))",
                [id],
                |row| row.get(0),
            )?;

            for (kind, table) in [("pr", "gh_pr"), ("branch", "gh_branch"), ("commit", "gh_commit")] {
                tx.execute(&format!("DELETE FROM gh_ref WHERE kind = ?1 AND ref_id IN (SELECT id FROM {table} WHERE repo_id = ?2)"), (kind, id))?;
            }

            tx.execute("DELETE FROM gh_repo WHERE id = ?1", [id])?;
            tx.execute(&format!("DELETE FROM gh_drift WHERE issue_id IN ({})", id_list(&issues)), [])?;
            recompute_drift(tx, &issues)?;
            tx.emit("github.synced", Some(repo.project_id), None, json!({ "repo": repo.full_name }))
        })
    })
    .await?;

    Ok(Json(Value::Null))
}

async fn set_rules(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(rules): Json<Vec<GithubRule>>) -> Reply<GithubProjectCode> {
    app.run(move |app| {
        app.write(user.actor(), |tx| {
            let access = project_access(tx, &user, &key)?;
            access.require_admin()?;
            tx.execute("DELETE FROM gh_rule WHERE project_id = ?1", [access.id])?;

            for rule in &rules {
                let Some(status_id) = rule.status_id else { continue };
                let owned: Option<Id> =
                    query_one(tx, "SELECT id FROM status WHERE id = ?1 AND project_id = ?2", (status_id, access.id), |row| row.get(0))?;

                if owned.is_none() {
                    return Err(ApiError::bad_request("That status does not belong to this project"));
                }

                tx.execute(
                    "INSERT OR REPLACE INTO gh_rule (project_id, trigger, status_id) VALUES (?1, ?2, ?3)",
                    (access.id, rule.trigger.as_str(), status_id),
                )?;
            }

            load_project_code(app, tx, &user, &key)
        })
    })
    .await
    .map(Json)
}

async fn sync_repo(State(app): State<App>, user: AuthUser, Path(id): Path<Id>) -> Reply<GithubSyncReport> {
    let repo = app
        .run(move |app| {
            app.read(|connection| {
                let repo = repo_by_id(connection, id)?.ok_or_else(|| ApiError::not_found("Repository"))?;
                project_access_by_id(connection, &user, repo.project_id)?.require_edit()?;

                Ok(repo)
            })
        })
        .await?;
    let worker = app.clone();

    tokio::task::spawn_blocking(move || sync_and_record(&worker, &repo)).await.map_err(|_| ApiError::internal("the sync was interrupted"))?.map(Json)
}

pub fn issue_panel(app: &App, connection: &Connection, issue: &IssueRow) -> ApiResult<Option<GithubIssuePanel>> {
    let repos = project_repos(connection, issue.project_id)?;
    let branches = issue_branches(connection, issue.id)?;
    let commits = issue_commits(connection, issue.id)?;
    let prs = issue_pulls(connection, issue.id)?;

    if repos.is_empty() && branches.is_empty() && commits.is_empty() && prs.is_empty() {
        return Ok(None);
    }

    let type_name: String = connection.query_row("SELECT name FROM issue_type WHERE id = ?1", [issue.type_id], |row| row.get(0))?;
    let token_configured = secret_hint(connection, TOKEN_SECRET)?.is_some();
    let _ = app;

    Ok(Some(GithubIssuePanel {
        drift: stored_drift(connection, &[issue.id])?.remove(&issue.id).unwrap_or_default(),
        suggested_branch: suggest_branch_name(&issue.key, &issue.summary, &type_name),
        can_create_branch: token_configured && !repos.is_empty(),
        repos: repos.into_iter().map(|repo| GithubRepoRef { id: repo.id, full_name: repo.full_name }).collect(),
        branches,
        commits,
        prs,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchInput {
    repo_id: Id,
    name: Option<String>,
}

fn valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.chars().all(|character| character.is_ascii_alphanumeric() || "-_./".contains(character))
        && !name.contains("..")
        && !name.starts_with(['/', '-', '.'])
        && !name.ends_with(['/', '.'])
}

/// Creates a branch for the issue on GitHub, from the tip of the default branch.
async fn create_branch(
    State(app): State<App>,
    user: AuthUser,
    Path(key): Path<String>,
    Json(input): Json<BranchInput>,
) -> Reply<Option<GithubIssuePanel>> {
    let reader = user.clone();
    let (issue, repo, name) = app
        .run(move |app| {
            app.read(|connection| {
                let (issue_id, access) = issue_access(connection, &reader, &key)?;
                access.require_edit()?;
                let issue = hub_app::services::issues::row_by_id(app, connection, issue_id)?;
                let repo = repo_by_id(connection, input.repo_id)?
                    .filter(|repo| repo.project_id == issue.project_id)
                    .ok_or_else(|| ApiError::bad_request("That repository is not connected to this issue's project"))?;
                let type_name: String = connection.query_row("SELECT name FROM issue_type WHERE id = ?1", [issue.type_id], |row| row.get(0))?;
                let name = input
                    .name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| suggest_branch_name(&issue.key, &issue.summary, &type_name));

                if !valid_branch_name(name.trim()) {
                    return Err(ApiError::bad_request("A branch name may contain letters, digits, dashes, dots, underscores and slashes"));
                }

                Ok((issue, repo, name.trim().to_string()))
            })
        })
        .await?;

    let worker = app.clone();
    let (full_name, default_branch, branch) = (repo.full_name.clone(), repo.default_branch.clone(), name.clone());
    let sha = tokio::task::spawn_blocking(move || GithubClient::from_app(&worker)?.create_branch(&full_name, &default_branch, &branch))
        .await
        .map_err(|_| ApiError::internal("creating the branch was interrupted"))??;

    app.run(move |app| {
        let created = app.write(user.actor(), |tx| {
            let projects = project_keys(tx)?;
            let (issues, is_new) = upsert_branch(tx, repo.id, &repo.full_name, &name, Some(&sha), &tx.now, &projects)?;

            // A name without the key still belongs to the issue it was created from.
            let branch_id: Id = tx.query_row("SELECT id FROM gh_branch WHERE repo_id = ?1 AND name = ?2", (repo.id, &name), |row| row.get(0))?;
            tx.execute("INSERT OR IGNORE INTO gh_ref (issue_id, kind, ref_id) VALUES (?1, 'branch', ?2)", (issue.id, branch_id))?;
            recompute_drift(tx, &[issue.id])?;
            tx.emit("github.updated", Some(issue.project_id), Some(issue.id), json!({ "repo": repo.full_name, "branch": name }))?;
            let _ = issues;

            Ok(is_new)
        })?;

        if created {
            run_actions(app, vec![Action::Rule { issue_id: issue.id, trigger: GithubTrigger::BranchCreated, closes: false }]);
        }

        app.read(|connection| issue_panel(app, connection, &issue))
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestInput {
    repo_id: Id,
    /// The branch with the work; it has to be on GitHub already.
    head: String,
    title: Option<String>,
    #[serde(default)]
    body: String,
    /// The branch to merge into; the repository's default one when left out.
    base: Option<String>,
    #[serde(default)]
    draft: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenedPullRequest {
    number: i64,
    url: String,
    title: String,
    panel: Option<GithubIssuePanel>,
}

/// Opens a pull request for the issue with the instance's GitHub token: for whoever has
/// pushed a branch and has no GitHub client of their own at hand (the desktop app, an
/// agent's runner). What GitHub answers is taken in like a delivery, so the pull request is
/// on the issue, and the project's rules have run, before the webhook says the same.
async fn create_pull_request(
    State(app): State<App>,
    user: AuthUser,
    Path(key): Path<String>,
    Json(input): Json<PullRequestInput>,
) -> Reply<OpenedPullRequest> {
    let reader = user.clone();
    let (issue, repo, new) = app
        .run(move |app| {
            app.read(|connection| {
                let (issue_id, access) = issue_access(connection, &reader, &key)?;
                access.require_edit()?;
                let issue = hub_app::services::issues::row_by_id(app, connection, issue_id)?;
                let repo = repo_by_id(connection, input.repo_id)?
                    .filter(|repo| repo.project_id == issue.project_id)
                    .ok_or_else(|| ApiError::bad_request("That repository is not connected to this issue's project"))?;
                let head = input.head.trim().to_string();
                let base = input.base.as_deref().map(str::trim).filter(|base| !base.is_empty()).unwrap_or(&repo.default_branch).to_string();

                if !valid_branch_name(&head) || !valid_branch_name(&base) {
                    return Err(ApiError::bad_request("A branch name may contain letters, digits, dashes, dots, underscores and slashes"));
                }

                if head == base {
                    return Err(ApiError::bad_request("A pull request needs a branch other than the one it merges into"));
                }

                let title = input.title.as_deref().map(str::trim).filter(|title| !title.is_empty()).unwrap_or(&issue.summary).to_string();

                if title.chars().count() > 250 || input.body.len() > 60_000 {
                    return Err(ApiError::bad_request("The title or the description is too long"));
                }

                // A pull request belongs to the issues its title or its branch names. One
                // that names neither would not stay on this issue, so the title gets the key.
                let names_issue = |text: &str| text.to_uppercase().contains(&issue.key.to_uppercase());
                let title = if names_issue(&title) || names_issue(&head) { title } else { format!("{} {title}", issue.key) };

                Ok((issue, repo, NewPullRequest { title, body: input.body.clone(), head, base, draft: input.draft }))
            })
        })
        .await?;

    let worker = app.clone();
    let full_name = repo.full_name.clone();
    let opened = tokio::task::spawn_blocking(move || GithubClient::from_app(&worker)?.create_pull_request(&full_name, &new))
        .await
        .map_err(|_| ApiError::internal("opening the pull request was interrupted"))??;
    let number = opened["number"].as_i64().ok_or_else(|| ApiError::upstream("GitHub did not say which pull request it opened"))?;
    let url = opened["html_url"].as_str().map(String::from).unwrap_or_else(|| format!("https://github.com/{}/pull/{number}", repo.full_name));
    let title = opened["title"].as_str().unwrap_or_default().to_string();

    app.run(move |app| {
        let delivery = json!({ "action": "opened", "repository": { "full_name": repo.full_name }, "pull_request": opened });
        let ingested = app.write(user.actor(), |tx| ingest(tx, "pull_request", &delivery))?;

        run_actions(app, ingested.actions);

        let panel = app.read(|connection| issue_panel(app, connection, &issue))?;

        Ok(OpenedPullRequest { number, url, title, panel })
    })
    .await
    .map(Json)
}

fn signature_matches(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(sent) = header.and_then(|header| header.strip_prefix("sha256=")).and_then(|hex_text| hex::decode(hex_text).ok()) else { return false };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else { return false };
    mac.update(body);

    mac.verify_slice(&sent).is_ok()
}

/// The webhook GitHub calls. Only deliveries signed with the configured secret are read;
/// a redelivery of one already handled is acknowledged and ignored.
async fn webhook(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Reply<Value> {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).map(String::from);
    let event = header("x-github-event").ok_or_else(|| ApiError::bad_request("Missing X-GitHub-Event"))?;
    let delivery = header("x-github-delivery");
    let signature = header("x-hub-signature-256");

    app.run(move |app| {
        let secret = app
            .read(|connection| get_secret(connection, &app.config.secret_key, WEBHOOK_SECRET))?
            .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "not_configured", "No webhook secret is configured"))?;

        if !signature_matches(&secret, &body, signature.as_deref()) {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "bad_signature", "The signature does not match"));
        }

        if let Some(delivery) = &delivery {
            let seen: Option<Id> =
                app.read(|connection| query_one(connection, "SELECT id FROM gh_delivery WHERE delivery_id = ?1", [delivery], |row| row.get(0)))?;

            if seen.is_some() {
                return Ok(json!({ "outcome": "duplicate" }));
            }
        }

        let payload: Value = serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("The payload is not JSON"))?;
        let repo = payload["repository"]["full_name"].as_str().map(String::from);
        let ingested = app.write(Actor::system("GitHub"), |tx| ingest(tx, &event, &payload))?;
        let notes = run_actions(app, ingested.actions);

        app.db.write(|tx| {
            Ok(tx.execute(
                "INSERT OR IGNORE INTO gh_delivery (delivery_id, event, repo, received_at, outcome, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (&delivery, &event, &repo, now_iso(), &ingested.outcome, notes.join("; ")),
            )?)
        })?;

        Ok(json!({ "outcome": ingested.outcome, "notes": notes }))
    })
    .await
    .map(Json)
}

pub fn configured_api_url(app: &App) -> ApiResult<String> {
    api_url(app)
}
