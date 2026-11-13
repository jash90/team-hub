//! Core tables. Migrations only ever get appended to.

pub const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE user (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE COLLATE NOCASE,
    name TEXT NOT NULL,
    password_hash TEXT,
    role TEXT NOT NULL DEFAULT 'member',
    active INTEGER NOT NULL DEFAULT 1,
    color TEXT NOT NULL,
    github_login TEXT COLLATE NOCASE,
    created_at TEXT NOT NULL
);

CREATE TABLE session (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    csrf TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE api_token (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    hint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_used_at TEXT
);

CREATE TABLE project (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    lead_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    issue_seq INTEGER NOT NULL DEFAULT 0,
    board_mode TEXT NOT NULL DEFAULT 'scrum',
    strict_workflow INTEGER NOT NULL DEFAULT 0,
    is_private INTEGER NOT NULL DEFAULT 0,
    archived INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE project_member (
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    PRIMARY KEY (project_id, user_id)
);

CREATE TABLE issue_type (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name TEXT NOT NULL COLLATE NOCASE,
    level TEXT NOT NULL,
    icon TEXT NOT NULL,
    position INTEGER NOT NULL,
    UNIQUE (project_id, name)
);

CREATE TABLE status (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name TEXT NOT NULL COLLATE NOCASE,
    category TEXT NOT NULL,
    position INTEGER NOT NULL,
    on_board INTEGER NOT NULL DEFAULT 1,
    wip_limit INTEGER,
    UNIQUE (project_id, name)
);

CREATE TABLE transition (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    from_status_id INTEGER REFERENCES status (id) ON DELETE CASCADE,
    to_status_id INTEGER NOT NULL REFERENCES status (id) ON DELETE CASCADE,
    name TEXT NOT NULL DEFAULT ''
);

CREATE TABLE component (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name TEXT NOT NULL COLLATE NOCASE,
    description TEXT NOT NULL DEFAULT '',
    lead_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    UNIQUE (project_id, name)
);

CREATE TABLE version (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name TEXT NOT NULL COLLATE NOCASE,
    description TEXT NOT NULL DEFAULT '',
    start_date TEXT,
    release_date TEXT,
    released INTEGER NOT NULL DEFAULT 0,
    released_at TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    UNIQUE (project_id, name)
);

CREATE TABLE sprint (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    goal TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'future',
    start_date TEXT,
    end_date TEXT,
    completed_at TEXT,
    position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE issue (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    key TEXT NOT NULL UNIQUE,
    type_id INTEGER NOT NULL REFERENCES issue_type (id),
    status_id INTEGER NOT NULL REFERENCES status (id),
    priority INTEGER NOT NULL DEFAULT 3,
    summary TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    assignee_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    reporter_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    parent_id INTEGER REFERENCES issue (id) ON DELETE SET NULL,
    sprint_id INTEGER REFERENCES sprint (id) ON DELETE SET NULL,
    version_id INTEGER REFERENCES version (id) ON DELETE SET NULL,
    points REAL,
    due_date TEXT,
    estimate_minutes INTEGER,
    rank TEXT NOT NULL,
    status_since TEXT NOT NULL,
    resolved_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (project_id, number)
);

CREATE INDEX issue_by_status ON issue (project_id, status_id);
CREATE INDEX issue_by_sprint ON issue (sprint_id);
CREATE INDEX issue_by_assignee ON issue (assignee_id);
CREATE INDEX issue_by_parent ON issue (parent_id);
CREATE INDEX issue_by_version ON issue (version_id);
CREATE INDEX issue_by_updated ON issue (updated_at);

CREATE TABLE issue_label (
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    label TEXT NOT NULL COLLATE NOCASE,
    PRIMARY KEY (issue_id, label)
);

CREATE TABLE issue_component (
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    component_id INTEGER NOT NULL REFERENCES component (id) ON DELETE CASCADE,
    PRIMARY KEY (issue_id, component_id)
);

CREATE TABLE issue_link (
    id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    target_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (source_id, target_id, type)
);

CREATE TABLE comment (
    id INTEGER PRIMARY KEY,
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    author_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    author_label TEXT,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX comment_by_issue ON comment (issue_id);

CREATE TABLE attachment (
    id INTEGER PRIMARY KEY,
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    path TEXT NOT NULL,
    uploader_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE watcher (
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    PRIMARY KEY (issue_id, user_id)
);

CREATE TABLE change (
    id INTEGER PRIMARY KEY,
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    actor_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    actor_label TEXT,
    field TEXT NOT NULL,
    old_value TEXT,
    new_value TEXT,
    old_ref TEXT,
    new_ref TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX change_by_issue ON change (issue_id);
CREATE INDEX change_by_field ON change (field, created_at);

CREATE TABLE worklog (
    id INTEGER PRIMARY KEY,
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    user_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    minutes INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT ''
);

CREATE TABLE filter (
    id INTEGER PRIMARY KEY,
    owner_id INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    query TEXT NOT NULL,
    shared INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE notification (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    issue_id INTEGER REFERENCES issue (id) ON DELETE CASCADE,
    actor_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL,
    read_at TEXT
);

CREATE INDEX notification_by_user ON notification (user_id, read_at);

-- Append-only journal of everything that happened; modules and the live stream read it.
CREATE TABLE event (
    id INTEGER PRIMARY KEY,
    at TEXT NOT NULL,
    actor_id INTEGER,
    actor_label TEXT,
    kind TEXT NOT NULL,
    project_id INTEGER,
    issue_id INTEGER,
    payload TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX event_by_project ON event (project_id, id);
CREATE INDEX event_by_issue ON event (issue_id, id);

-- Edges between things of any kind (issue, page, pull request, ...), so a new kind of
-- relation never needs a new table.
CREATE TABLE link (
    id INTEGER PRIMARY KEY,
    from_kind TEXT NOT NULL,
    from_id TEXT NOT NULL,
    to_kind TEXT NOT NULL,
    to_id TEXT NOT NULL,
    relation TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (from_kind, from_id, to_kind, to_id, relation)
);

CREATE INDEX link_by_target ON link (to_kind, to_id);

CREATE TABLE setting (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE secret (
    name TEXT PRIMARY KEY,
    nonce BLOB NOT NULL,
    sealed BLOB NOT NULL,
    hint TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE VIRTUAL TABLE issue_fts USING fts5 (key, summary, description, comments, tokenize = 'unicode61');
"#,
    r#"
-- Modules switched off in one project. No row means the module is on there, so a new
-- module or a new project needs nothing here.
CREATE TABLE project_module_off (
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    module_id TEXT NOT NULL,
    PRIMARY KEY (project_id, module_id)
) WITHOUT ROWID;
"#,
    r#"
-- Recurring work of a project (docs/schedule.md). What a task does is its kind's business,
-- and the kind belongs to a module: `config` is whatever that kind's form holds.
CREATE TABLE scheduled_task (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    config TEXT NOT NULL DEFAULT '{}',
    slot TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    run_as INTEGER NOT NULL REFERENCES user (id),
    state TEXT NOT NULL DEFAULT 'idle',
    last_started_at TEXT,
    last_finished_at TEXT,
    last_error TEXT,
    -- What the module said when it started work that is still going on.
    progress TEXT,
    -- The last attempt that worked; a failure leaves these three alone.
    result_at TEXT,
    result_summary TEXT,
    result_link TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX scheduled_task_by_project ON scheduled_task (project_id, id);

-- The day, in the schedule's time zone, a project's slot last ran for. A slot is due when
-- its time has passed today and this is not today yet.
CREATE TABLE schedule_slot_run (
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    slot TEXT NOT NULL,
    ran_for TEXT NOT NULL,
    ran_at TEXT NOT NULL,
    PRIMARY KEY (project_id, slot)
) WITHOUT ROWID;
"#,
    r#"
-- A desktop app asking to act as a person (docs/desktop-pairing.md). The app holds the
-- secret, the person sees the code; a row lives ten minutes at most and is gone once the
-- app has its token. `user_id` is who said yes, and is empty until somebody has.
CREATE TABLE desktop_pairing (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    secret_hash TEXT NOT NULL,
    name TEXT NOT NULL,
    runner_id TEXT NOT NULL DEFAULT '',
    user_id INTEGER REFERENCES user (id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
"#,
];
