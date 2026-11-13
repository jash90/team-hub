//! What a page's body refers to: issues by key, other pages by `[[Title]]`. The edges live in
//! the shared `link` table so the issue page and other modules can follow them.

use std::collections::{HashMap, HashSet};

use hub_app::db::query_all;
use hub_app::{ApiResult, Tx};
use hub_core::keys::{extract_issue_keys, is_project_key, KeyCase};
use hub_core::text::fold;
use hub_core::Id;
use rusqlite::Connection;

pub const MENTIONS: &str = "mentions";
pub const DOCUMENTS: &str = "documents";
pub const LINKS: &str = "links";

/// The target of a `[[...]]` link as written.
#[derive(Debug, Clone, PartialEq)]
pub struct WikiLink {
    /// Everything before the `|`, for when the prefix turns out not to be a space.
    pub whole: String,
    /// The `SPACE` of `[[SPACE:Title]]`, when the prefix has the shape of a space key.
    pub space: Option<String>,
    pub title: String,
}

/// `[[Title]]`, `[[SPACE:Title]]` and `[[Title|shown text]]` in a Markdown body.
pub fn wiki_links(body: &str) -> Vec<WikiLink> {
    let mut found = Vec::new();
    let mut rest = body;

    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let inner = &after[..end];

        if inner.contains('\n') || inner.contains("[[") {
            rest = after;
            continue;
        }

        rest = &after[end + 2..];
        let whole = inner.split('|').next().unwrap_or("").trim();

        if whole.is_empty() {
            continue;
        }

        let (space, title) = match whole.split_once(':') {
            Some((prefix, title)) if is_project_key(prefix.trim()) && !title.trim().is_empty() => (Some(prefix.trim().to_string()), title.trim()),
            _ => (None, whole),
        };

        found.push(WikiLink {
            whole: whole.to_string(),
            space,
            title: title.to_string(),
        });
    }

    found
}

/// A Markdown task-list line, split so the text can be rewritten in place.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskLine<'a> {
    /// Indentation, bullet and checkbox, up to and including the space before the text.
    pub prefix: &'a str,
    pub done: bool,
    pub text: &'a str,
    /// A carriage return the line ended with, kept so the file's line endings survive.
    pub suffix: &'a str,
}

/// `- [ ] text` (also `*`, `+` and `1.` bullets, indented or not).
pub fn parse_task_line(line: &str) -> Option<TaskLine<'_>> {
    let (line, suffix) = match line.strip_suffix('\r') {
        Some(stripped) => (stripped, "\r"),
        None => (line, ""),
    };
    let indent = line.len() - line.trim_start().len();
    let after_indent = &line[indent..];
    let bullet = match after_indent.chars().next()? {
        '-' | '*' | '+' => 1,
        first if first.is_ascii_digit() => {
            let digits = after_indent.chars().take_while(char::is_ascii_digit).count();

            if !matches!(after_indent[digits..].chars().next(), Some('.' | ')')) {
                return None;
            }

            digits + 1
        }
        _ => return None,
    };
    let after_bullet = &after_indent[bullet..];
    let gap = after_bullet.len() - after_bullet.trim_start_matches(' ').len();
    let checkbox = &after_bullet[gap..];

    if gap == 0 || checkbox.len() < 4 {
        return None;
    }

    let done = match &checkbox.as_bytes()[..3] {
        b"[ ]" => false,
        b"[x]" | b"[X]" => true,
        _ => return None,
    };
    let after_checkbox = &checkbox[3..];
    let text = after_checkbox.trim_start_matches(' ');

    if text.len() == after_checkbox.len() || text.trim().is_empty() {
        return None;
    }

    Some(TaskLine {
        prefix: &line[..line.len() - text.len()],
        done,
        text: text.trim_end(),
        suffix,
    })
}

fn title_key(title: &str) -> String {
    fold(title).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Looks up what links point at. Built once per save (or once per import), after the rows it
/// should see have been written.
pub struct Resolver<'a> {
    connection: &'a Connection,
    pub projects: HashSet<String>,
    spaces: HashMap<String, Id>,
    titles: HashMap<Id, HashMap<String, Id>>,
}

impl<'a> Resolver<'a> {
    pub fn new(connection: &'a Connection) -> ApiResult<Self> {
        let projects = query_all(connection, "SELECT key FROM project", [], |row| row.get::<_, String>(0))?
            .into_iter()
            .collect();
        let spaces = query_all(connection, "SELECT key, id FROM wiki_space", [], |row| {
            Ok((row.get::<_, String>(0)?, row.get(1)?))
        })?
        .into_iter()
        .collect();

        Ok(Self {
            connection,
            projects,
            spaces,
            titles: HashMap::new(),
        })
    }

    fn titles_of(&mut self, space_id: Id) -> ApiResult<&HashMap<String, Id>> {
        if !self.titles.contains_key(&space_id) {
            // Newest first, so that of two pages with one title the older one wins.
            let rows = query_all(
                self.connection,
                "SELECT title, id FROM wiki_page WHERE space_id = ?1 ORDER BY id DESC",
                [space_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Id>(1)?)),
            )?;
            self.titles
                .insert(space_id, rows.into_iter().map(|(title, id)| (title_key(&title), id)).collect());
        }

        Ok(&self.titles[&space_id])
    }

    fn page(&mut self, own_space: Id, link: &WikiLink) -> ApiResult<Option<Id>> {
        if let Some(space_id) = link.space.as_ref().and_then(|key| self.spaces.get(key).copied()) {
            return Ok(self.titles_of(space_id)?.get(&title_key(&link.title)).copied());
        }

        Ok(self.titles_of(own_space)?.get(&title_key(&link.whole)).copied())
    }
}

/// Makes the rows of one relation from a page equal to `wanted`.
fn replace(tx: &Tx, page_id: Id, to_kind: &str, relation: &str, wanted: &[Id]) -> ApiResult<()> {
    let from = page_id.to_string();
    let existing: Vec<String> = query_all(
        tx,
        "SELECT to_id FROM link WHERE from_kind = 'page' AND from_id = ?1 AND to_kind = ?2 AND relation = ?3",
        (&from, to_kind, relation),
        |row| row.get(0),
    )?;
    let wanted: Vec<String> = wanted.iter().map(Id::to_string).collect();

    for stale in existing.iter().filter(|id| !wanted.contains(id)) {
        tx.execute(
            "DELETE FROM link WHERE from_kind = 'page' AND from_id = ?1 AND to_kind = ?2 AND to_id = ?3 AND relation = ?4",
            (&from, to_kind, stale, relation),
        )?;
    }

    for new in wanted.iter().filter(|id| !existing.contains(id)) {
        tx.execute(
            "INSERT OR IGNORE INTO link (from_kind, from_id, to_kind, to_id, relation, created_at) VALUES ('page', ?1, ?2, ?3, ?4, ?5)",
            (&from, to_kind, new, relation, &tx.now),
        )?;
    }

    Ok(())
}

/// Brings the `mentions` and `links` edges of a page in line with its body.
pub fn sync_page(tx: &Tx, resolver: &mut Resolver, page_id: Id, space_id: Id, body: &str) -> ApiResult<()> {
    let mut issues: Vec<Id> = Vec::new();

    for key in extract_issue_keys(body, &resolver.projects, KeyCase::Exact) {
        if let Some((issue_id, _)) = hub_app::services::issues::issue_id_by_key(tx, &key)? {
            issues.push(issue_id);
        }
    }

    let mut pages: Vec<Id> = Vec::new();

    for link in wiki_links(body) {
        if let Some(target) = resolver.page(space_id, &link)? {
            if target != page_id && !pages.contains(&target) {
                pages.push(target);
            }
        }
    }

    replace(tx, page_id, "issue", MENTIONS, &issues)?;
    replace(tx, page_id, "page", LINKS, &pages)
}

/// Re-reads the links of the pages whose body may name one of `titles`: a page created or
/// renamed can turn a dead `[[Title]]` elsewhere into a live one, or the reverse.
pub fn refresh_referrers(tx: &Tx, resolver: &mut Resolver, page_id: Id, titles: &[&str]) -> ApiResult<()> {
    let mut referrers: Vec<Id> = Vec::new();

    for title in titles {
        let words: Vec<String> = fold(title)
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(String::from)
            .collect();

        if words.is_empty() {
            continue;
        }

        let phrase = format!("body : \"{}\"", words.join(" "));
        let found: Vec<Id> = query_all(tx, "SELECT rowid FROM wiki_fts WHERE wiki_fts MATCH ?1 LIMIT 500", [phrase], |row| {
            row.get(0)
        })?;

        referrers.extend(found.into_iter().filter(|id| *id != page_id));
    }

    referrers.sort_unstable();
    referrers.dedup();

    for referrer in referrers {
        let (space_id, body): (Id, String) = tx.query_row("SELECT space_id, body FROM wiki_page WHERE id = ?1", [referrer], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;

        if body.contains("[[") {
            sync_page(tx, resolver, referrer, space_id, &body)?;
        }
    }

    Ok(())
}

pub fn set_documents(tx: &Tx, page_id: Id, issue_id: Id, linked: bool) -> ApiResult<()> {
    if linked {
        tx.execute(
            "INSERT OR IGNORE INTO link (from_kind, from_id, to_kind, to_id, relation, created_at) VALUES ('page', ?1, 'issue', ?2, ?3, ?4)",
            (page_id.to_string(), issue_id.to_string(), DOCUMENTS, &tx.now),
        )?;
    } else {
        tx.execute(
            "DELETE FROM link WHERE from_kind = 'page' AND from_id = ?1 AND to_kind = 'issue' AND to_id = ?2 AND relation = ?3",
            (page_id.to_string(), issue_id.to_string(), DOCUMENTS),
        )?;
    }

    Ok(())
}

/// Removes every edge from or to the pages selected by `pages`, a subquery yielding page ids.
pub fn forget_pages(connection: &Connection, pages: &str, params: impl rusqlite::Params + Clone) -> ApiResult<()> {
    connection.execute(
        &format!("DELETE FROM link WHERE from_kind = 'page' AND from_id IN (SELECT CAST(id AS TEXT) FROM ({pages}))"),
        params.clone(),
    )?;
    connection.execute(
        &format!("DELETE FROM link WHERE to_kind = 'page' AND to_id IN (SELECT CAST(id AS TEXT) FROM ({pages}))"),
        params,
    )?;

    Ok(())
}

/// Removes edges to issues that no longer exist, so a reused id never inherits them.
pub fn forget_deleted_issues(connection: &Connection) -> ApiResult<()> {
    connection.execute(
        "DELETE FROM link WHERE from_kind = 'page' AND to_kind = 'issue' AND CAST(to_id AS INTEGER) NOT IN (SELECT id FROM issue)",
        [],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_wiki_links_with_spaces_and_labels() {
        let links =
            wiki_links("See [[Płatności]] and [[TEAM:Zasady pracy|the rules]], not [[ ]] or [[open\nline]]. Also [[Bramka kartowa: sandbox]].");

        assert_eq!(
            links,
            vec![
                WikiLink {
                    whole: "Płatności".into(),
                    space: None,
                    title: "Płatności".into()
                },
                WikiLink {
                    whole: "TEAM:Zasady pracy".into(),
                    space: Some("TEAM".into()),
                    title: "Zasady pracy".into()
                },
                WikiLink {
                    whole: "Bramka kartowa: sandbox".into(),
                    space: None,
                    title: "Bramka kartowa: sandbox".into()
                },
            ]
        );
    }

    #[test]
    fn an_unclosed_link_does_not_swallow_the_next_one() {
        assert_eq!(
            wiki_links("[[a [[Real]]").into_iter().map(|link| link.title).collect::<Vec<_>>(),
            vec!["Real"]
        );
    }

    #[test]
    fn recognises_task_lines() {
        let task = parse_task_line("  - [ ] Zamówić klucze sandboxa  \r").expect("a task");

        assert_eq!(
            task,
            TaskLine {
                prefix: "  - [ ] ",
                done: false,
                text: "Zamówić klucze sandboxa",
                suffix: "\r"
            }
        );
        assert!(parse_task_line("* [x] done").expect("a task").done);
        assert_eq!(parse_task_line("12. [ ] numbered").expect("a task").prefix, "12. [ ] ");

        for line in [
            "- [ ]",
            "- [ ]   ",
            "- [] nothing",
            "-[ ] glued",
            "text - [ ] inside",
            "- [ ]glued",
            "",
            "1 [ ] no dot",
            "- [y] other",
        ] {
            assert_eq!(parse_task_line(line), None, "{line:?}");
        }
    }
}
