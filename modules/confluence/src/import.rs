//! Importing a Confluence space into a knowledge base space, and running the same import
//! again later. Pages are matched by their Confluence id; the knowledge base decides what
//! an unchanged, changed or detached page means (`hub_mod_wiki::store`).

use std::collections::{HashMap, HashSet};

use hub_app::db::query_one;
use hub_app::{Actor, ApiError, ApiResult, App, AuthUser};
use hub_core::text::fold;
use hub_core::Id;
use hub_mod_wiki::store::{self, ImportOutcome, ImportedPage, ImportedSpace, SpaceRecord};
use rayon::prelude::*;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::client::{load_settings, Client, RemotePage};
use crate::convert::storage_to_markdown;
use crate::types::{ConfluenceFailure, ConfluenceGonePage, ConfluenceImport, ConfluenceImportReport};

pub const SOURCE: &str = "confluence";
const LABEL: &str = "Confluence";

/// What is stored on the knowledge base space (`source_ref`), enough to run the import again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportConfig {
    pub space_key: String,
    pub root_page_id: Option<String>,
    pub exclude: Vec<String>,
    pub base_url: String,
}

impl ImportConfig {
    pub fn of(space: &SpaceRecord) -> Option<Self> {
        space.source_ref.clone().and_then(|stored| serde_json::from_value(stored).ok())
    }
}

/// A page that will be stored, and the Confluence id of the page it goes under.
struct Planned {
    page: RemotePage,
    parent: Option<String>,
}

struct Plan {
    pages: Vec<Planned>,
    excluded: i64,
    /// Every page id Confluence listed for the space, kept or not.
    listed: HashSet<String>,
}

/// Chooses the pages to store: those under the root page if one is set, minus the excluded
/// ones with everything under them, parents before their children.
fn plan(config: &ImportConfig, remote: Vec<RemotePage>) -> ApiResult<Plan> {
    let listed: HashSet<String> = remote.iter().map(|page| page.id.clone()).collect();
    let root = config.root_page_id.as_deref().map(str::trim).filter(|root| !root.is_empty());

    if let Some(root) = root.filter(|root| !listed.contains(*root)) {
        return Err(ApiError::bad_request(format!("The Confluence space {} has no page with the id {root}", config.space_key)));
    }

    let exclude: HashSet<String> = config.exclude.iter().map(|entry| fold(entry.trim())).filter(|entry| !entry.is_empty()).collect();
    let named = |id: &str, title: &str| exclude.contains(id) || exclude.contains(&fold(title.trim()));
    let mut excluded = 0;
    let mut pages: Vec<Planned> = Vec::new();

    for page in remote {
        let in_scope = root.is_none_or(|root| page.id == root || page.ancestors.iter().any(|ancestor| ancestor.id == root));

        if !in_scope {
            continue;
        }

        if named(&page.id, &page.title) || page.ancestors.iter().any(|ancestor| named(&ancestor.id, &ancestor.title)) {
            excluded += 1;
            continue;
        }

        // The root page hangs under nothing here, whatever is above it in Confluence.
        let parent = page.ancestors.last().map(|ancestor| ancestor.id.clone()).filter(|_| root != Some(page.id.as_str()));
        pages.push(Planned { page, parent });
    }

    pages.sort_by_key(|planned| planned.page.ancestors.len());

    Ok(Plan { pages, excluded, listed })
}

fn convert(page: &RemotePage) -> Result<String, String> {
    let storage = page.storage.as_deref().ok_or("Confluence sent no body for this page")?;

    std::panic::catch_unwind(|| storage_to_markdown(storage)).map_err(|_| "The page could not be converted to Markdown".to_string())
}

/// The space the import writes into, when it exists; a space that is not a Confluence mirror
/// is refused before anything is fetched.
fn target_space(connection: &Connection, key: &str) -> ApiResult<Option<SpaceRecord>> {
    if !store::is_space_key(key) {
        return Err(ApiError::bad_request(format!(
            "\"{key}\" cannot be a space key here (2 to 10 letters and digits, starting with a letter); choose a target space key"
        )));
    }

    let space = store::space_by_key(connection, key)?;

    if let Some(other) = space.as_ref().filter(|space| space.source != SOURCE) {
        return Err(ApiError::conflict(format!(
            "The space {} already exists and is not a Confluence import; choose another target space key",
            other.key
        )));
    }

    Ok(space)
}

type Known = (HashMap<String, i64>, HashMap<String, Id>);

fn known(connection: &Connection, space: Option<&SpaceRecord>) -> ApiResult<Known> {
    match space {
        Some(space) => Ok((store::imported_versions(connection, space.id)?, store::detached_sources(connection, space.id)?)),
        None => Ok(Known::default()),
    }
}

fn needs_body(known: &Known, page: &RemotePage) -> bool {
    !known.1.contains_key(&page.id) && known.0.get(&page.id) != Some(&page.version)
}

/// Converts what changed (in parallel: it is computation) and writes everything in one transaction.
fn store_pages(
    app: &App,
    config: &ImportConfig,
    target_key: &str,
    space_name: Option<String>,
    remote: Vec<RemotePage>,
) -> ApiResult<ConfluenceImportReport> {
    let plan = plan(config, remote)?;
    let before = app.read(|connection| known(connection, target_space(connection, target_key)?.as_ref()))?;
    let mut converted: Vec<Option<Result<String, String>>> =
        plan.pages.par_iter().map(|planned| needs_body(&before, &planned.page).then(|| convert(&planned.page))).collect();

    app.write(Actor::system(LABEL), |tx| {
        target_space(tx, target_key)?;

        let space = store::upsert_space(
            tx,
            &ImportedSpace {
                key: target_key,
                name: space_name.as_deref().unwrap_or(&config.space_key),
                description: &format!("Imported from the Confluence space {}.", config.space_key),
                source: SOURCE,
                source_ref: &serde_json::to_value(config)?,
            },
        )?;
        // Read again inside the transaction: this is what the pages are compared against.
        let now = known(tx, Some(&space))?;
        let mut report = ConfluenceImportReport { wiki_space_key: space.key.clone(), skipped: plan.excluded, ..Default::default() };
        // Pages already here keep their place even if they cannot be stored this time.
        let mut local: HashMap<String, Id> = store::imported_pages(tx, space.id)?.into_iter().map(|(id, source_id, _)| (source_id, id)).collect();
        let mut written: Vec<Id> = Vec::new();

        for (planned, converted) in plan.pages.iter().zip(converted.iter_mut()) {
            let page = &planned.page;
            let body = match (needs_body(&now, page), converted.take()) {
                (false, _) => Ok(String::new()),
                (true, Some(body)) => body,
                (true, None) => convert(page),
            };
            let stored = body.and_then(|body| {
                let imported = ImportedPage {
                    source_id: &page.id,
                    source_version: page.version,
                    source_url: Some(&page.url),
                    title: &page.title,
                    body: &body,
                    parent_id: planned.parent.as_deref().and_then(|parent| local.get(parent).copied()),
                };

                store::upsert_imported_page(tx, &space, &imported).map_err(|error| error.message)
            });

            match stored {
                Ok((id, outcome)) => {
                    local.insert(page.id.clone(), id);

                    match outcome {
                        ImportOutcome::Created => report.created += 1,
                        ImportOutcome::Updated => report.updated += 1,
                        ImportOutcome::Unchanged => report.unchanged += 1,
                        ImportOutcome::Detached => report.skipped += 1,
                    }

                    if matches!(outcome, ImportOutcome::Created | ImportOutcome::Updated) {
                        written.push(id);
                    }
                }
                Err(reason) => report.failed.push(ConfluenceFailure { title: page.title.clone(), reason }),
            }
        }

        store::refresh_links(tx, &written)?;

        report.gone = store::imported_pages(tx, space.id)?
            .into_iter()
            .filter(|(_, source_id, _)| !plan.listed.contains(source_id))
            .map(|(page_id, _, title)| ConfluenceGonePage { page_id, title })
            .collect();

        tx.execute(
            "INSERT INTO confluence_import (space_id, last_run_at, last_report) VALUES (?1, ?2, ?3)
             ON CONFLICT (space_id) DO UPDATE SET last_run_at = ?2, last_report = ?3",
            (space.id, &tx.now, serde_json::to_string(&report)?),
        )?;
        tx.emit(
            "confluence.imported",
            None,
            None,
            json!({
                "wikiSpaceKey": space.key,
                "spaceKey": config.space_key,
                "created": report.created,
                "updated": report.updated,
                "unchanged": report.unchanged,
                "skipped": report.skipped,
                "failed": report.failed.len(),
            }),
        )?;

        Ok(report)
    })
}

/// Runs an import: checks on the worker pool, fetches on a blocking thread, converts and
/// stores on the worker pool again.
pub async fn run(app: &App, mut config: ImportConfig, target_key: String) -> ApiResult<ConfluenceImportReport> {
    config.space_key = config.space_key.trim().to_string();

    if config.space_key.is_empty() {
        return Err(ApiError::bad_request("Name the Confluence space to import"));
    }

    let target_key = target_key.trim().to_uppercase();
    let key = target_key.clone();
    let (client, exists) = app
        .run(move |app| {
            app.read(|connection| {
                let exists = target_space(connection, &key)?.is_some();

                Ok((Client::new(&load_settings(connection, &app.config.secret_key)?)?, exists))
            })
        })
        .await?;

    config.base_url = client.base().to_string();

    let space_key = config.space_key.clone();
    let (remote, space_name) = tokio::task::spawn_blocking(move || {
        let pages = client.pages(&space_key)?;
        // The name only matters when the space is created, and the import works without it.
        let name = if exists { None } else { client.space_name(&space_key).ok().flatten() };

        Ok::<_, ApiError>((pages, name))
    })
    .await
    .map_err(|error| ApiError::internal(format!("the import stopped: {error}")))??;

    app.run(move |app| store_pages(app, &config, &target_key, space_name, remote)).await
}

/// The imports the viewer may see, with the outcome of their last run.
pub fn list(connection: &Connection, user: &AuthUser) -> ApiResult<Vec<ConfluenceImport>> {
    let mut imports = Vec::new();

    for space in store::spaces_by_source(connection, SOURCE)? {
        if store::space_access(connection, user, &space.key).is_err() {
            continue;
        }

        let config = ImportConfig::of(&space).unwrap_or_default();
        let last: Option<(String, String)> =
            query_one(connection, "SELECT last_run_at, last_report FROM confluence_import WHERE space_id = ?1", [space.id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;

        imports.push(ConfluenceImport {
            wiki_space_key: space.key,
            wiki_space_name: space.name,
            space_key: config.space_key,
            root_page_id: config.root_page_id,
            exclude: config.exclude,
            base_url: config.base_url,
            last_report: last.as_ref().and_then(|(_, report)| serde_json::from_str(report).ok()),
            last_run_at: last.map(|(at, _)| at),
        });
    }

    Ok(imports)
}

/// The stored configuration of the import behind a knowledge base space.
pub fn stored_config(connection: &Connection, wiki_space_key: &str) -> ApiResult<(String, ImportConfig)> {
    let space =
        store::space_by_key(connection, wiki_space_key)?.filter(|space| space.source == SOURCE).ok_or_else(|| ApiError::not_found("Import"))?;
    let config = ImportConfig::of(&space).filter(|config| !config.space_key.is_empty()).ok_or_else(|| ApiError::not_found("Import"))?;

    Ok((space.key, config))
}
