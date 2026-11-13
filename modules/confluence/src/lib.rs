//! Imports Confluence spaces (Cloud and Server/Data Center) into the knowledge base as a
//! read-only mirror, and brings them up to date on request. `README.md` lists the routes.

mod client;
pub mod convert;
mod import;
mod routes;
pub mod types;

use axum::Router;
use hub_app::{App, Module, ModuleMeta};

pub struct ConfluenceModule;

const MIGRATIONS: &[&str] = &[r#"
-- The outcome of the last run of each import; the configuration itself lives on the space.
CREATE TABLE confluence_import (
    space_id INTEGER PRIMARY KEY REFERENCES wiki_space (id) ON DELETE CASCADE,
    last_run_at TEXT NOT NULL,
    last_report TEXT NOT NULL
);
"#];

impl Module for ConfluenceModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: "confluence",
            name: "Confluence import",
            description: "Imports Confluence spaces into the knowledge base and keeps the copy up to date.",
            version: env!("CARGO_PKG_VERSION"),
            depends_on: &["wiki"],
            default_enabled: true,
        }
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(routes::router())
    }
}
