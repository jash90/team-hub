//! A project's test settings: which repository holds its suite and how a runner runs it.
//! Plain values are a row of `e2e_settings`; secret values are encrypted in the core's
//! `secret` table and leave the server only for the runner that claims a job.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use hub_app::access::project_access;
use hub_app::db::{query_all, query_one};
use hub_app::secrets::{clear_secret, get_secret, set_secret};
use hub_app::{ApiError, ApiResult, App, AuthUser, Tx};
use hub_core::Id;
use rusqlite::Connection;
use serde::{Deserialize, Deserializer};
use serde_json::json;

use crate::types::{E2eSettings, E2eTraceMode, E2eVideoMode};

const DEFAULT_REF: &str = "main";
const DEFAULT_INSTALL: &str = "npm ci";
const DEFAULT_TIMEOUT_MINUTES: i64 = 30;
const MAX_TIMEOUT_MINUTES: i64 = 240;
const MAX_VARIABLES: usize = 50;
const MAX_NAME_CHARS: usize = 100;
const MAX_VALUE_CHARS: usize = 8000;
const MAX_COMMAND_CHARS: usize = 500;

pub fn routes() -> Router<App> {
    Router::new().route("/projects/{key}/settings", get(read_settings).put(write_settings))
}

/// `Some(None)` for a field sent as `null`, `None` for one not sent.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

/// What `PUT /projects/{key}/settings` takes: the settings, with `secrets` (name → value, or
/// null to remove one) in place of `secretNames`. A field left out keeps what is stored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsInput {
    #[serde(deserialize_with = "present")]
    pub repo: Option<Option<String>>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub directory: Option<String>,
    pub install_command: Option<String>,
    pub run_args: Option<String>,
    pub video: Option<String>,
    pub trace: Option<String>,
    pub env: Option<BTreeMap<String, String>>,
    pub secrets: Option<BTreeMap<String, Option<String>>>,
    pub timeout_minutes: Option<i64>,
}

fn defaults() -> E2eSettings {
    E2eSettings {
        repo: None,
        git_ref: DEFAULT_REF.to_string(),
        directory: String::new(),
        install_command: DEFAULT_INSTALL.to_string(),
        run_args: String::new(),
        video: E2eVideoMode::On,
        trace: E2eTraceMode::On,
        env: BTreeMap::new(),
        secret_names: Vec::new(),
        timeout_minutes: DEFAULT_TIMEOUT_MINUTES,
    }
}

/// Where the core's `secret` table keeps a project's secret of this name.
fn secret_key(project_id: Id, name: &str) -> String {
    format!("{}{name}", secret_prefix(project_id))
}

fn secret_prefix(project_id: Id) -> String {
    format!("e2e:{project_id}:")
}

fn secret_names(connection: &Connection, project_id: Id) -> ApiResult<Vec<String>> {
    let prefix = secret_prefix(project_id);

    query_all(
        connection,
        "SELECT substr(name, ?2) FROM secret WHERE substr(name, 1, ?3) = ?1 ORDER BY name",
        (&prefix, prefix.len() as i64 + 1, prefix.len() as i64),
        |row| row.get(0),
    )
}

/// The project's settings; the defaults, without a repository, when none were saved.
pub fn load(connection: &Connection, project_id: Id) -> ApiResult<E2eSettings> {
    let stored = query_one(
        connection,
        "SELECT repo, git_ref, directory, install_command, run_args, video, trace, env, timeout_minutes FROM e2e_settings WHERE project_id = ?1",
        [project_id],
        |row| {
            let (video, trace, env): (String, String, String) = (row.get(5)?, row.get(6)?, row.get(7)?);

            Ok(E2eSettings {
                repo: row.get(0)?,
                git_ref: row.get(1)?,
                directory: row.get(2)?,
                install_command: row.get(3)?,
                run_args: row.get(4)?,
                video: E2eVideoMode::parse(&video).unwrap_or(E2eVideoMode::On),
                trace: E2eTraceMode::parse(&trace).unwrap_or(E2eTraceMode::On),
                env: serde_json::from_str(&env).unwrap_or_default(),
                secret_names: Vec::new(),
                timeout_minutes: row.get(8)?,
            })
        },
    )?;
    let mut settings = stored.unwrap_or_else(defaults);
    settings.secret_names = secret_names(connection, project_id)?;

    Ok(settings)
}

/// The values of the project's secrets, by name: for the runner that claims a job, nobody else.
pub fn secret_values(app: &App, connection: &Connection, project_id: Id) -> ApiResult<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();

    for name in secret_names(connection, project_id)? {
        if let Some(value) = get_secret(connection, &app.config.secret_key, &secret_key(project_id, &name))? {
            values.insert(name, value);
        }
    }

    Ok(values)
}

fn one_line(text: &str, what: &str, max_chars: usize) -> ApiResult<String> {
    let text = text.trim();

    if text.chars().any(char::is_control) {
        return Err(ApiError::bad_request(format!("{what} must be one line of plain text")));
    }

    if text.chars().count() > max_chars {
        return Err(ApiError::bad_request(format!("{what} can be at most {max_chars} characters")));
    }

    Ok(text.to_string())
}

/// `owner/name`, as GitHub writes a repository; blank for none.
fn checked_repo(text: &str) -> ApiResult<Option<String>> {
    let text = text.trim().trim_end_matches(".git");
    let allowed = |part: &str| !part.is_empty() && part.len() <= 100 && !part.starts_with('.') && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));

    if text.is_empty() {
        return Ok(None);
    }

    match text.split_once('/') {
        Some((owner, name)) if allowed(owner) && allowed(name) => Ok(Some(text.to_string())),
        _ => Err(ApiError::bad_request("Write the repository as owner/name, for example acme/shop")),
    }
}

/// A branch, tag or commit. It ends up on a git command line, so it cannot look like an option.
pub fn checked_ref(text: &str) -> ApiResult<String> {
    let text = one_line(text, "The ref", 200)?;

    if text.is_empty() || text.starts_with('-') || text.chars().any(char::is_whitespace) {
        return Err(ApiError::bad_request("The ref is a branch, a tag or a commit, without spaces"));
    }

    Ok(text)
}

/// A folder inside the repository, written from its root.
fn checked_directory(text: &str) -> ApiResult<String> {
    let text = one_line(text, "The directory", 300)?;
    let text = text.trim_start_matches("./").trim_end_matches('/');

    if text.starts_with('/') || text.contains('\\') || text.split('/').any(|part| part == ".." || part.is_empty() && !text.is_empty()) {
        return Err(ApiError::bad_request("The directory is a path inside the repository, such as e2e or apps/web"));
    }

    Ok(text.to_string())
}

/// A name a shell can export: letters, digits and `_`, not starting with a digit.
fn checked_name(name: &str) -> ApiResult<()> {
    let valid = !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());

    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!("\"{name}\" cannot name a variable: use letters, digits and _, not starting with a digit")))
    }
}

fn checked_value(name: &str, value: &str) -> ApiResult<()> {
    if value.contains('\0') || value.chars().count() > MAX_VALUE_CHARS {
        return Err(ApiError::bad_request(format!("The value of {name} can be at most {MAX_VALUE_CHARS} characters")));
    }

    Ok(())
}

pub fn checked_video(text: &str) -> ApiResult<E2eVideoMode> {
    E2eVideoMode::parse(text.trim()).ok_or_else(|| ApiError::bad_request("The video mode is one of: on, retain-on-failure, on-first-retry, off"))
}

pub fn checked_trace(text: &str) -> ApiResult<E2eTraceMode> {
    E2eTraceMode::parse(text.trim()).ok_or_else(|| ApiError::bad_request("The trace mode is one of: on, retain-on-failure, off"))
}

/// `current` with what `input` says, each field checked.
fn merged(current: E2eSettings, input: &SettingsInput) -> ApiResult<E2eSettings> {
    let mut settings = current;

    if let Some(repo) = &input.repo {
        settings.repo = checked_repo(repo.as_deref().unwrap_or(""))?;
    }

    if let Some(git_ref) = &input.git_ref {
        settings.git_ref = if git_ref.trim().is_empty() { DEFAULT_REF.to_string() } else { checked_ref(git_ref)? };
    }

    if let Some(directory) = &input.directory {
        settings.directory = checked_directory(directory)?;
    }

    if let Some(command) = &input.install_command {
        let command = one_line(command, "The install command", MAX_COMMAND_CHARS)?;
        settings.install_command = if command.is_empty() { DEFAULT_INSTALL.to_string() } else { command };
    }

    if let Some(arguments) = &input.run_args {
        settings.run_args = one_line(arguments, "The run arguments", MAX_COMMAND_CHARS)?;
    }

    if let Some(video) = &input.video {
        settings.video = checked_video(video)?;
    }

    if let Some(trace) = &input.trace {
        settings.trace = checked_trace(trace)?;
    }

    if let Some(env) = &input.env {
        for (name, value) in env {
            checked_name(name)?;
            checked_value(name, value)?;
        }

        settings.env = env.clone();
    }

    if let Some(minutes) = input.timeout_minutes {
        if !(1..=MAX_TIMEOUT_MINUTES).contains(&minutes) {
            return Err(ApiError::bad_request(format!("The timeout is between 1 and {MAX_TIMEOUT_MINUTES} minutes")));
        }

        settings.timeout_minutes = minutes;
    }

    Ok(settings)
}

/// Stores, replaces and removes the secrets `changes` names, and leaves the others alone.
fn save_secrets(app: &App, tx: &Tx, project_id: Id, settings: &E2eSettings, changes: &BTreeMap<String, Option<String>>) -> ApiResult<()> {
    for (name, value) in changes {
        checked_name(name)?;

        match value.as_deref() {
            None => clear_secret(tx, &secret_key(project_id, name))?,
            Some("") => return Err(ApiError::bad_request(format!("The secret {name} has no value; leave it out to keep the stored one"))),
            Some(value) => {
                checked_value(name, value)?;
                set_secret(tx, &app.config.secret_key, &secret_key(project_id, name), value)?;
            }
        }
    }

    let names = secret_names(tx, project_id)?;

    if let Some(twice) = names.iter().find(|name| settings.env.contains_key(*name)) {
        return Err(ApiError::bad_request(format!("{twice} is both a variable and a secret; keep one of them")));
    }

    if names.len() > MAX_VARIABLES || settings.env.len() > MAX_VARIABLES {
        return Err(ApiError::bad_request(format!("A project can have at most {MAX_VARIABLES} variables and {MAX_VARIABLES} secrets")));
    }

    Ok(())
}

/// Saves a project administrator's changes and answers with the settings as they now are.
pub fn save(app: &App, tx: &Tx, user: &AuthUser, project_key: &str, input: &SettingsInput) -> ApiResult<E2eSettings> {
    let access = project_access(tx, user, project_key)?;
    access.require_admin()?;

    let settings = merged(load(tx, access.id)?, input)?;

    tx.execute(
        "INSERT INTO e2e_settings (project_id, repo, git_ref, directory, install_command, run_args, video, trace, env, timeout_minutes, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (project_id) DO UPDATE SET repo = ?2, git_ref = ?3, directory = ?4, install_command = ?5, run_args = ?6, video = ?7, trace = ?8,
                                                env = ?9, timeout_minutes = ?10, updated_at = ?11",
        rusqlite::params![
            access.id,
            settings.repo,
            settings.git_ref,
            settings.directory,
            settings.install_command,
            settings.run_args,
            settings.video.as_str(),
            settings.trace.as_str(),
            serde_json::to_string(&settings.env)?,
            settings.timeout_minutes,
            &tx.now,
        ],
    )?;
    save_secrets(app, tx, access.id, &settings, input.secrets.as_ref().unwrap_or(&BTreeMap::new()))?;
    tx.emit("e2e.settings", Some(access.id), None, json!({ "repo": settings.repo }))?;

    load(tx, access.id)
}

async fn read_settings(State(app): State<App>, user: AuthUser, Path(key): Path<String>) -> ApiResult<Json<E2eSettings>> {
    app.run(move |app| app.read(|connection| load(connection, project_access(connection, &user, &key)?.id))).await.map(Json)
}

async fn write_settings(State(app): State<App>, user: AuthUser, Path(key): Path<String>, Json(input): Json<SettingsInput>) -> ApiResult<Json<E2eSettings>> {
    app.run(move |app| app.write(user.actor(), |tx| save(app, tx, &user, &key, &input))).await.map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories_refs_and_directories_are_checked() {
        assert_eq!(checked_repo(" acme/shop.git ").unwrap().as_deref(), Some("acme/shop"));
        assert_eq!(checked_repo("").unwrap(), None);
        assert!(checked_repo("https://github.com/acme/shop").is_err());
        assert!(checked_repo("acme/../shop").is_err());

        assert_eq!(checked_ref("feature/WEB-9").unwrap(), "feature/WEB-9");
        // It must not read as an option of git.
        assert!(checked_ref("--upload-pack=x").is_err());
        assert!(checked_ref("main branch").is_err());

        assert_eq!(checked_directory("./apps/web/").unwrap(), "apps/web");
        assert_eq!(checked_directory("").unwrap(), "");
        assert!(checked_directory("../other").is_err());
        assert!(checked_directory("/etc").is_err());
        assert!(checked_directory("a//b").is_err());
    }
}
