//! The app's own records: repositories, threads, what they said, what they proposed, and the
//! sealed copy of the server's data. One SQLite file under the state folder.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::error::{CoreError, CoreResult};
use crate::model::{EventKind, Proposal, ProposalKind, ProposalState, RepoMapping, Thread, ThreadEvent, ThreadKind, ThreadOrigin, ThreadState};
use crate::secrets::CacheKey;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS repo (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_key TEXT NOT NULL,
    github_repo TEXT,
    path TEXT NOT NULL,
    UNIQUE (project_key, path)
);
CREATE TABLE IF NOT EXISTS thread (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issue_key TEXT NOT NULL,
    issue_summary TEXT NOT NULL,
    issue_type TEXT NOT NULL DEFAULT '',
    project_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    instructions TEXT NOT NULL,
    model TEXT NOT NULL,
    repo_id INTEGER NOT NULL,
    repo_path TEXT NOT NULL,
    worktree TEXT,
    branch TEXT,
    wanted_branch TEXT,
    base TEXT,
    base_commit TEXT,
    state TEXT NOT NULL,
    origin TEXT NOT NULL,
    session_id TEXT,
    run_id INTEGER,
    run_sent TEXT NOT NULL DEFAULT '{}',
    answers_seen INTEGER NOT NULL DEFAULT 0,
    first_prompt TEXT NOT NULL DEFAULT '',
    next_prompt TEXT,
    summary TEXT,
    pull_request_url TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL NOT NULL DEFAULT 0,
    turns INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);
CREATE TABLE IF NOT EXISTS thread_event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT 'null',
    at TEXT NOT NULL,
    mirrored INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS thread_event_thread ON thread_event (thread_id, id);
CREATE TABLE IF NOT EXISTS proposal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    state TEXT NOT NULL,
    remote_id INTEGER,
    note TEXT,
    result TEXT NOT NULL DEFAULT 'null',
    created_at TEXT NOT NULL,
    decided_at TEXT
);
CREATE TABLE IF NOT EXISTS cache (
    name TEXT PRIMARY KEY,
    sealed BLOB NOT NULL,
    fetched_at TEXT NOT NULL
);
";

const THREAD_COLUMNS: &str =
    "t.id, t.issue_key, t.issue_summary, t.project_key, t.kind, t.instructions, t.model, t.repo_id, t.repo_path, t.worktree, \
     t.branch, t.base, t.base_commit, t.state, t.origin, t.session_id, t.run_id, t.summary, t.pull_request_url, t.input_tokens, t.output_tokens, \
     t.cost_usd, t.turns, t.created_at, t.started_at, t.finished_at, \
     (SELECT COUNT(*) FROM proposal p WHERE p.thread_id = t.id AND p.state = 'pending'), \
     (SELECT e.kind FROM thread_event e WHERE e.thread_id = t.id AND e.kind IN ('question', 'answer') ORDER BY e.id DESC LIMIT 1), \
     (SELECT e.text FROM thread_event e WHERE e.thread_id = t.id AND e.kind = 'message' ORDER BY e.id DESC LIMIT 1)";

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn unknown(what: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnName(format!("{what} \"{value}\""))
}

fn json_column(text: String) -> Value {
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn thread_from(row: &Row) -> rusqlite::Result<Thread> {
    let kind: String = row.get(4)?;
    let state: String = row.get(13)?;
    let origin: String = row.get(14)?;
    let last_exchange: Option<String> = row.get(27)?;
    let last_text: Option<String> = row.get(28)?;

    Ok(Thread {
        id: row.get(0)?,
        issue_key: row.get(1)?,
        issue_summary: row.get(2)?,
        project_key: row.get(3)?,
        kind: ThreadKind::parse(&kind).ok_or_else(|| unknown("thread kind", &kind))?,
        instructions: row.get(5)?,
        model: row.get(6)?,
        repo_id: row.get(7)?,
        repo_path: row.get(8)?,
        worktree: row.get(9)?,
        branch: row.get(10)?,
        base: row.get(11)?,
        base_commit: row.get(12)?,
        state: ThreadState::parse(&state).ok_or_else(|| unknown("thread state", &state))?,
        origin: ThreadOrigin::parse(&origin).ok_or_else(|| unknown("thread origin", &origin))?,
        session_id: row.get(15)?,
        run_id: row.get(16)?,
        summary: row.get(17)?,
        pull_request_url: row.get(18)?,
        input_tokens: row.get(19)?,
        output_tokens: row.get(20)?,
        cost_usd: row.get(21)?,
        turns: row.get(22)?,
        created_at: row.get(23)?,
        started_at: row.get(24)?,
        finished_at: row.get(25)?,
        pending_proposals: row.get(26)?,
        pending_question: last_exchange.as_deref() == Some("question"),
        last_text: last_text.map(|text| hub_core::text::truncate(&text, 240)),
    })
}

fn event_from(row: &Row) -> rusqlite::Result<ThreadEvent> {
    let kind: String = row.get(2)?;

    Ok(ThreadEvent {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        kind: EventKind::parse(&kind).ok_or_else(|| unknown("event kind", &kind))?,
        text: row.get(3)?,
        data: json_column(row.get(4)?),
        at: row.get(5)?,
    })
}

fn proposal_from(row: &Row) -> rusqlite::Result<Proposal> {
    let kind: String = row.get(3)?;
    let state: String = row.get(5)?;

    Ok(Proposal {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        issue_key: row.get(2)?,
        kind: ProposalKind::parse(&kind).ok_or_else(|| unknown("proposal kind", &kind))?,
        payload: json_column(row.get(4)?),
        state: ProposalState::parse(&state).ok_or_else(|| unknown("proposal state", &state))?,
        remote_id: row.get(6)?,
        note: row.get(7)?,
        result: json_column(row.get(8)?),
        created_at: row.get(9)?,
        decided_at: row.get(10)?,
    })
}

const PROPOSAL_COLUMNS: &str =
    "p.id, p.thread_id, (SELECT t.issue_key FROM thread t WHERE t.id = p.thread_id), p.kind, p.payload, p.state, p.remote_id, p.note, p.result, p.created_at, p.decided_at";

/// What a new thread starts with.
#[derive(Debug, Clone)]
pub struct ThreadSeed {
    pub issue_key: String,
    pub issue_summary: String,
    pub issue_type: String,
    pub project_key: String,
    pub kind: ThreadKind,
    pub instructions: String,
    pub model: String,
    pub repo: RepoMapping,
    pub wanted_branch: Option<String>,
    pub origin: ThreadOrigin,
    pub run_id: Option<i64>,
    pub prompt: String,
}

/// The parts of a thread only the core needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadPrivate {
    pub issue_type: String,
    pub wanted_branch: Option<String>,
    /// What the thread was asked at the start.
    pub first_prompt: String,
    /// What the person said since the agent last listened; it starts the next turn.
    pub next_prompt: Option<String>,
    pub run_sent: Value,
    pub answers_seen: i64,
}

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> CoreResult<Self> {
        let connection = Connection::open(path)?;

        connection.pragma_update(None, "journal_mode", "WAL")?;
        Self::prepared(connection)
    }

    pub fn in_memory() -> CoreResult<Self> {
        Self::prepared(Connection::open_in_memory()?)
    }

    fn prepared(connection: Connection) -> CoreResult<Self> {
        connection.execute_batch(SCHEMA)?;

        Ok(Self { connection: Mutex::new(connection) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.connection.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // ---------- repositories ----------

    pub fn repos(&self) -> CoreResult<Vec<RepoMapping>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT id, project_key, github_repo, path FROM repo ORDER BY project_key, path")?;
        let rows = statement
            .query_map([], |row| Ok(RepoMapping { id: row.get(0)?, project_key: row.get(1)?, github_repo: row.get(2)?, path: row.get(3)? }))?;

        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn repo(&self, id: i64) -> CoreResult<RepoMapping> {
        self.repos()?.into_iter().find(|repo| repo.id == id).ok_or_else(|| CoreError::not_possible(format!("There is no mapped repository {id}")))
    }

    pub fn add_repo(&self, project_key: &str, github_repo: Option<&str>, path: &str) -> CoreResult<RepoMapping> {
        let db = self.db();

        db.execute(
            "INSERT INTO repo (project_key, github_repo, path) VALUES (?1, ?2, ?3)
             ON CONFLICT (project_key, path) DO UPDATE SET github_repo = excluded.github_repo",
            params![project_key, github_repo, path],
        )?;

        let id = db.query_row("SELECT id FROM repo WHERE project_key = ?1 AND path = ?2", params![project_key, path], |row| row.get(0))?;

        Ok(RepoMapping { id, project_key: project_key.to_string(), github_repo: github_repo.map(String::from), path: path.to_string() })
    }

    pub fn remove_repo(&self, id: i64) -> CoreResult<()> {
        let db = self.db();
        let busy: i64 = db.query_row("SELECT COUNT(*) FROM thread WHERE repo_id = ?1 AND worktree IS NOT NULL", [id], |row| row.get(0))?;

        if busy > 0 {
            return Err(CoreError::not_possible("Threads still have working trees in this repository; remove those first"));
        }

        db.execute("DELETE FROM repo WHERE id = ?1", [id])?;

        Ok(())
    }

    // ---------- threads ----------

    pub fn add_thread(&self, seed: &ThreadSeed) -> CoreResult<Thread> {
        let id = {
            let db = self.db();

            db.execute(
                "INSERT INTO thread (issue_key, issue_summary, issue_type, project_key, kind, instructions, model, repo_id, repo_path, wanted_branch,
                                     state, origin, run_id, first_prompt, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'queued', ?11, ?12, ?13, ?14)",
                params![
                    seed.issue_key,
                    seed.issue_summary,
                    seed.issue_type,
                    seed.project_key,
                    seed.kind.as_str(),
                    seed.instructions,
                    seed.model,
                    seed.repo.id,
                    seed.repo.path,
                    seed.wanted_branch,
                    seed.origin.as_str(),
                    seed.run_id,
                    seed.prompt,
                    now(),
                ],
            )?;
            db.last_insert_rowid()
        };

        self.thread(id)
    }

    pub fn thread(&self, id: i64) -> CoreResult<Thread> {
        self.db()
            .query_row(&format!("SELECT {THREAD_COLUMNS} FROM thread t WHERE t.id = ?1"), [id], thread_from)
            .optional()?
            .ok_or_else(|| CoreError::not_possible(format!("There is no thread {id}")))
    }

    fn threads_where(&self, condition: &str, order: &str) -> CoreResult<Vec<Thread>> {
        let db = self.db();
        let mut statement = db.prepare(&format!("SELECT {THREAD_COLUMNS} FROM thread t WHERE {condition} ORDER BY {order}"))?;
        let rows = statement.query_map([], thread_from)?;

        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Newest first.
    pub fn threads(&self) -> CoreResult<Vec<Thread>> {
        self.threads_where("1", "t.id DESC")
    }

    pub fn threads_in(&self, state: ThreadState) -> CoreResult<Vec<Thread>> {
        self.threads_where(&format!("t.state = '{}'", state.as_str()), "t.id")
    }

    /// Threads whose run on the server may still have something to say or to be told.
    pub fn threads_with_live_runs(&self) -> CoreResult<Vec<Thread>> {
        self.threads_where(
            "t.run_id IS NOT NULL AND (t.state IN ('queued', 'running', 'waiting')
                 OR EXISTS (SELECT 1 FROM proposal p WHERE p.thread_id = t.id AND p.state = 'pending'))",
            "t.id",
        )
    }

    pub fn thread_of_run(&self, run_id: i64) -> CoreResult<Option<Thread>> {
        Ok(self.threads_where(&format!("t.run_id = {run_id}"), "t.id DESC")?.into_iter().next())
    }

    pub fn thread_private(&self, id: i64) -> CoreResult<ThreadPrivate> {
        Ok(self.db().query_row(
            "SELECT issue_type, wanted_branch, first_prompt, next_prompt, run_sent, answers_seen FROM thread WHERE id = ?1",
            [id],
            |row| {
                Ok(ThreadPrivate {
                    issue_type: row.get(0)?,
                    wanted_branch: row.get(1)?,
                    first_prompt: row.get(2)?,
                    next_prompt: row.get(3)?,
                    run_sent: json_column(row.get(4)?),
                    answers_seen: row.get(5)?,
                })
            },
        )?)
    }

    fn update(&self, sql: &str, parameters: impl rusqlite::Params) -> CoreResult<()> {
        self.db().execute(sql, parameters)?;

        Ok(())
    }

    /// Moves a thread to `state` only when it is in one of `from`; says whether it moved.
    pub fn move_thread(&self, id: i64, from: &[ThreadState], state: ThreadState) -> CoreResult<bool> {
        let allowed = from.iter().map(|state| format!("'{}'", state.as_str())).collect::<Vec<_>>().join(", ");
        let finished = matches!(state, ThreadState::Done | ThreadState::Failed | ThreadState::Stopped);
        let started = state == ThreadState::Running;
        let changed = self.db().execute(
            &format!(
                "UPDATE thread SET state = ?2,
                        started_at = CASE WHEN ?3 AND started_at IS NULL THEN ?5 ELSE started_at END,
                        finished_at = CASE WHEN ?4 THEN ?5 ELSE NULL END
                 WHERE id = ?1 AND state IN ({allowed})"
            ),
            params![id, state.as_str(), started, finished, now()],
        )?;

        Ok(changed > 0)
    }

    pub fn set_worktree(
        &self,
        id: i64,
        worktree: Option<&str>,
        branch: Option<&str>,
        base: Option<&str>,
        base_commit: Option<&str>,
    ) -> CoreResult<()> {
        self.update(
            "UPDATE thread SET worktree = ?2, branch = COALESCE(?3, branch), base = COALESCE(?4, base), base_commit = COALESCE(?5, base_commit) WHERE id = ?1",
            params![id, worktree, branch, base, base_commit],
        )
    }

    pub fn set_session(&self, id: i64, session_id: &str) -> CoreResult<()> {
        self.update("UPDATE thread SET session_id = ?2 WHERE id = ?1", params![id, session_id])
    }

    pub fn set_next_prompt(&self, id: i64, prompt: Option<&str>) -> CoreResult<()> {
        self.update("UPDATE thread SET next_prompt = ?2 WHERE id = ?1", params![id, prompt])
    }

    pub fn set_run(&self, id: i64, run_id: Option<i64>) -> CoreResult<()> {
        self.update("UPDATE thread SET run_id = ?2, run_sent = '{}', answers_seen = 0 WHERE id = ?1", params![id, run_id])
    }

    pub fn set_run_sent(&self, id: i64, sent: &Value) -> CoreResult<()> {
        self.update("UPDATE thread SET run_sent = ?2 WHERE id = ?1", params![id, sent.to_string()])
    }

    pub fn set_answers_seen(&self, id: i64, event_id: i64) -> CoreResult<()> {
        self.update("UPDATE thread SET answers_seen = MAX(answers_seen, ?2) WHERE id = ?1", params![id, event_id])
    }

    pub fn set_pull_request(&self, id: i64, url: &str) -> CoreResult<()> {
        self.update("UPDATE thread SET pull_request_url = ?2 WHERE id = ?1", params![id, url])
    }

    /// The outcome of one turn. Turns add up. Tokens and cost are the session's running
    /// totals as the agent reports them: a resumed session that carries its totals over
    /// reports at least what is stored and replaces it; one that starts counting again is added.
    pub fn record_result(&self, id: i64, summary: &str, input_tokens: i64, output_tokens: i64, cost_usd: f64, turns: i64) -> CoreResult<()> {
        self.update(
            "UPDATE thread SET summary = ?2,
                    input_tokens = CASE WHEN ?3 >= input_tokens THEN ?3 ELSE input_tokens + ?3 END,
                    output_tokens = CASE WHEN ?4 >= output_tokens THEN ?4 ELSE output_tokens + ?4 END,
                    cost_usd = CASE WHEN ?5 >= cost_usd THEN ?5 ELSE cost_usd + ?5 END,
                    turns = turns + ?6
             WHERE id = ?1",
            params![id, summary, input_tokens, output_tokens, cost_usd, turns],
        )
    }

    pub fn delete_thread(&self, id: i64) -> CoreResult<()> {
        let db = self.db();

        db.execute("DELETE FROM thread_event WHERE thread_id = ?1", [id])?;
        db.execute("DELETE FROM proposal WHERE thread_id = ?1", [id])?;
        db.execute("DELETE FROM thread WHERE id = ?1", [id])?;

        Ok(())
    }

    // ---------- events ----------

    /// `mirrored` is set for events the server already has (an answer given on the web).
    pub fn add_event(&self, thread_id: i64, kind: EventKind, text: &str, data: &Value, mirrored: bool) -> CoreResult<ThreadEvent> {
        let at = now();
        let db = self.db();

        db.execute(
            "INSERT INTO thread_event (thread_id, kind, text, data, at, mirrored) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![thread_id, kind.as_str(), text, data.to_string(), at, mirrored],
        )?;

        Ok(ThreadEvent { id: db.last_insert_rowid(), thread_id, kind, text: text.to_string(), data: data.clone(), at })
    }

    fn events_where(&self, condition: &str, parameters: impl rusqlite::Params) -> CoreResult<Vec<ThreadEvent>> {
        let db = self.db();
        let mut statement = db.prepare(&format!("SELECT id, thread_id, kind, text, data, at FROM thread_event WHERE {condition} ORDER BY id"))?;
        let rows = statement.query_map(parameters, event_from)?;

        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn events(&self, thread_id: i64) -> CoreResult<Vec<ThreadEvent>> {
        self.events_where("thread_id = ?1", [thread_id])
    }

    pub fn unmirrored_events(&self, thread_id: i64) -> CoreResult<Vec<ThreadEvent>> {
        self.events_where("thread_id = ?1 AND mirrored = 0", [thread_id])
    }

    pub fn mark_mirrored(&self, event_id: i64) -> CoreResult<()> {
        self.update("UPDATE thread_event SET mirrored = 1 WHERE id = ?1", [event_id])
    }

    /// Events written before the thread had a run are not sent to a later one.
    pub fn mark_all_mirrored(&self, thread_id: i64) -> CoreResult<()> {
        self.update("UPDATE thread_event SET mirrored = 1 WHERE thread_id = ?1", [thread_id])
    }

    // ---------- proposals ----------

    pub fn add_proposal(&self, thread_id: i64, kind: ProposalKind, payload: &Value, remote_id: Option<i64>) -> CoreResult<Proposal> {
        let id = {
            let db = self.db();

            db.execute(
                "INSERT INTO proposal (thread_id, kind, payload, state, remote_id, created_at) VALUES (?1, ?2, ?3, 'pending', ?4, ?5)",
                params![thread_id, kind.as_str(), payload.to_string(), remote_id, now()],
            )?;
            db.last_insert_rowid()
        };

        self.proposal(id)
    }

    pub fn proposal(&self, id: i64) -> CoreResult<Proposal> {
        self.db()
            .query_row(&format!("SELECT {PROPOSAL_COLUMNS} FROM proposal p WHERE p.id = ?1"), [id], proposal_from)
            .optional()?
            .ok_or_else(|| CoreError::not_possible(format!("There is no proposal {id}")))
    }

    fn proposals_where(&self, condition: &str) -> CoreResult<Vec<Proposal>> {
        let db = self.db();
        let mut statement = db.prepare(&format!("SELECT {PROPOSAL_COLUMNS} FROM proposal p WHERE {condition} ORDER BY p.id"))?;
        let rows = statement.query_map([], proposal_from)?;

        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn proposals(&self, thread_id: i64) -> CoreResult<Vec<Proposal>> {
        self.proposals_where(&format!("p.thread_id = {thread_id}"))
    }

    pub fn pending_proposals(&self) -> CoreResult<Vec<Proposal>> {
        self.proposals_where("p.state = 'pending'")
    }

    pub fn proposal_by_remote(&self, remote_id: i64) -> CoreResult<Option<Proposal>> {
        Ok(self.proposals_where(&format!("p.remote_id = {remote_id}"))?.into_iter().next())
    }

    /// Records a decision; says whether the state changed.
    pub fn decide_proposal(&self, id: i64, state: ProposalState, note: Option<&str>, result: &Value) -> CoreResult<bool> {
        let decided_at = (state != ProposalState::Pending).then(now);
        let changed = self.db().execute(
            "UPDATE proposal SET state = ?2, note = ?3, result = ?4, decided_at = ?5 WHERE id = ?1 AND state <> ?2",
            params![id, state.as_str(), note, result.to_string(), decided_at],
        )?;

        Ok(changed > 0)
    }

    // ---------- the sealed copy of the server's data ----------

    pub fn cache_put<T: Serialize>(&self, key: &CacheKey, name: &str, value: &T) -> CoreResult<String> {
        let sealed = key.seal(&serde_json::to_vec(value)?)?;
        let fetched_at = now();

        self.update(
            "INSERT INTO cache (name, sealed, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (name) DO UPDATE SET sealed = excluded.sealed, fetched_at = excluded.fetched_at",
            params![name, sealed, fetched_at],
        )?;

        Ok(fetched_at)
    }

    /// The stored value and when it was fetched. A row that the key does not open is dropped.
    pub fn cache_get<T: DeserializeOwned>(&self, key: &CacheKey, name: &str) -> CoreResult<Option<(T, String)>> {
        let found: Option<(Vec<u8>, String)> =
            self.db().query_row("SELECT sealed, fetched_at FROM cache WHERE name = ?1", [name], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
        let Some((sealed, fetched_at)) = found else { return Ok(None) };
        let value = key.open(&sealed).and_then(|plain| serde_json::from_slice(&plain).ok());

        if value.is_none() {
            self.update("DELETE FROM cache WHERE name = ?1", [name])?;
        }

        Ok(value.map(|value| (value, fetched_at)))
    }

    pub fn cache_clear(&self) -> CoreResult<()> {
        self.update("DELETE FROM cache", [])
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::secrets::MemoryKeychain;

    pub fn seed(store: &Store, key: &str) -> ThreadSeed {
        let repo = store.add_repo("WEB", Some("acme/web"), "/repos/web").unwrap();

        ThreadSeed {
            issue_key: key.to_string(),
            issue_summary: "Login form".to_string(),
            issue_type: "Task".to_string(),
            project_key: "WEB".to_string(),
            kind: ThreadKind::Implement,
            instructions: String::new(),
            model: "sonnet".to_string(),
            repo,
            wanted_branch: None,
            origin: ThreadOrigin::App,
            run_id: None,
            prompt: "Do it".to_string(),
        }
    }

    #[test]
    fn a_thread_moves_only_from_the_states_named() {
        let store = Store::in_memory().unwrap();
        let thread = store.add_thread(&seed(&store, "WEB-1")).unwrap();

        assert_eq!(thread.state, ThreadState::Queued);
        assert!(!store.move_thread(thread.id, &[ThreadState::Running], ThreadState::Done).unwrap());
        assert!(store.move_thread(thread.id, &[ThreadState::Queued], ThreadState::Running).unwrap());
        assert!(store.move_thread(thread.id, &[ThreadState::Running], ThreadState::Done).unwrap());

        let done = store.thread(thread.id).unwrap();

        assert_eq!(done.state, ThreadState::Done);
        assert!(done.started_at.is_some() && done.finished_at.is_some());
    }

    #[test]
    fn a_question_is_pending_until_an_answer_follows_it() {
        let store = Store::in_memory().unwrap();
        let thread = store.add_thread(&seed(&store, "WEB-1")).unwrap();

        store.add_event(thread.id, EventKind::Question, "Which database?", &Value::Null, false).unwrap();
        assert!(store.thread(thread.id).unwrap().pending_question);

        store.add_event(thread.id, EventKind::Answer, "SQLite", &Value::Null, true).unwrap();
        store.add_event(thread.id, EventKind::Message, "Thanks", &Value::Null, false).unwrap();

        let thread = store.thread(thread.id).unwrap();

        assert!(!thread.pending_question);
        assert_eq!(thread.last_text.as_deref(), Some("Thanks"));
        assert_eq!(store.unmirrored_events(thread.id).unwrap().len(), 2);
    }

    #[test]
    fn proposals_are_counted_while_pending_and_decided_once() {
        let store = Store::in_memory().unwrap();
        let thread = store.add_thread(&seed(&store, "WEB-1")).unwrap();
        let proposal = store.add_proposal(thread.id, ProposalKind::Comment, &json!({ "body": "Done" }), Some(41)).unwrap();

        assert_eq!(proposal.issue_key, "WEB-1");
        assert_eq!(store.thread(thread.id).unwrap().pending_proposals, 1);
        assert!(store.decide_proposal(proposal.id, ProposalState::Approved, None, &json!({ "commentId": 3 })).unwrap());
        assert!(!store.decide_proposal(proposal.id, ProposalState::Approved, None, &Value::Null).unwrap());
        assert_eq!(store.thread(thread.id).unwrap().pending_proposals, 0);
        assert_eq!(store.proposal_by_remote(41).unwrap().unwrap().state, ProposalState::Approved);
    }

    #[test]
    fn the_copy_of_server_data_is_sealed_on_disk_and_dropped_when_the_key_changes() {
        let folder = std::env::temp_dir().join(format!("hub-desktop-store-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&folder);
        let file = folder.join("cache.db");
        let store = Store::open(&file).unwrap();
        let key = CacheKey::load_or_create(&MemoryKeychain::default()).unwrap();

        store.cache_put(&key, "issue:WEB-1", &json!({ "summary": "Sekretny projekt Feniks" })).unwrap();
        drop(store);

        let on_disk = std::fs::read(&file).unwrap();
        let wal = std::fs::read(folder.join("cache.db-wal")).unwrap_or_default();

        assert!(![on_disk, wal].concat().windows(6).any(|window| window == b"Feniks"));

        let store = Store::open(&file).unwrap();
        let read: Option<(Value, String)> = store.cache_get(&key, "issue:WEB-1").unwrap();

        assert_eq!(read.unwrap().0["summary"], "Sekretny projekt Feniks");

        let other = CacheKey::load_or_create(&MemoryKeychain::default()).unwrap();

        assert!(store.cache_get::<Value>(&other, "issue:WEB-1").unwrap().is_none());
        assert!(store.cache_get::<Value>(&key, "issue:WEB-1").unwrap().is_none());

        let _ = std::fs::remove_dir_all(folder);
    }
}
