//! Full-text search over pages. The index holds folded text (no diacritics, lower case), so
//! excerpts are cut from the page itself, using the positions the index reports.

use hub_app::db::query_all;
use hub_app::services::search::fts_match;
use hub_app::{ApiResult, AuthUser};
use hub_core::text::truncate;
use hub_core::{Id, QuickHit};
use rusqlite::types::Value;
use rusqlite::Connection;

use crate::spaces::visible_spaces;
use crate::types::WikiSearchHit;

const OPEN: char = '\u{1}';
const CLOSE: char = '\u{2}';
const MARK_OPEN: &str = "<mark>";
const MARK_CLOSE: &str = "</mark>";
const BEFORE: usize = 60;
const LENGTH: usize = 220;
const MAX_HITS: i64 = 30;

/// The text without the markers, and the marked ranges as character positions in it.
fn marked_ranges(highlighted: &str) -> (Vec<char>, Vec<(usize, usize)>) {
    let mut text = Vec::with_capacity(highlighted.len());
    let mut ranges = Vec::new();
    let mut open = None;

    for character in highlighted.chars() {
        match character {
            OPEN => open = Some(text.len()),
            CLOSE => {
                if let Some(start) = open.take() {
                    ranges.push((start, text.len()));
                }
            }
            other => text.push(other),
        }
    }

    (text, ranges)
}

/// An excerpt of `body` around the first match. `highlighted` is the indexed (folded) body
/// with the matches between markers; folding keeps positions except for a few rare letters,
/// and for those the excerpt is cut from the folded text instead.
fn excerpt(body: &str, highlighted: &str) -> String {
    let (folded, ranges) = marked_ranges(highlighted);
    let original: Vec<char> = body.chars().collect();
    let text = if original.len() == folded.len() { original } else { folded };
    let Some(&(first, _)) = ranges.first() else {
        return truncate(&text.iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "), LENGTH);
    };

    let mut start = first.saturating_sub(BEFORE);

    while start > 0 && start < first && !text[start - 1].is_whitespace() {
        start += 1;
    }

    let end = (start + LENGTH).min(text.len());
    let mut snippet = String::new();

    if start > 0 {
        snippet.push('…');
    }

    for (index, character) in text.iter().enumerate().take(end).skip(start) {
        if ranges.iter().any(|(from, to)| *from == index && *to <= end) {
            snippet.push_str(MARK_OPEN);
        }

        snippet.push(if character.is_whitespace() { ' ' } else { *character });

        if ranges.iter().any(|(from, to)| *to == index + 1 && *from >= start) {
            snippet.push_str(MARK_CLOSE);
        }
    }

    if end < text.len() {
        snippet.push('…');
    }

    let mut collapsed = String::with_capacity(snippet.len());

    for word in snippet.split(' ').filter(|word| !word.is_empty()) {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }

        collapsed.push_str(word);
    }

    collapsed
}

/// Pages matching every word of `text` (as prefixes), best first, in the spaces `user` may see.
pub fn search_pages(connection: &Connection, user: &AuthUser, text: &str, space_key: Option<&str>) -> ApiResult<Vec<WikiSearchHit>> {
    let Some(expression) = fts_match(text, None) else {
        return Ok(Vec::new());
    };
    let (visible, visible_params) = visible_spaces(user, "s");
    let mut params = vec![Value::Text(expression)];
    params.extend(visible_params);
    let mut condition = String::new();

    if let Some(key) = space_key.map(str::trim).filter(|key| !key.is_empty()) {
        condition.push_str(" AND s.key = ? COLLATE NOCASE");
        params.push(Value::Text(key.to_string()));
    }

    params.push(Value::Integer(MAX_HITS));

    let rows: Vec<(Id, String, String, String, String)> = query_all(
        connection,
        &format!(
            "SELECT p.id, s.key, p.title, p.body, highlight(wiki_fts, 1, char(1), char(2))
             FROM wiki_fts JOIN wiki_page p ON p.id = wiki_fts.rowid JOIN wiki_space s ON s.id = p.space_id
             WHERE wiki_fts MATCH ? AND {visible}{condition}
             ORDER BY bm25(wiki_fts, 8.0, 1.0), p.updated_at DESC LIMIT ?"
        ),
        rusqlite::params_from_iter(params),
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;

    Ok(rows
        .into_iter()
        .map(|(page_id, space_key, title, body, highlighted)| WikiSearchHit { page_id, space_key, title, snippet: excerpt(&body, &highlighted) })
        .collect())
}

pub fn quick_hits(connection: &Connection, user: &AuthUser, text: &str) -> ApiResult<Vec<QuickHit>> {
    let Some(expression) = fts_match(text, None) else {
        return Ok(Vec::new());
    };
    let (visible, visible_params) = visible_spaces(user, "s");
    let mut params = vec![Value::Text(expression)];
    params.extend(visible_params);

    query_all(
        connection,
        &format!(
            "SELECT p.id, s.key, p.title, s.name
             FROM wiki_fts JOIN wiki_page p ON p.id = wiki_fts.rowid JOIN wiki_space s ON s.id = p.space_id
             WHERE wiki_fts MATCH ? AND {visible}
             ORDER BY bm25(wiki_fts, 8.0, 1.0), p.updated_at DESC LIMIT 5"
        ),
        rusqlite::params_from_iter(params),
        |row| {
            let id: Id = row.get(0)?;
            let space_key: String = row.get(1)?;

            Ok(QuickHit {
                kind: "page".to_string(),
                id: id.to_string(),
                title: row.get(2)?,
                subtitle: row.get(3)?,
                url: format!("/wiki/{space_key}/{id}"),
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_excerpt_keeps_the_original_letters_and_marks_the_match() {
        let body = "Kanał powiadomień\n\nPłatność BLIK wymaga osobnego kanału.";
        let highlighted = "kanal powiadomien\n\n\u{1}platnosc\u{2} blik wymaga osobnego kanalu.";

        assert_eq!(excerpt(body, highlighted), "Kanał powiadomień <mark>Płatność</mark> BLIK wymaga osobnego kanału.");
    }

    #[test]
    fn a_long_body_is_cut_around_the_match_at_a_word() {
        let body = format!("{} sedno sprawy {}", "słowo ".repeat(40), "koniec ".repeat(60));
        let folded = hub_core::text::fold(&body).replace("sedno", "\u{1}sedno\u{2}");
        let snippet = excerpt(&body, &folded);

        assert!(snippet.starts_with("…słowo "), "{snippet}");
        assert!(snippet.contains("<mark>sedno</mark> sprawy"), "{snippet}");
        assert!(snippet.ends_with('…'), "{snippet}");
        assert!(snippet.chars().count() < 260, "{snippet}");
    }

    #[test]
    fn falls_back_to_the_indexed_text_when_folding_changed_its_length() {
        assert_eq!(excerpt("Straße frei", "\u{1}strasse\u{2} frei"), "<mark>strasse</mark> frei");
    }

    #[test]
    fn without_a_match_in_the_body_the_excerpt_is_its_beginning() {
        assert_eq!(excerpt("Pierwszy akapit.\n\nDrugi.", "pierwszy akapit.\n\ndrugi."), "Pierwszy akapit. Drugi.");
    }
}
