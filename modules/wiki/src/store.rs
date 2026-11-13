//! What importing modules may do to the knowledge base. A module that mirrors another
//! system (Confluence) writes spaces and pages through these functions, never by hand, so
//! history, search and links stay right.
//!
//! An import runs in three steps: `upsert_space`, then `upsert_imported_page` for every page
//! with parents before their children, then `refresh_links` once for the pages written.

use std::collections::HashMap;

use hub_app::db::query_all;
use hub_app::{ApiError, ApiResult, Tx};
use hub_core::text::truncate;
use hub_core::Id;
use rusqlite::Connection;

use crate::links::{self, Resolver};
use crate::pages::{self, Source};
use crate::spaces::{clean_key, create_space, NewSpace};
pub use crate::spaces::{space_access, space_by_key, spaces_by_source, SpaceAccess, SpaceRecord};

const MAX_TITLE: usize = 255;

pub struct ImportedSpace<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    /// The system the space mirrors, e.g. `confluence`.
    pub source: &'a str,
    /// Whatever the importer needs to run the same import again.
    pub source_ref: &'a serde_json::Value,
}

/// Whether `key` could be the key of a space.
pub fn is_space_key(key: &str) -> bool {
    clean_key(key).is_ok()
}

/// The space an import writes into: created on the first run, and on later runs given the
/// new `source_ref`. A space that exists but mirrors something else, or nothing, is refused.
pub fn upsert_space(tx: &Tx, space: &ImportedSpace) -> ApiResult<SpaceRecord> {
    let Some(existing) = space_by_key(tx, space.key)? else {
        return create_space(
            tx,
            &NewSpace {
                key: space.key,
                name: space.name,
                description: space.description,
                project_id: None,
                source: space.source,
                source_ref: Some(space.source_ref),
            },
        );
    };

    if existing.source != space.source {
        return Err(ApiError::conflict(format!(
            "The space {} already exists and is not a {} import; choose another key",
            existing.key, space.source
        )));
    }

    tx.execute("UPDATE wiki_space SET source_ref = ?1 WHERE id = ?2", (space.source_ref.to_string(), existing.id))?;

    Ok(SpaceRecord { source_ref: Some(space.source_ref.clone()), ..existing })
}

/// What is already here from the source: the stored version of each mirrored page, by its
/// id in the source. An importer uses it to skip converting pages that have not changed.
pub fn imported_versions(connection: &Connection, space_id: Id) -> ApiResult<HashMap<String, i64>> {
    let rows = query_all(
        connection,
        "SELECT source_id, coalesce(source_version, 0) FROM wiki_page WHERE space_id = ?1 AND source_id IS NOT NULL",
        [space_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    )?;

    Ok(rows.into_iter().collect())
}

/// Source ids of pages that were detached and now live their own life, with the page each became.
pub fn detached_sources(connection: &Connection, space_id: Id) -> ApiResult<HashMap<String, Id>> {
    let rows = query_all(connection, "SELECT source_id, page_id FROM wiki_detached WHERE space_id = ?1", [space_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Id>(1)?))
    })?;

    Ok(rows.into_iter().collect())
}

/// The mirrored pages of a space as (page id, source id, title).
pub fn imported_pages(connection: &Connection, space_id: Id) -> ApiResult<Vec<(Id, String, String)>> {
    query_all(
        connection,
        "SELECT id, source_id, title FROM wiki_page WHERE space_id = ?1 AND source_id IS NOT NULL ORDER BY title COLLATE NOCASE, id",
        [space_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
}

pub struct ImportedPage<'a> {
    pub source_id: &'a str,
    pub source_version: i64,
    pub source_url: Option<&'a str>,
    pub title: &'a str,
    /// Markdown. Not read when the stored version equals `source_version`.
    pub body: &'a str,
    /// The page (in this space) the page sits under.
    pub parent_id: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportOutcome {
    /// New here: stored locked, as version 1.
    Created,
    /// The source has a newer version: stored as a new revision.
    Updated,
    /// Same version as stored: content and history are not touched. Only its place in the
    /// tree follows the source, which can move a page without giving it a new version.
    Unchanged,
    /// Detached here earlier: not touched, and never will be.
    Detached,
}

fn imported_title(title: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");

    if title.is_empty() {
        "Untitled".to_string()
    } else {
        truncate(&title, MAX_TITLE)
    }
}

/// Stores one page of a mirrored space, matched by its id in the source, and says what
/// happened. The revision is signed with the transaction's actor (`Actor::system("Confluence")`).
/// The returned id is the local page, so the importer can hang children under it.
pub fn upsert_imported_page(tx: &Tx, space: &SpaceRecord, page: &ImportedPage) -> ApiResult<(Id, ImportOutcome)> {
    let detached: Option<Id> =
        hub_app::db::query_one(tx, "SELECT page_id FROM wiki_detached WHERE space_id = ?1 AND source_id = ?2", (space.id, page.source_id), |row| {
            row.get(0)
        })?;

    if let Some(page_id) = detached {
        return Ok((page_id, ImportOutcome::Detached));
    }

    let existing: Option<(Id, Option<i64>)> = hub_app::db::query_one(
        tx,
        "SELECT id, source_version FROM wiki_page WHERE space_id = ?1 AND source_id = ?2",
        (space.id, page.source_id),
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let title = imported_title(page.title);
    let source = Source { id: page.source_id, version: page.source_version, url: page.source_url };

    let Some((id, stored_version)) = existing else {
        return Ok((pages::insert_page(tx, space.id, page.parent_id, &title, page.body, Some(&source))?, ImportOutcome::Created));
    };

    if stored_version == Some(page.source_version) {
        pages::place(tx, space.id, id, page.parent_id)?;

        return Ok((id, ImportOutcome::Unchanged));
    }

    let current = pages::load_page(tx, id)?;

    pages::write_version(tx, &current, &title, page.body, &format!("Version {} from {}", page.source_version, space.source))?;
    tx.execute("UPDATE wiki_page SET source_version = ?1, source_url = ?2 WHERE id = ?3", (page.source_version, page.source_url, id))?;
    pages::place(tx, space.id, id, page.parent_id)?;

    Ok((id, ImportOutcome::Updated))
}

/// Re-reads the issue mentions and `[[Title]]` links of the given pages. Called once after
/// all pages of an import are stored, so links between them resolve whatever their order.
pub fn refresh_links(tx: &Tx, page_ids: &[Id]) -> ApiResult<()> {
    let mut resolver = Resolver::new(tx)?;

    for page_id in page_ids {
        let (space_id, body): (Id, String) =
            tx.query_row("SELECT space_id, body FROM wiki_page WHERE id = ?1", [page_id], |row| Ok((row.get(0)?, row.get(1)?)))?;

        links::sync_page(tx, &mut resolver, *page_id, space_id, &body)?;
    }

    Ok(())
}
