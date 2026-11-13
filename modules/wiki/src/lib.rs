//! The knowledge base: spaces of Markdown pages in a tree, with history, search, and links
//! to the issues the pages talk about. `README.md` lists the routes.

mod links;
mod pages;
mod routes;
mod search;
mod seed;
mod spaces;
pub mod store;
mod templates;
mod tools;
pub mod types;

use axum::Router;
use hub_app::db::query_all;
use hub_app::{ApiResult, App, AuthUser, Event, Module, ModuleMeta, ToolSpec};
use hub_core::{IssueRow, QuickHit};
use rusqlite::types::Value;
use rusqlite::Connection;

use crate::links::DOCUMENTS;
use crate::spaces::visible_spaces;
use crate::types::{WikiIssuePage, WikiIssuePanel};

pub struct WikiModule;

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE wiki_space (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    project_id INTEGER REFERENCES project (id) ON DELETE SET NULL,
    source TEXT NOT NULL DEFAULT 'local',
    source_ref TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE wiki_page (
    id INTEGER PRIMARY KEY,
    space_id INTEGER NOT NULL REFERENCES wiki_space (id) ON DELETE CASCADE,
    parent_id INTEGER REFERENCES wiki_page (id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    author_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    updated_by INTEGER REFERENCES user (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    source_id TEXT,
    source_version INTEGER,
    source_url TEXT,
    locked INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX wiki_page_by_parent ON wiki_page (space_id, parent_id, position);
CREATE INDEX wiki_page_by_parent_only ON wiki_page (parent_id);
CREATE UNIQUE INDEX wiki_page_by_source ON wiki_page (space_id, source_id) WHERE source_id IS NOT NULL;

CREATE TABLE wiki_revision (
    id INTEGER PRIMARY KEY,
    page_id INTEGER NOT NULL REFERENCES wiki_page (id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    author_id INTEGER REFERENCES user (id) ON DELETE SET NULL,
    author_label TEXT,
    message TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    UNIQUE (page_id, version)
);

-- Imported pages that were detached: the importer skips their source from then on.
CREATE TABLE wiki_detached (
    space_id INTEGER NOT NULL REFERENCES wiki_space (id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    page_id INTEGER NOT NULL REFERENCES wiki_page (id) ON DELETE CASCADE,
    PRIMARY KEY (space_id, source_id)
);

-- Holds folded text (hub_core::text::fold); rowid is the page id.
CREATE VIRTUAL TABLE wiki_fts USING fts5 (title, body, tokenize = 'unicode61');
"#];

impl Module for WikiModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: "wiki",
            name: "Knowledge base",
            description: "Spaces of Markdown pages with history and search, linked to the issues they describe.",
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

    fn on_event(&self, app: &App, event: &Event) {
        if event.kind != "issue.deleted" {
            return;
        }

        if let Err(error) = app.db.write(|tx| links::forget_deleted_issues(tx)) {
            tracing::warn!("could not drop page links of a deleted issue: {}", error.message);
        }
    }

    /// The pages that mention or document the issue, newest first.
    fn issue_panel(&self, _app: &App, connection: &Connection, issue: &IssueRow, viewer: &AuthUser) -> ApiResult<Option<serde_json::Value>> {
        let (visible, mut params) = visible_spaces(viewer, "s");
        params.insert(0, Value::Text(issue.id.to_string()));

        let mut pages: Vec<WikiIssuePage> = Vec::new();
        let rows = query_all(
            connection,
            &format!(
                "SELECT p.id, s.key, p.title, l.relation, p.updated_at FROM link l
                 JOIN wiki_page p ON p.id = CAST(l.from_id AS INTEGER) JOIN wiki_space s ON s.id = p.space_id
                 WHERE l.to_kind = 'issue' AND l.to_id = ? AND l.from_kind = 'page' AND l.relation IN ('mentions', 'documents') AND {visible}
                 ORDER BY p.updated_at DESC, p.id"
            ),
            rusqlite::params_from_iter(params),
            |row| {
                Ok(WikiIssuePage { page_id: row.get(0)?, space_key: row.get(1)?, title: row.get(2)?, relation: row.get(3)?, updated_at: row.get(4)? })
            },
        )?;

        // A page that both mentions and documents the issue is listed once, as documenting it.
        for row in rows {
            match pages.iter_mut().find(|page| page.page_id == row.page_id) {
                Some(page) if row.relation == DOCUMENTS => page.relation = row.relation,
                Some(_) => {}
                None => pages.push(row),
            }
        }

        Ok(Some(serde_json::to_value(WikiIssuePanel { pages })?))
    }

    fn quick_search(&self, connection: &Connection, viewer: &AuthUser, text: &str) -> ApiResult<Vec<QuickHit>> {
        search::quick_hits(connection, viewer, text)
    }

    fn tools(&self) -> Vec<ToolSpec> {
        tools::specs()
    }

    fn call_tool(&self, app: &App, user: &AuthUser, name: &str, arguments: &serde_json::Value) -> Option<ApiResult<serde_json::Value>> {
        tools::call(app, user, name, arguments)
    }

    fn seed_demo(&self, app: &App) -> ApiResult<()> {
        seed::seed_demo(app)
    }
}
