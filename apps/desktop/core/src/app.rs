//! The desktop app without its window: the connection, the copy of the server's data, the
//! repositories, the threads and the runner, behind one object. The window, the command line
//! and the tests all drive this.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};

use hub_core::{Bootstrap, Dashboard, Issue, IssuePatch, IssueRow, NewIssue, Notification, Project, TypeLevel, User};
use rayon::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, watch, Notify};

use crate::api::{HubClient, LiveEvent, PairingStarted, RunnerIdentity, WebSession};
use crate::config::{self, Paths};
use crate::error::{CoreError, CoreResult};
use crate::git;
use crate::headless::Headless;
use crate::jobs::{JobOptions, JobRunner};
use crate::model::{
    AppEvent, CacheMode, ConnectionStatus, DashboardView, Delegation, EventKind, IssueView, NewTestJob, NewThread, PairingView, Proposal, ProposalKind,
    ProposalState, PullRequestPreview, PullRequestResult, PushPreview, PushResult, RepoCheck, RepoMapping, RepoStatus, RunnerStatus, SandboxState,
    SearchView, Settings, Thread, ThreadDetail, ThreadDiff, ThreadKind, ThreadOrigin, ThreadState, WorkspaceView, WorktreeInfo,
};
use crate::policy;
use crate::protocol::{self, HubTool};
use crate::push_guard;
use crate::repos::Repos;
use crate::runs::{
    AgentRun, AgentRunDetail, NewRun, RunChanges, RunEventKind, RunKind, RunPatch, RunProposal, RunProposalState, RunState, WorkKeys, RUNNER_KINDS,
};
use crate::secrets::{self, CacheKey, Keychain, Secret};
use crate::store::{self, Store, ThreadSeed};
use crate::threads::{Host, Runtime, RuntimeConfig, Unattended};

const HEARTBEAT_EVERY: Duration = Duration::from_secs(30);
const MAX_CLAIMS_PER_TICK: usize = 20;
const MAX_SUBTASKS: usize = 20;
const RECORDED: &str = "A person will decide; do not wait for the decision.";
const LOST_KEYS: &str = "The runner restarted and no longer holds your keys; start the run again.";

/// How the core is opened. Tests and the command line replace the keychain and the helper.
pub struct OpenOptions {
    pub paths: Paths,
    pub keychain: Arc<dyn Keychain>,
    /// The helper process: the program and its arguments. None: `node` with the sidecar script.
    pub helper: Option<(String, Vec<String>)>,
    /// Where the sidecar's `main.ts` is when the app is packaged (its resource folder).
    pub sidecar_script: Option<PathBuf>,
    pub stop_grace: Duration,
    /// How often the runner looks at the server when nothing wakes it.
    pub poll_every: Duration,
    /// None: ask the operating system.
    pub sandbox: Option<SandboxState>,
    /// Whether GitHub's `gh` command is used to open pull requests. None: when it is installed.
    pub github_cli: Option<bool>,
    /// Set for a runner nobody sits at (docs/cloud-runner.md); none: the desktop app.
    pub headless: Option<Headless>,
}

impl OpenOptions {
    pub fn new(paths: Paths, keychain: Arc<dyn Keychain>) -> Self {
        Self {
            paths,
            keychain,
            helper: None,
            sidecar_script: None,
            stop_grace: Duration::from_secs(5),
            poll_every: Duration::from_secs(5),
            sandbox: None,
            github_cli: None,
            headless: None,
        }
    }
}

#[derive(Default)]
struct Session {
    client: Option<Arc<HubClient>>,
    user: Option<User>,
    online: bool,
    version: Option<String>,
    error: Option<String>,
}

#[derive(Default)]
struct RunnerState {
    runs_api: Option<bool>,
    last_heartbeat: Option<String>,
    last_beat: Option<Instant>,
    error: Option<String>,
    /// Runs this app created for its own threads, waiting to be claimed: run id → thread id.
    attach: HashMap<i64, i64>,
}

/// What a runner nobody sits at keeps beside the rest.
struct HeadlessState {
    /// The team's runner: every run comes with its requester's keys, held here in memory
    /// for as long as the run lives and written nowhere.
    shared: bool,
    keys: Mutex<HashMap<i64, WorkKeys>>,
    kinds: Vec<RunKind>,
    projects: Option<Vec<String>>,
    repos: Repos,
    /// Claimed runs whose repository is being cloned or fetched: they hold a slot already.
    preparing: Mutex<HashSet<i64>>,
    /// The head last sent as a run's changes: run id → commit.
    changes_sent: Mutex<HashMap<i64, String>>,
    /// How approved pushes and pull requests went, where the server has not heard it yet.
    unreported: Mutex<Vec<(i64, Result<Value, String>)>>,
    /// Threads the runner was working on when it last stopped. They are continued once the
    /// server says their runs are still this runner's; until then the runs are told nothing.
    interrupted: Mutex<Vec<i64>>,
    /// Whether runs this runner holds without a thread have been looked for since it started.
    orphans_adopted: AtomicBool,
    /// Set when the runner is told to stop: from then on the server hears nothing about
    /// threads, so that stopping for a redeploy does not read as cancelling them.
    suspended: AtomicBool,
}

/// The logic. Its methods block (network, git, disk); `Desktop` gives them a thread.
pub struct Core {
    /// Itself, for work that outlives the call that started it.
    this: Weak<Core>,
    headless: Option<HeadlessState>,
    /// Poked when the server announces something about a test job.
    jobs_wake: Arc<Notify>,
    /// The desktop app's own clones, for test jobs of repositories it has no mapped clone of.
    job_clones: Repos,
    paths: Paths,
    store: Arc<Store>,
    keychain: Arc<dyn Keychain>,
    cache_key: CacheKey,
    settings: RwLock<Settings>,
    session: RwLock<Session>,
    runner: Mutex<RunnerState>,
    tools: Mutex<Option<Vec<HubTool>>>,
    events: broadcast::Sender<AppEvent>,
    runtime: OnceLock<Runtime>,
    sandbox: SandboxState,
    github_cli: bool,
    seen_notification: AtomicI64,
    wake: Arc<Notify>,
    closing: Arc<AtomicBool>,
    /// The request to connect that waits for its person, when there is one.
    pairing: Mutex<Option<Pairing>>,
}

/// A request to connect, as the app holds it: with the secret that collects the token.
#[derive(Clone)]
struct Pairing {
    url: String,
    started: PairingStarted,
}

/// How often the app asks whether its person has said yes.
const PAIR_POLL: Duration = Duration::from_secs(2);
/// The server gives a request ten minutes; a little after that the app stops asking by itself.
const PAIR_PATIENCE: Duration = Duration::from_secs(11 * 60);

struct Bridge(Weak<Core>);

impl Host for Bridge {
    fn hub_tools(&self, _thread: &Thread) -> Vec<HubTool> {
        self.0.upgrade().map(|core| core.read_only_tools()).unwrap_or_default()
    }

    fn call_tool(&self, thread: &Thread, tool: &str, input: &Value) -> Result<Value, String> {
        let core = self.0.upgrade().ok_or("The app is closing")?;

        core.agent_tool(thread, tool, input).map_err(|error| error.to_string())
    }

    fn model_key(&self, thread: &Thread) -> Result<Option<Secret>, String> {
        let core = self.0.upgrade().ok_or("The app is closing")?;

        core.model_key_of(thread).map_err(|error| error.to_string())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn text_of<'a>(input: &'a Value, field: &str, max: usize) -> CoreResult<&'a str> {
    let text = input[field].as_str().map(str::trim).unwrap_or_default();

    if text.is_empty() {
        return Err(CoreError::not_possible(format!("\"{field}\" is required")));
    }

    if text.chars().count() > max {
        return Err(CoreError::not_possible(format!("\"{field}\" is longer than {max} characters")));
    }

    Ok(text)
}

/// Where the sidecar's script is: where the packaged app says, else in the source tree.
fn default_helper(node: &str, packaged: Option<&Path>) -> (String, Vec<String>) {
    let script = packaged
        .filter(|script| script.exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../sidecar/src/main.ts"));

    (node.to_string(), vec![frozen_copy(&script).to_string_lossy().into_owned()])
}

/// A copy of the helper's scripts taken when this process starts, so that editing or updating
/// them while the app runs cannot break the threads it starts. The copy sits beside
/// `node_modules`, where Node still finds the script's dependencies. When copying fails the
/// script itself is used.
fn frozen_copy(script: &Path) -> PathBuf {
    const STALE: Duration = Duration::from_secs(24 * 60 * 60);

    let copy = || -> std::io::Result<PathBuf> {
        let source = script.parent().ok_or(std::io::ErrorKind::NotFound)?;
        let copies = source.parent().ok_or(std::io::ErrorKind::NotFound)?.join(".run");
        let mine = copies.join(std::process::id().to_string());

        std::fs::create_dir_all(&mine)?;

        for entry in std::fs::read_dir(&copies)?.flatten() {
            let age = entry.metadata().and_then(|found| found.modified()).ok().and_then(|at| at.elapsed().ok());

            if entry.path() != mine && age.is_some_and(|age| age > STALE) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }

        for entry in std::fs::read_dir(source)?.flatten() {
            if entry.path().extension().is_some_and(|extension| extension == "ts") {
                std::fs::copy(entry.path(), mine.join(entry.file_name()))?;
            }
        }

        Ok(mine.join(script.file_name().unwrap_or_default()))
    };

    copy().unwrap_or_else(|_| script.to_path_buf())
}

impl Core {
    fn runtime(&self) -> &Runtime {
        self.runtime.get().expect("the runtime is set when the core is opened")
    }

    fn emit(&self, event: AppEvent) {
        let _ = self.events.send(event);
    }

    fn data_changed(&self, scope: &str, issue_key: Option<&str>) {
        self.emit(AppEvent::DataChanged { scope: scope.to_string(), issue_key: issue_key.map(String::from) });
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Saves what the person changed. The server address and the runner's id are not changed here.
    pub fn save_settings(&self, wanted: Settings) -> CoreResult<Settings> {
        let mut settings = self.settings.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let kept = Settings { server_url: settings.server_url.clone(), runner_id: settings.runner_id.clone(), ..wanted };

        if kept.max_running == 0 || kept.max_running > 16 {
            return Err(CoreError::not_possible("Between 1 and 16 threads can work at once"));
        }

        if kept.runner_name.trim().is_empty() || kept.default_model.trim().is_empty() || kept.node_command.trim().is_empty() {
            return Err(CoreError::not_possible("The runner's name, the model and the Node command cannot be empty"));
        }

        config::save_settings(&self.paths, &kept)?;
        self.runtime().set_max_running(kept.max_running as usize);
        *settings = kept.clone();

        Ok(kept)
    }

    // ---------- connection ----------

    pub fn client(&self) -> CoreResult<Arc<HubClient>> {
        self.session.read().unwrap_or_else(|poisoned| poisoned.into_inner()).client.clone().ok_or(CoreError::NotConnected)
    }

    fn me(&self) -> Option<User> {
        self.session.read().unwrap_or_else(|poisoned| poisoned.into_inner()).user.clone()
    }

    /// Notes whether the server answered, and tells the window when that changed.
    fn seen<T>(&self, result: CoreResult<T>) -> CoreResult<T> {
        let online = match &result {
            Ok(_) | Err(CoreError::Api { .. }) => true,
            Err(CoreError::Offline(_)) => false,
            Err(_) => return result,
        };
        let changed = {
            let mut session = self.session.write().unwrap_or_else(|poisoned| poisoned.into_inner());
            let changed = session.online != online;

            session.online = online;
            session.error = match &result {
                Err(error) if !online || error.is_unauthorized() => Some(error.to_string()),
                _ => None,
            };
            changed
        };

        if changed {
            self.emit(AppEvent::Connection { online });
        }

        result
    }

    pub fn status(&self) -> ConnectionStatus {
        let settings = self.settings();
        let session = self.session.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        let runner = lock(&self.runner);

        ConnectionStatus {
            server_url: settings.server_url.clone(),
            has_token: session.client.is_some(),
            online: session.online,
            user: session.user.clone(),
            server_version: session.version.clone(),
            error: session.error.clone(),
            cache: if self.cache_key.persistent { CacheMode::Encrypted } else { CacheMode::MemoryOnly },
            sandbox: self.sandbox,
            runner: RunnerStatus {
                id: settings.runner_id.clone(),
                name: settings.runner_name.clone(),
                enabled: settings.runner_enabled,
                runs_api: runner.runs_api,
                last_heartbeat: runner.last_heartbeat.clone(),
                error: runner.error.clone(),
            },
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn adopt_session(&self, client: HubClient, boot: Option<&Bootstrap>) {
        let mut session = self.session.write().unwrap_or_else(|poisoned| poisoned.into_inner());

        session.client = Some(Arc::new(client));

        if let Some(boot) = boot {
            session.user = boot.user.clone();
            session.version = Some(boot.version.clone());
            session.online = true;
            session.error = None;
        }
    }

    /// Checks the address and the token against the server, then keeps the address in the
    /// settings and the token in the keychain.
    pub fn connect(&self, url: &str, token: &str) -> CoreResult<ConnectionStatus> {
        let url = config::normalise_server_url(url)?;
        let token = Secret::new(token.trim());

        if token.expose().is_empty() {
            return Err(CoreError::not_possible("An API token is needed: create one in team-hub under Settings → API tokens"));
        }

        let client = HubClient::new(&url, token.clone());
        let boot = client.bootstrap()?;

        if boot.user.is_none() {
            return Err(CoreError::not_possible("The server did not accept this token"));
        }

        // Whatever token was there before, its id on the server is not this one's.
        let _ = self.keychain.delete(secrets::TOKEN_ID_ENTRY);

        // Where there is no keychain, a token given through the environment is used as it is
        // and stored nowhere.
        let from_environment = std::env::var(secrets::TOKEN_VARIABLE).is_ok_and(|given| given.trim() == token.expose());

        if let Err(error) = self.keychain.set(secrets::TOKEN_ENTRY, &token) {
            if !from_environment {
                let advice = format!("The token was not saved; on a machine without a keychain set {} instead", secrets::TOKEN_VARIABLE);

                return Err(CoreError::Keychain(format!("{error}. {advice}")));
            }
        }

        {
            let mut settings = self.settings.write().unwrap_or_else(|poisoned| poisoned.into_inner());

            settings.server_url = Some(url);
            config::save_settings(&self.paths, &settings)?;
        }

        self.store.cache_clear()?;
        self.store.cache_put(&self.cache_key, "bootstrap", &boot)?;
        self.adopt_session(client, Some(&boot));
        *lock(&self.tools) = None;
        *lock(&self.runner) = RunnerState::default();
        self.emit(AppEvent::Connection { online: true });
        self.wake.notify_one();

        Ok(self.status())
    }

    /// Signs in with the e-mail address and password of an account, as the web interface
    /// does, and connects with a token made for this app in that session. The password goes
    /// to the server and nowhere else. The session comes back with the status: the window
    /// hands it to the tracker it shows, and whoever has no use for it ends it.
    pub fn sign_in(&self, url: &str, email: &str, password: &str) -> CoreResult<(ConnectionStatus, WebSession)> {
        let url = config::normalise_server_url(url)?;

        if email.trim().is_empty() || password.is_empty() {
            return Err(CoreError::not_possible("Give the e-mail address and the password of your account"));
        }

        let session = HubClient::sign_in(&url, email.trim(), password)?;
        let connected = session.create_token(&self.settings().runner_name).and_then(|grant| match self.connect(&url, grant.token.expose()) {
            Ok(status) => {
                // Without this the app still works; it only cannot take its token back itself.
                let _ = self.keychain.set(secrets::TOKEN_ID_ENTRY, &Secret::new(grant.token_id.to_string()));

                Ok(status)
            }
            Err(error) => {
                // A token that could not be kept is not left behind on the server.
                let _ = HubClient::new(&url, grant.token.clone()).revoke_token(grant.token_id);

                Err(error)
            }
        });

        match connected {
            Ok(status) => Ok((status, session)),
            Err(error) => {
                session.end();

                Err(error)
            }
        }
    }

    /// Asks the server at `url` to connect this app, and keeps the address. Nothing is
    /// connected yet: the person says yes in the tracker, at the page this names, and
    /// `pair_wait` collects the token.
    pub fn pair_start(&self, url: &str) -> CoreResult<PairingView> {
        let url = config::normalise_server_url(url)?;

        if self.client().is_ok() {
            return Err(CoreError::not_possible("The app is connected already; disconnect it first"));
        }

        let settings = self.settings();
        let started = HubClient::start_pairing(&url, &settings.runner_name, &settings.runner_id).map_err(|error| match error {
            error if error.is_not_found() => {
                CoreError::not_possible("This server is too old to sign in from the window. Connect with an API token instead, under Advanced.")
            }
            other => other,
        })?;

        {
            let mut settings = self.settings.write().unwrap_or_else(|poisoned| poisoned.into_inner());

            settings.server_url = Some(url.clone());
            config::save_settings(&self.paths, &settings)?;
        }

        let view = PairingView {
            server_url: url.clone(),
            path: format!("/desktop/pair?code={}", started.code),
            code: started.code.clone(),
            expires_at: started.expires_at.clone(),
        };

        *lock(&self.pairing) = Some(Pairing { url, started });

        Ok(view)
    }

    /// Gives up the request that waits; whoever waits for it is told so.
    pub fn pair_cancel(&self) {
        *lock(&self.pairing) = None;
    }

    fn pairing_is(&self, id: &str) -> bool {
        lock(&self.pairing).as_ref().is_some_and(|pairing| pairing.started.id == id)
    }

    /// Waits until the person has said yes, then connects with the token the server hands
    /// over, the same way a pasted token is connected. Ends with an error when they said no,
    /// took too long, or the request was given up here.
    pub fn pair_wait(&self) -> CoreResult<ConnectionStatus> {
        let Some(Pairing { url, started }) = lock(&self.pairing).clone() else {
            return Err(CoreError::not_possible("Nothing is waiting to be connected; start again"));
        };
        let began = Instant::now();

        loop {
            if !self.pairing_is(&started.id) || self.closing.load(Ordering::Relaxed) {
                return Err(CoreError::not_possible("Connecting was cancelled"));
            }

            match HubClient::claim_pairing(&url, &started.id, &started.secret) {
                Ok(Some(grant)) => {
                    self.pair_cancel();

                    let status = match self.connect(&url, grant.token.expose()) {
                        Ok(status) => status,
                        Err(error) => {
                            // A token that could not be kept (no keychain) is of no use to
                            // anybody: it is not left behind on the server.
                            let _ = HubClient::new(&url, grant.token.clone()).revoke_token(grant.token_id);

                            return Err(error);
                        }
                    };

                    // Without this the app still works; it only cannot take its token back itself.
                    let _ = self.keychain.set(secrets::TOKEN_ID_ENTRY, &Secret::new(grant.token_id.to_string()));

                    return Ok(status);
                }
                // Nobody has said yes yet, or the server is away for a moment.
                Ok(None) => {}
                Err(error) if error.is_offline() => {}
                Err(CoreError::Api { status: 410, .. }) => {
                    self.pair_cancel();

                    return Err(CoreError::not_possible("The request to connect was declined or ran out of time. Start again."));
                }
                Err(error) => {
                    self.pair_cancel();

                    return Err(error);
                }
            }

            if began.elapsed() > PAIR_PATIENCE {
                self.pair_cancel();

                return Err(CoreError::not_possible("Nobody approved the request to connect in time. Start again."));
            }

            // Slept in short pieces, so that giving up is noticed at once.
            let asked = Instant::now();

            while asked.elapsed() < PAIR_POLL && self.pairing_is(&started.id) && !self.closing.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// Forgets the token and the copy of the server's data. Threads and repositories stay.
    /// A token the app was given in the tracker is removed on the server too, when the
    /// server can be reached; one that was pasted is its person's to remove.
    pub fn disconnect(&self) -> CoreResult<ConnectionStatus> {
        let given = self.keychain.get(secrets::TOKEN_ID_ENTRY).ok().flatten().and_then(|id| id.expose().parse::<i64>().ok());

        if let (Some(token_id), Ok(client)) = (given, self.client()) {
            // When this fails the token stays listed under Settings → Profile and tokens.
            let _ = client.revoke_token(token_id);
        }

        self.pair_cancel();
        self.keychain.delete(secrets::TOKEN_ID_ENTRY)?;
        self.keychain.delete(secrets::TOKEN_ENTRY)?;
        self.store.cache_clear()?;
        *self.session.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = Session::default();
        *lock(&self.tools) = None;
        self.emit(AppEvent::Connection { online: false });

        Ok(self.status())
    }

    /// At startup: the saved address with the token from the keychain (or the environment).
    pub fn reconnect(&self) -> ConnectionStatus {
        let url = self.settings().server_url;
        let token = secrets::load_token(self.keychain.as_ref());

        match (url, token) {
            (Some(url), Ok(Some(token))) => {
                let client = HubClient::new(&url, token);
                let boot = self.seen(client.bootstrap());

                if let Ok(boot) = &boot {
                    let _ = self.store.cache_put(&self.cache_key, "bootstrap", boot);
                }

                let stored =
                    boot.ok().or_else(|| self.store.cache_get::<Bootstrap>(&self.cache_key, "bootstrap").ok().flatten().map(|(boot, _)| boot));

                self.adopt_session(client, None);

                if let Some(boot) = stored {
                    let mut session = self.session.write().unwrap_or_else(|poisoned| poisoned.into_inner());

                    session.user = boot.user.clone();
                    session.version = Some(boot.version.clone());
                }
            }
            (_, Err(error)) => {
                self.session.write().unwrap_or_else(|poisoned| poisoned.into_inner()).error = Some(error.to_string());
            }
            _ => {}
        }

        self.status()
    }

    // ---------- the server's data, with the stored copy behind it ----------

    /// Fresh from the server and stored; or, when the server cannot be reached, the stored copy.
    fn cached<T: Serialize + DeserializeOwned>(&self, name: &str, fetch: impl FnOnce(&HubClient) -> CoreResult<T>) -> CoreResult<(T, String, bool)> {
        let client = self.client()?;

        match self.seen(fetch(&client)) {
            Ok(value) => {
                let fetched_at = self.store.cache_put(&self.cache_key, name, &value)?;

                Ok((value, fetched_at, false))
            }
            Err(error) if error.is_offline() => match self.store.cache_get::<T>(&self.cache_key, name)? {
                Some((value, fetched_at)) => Ok((value, fetched_at, true)),
                None => Err(error),
            },
            Err(error) => Err(error),
        }
    }

    pub fn workspace(&self) -> CoreResult<WorkspaceView> {
        let (bootstrap, fetched_at, offline) = self.cached("bootstrap", |client| client.bootstrap())?;

        if bootstrap.user.is_none() {
            return Err(CoreError::not_possible("The server no longer accepts this token; connect again with a new one"));
        }

        Ok(WorkspaceView { bootstrap, fetched_at, offline })
    }

    /// The configuration of each project, read side by side: these are calls to another
    /// system, so each gets a thread of its own.
    fn projects_of(client: &HubClient, keys: &[String]) -> CoreResult<Vec<Project>> {
        std::thread::scope(|scope| {
            let handles: Vec<_> = keys.iter().map(|key| scope.spawn(move || client.project(key))).collect();

            handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| Err(CoreError::not_possible("a request panicked")))).collect()
        })
    }

    fn project_keys(rows: &[&[IssueRow]]) -> Vec<String> {
        let mut keys: Vec<String> = rows.iter().flat_map(|rows| rows.iter().map(|row| row.project_key.clone())).collect();

        keys.sort();
        keys.dedup();
        keys
    }

    pub fn dashboard(&self) -> CoreResult<DashboardView> {
        let (view, fetched_at, offline) = self.cached("dashboard", |client| {
            let dashboard: Dashboard = client.dashboard()?;
            let keys = Self::project_keys(&[&dashboard.assigned, &dashboard.reported, &dashboard.watching, &dashboard.recently_done]);
            let projects = Self::projects_of(client, &keys)?;

            Ok((dashboard, projects))
        })?;

        Ok(DashboardView { dashboard: view.0, projects: view.1, fetched_at, offline })
    }

    fn issue_with_project(client: &HubClient, key: &str) -> CoreResult<(Issue, Option<Project>)> {
        let issue = client.issue(key)?;
        let project = client.project(&issue.row.project_key).ok();

        Ok((issue, project))
    }

    pub fn issue(&self, key: &str) -> CoreResult<IssueView> {
        let key = key.trim().to_uppercase();
        let (view, fetched_at, offline) = self.cached(&format!("issue:{key}"), |client| Self::issue_with_project(client, &key))?;

        Ok(IssueView { issue: view.0, project: view.1, fetched_at, offline })
    }

    pub fn search(&self, query: &str) -> CoreResult<SearchView> {
        let client = self.client()?;
        let result = self.seen(client.search(query, 50))?;
        let projects = Self::projects_of(&client, &Self::project_keys(&[&result.issues]))?;

        Ok(SearchView { result, projects })
    }

    pub fn notifications(&self) -> CoreResult<Vec<Notification>> {
        let (notifications, _, _) = self.cached("notifications", |client| client.notifications())?;

        Ok(notifications)
    }

    pub fn mark_notifications_read(&self, id: Option<i64>) -> CoreResult<()> {
        self.seen(self.client()?.mark_notifications_read(id))
    }

    /// A project's "Code" page of the github module; null when the module is off.
    pub fn project_code(&self, project_key: &str) -> CoreResult<Value> {
        let path = format!("/api/m/github/projects/{}", project_key.trim().to_uppercase());

        match self.cached(&format!("code:{project_key}"), |client| client.module_data(&path)) {
            Ok((code, _, _)) => Ok(code),
            Err(error) if error.is_not_found() => Ok(Value::Null),
            Err(error) => Err(error),
        }
    }

    // ---------- the basics on an issue, done by the person ----------

    fn after_change(&self, key: &str) -> CoreResult<IssueView> {
        self.data_changed("issue", Some(key));
        self.issue(key)
    }

    pub fn comment(&self, key: &str, body: &str) -> CoreResult<IssueView> {
        if body.trim().is_empty() {
            return Err(CoreError::not_possible("The comment is empty"));
        }

        self.seen(self.client()?.add_comment(key, body))?;
        self.after_change(key)
    }

    pub fn transition(&self, key: &str, status_id: i64) -> CoreResult<IssueView> {
        self.seen(self.client()?.update_issue(key, &IssuePatch { status_id: Some(status_id), ..Default::default() }))?;
        self.after_change(key)
    }

    pub fn assign_to_me(&self, key: &str) -> CoreResult<IssueView> {
        let me = self.me().ok_or(CoreError::NotConnected)?;

        self.seen(self.client()?.update_issue(key, &IssuePatch { assignee_id: Some(Some(me.id)), ..Default::default() }))?;
        self.after_change(key)
    }

    pub fn log_work(&self, key: &str, spent: &str, note: &str) -> CoreResult<IssueView> {
        if hub_core::text::parse_duration(spent).is_none() {
            return Err(CoreError::not_possible("Time spent looks like \"1h 30m\""));
        }

        self.seen(self.client()?.log_work(key, spent, note))?;
        self.after_change(key)
    }

    // ---------- repositories ----------

    pub fn check_repo(&self, path: &str, github_repo: Option<&str>) -> RepoCheck {
        let wanted = github_repo.map(str::trim).filter(|repo| !repo.is_empty());

        match git::inspect(Path::new(path.trim())) {
            Ok(found) => {
                let remote_repo = found.remote_url.as_deref().and_then(git::repo_of_remote);
                let matches = match (wanted, &found.remote_url) {
                    (Some(wanted), Some(url)) => git::remote_matches(url, wanted),
                    (Some(_), None) => false,
                    (None, _) => true,
                };
                let problem = match (matches, wanted, &found.remote_url) {
                    (true, _, _) => None,
                    (false, Some(wanted), Some(url)) => Some(format!("origin is {url}, not {wanted}")),
                    (false, Some(wanted), None) => Some(format!("The clone has no remote called origin, so it cannot be checked against {wanted}")),
                    (false, None, _) => None,
                };

                RepoCheck {
                    path: found.root.to_string_lossy().into_owned(),
                    is_repository: true,
                    remote_url: found.remote_url,
                    remote_repo,
                    matches,
                    default_branch: found.default_branch,
                    problem,
                }
            }
            Err(error) => RepoCheck {
                path: path.trim().to_string(),
                is_repository: false,
                remote_url: None,
                remote_repo: None,
                matches: false,
                default_branch: None,
                problem: Some(format!("Not a git repository: {error}")),
            },
        }
    }

    /// Ties a project to a local clone. The clone's origin has to be the named repository.
    pub fn map_repo(&self, project_key: &str, github_repo: Option<&str>, path: &str) -> CoreResult<RepoMapping> {
        let project_key = project_key.trim().to_uppercase();

        if !hub_core::keys::is_project_key(&project_key) {
            return Err(CoreError::not_possible(format!("\"{project_key}\" is not a project key")));
        }

        let check = self.check_repo(path, github_repo);

        if let Some(problem) = check.problem.filter(|_| !check.is_repository || !check.matches) {
            return Err(CoreError::not_possible(problem));
        }

        let mapping = self.store.add_repo(&project_key, github_repo.map(str::trim).filter(|repo| !repo.is_empty()), &check.path)?;

        self.wake.notify_one();

        Ok(mapping)
    }

    pub fn unmap_repo(&self, id: i64) -> CoreResult<()> {
        self.store.remove_repo(id)
    }

    /// Every mapped repository as it is on disk now. Repositories are independent, so they
    /// are scanned side by side.
    pub fn repos(&self) -> CoreResult<Vec<RepoStatus>> {
        let mappings = self.store.repos()?;
        let threads = self.store.threads()?;
        let thread_of = |path: &Path| {
            threads.iter().find(|thread| thread.worktree.as_deref().is_some_and(|tree| Path::new(tree) == path)).map(|thread| thread.id)
        };

        Ok(mappings
            .into_par_iter()
            .map(|mapping| match git::inspect(Path::new(&mapping.path)) {
                Ok(found) => RepoStatus {
                    exists: true,
                    remote_matches: mapping
                        .github_repo
                        .as_deref()
                        .map(|repo| found.remote_url.as_deref().is_some_and(|url| git::remote_matches(url, repo))),
                    default_branch: found.default_branch,
                    current_branch: found.current_branch,
                    changed_files: found.changed_files,
                    remote_url: found.remote_url,
                    worktrees: found
                        .worktrees
                        .into_iter()
                        .map(|tree| WorktreeInfo {
                            thread_id: thread_of(&tree.path),
                            path: tree.path.to_string_lossy().into_owned(),
                            branch: tree.branch,
                            head: tree.head,
                        })
                        .collect(),
                    error: None,
                    mapping,
                },
                Err(error) => RepoStatus {
                    exists: Path::new(&mapping.path).exists(),
                    default_branch: None,
                    current_branch: None,
                    changed_files: 0,
                    remote_url: None,
                    remote_matches: None,
                    worktrees: Vec::new(),
                    error: Some(error.to_string()),
                    mapping,
                },
            })
            .collect())
    }

    fn repo_for(&self, project_key: &str, wanted: Option<i64>, github_repo: Option<&str>) -> CoreResult<RepoMapping> {
        let of_project: Vec<RepoMapping> = self.store.repos()?.into_iter().filter(|repo| repo.project_key == project_key).collect();
        let by_id = wanted.and_then(|id| of_project.iter().find(|repo| repo.id == id));
        // A mapping made without a repository name still counts when its clone's origin is the one asked for.
        let is_named = |repo: &RepoMapping, name: &str| match repo.github_repo.as_deref() {
            Some(mapped) => mapped.eq_ignore_ascii_case(name),
            None => git::inspect(Path::new(&repo.path)).ok().and_then(|found| found.remote_url).is_some_and(|url| git::remote_matches(&url, name)),
        };
        let by_name = github_repo.and_then(|name| of_project.iter().find(|repo| is_named(repo, name)));

        match (by_id.or(by_name), of_project.as_slice()) {
            (Some(repo), _) => Ok(repo.clone()),
            (None, _) if wanted.is_some() => Err(CoreError::not_possible(format!("That repository is not mapped to project {project_key}"))),
            (None, [only]) if github_repo.is_none() => Ok(only.clone()),
            (None, []) => {
                Err(CoreError::not_possible(format!("No local repository is mapped to project {project_key}. Map one under Repositories first")))
            }
            (None, _) => match github_repo {
                Some(name) => Err(CoreError::not_possible(format!("{name} has no local clone mapped on this machine"))),
                None => Err(CoreError::not_possible(format!("Project {project_key} has several repositories mapped; choose one"))),
            },
        }
    }

    // ---------- threads ----------

    pub fn threads(&self) -> CoreResult<Vec<Thread>> {
        self.store.threads()
    }

    pub fn thread(&self, id: i64) -> CoreResult<ThreadDetail> {
        Ok(ThreadDetail { thread: self.store.thread(id)?, events: self.store.events(id)?, proposals: self.store.proposals(id)? })
    }

    pub fn pending_proposals(&self) -> CoreResult<Vec<Proposal>> {
        self.store.pending_proposals()
    }

    fn type_name(issue: &Issue, project: Option<&Project>) -> String {
        project.and_then(|project| project.types.iter().find(|kind| kind.id == issue.row.type_id)).map(|kind| kind.name.clone()).unwrap_or_default()
    }

    /// Starts a thread for an issue: a queued thread here, and a run on the server so the web
    /// shows it too. Without a server (or without its runs API) the thread is local only.
    pub fn start_thread(&self, new: &NewThread) -> CoreResult<Thread> {
        let key = new.issue_key.trim().to_uppercase();
        let (project_key, _) =
            hub_core::keys::parse_issue_key(&key).ok_or_else(|| CoreError::not_possible(format!("\"{key}\" is not an issue key")))?;
        let repo = self.repo_for(&project_key, new.repo_id, None)?;
        let settings = self.settings();
        let model = new.model.clone().filter(|model| !model.trim().is_empty()).unwrap_or(settings.default_model);

        if let Some(branch) = new.branch.as_deref().filter(|branch| !git::is_branch_name(branch)) {
            return Err(CoreError::not_possible(format!("\"{branch}\" is not a usable branch name")));
        }

        let view = self.issue(&key)?;
        let project = view.project.as_ref();
        let seed = ThreadSeed {
            issue_key: key.clone(),
            issue_summary: view.issue.row.summary.clone(),
            issue_type: Self::type_name(&view.issue, project),
            project_key,
            kind: new.kind,
            instructions: new.instructions.trim().to_string(),
            model: model.clone(),
            repo: repo.clone(),
            wanted_branch: new.branch.clone(),
            origin: ThreadOrigin::App,
            run_id: None,
            prompt: policy::first_prompt(new.kind, Some((&view.issue, project)), &key, &view.issue.row.summary, &new.instructions),
        };
        let run = self.create_run(&key, new.kind, &seed.instructions, &model, &Self::repo_name(&repo));
        let thread = self.runtime().enqueue(&seed)?;

        match run {
            Ok(run) => {
                self.tie(&run, thread.id);
                self.claim_runs();
            }
            Err(error) => {
                self.store.add_event(
                    thread.id,
                    EventKind::Status,
                    &format!("This thread is not shown on the server: {error}"),
                    &Value::Null,
                    true,
                )?;
            }
        }

        self.store.thread(thread.id)
    }

    /// The repository a run is for, as "owner/name". A run always names one: without it the
    /// server may carry out a `custom` run itself, and this one needs the code.
    fn repo_name(repo: &RepoMapping) -> String {
        let from_remote = || git::inspect(Path::new(&repo.path)).ok().and_then(|found| found.remote_url).and_then(|url| git::repo_of_remote(&url));
        let from_folder =
            || format!("local/{}", Path::new(&repo.path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());

        repo.github_repo.clone().or_else(from_remote).unwrap_or_else(from_folder)
    }

    fn create_run(&self, key: &str, kind: ThreadKind, instructions: &str, model: &str, repo: &str) -> CoreResult<AgentRun> {
        let client = self.client()?;

        // The server gives a run only to a runner it has heard from.
        self.heartbeat(&client);

        let new = NewRun {
            kind: RunKind::of_thread(kind),
            instructions: instructions.to_string(),
            model: Some(model.to_string()),
            repo: Some(repo.to_string()),
            shared: false,
            runner_id: self.identity().ok().map(|identity| identity.id),
            on_runner: None,
        };
        let run = self.seen(client.create_run(key, &new));

        if run.as_ref().is_err_and(CoreError::is_not_found) {
            lock(&self.runner).runs_api = Some(false);
        }

        run
    }

    /// Ties a run this app created to its thread. The server hands the run over already
    /// claimed; an older server leaves it queued, and it is claimed like any other.
    fn tie(&self, run: &AgentRun, thread_id: i64) {
        if run.state == RunState::Queued {
            lock(&self.runner).attach.insert(run.id, thread_id);
        } else if self.store.set_run(thread_id, Some(run.id)).is_ok() {
            self.emit(AppEvent::ThreadChanged { thread_id });
        }
    }

    /// Says something to a thread: the answer to its question, or a follow-up. A thread whose
    /// run on the server is over gets a new run, so the web keeps showing what happens.
    pub fn say_to_thread(&self, thread_id: i64, text: Option<&str>) -> CoreResult<Thread> {
        let thread = self.store.thread(thread_id)?;
        let text = text.map(str::trim).filter(|text| !text.is_empty());
        let mut mirrored = false;

        if let (true, Some(text), Some(run_id)) = (thread.pending_question, text, thread.run_id) {
            // The run has to know the question before it gets the answer; then the answer goes
            // to the run first, so the web shows who answered.
            self.sync_thread(&thread);

            if let Ok(detail) = self.client().and_then(|client| self.seen(client.answer_run(run_id, text))) {
                let seen = detail.events.iter().filter(|event| event.kind == RunEventKind::Answer).map(|event| event.id).max().unwrap_or(0);

                self.store.set_answers_seen(thread_id, seen)?;
                mirrored = true;
            }
        } else if thread.run_id.is_some() && self.patch_of(&thread).state.is_some_and(RunState::is_finished) {
            // The run has ended on the server and takes no more writes: what happens next is a new run.
            self.sync_thread(&thread);
            self.store.mark_all_mirrored(thread_id)?;
            self.store.set_run(thread_id, None)?;

            let instructions = text.unwrap_or("Resumed in the desktop app.");
            let repo = self.store.repo(thread.repo_id).map(|repo| Self::repo_name(&repo)).unwrap_or_else(|_| "local/unknown".to_string());

            if let Ok(run) = self.create_run(&thread.issue_key, thread.kind, instructions, &thread.model, &repo) {
                self.tie(&run, thread_id);
            }
        }

        let thread = self.runtime().resume(thread_id, text, mirrored)?;

        self.claim_runs();

        Ok(thread)
    }

    pub fn thread_diff(&self, thread_id: i64) -> CoreResult<ThreadDiff> {
        let thread = self.store.thread(thread_id)?;
        let (Some(worktree), Some(base_commit)) = (thread.worktree.as_deref(), thread.base_commit.as_deref()) else {
            return Err(CoreError::not_possible("The thread has no working tree (not started yet, or removed)"));
        };
        let (files, commits, uncommitted) = git::thread_changes(Path::new(&thread.repo_path), Path::new(worktree), base_commit)?;

        Ok(ThreadDiff { base: thread.base.unwrap_or_else(|| base_commit.to_string()), files, commits, uncommitted })
    }

    // ---------- what the agent may ask of the app ----------

    fn read_only_tools(&self) -> Vec<HubTool> {
        if let Some(tools) = lock(&self.tools).clone() {
            return tools;
        }

        let Ok(tools) = self.client().and_then(|client| self.seen(client.read_only_tools())) else { return Vec::new() };

        *lock(&self.tools) = Some(tools.clone());
        tools
    }

    /// One tool call of an agent. Reads go to the server's MCP endpoint, and only for tools the
    /// server marks read-only; everything else the agent can do here is to propose or to ask.
    fn agent_tool(&self, thread: &Thread, tool: &str, input: &Value) -> CoreResult<Value> {
        if let Some(name) = tool.strip_prefix(protocol::READ_PREFIX) {
            if !self.read_only_tools().iter().any(|known| known.name == name) {
                return Err(CoreError::not_possible(format!("{name} is not a read-only tool of the server. To change something, propose it.")));
            }

            return Ok(Value::String(self.seen(self.client()?.call_tool(name, input))?));
        }

        match tool {
            protocol::ASK_QUESTION => {
                let question = text_of(input, "question", 4000)?;
                let event = self.store.add_event(thread.id, EventKind::Question, question, &Value::Null, false)?;

                self.emit(AppEvent::ThreadEvent { thread_id: thread.id, event });

                Ok(json!("The question is recorded. End your turn now; you will be resumed with the answer."))
            }
            protocol::PROPOSE_COMMENT => self.propose(thread, ProposalKind::Comment, json!({ "body": text_of(input, "body", 20_000)? })),
            protocol::PROPOSE_STATUS => self.propose(thread, ProposalKind::Transition, json!({ "status": text_of(input, "status", 80)? })),
            protocol::PROPOSE_SUBTASKS => {
                let items = input["items"].as_array().cloned().unwrap_or_default();

                if items.is_empty() || items.len() > MAX_SUBTASKS {
                    return Err(CoreError::not_possible(format!("\"items\" needs between 1 and {MAX_SUBTASKS} subtasks")));
                }

                let items = items
                    .iter()
                    .map(|item| {
                        let description = item["description"].as_str().map(str::trim).filter(|text| !text.is_empty());

                        Ok(match description {
                            Some(description) => json!({ "summary": text_of(item, "summary", 255)?, "description": description }),
                            None => json!({ "summary": text_of(item, "summary", 255)? }),
                        })
                    })
                    .collect::<CoreResult<Vec<Value>>>()?;

                self.propose(thread, ProposalKind::CreateSubtasks, json!({ "items": items }))
            }
            protocol::PROPOSE_PUSH => {
                let branch = thread.branch.clone().ok_or_else(|| CoreError::not_possible("The thread has no branch to push"))?;
                let note = input["note"].as_str().unwrap_or_default().trim();
                // The commit the branch is at now: what a person is shown, and all that an approval covers.
                let commit = thread.worktree.as_deref().and_then(|worktree| git::head(Path::new(&thread.repo_path), Path::new(worktree)).ok());

                self.propose(thread, ProposalKind::Push, json!({ "branch": branch, "note": hub_core::text::truncate(note, 2000), "commit": commit }))
            }
            protocol::PROPOSE_PULL_REQUEST => {
                let branch = thread.branch.clone().ok_or_else(|| CoreError::not_possible("The thread has no branch to open a pull request from"))?;
                let body = input["body"].as_str().unwrap_or_default().trim();

                self.propose(
                    thread,
                    ProposalKind::PullRequest,
                    json!({ "branch": branch, "title": text_of(input, "title", 200)?, "body": hub_core::text::truncate(body, 20_000) }),
                )
            }
            protocol::COMMIT_CHANGES => {
                if !policy::writes_files(thread.kind) {
                    return Err(CoreError::not_possible("This thread only reads; it has nothing to commit"));
                }

                let worktree = thread.worktree.clone().ok_or_else(|| CoreError::not_possible("The thread has no working tree"))?;
                let paths: Vec<String> = input["paths"]
                    .as_array()
                    .map(|paths| paths.iter().filter_map(|path| path.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let made = git::commit(Path::new(&thread.repo_path), Path::new(&worktree), text_of(input, "message", 4000)?, &paths)?;
                let event = self.store.add_event(thread.id, EventKind::Status, &format!("Committed {made}"), &Value::Null, false)?;

                self.emit(AppEvent::ThreadEvent { thread_id: thread.id, event });

                Ok(json!(format!("Committed {made}")))
            }
            other => Err(CoreError::not_possible(format!("There is no tool called {other}"))),
        }
    }

    /// A push or a pull request as the run on the server shows it.
    fn shown_on_run(kind: ProposalKind, payload: &Value) -> Value {
        match kind {
            ProposalKind::PullRequest => json!({ "title": payload["title"], "body": payload["body"], "head": payload["branch"] }),
            _ => payload.clone(),
        }
    }

    /// Tells the run how a push or a pull request decided here went. The run only shows it,
    /// so a server that cannot be told now is told on a later round (`settle_decided`).
    fn settle_on_run(&self, proposal: &Proposal) {
        // A headless runner's pushes and pull requests are decided on the web, not settled from here.
        if self.headless.is_some() {
            return;
        }

        let (Some(remote_id), false) = (proposal.remote_id, proposal.kind.is_remote()) else { return };
        let approve = match proposal.state {
            ProposalState::Approved => true,
            ProposalState::Rejected | ProposalState::Failed => false,
            ProposalState::Pending => return,
        };

        let _ = self.client().and_then(|client| self.seen(client.settle_proposal(remote_id, approve, proposal.note.as_deref(), &proposal.result)));
    }

    /// Catches up on what `settle_on_run` could not say: a push or a pull request the run
    /// still shows as undecided although it was decided here.
    fn settle_decided(&self, proposals: &[RunProposal]) {
        for remote in proposals.iter().filter(|remote| remote.state == RunProposalState::Pending) {
            if let Ok(Some(local)) = self.store.proposal_by_remote(remote.id) {
                self.settle_on_run(&local);
            }
        }
    }

    /// Records a proposal. It also becomes a proposal of the run on the server: a change to
    /// the issue so that it can be decided on the web as well, a push or a pull request so
    /// that the web shows what waits here. Those two are decided only here.
    fn propose(&self, thread: &Thread, kind: ProposalKind, payload: Value) -> CoreResult<Value> {
        let pending_same = self
            .store
            .proposals(thread.id)?
            .into_iter()
            .find(|proposal| proposal.kind == kind && proposal.state == ProposalState::Pending && !kind.is_remote());

        if let Some(existing) = pending_same {
            // One pending push or pull request per thread: the newer wording replaces the older.
            self.store.decide_proposal(existing.id, ProposalState::Rejected, Some("Replaced by a newer proposal of the thread"), &Value::Null)?;
            self.settle_on_run(&self.store.proposal(existing.id)?);
        }

        // The person deciding on the web has to see what the proposal is about before it arrives.
        if !kind.is_remote() {
            self.upload_changes(thread);
        }

        let shown = if kind.is_remote() { payload.clone() } else { Self::shown_on_run(kind, &payload) };
        let remote = match thread.run_id {
            Some(run_id) => match self.client().and_then(|client| self.seen(client.add_run_proposal(run_id, kind.as_str(), &shown))) {
                Ok(proposal) => Some(proposal),
                // The server checked the proposal against the project and refused it (an unknown
                // status, say): the agent reads why and can correct it.
                Err(CoreError::Api { status: 400 | 422, message, .. }) if kind.is_remote() => return Err(CoreError::not_possible(message)),
                // A push or a pull request is decided here whether or not the server can show it.
                Err(_) => None,
            },
            None => None,
        };
        let proposal = self.store.add_proposal(thread.id, kind, &payload, remote.as_ref().map(|proposal| proposal.id))?;

        // A kind the project trusts is applied by the server at once.
        if let Some(remote) = remote.filter(|remote| remote.state != RunProposalState::Pending) {
            self.apply_remote_proposals(thread, &[remote])?;
        }

        let what = match (kind, self.headless.is_some()) {
            (ProposalKind::Comment, _) => "a comment on the issue",
            (ProposalKind::Transition, _) => "a status change",
            (ProposalKind::CreateSubtasks, _) => "subtasks",
            (ProposalKind::Push, false) => "pushing the branch (decided in the desktop app)",
            (ProposalKind::PullRequest, false) => "opening a pull request (decided in the desktop app)",
            (ProposalKind::Push, true) => "pushing the branch (decided on the web)",
            (ProposalKind::PullRequest, true) => "opening a pull request (decided on the web)",
        };
        let event = self.store.add_event(
            thread.id,
            EventKind::Status,
            &format!("Proposed: {what}"),
            &json!({ "proposalId": proposal.id, "kind": kind }),
            false,
        )?;

        self.emit(AppEvent::ThreadEvent { thread_id: thread.id, event });

        Ok(json!(format!("Recorded as proposal #{}. {RECORDED}", proposal.id)))
    }

    // ---------- decisions ----------

    /// Takes over what the server says about proposals: decided on the web, or applied at once.
    fn apply_remote_proposals(&self, thread: &Thread, proposals: &[RunProposal]) -> CoreResult<()> {
        for remote in proposals {
            let Some(local) = self.store.proposal_by_remote(remote.id)? else { continue };
            let state = match remote.state {
                RunProposalState::Pending | RunProposalState::Other => continue,
                // Approved on the web for a headless runner means "carry it out", which
                // `carry_out_approved` does and records.
                RunProposalState::Approved if self.headless.is_some() && !local.kind.is_remote() => continue,
                RunProposalState::Approved => ProposalState::Approved,
                RunProposalState::Rejected => ProposalState::Rejected,
                RunProposalState::Failed => ProposalState::Failed,
            };

            if self.store.decide_proposal(local.id, state, remote.note.as_deref(), &remote.result)? {
                self.emit(AppEvent::ThreadChanged { thread_id: thread.id });
                self.data_changed("issue", Some(&thread.issue_key));
            }
        }

        Ok(())
    }

    /// Applies an approved proposal of a thread that has no run on the server, through the
    /// ordinary API, as the person who clicked.
    fn apply_here(&self, proposal: &Proposal) -> CoreResult<Value> {
        let client = self.client()?;
        let key = proposal.issue_key.as_str();

        match proposal.kind {
            ProposalKind::Comment => {
                let comment = client.add_comment(key, proposal.payload["body"].as_str().unwrap_or_default())?;

                Ok(json!({ "commentId": comment.id }))
            }
            ProposalKind::Transition => {
                let (issue, project) = Self::issue_with_project(&client, key)?;
                let wanted = proposal.payload["status"].as_str().unwrap_or_default();
                let statuses = project.map(|project| project.statuses).unwrap_or_default();
                let status = statuses.iter().find(|status| status.name.eq_ignore_ascii_case(wanted)).ok_or_else(|| {
                    let valid = statuses.iter().map(|status| status.name.as_str()).collect::<Vec<_>>().join(", ");

                    CoreError::not_possible(format!("Unknown status \"{wanted}\" (valid: {valid})"))
                })?;

                client.update_issue(&issue.row.key, &IssuePatch { status_id: Some(status.id), ..Default::default() })?;

                Ok(json!({ "status": status.name }))
            }
            ProposalKind::CreateSubtasks => {
                let (issue, project) = Self::issue_with_project(&client, key)?;
                let subtask = project.and_then(|project| project.types.into_iter().find(|kind| kind.level == TypeLevel::Subtask));
                let mut keys = Vec::new();

                for item in proposal.payload["items"].as_array().cloned().unwrap_or_default() {
                    let fields = IssuePatch {
                        summary: item["summary"].as_str().map(String::from),
                        description: item["description"].as_str().map(String::from),
                        type_id: subtask.as_ref().map(|kind| kind.id),
                        parent_key: Some(Some(issue.row.key.clone())),
                        ..Default::default()
                    };

                    keys.push(client.create_issue(&NewIssue { project_key: issue.row.project_key.clone(), fields })?.key);
                }

                Ok(json!({ "keys": keys }))
            }
            ProposalKind::Push | ProposalKind::PullRequest => {
                Err(CoreError::not_possible("A push or a pull request goes out through its own preview"))
            }
        }
    }

    /// The person's decision on a proposal. Nothing a thread proposed takes effect before this.
    pub fn decide_proposal(&self, id: i64, approve: bool, note: Option<&str>) -> CoreResult<Proposal> {
        let proposal = self.store.proposal(id)?;
        let thread = self.store.thread(proposal.thread_id)?;
        let note = note.map(str::trim).filter(|note| !note.is_empty());

        if proposal.state != ProposalState::Pending {
            return Err(CoreError::not_possible("This proposal is already decided"));
        }

        match (proposal.kind.is_remote(), proposal.remote_id, approve) {
            (true, Some(remote_id), _) => {
                let detail = self.seen(self.client()?.decide_proposal(remote_id, approve, note))?;

                self.apply_remote_proposals(&thread, &detail.proposals)?;
            }
            (true, None, true) => match self.seen(self.apply_here(&proposal)) {
                Ok(result) => {
                    self.store.decide_proposal(id, ProposalState::Approved, note, &result)?;
                    self.data_changed("issue", Some(&thread.issue_key));
                }
                Err(error) if error.is_offline() => return Err(error),
                Err(error) => {
                    self.store.decide_proposal(id, ProposalState::Failed, Some(&error.to_string()), &Value::Null)?;
                }
            },
            (false, _, true) => {
                return Err(CoreError::not_possible("Approve a push or a pull request from its preview, where what goes out is shown"))
            }
            (_, _, false) => {
                self.store.decide_proposal(id, ProposalState::Rejected, note, &Value::Null)?;
                self.settle_on_run(&self.store.proposal(id)?);
            }
        }

        self.emit(AppEvent::ThreadChanged { thread_id: thread.id });
        self.store.proposal(id)
    }

    fn settle_local(&self, thread_id: i64, kind: ProposalKind, state: ProposalState, result: &Value) -> CoreResult<()> {
        for proposal in self.store.proposals(thread_id)? {
            if proposal.kind == kind && proposal.state == ProposalState::Pending {
                self.store.decide_proposal(proposal.id, state, None, result)?;
                self.settle_on_run(&self.store.proposal(proposal.id)?);
            }
        }

        self.emit(AppEvent::ThreadChanged { thread_id });

        Ok(())
    }

    fn push_read(&self, thread: &Thread, commit: Option<&str>) -> CoreResult<(git::PushRead, String)> {
        let branch = thread.branch.clone().ok_or_else(|| CoreError::not_possible("The thread has no branch yet"))?;
        let fork_point = thread.base_commit.clone().unwrap_or_default();

        Ok((git::read_push(Path::new(&thread.repo_path), &branch, commit, &fork_point)?, branch))
    }

    /// Exactly what pushing the thread's branch would send, and anything in it that needs a
    /// second, explicit confirmation.
    pub fn push_preview(&self, thread_id: i64) -> CoreResult<PushPreview> {
        let thread = self.store.thread(thread_id)?;
        let (read, branch) = self.push_read(&thread, None)?;
        let uncommitted = self.thread_diff(thread_id).map(|diff| diff.uncommitted).unwrap_or(0);

        Ok(PushPreview {
            thread_id,
            branch,
            remote_url: read.remote_url,
            commit: read.commit,
            base: read.base,
            commits: read.commits,
            files: read.files,
            findings: push_guard::review(&read.readings),
            blocked: read.blocked,
            uncommitted,
        })
    }

    /// Pushes the commit the person looked at, and only that one: a commit the thread added
    /// since stays here. With findings (CI or build files, something like a secret) the push
    /// is refused unless `confirmed_findings` says the person confirmed them.
    pub fn push(&self, thread_id: i64, commit: &str, confirmed_findings: bool) -> CoreResult<PushResult> {
        let thread = self.store.thread(thread_id)?;
        let (read, branch) = self.push_read(&thread, Some(commit))?;

        if let Some(blocked) = read.blocked {
            return Err(CoreError::not_possible(blocked));
        }

        let findings = push_guard::review(&read.readings);

        if !findings.is_empty() && !confirmed_findings {
            let listed = findings.iter().map(|found| found.detail.clone()).collect::<Vec<_>>().join("; ");

            return Err(CoreError::NeedsConfirmation(listed));
        }

        git::push(Path::new(&thread.repo_path), &branch, &read.commit)?;
        self.say_pushed(thread_id, &branch, &read)?;

        let result = PushResult { branch: branch.clone(), commit: read.commit.clone(), remote_url: read.remote_url };

        self.settle_local(thread_id, ProposalKind::Push, ProposalState::Approved, &serde_json::to_value(&result)?)?;

        Ok(result)
    }

    fn say_pushed(&self, thread_id: i64, branch: &str, read: &git::PushRead) -> CoreResult<()> {
        let short: String = read.commit.chars().take(8).collect();
        let count = if read.commits.len() == 1 { "1 commit".to_string() } else { format!("{} commits", read.commits.len()) };
        let event = self.store.add_event(
            thread_id,
            EventKind::Status,
            &format!("Pushed {branch} at {short} ({count})"),
            &json!({ "commit": read.commit }),
            false,
        )?;

        self.emit(AppEvent::ThreadEvent { thread_id, event });

        Ok(())
    }

    fn default_branch_name(repo: &Path) -> String {
        let base = git::base_ref(repo).unwrap_or_else(|_| "main".to_string());

        base.strip_prefix("origin/").unwrap_or(&base).to_string()
    }

    fn gh_installed() -> bool {
        std::process::Command::new("gh").arg("--version").output().is_ok_and(|output| output.status.success())
    }

    /// What the server's code module says about the thread's issue; none when the module is
    /// off there or the server cannot be asked.
    fn code_on_server(&self, thread: &Thread) -> Option<Value> {
        self.client().ok()?.issue(&thread.issue_key).ok()?.panels.get("github").cloned()
    }

    /// The pull request of this thread's branch that the server already knows, if any.
    fn known_pull_request(code: Option<&Value>, thread: &Thread) -> Option<String> {
        let branch = thread.branch.as_deref()?;

        code?
            .get("prs")?
            .as_array()?
            .iter()
            .find(|pr| pr["headRef"].as_str() == Some(branch) && pr["state"].as_str() == Some("open"))
            .and_then(|pr| pr["url"].as_str().map(String::from))
    }

    /// The server's id of the repository, when it is connected to the issue's project there:
    /// the server can then open the pull request with its own GitHub token.
    fn repo_on_server(code: Option<&Value>, repo: Option<&str>) -> Option<i64> {
        let repo = repo?;

        code?.get("repos")?.as_array()?.iter().find(|known| known["fullName"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(repo)))?["id"]
            .as_i64()
    }

    pub fn pull_request_preview(&self, thread_id: i64) -> CoreResult<PullRequestPreview> {
        let thread = self.store.thread(thread_id)?;
        let branch = thread.branch.clone().ok_or_else(|| CoreError::not_possible("The thread has no branch yet"))?;
        let repo_path = Path::new(&thread.repo_path);
        let mapping = self.store.repo(thread.repo_id).ok();
        let repo = mapping
            .and_then(|mapping| mapping.github_repo)
            .or_else(|| git::inspect(repo_path).ok().and_then(|found| found.remote_url).and_then(|url| git::repo_of_remote(&url)));
        let base = Self::default_branch_name(repo_path);
        let proposed = self
            .store
            .proposals(thread_id)?
            .into_iter()
            .rev()
            .find(|proposal| proposal.kind == ProposalKind::PullRequest && proposal.state == ProposalState::Pending);
        let title = proposed
            .as_ref()
            .and_then(|proposal| proposal.payload["title"].as_str().map(String::from))
            .unwrap_or_else(|| format!("{} {}", thread.issue_key, thread.issue_summary));
        let body = proposed
            .as_ref()
            .and_then(|proposal| proposal.payload["body"].as_str().map(String::from))
            .filter(|body| !body.is_empty())
            .or(thread.summary.clone())
            .unwrap_or_default();
        let head = thread.worktree.as_deref().and_then(|worktree| git::head(repo_path, Path::new(worktree)).ok());
        let code = self.code_on_server(&thread);
        let existing_url = thread.pull_request_url.clone().or_else(|| Self::known_pull_request(code.as_ref(), &thread));
        let server_repo = Self::repo_on_server(code.as_ref(), repo.as_deref());

        if let (Some(url), None) = (&existing_url, &thread.pull_request_url) {
            self.store.set_pull_request(thread_id, url)?;
        }

        Ok(PullRequestPreview {
            thread_id,
            compare_url: repo.as_ref().map(|repo| format!("https://github.com/{repo}/compare/{base}...{branch}?expand=1")),
            repo,
            base,
            pushed: head.is_some_and(|head| git::is_pushed(repo_path, &branch, &head)),
            head: branch,
            title,
            body,
            can_create: self.github_cli || server_repo.is_some(),
            server_repo,
            existing_url,
        })
    }

    /// Opens the pull request after the person's click: with `gh` when it is installed (the
    /// pull request is then the person's own), else through the server when it has the
    /// repository connected (with the server's GitHub token), else by handing back the page
    /// where the person finishes it. The page is also what is left when the server cannot.
    pub fn open_pull_request(&self, thread_id: i64, title: &str, body: &str) -> CoreResult<PullRequestResult> {
        let preview = self.pull_request_preview(thread_id)?;
        let thread = self.store.thread(thread_id)?;

        if !preview.pushed {
            return Err(CoreError::not_possible("Push the branch first: the server does not have the thread's last commit"));
        }

        if let Some(url) = preview.existing_url {
            return Ok(PullRequestResult { url: Some(url), browser_url: None });
        }

        let repo = preview.repo.ok_or_else(|| CoreError::not_possible("The repository's address on GitHub is not known"))?;

        if title.trim().is_empty() {
            return Err(CoreError::not_possible("The pull request needs a title"));
        }

        let browser = |compare_url: Option<String>| {
            let query = format!("&title={}&body={}", url_encode(title.trim()), url_encode(body));

            PullRequestResult { url: None, browser_url: compare_url.map(|url| format!("{url}{query}")) }
        };
        let result = if self.github_cli {
            let output = std::process::Command::new("gh")
                .args(["pr", "create", "--repo", &repo, "--head", &preview.head, "--base", &preview.base, "--title", title.trim(), "--body", body])
                .current_dir(&thread.repo_path)
                .output()?;

            if !output.status.success() {
                return Err(CoreError::not_possible(format!("gh pr create: {}", String::from_utf8_lossy(&output.stderr).trim())));
            }

            let url = String::from_utf8_lossy(&output.stdout).lines().rev().find(|line| line.starts_with("https://")).map(str::to_string);

            PullRequestResult { url, browser_url: None }
        } else if let Some(repo_id) = preview.server_repo {
            let new = json!({ "repoId": repo_id, "head": preview.head, "base": preview.base, "title": title.trim(), "body": body });

            match self.client().and_then(|client| self.seen(client.open_pull_request(&thread.issue_key, &new))) {
                Ok(opened) => PullRequestResult { url: opened["url"].as_str().map(String::from), browser_url: None },
                Err(error) if error.is_offline() => return Err(error),
                // No token on the server, or GitHub said no: the person can still finish it by hand.
                Err(_) => browser(preview.compare_url),
            }
        } else {
            browser(preview.compare_url)
        };

        if let Some(url) = &result.url {
            self.store.set_pull_request(thread_id, url)?;

            let event = self.store.add_event(thread_id, EventKind::Status, &format!("Pull request opened: {url}"), &json!({ "url": url }), false)?;

            self.emit(AppEvent::ThreadEvent { thread_id, event });
        }

        self.settle_local(thread_id, ProposalKind::PullRequest, ProposalState::Approved, &serde_json::to_value(&result)?)?;

        Ok(result)
    }

    /// Notes the pull request the person opened in the browser, so the thread and its run show it.
    pub fn record_pull_request(&self, thread_id: i64, url: &str) -> CoreResult<Thread> {
        let url = url.trim();

        if !url.starts_with("https://") || url.contains(char::is_whitespace) {
            return Err(CoreError::not_possible("The address of a pull request starts with https://"));
        }

        self.store.set_pull_request(thread_id, url)?;

        let event = self.store.add_event(thread_id, EventKind::Status, &format!("Pull request: {url}"), &json!({ "url": url }), false)?;

        self.emit(AppEvent::ThreadEvent { thread_id, event });
        self.settle_local(thread_id, ProposalKind::PullRequest, ProposalState::Approved, &json!({ "url": url }))?;
        self.store.thread(thread_id)
    }

    // ---------- the runner: docs/agent-runs.md ----------

    fn identity(&self) -> CoreResult<RunnerIdentity> {
        let settings = self.settings();
        let version = env!("CARGO_PKG_VERSION").to_string();

        // A headless runner clones what a run names, so it works for the projects it was
        // told to, or for all of them; the desktop app works for those it has a clone of.
        if let Some(headless) = &self.headless {
            return Ok(RunnerIdentity {
                id: settings.runner_id,
                name: settings.runner_name,
                kinds: headless.kinds.clone(),
                projects: headless.projects.clone(),
                version,
                interactive: false,
                pinned_only: false,
                shared: headless.shared,
            });
        }

        let mut projects: Vec<String> = self.store.repos()?.into_iter().map(|repo| repo.project_key).collect();

        projects.sort();
        projects.dedup();

        Ok(RunnerIdentity {
            id: settings.runner_id,
            name: settings.runner_name,
            kinds: RUNNER_KINDS.to_vec(),
            projects: Some(projects),
            version,
            interactive: true,
            pinned_only: false,
            shared: false,
        })
    }

    fn runs_api(&self) -> bool {
        lock(&self.runner).runs_api != Some(false)
    }

    fn heartbeat(&self, client: &HubClient) {
        let due = lock(&self.runner).last_beat.is_none_or(|at| at.elapsed() >= HEARTBEAT_EVERY);

        if !due {
            return;
        }

        let outcome = self.identity().and_then(|identity| self.seen(client.heartbeat(&identity)));
        let mut runner = lock(&self.runner);

        runner.last_beat = Some(Instant::now());

        match outcome {
            Ok(()) => {
                runner.runs_api = Some(true);
                runner.last_heartbeat = Some(store::now());
                runner.error = None;
            }
            Err(error) if error.is_not_found() => {
                runner.runs_api = Some(false);
                runner.error = Some("This server has no agent runs API; threads work locally and are not shown on the web.".to_string());
            }
            // Only an administrator's token may run the team's runner. Asked again with the next heartbeat.
            Err(CoreError::Api { status: 403, message, .. }) if self.headless.as_ref().is_some_and(|headless| headless.shared) => {
                runner.error = Some(format!(
                    "The server does not take this runner as a shared one: {message}. Give it an administrator's API token (TEAM_HUB_TOKEN), or set RUNNER_SHARED=0 and keys of its own. Trying again."
                ));
            }
            Err(error) => runner.error = Some(error.to_string()),
        }
    }

    /// Takes queued runs from the server: first the ones this app created for its own
    /// threads, then delegated ones while a slot is free. Returns how many were taken.
    pub fn claim_runs(&self) -> usize {
        let Ok(client) = self.client() else { return 0 };
        let Ok(identity) = self.identity() else { return 0 };
        let settings = self.settings();
        let mut taken = 0;

        // The server names the runner on a run from its last heartbeat.
        self.heartbeat(&client);

        // A shared runner the server has not taken as one asks for nothing: what it would be
        // given is meant for a runner that gets people's keys.
        if identity.shared && lock(&self.runner).error.is_some() {
            return 0;
        }

        while taken < MAX_CLAIMS_PER_TICK && self.runs_api() {
            let waiting_for_own = !lock(&self.runner).attach.is_empty();
            let preparing = self.headless.as_ref().map_or(0, |headless| lock(&headless.preparing).len());
            let working = self.store.threads().map(|threads| threads.iter().filter(|thread| thread.state.is_active()).count()).unwrap_or(usize::MAX);
            let busy = working.saturating_add(preparing);
            let has_work = !identity.kinds.is_empty() && identity.projects.as_ref().is_none_or(|projects| !projects.is_empty());
            let has_slot = settings.runner_enabled && has_work && busy < settings.max_running as usize;

            if !waiting_for_own && !has_slot {
                break;
            }

            match self.seen(client.claim(&identity)) {
                Ok(Some(claimed)) => {
                    taken += 1;

                    let detail = claimed.detail;

                    if let (Some(headless), Some(keys)) = (&self.headless, claimed.credentials) {
                        lock(&headless.keys).insert(detail.run.id, keys);
                    }

                    if let Err(error) = self.adopt(&client, &detail) {
                        self.refuse_run(&client, detail.run.id, &error);
                    }
                }
                Ok(None) => {
                    // Whatever is still waiting was taken by another runner, or is gone.
                    for (_, thread_id) in lock(&self.runner).attach.drain() {
                        let _ = self.store.add_event(
                            thread_id,
                            EventKind::Status,
                            "The run made for this thread was taken elsewhere; the thread is not shown on the server.",
                            &Value::Null,
                            true,
                        );
                    }

                    break;
                }
                Err(error) => {
                    if error.is_not_found() {
                        lock(&self.runner).runs_api = Some(false);
                    }

                    break;
                }
            }
        }

        taken
    }

    /// Ends a claimed run that could not be started here, with the reason for the person.
    fn refuse_run(&self, client: &HubClient, run_id: i64, error: &CoreError) {
        let who = if self.headless.is_some() { "The runner" } else { "The desktop runner" };

        self.end_failed(client, run_id, format!("{who} could not start this run: {error}"));
    }

    fn end_failed(&self, client: &HubClient, run_id: i64, summary: String) {
        let _ = client.patch_run(run_id, &RunPatch { state: Some(RunState::Failed), summary: Some(summary), ..Default::default() });
    }

    /// Turns a claimed run into a thread, or ties it to the thread it was created for.
    fn adopt(&self, client: &HubClient, detail: &AgentRunDetail) -> CoreResult<()> {
        let run = &detail.run;
        let own = lock(&self.runner).attach.remove(&run.id);

        if let Some(thread_id) = own {
            self.store.set_run(thread_id, Some(run.id))?;
            self.emit(AppEvent::ThreadChanged { thread_id });

            return Ok(());
        }

        if self.store.thread_of_run(run.id)?.is_some() {
            return Ok(());
        }

        let (project_key, _) = hub_core::keys::parse_issue_key(&run.issue_key).ok_or_else(|| CoreError::not_possible("the run names no issue"))?;

        if let (Some(headless), Some(core)) = (&self.headless, self.this.upgrade()) {
            // Cloning can take minutes, and the runner has to keep answering meanwhile.
            let (client, detail) = (client.clone(), detail.clone());

            lock(&headless.preparing).insert(run.id);

            // On the runtime's own blocking threads, because a thread is started from there.
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => drop(runtime.spawn_blocking(move || core.adopt_with_clone(&client, &detail, &project_key))),
                Err(_) => core.adopt_with_clone(&client, &detail, &project_key),
            }

            return Ok(());
        }

        let repo = self.repo_for(&project_key, None, run.repo.as_deref())?;

        self.enqueue_run(client, detail, project_key, repo)
    }

    /// The headless way to a repository: the clone of what the run names, made or fetched
    /// now and mapped to the run's project. The run then becomes a thread like any other.
    fn adopt_with_clone(&self, client: &HubClient, detail: &AgentRunDetail, project_key: &str) {
        let Some(headless) = &self.headless else { return };
        let run = &detail.run;
        let cloned = || -> CoreResult<RepoMapping> {
            let name = run.repo.as_deref().map(str::trim).filter(|name| !name.is_empty()).ok_or_else(|| {
                CoreError::not_possible(
                    "the run names no repository. A cloud runner works in the repository a run names as owner/name: connect one to the project and start the run again",
                )
            })?;
            let token = self.github_token_of(run.id)?;
            let path = headless.repos.ensure(name, token.as_ref())?;

            self.store.add_repo(project_key, Some(&name.to_lowercase()), &path.to_string_lossy())
        };
        let started = cloned().and_then(|repo| self.enqueue_run(client, detail, project_key.to_string(), repo));

        lock(&headless.preparing).remove(&run.id);

        if let Err(error) = started {
            self.refuse_run(client, run.id, &error);
        }

        self.wake.notify_one();
    }

    fn enqueue_run(&self, client: &HubClient, detail: &AgentRunDetail, project_key: String, repo: RepoMapping) -> CoreResult<()> {
        let run = &detail.run;
        let (issue, project) = Self::issue_with_project(client, &run.issue_key)?;
        let kind = run.kind.thread_kind();
        let seed = ThreadSeed {
            issue_key: issue.row.key.clone(),
            issue_summary: issue.row.summary.clone(),
            issue_type: Self::type_name(&issue, project.as_ref()),
            project_key,
            kind,
            instructions: run.instructions.clone(),
            model: run.model.clone().filter(|model| !model.trim().is_empty()).unwrap_or_else(|| self.settings().default_model),
            repo,
            wanted_branch: run.branch.clone().filter(|branch| git::is_branch_name(branch)),
            origin: ThreadOrigin::Delegated,
            run_id: Some(run.id),
            prompt: policy::first_prompt(kind, Some((&issue, project.as_ref())), &issue.row.key, &issue.row.summary, &run.instructions),
        };
        let thread = self.runtime().enqueue(&seed)?;

        self.emit(AppEvent::Notify {
            title: format!("{} was delegated to this {}", thread.issue_key, if self.headless.is_some() { "runner" } else { "computer" }),
            body: thread.issue_summary.clone(),
            thread_id: Some(thread.id),
            issue_key: Some(thread.issue_key),
        });

        Ok(())
    }

    /// What the run should say. A thread that is done while its push or pull request still
    /// waits for the person keeps the run `waiting`: a run that has ended takes no more
    /// writes, and the branch and the pull request are reported when they exist.
    fn patch_of(&self, thread: &Thread) -> RunPatch {
        let finished = matches!(thread.state, ThreadState::Done | ThreadState::Failed | ThreadState::Waiting | ThreadState::Stopped);
        let decided_here = self
            .store
            .proposals(thread.id)
            .unwrap_or_default()
            .iter()
            .any(|proposal| !proposal.kind.is_remote() && proposal.state == ProposalState::Pending);
        let state = match thread.state {
            ThreadState::Done if decided_here => Some(RunState::Waiting),
            state => RunState::of_thread(state),
        };

        RunPatch {
            state,
            summary: thread.summary.clone().filter(|_| finished),
            branch: thread.branch.clone(),
            pull_request_url: thread.pull_request_url.clone(),
            model: Some(thread.model.clone()),
            input_tokens: (thread.input_tokens > 0).then_some(thread.input_tokens),
            output_tokens: (thread.output_tokens > 0).then_some(thread.output_tokens),
            cost_usd: (thread.cost_usd > 0.0).then_some(thread.cost_usd),
        }
    }

    /// Sends what the run on the server does not have yet: the thread's events in order, then
    /// its state. Says whether the server asked for the run to be cancelled.
    fn sync_thread(&self, thread: &Thread) -> bool {
        if self.holds_back(thread.id) {
            return false;
        }

        let (Some(run_id), Ok(client)) = (thread.run_id, self.client()) else { return false };

        let mut posted = false;

        for event in self.store.unmirrored_events(thread.id).unwrap_or_default() {
            // What the person said to the thread here goes to the run as said by them. It
            // comes back among the run's answers, so it is marked as seen, or the thread would
            // hear it a second time. An older server takes answers only through its own call
            // for a pending question; there it is told as a line of the runner.
            let mut outcome = self.seen(client.add_run_event(run_id, RunEventKind::of_thread(event.kind), &event.text, &event.data));

            if event.kind == EventKind::Answer {
                match &outcome {
                    Ok(answer_id) => {
                        let _ = self.store.set_answers_seen(thread.id, *answer_id);
                    }
                    Err(error) if error.is_bad_request() => {
                        let line = format!("Said in the desktop app: {}", hub_core::text::truncate(&event.text.replace('\n', " "), 400));

                        outcome = self.seen(client.add_run_event(run_id, RunEventKind::Status, &line, &event.data));
                    }
                    Err(_) => {}
                }
            }

            match outcome {
                Ok(_) => posted = true,
                Err(error) if error.is_offline() => return false,
                // The server refused this one (the run is over, or it is not ours any more);
                // sending it again would not change that.
                Err(_) => {}
            }

            let _ = self.store.mark_mirrored(event.id);
        }

        let Ok(private) = self.store.thread_private(thread.id) else { return false };
        let mut sent: RunPatch = serde_json::from_value(private.run_sent).unwrap_or_default();
        let now = self.patch_of(thread);

        // An event of the runner puts a waiting run back to running on the server; a run that
        // still waits for a person has to say so again.
        if posted && now.state == Some(RunState::Waiting) {
            sent.state = None;
        }

        let patch = now.changed_since(&sent);

        if patch.is_empty() {
            return false;
        }

        // A run that ends takes no more writes: what the thread committed goes first.
        if patch.state.is_some() {
            self.upload_changes(thread);
        }

        match self.seen(client.patch_run(run_id, &patch)) {
            Ok(run) => {
                let _ = self.store.set_run_sent(thread.id, &serde_json::to_value(&now).unwrap_or_default());

                run.cancel_requested && !run.state.is_finished()
            }
            Err(error) if error.is_offline() => false,
            Err(_) => {
                let _ = self.store.set_run_sent(thread.id, &serde_json::to_value(&now).unwrap_or_default());

                false
            }
        }
    }

    /// Reads the runs of live threads: a cancel request, answers given on the web, decisions
    /// on proposals. Returns the threads that have to stop.
    fn poll_runs(&self, client: &HubClient) -> Vec<i64> {
        let mut to_stop = Vec::new();

        for thread in self.store.threads_with_live_runs().unwrap_or_default() {
            let Some(run_id) = thread.run_id else { continue };
            let Ok(detail) = self.seen(client.run(run_id)) else { continue };
            let live = matches!(thread.state, ThreadState::Queued | ThreadState::Running | ThreadState::Waiting);

            let cancelled = detail.run.cancel_requested || detail.run.state == RunState::Cancelled;

            if cancelled {
                for proposal in self.store.proposals(thread.id).unwrap_or_default() {
                    if proposal.state == ProposalState::Pending {
                        let _ = self.store.decide_proposal(proposal.id, ProposalState::Rejected, Some("The run was cancelled"), &Value::Null);
                    }
                }

                self.emit(AppEvent::ThreadChanged { thread_id: thread.id });
            }

            if live && cancelled {
                to_stop.push(thread.id);

                continue;
            }

            let seen = self.store.thread_private(thread.id).map(|private| private.answers_seen).unwrap_or(i64::MAX);

            for answer in detail.events.iter().filter(|event| event.kind == RunEventKind::Answer && event.id > seen) {
                let _ = self.store.set_answers_seen(thread.id, answer.id);
                let _ = self.runtime().resume(thread.id, Some(&answer.text), true);
            }

            self.upload_changes(&thread);
            self.carry_out_approved(&thread, &detail.proposals);

            let _ = self.apply_remote_proposals(&thread, &detail.proposals);

            self.settle_decided(&detail.proposals);

            // The server moves a run by itself too (a decision puts a waiting run back to
            // running). When the thread is at rest and the run says otherwise, say it again.
            let wanted = self.patch_of(&thread).state;
            let differs = wanted.is_some_and(|wanted| wanted != detail.run.state) && !detail.run.state.is_finished();

            if differs && !thread.state.is_active() {
                if let Ok(mut private) = self.store.thread_private(thread.id) {
                    if let Some(sent) = private.run_sent.as_object_mut() {
                        sent.remove("state");
                    }

                    let _ = self.store.set_run_sent(thread.id, &private.run_sent);
                }

                self.wake.notify_one();
            }
        }

        to_stop
    }

    /// Raises a system notification for each new mention or assignment.
    fn check_notifications(&self, client: &HubClient) {
        let Ok(notifications) = client.notifications() else { return };
        let newest = notifications.iter().map(|notification| notification.id).max().unwrap_or(0);
        let seen = self.seen_notification.swap(newest, Ordering::SeqCst);

        if seen < 0 {
            // The first look only sets the mark: old notifications are not news.
            return;
        }

        for notification in notifications.iter().filter(|notification| notification.id > seen && notification.read_at.is_none()) {
            if matches!(notification.kind.as_str(), "mention" | "assigned") {
                self.emit(AppEvent::Notify {
                    title: notification.issue_key.clone().unwrap_or_else(|| "team-hub".to_string()),
                    body: notification.text.clone(),
                    thread_id: None,
                    issue_key: notification.issue_key.clone(),
                });
            }
        }
    }

    /// One round of the runner. `full` also talks to the server about things that did not
    /// start here: heartbeat, queued runs, answers and decisions. Returns threads to stop.
    pub fn tick(&self, full: bool) -> Vec<i64> {
        // A runner that is stopping claims nothing more and tells the server nothing.
        if self.headless.as_ref().is_some_and(|headless| headless.suspended.load(Ordering::SeqCst)) {
            return Vec::new();
        }

        let Ok(client) = self.client() else { return Vec::new() };
        let mut to_stop = Vec::new();

        if full {
            self.heartbeat(&client);
        }

        if !self.runs_api() {
            return to_stop;
        }

        if full {
            self.continue_interrupted(&client);
        }

        for thread in self.store.threads().unwrap_or_default().iter().filter(|thread| thread.run_id.is_some()) {
            if self.sync_thread(thread) {
                to_stop.push(thread.id);
            }
        }

        if full {
            self.claim_runs();
            to_stop.extend(self.poll_runs(&client));
            self.report_unreported();
            self.forget_keys();

            // Mentions and assignments are news for a person; a headless runner has none.
            if self.headless.is_none() {
                self.check_notifications(&client);
            }
        }

        to_stop.sort_unstable();
        to_stop.dedup();
        to_stop
    }

    pub fn runs(&self, mine: bool) -> CoreResult<Vec<AgentRun>> {
        self.seen(self.client()?.runs(mine, 50))
    }

    fn on_live(&self, event: &LiveEvent) {
        // A job's log lines are not news for whoever would run a job.
        if event.kind == "e2e.job" {
            self.jobs_wake.notify_one();
            self.data_changed("jobs", None);

            return;
        }

        if event.kind.starts_with("e2e.") {
            return;
        }

        let key = event.payload["issueKey"].as_str().or_else(|| event.payload["key"].as_str());
        let scope = if event.kind.starts_with("agent.") { "runs" } else { "issue" };

        self.data_changed(scope, key);
        self.wake.notify_one();
    }
}

// ---------- a runner nobody sits at: docs/cloud-runner.md ----------

impl Core {
    pub fn is_headless(&self) -> bool {
        self.headless.is_some()
    }

    /// The clones this runner keeps by itself: a headless runner's for all its work, the
    /// desktop app's for test jobs of repositories no local clone is mapped for.
    pub fn clones(&self) -> CoreResult<&Repos> {
        Ok(self.headless.as_ref().map_or(&self.job_clones, |headless| &headless.repos))
    }

    /// The repository a test job runs from, up to date with its origin. A headless runner
    /// clones or fetches it with the token the job came with. The desktop app uses the
    /// clone mapped to the project when there is one, fetched with the person's own git
    /// sign-in (their checkout is not touched: the job gets a copy of one commit), and
    /// otherwise a clone of its own.
    pub fn clone_for_job(&self, project_key: &str, repo: &str, token: Option<&Secret>) -> CoreResult<PathBuf> {
        if self.headless.is_some() {
            return self.clones()?.ensure(repo, token);
        }

        match self.repo_for(project_key, None, Some(repo)) {
            Ok(mapped) => {
                // A clone that cannot be fetched right now still has what it had.
                let _ = git::fetch(Path::new(&mapped.path), Some(&git::RemoteAuth::PersonsOwn));

                Ok(PathBuf::from(mapped.path))
            }
            Err(_) => self.job_clones.ensure(repo, None),
        }
    }

    /// Whether this runner takes test jobs now. The desktop app can be told not to.
    pub fn takes_test_jobs(&self) -> bool {
        self.headless.is_some() || self.settings().test_jobs
    }

    /// How this runner announces itself when it asks for a test job: the same id and name
    /// as for agent runs (docs/runners.md).
    pub fn job_identity(&self) -> CoreResult<RunnerIdentity> {
        let identity = self.identity()?;

        if self.headless.is_some() {
            return Ok(identity);
        }

        // On a person's computer a suite runs when it was sent there by name, unless the
        // person chose to take the team's queue too. Any project's: the repository is
        // cloned when no local clone is mapped.
        Ok(RunnerIdentity { projects: None, pinned_only: !self.settings().test_jobs_from_queue, ..identity })
    }

    // ---------- the same runners and work as the web shows: docs/runners.md ----------

    /// `GET /api/runners`: every runner the person may see, with what each takes and carries out.
    pub fn runners(&self) -> CoreResult<Value> {
        self.seen(self.client()?.get("/api/runners"))
    }

    /// Hands work on an issue to a runner by name, or to whichever fits. Nothing is started
    /// here: the run is carried out there, and this app follows it like the web does.
    pub fn delegate(&self, new: &Delegation) -> CoreResult<AgentRun> {
        let key = new.issue_key.trim().to_uppercase();
        let (project_key, _) =
            hub_core::keys::parse_issue_key(&key).ok_or_else(|| CoreError::not_possible(format!("\"{key}\" is not an issue key")))?;
        let named = new.repo.clone().filter(|repo| !repo.trim().is_empty());
        let mapped = || self.repo_for(&project_key, None, None).ok().map(|repo| Self::repo_name(&repo));
        let run = NewRun {
            kind: RunKind::of_thread(new.kind),
            instructions: new.instructions.trim().to_string(),
            model: new.model.clone().filter(|model| !model.trim().is_empty()),
            repo: named.or_else(mapped),
            shared: false,
            runner_id: None,
            on_runner: new.on_runner.clone().filter(|runner| !runner.trim().is_empty()),
        };
        let created = self.seen(self.client()?.create_run(&key, &run))?;

        self.data_changed("runs", Some(&key));

        Ok(created)
    }

    /// A run wherever it is carried out: its events and proposals, to follow it.
    pub fn run_detail(&self, id: i64) -> CoreResult<AgentRunDetail> {
        self.seen(self.client()?.run(id))
    }

    /// The person's decision on a proposal of a run another runner carries out: a change to
    /// the issue, or the push or pull request of a runner nobody sits at.
    pub fn decide_on_run(&self, proposal_id: i64, approve: bool, note: Option<&str>) -> CoreResult<AgentRunDetail> {
        let detail = self.seen(self.client()?.decide_proposal(proposal_id, approve, note.map(str::trim).filter(|note| !note.is_empty())))?;

        self.data_changed("runs", Some(&detail.run.issue_key));

        Ok(detail)
    }

    pub fn cancel_run(&self, id: i64) -> CoreResult<AgentRun> {
        self.seen(self.client()?.cancel_run(id))
    }

    fn e2e(project_key: &str, rest: &str) -> String {
        format!("/api/m/e2e/projects/{}{rest}", project_key.trim().to_uppercase())
    }

    /// The project's test settings (which repository, which ref by default); null when the
    /// server has no tests module.
    pub fn test_settings(&self, project_key: &str) -> CoreResult<Value> {
        match self.seen(self.client()?.get(&Self::e2e(project_key, "/settings"))) {
            Err(error) if error.is_not_found() => Ok(Value::Null),
            other => other,
        }
    }

    /// The project's test jobs, newest first.
    pub fn test_jobs(&self, project_key: &str) -> CoreResult<Value> {
        self.seen(self.client()?.get(&Self::e2e(project_key, "/jobs?limit=30")))
    }

    /// Starts a test job: on the runner named, or on whichever fits.
    pub fn start_test_job(&self, new: &NewTestJob) -> CoreResult<Value> {
        let text = |value: &Option<String>| value.as_deref().map(str::trim).filter(|text| !text.is_empty()).map(String::from);
        let mut body = json!({});

        for (name, value) in [("ref", text(&new.reference)), ("filter", text(&new.filter)), ("onRunner", text(&new.on_runner))] {
            if let Some(value) = value {
                body[name] = json!(value);
            }
        }

        let job = self.seen(self.client()?.post(&Self::e2e(&new.project_key, "/jobs"), &body))?;

        self.jobs_wake.notify_one();

        Ok(job)
    }

    /// A job with the log lines after `after`.
    pub fn test_job(&self, id: i64, after: i64) -> CoreResult<Value> {
        self.seen(self.client()?.get(&format!("/api/m/e2e/jobs/{id}?after={after}")))
    }

    pub fn cancel_test_job(&self, id: i64) -> CoreResult<Value> {
        self.seen(self.client()?.post(&format!("/api/m/e2e/jobs/{id}/cancel"), &json!({})))
    }

    /// Poked when the server announces a change to a test job.
    pub fn jobs_wake(&self) -> Arc<Notify> {
        Arc::clone(&self.jobs_wake)
    }

    /// The model key a thread's helper gets, on a shared runner: the one its run came with.
    fn model_key_of(&self, thread: &Thread) -> CoreResult<Option<Secret>> {
        let Some(headless) = self.headless.as_ref().filter(|headless| headless.shared) else { return Ok(None) };
        let keys = lock(&headless.keys);
        let held = thread.run_id.and_then(|run_id| keys.get(&run_id)).ok_or_else(|| CoreError::not_possible(LOST_KEYS))?;

        held.anthropic_api_key.clone().map(Some).ok_or_else(|| {
            CoreError::not_possible(
                "This run came without a model key. Store your Anthropic API key in team-hub under Settings → My keys and start it again.",
            )
        })
    }

    /// The GitHub token for a run's git commands: on a shared runner the one the run came
    /// with (none: the person stored none, and only what is public can be read); else the
    /// runner's own, which `Repos` holds.
    fn github_token_of(&self, run_id: i64) -> CoreResult<Option<Secret>> {
        let Some(headless) = self.headless.as_ref().filter(|headless| headless.shared) else { return Ok(None) };

        lock(&headless.keys).get(&run_id).map(|keys| keys.github_token.clone()).ok_or_else(|| CoreError::not_possible(LOST_KEYS))
    }

    /// Forgets the keys of runs that are over: nothing of theirs is left to do here.
    fn forget_keys(&self) {
        let Some(headless) = &self.headless else { return };
        let mut live: HashSet<i64> = self.store.threads_with_live_runs().unwrap_or_default().iter().filter_map(|thread| thread.run_id).collect();

        live.extend(lock(&headless.preparing).iter().copied());
        lock(&headless.keys).retain(|run_id, _| live.contains(run_id));
    }

    /// Whether the server is told nothing about this thread for now: the runner is stopping,
    /// or it has yet to learn whether the thread's run is still its own.
    fn holds_back(&self, thread_id: i64) -> bool {
        self.headless.as_ref().is_some_and(|headless| headless.suspended.load(Ordering::SeqCst) || lock(&headless.interrupted).contains(&thread_id))
    }

    /// Sends what the thread has committed to its run, when the run does not have that head
    /// yet (`PUT /runs/{id}/changes`), so a person can look before anything is pushed.
    fn upload_changes(&self, thread: &Thread) {
        let Some(headless) = &self.headless else { return };
        let (Some(run_id), Some(worktree), Some(base)) = (thread.run_id, thread.worktree.as_deref(), thread.base_commit.as_deref()) else { return };
        let repo = Path::new(&thread.repo_path);
        let Ok(head) = git::head(repo, Path::new(worktree)) else { return };

        if head == base || lock(&headless.changes_sent).get(&run_id) == Some(&head) {
            return;
        }

        let sent = git::committed(repo, base, &head)
            .map(|committed| RunChanges::of(base, &head, committed))
            .and_then(|changes| self.seen(self.client()?.put_changes(run_id, &changes)));

        // A server that could not be reached is asked again on the next round. One that
        // refused (the run is over) would refuse this head again, so it is not asked twice.
        if !sent.is_err_and(|error| error.is_offline()) {
            lock(&headless.changes_sent).insert(run_id, head);
        }
    }

    fn push_is_undecided(&self, thread_id: i64) -> bool {
        self.store
            .proposals(thread_id)
            .unwrap_or_default()
            .iter()
            .any(|proposal| proposal.kind == ProposalKind::Push && proposal.state == ProposalState::Pending)
    }

    /// Carries out the pushes and pull requests of this thread that a person approved on the
    /// web: `approved` with no result means "approved, the runner is carrying it out". Pushes
    /// go first, and a pull request waits while a push of the thread is still undecided,
    /// because there is nothing to open it from before the branch is on the server.
    fn carry_out_approved(&self, thread: &Thread, proposals: &[RunProposal]) {
        if self.headless.is_none() {
            return;
        }

        let waits = |remote: &&RunProposal| {
            remote.state == RunProposalState::Approved && remote.result.is_null() && matches!(remote.kind.as_str(), "push" | "pull_request")
        };
        let mut approved: Vec<&RunProposal> = proposals.iter().filter(waits).collect();

        approved.sort_by_key(|remote| remote.kind != "push");

        for remote in approved {
            let Ok(Some(local)) = self.store.proposal_by_remote(remote.id) else { continue };
            let outcome = match local.state {
                ProposalState::Pending if local.kind == ProposalKind::PullRequest && self.push_is_undecided(thread.id) => continue,
                ProposalState::Pending => {
                    let done = match local.kind {
                        ProposalKind::Push => self.push_approved(thread, &local, remote),
                        _ => self.pull_request_approved(thread, remote),
                    };
                    let outcome = done.map_err(|error| error.to_string());
                    let _ = match &outcome {
                        Ok(result) => self.store.decide_proposal(local.id, ProposalState::Approved, None, result),
                        Err(reason) => self.store.decide_proposal(local.id, ProposalState::Failed, Some(reason), &Value::Null),
                    };

                    self.emit(AppEvent::ThreadChanged { thread_id: thread.id });
                    outcome
                }
                // Carried out on an earlier round, and the server did not hear how it went.
                ProposalState::Approved => Ok(local.result.clone()),
                ProposalState::Failed | ProposalState::Rejected => Err(local.note.clone().unwrap_or_else(|| "It was not carried out.".to_string())),
            };

            self.report_outcome(remote.id, outcome);
        }
    }

    /// Tells the server how an approved push or pull request went. An answer that cannot be
    /// delivered now is kept and sent on a later round.
    fn report_outcome(&self, remote_id: i64, outcome: Result<Value, String>) {
        let Some(headless) = &self.headless else { return };
        let sent = self.client().and_then(|client| self.seen(client.report_proposal(remote_id, &outcome)));
        let mut unreported = lock(&headless.unreported);

        unreported.retain(|(id, _)| *id != remote_id);

        if sent.is_err_and(|error| error.is_offline()) {
            unreported.push((remote_id, outcome));
        }
    }

    fn report_unreported(&self) {
        let waiting = self.headless.as_ref().map(|headless| lock(&headless.unreported).clone()).unwrap_or_default();

        for (remote_id, outcome) in waiting {
            self.report_outcome(remote_id, outcome);
        }
    }

    /// Pushes the commit a person approved on the web: exactly the one the proposal showed,
    /// never a later one, never forced. What the desktop app would ask a second confirmation
    /// for (CI and build files, something like a secret, a binary) is not pushed from here
    /// at all: nobody is at this machine to give it.
    fn push_approved(&self, thread: &Thread, local: &Proposal, remote: &RunProposal) -> CoreResult<Value> {
        let shown = remote.payload["commit"].as_str().unwrap_or_default();

        if shown.is_empty() || local.payload["commit"].as_str() != Some(shown) {
            return Err(CoreError::not_possible("The approved proposal does not name the commit this runner showed, so nothing was pushed."));
        }

        let token = thread.run_id.map(|run_id| self.github_token_of(run_id)).transpose()?.flatten();

        // The server's branch as it is now, so that what is read and judged is what would be sent.
        self.clones()?.fetch(Path::new(&thread.repo_path), token.as_ref())?;

        let (read, branch) = self.push_read(thread, Some(shown))?;

        if let Some(blocked) = read.blocked {
            return Err(CoreError::not_possible(blocked));
        }

        let findings = push_guard::review(&read.readings);

        if !findings.is_empty() {
            let listed = findings.iter().map(|found| found.detail.clone()).collect::<Vec<_>>().join("; ");

            return Err(CoreError::not_possible(format!(
                "Not pushed. This needs a second look that a runner nobody sits at cannot give: {listed}. The diff is on the run; apply it by hand if it is wanted."
            )));
        }

        self.clones()?.push(Path::new(&thread.repo_path), &branch, &read.commit, token.as_ref())?;
        self.say_pushed(thread.id, &branch, &read)?;

        Ok(json!({ "commit": read.commit }))
    }

    /// Opens the pull request a person approved on the web, through the server and with the
    /// server's GitHub token: a headless runner has no `gh` and no browser to fall back on.
    fn pull_request_approved(&self, thread: &Thread, remote: &RunProposal) -> CoreResult<Value> {
        let branch = thread.branch.clone().ok_or_else(|| CoreError::not_possible("The thread has no branch yet"))?;
        let repo_path = Path::new(&thread.repo_path);

        if !git::on_remote(repo_path, &branch) {
            return Err(CoreError::not_possible(format!("Branch {branch} is not on GitHub: its push has to be approved first.")));
        }

        let code = self.code_on_server(thread);

        if let Some(url) = thread.pull_request_url.clone().or_else(|| Self::known_pull_request(code.as_ref(), thread)) {
            self.store.set_pull_request(thread.id, &url)?;

            return Ok(json!({ "url": url }));
        }

        let repo = self.store.repo(thread.repo_id).ok().and_then(|mapping| mapping.github_repo);
        let repo_id = Self::repo_on_server(code.as_ref(), repo.as_deref()).ok_or_else(|| {
            CoreError::not_possible(format!(
                "The server cannot open the pull request: {} is not connected to project {} there (Code → Repositories).",
                repo.as_deref().unwrap_or("the repository"),
                thread.project_key
            ))
        })?;
        let text = |field: &str| remote.payload[field].as_str().map(str::trim).filter(|text| !text.is_empty());
        let title = text("title").map(String::from).unwrap_or_else(|| format!("{} {}", thread.issue_key, thread.issue_summary));
        let base = text("base").map(String::from).unwrap_or_else(|| Self::default_branch_name(repo_path));
        let new = json!({ "repoId": repo_id, "head": branch, "base": base, "title": title, "body": text("body").unwrap_or_default() });
        let opened = self.seen(self.client()?.open_pull_request(&thread.issue_key, &new))?;
        let url = opened["url"].as_str().ok_or_else(|| CoreError::not_possible("The server opened the pull request but did not say where it is."))?;

        self.store.set_pull_request(thread.id, url)?;

        let event = self.store.add_event(thread.id, EventKind::Status, &format!("Pull request opened: {url}"), &json!({ "url": url }), false)?;

        self.emit(AppEvent::ThreadEvent { thread_id: thread.id, event });

        Ok(json!({ "url": url }))
    }

    /// After a restart: threads that were working when the runner stopped go on where they
    /// were, in the same session, when the server says their runs are still this runner's.
    /// A run that went back to the queue or to another runner meanwhile is let go, and one
    /// that was cancelled is told so by the ordinary round. Runs this runner holds without
    /// a thread (it stopped while cloning for them) are taken up again.
    fn continue_interrupted(&self, client: &HubClient) {
        let Some(headless) = &self.headless else { return };
        let runner_id = self.settings().runner_id;
        let waiting = lock(&headless.interrupted).clone();
        let settled = |thread_id: i64| lock(&headless.interrupted).retain(|id| *id != thread_id);

        for thread_id in waiting {
            let run_id = self.store.thread(thread_id).ok().and_then(|thread| thread.run_id);
            let detail = match run_id.map(|run_id| self.seen(client.run(run_id))) {
                Some(Err(error)) if error.is_offline() => continue,
                Some(Ok(detail)) => Some(detail),
                _ => None,
            };
            let still_ours = detail.as_ref().is_some_and(|detail| {
                detail.run.runner_id.as_deref() == Some(runner_id.as_str()) && !detail.run.state.is_finished() && detail.run.state != RunState::Queued
            });

            match detail {
                // The keys the run came with went with the process that held them.
                Some(detail) if still_ours && !detail.run.cancel_requested && headless.shared => {
                    let _ = self.store.add_event(thread_id, EventKind::Error, LOST_KEYS, &Value::Null, false);
                    let _ = self.store.move_thread(thread_id, &[ThreadState::Stopped], ThreadState::Failed);

                    self.end_failed(client, detail.run.id, LOST_KEYS.to_string());
                }
                Some(detail) if still_ours && !detail.run.cancel_requested => {
                    let _ = self.runtime().resume(thread_id, None, true);
                }
                Some(_) if still_ours => {}
                _ => {
                    let _ = self.store.set_run(thread_id, None);
                    let _ = self.store.add_event(
                        thread_id,
                        EventKind::Status,
                        "The run of this thread went on without this runner while it was stopped; the thread stays stopped.",
                        &Value::Null,
                        true,
                    );
                }
            }

            settled(thread_id);
        }

        if !headless.orphans_adopted.load(Ordering::SeqCst) {
            let Ok(runs) = self.seen(client.runs(false, 100)) else { return };

            headless.orphans_adopted.store(true, Ordering::SeqCst);

            for run in runs.iter().filter(|run| run.runner_id.as_deref() == Some(runner_id.as_str()) && run.state == RunState::Claimed) {
                let known = self.store.thread_of_run(run.id).ok().flatten().is_some() || lock(&headless.preparing).contains(&run.id);

                match (known, headless.shared, client.run(run.id)) {
                    (true, _, _) | (_, _, Err(_)) => {}
                    (false, true, Ok(_)) => self.end_failed(client, run.id, LOST_KEYS.to_string()),
                    (false, false, Ok(detail)) => {
                        if let Err(error) = self.adopt(client, &detail) {
                            self.refuse_run(client, run.id, &error);
                        }
                    }
                }
            }
        }
    }
}

fn url_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The core with a thread for every call: what the window, the command line and tests hold.
#[derive(Clone)]
pub struct Desktop {
    core: Arc<Core>,
    poll_every: Duration,
    job_options: Arc<JobOptions>,
    jobs_stop: Arc<watch::Sender<bool>>,
}

impl Desktop {
    /// Opens the local state and prepares the runtime. Must be called inside a Tokio runtime.
    pub fn open(options: OpenOptions) -> CoreResult<Self> {
        options.paths.create()?;

        let store = Arc::new(Store::open(&options.paths.database())?);
        let saved = config::load_settings(&options.paths)?;
        let settings = match &options.headless {
            Some(headless) => {
                let settings = headless.settings(saved);

                headless.folders.create()?;
                config::private_to_owner(&options.paths.worktrees);
                config::save_settings(&options.paths, &settings)?;
                settings
            }
            None => saved,
        };
        let cache_key = CacheKey::load_or_create(options.keychain.as_ref())?;
        let (events, _) = broadcast::channel(1024);
        // Where nobody is there to notice a shell that fails to start, the sandbox is tried, not assumed.
        let detect = if options.headless.is_some() { policy::sandbox_probe } else { policy::sandbox_state };
        let sandbox = options.sandbox.unwrap_or_else(detect);
        let from_env = std::env::var(config::SIDECAR_VARIABLE)
            .ok()
            .map(|line| line.split_whitespace().map(String::from).collect::<Vec<_>>())
            .filter(|parts| !parts.is_empty());
        let (program, arguments) = match (options.helper, from_env) {
            (Some(helper), _) => helper,
            (None, Some(parts)) => (parts[0].clone(), parts[1..].to_vec()),
            (None, None) => default_helper(&settings.node_command, options.sidecar_script.as_deref()),
        };
        let mut private_paths = policy::private_paths(&options.paths.state);
        let unattended = options.headless.as_ref().map(|headless| {
            let folders = &headless.folders;

            private_paths.extend([folders.jobs.clone(), folders.browsers.clone(), folders.helper_home.clone()]);

            Unattended {
                helper_home: folders.helper_home.clone(),
                model_key: headless.model_key.clone(),
                shared_roots: vec![(folders.repos.clone(), 2), (options.paths.worktrees.clone(), 2)],
            }
        });
        let runtime_config = RuntimeConfig {
            program,
            arguments,
            max_running: settings.max_running as usize,
            max_turns: settings.max_turns,
            stop_grace: options.stop_grace,
            worktrees_root: options.paths.worktrees.clone(),
            private_paths,
            sandbox,
            allow_unsandboxed_shell: settings.allow_unsandboxed_shell,
            env: BTreeMap::new(),
            unattended,
        };
        let is_headless = options.headless.is_some();
        let job_options = JobOptions::for_desktop(&options.paths, sandbox);
        let headless = options.headless.map(|headless| HeadlessState {
            shared: headless.shared,
            keys: Mutex::new(HashMap::new()),
            kinds: headless.kinds,
            projects: headless.projects,
            repos: Repos::new(headless.folders.repos, &headless.git_base, headless.github_token, headless.shared),
            preparing: Mutex::new(HashSet::new()),
            changes_sent: Mutex::new(HashMap::new()),
            unreported: Mutex::new(Vec::new()),
            interrupted: Mutex::new(Vec::new()),
            orphans_adopted: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
        });
        let core = Arc::new_cyclic(|this| Core {
            this: this.clone(),
            headless,
            jobs_wake: Arc::new(Notify::new()),
            job_clones: Repos::of_the_person(options.paths.root().join("repos")),
            paths: options.paths,
            store: Arc::clone(&store),
            keychain: options.keychain,
            cache_key,
            settings: RwLock::new(settings),
            session: RwLock::new(Session::default()),
            runner: Mutex::new(RunnerState::default()),
            tools: Mutex::new(None),
            events: events.clone(),
            runtime: OnceLock::new(),
            sandbox,
            // A headless runner opens pull requests through the server and never with `gh`.
            github_cli: !is_headless && options.github_cli.unwrap_or_else(Core::gh_installed),
            seen_notification: AtomicI64::new(-1),
            wake: Arc::new(Notify::new()),
            closing: Arc::new(AtomicBool::new(false)),
            pairing: Mutex::new(None),
        });
        let runtime = Runtime::new(Arc::clone(&store), runtime_config, Arc::new(Bridge(Arc::downgrade(&core))), events);

        runtime.recover()?;

        if let Some(headless) = &core.headless {
            // Stopped without the server having been told: the runner stopped under them.
            let told = |thread: &Thread| {
                let sent = store.thread_private(thread.id).map(|private| private.run_sent).unwrap_or_default();

                serde_json::from_value::<RunPatch>(sent).is_ok_and(|sent| sent.state.is_some_and(RunState::is_finished))
            };
            let interrupted = store.threads_in(ThreadState::Stopped)?.into_iter().filter(|thread| thread.run_id.is_some() && !told(thread));

            *lock(&headless.interrupted) = interrupted.map(|thread| thread.id).collect();
        }

        let _ = core.runtime.set(runtime);

        let (jobs_stop, _) = watch::channel(false);

        Ok(Self { core, poll_every: options.poll_every, job_options: Arc::new(job_options), jobs_stop: Arc::new(jobs_stop) })
    }

    /// What carries out test jobs on this computer (docs/runners.md): the executor of the
    /// headless runner, as the person's own user, confined where the system can do that.
    pub fn job_runner(&self) -> JobRunner {
        JobRunner::new(Arc::clone(&self.core), (*self.job_options).clone())
    }

    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.core.events.subscribe()
    }

    /// Runs a piece of the core on a thread that may block.
    pub async fn run<T: Send + 'static>(&self, work: impl FnOnce(&Core) -> CoreResult<T> + Send + 'static) -> CoreResult<T> {
        let core = Arc::clone(&self.core);

        tokio::task::spawn_blocking(move || work(&core)).await?
    }

    pub async fn stop_thread(&self, thread_id: i64) -> CoreResult<Thread> {
        let thread = self.core.runtime().stop(thread_id).await?;

        self.core.wake.notify_one();

        Ok(thread)
    }

    pub async fn remove_worktree(&self, thread_id: i64) -> CoreResult<Thread> {
        self.core.runtime().remove_worktree(thread_id).await
    }

    pub fn helper_pid(&self, thread_id: i64) -> Option<u32> {
        self.core.runtime().helper_pid(thread_id)
    }

    /// Forgets a finished thread whose working tree is already removed.
    pub async fn forget_thread(&self, thread_id: i64) -> CoreResult<()> {
        let thread = self.core.store.thread(thread_id)?;

        if thread.state.is_active() || thread.worktree.is_some() {
            return Err(CoreError::not_possible("Stop the thread and remove its working tree first"));
        }

        self.core.store.delete_thread(thread_id)
    }

    /// One round of the runner, with the stops it asks for carried out.
    pub async fn tick(&self, full: bool) -> CoreResult<()> {
        let to_stop = self.run(move |core| Ok(core.tick(full))).await?;

        for thread_id in to_stop {
            self.core.runtime().stop(thread_id).await?;
        }

        Ok(())
    }

    pub async fn wait_idle(&self, limit: Duration) -> bool {
        self.core.runtime().wait_idle(limit).await
    }

    /// Starts what runs for as long as the app does: the runner's rounds and the live stream.
    pub fn start_background(&self) {
        let desktop = self.clone();
        let activity = self.core.runtime().activity();

        // The desktop app is a runner for test jobs too; a headless runner starts its own
        // loop, with the user and the folders its environment names.
        if !self.core.is_headless() {
            let (runner, stopped) = (self.job_runner(), self.jobs_stop.subscribe());

            tokio::spawn(async move { runner.run(stopped).await });
        }

        tokio::spawn(async move {
            let mut last_full = Instant::now() - desktop.poll_every;

            while !desktop.core.closing.load(Ordering::SeqCst) {
                let woken = tokio::select! {
                    () = activity.notified() => false,
                    () = desktop.core.wake.notified() => true,
                    () = tokio::time::sleep(desktop.poll_every) => true,
                };
                let full = woken || last_full.elapsed() >= desktop.poll_every;

                if full {
                    last_full = Instant::now();
                }

                let _ = desktop.tick(full).await;
            }
        });

        let core = Arc::clone(&self.core);

        std::thread::spawn(move || {
            let mut pause = Duration::from_secs(2);

            while !core.closing.load(Ordering::SeqCst) {
                let outcome = core.client().and_then(|client| client.stream(&core.closing, |event| core.on_live(&event)));

                // A stream that ended cleanly reconnects soon; a failing one backs off.
                pause = if outcome.is_ok() { Duration::from_secs(2) } else { (pause * 2).min(Duration::from_secs(30)) };
                std::thread::sleep(pause);
            }
        });
    }

    /// Ends the background work. Threads that are working keep working until `shutdown`.
    pub fn close(&self) {
        self.core.closing.store(true, Ordering::SeqCst);
        self.core.wake.notify_one();
        self.jobs_stop.send_replace(true);
    }

    /// For a headless runner that is told to stop (a redeploy, a restart): every helper ends
    /// at once, and the server is told nothing, so the runs stay this runner's. When it comes
    /// back it continues them (`continue_interrupted`); if it never does, the server gives
    /// them to another runner by itself.
    pub async fn suspend(&self) {
        if let Some(headless) = &self.core.headless {
            headless.suspended.store(true, Ordering::SeqCst);
        }

        self.close();

        let mut stops = tokio::task::JoinSet::new();

        for state in [ThreadState::Queued, ThreadState::Running] {
            for thread in self.core.store.threads_in(state).unwrap_or_default() {
                let runtime = self.core.runtime().clone();

                stops.spawn(async move { runtime.stop(thread.id).await.map(|_| ()) });
            }
        }

        while stops.join_next().await.is_some() {}
    }

    /// For when the app quits: stops every working thread, so no agent runs on unseen, and
    /// tells the server.
    pub async fn shutdown(&self) {
        self.close();

        for state in [ThreadState::Queued, ThreadState::Running] {
            for thread in self.core.store.threads_in(state).unwrap_or_default() {
                let _ = self.core.runtime().stop(thread.id).await;
            }
        }

        let _ = self.tick(false).await;
    }
}
