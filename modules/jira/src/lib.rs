//! Imports Jira projects (Cloud and Server/Data Center, or a CSV export) with their issue
//! keys, history, comments, sprints and versions, and brings them up to date on request.
//! `routes.rs` lists the routes.

pub mod adf;
mod client;
pub mod csv;
mod import;
mod jobs;
mod mapping;
mod md;
mod remote;
mod routes;
pub mod types;
pub mod wiki;

use axum::Router;
use hub_app::db::query_one;
use hub_app::{ApiResult, App, AuthUser, Module, ModuleMeta};
use hub_core::IssueRow;
use rusqlite::Connection;

use crate::types::JiraIssuePanel;

pub struct JiraModule;

const MIGRATIONS: &[&str] = &[r#"
-- A project that was imported: where from, how its vocabulary was mapped, and the mark the
-- next import continues from.
CREATE TABLE jira_project (
    project_id INTEGER PRIMARY KEY REFERENCES project (id) ON DELETE CASCADE,
    remote_key TEXT NOT NULL,
    base_url TEXT NOT NULL DEFAULT '',
    cursor TEXT,
    last_import_at TEXT,
    mapping TEXT NOT NULL DEFAULT '{}'
);

-- An imported issue and its original. `fingerprint` is the state the import left; when the
-- issue no longer matches it, somebody has edited it here.
CREATE TABLE jira_issue (
    issue_id INTEGER PRIMARY KEY REFERENCES issue (id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    remote_key TEXT NOT NULL,
    self_url TEXT NOT NULL DEFAULT '',
    browse_url TEXT NOT NULL DEFAULT '',
    imported_at TEXT NOT NULL,
    remote_updated TEXT,
    fingerprint TEXT NOT NULL DEFAULT '',
    last_history INTEGER NOT NULL DEFAULT 0,
    history_through TEXT NOT NULL DEFAULT ''
);

CREATE INDEX jira_issue_by_key ON jira_issue (remote_key);

-- Comments and work logs that were imported, so a second import does not add them twice.
CREATE TABLE jira_item (
    issue_id INTEGER NOT NULL REFERENCES issue (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    local_id INTEGER NOT NULL,
    remote_updated TEXT,
    PRIMARY KEY (issue_id, kind, remote_id)
);

CREATE TABLE jira_sprint (
    project_id INTEGER NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    sprint_id INTEGER NOT NULL REFERENCES sprint (id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, remote_id)
);

-- Who a person of Jira is here.
CREATE TABLE jira_user (
    account TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    name TEXT NOT NULL DEFAULT ''
);

CREATE TABLE jira_job (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    project_key TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued',
    step TEXT NOT NULL DEFAULT 'waiting',
    total INTEGER,
    processed INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL DEFAULT 0,
    updated INTEGER NOT NULL DEFAULT 0,
    unchanged INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    failed INTEGER NOT NULL DEFAULT 0,
    -- The whole report, with the lists of skipped and failed issues and their reasons.
    report TEXT NOT NULL DEFAULT '{}',
    error TEXT,
    cancel INTEGER NOT NULL DEFAULT 0,
    started_by INTEGER REFERENCES user (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    finished_at TEXT
);

-- An uploaded CSV export, kept between its preview and its import.
CREATE TABLE jira_upload (
    id INTEGER PRIMARY KEY,
    filename TEXT NOT NULL,
    content TEXT NOT NULL,
    uploaded_by INTEGER REFERENCES user (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);
"#];

impl Module for JiraModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: "jira",
            name: "Jira import",
            description: "Brings projects over from Jira with their issue keys, history, comments, sprints and versions.",
            version: env!("CARGO_PKG_VERSION"),
            depends_on: &[],
            default_enabled: true,
        }
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(routes::router())
    }

    /// "Imported from Jira", with the way back to the original.
    fn issue_panel(&self, _app: &App, connection: &Connection, issue: &IssueRow, _viewer: &AuthUser) -> ApiResult<Option<serde_json::Value>> {
        let panel = query_one(
            connection,
            "SELECT remote_key, browse_url, imported_at, remote_updated FROM jira_issue WHERE issue_id = ?1",
            [issue.id],
            |row| {
                let url: String = row.get(1)?;

                Ok(JiraIssuePanel {
                    key: row.get(0)?,
                    url: Some(url).filter(|url| !url.is_empty()),
                    imported_at: row.get(2)?,
                    remote_updated: row.get(3)?,
                })
            },
        )?;

        Ok(panel.map(serde_json::to_value).transpose()?)
    }

    /// An import runs on a thread of the process that started it; one that was running when
    /// the server stopped is over.
    fn start(&self, app: &App) {
        match app.db.write(|tx| jobs::fail_interrupted(tx)) {
            Ok(0) => {}
            Ok(count) => tracing::warn!("{count} Jira import(s) were interrupted by the restart"),
            Err(error) => tracing::error!("could not look for interrupted Jira imports: {}", error.message),
        }
    }
}
