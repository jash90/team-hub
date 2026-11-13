//! Runs agent threads: a few at a time, each in its own working tree and its own helper
//! process, so that stopping one or a crash of one touches nothing else.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, watch, Notify};

use crate::error::{CoreError, CoreResult};
use crate::git;
use crate::model::{AppEvent, EventKind, SandboxState, Thread, ThreadEvent, ThreadState};
use crate::policy;
use crate::protocol::{AgentEvent, FromSidecar, HubTool, SandboxPolicy, StartThread, ToSidecar};
use crate::store::{Store, ThreadSeed};

const INTERRUPTED: &str = "Interrupted: the app was closed while this thread was working. Resume it to continue.";
const STOPPED: &str = "Stopped.";
const CONTINUE: &str = "Continue where you stopped.";
const KEPT_ERROR_LINES: usize = 12;

/// What the runtime needs from whoever owns the connection to the server. Both calls block.
pub trait Host: Send + Sync + 'static {
    /// The server's read-only tools this thread may call; empty when there is no connection.
    fn hub_tools(&self, thread: &Thread) -> Vec<HubTool>;
    /// Answers a tool call the agent made: a read of the server, a proposal, a question.
    fn call_tool(&self, thread: &Thread, tool: &str, input: &Value) -> Result<Value, String>;
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// The helper process: the program and its arguments.
    pub program: String,
    pub arguments: Vec<String>,
    pub max_running: usize,
    pub max_turns: u32,
    /// How long a helper gets to end by itself after `stop` or after its result.
    pub stop_grace: Duration,
    pub worktrees_root: PathBuf,
    /// Folders no thread may read: credentials, and the app's own state.
    pub private_paths: Vec<PathBuf>,
    pub sandbox: SandboxState,
    pub allow_unsandboxed_shell: bool,
    /// Added to the helper's environment.
    pub env: BTreeMap<String, String>,
}

struct Running {
    to_helper: mpsc::UnboundedSender<ToSidecar>,
    pid: Option<u32>,
    stopping: Arc<AtomicBool>,
    exited: watch::Receiver<bool>,
}

struct Inner {
    store: Arc<Store>,
    config: RuntimeConfig,
    max_running: AtomicUsize,
    host: Arc<dyn Host>,
    events: broadcast::Sender<AppEvent>,
    running: Mutex<HashMap<i64, Running>>,
    pump_lock: tokio::sync::Mutex<()>,
    /// Poked whenever a thread said or became something, for whoever mirrors threads elsewhere.
    activity: Arc<Notify>,
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<Inner>,
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    // The helper leads its own process group, so this reaches the agent process and
    // whatever its shell commands started.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(pid: u32) {
    let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output();
}

fn repo_folder_name(repo: &Path) -> String {
    repo.file_name().map(|name| name.to_string_lossy().into_owned()).filter(|name| !name.is_empty()).unwrap_or_else(|| "repository".to_string())
}

impl Inner {
    fn emit(&self, event: AppEvent) {
        let _ = self.events.send(event);
        self.activity.notify_one();
    }

    fn changed(&self, thread_id: i64) {
        self.emit(AppEvent::ThreadChanged { thread_id });
    }

    fn say(&self, thread_id: i64, kind: EventKind, text: &str, data: Value) -> Option<ThreadEvent> {
        let event = self.store.add_event(thread_id, kind, text, &data, false).ok()?;

        self.emit(AppEvent::ThreadEvent { thread_id, event: event.clone() });

        Some(event)
    }

    fn notify(&self, thread: &Thread, title: String, body: &str) {
        self.emit(AppEvent::Notify { title, body: hub_core::text::truncate(body, 200), thread_id: Some(thread.id), issue_key: Some(thread.issue_key.clone()) });
    }

    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Running>> {
        self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn fail(&self, thread_id: i64, reason: &str) {
        self.say(thread_id, EventKind::Error, reason, Value::Null);

        if self.store.move_thread(thread_id, &[ThreadState::Queued, ThreadState::Running], ThreadState::Failed).unwrap_or(false) {
            if let Ok(thread) = self.store.thread(thread_id) {
                self.notify(&thread, format!("{} failed", thread.issue_key), reason);
            }
        }

        self.changed(thread_id);
    }

    /// The working tree of the thread, made on first use, and what the helper is told.
    fn prepare(&self, thread_id: i64) -> CoreResult<StartThread> {
        let thread = self.store.thread(thread_id)?;
        let private = self.store.thread_private(thread_id)?;
        let repo = PathBuf::from(&thread.repo_path);
        let existing = thread.worktree.as_ref().map(PathBuf::from).filter(|path| path.exists()).zip(thread.branch.clone());
        let (worktree, branch) = match existing {
            Some(found) => found,
            None => {
                let suggested = || hub_core::keys::suggest_branch_name(&thread.issue_key, &thread.issue_summary, &private.issue_type);
                let branch = thread.branch.clone().or(private.wanted_branch.clone()).unwrap_or_else(suggested);
                let folder = self.config.worktrees_root.join(repo_folder_name(&repo)).join(format!("{}-t{}", thread.issue_key.to_lowercase(), thread.id));
                let made = git::create_worktree(&repo, &folder, &branch)?;
                let place = made.path.to_string_lossy().into_owned();
                let how = if made.continued { "continuing branch" } else { "new branch" };

                self.store.set_worktree(thread.id, Some(&place), Some(&made.branch), Some(&made.base), Some(&made.base_commit))?;
                self.say(
                    thread.id,
                    EventKind::Status,
                    &format!("Working tree ready: {how} {} from {}", made.branch, made.base),
                    json!({ "worktree": place, "branch": made.branch, "base": made.base, "continued": made.continued }),
                );

                (made.path, made.branch)
            }
        };
        let access = git::write_access(&repo, &worktree, &branch)?;
        let cwd = std::fs::canonicalize(&worktree)?.to_string_lossy().into_owned();
        let sandboxed = self.config.sandbox == SandboxState::Available;
        let shell = sandboxed || self.config.allow_unsandboxed_shell;
        let unreadable: Vec<String> = self.config.private_paths.iter().map(|path| path.to_string_lossy().into_owned()).collect();
        let mut denied: Vec<String> = policy::DENIED_TOOLS.iter().map(|tool| (*tool).to_string()).collect();

        denied.extend(policy::read_denials(&unreadable));

        if !sandboxed {
            let note = if shell {
                "This system has no sandbox for shell commands; the shell runs unconfined because the settings allow it."
            } else {
                "This system has no sandbox for shell commands, so this thread works without a shell."
            };

            self.say(thread.id, EventKind::Status, note, Value::Null);
        }

        let thread = self.store.thread(thread_id)?;
        let prompt = match (&thread.session_id, private.next_prompt) {
            (Some(_), Some(said)) => said,
            (Some(_), None) => CONTINUE.to_string(),
            (None, Some(said)) => format!("{}\n\n{said}", private.first_prompt),
            (None, None) => private.first_prompt,
        };

        Ok(StartThread {
            thread: thread.id,
            prompt,
            cwd: cwd.clone(),
            model: thread.model.clone(),
            resume: thread.session_id.clone(),
            max_turns: self.config.max_turns,
            rules: policy::rules(thread.kind),
            allowed_tools: policy::allowed_tools(thread.kind, &cwd, shell),
            denied_tools: denied,
            sandbox: sandboxed.then(|| SandboxPolicy { writable: access.writable, protected: access.protected, unreadable }),
            hub_tools: self.host.hub_tools(&thread),
            env: BTreeMap::new(),
        })
    }

    async fn launch(self: &Arc<Self>, thread_id: i64) -> CoreResult<()> {
        let inner = Arc::clone(self);
        let start = tokio::task::spawn_blocking(move || inner.prepare(thread_id)).await??;
        let mut command = Command::new(&self.config.program);

        command.args(&self.config.arguments).envs(&self.config.env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);

        #[cfg(unix)]
        command.process_group(0);

        let mut child = command
            .spawn()
            .map_err(|error| CoreError::not_possible(format!("The agent helper could not be started ({}): {error}", self.config.program)))?;
        let pid = child.id();
        let mut stdin = child.stdin.take().ok_or_else(|| CoreError::not_possible("The agent helper has no input"))?;
        let stdout = child.stdout.take().ok_or_else(|| CoreError::not_possible("The agent helper has no output"))?;
        let stderr = child.stderr.take();
        let (to_helper, mut outbox) = mpsc::unbounded_channel::<ToSidecar>();
        let (exited_tx, exited) = watch::channel(false);
        let stopping = Arc::new(AtomicBool::new(false));
        let complaints = Arc::new(Mutex::new(VecDeque::<String>::new()));

        tokio::spawn(async move {
            while let Some(message) = outbox.recv().await {
                let Ok(mut line) = serde_json::to_string(&message) else { continue };

                line.push('\n');

                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        if let Some(stderr) = stderr {
            let kept = Arc::clone(&complaints);

            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();

                while let Ok(Some(line)) = lines.next_line().await {
                    let mut kept = kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

                    if kept.len() == KEPT_ERROR_LINES {
                        kept.pop_front();
                    }

                    kept.push_back(line);
                }
            });
        }

        let _ = to_helper.send(ToSidecar::Start(Box::new(start)));
        let _ = self.store.set_next_prompt(thread_id, None);
        self.running().insert(thread_id, Running { to_helper: to_helper.clone(), pid, stopping: Arc::clone(&stopping), exited });

        let inner = Arc::clone(self);

        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut finished = false;

            loop {
                let line = if finished {
                    // The turn is over; a helper that lingers is not waited for.
                    match tokio::time::timeout(inner.config.stop_grace, lines.next_line()).await {
                        Ok(line) => line,
                        Err(_) => break,
                    }
                } else {
                    lines.next_line().await
                };
                let Ok(Some(line)) = line else { break };
                let Ok(message) = serde_json::from_str::<FromSidecar>(&line) else { continue };

                match message {
                    FromSidecar::Ready { .. } => {}
                    FromSidecar::Event { event } => finished |= inner.on_event(thread_id, event),
                    FromSidecar::Request { request, tool, input } => inner.answer(thread_id, request, tool, input, to_helper.clone()),
                }
            }

            if let Some(pid) = pid {
                kill_group(pid);
            }

            let status = child.wait().await.ok();

            inner.running().remove(&thread_id);

            let complaint = complaints.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().cloned().collect::<Vec<_>>().join("\n");

            inner.on_exit(thread_id, stopping.load(Ordering::SeqCst), status.and_then(|status| status.code()), &complaint);

            let _ = exited_tx.send(true);

            inner.pump().await;
        });

        Ok(())
    }

    fn answer(self: &Arc<Self>, thread_id: i64, request: u64, tool: String, input: Value, to_helper: mpsc::UnboundedSender<ToSidecar>) {
        let inner = Arc::clone(self);

        tokio::spawn(async move {
            let outcome = tokio::task::spawn_blocking(move || {
                let thread = inner.store.thread(thread_id).map_err(|error| error.to_string())?;
                let result = inner.host.call_tool(&thread, &tool, &input);

                inner.changed(thread_id);
                result
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let (ok, result) = match outcome {
                Ok(result) => (true, result),
                Err(reason) => (false, Value::String(reason)),
            };

            let _ = to_helper.send(ToSidecar::Reply { request, ok, result });
        });
    }

    /// Records what the agent did. True when the turn is over.
    fn on_event(&self, thread_id: i64, event: AgentEvent) -> bool {
        match event {
            AgentEvent::Session { session_id } => {
                let _ = self.store.set_session(thread_id, &session_id);

                false
            }
            AgentEvent::Assistant { text } => {
                self.say(thread_id, EventKind::Message, &text, Value::Null);

                false
            }
            AgentEvent::Tool { name, detail } => {
                self.say(thread_id, EventKind::Tool, &format!("{name} {detail}").trim().to_string(), json!({ "name": name, "detail": detail }));

                false
            }
            AgentEvent::Error { message } => {
                self.fail(thread_id, &message);

                true
            }
            AgentEvent::Result { ok, summary, cost_usd, turns, input_tokens, output_tokens } => {
                let _ = self.store.record_result(
                    thread_id,
                    &summary,
                    input_tokens.unwrap_or(0),
                    output_tokens.unwrap_or(0),
                    cost_usd.unwrap_or(0.0),
                    i64::from(turns.unwrap_or(0)),
                );
                let Ok(thread) = self.store.thread(thread_id) else { return true };
                let said_meanwhile = self.store.thread_private(thread_id).is_ok_and(|private| private.next_prompt.is_some());
                let next = match (said_meanwhile, thread.pending_question, ok) {
                    // The person answered while the agent was still finishing its turn.
                    (true, _, _) => ThreadState::Queued,
                    (false, true, _) => ThreadState::Waiting,
                    (false, false, true) => ThreadState::Done,
                    (false, false, false) => ThreadState::Failed,
                };

                if !ok {
                    self.say(thread_id, EventKind::Error, &summary, Value::Null);
                }

                if self.store.move_thread(thread_id, &[ThreadState::Running], next).unwrap_or(false) {
                    match next {
                        ThreadState::Waiting => self.notify(&thread, format!("{} is waiting for your answer", thread.issue_key), &summary),
                        ThreadState::Failed => self.notify(&thread, format!("{} failed", thread.issue_key), &summary),
                        ThreadState::Queued => {}
                        _ if thread.pending_proposals > 0 => self.notify(&thread, format!("{} has proposals to decide", thread.issue_key), &summary),
                        _ => self.notify(&thread, format!("{} is done", thread.issue_key), &summary),
                    }
                }

                self.changed(thread_id);

                true
            }
        }
    }

    fn on_exit(&self, thread_id: i64, stopped: bool, code: Option<i32>, complaint: &str) {
        if stopped {
            if self.store.move_thread(thread_id, &[ThreadState::Running, ThreadState::Queued], ThreadState::Stopped).unwrap_or(false) {
                self.say(thread_id, EventKind::Status, STOPPED, Value::Null);
            }

            self.changed(thread_id);

            return;
        }

        let still_running = self.store.thread(thread_id).is_ok_and(|thread| thread.state == ThreadState::Running);

        if still_running {
            let code = code.map_or_else(|| "killed".to_string(), |code| format!("exit code {code}"));
            let detail = if complaint.is_empty() { String::new() } else { format!("\n{complaint}") };

            self.fail(thread_id, &format!("The agent process ended without a result ({code}).{detail}"));
        }
    }

    async fn pump(self: &Arc<Self>) {
        let _turn = self.pump_lock.lock().await;

        loop {
            let working = self.store.threads_in(ThreadState::Running).map(|threads| threads.len()).unwrap_or(usize::MAX);

            if working >= self.max_running.load(Ordering::SeqCst) {
                break;
            }

            let Some(next) = self.store.threads_in(ThreadState::Queued).ok().and_then(|queued| queued.into_iter().next()) else { break };

            if !self.store.move_thread(next.id, &[ThreadState::Queued], ThreadState::Running).unwrap_or(false) {
                break;
            }

            self.changed(next.id);

            if let Err(error) = self.launch(next.id).await {
                self.fail(next.id, &error.to_string());
            }
        }
    }
}

impl Runtime {
    pub fn new(store: Arc<Store>, config: RuntimeConfig, host: Arc<dyn Host>, events: broadcast::Sender<AppEvent>) -> Self {
        let max_running = AtomicUsize::new(config.max_running.max(1));

        Self {
            inner: Arc::new(Inner {
                store,
                config,
                max_running,
                host,
                events,
                running: Mutex::new(HashMap::new()),
                pump_lock: tokio::sync::Mutex::new(()),
                activity: Arc::new(Notify::new()),
            }),
        }
    }

    pub fn activity(&self) -> Arc<Notify> {
        Arc::clone(&self.inner.activity)
    }

    pub fn set_max_running(&self, limit: usize) {
        self.inner.max_running.store(limit.max(1), Ordering::SeqCst);
        self.kick();
    }

    /// Threads the last run of the app left working have no process any more.
    pub fn recover(&self) -> CoreResult<()> {
        for thread in self.inner.store.threads_in(ThreadState::Running)? {
            self.inner.store.move_thread(thread.id, &[ThreadState::Running], ThreadState::Stopped)?;
            self.inner.say(thread.id, EventKind::Status, INTERRUPTED, Value::Null);
        }

        self.kick();

        Ok(())
    }

    /// Starts whatever is queued and has a free slot, without waiting for it.
    pub fn kick(&self) {
        let inner = Arc::clone(&self.inner);

        tokio::spawn(async move { inner.pump().await });
    }

    /// A new thread, queued. It starts as soon as a slot is free.
    pub fn enqueue(&self, seed: &ThreadSeed) -> CoreResult<Thread> {
        let thread = self.inner.store.add_thread(seed)?;

        self.inner.changed(thread.id);
        self.kick();

        Ok(thread)
    }

    pub fn is_working(&self, thread_id: i64) -> bool {
        self.inner.running().contains_key(&thread_id)
    }

    /// Stops a thread. Returns once its process is gone: asked first, killed after the grace
    /// period, together with everything it started.
    pub async fn stop(&self, thread_id: i64) -> CoreResult<Thread> {
        let inner = &self.inner;
        let handle = inner.running().get(&thread_id).map(|running| (running.to_helper.clone(), running.pid, Arc::clone(&running.stopping), running.exited.clone()));

        match handle {
            Some((to_helper, pid, stopping, mut exited)) => {
                stopping.store(true, Ordering::SeqCst);

                let _ = to_helper.send(ToSidecar::Stop);
                let ended = tokio::time::timeout(inner.config.stop_grace, exited.wait_for(|gone| *gone)).await.is_ok();

                if !ended {
                    if let Some(pid) = pid {
                        kill_group(pid);
                    }

                    let _ = tokio::time::timeout(Duration::from_secs(10), exited.wait_for(|gone| *gone)).await;
                }
            }
            None => {
                if inner.store.move_thread(thread_id, &[ThreadState::Queued, ThreadState::Running, ThreadState::Waiting], ThreadState::Stopped)? {
                    inner.say(thread_id, EventKind::Status, STOPPED, Value::Null);
                    inner.changed(thread_id);
                }
            }
        }

        inner.store.thread(thread_id)
    }

    /// Puts a thread that is not working back in the queue, in the same session. `message` is
    /// what the person says to it; `mirrored` tells whether the server already has that text.
    pub fn resume(&self, thread_id: i64, message: Option<&str>, mirrored: bool) -> CoreResult<Thread> {
        let inner = &self.inner;
        let thread = inner.store.thread(thread_id)?;

        let message = message.map(str::trim).filter(|message| !message.is_empty());

        if let Some(message) = message {
            if let Ok(event) = inner.store.add_event(thread_id, EventKind::Answer, message, &Value::Null, mirrored) {
                inner.emit(AppEvent::ThreadEvent { thread_id, event });
            }

            inner.store.set_next_prompt(thread_id, Some(message))?;
        }

        // A working thread hears it when its turn ends; a queued one when it starts.
        if thread.state.is_active() {
            return inner.store.thread(thread_id);
        }

        let from = [ThreadState::Waiting, ThreadState::Done, ThreadState::Failed, ThreadState::Stopped];

        if inner.store.move_thread(thread_id, &from, ThreadState::Queued)? {
            inner.say(thread_id, EventKind::Status, "Resumed.", Value::Null);
            inner.changed(thread_id);
            self.kick();
        }

        inner.store.thread(thread_id)
    }

    /// Removes the thread's working tree; the branch and the thread's record stay. Git refuses
    /// while the tree holds uncommitted changes, and that refusal is passed on.
    pub async fn remove_worktree(&self, thread_id: i64) -> CoreResult<Thread> {
        let thread = self.inner.store.thread(thread_id)?;

        if thread.state.is_active() || self.is_working(thread_id) {
            return Err(CoreError::not_possible("Stop the thread before removing its working tree"));
        }

        if let Some(worktree) = thread.worktree.clone() {
            let repo = PathBuf::from(&thread.repo_path);

            tokio::task::spawn_blocking(move || if Path::new(&worktree).exists() { git::remove_worktree(&repo, Path::new(&worktree)) } else { Ok(()) }).await??;
            self.inner.store.set_worktree(thread_id, None, None, None, None)?;
            self.inner.say(thread_id, EventKind::Status, "Working tree removed; the branch is kept.", Value::Null);
            self.inner.changed(thread_id);
        }

        self.inner.store.thread(thread_id)
    }

    /// Waits until no thread is working or queued.
    pub async fn wait_idle(&self, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;

        loop {
            let busy = [ThreadState::Queued, ThreadState::Running]
                .iter()
                .any(|state| self.inner.store.threads_in(*state).map(|threads| !threads.is_empty()).unwrap_or(false));

            if !busy && self.inner.running().is_empty() {
                return true;
            }

            if tokio::time::Instant::now() >= deadline {
                return false;
            }

            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}
