//! The module's tables. Migrations only ever get appended to.

pub const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE agent_run (
    id INTEGER PRIMARY KEY,
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    instructions TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'queued',
    requested_by INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    shared INTEGER NOT NULL DEFAULT 0,
    -- Who carries the run out: 'server', or the id of a runner. The user is whose token the runner uses.
    runner_id TEXT,
    runner_name TEXT,
    runner_user_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    runner_seen_at TEXT,
    model TEXT,
    repo TEXT,
    branch TEXT,
    pull_request_url TEXT,
    summary TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cost_usd REAL,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    question_pending INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);

CREATE INDEX agent_run_by_issue ON agent_run (issue_id);
CREATE INDEX agent_run_by_state ON agent_run (state, id);

CREATE TABLE agent_run_event (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES agent_run (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    data TEXT,
    actor_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    at TEXT NOT NULL
);

CREATE INDEX agent_run_event_by_run ON agent_run_event (run_id, id);

CREATE TABLE agent_proposal (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES agent_run (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    -- What the proposal would change, as it looked when it was decided.
    preview TEXT,
    state TEXT NOT NULL DEFAULT 'pending',
    decided_by INTEGER REFERENCES user (id) ON DELETE SET NULL,
    decided_at TEXT,
    note TEXT,
    result TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX agent_proposal_by_run ON agent_proposal (run_id, id);

CREATE TABLE agent_runner (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    user_id INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    kinds TEXT NOT NULL,
    projects TEXT,
    version TEXT NOT NULL DEFAULT '',
    last_seen_at TEXT NOT NULL
);
"#, r#"
-- A runner with nobody at it (a container) is not interactive: what its threads want to
-- push is then decided on the web, from the changes it uploads.
ALTER TABLE agent_runner ADD COLUMN interactive INTEGER NOT NULL DEFAULT 1;

-- What a run's thread has committed, as its runner last described it: one JSON document.
CREATE TABLE agent_run_changes (
    run_id INTEGER PRIMARY KEY REFERENCES agent_run (id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#];
