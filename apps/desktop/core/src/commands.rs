//! The commands of the window, by name. The Tauri crate forwards `(name, arguments)` here and
//! does nothing else, and the development server does the same over HTTP, so what a browser
//! exercises is exactly what the window runs.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::Desktop;
use crate::config::MODELS;
use crate::error::{CoreError, CoreResult};
use crate::model::{Delegation, NewTestJob, NewThread, Settings};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connect {
    url: String,
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerUrl {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ByKey {
    key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ById {
    id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Query {
    query: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentOn {
    key: String,
    body: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Transition {
    key: String,
    status_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogWork {
    key: String,
    spent: String,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkRead {
    #[serde(default)]
    id: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoPath {
    path: String,
    #[serde(default)]
    github_repo: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MapRepo {
    project_key: String,
    path: String,
    #[serde(default)]
    github_repo: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Say {
    id: i64,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobLog {
    id: i64,
    #[serde(default)]
    after: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Decide {
    id: i64,
    approve: bool,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Push {
    id: i64,
    commit: String,
    #[serde(default)]
    confirmed_findings: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenPullRequest {
    id: i64,
    title: String,
    #[serde(default)]
    body: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordPullRequest {
    id: i64,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Runs {
    #[serde(default)]
    mine: bool,
}

fn arguments<T: DeserializeOwned>(command: &str, value: Value) -> CoreResult<T> {
    let value = if value.is_null() { Value::Object(Default::default()) } else { value };

    serde_json::from_value(value).map_err(|error| CoreError::not_possible(format!("{command}: {error}")))
}

fn answer<T: Serialize>(value: CoreResult<T>) -> CoreResult<Value> {
    Ok(serde_json::to_value(value?)?)
}

/// Every command the interface may call.
pub const COMMANDS: [&str; 40] = [
    "status",
    "connect",
    "disconnect",
    "pair_start",
    "pair_wait",
    "pair_cancel",
    "settings",
    "save_settings",
    "models",
    "workspace",
    "dashboard",
    "issue",
    "search",
    "comment",
    "transition",
    "assign_to_me",
    "log_work",
    "notifications",
    "mark_notifications_read",
    "project_code",
    "repos",
    "check_repo",
    "map_repo",
    "unmap_repo",
    "threads",
    "thread",
    "start_thread",
    "stop_thread",
    "say_to_thread",
    "thread_diff",
    "remove_worktree",
    "forget_thread",
    "pending_proposals",
    "decide_proposal",
    "push_preview",
    "push",
    "pull_request_preview",
    "open_pull_request",
    "record_pull_request",
    "runs",
];

/// Runs one command and returns its answer as JSON. Errors are text for the person.
pub async fn invoke(desktop: &Desktop, command: &str, args: Value) -> Result<Value, String> {
    dispatch(desktop, command, args).await.map_err(|error| error.to_string())
}

async fn dispatch(desktop: &Desktop, command: &str, args: Value) -> CoreResult<Value> {
    match command {
        "status" => desktop.run(|core| answer(Ok(core.status()))).await,
        "connect" => {
            let Connect { url, token } = arguments(command, args)?;

            desktop.run(move |core| answer(core.connect(&url, &token))).await
        }
        "disconnect" => desktop.run(|core| answer(core.disconnect())).await,
        // Signing in from the window: docs/desktop-pairing.md.
        "pair_start" => {
            let ServerUrl { url } = arguments(command, args)?;

            desktop.run(move |core| answer(core.pair_start(&url))).await
        }
        "pair_wait" => desktop.run(|core| answer(core.pair_wait())).await,
        "pair_cancel" => {
            desktop.run(|core| {
                core.pair_cancel();
                answer(Ok(()))
            })
            .await
        }
        "settings" => desktop.run(|core| answer(Ok(core.settings()))).await,
        "save_settings" => {
            let settings: Settings = arguments(command, args)?;

            desktop.run(move |core| answer(core.save_settings(settings))).await
        }
        "models" => answer(Ok(MODELS)),
        "workspace" => desktop.run(|core| answer(core.workspace())).await,
        "dashboard" => desktop.run(|core| answer(core.dashboard())).await,
        "issue" => {
            let ByKey { key } = arguments(command, args)?;

            desktop.run(move |core| answer(core.issue(&key))).await
        }
        "search" => {
            let Query { query } = arguments(command, args)?;

            desktop.run(move |core| answer(core.search(&query))).await
        }
        "comment" => {
            let CommentOn { key, body } = arguments(command, args)?;

            desktop.run(move |core| answer(core.comment(&key, &body))).await
        }
        "transition" => {
            let Transition { key, status_id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.transition(&key, status_id))).await
        }
        "assign_to_me" => {
            let ByKey { key } = arguments(command, args)?;

            desktop.run(move |core| answer(core.assign_to_me(&key))).await
        }
        "log_work" => {
            let LogWork { key, spent, note } = arguments(command, args)?;

            desktop.run(move |core| answer(core.log_work(&key, &spent, &note))).await
        }
        "notifications" => desktop.run(|core| answer(core.notifications())).await,
        "mark_notifications_read" => {
            let MarkRead { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.mark_notifications_read(id))).await
        }
        "project_code" => {
            let ByKey { key } = arguments(command, args)?;

            desktop.run(move |core| answer(core.project_code(&key))).await
        }
        "repos" => desktop.run(|core| answer(core.repos())).await,
        "check_repo" => {
            let RepoPath { path, github_repo } = arguments(command, args)?;

            desktop.run(move |core| answer(Ok(core.check_repo(&path, github_repo.as_deref())))).await
        }
        "map_repo" => {
            let MapRepo { project_key, path, github_repo } = arguments(command, args)?;

            desktop.run(move |core| answer(core.map_repo(&project_key, github_repo.as_deref(), &path))).await
        }
        "unmap_repo" => {
            let ById { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.unmap_repo(id))).await
        }
        "threads" => desktop.run(|core| answer(core.threads())).await,
        "thread" => {
            let ById { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.thread(id))).await
        }
        "start_thread" => {
            let new: NewThread = arguments(command, args)?;

            desktop.run(move |core| answer(core.start_thread(&new))).await
        }
        "stop_thread" => {
            let ById { id } = arguments(command, args)?;

            answer(desktop.stop_thread(id).await)
        }
        "say_to_thread" => {
            let Say { id, text } = arguments(command, args)?;

            desktop.run(move |core| answer(core.say_to_thread(id, text.as_deref()))).await
        }
        "thread_diff" => {
            let ById { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.thread_diff(id))).await
        }
        "remove_worktree" => {
            let ById { id } = arguments(command, args)?;

            answer(desktop.remove_worktree(id).await)
        }
        "forget_thread" => {
            let ById { id } = arguments(command, args)?;

            answer(desktop.forget_thread(id).await)
        }
        "pending_proposals" => desktop.run(|core| answer(core.pending_proposals())).await,
        "decide_proposal" => {
            let Decide { id, approve, note } = arguments(command, args)?;

            desktop.run(move |core| answer(core.decide_proposal(id, approve, note.as_deref()))).await
        }
        "push_preview" => {
            let ById { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.push_preview(id))).await
        }
        "push" => {
            let Push { id, commit, confirmed_findings } = arguments(command, args)?;

            desktop.run(move |core| answer(core.push(id, &commit, confirmed_findings))).await
        }
        "pull_request_preview" => {
            let ById { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.pull_request_preview(id))).await
        }
        "open_pull_request" => {
            let OpenPullRequest { id, title, body } = arguments(command, args)?;

            desktop.run(move |core| answer(core.open_pull_request(id, &title, &body))).await
        }
        "record_pull_request" => {
            let RecordPullRequest { id, url } = arguments(command, args)?;

            desktop.run(move |core| answer(core.record_pull_request(id, &url))).await
        }
        "runs" => {
            let Runs { mine } = arguments(command, args)?;

            desktop.run(move |core| answer(core.runs(mine))).await
        }
        // The same runners and work as the web shows: docs/runners.md.
        "runners" => desktop.run(|core| answer(core.runners())).await,
        "delegate" => {
            let new: Delegation = arguments(command, args)?;

            desktop.run(move |core| answer(core.delegate(&new))).await
        }
        "run_detail" => {
            let ById { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.run_detail(id))).await
        }
        "decide_on_run" => {
            let Decide { id, approve, note } = arguments(command, args)?;

            desktop.run(move |core| answer(core.decide_on_run(id, approve, note.as_deref()))).await
        }
        "cancel_run" => {
            let ById { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.cancel_run(id))).await
        }
        "test_settings" => {
            let ByKey { key } = arguments(command, args)?;

            desktop.run(move |core| answer(core.test_settings(&key))).await
        }
        "test_jobs" => {
            let ByKey { key } = arguments(command, args)?;

            desktop.run(move |core| answer(core.test_jobs(&key))).await
        }
        "start_test_job" => {
            let new: NewTestJob = arguments(command, args)?;

            desktop.run(move |core| answer(core.start_test_job(&new))).await
        }
        "test_job" => {
            let JobLog { id, after } = arguments(command, args)?;

            desktop.run(move |core| answer(core.test_job(id, after))).await
        }
        "cancel_test_job" => {
            let ById { id } = arguments(command, args)?;

            desktop.run(move |core| answer(core.cancel_test_job(id))).await
        }
        other => Err(CoreError::not_possible(format!("There is no command called {other}"))),
    }
}
