//! Test jobs (docs/e2e-jobs.md): a headless runner checks a repository out, runs its e2e
//! suite with recording on, and sends the report and the files it names back.
//!
//! A job runs the repository's own code, so it is kept apart from the runner: a folder of
//! its own (not a thread's working tree, and with a repository that holds one commit and no
//! way back to the runner's clone), an environment made from nothing, a process group of its
//! own, and, when the runner is root, another user who can read neither the runner's state
//! nor its clones. One job runs at a time.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::api::HubClient;
use crate::app::Core;
use crate::config::Paths;
use crate::error::{CoreError, CoreResult};
use crate::git;
use crate::headless::Folders;
use crate::model::SandboxState;
use crate::runner::say;
use crate::runs::WorkKeys;
use crate::secrets::Secret;
use crate::threads::kill_group;

/// The server cuts a log line here; the runner does not send more.
const MAX_LINE: usize = 2000;
/// More lines than this waiting to be sent means the suite prints faster than anyone reads.
const MAX_WAITING_LINES: usize = 1000;
const MAX_ARTIFACT_BYTES: u64 = 200 * 1024 * 1024;
const MAX_RUN_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_REPORT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TIMEOUT_MINUTES: u64 = 240;
const DEFAULT_OUTPUT: &str = ".e2e";
const REPORT_FILE: &str = "report.json";
const TRACE_PAGE: &str = "trace.md";
/// Set by a suite that brings its own browser: nothing is downloaded then.
const OWN_BROWSER_VARIABLE: &str = "CHROMIUM_EXECUTABLE";
const BROWSERS_VARIABLE: &str = "PLAYWRIGHT_BROWSERS_PATH";

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("valid regex"));

/// `E2eJob`, as far as a runner reads it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestJob {
    pub id: i64,
    pub project_key: String,
    #[serde(default)]
    pub state: String,
    pub repo: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub video: String,
    #[serde(default)]
    pub trace: String,
    #[serde(default)]
    pub cancel_requested: bool,
}

fn default_install() -> String {
    "npm ci".to_string()
}

fn default_timeout() -> u64 {
    30
}

impl TestJob {
    /// A person asked for the job to stop. A job that is held is only marked; the answer to
    /// the runner's next write says so, by the mark or by the state it has then.
    fn is_called_off(&self) -> bool {
        self.cancel_requested || self.state == "cancelled"
    }
}

/// A write to a job that has already ended is answered 409: it is over, whoever ended it.
fn is_over(error: &CoreError) -> bool {
    matches!(error, CoreError::Api { status: 409, .. })
}

/// `E2eSettings` with the values of the secrets, as the claim hands them out.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSettings {
    #[serde(default)]
    pub directory: String,
    #[serde(default = "default_install")]
    pub install_command: String,
    #[serde(default)]
    pub run_args: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    #[serde(default = "default_timeout")]
    pub timeout_minutes: u64,
}

impl std::fmt::Debug for JobSettings {
    /// Secrets are shown by name only.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JobSettings")
            .field("directory", &self.directory)
            .field("install_command", &self.install_command)
            .field("run_args", &self.run_args)
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("secrets", &self.secrets.keys().collect::<Vec<_>>())
            .field("timeout_minutes", &self.timeout_minutes)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ClaimedJob {
    pub job: TestJob,
    pub settings: JobSettings,
    /// For a shared runner: the keys of the person the job runs for.
    #[serde(default)]
    pub credentials: Option<WorkKeys>,
}

/// `PATCH /jobs/{id}`: only what is set is sent.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct JobPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

/// Who a job's processes run as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobUser {
    /// The runner's own user (a developer's machine, a test): a job is kept apart by its
    /// environment and its folder only.
    Same,
    /// Another, unprivileged user, because the runner is root.
    Other { name: String, uid: u32, gid: u32 },
    /// The runner is root and the user named for jobs does not exist: no job is run, rather
    /// than run a repository's code as root.
    Missing { name: String },
}

impl JobUser {
    /// Root runs jobs as the user called `name`; anyone else runs them as themselves. Naming
    /// root itself is saying that no separation is wanted.
    #[cfg(unix)]
    pub fn for_runner(name: &str) -> Self {
        // SAFETY: `geteuid` only reads; `getpwnam` returns a record owned by libc that is
        // copied before anything else can ask for another.
        unsafe {
            if libc::geteuid() != 0 {
                return Self::Same;
            }

            let Ok(wanted) = std::ffi::CString::new(name) else { return Self::Missing { name: name.to_string() } };
            let record = libc::getpwnam(wanted.as_ptr());

            match record.as_ref() {
                None => Self::Missing { name: name.to_string() },
                Some(found) if found.pw_uid == 0 => Self::Same,
                Some(found) => Self::Other { name: name.to_string(), uid: found.pw_uid, gid: found.pw_gid },
            }
        }
    }

    #[cfg(not(unix))]
    pub fn for_runner(_name: &str) -> Self {
        Self::Same
    }

    /// One line for the runner's log.
    pub fn describe(&self) -> String {
        match self {
            Self::Same => "Test jobs run as the runner's own user: they are kept apart by their environment and folder only.".to_string(),
            Self::Other { name, uid, .. } => format!("Test jobs run as user {name} (uid {uid}), who cannot read the runner's state or clones."),
            Self::Missing { name } => {
                format!("This runner is root and there is no user \"{name}\" to run test jobs as: jobs will fail until there is one.")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobOptions {
    pub folders: Folders,
    pub user: JobUser,
    /// How often waiting log lines are sent.
    pub log_every: Duration,
    /// How often the server is asked whether the job was cancelled.
    pub look_every: Duration,
    /// How often the queue is looked at when nothing announces a job.
    pub idle_every: Duration,
    /// How long one of a job's `timeoutMinutes` lasts. A minute; a test makes it shorter.
    pub minute: Duration,
    /// Set on a person's computer where the system has a sandbox: the job's processes see
    /// the disk read-only, write only in the job's own folder and the browsers' folder, and
    /// are shown nothing of these folders (credentials, the app's own state).
    pub hidden: Option<Vec<PathBuf>>,
}

impl JobOptions {
    pub fn new(folders: Folders, user: JobUser) -> Self {
        Self {
            folders,
            user,
            log_every: Duration::from_secs(1),
            look_every: Duration::from_secs(3),
            idle_every: Duration::from_secs(10),
            minute: Duration::from_secs(60),
            hidden: None,
        }
    }

    /// For the desktop app: beside its other data, as the person's own user (there is no
    /// other to switch to), inside the same kind of sandbox as agent shells where there is one.
    pub fn for_desktop(paths: &Paths, sandbox: SandboxState) -> Self {
        let hidden = (cfg!(target_os = "linux") && sandbox == SandboxState::Available).then(|| crate::policy::private_paths(&paths.state));

        Self { hidden, ..Self::new(Folders::under(&paths.root()), JobUser::Same) }
    }
}

/// Why a job ended without a run.
enum Failure {
    Cancelled,
    RunnerStopped,
    Because(String),
}

impl Failure {
    fn because(reason: impl Into<String>) -> Self {
        Self::Because(reason.into())
    }

    fn text(&self) -> String {
        match self {
            Self::Cancelled => "Cancelled.".to_string(),
            Self::RunnerStopped => "The runner was stopped while this job ran (a restart or a redeploy). Start the job again.".to_string(),
            Self::Because(reason) => reason.clone(),
        }
    }
}

/// One job being carried out: what its processes get, and the log lines not sent yet.
struct Session {
    id: i64,
    step: &'static str,
    env: BTreeMap<String, String>,
    /// Values that must not show in the log.
    masked: Vec<String>,
    deadline: Instant,
    timeout_minutes: u64,
    waiting: Vec<String>,
    /// The process groups this job started, for what they leave behind.
    groups: Vec<u32>,
}

impl Session {
    fn push(&mut self, line: &str) {
        let mut text = ANSI.replace_all(line, "").into_owned();

        for secret in &self.masked {
            text = text.replace(secret.as_str(), "***");
        }

        if text.chars().count() > MAX_LINE {
            text = text.chars().take(MAX_LINE).collect();
        }

        self.waiting.push(text);

        if self.waiting.len() > MAX_WAITING_LINES {
            let skipped = self.waiting.len() - MAX_WAITING_LINES / 2;

            self.waiting.drain(..skipped);
            self.waiting.insert(0, format!("[runner] {skipped} lines were left out: the suite prints faster than the log is sent"));
        }
    }
}

/// `--workers 2 "a b"` → [`--workers`, `2`, `a b`]: words as a shell would split them, with
/// quotes, and nothing a shell would run.
pub fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut characters = text.chars();

    while let Some(character) = characters.next() {
        match (character, quote) {
            ('\\', None | Some('"')) => {
                word.extend(characters.next());
                started = true;
            }
            ('\'' | '"', None) => {
                quote = Some(character);
                started = true;
            }
            (closing, Some(open)) if closing == open => quote = None,
            (space, None) if space.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (other, _) => {
                word.push(other);
                started = true;
            }
        }
    }

    if started {
        words.push(word);
    }

    words
}

/// Where the suite writes: `--output` among the run's arguments, else the framework's default.
fn output_folder(run_arguments: &[String]) -> String {
    let named = run_arguments.iter().enumerate().find_map(|(index, argument)| match argument.strip_prefix("--output") {
        Some("") => run_arguments.get(index + 1).cloned(),
        Some(rest) => rest.strip_prefix('=').map(String::from),
        None => None,
    });

    named.filter(|folder| !folder.is_empty()).unwrap_or_else(|| DEFAULT_OUTPUT.to_string())
}

/// A path that stays below the folder it is joined to: no root, no `..`.
fn is_inside_path(path: &str) -> bool {
    !path.is_empty() && Path::new(path).components().all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

/// The files a report names: every artifact record that has a path (one whose file never
/// appeared has none, and a recording kept by a hosted service has an address instead).
/// The paths are the report's own, relative to `<output>/results`.
pub fn named_artifacts(report: &Value) -> Vec<(String, String)> {
    fn walk(value: &Value, found: &mut Vec<(String, String)>) {
        match value {
            Value::Object(fields) => {
                if let (Some(path), Some(media_type)) = (fields.get("path").and_then(Value::as_str), fields.get("mediaType").and_then(Value::as_str))
                {
                    if !found.iter().any(|(known, _)| known == path) {
                        found.push((path.to_string(), media_type.to_string()));
                    }
                }

                fields.values().for_each(|field| walk(field, found));
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, found)),
            _ => {}
        }
    }

    let mut found = Vec::new();

    walk(report, &mut found);
    found
}

fn media_type_of(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_lowercase().as_str() {
        "webm" => "video/webm",
        "mp4" => "video/mp4",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// The trace pages of a run: `results/<test>/trace.md`. The report does not name them (the
/// framework hands them to its reporters beside the report), so they are found on disk and
/// sent under the same kind of path its artifacts have.
fn trace_pages(results: &Path) -> Vec<String> {
    let mut pages: Vec<String> = std::fs::read_dir(results)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().join(TRACE_PAGE).is_file())
                .map(|entry| format!("{}/{TRACE_PAGE}", entry.file_name().to_string_lossy()))
                .collect()
        })
        .unwrap_or_default();

    pages.sort();
    pages
}

/// Opens a file a report names, when it really is a file below `results`: a link that leads
/// out of the job's folder is not followed into the runner's own files.
fn open_artifact(results: &Path, path: &str) -> Option<(std::fs::File, u64)> {
    if !is_inside_path(path) {
        return None;
    }

    let root = std::fs::canonicalize(results).ok()?;
    let place = std::fs::canonicalize(root.join(path)).ok()?;
    let meta = std::fs::metadata(&place).ok()?;

    (place.starts_with(&root) && meta.is_file()).then(|| std::fs::File::open(&place).ok()).flatten().map(|file| (file, meta.len()))
}

#[cfg(unix)]
fn give_to(folder: &Path, uid: u32, gid: u32) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let name = std::ffi::CString::new(folder.as_os_str().as_bytes())?;

    // SAFETY: `name` is a NUL-terminated path that lives through the call; a link is given
    // away itself, never what it points at.
    if unsafe { libc::lchown(name.as_ptr(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    if std::fs::symlink_metadata(folder)?.is_dir() {
        for entry in std::fs::read_dir(folder)? {
            give_to(&entry?.path(), uid, gid)?;
        }
    }

    Ok(())
}

/// Looks at what this process is the parent of and takes away the dead ones a job left: as
/// the first process of a container the runner inherits every orphan, and nobody else would.
/// Only processes of the job's own groups, or of the job's own user, are touched, so no
/// child the runner itself is waiting for is taken from it.
#[cfg(target_os = "linux")]
fn reap_leftovers(groups: &[u32], user: &JobUser) {
    use std::os::unix::fs::MetadataExt;

    let me = std::process::id();

    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<i32>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        // "pid (name) state ppid pgrp …": the name can hold anything, so read after its end.
        let Some(fields) = stat.rsplit_once(')').map(|(_, rest)| rest.split_whitespace().collect::<Vec<_>>()) else { continue };
        let [state, parent, group, ..] = fields[..] else { continue };
        let of_job = match user {
            JobUser::Other { uid, .. } => entry.metadata().is_ok_and(|meta| meta.uid() == *uid),
            _ => group.parse::<u32>().is_ok_and(|group| groups.contains(&group)),
        };

        if state == "Z" && parent.parse::<u32>() == Ok(me) && of_job {
            // SAFETY: waits, without blocking, for one process this process is the parent of.
            unsafe {
                libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn reap_leftovers(_groups: &[u32], _user: &JobUser) {}

pub struct JobRunner {
    core: Arc<Core>,
    options: JobOptions,
}

impl JobRunner {
    /// Folders of jobs an earlier run of the runner left are removed: no job survives it.
    pub fn new(core: Arc<Core>, options: JobOptions) -> Self {
        let _ = std::fs::create_dir_all(&options.folders.jobs);
        let _ = std::fs::create_dir_all(&options.folders.browsers);

        for entry in std::fs::read_dir(&options.folders.jobs).into_iter().flatten().flatten() {
            let _ = std::fs::remove_dir_all(entry.path());
        }

        Self { core, options }
    }

    async fn api<T: Send + 'static>(&self, call: impl FnOnce(&HubClient) -> CoreResult<T> + Send + 'static) -> CoreResult<T> {
        let core = Arc::clone(&self.core);

        tokio::task::spawn_blocking(move || call(core.client()?.as_ref())).await?
    }

    /// Carries out queued jobs, one at a time, until `stop` is set.
    pub async fn run(&self, mut stop: watch::Receiver<bool>) {
        let wake = self.core.jobs_wake();
        let mut last_problem = String::new();

        while !*stop.borrow() {
            let outcome = if self.core.takes_test_jobs() { self.run_next(&mut stop).await } else { Ok(false) };
            let pause = match outcome {
                Ok(true) => continue,
                Ok(false) => self.options.idle_every,
                // A server without test jobs (an older one, or the module is off) is asked rarely.
                Err(error) => {
                    let problem = if error.is_not_found() { "This server has no test jobs API.".to_string() } else { format!("Test jobs: {error}") };

                    if problem != last_problem {
                        say(&problem);
                        last_problem = problem;
                    }

                    // So is one that refuses this runner (a shared runner without an administrator's token).
                    if error.is_not_found() || matches!(error, CoreError::Api { status: 403, .. }) {
                        Duration::from_secs(60).max(self.options.idle_every)
                    } else {
                        self.options.idle_every
                    }
                }
            };

            tokio::select! {
                () = wake.notified() => {}
                () = tokio::time::sleep(pause) => {}
                _ = stop.changed() => {}
            }
        }
    }

    /// Takes the oldest queued job, if there is one, and carries it out to its end. Says
    /// whether there was one.
    pub async fn run_next(&self, stop: &mut watch::Receiver<bool>) -> CoreResult<bool> {
        let identity = self.core.job_identity()?;
        let claimed = self.api(move |client| client.claim_job(&identity)).await?;
        let Some(ClaimedJob { job, settings, credentials }) = claimed else { return Ok(false) };
        // Held for this job only, and dropped with it.
        let token = credentials.and_then(|keys| keys.github_token);
        let folder = self.options.folders.jobs.join(job.id.to_string());
        let mut session = Session {
            id: job.id,
            step: "cloning",
            env: BTreeMap::new(),
            masked: settings.secrets.values().filter(|value| value.len() >= 4).cloned().collect(),
            timeout_minutes: settings.timeout_minutes.clamp(1, MAX_TIMEOUT_MINUTES),
            deadline: Instant::now() + self.options.minute * settings.timeout_minutes.clamp(1, MAX_TIMEOUT_MINUTES) as u32,
            waiting: Vec::new(),
            groups: Vec::new(),
        };

        say(&format!("Test job {} for {}: {} at {}", job.id, job.project_key, job.repo, job.reference));

        let outcome = self.carry_out(&job, &settings, token, &folder, &mut session, stop).await;
        let error = outcome.as_ref().err().map(Failure::text);

        if let Some(error) = &error {
            session.push(&format!("[runner] {error}"));
        }

        self.sweep().await;
        self.send_log(&mut session).await;

        let (id, reason) = (job.id, error.clone());
        let finished = self.api(move |client| client.finish_job(id, reason.as_deref())).await;

        say(&match (&error, finished) {
            (None, Ok(_)) => format!("Test job {id} is done; its report is on the server."),
            (Some(error), Ok(_)) => format!("Test job {id} failed: {error}"),
            (_, Err(unsent)) => format!("Test job {id} ended, but the server could not be told: {unsent}"),
        });

        let (groups, user) = (session.groups, self.options.user.clone());
        let _ = tokio::task::spawn_blocking(move || {
            let _ = std::fs::remove_dir_all(&folder);

            reap_leftovers(&groups, &user);
        })
        .await;

        Ok(true)
    }

    /// Tells the server which step the job is at; refuses to go on when it was cancelled.
    async fn step(&self, session: &mut Session, step: &'static str, starting: bool) -> Result<(), Failure> {
        session.step = step;

        let (id, patch) = (session.id, JobPatch { state: starting.then_some("running"), step: Some(step), commit: None });
        match self.api(move |client| client.patch_job(id, &patch)).await {
            Ok(job) if job.is_called_off() => Err(Failure::Cancelled),
            Ok(_) => Ok(()),
            Err(error) if is_over(&error) => Err(Failure::Cancelled),
            Err(error) => Err(Failure::because(format!("The server could not be told about the job: {error}"))),
        }
    }

    async fn send_log(&self, session: &mut Session) {
        if session.waiting.is_empty() {
            return;
        }

        let (id, lines) = (session.id, std::mem::take(&mut session.waiting));
        let kept = lines.clone();

        // Lines the server could not take now are tried again with the next ones; a job
        // that has ended takes none, and they are let go.
        if self.api(move |client| client.job_log(id, &lines)).await.is_err_and(|error| !is_over(&error)) {
            session.waiting = kept;
        }
    }

    /// Whether a person cancelled the job. A server that cannot be asked right now has not said so.
    async fn cancelled(&self, session: &Session) -> bool {
        let (id, patch) = (session.id, JobPatch { step: Some(session.step), ..Default::default() });

        match self.api(move |client| client.patch_job(id, &patch)).await {
            Ok(job) => job.is_called_off(),
            Err(error) => is_over(&error),
        }
    }

    async fn carry_out(
        &self,
        job: &TestJob,
        settings: &JobSettings,
        token: Option<Secret>,
        folder: &Path,
        session: &mut Session,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<(), Failure> {
        if let JobUser::Missing { name } = &self.options.user {
            return Err(Failure::because(format!(
                "This runner runs as root and has no user \"{name}\" to run a repository's code as. Create that user in its image, or name another with RUNNER_JOB_USER."
            )));
        }

        // 1. The repository at the commit, in a folder of the job's own.
        self.step(session, "cloning", true).await?;

        let (core, wanted, place) = (Arc::clone(&self.core), job.clone(), folder.to_path_buf());
        let source = folder.join("src");
        let home = folder.join("home");
        let checked_out = tokio::task::spawn_blocking(move || check_out(&core, &wanted, token.as_ref(), &place)).await;
        let commit = match checked_out {
            Ok(Ok(commit)) => commit,
            Ok(Err(error)) => return Err(Failure::because(format!("{} could not be checked out at {}: {error}", job.repo, job.reference))),
            Err(error) => return Err(Failure::because(format!("The checkout failed: {error}"))),
        };
        let suite = suite_folder(&source, &settings.directory).map_err(Failure::Because)?;

        std::fs::create_dir_all(&home).map_err(|error| Failure::because(format!("The job's folder could not be made: {error}")))?;
        self.hand_over(folder).map_err(|error| Failure::because(format!("The job's folder could not be given to the job's user: {error}")))?;
        session.push(&format!("[runner] {} at {commit}", job.repo));

        let (id, patch) = (job.id, JobPatch { commit: Some(commit), ..Default::default() });
        let _ = self.api(move |client| client.patch_job(id, &patch)).await;

        // What the suite's processes get: this, and nothing else of the runner's.
        session.env = self.environment(&home, settings);

        // 2. The suite's dependencies, then the browser its engine wants.
        self.step(session, "installing", false).await?;

        if !settings.install_command.trim().is_empty() {
            let installed = self.exec(session, stop, "sh", &["-c".to_string(), settings.install_command.clone()], &suite).await?;

            if installed != Some(0) {
                return Err(Failure::because(format!("The install command ended with {}: {}", exit_text(installed), settings.install_command)));
            }
        }

        if !session.env.contains_key(OWN_BROWSER_VARIABLE) {
            let installed = self.exec(session, stop, "npx", &owned(&["--no", "--", "e2e-web", "install", "chromium"]), &suite).await?;

            if installed != Some(0) {
                return Err(Failure::because(format!(
                    "The browser could not be installed (npx e2e-web install chromium ended with {}).",
                    exit_text(installed)
                )));
            }
        }

        // 3. The suite, recording as the job asks.
        self.step(session, "running", false).await?;

        let run_arguments = words(&settings.run_args);
        let output = output_folder(&run_arguments);

        if !is_inside_path(&output) {
            return Err(Failure::because(format!("The run arguments send the output to \"{output}\", which is not a folder inside the suite.")));
        }

        // A report that came with the repository must not pass for this run's.
        let report = suite.join(&output).join(REPORT_FILE);
        let _ = std::fs::remove_file(&report);
        let mut arguments = owned(&["--no", "--", "e2e", "run"]);

        arguments.extend(words(job.filter.as_deref().unwrap_or_default()));
        arguments.extend([format!("--video={}", mode(&job.video, "on")), format!("--trace={}", mode(&job.trace, "on"))]);
        arguments.extend(run_arguments);

        let ended = self.exec(session, stop, "npx", &arguments, &suite).await?;

        // 4. The report, then the files it names.
        self.step(session, "uploading", false).await?;

        let (core, wanted, results) = (Arc::clone(&self.core), job.clone(), suite.join(&output).join("results"));
        let shown = Path::new(&settings.directory).join(&output).join(REPORT_FILE).display().to_string();
        let sent = tokio::task::spawn_blocking(move || send_results(&core, &wanted, &report, &results, &shown, ended)).await;

        match sent {
            Ok(Ok(lines)) => lines.iter().for_each(|line| session.push(line)),
            Ok(Err(reason)) => return Err(Failure::Because(reason)),
            Err(error) => return Err(Failure::because(format!("Sending the results failed: {error}"))),
        }

        Ok(())
    }

    /// PATH, a home inside the job's folder, the browsers' folder, `CI=1`, and what the
    /// project's settings list. Nothing is inherited.
    fn environment(&self, home: &Path, settings: &JobSettings) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();

        env.insert("PATH".to_string(), std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string()));
        env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
        env.insert("CI".to_string(), "1".to_string());
        env.insert(BROWSERS_VARIABLE.to_string(), self.options.folders.browsers.to_string_lossy().into_owned());
        env.extend(settings.env.clone());
        env.extend(settings.secrets.clone());
        env
    }

    /// Gives the job's folder, and the browsers kept between jobs, to the job's user.
    #[cfg(unix)]
    fn hand_over(&self, folder: &Path) -> std::io::Result<()> {
        crate::config::private_to_owner(folder);

        if let JobUser::Other { uid, gid, .. } = &self.options.user {
            give_to(folder, *uid, *gid)?;
            give_to(&self.options.folders.browsers, *uid, *gid)?;
        }

        Ok(())
    }

    #[cfg(not(unix))]
    fn hand_over(&self, _folder: &Path) -> std::io::Result<()> {
        Ok(())
    }

    /// Ends whatever the job's user still runs, wherever it put itself: a process that left
    /// its group would otherwise be there for the next job. Only with a user of the job's own.
    async fn sweep(&self) {
        #[cfg(unix)]
        if let JobUser::Other { uid, gid, .. } = &self.options.user {
            let mut command = tokio::process::Command::new("sh");

            command.args(["-c", "kill -9 -1"]).env_clear().uid(*uid).gid(*gid).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());

            if let Ok(mut child) = command.spawn() {
                let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            }
        }
    }

    /// Runs one command of the job and sends what it prints to the job's log as it comes.
    /// Returns its exit code; ends it, with everything it started, when the job is cancelled,
    /// runs past its time, or the runner is told to stop.
    async fn exec(
        &self,
        session: &mut Session,
        stop: &mut watch::Receiver<bool>,
        program: &str,
        arguments: &[String],
        folder: &Path,
    ) -> Result<Option<i32>, Failure> {
        session.push(&format!("$ {program} {}", arguments.join(" ")));

        let job_folder = self.options.folders.jobs.join(session.id.to_string());
        let (program, arguments) = match &self.options.hidden {
            Some(hidden) => ("bwrap".to_string(), confined(&job_folder, &self.options.folders.browsers, hidden, program, arguments)),
            None => (program.to_string(), arguments.to_vec()),
        };
        let mut command = tokio::process::Command::new(&program);

        command
            .args(&arguments)
            .current_dir(folder)
            .env_clear()
            .envs(&session.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(unix)]
        {
            command.process_group(0);

            if let JobUser::Other { uid, gid, .. } = &self.options.user {
                command.uid(*uid).gid(*gid);
                // SAFETY: runs in the child between fork and exec and only makes one system
                // call: from here on no program it starts can gain rights (a setuid binary).
                unsafe {
                    command.pre_exec(|| match libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) {
                        0 => Ok(()),
                        _ => Err(std::io::Error::last_os_error()),
                    });
                }
            }
        }

        let program = program.as_str();
        let mut child = command.spawn().map_err(|error| {
            // The usual reason beside a missing program: the job's user cannot get to its folder.
            let hint = match (&self.options.user, error.kind()) {
                (JobUser::Other { name, .. }, std::io::ErrorKind::PermissionDenied) => {
                    format!(". User {name} has to be able to pass through every folder above {}", self.options.folders.jobs.display())
                }
                _ => String::new(),
            };

            Failure::because(format!("{program} could not be started: {error}{hint}"))
        })?;
        let group = child.id();
        let (lines, mut printed) = mpsc::unbounded_channel::<String>();

        session.groups.extend(group);

        if let Some(output) = child.stdout.take() {
            tokio::spawn(forward(output, lines.clone()));
        }

        if let Some(output) = child.stderr.take() {
            tokio::spawn(forward(output, lines.clone()));
        }

        drop(lines);

        let mut send = tokio::time::interval_at(Instant::now() + self.options.log_every, self.options.log_every);
        let mut look = tokio::time::interval_at(Instant::now() + self.options.look_every, self.options.look_every);
        let ended: Result<Option<i32>, Failure> = loop {
            tokio::select! {
                status = child.wait() => break Ok(status.ok().and_then(|status| status.code())),
                Some(line) = printed.recv() => session.push(&line),
                _ = send.tick() => self.send_log(session).await,
                _ = look.tick() => {
                    if self.cancelled(session).await {
                        break Err(Failure::Cancelled);
                    }
                }
                () = tokio::time::sleep_until(session.deadline) => {
                    break Err(Failure::because(format!("Stopped after {} minutes, the project's limit for a job.", session.timeout_minutes)));
                }
                _ = stop.changed() => break Err(Failure::RunnerStopped),
            }
        };

        // Whatever the command left running, which may also hold its output open.
        if let Some(group) = group {
            kill_group(group);
        }

        if ended.is_err() {
            let _ = child.wait().await;
        }

        while let Ok(Some(line)) = tokio::time::timeout(Duration::from_millis(300), printed.recv()).await {
            session.push(&line);
        }

        self.send_log(session).await;

        ended
    }
}

/// The command inside bubblewrap: the whole disk read-only, a `/tmp` of its own, the job's
/// folder and the browsers' folder writable, and the hidden folders empty. The network
/// stays: a suite has to reach the application it tests.
fn confined(job_folder: &Path, browsers: &Path, hidden: &[PathBuf], program: &str, arguments: &[String]) -> Vec<String> {
    let text = |path: &Path| path.to_string_lossy().into_owned();
    let mut wrapped = owned(&["--die-with-parent", "--unshare-pid", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"]);

    for folder in [job_folder, browsers] {
        wrapped.extend(["--bind".to_string(), text(folder), text(folder)]);
    }

    for path in hidden.iter().filter(|path| path.exists() && !job_folder.starts_with(path)) {
        match path.is_dir() {
            true => wrapped.extend(["--tmpfs".to_string(), text(path)]),
            false => wrapped.extend(["--ro-bind".to_string(), "/dev/null".to_string(), text(path)]),
        }
    }

    wrapped.extend(["--".to_string(), program.to_string()]);
    wrapped.extend(arguments.iter().cloned());
    wrapped
}

fn owned(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

fn exit_text(code: Option<i32>) -> String {
    code.map_or_else(|| "a signal".to_string(), |code| format!("exit code {code}"))
}

/// A recording mode as the job gives it; a job from a server that names none records.
fn mode<'a>(asked: &'a str, default: &'a str) -> &'a str {
    if asked.trim().is_empty() {
        default
    } else {
        asked.trim()
    }
}

/// Reads a command's output line by line. Bytes that are not text are replaced, not fatal.
async fn forward(output: impl AsyncRead + Unpin, lines: mpsc::UnboundedSender<String>) {
    let mut reader = BufReader::new(output);
    let mut line = Vec::new();

    while reader.read_until(b'\n', &mut line).await.is_ok_and(|read| read > 0) {
        let text = String::from_utf8_lossy(&line);

        // A progress line redrawn with carriage returns is shown as it ended.
        if lines.send(text.trim_end_matches(['\n', '\r']).rsplit('\r').next().unwrap_or_default().to_string()).is_err() {
            break;
        }

        line.clear();
    }
}

/// Clones or fetches the job's repository and checks its ref out under `folder/src`. The
/// commit that was checked out.
fn check_out(core: &Core, job: &TestJob, token: Option<&Secret>, folder: &Path) -> CoreResult<String> {
    let clone = core.clone_for_job(&job.project_key, &job.repo, token)?;
    let commit = git::resolve_commit(&clone, &job.reference)?;
    let asked_by_name = !commit.starts_with(job.reference.trim());

    let _ = std::fs::remove_dir_all(folder);
    git::checkout_copy(&clone, &commit, &folder.join("src"), asked_by_name.then_some(job.reference.trim()))?;

    Ok(commit)
}

/// Where the suite's `package.json` is: `directory` below the checkout, and really below it.
fn suite_folder(source: &Path, directory: &str) -> Result<PathBuf, String> {
    let directory = directory.trim().trim_matches('/');
    let missing = || format!("The repository has no folder \"{directory}\" (the suite's directory in the project's test settings).");

    if !directory.is_empty() && !is_inside_path(directory) {
        return Err(format!("\"{directory}\" is not a folder inside the repository."));
    }

    let root = std::fs::canonicalize(source).map_err(|_| missing())?;
    let suite = std::fs::canonicalize(root.join(directory)).map_err(|_| missing())?;

    if suite.starts_with(&root) && suite.is_dir() {
        Ok(suite)
    } else {
        Err(missing())
    }
}

/// Sends the report and every file it names that is on disk, then the trace pages. Returns
/// lines for the job's log; an error when there is no report to send.
fn send_results(core: &Core, job: &TestJob, report: &Path, results: &Path, shown: &str, ended: Option<i32>) -> Result<Vec<String>, String> {
    let no_report = || format!("The suite left no report at {shown} (e2e run ended with {}). Its output is in the log.", exit_text(ended));
    let meta = std::fs::symlink_metadata(report).map_err(|_| no_report())?;

    if !meta.is_file() || meta.len() > MAX_REPORT_BYTES {
        return Err(no_report());
    }

    let bytes = std::fs::read(report).map_err(|_| no_report())?;
    let named = serde_json::from_slice::<Value>(&bytes)
        .map(|report| named_artifacts(&report))
        .map_err(|error| format!("The report at {shown} is not JSON: {error}"))?;
    let client = core.client().map_err(|error| error.to_string())?;
    let run_id = client.send_report(&job.project_key, job.id, bytes).map_err(|error| format!("The server did not take the report: {error}"))?;
    let traces = trace_pages(results).into_iter().map(|path| (path, "text/markdown".to_string()));
    let mut lines = vec![format!("[runner] The report is sent (e2e run ended with {}).", exit_text(ended))];
    let (mut sent, mut total) = (0, 0u64);

    for (path, media_type) in named.into_iter().chain(traces) {
        let Some((file, size)) = open_artifact(results, &path) else {
            lines.push(format!("[runner] {path} is named by the report but is not a file in its results folder; left out"));
            continue;
        };

        if size > MAX_ARTIFACT_BYTES || total + size > MAX_RUN_BYTES {
            lines.push(format!("[runner] {path} is left out: {} MB is past what a run may hold", size / (1024 * 1024)));
            continue;
        }

        let media_type = if media_type.is_empty() { media_type_of(&path).to_string() } else { media_type };

        match client.send_artifact(run_id, &path, &media_type, file) {
            Ok(()) => {
                sent += 1;
                total += size;
            }
            Err(error) => lines.push(format!("[runner] {path} could not be sent: {error}")),
        }
    }

    lines.push(format!("[runner] {sent} files sent ({} KB).", total / 1024));

    Ok(lines)
}

impl From<Failure> for CoreError {
    fn from(failure: Failure) -> Self {
        CoreError::not_possible(failure.text())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn arguments_are_split_like_a_shell_would_and_nothing_is_run() {
        assert_eq!(words("--workers 2  tests/a.e2e.ts"), vec!["--workers", "2", "tests/a.e2e.ts"]);
        assert_eq!(words("--grep \"signs in\" --tag 'a b'"), vec!["--grep", "signs in", "--tag", "a b"]);
        assert_eq!(words("a\\ b \"\" $(rm -rf /); `x`"), vec!["a b", "", "$(rm", "-rf", "/);", "`x`"]);
        assert!(words("   ").is_empty());
    }

    #[test]
    fn the_output_folder_follows_the_run_arguments() {
        assert_eq!(output_folder(&words("--workers 2")), ".e2e");
        assert_eq!(output_folder(&words("--output out/e2e --workers 2")), "out/e2e");
        assert_eq!(output_folder(&words("--output=test-results")), "test-results");
        assert!(is_inside_path("out/e2e") && !is_inside_path("../out") && !is_inside_path("/tmp/out") && !is_inside_path(""));
    }

    #[test]
    fn a_report_names_its_files_by_path_and_the_ones_without_a_path_are_not_files() {
        let report = json!({ "run": { "results": [{ "attempts": [{
            "artifacts": [
                { "id": "a:0", "kind": "screenshot", "mediaType": "image/png", "path": "login-1a2b/attempt-1/screenshots/001-failure.png", "size": 10 },
                { "id": "a:1", "kind": "video", "mediaType": "video/webm", "startedAt": "2026-11-13T10:00:00Z" },
                { "id": "a:2", "kind": "video", "mediaType": "video/mp4", "url": "https://recordings.example/1" },
            ],
            "failure": { "screenshot": { "mediaType": "image/png", "path": "login-1a2b/attempt-1/screenshots/001-failure.png" } },
            "steps": [{ "source": { "file": "tests/a.e2e.ts", "line": 3 }, "artifacts": [{ "kind": "download", "mediaType": "text/csv", "path": "login-1a2b/attempt-1/downloads/export.csv" }] }],
        }] }] } });

        assert_eq!(
            named_artifacts(&report),
            vec![
                ("login-1a2b/attempt-1/screenshots/001-failure.png".to_string(), "image/png".to_string()),
                ("login-1a2b/attempt-1/downloads/export.csv".to_string(), "text/csv".to_string()),
            ]
        );
    }

    #[test]
    fn a_log_line_is_cut_cleaned_and_shows_no_secret() {
        let mut session = Session {
            id: 1,
            step: "running",
            env: BTreeMap::new(),
            masked: vec!["hunter2hunter2".to_string()],
            deadline: Instant::now(),
            timeout_minutes: 1,
            waiting: Vec::new(),
            groups: Vec::new(),
        };

        session.push("\u{1b}[32m✓\u{1b}[0m signed in with hunter2hunter2");
        session.push(&"ż".repeat(MAX_LINE + 50));

        assert_eq!(session.waiting[0], "✓ signed in with ***");
        assert_eq!(session.waiting[1].chars().count(), MAX_LINE);

        for _ in 0..MAX_WAITING_LINES + 10 {
            session.push("noise");
        }

        assert!(session.waiting.len() <= MAX_WAITING_LINES && session.waiting[0].contains("lines were left out"));
    }

    #[test]
    fn settings_show_their_secrets_by_name_only() {
        let settings: JobSettings =
            serde_json::from_value(json!({ "secrets": { "API_KEY": "s3cr3t-value" }, "env": { "BASE": "http://x" } })).unwrap();

        assert_eq!((settings.install_command.as_str(), settings.timeout_minutes), ("npm ci", 30));
        assert!(format!("{settings:?}").contains("API_KEY") && !format!("{settings:?}").contains("s3cr3t-value"));
    }
}
