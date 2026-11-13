//! Pages: the tree, saving with history, and everything shown on a page.

use std::collections::{HashMap, HashSet};

use axum::http::StatusCode;
use hub_app::access::project_access_by_id;
use hub_app::db::{query_all, query_one};
use hub_app::services::issues::{create_issue, rows_by_ids};
use hub_app::{ApiError, ApiResult, App, AuthUser, Tx};
use hub_core::diff::diff_lines;
use hub_core::keys::{extract_issue_keys, KeyCase};
use hub_core::text::{fold, truncate};
use hub_core::{Id, IssuePatch, NewIssue};
use rusqlite::types::Value;
use rusqlite::Connection;
use serde_json::json;

use crate::links::{self, parse_task_line, Resolver, DOCUMENTS, LINKS, MENTIONS};
use crate::spaces::{access_to, describe, space_by_id, visible_spaces, SpaceAccess, SpaceRecord};
use crate::types::{WikiBacklink, WikiDiff, WikiPage, WikiPageDetail, WikiPageIssue, WikiPageRef, WikiRevision, WikiRevisionInfo, WikiTreeNode};

const MAX_TITLE: usize = 255;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_MESSAGE: usize = 200;
const MAX_SUMMARY: usize = 255;
/// Deeper than any real tree; stops a walk should the data ever hold a cycle.
const MAX_DEPTH: usize = 100;

pub const LOCKED: &str = "This page mirrors Confluence; detach it to edit";

const PAGE_SELECT: &str = "SELECT p.id, p.space_id, s.key, p.parent_id, p.title, p.body, p.position, p.version, p.author_id, p.updated_by,
        p.created_at, p.updated_at, p.source_id, p.source_version, p.source_url, p.locked
    FROM wiki_page p JOIN wiki_space s ON s.id = p.space_id";

fn map_page(row: &rusqlite::Row) -> rusqlite::Result<WikiPage> {
    Ok(WikiPage {
        id: row.get(0)?,
        space_id: row.get(1)?,
        space_key: row.get(2)?,
        parent_id: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        position: row.get(6)?,
        version: row.get(7)?,
        author_id: row.get(8)?,
        updated_by: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        source_id: row.get(12)?,
        source_version: row.get(13)?,
        source_url: row.get(14)?,
        locked: row.get(15)?,
    })
}

fn map_node(row: &rusqlite::Row) -> rusqlite::Result<WikiTreeNode> {
    Ok(WikiTreeNode {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        title: row.get(2)?,
        position: row.get(3)?,
        updated_at: row.get(4)?,
        locked: row.get(5)?,
    })
}

pub fn load_page(connection: &Connection, id: Id) -> ApiResult<WikiPage> {
    query_one(connection, &format!("{PAGE_SELECT} WHERE p.id = ?1"), [id], map_page)?.ok_or_else(|| ApiError::not_found("Page"))
}

/// The page and the caller's access to its space. A page they may not see is reported as missing.
pub fn page_access(connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<(WikiPage, SpaceAccess)> {
    let page = load_page(connection, id)?;
    let space = space_by_id(connection, page.space_id)?.ok_or_else(|| ApiError::not_found("Page"))?;
    let access =
        access_to(connection, user, space).map_err(
            |error| {
                if error.status == StatusCode::NOT_FOUND {
                    ApiError::not_found("Page")
                } else {
                    error
                }
            },
        )?;

    Ok((page, access))
}

pub fn clean_title(title: &str) -> ApiResult<String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");

    if title.is_empty() {
        return Err(ApiError::bad_request("A page needs a title"));
    }

    if title.chars().count() > MAX_TITLE {
        return Err(ApiError::bad_request(format!("The title is longer than {MAX_TITLE} characters")));
    }

    Ok(title)
}

fn check_body(body: &str) -> ApiResult<()> {
    if body.len() > MAX_BODY_BYTES {
        return Err(ApiError::bad_request("The page is larger than 1 MB; split it into several pages"));
    }

    Ok(())
}

fn parent_of(connection: &Connection, id: Id) -> ApiResult<Option<Id>> {
    Ok(query_one(connection, "SELECT parent_id FROM wiki_page WHERE id = ?1", [id], |row| row.get::<_, Option<Id>>(0))?.flatten())
}

fn require_in_space(connection: &Connection, page_id: Id, space_id: Id) -> ApiResult<()> {
    let space: Option<Id> = query_one(connection, "SELECT space_id FROM wiki_page WHERE id = ?1", [page_id], |row| row.get(0))?;

    match space {
        Some(found) if found == space_id => Ok(()),
        Some(_) => Err(ApiError::bad_request("The parent page is in another space")),
        None => Err(ApiError::bad_request("The parent page does not exist")),
    }
}

fn next_position(connection: &Connection, space_id: Id, parent_id: Option<Id>) -> ApiResult<i64> {
    Ok(connection.query_row(
        "SELECT coalesce(max(position), -1) + 1 FROM wiki_page WHERE space_id = ?1 AND parent_id IS ?2",
        (space_id, parent_id),
        |row| row.get(0),
    )?)
}

pub fn index_page(connection: &Connection, id: Id, title: &str, body: &str) -> ApiResult<()> {
    connection.execute("DELETE FROM wiki_fts WHERE rowid = ?1", [id])?;
    connection.execute("INSERT INTO wiki_fts (rowid, title, body) VALUES (?1, ?2, ?3)", (id, fold(title), fold(body)))?;

    Ok(())
}

fn add_revision(tx: &Tx, page_id: Id, version: i64, title: &str, body: &str, message: &str) -> ApiResult<()> {
    tx.execute(
        "INSERT INTO wiki_revision (page_id, version, title, body, author_id, author_label, message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        (page_id, version, title, body, tx.actor.user_id, &tx.actor.label, truncate(message.trim(), MAX_MESSAGE), &tx.now),
    )?;

    Ok(())
}

/// Where an imported page came from.
pub struct Source<'a> {
    pub id: &'a str,
    pub version: i64,
    pub url: Option<&'a str>,
}

/// Writes a new page with its first revision and search entry; links are the caller's business.
pub fn insert_page(tx: &Tx, space_id: Id, parent_id: Option<Id>, title: &str, body: &str, source: Option<&Source>) -> ApiResult<Id> {
    tx.execute(
        "INSERT INTO wiki_page (space_id, parent_id, title, body, position, version, author_id, updated_by, created_at, updated_at,
                                source_id, source_version, source_url, locked)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6, ?7, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            space_id,
            parent_id,
            title,
            body,
            next_position(tx, space_id, parent_id)?,
            tx.actor.user_id,
            tx.now,
            source.map(|source| source.id),
            source.map(|source| source.version),
            source.and_then(|source| source.url),
            source.is_some(),
        ],
    )?;
    let id = tx.last_insert_rowid();

    add_revision(tx, id, 1, title, body, "")?;
    index_page(tx, id, title, body)?;

    Ok(id)
}

/// Overwrites a page with a new version: the row, a revision, the search entry.
pub fn write_version(tx: &Tx, page: &WikiPage, title: &str, body: &str, message: &str) -> ApiResult<()> {
    let version = page.version + 1;

    tx.execute(
        "UPDATE wiki_page SET title = ?1, body = ?2, version = ?3, updated_by = ?4, updated_at = ?5 WHERE id = ?6",
        (title, body, version, tx.actor.user_id, &tx.now, page.id),
    )?;
    add_revision(tx, page.id, version, title, body, message)?;
    index_page(tx, page.id, title, body)
}

fn announce(tx: &Tx, kind: &str, space: &SpaceRecord, page_id: Id, title: &str, change: Option<&str>) -> ApiResult<()> {
    let mut payload = json!({ "pageId": page_id, "spaceKey": space.key, "title": title });

    if let Some(change) = change {
        payload["change"] = json!(change);
    }

    tx.emit(kind, space.project_id, None, payload)
}

/// Creates a page at the end of its siblings. The caller has already checked that the actor
/// may edit the space.
pub fn create_page(tx: &Tx, space: &SpaceRecord, title: &str, body: &str, parent_id: Option<Id>) -> ApiResult<Id> {
    let title = clean_title(title)?;
    check_body(body)?;

    if let Some(parent_id) = parent_id {
        require_in_space(tx, parent_id, space.id)?;
    }

    let id = insert_page(tx, space.id, parent_id, &title, body, None)?;
    let mut resolver = Resolver::new(tx)?;

    links::sync_page(tx, &mut resolver, id, space.id, body)?;
    links::refresh_referrers(tx, &mut resolver, id, &[&title])?;
    announce(tx, "page.created", space, id, &title, None)?;

    Ok(id)
}

/// A new version of `page` for people: history, search, links, and the event.
fn save(tx: &Tx, space: &SpaceRecord, page: &WikiPage, title: &str, body: &str, message: &str, change: &str) -> ApiResult<()> {
    write_version(tx, page, title, body, message)?;

    let mut resolver = Resolver::new(tx)?;
    links::sync_page(tx, &mut resolver, page.id, page.space_id, body)?;

    if title != page.title {
        links::refresh_referrers(tx, &mut resolver, page.id, &[&page.title, title])?;
    }

    announce(tx, "page.updated", space, page.id, title, Some(change))
}

fn require_unlocked(page: &WikiPage) -> ApiResult<()> {
    if page.locked {
        Err(ApiError::conflict(LOCKED))
    } else {
        Ok(())
    }
}

fn require_current(page: &WikiPage, base_version: i64) -> ApiResult<()> {
    if base_version == page.version {
        Ok(())
    } else {
        Err(ApiError::conflict(format!("Someone else saved version {} of this page; reload it to see their changes", page.version)))
    }
}

pub struct PageEdit<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub base_version: i64,
    pub message: &'a str,
}

/// Saves an edit as a new version; an edit that changes nothing saves nothing.
pub fn update_page(tx: &Tx, space: &SpaceRecord, page: &WikiPage, edit: &PageEdit) -> ApiResult<()> {
    require_unlocked(page)?;
    require_current(page, edit.base_version)?;

    let title = edit.title.map(clean_title).transpose()?.unwrap_or_else(|| page.title.clone());
    let body = edit.body.unwrap_or(&page.body);
    check_body(body)?;

    if title == page.title && body == page.body {
        return Ok(());
    }

    save(tx, space, page, &title, body, edit.message, "edited")
}

pub fn restore_page(tx: &Tx, space: &SpaceRecord, page: &WikiPage, version: i64) -> ApiResult<()> {
    require_unlocked(page)?;
    let revision = load_revision(tx, page.id, version)?;

    if revision.info.title == page.title && revision.body == page.body {
        return Ok(());
    }

    save(tx, space, page, &revision.info.title, &revision.body, &format!("Restored version {version}"), "restored")
}

/// Moves a page under `parent_id` (or to the top level), to index `position` among its new siblings.
pub fn move_page(tx: &Tx, space: &SpaceRecord, page: &WikiPage, parent_id: Option<Id>, position: i64) -> ApiResult<()> {
    if page.locked {
        return Err(ApiError::conflict("This page mirrors Confluence, which decides where it sits; detach it to move it"));
    }

    if let Some(parent_id) = parent_id {
        require_in_space(tx, parent_id, page.space_id)?;
        let mut cursor = Some(parent_id);

        for _ in 0..MAX_DEPTH {
            let Some(current) = cursor else { break };

            if current == page.id {
                return Err(ApiError::bad_request("A page cannot be moved under itself or one of its own descendants"));
            }

            cursor = parent_of(tx, current)?;
        }
    }

    let mut siblings: Vec<Id> = query_all(
        tx,
        "SELECT id FROM wiki_page WHERE space_id = ?1 AND parent_id IS ?2 AND id != ?3 ORDER BY position, id",
        (page.space_id, parent_id, page.id),
        |row| row.get(0),
    )?;
    let index = usize::try_from(position.max(0)).unwrap_or(0).min(siblings.len());
    siblings.insert(index, page.id);

    tx.execute("UPDATE wiki_page SET parent_id = ?1 WHERE id = ?2", (parent_id, page.id))?;

    for (index, sibling) in siblings.iter().enumerate() {
        tx.execute("UPDATE wiki_page SET position = ?1 WHERE id = ?2", (index as i64, sibling))?;
    }

    announce(tx, "page.updated", space, page.id, &page.title, Some("moved"))
}

/// Puts a mirrored page under the parent its source names, at the end of the siblings. A
/// parent that sits under the page itself (possible once a detached page is in between)
/// would close a loop, so the page then goes to the top level.
pub fn place(tx: &Tx, space_id: Id, page_id: Id, parent_id: Option<Id>) -> ApiResult<()> {
    let mut parent_id = parent_id;
    let mut cursor = parent_id;

    for _ in 0..MAX_DEPTH {
        let Some(current) = cursor else { break };

        if current == page_id {
            parent_id = None;
            break;
        }

        cursor = parent_of(tx, current)?;
    }

    if parent_of(tx, page_id)? != parent_id {
        tx.execute(
            "UPDATE wiki_page SET parent_id = ?1, position = ?2 WHERE id = ?3",
            (parent_id, next_position(tx, space_id, parent_id)?, page_id),
        )?;
    }

    Ok(())
}

fn descendants(connection: &Connection, page: &WikiPage) -> ApiResult<Vec<Id>> {
    let rows: Vec<(Id, Option<Id>)> =
        query_all(connection, "SELECT id, parent_id FROM wiki_page WHERE space_id = ?1", [page.space_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut children: HashMap<Id, Vec<Id>> = HashMap::new();

    for (id, parent_id) in rows {
        if let Some(parent_id) = parent_id {
            children.entry(parent_id).or_default().push(id);
        }
    }

    let mut found = Vec::new();
    let mut seen: HashSet<Id> = HashSet::from([page.id]);
    let mut queue = vec![page.id];

    while let Some(current) = queue.pop() {
        for child in children.get(&current).into_iter().flatten() {
            if seen.insert(*child) {
                found.push(*child);
                queue.push(*child);
            }
        }
    }

    Ok(found)
}

/// Deletes a page; with `recursive`, everything under it too. Returns how many pages went.
pub fn delete_page(tx: &Tx, space: &SpaceRecord, page: &WikiPage, recursive: bool) -> ApiResult<usize> {
    let mut doomed = descendants(tx, page)?;

    if !doomed.is_empty() && !recursive {
        return Err(ApiError::conflict("This page has pages under it; delete them too, or move them first"));
    }

    doomed.push(page.id);

    for id in &doomed {
        links::forget_pages(tx, "SELECT ?1 AS id", [id])?;
        tx.execute("DELETE FROM wiki_fts WHERE rowid = ?1", [id])?;
        tx.execute("DELETE FROM wiki_page WHERE id = ?1", [id])?;
    }

    tx.emit("page.deleted", space.project_id, None, json!({ "pageId": page.id, "spaceKey": space.key, "title": page.title, "pages": doomed.len() }))?;

    Ok(doomed.len())
}

/// Cuts an imported page loose from its source: it becomes an ordinary page, and the source
/// is remembered so that the next import leaves it alone.
pub fn detach_page(tx: &Tx, space: &SpaceRecord, page: &WikiPage) -> ApiResult<()> {
    if let Some(source_id) = &page.source_id {
        tx.execute("INSERT OR REPLACE INTO wiki_detached (space_id, source_id, page_id) VALUES (?1, ?2, ?3)", (page.space_id, source_id, page.id))?;
    }

    tx.execute("UPDATE wiki_page SET locked = 0, source_id = NULL, source_version = NULL, source_url = NULL WHERE id = ?1", [page.id])?;

    announce(tx, "page.updated", space, page.id, &page.title, Some("detached"))
}

/// Turns the given task lines of the body into issues of `project_key` and writes each key
/// into its line. The caller has already checked that the actor may edit both the space and
/// the project.
pub fn tasks_to_issues(
    tx: &Tx,
    space: &SpaceRecord,
    page: &WikiPage,
    project_key: &str,
    type_id: Option<Id>,
    lines: &[usize],
) -> ApiResult<Vec<String>> {
    require_unlocked(page)?;

    let mut wanted: Vec<usize> = lines.to_vec();
    wanted.sort_unstable();
    wanted.dedup();

    if wanted.is_empty() {
        return Err(ApiError::bad_request("Choose at least one task"));
    }

    let projects: HashSet<String> = query_all(tx, "SELECT key FROM project", [], |row| row.get::<_, String>(0))?.into_iter().collect();
    let mut body_lines: Vec<String> = page.body.split('\n').map(String::from).collect();
    let mut keys = Vec::new();
    let origin = format!(
        "Created from a task on the knowledge base page [{}](/wiki/{}/{}).",
        page.title.replace('[', "\\[").replace(']', "\\]"),
        space.key,
        page.id
    );

    for index in wanted {
        let number = index + 1;
        let line = body_lines.get(index).cloned().ok_or_else(|| ApiError::bad_request(format!("The page has no line {number}")))?;
        let task =
            parse_task_line(&line).filter(|task| !task.done).ok_or_else(|| ApiError::bad_request(format!("Line {number} is not an open task")))?;
        let first_word = task.text.split_whitespace().next().unwrap_or("");

        if let Some(existing) = extract_issue_keys(first_word, &projects, KeyCase::Exact).first() {
            return Err(ApiError::bad_request(format!("Line {number} already belongs to {existing}")));
        }

        let new = NewIssue {
            project_key: project_key.to_string(),
            fields: IssuePatch { summary: Some(truncate(task.text, MAX_SUMMARY)), description: Some(origin.clone()), type_id, ..Default::default() },
        };
        let issue_id = create_issue(tx, &new)?;
        let key: String = tx.query_row("SELECT key FROM issue WHERE id = ?1", [issue_id], |row| row.get(0))?;

        body_lines[index] = format!("{}{key} {}{}", task.prefix, task.text, task.suffix);
        keys.push(key);
    }

    save(tx, space, page, &page.title, &body_lines.join("\n"), &format!("Created {} from tasks", keys.join(", ")), "edited")?;

    Ok(keys)
}

/// The pages of a space as a flat list in reading order: each page is followed by its subtree.
pub fn tree(connection: &Connection, space_id: Id) -> ApiResult<Vec<WikiTreeNode>> {
    let nodes = query_all(
        connection,
        "SELECT id, parent_id, title, position, updated_at, locked FROM wiki_page WHERE space_id = ?1 ORDER BY position, id",
        [space_id],
        map_node,
    )?;
    let ids: HashSet<Id> = nodes.iter().map(|node| node.id).collect();
    let mut children: HashMap<Option<Id>, Vec<usize>> = HashMap::new();

    for (index, node) in nodes.iter().enumerate() {
        children.entry(node.parent_id.filter(|parent| ids.contains(parent))).or_default().push(index);
    }

    let mut ordered = Vec::with_capacity(nodes.len());
    let mut seen: HashSet<Id> = HashSet::new();
    let mut stack: Vec<usize> = children.get(&None).map(|roots| roots.iter().rev().copied().collect()).unwrap_or_default();

    while let Some(index) = stack.pop() {
        let node = &nodes[index];

        if !seen.insert(node.id) {
            continue;
        }

        ordered.push(node.clone());
        stack.extend(children.get(&Some(node.id)).into_iter().flatten().rev());
    }

    Ok(ordered)
}

fn breadcrumbs(connection: &Connection, page: &WikiPage) -> ApiResult<Vec<WikiPageRef>> {
    let mut crumbs = Vec::new();
    let mut cursor = page.parent_id;

    for _ in 0..MAX_DEPTH {
        let Some(id) = cursor else { break };
        let Some((title, parent_id)) = query_one(connection, "SELECT title, parent_id FROM wiki_page WHERE id = ?1", [id], |row| {
            Ok((row.get(0)?, row.get::<_, Option<Id>>(1)?))
        })?
        else {
            break;
        };

        crumbs.push(WikiPageRef { id, title });
        cursor = parent_id;
    }

    crumbs.reverse();

    Ok(crumbs)
}

/// The issues a page mentions or documents, limited to the projects the viewer may see.
fn linked_issues(app: &App, connection: &Connection, user: &AuthUser, page_id: Id) -> ApiResult<Vec<WikiPageIssue>> {
    let edges: Vec<(String, String)> = query_all(
        connection,
        "SELECT to_id, relation FROM link WHERE from_kind = 'page' AND from_id = ?1 AND to_kind = 'issue' AND relation IN (?2, ?3)",
        (page_id.to_string(), MENTIONS, DOCUMENTS),
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut relations: HashMap<Id, String> = HashMap::new();

    for (issue_id, relation) in edges {
        let Ok(issue_id) = issue_id.parse::<Id>() else { continue };

        if relation == DOCUMENTS || !relations.contains_key(&issue_id) {
            relations.insert(issue_id, relation);
        }
    }

    let ids: Vec<Id> = relations.keys().copied().collect();
    let mut rows = rows_by_ids(app, connection, &ids)?;
    let mut visible: HashMap<Id, bool> = HashMap::new();

    for row in &rows {
        visible.entry(row.project_id).or_insert_with(|| project_access_by_id(connection, user, row.project_id).is_ok());
    }

    rows.retain(|row| visible.get(&row.project_id).copied().unwrap_or(false));
    rows.sort_by(|left, right| (&left.project_key, left.number).cmp(&(&right.project_key, right.number)));

    Ok(rows.into_iter().filter_map(|issue| relations.get(&issue.id).cloned().map(|relation| WikiPageIssue { relation, issue })).collect())
}

fn backlinks(connection: &Connection, user: &AuthUser, page_id: Id) -> ApiResult<Vec<WikiBacklink>> {
    let (visible, mut params) = visible_spaces(user, "s");
    params.insert(0, Value::Text(page_id.to_string()));
    params.insert(1, Value::Text(LINKS.to_string()));

    query_all(
        connection,
        &format!(
            "SELECT p.id, s.key, p.title FROM link l
             JOIN wiki_page p ON p.id = CAST(l.from_id AS INTEGER) JOIN wiki_space s ON s.id = p.space_id
             WHERE l.to_kind = 'page' AND l.to_id = ? AND l.from_kind = 'page' AND l.relation = ? AND {visible}
             ORDER BY p.title COLLATE NOCASE, p.id"
        ),
        rusqlite::params_from_iter(params),
        |row| Ok(WikiBacklink { page_id: row.get(0)?, space_key: row.get(1)?, title: row.get(2)? }),
    )
}

/// Everything shown on a page, for a viewer who may see it.
pub fn page_detail(app: &App, connection: &Connection, user: &AuthUser, id: Id) -> ApiResult<WikiPageDetail> {
    let (page, access) = page_access(connection, user, id)?;
    let children = query_all(
        connection,
        "SELECT id, parent_id, title, position, updated_at, locked FROM wiki_page WHERE parent_id = ?1 ORDER BY position, id",
        [id],
        map_node,
    )?;

    Ok(WikiPageDetail {
        space: describe(connection, &access)?,
        breadcrumbs: breadcrumbs(connection, &page)?,
        children,
        issues: linked_issues(app, connection, user, id)?,
        backlinks: backlinks(connection, user, id)?,
        page,
    })
}

fn map_revision_info(row: &rusqlite::Row) -> rusqlite::Result<WikiRevisionInfo> {
    Ok(WikiRevisionInfo {
        version: row.get(0)?,
        title: row.get(1)?,
        author_id: row.get(2)?,
        author_label: row.get(3)?,
        message: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub fn list_revisions(connection: &Connection, page_id: Id) -> ApiResult<Vec<WikiRevisionInfo>> {
    query_all(
        connection,
        "SELECT version, title, author_id, author_label, message, created_at FROM wiki_revision WHERE page_id = ?1 ORDER BY version DESC",
        [page_id],
        map_revision_info,
    )
}

pub fn load_revision(connection: &Connection, page_id: Id, version: i64) -> ApiResult<WikiRevision> {
    query_one(
        connection,
        "SELECT version, title, author_id, author_label, message, created_at, body FROM wiki_revision WHERE page_id = ?1 AND version = ?2",
        (page_id, version),
        |row| Ok(WikiRevision { info: map_revision_info(row)?, body: row.get(6)? }),
    )?
    .ok_or_else(|| ApiError::not_found("Version"))
}

/// The line diff between two versions; by default the latest against the one before it.
pub fn diff(connection: &Connection, page: &WikiPage, from: Option<i64>, to: Option<i64>) -> ApiResult<WikiDiff> {
    let to = to.unwrap_or(page.version);
    let from = from.unwrap_or((to - 1).max(1));
    let old = load_revision(connection, page.id, from)?;
    let new = load_revision(connection, page.id, to)?;

    Ok(WikiDiff { from, to, lines: diff_lines(&old.body, &new.body), from_title: old.info.title, to_title: new.info.title })
}
