//! The GitHub REST client: backfilling a repository through the API and creating branches.
//! Everything here blocks on the network, so it runs on a plain thread, never on the worker pool.

use std::time::Duration;

use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use hub_app::app::get_setting;
use hub_app::db::now_iso;
use hub_app::secrets::get_secret;
use hub_app::{Actor, ApiError, ApiResult, App};
use serde_json::{json, Value};

use crate::drift::recompute_all;
use crate::ingest::{normalize_time, project_keys, upsert_branch, upsert_commits, upsert_pr, CommitAuthor, NewCommit};
use crate::model::{GithubRepo, GithubSyncReport};

pub const DEFAULT_API_URL: &str = "https://api.github.com";
pub const API_URL_SETTING: &str = "github:api_url";
pub const TOKEN_SECRET: &str = "github.token";
pub const WEBHOOK_SECRET: &str = "github.webhook_secret";

const ATTEMPTS: u32 = 3;
const PULL_PAGES: usize = 3;
const BACKFILL_DAYS: i64 = 30;

pub struct GithubClient {
    agent: ureq::Agent,
    api_url: String,
    token: String,
}

/// The token is sent to this address, so it must be HTTPS (or this machine, for tests and
/// a local GitHub Enterprise).
pub fn clean_api_url(url: &str) -> ApiResult<String> {
    let url = url.trim().trim_end_matches('/');
    let local = ["http://127.0.0.1", "http://localhost"].iter().any(|prefix| url == *prefix || url.starts_with(&format!("{prefix}:")));

    if url.starts_with("https://") || local {
        Ok(url.to_string())
    } else {
        Err(ApiError::bad_request("The GitHub API address must start with https://"))
    }
}

pub fn api_url(app: &App) -> ApiResult<String> {
    Ok(app.read(|connection| get_setting::<String>(connection, API_URL_SETTING))?.unwrap_or_else(|| DEFAULT_API_URL.to_string()))
}

impl GithubClient {
    pub fn from_app(app: &App) -> ApiResult<Self> {
        let token = app
            .read(|connection| get_secret(connection, &app.config.secret_key, TOKEN_SECRET))?
            .ok_or_else(|| ApiError::conflict("No GitHub token is configured (Settings → GitHub)"))?;
        let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build();

        Ok(Self { agent: ureq::Agent::new_with_config(config), api_url: api_url(app)?, token })
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> ApiResult<Value> {
        let url = format!("{}{path}", self.api_url);

        for attempt in 1..=ATTEMPTS {
            let sent = match body {
                Some(body) => self
                    .agent
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.token))
                    .header("Accept", "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .header("User-Agent", "team-hub")
                    .send_json(body),
                None => self
                    .agent
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", self.token))
                    .header("Accept", "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .header("User-Agent", "team-hub")
                    .call(),
            };
            let mut response = match sent {
                Ok(response) => response,
                Err(_) if attempt < ATTEMPTS => {
                    std::thread::sleep(Duration::from_millis(300 * u64::from(attempt)));
                    continue;
                }
                Err(error) => return Err(ApiError::upstream(format!("GitHub is unreachable: {error}"))),
            };
            let status = response.status().as_u16();

            if status == 401 {
                return Err(ApiError::upstream("GitHub rejected the token"));
            }

            if (status == 429 || status >= 500) && attempt < ATTEMPTS {
                std::thread::sleep(Duration::from_millis(500 * u64::from(attempt)));
                continue;
            }

            let text = response.body_mut().read_to_string().unwrap_or_default();
            let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

            if !(200..300).contains(&status) {
                let message = value.get("message").and_then(Value::as_str).unwrap_or("no details");

                return Err(ApiError::upstream(format!("GitHub answered {status} to {method} {path}: {message}")));
            }

            return Ok(value);
        }

        Err(ApiError::upstream("GitHub kept failing"))
    }

    fn get(&self, path: &str) -> ApiResult<Value> {
        self.request("GET", path, None)
    }

    /// Creates `branch` from the tip of the default branch and returns the commit it points at.
    pub fn create_branch(&self, full_name: &str, default_branch: &str, branch: &str) -> ApiResult<String> {
        let base = self.get(&format!("/repos/{full_name}/git/ref/heads/{default_branch}"))?;
        let sha = base["object"]["sha"].as_str().ok_or_else(|| ApiError::upstream("GitHub did not say where the default branch is"))?.to_string();

        self.request("POST", &format!("/repos/{full_name}/git/refs"), Some(&json!({ "ref": format!("refs/heads/{branch}"), "sha": sha })))?;

        Ok(sha)
    }

    fn review_state(&self, full_name: &str, number: i64) -> ApiResult<&'static str> {
        let reviews = self.get(&format!("/repos/{full_name}/pulls/{number}/reviews?per_page=100"))?;
        let mut latest: Vec<(String, String)> = Vec::new();

        for review in reviews.as_array().into_iter().flatten() {
            let (Some(login), Some(state)) = (review["user"]["login"].as_str(), review["state"].as_str()) else { continue };

            if state != "APPROVED" && state != "CHANGES_REQUESTED" && state != "DISMISSED" {
                continue;
            }

            latest.retain(|(user, _)| user != login);
            latest.push((login.to_string(), state.to_string()));
        }

        Ok(if latest.iter().any(|(_, state)| state == "CHANGES_REQUESTED") {
            "changes_requested"
        } else if latest.iter().any(|(_, state)| state == "APPROVED") {
            "approved"
        } else {
            "pending"
        })
    }

    fn check_state(&self, full_name: &str, sha: &str) -> ApiResult<Option<&'static str>> {
        let runs = self.get(&format!("/repos/{full_name}/commits/{sha}/check-runs?per_page=100"))?;
        let runs = runs["check_runs"].as_array().cloned().unwrap_or_default();

        if runs.is_empty() {
            return Ok(None);
        }

        let failed =
            |run: &Value| matches!(run["conclusion"].as_str(), Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure"));
        let running = |run: &Value| run["status"].as_str() != Some("completed");

        Ok(Some(if runs.iter().any(failed) {
            "failing"
        } else if runs.iter().any(running) {
            "running"
        } else {
            "passing"
        }))
    }

    /// Reads a repository's pull requests, branches and recent commits and links them to
    /// issues. What webhooks deliver as it happens, this catches up on.
    pub fn sync_repo(&self, app: &App, repo: &GithubRepo) -> ApiResult<GithubSyncReport> {
        let full_name = &repo.full_name;
        let details = self.get(&format!("/repos/{full_name}"))?;
        let default_branch = details["default_branch"].as_str().unwrap_or("main").to_string();
        let mut pulls: Vec<Value> = Vec::new();

        for page in 1..=PULL_PAGES {
            let batch = self.get(&format!("/repos/{full_name}/pulls?state=all&sort=updated&direction=desc&per_page=100&page={page}"))?;
            let batch = batch.as_array().cloned().unwrap_or_default();
            let last = batch.len() < 100;
            pulls.extend(batch);

            if last {
                break;
            }
        }

        let mut verdicts: Vec<(i64, &'static str, Option<&'static str>)> = Vec::new();

        for pull in pulls.iter().filter(|pull| pull["state"].as_str() == Some("open")) {
            let Some(number) = pull["number"].as_i64() else { continue };
            let checks = match pull["head"]["sha"].as_str() {
                Some(sha) => self.check_state(full_name, sha)?,
                None => None,
            };

            verdicts.push((number, self.review_state(full_name, number)?, checks));
        }

        let branches = self.get(&format!("/repos/{full_name}/branches?per_page=100"))?.as_array().cloned().unwrap_or_default();
        let since = repo
            .last_synced_at
            .clone()
            .unwrap_or_else(|| (Utc::now() - ChronoDuration::days(BACKFILL_DAYS)).to_rfc3339_opts(SecondsFormat::Secs, true));
        let since = format!("{}Z", &since[..since.len().min(19)]).replace(':', "%3A");
        let commits = self.get(&format!("/repos/{full_name}/commits?per_page=100&since={since}"))?;
        let commits = commits.as_array().cloned().unwrap_or_default();

        app.write(Actor::system("GitHub"), |tx| {
            let projects = project_keys(tx)?;
            let mut report = GithubSyncReport::default();
            let mut touched: Vec<i64> = Vec::new();

            for pull in &pulls {
                if let Some(stored) = upsert_pr(tx, repo.id, full_name, pull, &projects)? {
                    report.pulls += 1;
                    touched.extend(stored.issues);
                }
            }

            for (number, review, checks) in &verdicts {
                tx.execute(
                    "UPDATE gh_pr SET review = ?1, checks = coalesce(?2, checks) WHERE repo_id = ?3 AND number = ?4",
                    (review, checks, repo.id, number),
                )?;
            }

            for branch in &branches {
                let Some(name) = branch["name"].as_str() else { continue };
                let (issues, _) = upsert_branch(tx, repo.id, full_name, name, branch["commit"]["sha"].as_str(), &tx.now, &projects)?;

                report.branches += 1;
                touched.extend(issues);
            }

            let new_commits: Vec<NewCommit> = commits
                .iter()
                .filter_map(|commit| {
                    Some(NewCommit {
                        sha: commit["sha"].as_str()?,
                        message: commit["commit"]["message"].as_str()?,
                        url: commit["html_url"].as_str(),
                        at: commit["commit"]["author"]["date"].as_str().map(normalize_time).unwrap_or_else(now_iso),
                        author: CommitAuthor {
                            name: commit["commit"]["author"]["name"].as_str().map(String::from),
                            login: commit["author"]["login"].as_str().map(String::from),
                            email: commit["commit"]["author"]["email"].as_str().map(String::from),
                        },
                    })
                })
                .collect();
            // A backfill only links: commands in old commit messages are not replayed.
            let (issues, _, stored) = upsert_commits(tx, repo.id, full_name, &new_commits, &projects)?;

            report.commits = stored;
            touched.extend(issues);
            touched.sort_unstable();
            touched.dedup();
            report.linked_issues = touched.len() as i64;

            tx.execute(
                "UPDATE gh_repo SET default_branch = ?1, last_synced_at = ?2, sync_error = NULL WHERE id = ?3",
                (&default_branch, &tx.now, repo.id),
            )?;
            recompute_all(tx)?;
            tx.emit("github.synced", Some(repo.project_id), None, json!({ "repo": full_name }))?;

            Ok(report)
        })
    }
}

/// Syncs one repository and records a failure on it instead of losing it.
pub fn sync_and_record(app: &App, repo: &GithubRepo) -> ApiResult<GithubSyncReport> {
    let outcome = GithubClient::from_app(app).and_then(|client| client.sync_repo(app, repo));

    if let Err(error) = &outcome {
        let _ = app.db.write(|tx| Ok(tx.execute("UPDATE gh_repo SET sync_error = ?1 WHERE id = ?2", (&error.message, repo.id))?));
    }

    outcome
}
