pub const MIGRATIONS: &[&str] = &[r#"
-- One run of a test suite, as a report posted by CI (or by hand) describes it.
CREATE TABLE e2e_run (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    -- The run id the report carries; posting the same report twice stores it once.
    external_id TEXT,
    name TEXT,
    format TEXT NOT NULL,
    status TEXT NOT NULL,
    branch TEXT,
    commit_sha TEXT,
    pr_number INTEGER,
    ci_url TEXT,
    runner TEXT,
    -- What kept the run from reaching its tests, when something did.
    error TEXT,
    started_at TEXT NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    passed INTEGER NOT NULL DEFAULT 0,
    failed INTEGER NOT NULL DEFAULT 0,
    flaky INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    agent_steps INTEGER NOT NULL DEFAULT 0,
    model_calls INTEGER NOT NULL DEFAULT 0,
    model_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL,
    created_by INTEGER REFERENCES user (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX e2e_run_by_external ON e2e_run (project_id, external_id) WHERE external_id IS NOT NULL;
CREATE INDEX e2e_run_by_project ON e2e_run (project_id, started_at);

-- A test, the same one across runs: its file and its full title. The `last_*` columns
-- repeat its result in the newest run that ran it, so issues can be asked about their tests cheaply.
CREATE TABLE e2e_test (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    file TEXT NOT NULL,
    title TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    line INTEGER,
    last_run_id INTEGER REFERENCES e2e_run (id) ON DELETE SET NULL,
    last_status TEXT,
    last_at TEXT,
    last_duration_ms INTEGER,
    last_attempts INTEGER,
    last_message TEXT,
    last_error_code TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (project_id, file, title)
);

-- A test in a run. `variant` tells apart the targets (browsers, devices) and agents one run
-- ran the test on; it is empty when there is only one.
CREATE TABLE e2e_result (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES e2e_run (id) ON DELETE CASCADE,
    test_id INTEGER NOT NULL REFERENCES e2e_test (id) ON DELETE CASCADE,
    variant TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    line INTEGER,
    message TEXT,
    error_code TEXT,
    agent_steps INTEGER NOT NULL DEFAULT 0,
    model_calls INTEGER NOT NULL DEFAULT 0,
    model_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL,
    UNIQUE (run_id, test_id, variant)
);

CREATE INDEX e2e_result_by_test ON e2e_result (test_id, run_id);

-- Test → issue edges live in the shared `link` table (from_kind 'e2e_test', to_kind 'issue',
-- relation 'verifies' when read from the title or tags, 'verifies_manual' when set by hand).
-- This table remembers the ones a person removed, so the next run does not bring them back.
CREATE TABLE e2e_unlinked (
    test_id INTEGER NOT NULL REFERENCES e2e_test (id) ON DELETE CASCADE,
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    PRIMARY KEY (test_id, issue_id)
);
"#,
// Test jobs: a project's test settings, the jobs runners carry out, what a job logs, the
// files a run's report names, and what a result remembers of its session.
r#"
-- How a project's suite is run. No row: the defaults, and no repository.
CREATE TABLE e2e_settings (
    project_id INTEGER PRIMARY KEY REFERENCES project (id) ON DELETE CASCADE,
    repo TEXT,
    git_ref TEXT NOT NULL,
    directory TEXT NOT NULL,
    install_command TEXT NOT NULL,
    run_args TEXT NOT NULL,
    video TEXT NOT NULL,
    trace TEXT NOT NULL,
    -- Plain variables as a JSON object. Secret values are in the core's `secret` table,
    -- named `e2e:<project id>:<name>`.
    env TEXT NOT NULL DEFAULT '{}',
    timeout_minutes INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);

-- One request to run the suite, from the moment somebody asks to the moment a runner is done.
CREATE TABLE e2e_job (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    repo TEXT NOT NULL,
    git_ref TEXT NOT NULL,
    commit_sha TEXT,
    filter TEXT,
    video TEXT NOT NULL,
    trace TEXT NOT NULL,
    requested_by INTEGER NOT NULL REFERENCES user (id),
    -- The scheduled task that asked for the job, when one did.
    task_id INTEGER,
    -- Who claimed it: the name it gave, and the person whose token it sent. Only that
    -- person's token may write to the job.
    runner_id TEXT,
    runner_name TEXT,
    runner_user_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    run_id INTEGER REFERENCES e2e_run (id) ON DELETE SET NULL,
    error TEXT,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    step TEXT,
    -- The number of the newest log line, and the newest one open pages were told about.
    log_seq INTEGER NOT NULL DEFAULT 0,
    log_announced_seq INTEGER NOT NULL DEFAULT 0,
    log_announced_at TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    -- When the runner last wrote to the job; a runner that goes quiet fails it.
    heard_at TEXT
);

CREATE INDEX e2e_job_by_project ON e2e_job (project_id, id);
CREATE INDEX e2e_job_by_state ON e2e_job (state, id);

-- The newest lines of a job's output; older ones are dropped as new ones arrive.
CREATE TABLE e2e_job_log (
    job_id INTEGER NOT NULL REFERENCES e2e_job (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    text TEXT NOT NULL,
    PRIMARY KEY (job_id, seq)
) WITHOUT ROWID;

-- A file a run's report names, uploaded after the report. `path` is the report's own; the
-- bytes are on disk under <data>/e2e-artifacts/<run id>/<stored>, a name made here.
CREATE TABLE e2e_artifact (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES e2e_run (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    kind TEXT NOT NULL,
    size INTEGER NOT NULL,
    stored TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (run_id, path)
);

-- What the report says happened in the test, attempt by attempt, as JSON (see `session.rs`);
-- and how many recordings and screenshots it names, so a list can say without reading that.
ALTER TABLE e2e_result ADD COLUMN session TEXT;
ALTER TABLE e2e_result ADD COLUMN videos INTEGER NOT NULL DEFAULT 0;
ALTER TABLE e2e_result ADD COLUMN screenshots INTEGER NOT NULL DEFAULT 0;
"#,
];
