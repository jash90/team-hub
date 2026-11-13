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
"#];
