pub const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE gh_repo (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    owner TEXT NOT NULL COLLATE NOCASE,
    name TEXT NOT NULL COLLATE NOCASE,
    default_branch TEXT NOT NULL DEFAULT 'main',
    created_at TEXT NOT NULL,
    last_synced_at TEXT,
    last_event_at TEXT,
    sync_error TEXT,
    UNIQUE (owner, name)
);

CREATE TABLE gh_pr (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES gh_repo (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL,
    draft INTEGER NOT NULL DEFAULT 0,
    author TEXT,
    head_ref TEXT NOT NULL,
    head_sha TEXT,
    base_ref TEXT NOT NULL,
    url TEXT NOT NULL,
    review TEXT,
    checks TEXT,
    additions INTEGER,
    deletions INTEGER,
    created_at TEXT,
    updated_at TEXT NOT NULL,
    merged_at TEXT,
    closed_at TEXT,
    UNIQUE (repo_id, number)
);

CREATE INDEX gh_pr_by_sha ON gh_pr (head_sha);

CREATE TABLE gh_branch (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES gh_repo (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sha TEXT,
    url TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    UNIQUE (repo_id, name)
);

CREATE TABLE gh_commit (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES gh_repo (id) ON DELETE CASCADE,
    sha TEXT NOT NULL,
    message TEXT NOT NULL,
    author_name TEXT,
    author_login TEXT,
    author_email TEXT,
    url TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    UNIQUE (repo_id, sha)
);

-- Which issue a pull request, branch or commit belongs to.
CREATE TABLE gh_ref (
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    ref_id INTEGER NOT NULL,
    PRIMARY KEY (issue_id, kind, ref_id)
);

CREATE INDEX gh_ref_by_target ON gh_ref (kind, ref_id);

-- What happens to an issue when something happens to its code.
CREATE TABLE gh_rule (
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    trigger TEXT NOT NULL,
    status_id INTEGER NOT NULL REFERENCES status (id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, trigger)
);

CREATE TABLE gh_delivery (
    id INTEGER PRIMARY KEY,
    delivery_id TEXT UNIQUE,
    event TEXT NOT NULL,
    repo TEXT,
    received_at TEXT NOT NULL,
    outcome TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);

-- Where an issue's status and its code disagree; recomputed whenever either changes.
CREATE TABLE gh_drift (
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    PRIMARY KEY (issue_id, kind)
);
"#];
