//! What the tests of the headless runner add to the fake team-hub: the web side of
//! `docs/agent-runs.md` § Headless runners, test jobs as `docs/e2e-jobs.md` describes them,
//! a stand-in for GitHub that serves git over HTTP and wants a token, and a headless core.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{patch, post, put};
use axum::{Json, Router};
use hub_desktop_core::headless::RunnerEnv;
use hub_desktop_core::jobs::{JobOptions, JobRunner, JobUser};
use hub_desktop_core::model::{AppEvent, SandboxState};
use hub_desktop_core::{Desktop, OpenOptions};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::{broadcast, watch};

use super::{error, fake_agent, git, write, Bench, FakeHub, HubState, Shared, TempDir, NOW, TOKEN};

pub const GITHUB_TOKEN: &str = "ghp_test0123456789abcdefghijklmnopqrstuv";
pub const MODEL_KEY: &str = "sk-ant-test-0123456789-not-a-real-key";
/// The keys one person stored for themselves, as a shared runner is given them.
pub const PERSON_GITHUB_TOKEN: &str = "ghp_person0123456789abcdefghijklmnopqrs";
pub const PERSON_MODEL_KEY: &str = "sk-ant-person-0123456789-not-a-real-key";

// ---------- headless runners: what the web decides ----------

pub async fn put_changes(State(state): Shared, headers: HeaderMap, UrlPath(id): UrlPath<i64>, Json(body): Json<Value>) -> Response {
    let mut state = guard!(runs state, headers);
    let Some(index) = state.run_index(id) else { return error(StatusCode::NOT_FOUND, "No such run") };

    if super::finished(&state.runs[index]) {
        return error(StatusCode::CONFLICT, "The run is finished");
    }

    state.writes.push(("PUT".into(), format!("/runs/{id}/changes"), json!({ "head": body["head"] })));
    state.changes.insert(id, body);

    Json(json!({ "ok": true })).into_response()
}

pub async fn get_changes(State(state): Shared, headers: HeaderMap, UrlPath(id): UrlPath<i64>) -> Response {
    let state = guard!(runs state, headers);

    match state.changes.get(&id) {
        Some(changes) => Json(changes.clone()).into_response(),
        None => error(StatusCode::NOT_FOUND, "This run has no changes"),
    }
}

/// The runner says how an approved push or pull request went.
pub async fn report_result(State(state): Shared, headers: HeaderMap, UrlPath(id): UrlPath<i64>, Json(body): Json<Value>) -> Response {
    let mut state = guard!(runs state, headers);
    let Some(index) = state.proposals.iter().position(|proposal| proposal["id"] == id) else {
        return error(StatusCode::NOT_FOUND, "No such proposal");
    };
    let proposal = state.proposals[index].clone();
    let run_id = proposal["runId"].as_i64().unwrap();

    if (proposal["kind"] != "push" && proposal["kind"] != "pull_request") || !state.held_headless(run_id) {
        return error(StatusCode::BAD_REQUEST, "Only a headless runner reports, and only on a push or a pull request");
    }

    if proposal["state"] != "approved" || !proposal["result"].is_null() {
        return error(StatusCode::CONFLICT, "This proposal is not waiting for its runner");
    }

    state.writes.push(("POST".into(), format!("/proposals/{id}/result"), body.clone()));

    match body["error"].as_str() {
        Some(reason) => {
            state.proposals[index]["state"] = json!("failed");
            state.proposals[index]["note"] = json!(reason);
        }
        None => state.proposals[index]["result"] = body["result"].clone(),
    }

    Json(state.detail(run_id)).into_response()
}

impl FakeHub {
    /// A person deciding a push or a pull request of a headless runner's run on the web.
    pub async fn decide_like_the_web(&self, proposal_id: i64, approve: bool) -> reqwest_free::Answer {
        reqwest_free::post(&format!("{}/api/m/agents/proposals/{proposal_id}/decide", self.url), &json!({ "approve": approve, "note": null })).await
    }

    pub fn proposal_on_run(&self, run_id: i64, kind: &str) -> Value {
        self.run(run_id)["proposals"].as_array().unwrap().iter().rev().find(|proposal| proposal["kind"] == kind).cloned().unwrap_or(Value::Null)
    }

    /// Paths of the requests that changed something, in order.
    pub fn written(&self) -> Vec<String> {
        self.state().writes.iter().map(|(method, path, _)| format!("{method} {path}")).collect()
    }
}

/// The one HTTP call the tests make themselves, as the web interface would: a POST with the
/// person's token. Written on the core's own client so the tests need no other.
pub mod reqwest_free {
    use hub_desktop_core::api::HubClient;
    use hub_desktop_core::secrets::Secret;
    use serde_json::Value;

    pub type Answer = Result<Value, String>;

    pub async fn post(url: &str, body: &Value) -> Answer {
        let (base, path) = url.split_at(url.find("/api/").unwrap());
        let (client, path, body) = (HubClient::new(base, Secret::new(super::TOKEN)), path.to_string(), body.clone());

        tokio::task::spawn_blocking(move || client.post::<Value>(&path, &body).map_err(|error| error.to_string())).await.unwrap()
    }
}

// ---------- test jobs: docs/e2e-jobs.md ----------

#[derive(Default)]
pub struct E2eState {
    /// False: a server from before test jobs.
    pub off: bool,
    pub settings: Value,
    pub secrets: BTreeMap<String, String>,
    pub jobs: Vec<Value>,
    /// Job id → its log lines, as received.
    pub logs: BTreeMap<i64, Vec<String>>,
    /// How many lines each call to the log brought.
    pub log_batches: Vec<usize>,
    /// Run id → the report as posted.
    pub reports: BTreeMap<i64, Value>,
    /// (run id, path, content type, bytes).
    pub artifacts: Vec<(i64, String, String, Vec<u8>)>,
    /// The bodies of the claims, as sent.
    pub claims: Vec<Value>,
}

fn job_index(state: &HubState, id: i64) -> Option<usize> {
    state.e2e.jobs.iter().position(|job| job["id"] == id)
}

async fn claim_job(State(state): Shared, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let mut state = guard!(state, headers);

    if state.e2e.off {
        return error(StatusCode::NOT_FOUND, "Not found");
    }

    if body["shared"] == true && state.refuses_shared {
        return error(StatusCode::FORBIDDEN, "Only an administrator's token can run a shared runner");
    }

    state.e2e.claims.push(body.clone());

    let projects = body["projects"].as_array().cloned();
    let pinned_only = body["pinnedOnly"] == true;
    let found = state.e2e.jobs.iter().position(|job| {
        let pinned_here = job["onRunner"] == body["runnerId"];

        job["state"] == "queued"
            && projects.as_ref().is_none_or(|projects| projects.contains(&job["projectKey"]))
            && (pinned_here || (job["onRunner"].is_null() && !pinned_only))
    });
    let Some(index) = found else { return Json(Value::Null).into_response() };

    state.e2e.jobs[index]["state"] = json!("claimed");
    state.e2e.jobs[index]["runnerName"] = body["runnerName"].clone();
    state.e2e.jobs[index]["runnerId"] = body["runnerId"].clone();

    let mut settings = state.e2e.settings.clone();

    settings["secrets"] = json!(state.e2e.secrets);

    let mut answer = json!({ "job": state.e2e.jobs[index], "settings": settings });

    if body["shared"] == true && !state.person_keys["githubToken"].is_null() {
        answer["credentials"] = json!({ "githubToken": state.person_keys["githubToken"] });
    }

    Json(answer).into_response()
}

async fn patch_job(State(state): Shared, headers: HeaderMap, UrlPath(id): UrlPath<i64>, Json(body): Json<Value>) -> Response {
    let mut state = guard!(state, headers);
    let Some(index) = job_index(&state, id) else { return error(StatusCode::NOT_FOUND, "No such job") };

    if matches!(state.e2e.jobs[index]["state"].as_str(), Some("succeeded" | "failed" | "cancelled")) {
        return error(StatusCode::CONFLICT, "The job is finished");
    }

    if !body.as_object().is_some_and(|fields| fields.keys().all(|name| ["state", "step", "commit", "error"].contains(&name.as_str()))) {
        return error(StatusCode::BAD_REQUEST, "A job takes state, step, commit and error");
    }

    if !body["state"].is_null() && body["state"] != "running" && body["state"] != "failed" {
        return error(StatusCode::BAD_REQUEST, "A runner sets a job running or failed");
    }

    if body.get("step").is_some() || body.get("state").is_some() {
        state.writes.push(("PATCH".into(), format!("/jobs/{id}"), body.clone()));
    }

    for (field, value) in body.as_object().cloned().unwrap_or_default() {
        state.e2e.jobs[index][field] = value;
    }

    if state.e2e.jobs[index]["state"] == "running" && state.e2e.jobs[index]["startedAt"].is_null() {
        state.e2e.jobs[index]["startedAt"] = json!(NOW);
    }

    Json(state.e2e.jobs[index].clone()).into_response()
}

async fn job_log(State(state): Shared, headers: HeaderMap, UrlPath(id): UrlPath<i64>, Json(body): Json<Value>) -> Response {
    let mut state = guard!(state, headers);

    if job_index(&state, id).is_none() {
        return error(StatusCode::NOT_FOUND, "No such job");
    }

    let lines: Vec<String> =
        body["lines"].as_array().cloned().unwrap_or_default().iter().map(|line| line.as_str().unwrap_or_default().chars().take(2000).collect()).collect();

    state.e2e.log_batches.push(lines.len());

    let log = state.e2e.logs.entry(id).or_default();

    log.extend(lines);

    Json(json!({ "seq": log.len() })).into_response()
}

async fn post_report(
    State(state): Shared,
    headers: HeaderMap,
    UrlPath(key): UrlPath<String>,
    Query(query): Query<BTreeMap<String, String>>,
    body: Bytes,
) -> Response {
    let mut state = guard!(state, headers);
    let Ok(report) = serde_json::from_slice::<Value>(&body) else { return error(StatusCode::BAD_REQUEST, "The report is not JSON") };

    if report["schemaVersion"] != "report-1" {
        return error(StatusCode::BAD_REQUEST, "Not a report of the e2e framework");
    }

    let run_id = state.id();

    if let Some(job) = query.get("job").and_then(|job| job.parse::<i64>().ok()) {
        let Some(index) = job_index(&state, job) else { return error(StatusCode::NOT_FOUND, "No such job") };

        state.e2e.jobs[index]["runId"] = json!(run_id);
    }

    state.writes.push(("POST".into(), format!("/projects/{key}/runs?job={}", query.get("job").cloned().unwrap_or_default()), Value::Null));
    state.e2e.reports.insert(run_id, report);

    Json(json!({ "run": { "id": run_id, "projectKey": key }, "created": true, "linkedIssues": [], "url": format!("/projects/{key}/e2e/runs/{run_id}") }))
        .into_response()
}

async fn put_artifact(
    State(state): Shared,
    headers: HeaderMap,
    UrlPath(run_id): UrlPath<i64>,
    Query(query): Query<BTreeMap<String, String>>,
    body: Bytes,
) -> Response {
    let mut state = guard!(state, headers);
    let Some(path) = query.get("path").filter(|path| !path.is_empty()) else { return error(StatusCode::BAD_REQUEST, "An artifact needs its path") };

    if !state.e2e.reports.contains_key(&run_id) {
        return error(StatusCode::NOT_FOUND, "No such run");
    }

    let content_type = headers.get("content-type").and_then(|value| value.to_str().ok()).unwrap_or_default().to_string();
    let id = state.id();

    state.e2e.artifacts.push((run_id, path.clone(), content_type, body.to_vec()));

    Json(json!({ "id": id })).into_response()
}

async fn finish_job(State(state): Shared, headers: HeaderMap, UrlPath(id): UrlPath<i64>, Json(body): Json<Value>) -> Response {
    let mut state = guard!(state, headers);
    let Some(index) = job_index(&state, id) else { return error(StatusCode::NOT_FOUND, "No such job") };
    let job = &state.e2e.jobs[index];
    // The contract does not say which call ends a job that was cancelled while it ran; here
    // the runner's `finish` does, and the job reads as cancelled rather than failed.
    let ended = match (job["cancelRequested"] == true, body["error"].as_str(), job["runId"].is_null()) {
        (true, _, _) => "cancelled",
        (false, None, false) => "succeeded",
        _ => "failed",
    };

    state.writes.push(("POST".into(), format!("/jobs/{id}/finish"), body.clone()));
    state.e2e.jobs[index]["state"] = json!(ended);
    state.e2e.jobs[index]["error"] = body["error"].clone();
    state.e2e.jobs[index]["finishedAt"] = json!(NOW);
    state.announce("e2e.job", json!({ "jobId": id, "state": ended }));

    Json(state.e2e.jobs[index].clone()).into_response()
}

pub fn e2e_routes() -> Router<Arc<Mutex<HubState>>> {
    Router::new()
        .route("/jobs/claim", post(claim_job))
        .route("/jobs/{id}", patch(patch_job))
        .route("/jobs/{id}/log", post(job_log))
        .route("/jobs/{id}/finish", post(finish_job))
        .route("/projects/{key}/runs", post(post_report))
        .route("/runs/{id}/artifacts", put(put_artifact))
}

impl FakeHub {
    /// The project's test settings, as an administrator would save them.
    pub fn set_e2e(&self, settings: Value, secrets: &[(&str, &str)]) {
        let mut state = self.state();

        state.e2e.settings = settings;
        state.e2e.secrets = secrets.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
    }

    /// What `POST /projects/{key}/jobs` does: a queued job from the project's settings.
    pub fn queue_job(&self, reference: Option<&str>, filter: Option<&str>, video: &str, on_runner: Option<&str>) -> i64 {
        let mut state = self.state();
        let id = state.id();
        let settings = state.e2e.settings.clone();
        let job = json!({
            "id": id, "projectKey": "WEB", "state": "queued", "repo": settings["repo"], "ref": reference.unwrap_or(settings["ref"].as_str().unwrap_or("main")),
            "commit": null, "filter": filter, "video": video, "trace": settings["trace"].as_str().unwrap_or("on"), "requestedBy": 1,
            "runnerName": null, "runId": null, "error": null, "cancelRequested": false, "step": null, "onRunner": on_runner, "onRunnerName": null,
            "createdAt": NOW, "startedAt": null, "finishedAt": null,
        });

        state.e2e.jobs.push(job);
        state.announce("e2e.job", json!({ "jobId": id, "state": "queued" }));

        id
    }

    pub fn job(&self, id: i64) -> Value {
        let state = self.state();

        state.e2e.jobs[job_index(&state, id).unwrap()].clone()
    }

    pub fn job_log(&self, id: i64) -> Vec<String> {
        self.state().e2e.logs.get(&id).cloned().unwrap_or_default()
    }

    /// A person cancelling the job on the web: a running one ends when its runner next looks.
    pub fn cancel_job(&self, id: i64) {
        let mut state = self.state();
        let index = job_index(&state, id).unwrap();

        state.e2e.jobs[index]["cancelRequested"] = json!(true);

        if state.e2e.jobs[index]["state"] == "queued" {
            state.e2e.jobs[index]["state"] = json!("cancelled");
        }
    }

    /// The artifacts of a run: path → (content type, bytes).
    pub fn artifacts_of(&self, run_id: i64) -> BTreeMap<String, (String, Vec<u8>)> {
        self.state().e2e.artifacts.iter().filter(|(run, ..)| *run == run_id).map(|(_, path, kind, bytes)| (path.clone(), (kind.clone(), bytes.clone()))).collect()
    }
}

// ---------- a stand-in for GitHub ----------

/// Bare repositories served over HTTP by git's own `http-backend`, to whoever sends the
/// token as GitHub takes it (any user name, the token as the password).
pub struct GitHost {
    pub root: TempDir,
    /// What `RUNNER_GIT_BASE` is set to.
    pub url: String,
    /// (path and query, whether it carried the token).
    pub requests: Arc<Mutex<Vec<(String, bool)>>>,
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    bytes
        .chunks(3)
        .flat_map(|chunk| {
            let group = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
            let character = |shift: u32, present: bool| if present { ALPHABET[((group >> shift) & 63) as usize] as char } else { '=' };

            [character(18, true), character(12, true), character(6, chunk.len() > 1), character(0, chunk.len() > 2)]
        })
        .collect()
}

async fn serve_git(State((root, requests)): State<(PathBuf, Arc<Mutex<Vec<(String, bool)>>>)>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let header = |name: &str| parts.headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or_default().to_string();
    let sent = header("authorization");
    let authorised = [GITHUB_TOKEN, PERSON_GITHUB_TOKEN].iter().any(|token| sent == format!("Basic {}", base64(format!("x-access-token:{token}").as_bytes())));
    let query = parts.uri.query().unwrap_or_default().to_string();

    requests.lock().unwrap().push((format!("{}?{query}", parts.uri.path()), authorised));

    if !authorised {
        return (StatusCode::UNAUTHORIZED, [("www-authenticate", "Basic realm=\"git\"")], "A token is needed").into_response();
    }

    let body = axum::body::to_bytes(body, 64 * 1024 * 1024).await.unwrap_or_default();
    let mut child = tokio::process::Command::new("git")
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", &root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REMOTE_USER", "x-access-token")
        .env("REQUEST_METHOD", parts.method.as_str())
        .env("PATH_INFO", parts.uri.path())
        .env("QUERY_STRING", &query)
        .env("CONTENT_TYPE", header("content-type"))
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("HTTP_CONTENT_ENCODING", header("content-encoding"))
        .env("GIT_PROTOCOL", header("git-protocol"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();

    tokio::spawn(async move {
        let _ = input.write_all(&body).await;
    });

    let output = child.wait_with_output().await.unwrap().stdout;
    let split = output.windows(4).position(|window| window == b"\r\n\r\n").unwrap_or(output.len());
    let mut response = Response::builder();

    for line in String::from_utf8_lossy(&output[..split]).lines() {
        match line.split_once(": ") {
            Some(("Status", status)) => response = response.status(status.split(' ').next().and_then(|code| code.parse::<u16>().ok()).unwrap_or(500)),
            Some((name, value)) => response = response.header(name, value),
            None => {}
        }
    }

    response.body(axum::body::Body::from(output.get(split + 4..).unwrap_or_default().to_vec())).unwrap()
}

/// GitHub with one repository, `acme/web`: one commit on `main`.
pub async fn github(label: &str) -> GitHost {
    let root = TempDir::new(&format!("{label}-github"));
    let bare = root.path().join("acme/web.git");
    let seed = root.path().join("seed");

    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "--initial-branch=main", "."]);
    git(root.path(), &["clone", bare.to_str().unwrap(), "seed"]);
    write(&seed, "README.md", "# web\n");
    write(&seed, "src/login.ts", "export const limit = 5;\n");
    git(&seed, &["add", "--all"]);
    git(&seed, &["commit", "-m", "Initial commit"]);
    git(&seed, &["push", "origin", "HEAD:main"]);

    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new().fallback(serve_git).with_state((root.path().to_path_buf(), Arc::clone(&requests)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    GitHost { root, url, requests }
}

impl GitHost {
    pub fn bare(&self) -> PathBuf {
        self.root.path().join("acme/web.git")
    }

    /// The working copy the repository was seeded from, for adding to it as somebody else would.
    pub fn seed(&self) -> PathBuf {
        self.root.path().join("seed")
    }

    pub fn commit_on_main(&self, file: &str, content: &str, message: &str) -> String {
        write(&self.seed(), file, content);
        git(&self.seed(), &["add", "--all"]);
        git(&self.seed(), &["commit", "-m", message]);
        git(&self.seed(), &["push", "origin", "HEAD:main"]);
        git(&self.seed(), &["rev-parse", "HEAD"])
    }

    pub fn refs(&self) -> String {
        git(&self.bare(), &["for-each-ref", "--format=%(refname)"])
    }

    pub fn tip(&self, branch: &str) -> String {
        git(&self.bare(), &["rev-parse", &format!("refs/heads/{branch}")])
    }

    pub fn authorised_requests(&self) -> usize {
        self.requests.lock().unwrap().iter().filter(|(_, authorised)| *authorised).count()
    }
}

// ---------- the headless core, wired for tests ----------

/// The environment of a shared runner: an address, a token, and no keys of its own.
pub fn shared_variables(hub: &FakeHub, git_host: &GitHost, data: &Path) -> Vec<(String, String)> {
    runner_variables(hub, git_host, data).into_iter().filter(|(name, _)| name != "GITHUB_TOKEN" && name != "ANTHROPIC_API_KEY").collect()
}

/// The environment a cloud runner would be given, as name and value pairs.
pub fn runner_variables(hub: &FakeHub, git_host: &GitHost, data: &Path) -> Vec<(String, String)> {
    [
        ("TEAM_HUB_URL", hub.url.clone()),
        ("TEAM_HUB_TOKEN", TOKEN.to_string()),
        ("GITHUB_TOKEN", GITHUB_TOKEN.to_string()),
        ("ANTHROPIC_API_KEY", MODEL_KEY.to_string()),
        ("RUNNER_DATA_DIR", data.to_string_lossy().into_owned()),
        ("RUNNER_GIT_BASE", git_host.url.clone()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect()
}

pub fn runner_env(variables: &[(String, String)]) -> RunnerEnv {
    RunnerEnv::read(|name| variables.iter().find(|(known, _)| known == name).map(|(_, value)| value.clone()))
}

pub fn for_tests(options: &mut OpenOptions) {
    options.helper = Some(fake_agent());
    options.stop_grace = Duration::from_millis(400);
    options.poll_every = Duration::from_millis(100);
    options.sandbox = Some(SandboxState::Unavailable);
}

/// A headless core opened the way `hub-runner` opens it, from variables; `more` adds to or
/// replaces what `runner_variables` gives, `adjust` changes how it is opened.
pub async fn open_headless(label: &str, hub: &FakeHub, git_host: &GitHost, more: &[(&str, &str)], adjust: impl FnOnce(&mut OpenOptions)) -> Bench {
    open_headless_in(TempDir::new(label), hub, git_host, more, adjust).await
}

pub async fn open_headless_in(data: TempDir, hub: &FakeHub, git_host: &GitHost, more: &[(&str, &str)], adjust: impl FnOnce(&mut OpenOptions)) -> Bench {
    let mut variables = runner_variables(hub, git_host, data.path());

    for (name, value) in more {
        variables.retain(|(known, _)| known != name);
        variables.push((name.to_string(), value.to_string()));
    }

    let mut options = runner_env(&variables).open_options().expect("the address and the token are set");

    for_tests(&mut options);
    adjust(&mut options);

    let desktop = Desktop::open(options).unwrap();
    let events: broadcast::Receiver<AppEvent> = desktop.subscribe();

    desktop.run(|core| Ok(core.reconnect())).await.unwrap();

    Bench { desktop, data, keychain: Arc::new(hub_desktop_core::secrets::MemoryKeychain::default()), events }
}

/// The job runner of a headless core, with short intervals; jobs run as the test's own user
/// unless `user` says otherwise.
pub fn job_runner(bench: &Bench, user: JobUser) -> (JobRunner, watch::Sender<bool>, watch::Receiver<bool>) {
    let folders = hub_desktop_core::headless::Folders::under(bench.data.path());
    let mut options = JobOptions::new(folders, user);

    options.log_every = Duration::from_millis(200);
    options.look_every = Duration::from_millis(200);
    options.idle_every = Duration::from_millis(100);

    let (stop, stopped) = watch::channel(false);

    (JobRunner::new(Arc::clone(bench.desktop.core()), options), stop, stopped)
}

/// Every file under a folder, with its bytes: for looking for something that must not be there.
pub fn holds(folder: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();

    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).unwrap();

        if meta.is_dir() {
            found.extend(holds(&path, needle));
        } else if meta.is_file() && std::fs::read(&path).is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle.as_bytes())) {
            found.push(path);
        }
    }

    found
}
