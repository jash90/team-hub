//! Jira wiki markup (what Jira Server and Data Center store, and what CSV exports contain) to
//! Markdown. A pure function of the text; markup it does not know stays as text.

use crate::md;

/// How far ahead a closing marker is looked for, so a long line of stray markers stays cheap.
const MAX_SPAN: usize = 2000;
/// Emphasis inside emphasis inside emphasis: deeper than this the text is taken as it is.
const MAX_NESTING: usize = 6;

pub fn wiki_to_markdown(source: &str) -> String {
    let source = source.replace("\r\n", "\n").replace('\r', "\n");

    render(&tokenize(&source), 0)
}

/// The source cut into lines of markup and whole code blocks, whose content is not markup.
#[derive(Debug, Clone, PartialEq)]
enum Line {
    Text(String),
    Code { language: String, body: String },
}

/// The next `{code…}` or `{noformat…}` tag: where it starts, where it ends, its name and parameters.
fn next_code_tag(text: &str, from: usize) -> Option<(usize, usize, &'static str, &str)> {
    let mut search = from;

    while let Some(offset) = text.get(search..)?.find('{') {
        let start = search + offset;
        let rest = &text[start + 1..];

        for name in ["code", "noformat"] {
            let Some(after) = rest.strip_prefix(name) else { continue };
            let Some(close) = after.find('}') else { continue };
            let params = &after[..close];

            if params.is_empty() || (params.starts_with(':') && !params.contains('\n') && !params.contains('{')) {
                return Some((start, start + 1 + name.len() + close + 1, name, params.trim_start_matches(':')));
            }
        }

        search = start + 1;
    }

    None
}

fn language_of(params: &str) -> String {
    params
        .split('|')
        .map(str::trim)
        .find_map(|part| match part.split_once('=') {
            Some((name, value)) if name.trim().eq_ignore_ascii_case("language") => Some(value.trim().to_string()),
            Some(_) => None,
            None if !part.is_empty() => Some(part.to_string()),
            None => None,
        })
        .unwrap_or_default()
}

/// Puts `{quote}` and `{panel…}` tags on lines of their own, wherever they were written.
fn split_tags(line: &str, lines: &mut Vec<Line>) {
    let mut rest = line;

    loop {
        let found = ["{quote}", "{panel"].iter().filter_map(|tag| rest.find(tag).map(|at| (at, *tag))).min();
        let Some((at, tag)) = found else { break };
        let end = if tag == "{quote}" { Some(at + tag.len()) } else { rest[at..].find('}').map(|close| at + close + 1) };
        let Some(end) = end else { break };

        if !rest[..at].trim().is_empty() {
            lines.push(Line::Text(rest[..at].to_string()));
        }

        lines.push(Line::Text(rest[at..end].to_string()));
        rest = &rest[end..];

        if rest.trim().is_empty() {
            return;
        }
    }

    lines.push(Line::Text(rest.to_string()));
}

fn tokenize(source: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut position = 0;

    while let Some((start, open_end, name, params)) = next_code_tag(source, position) {
        let before = &source[position..start];

        if !before.is_empty() {
            before.strip_suffix('\n').unwrap_or(before).split('\n').for_each(|line| split_tags(line, &mut lines));
        }

        let closing = format!("{{{name}}}");
        let (body, next) = match source[open_end..].find(&closing) {
            Some(offset) => (&source[open_end..open_end + offset], open_end + offset + closing.len()),
            None => (&source[open_end..], source.len()),
        };

        lines.push(Line::Code { language: if name == "code" { language_of(params) } else { String::new() }, body: body.to_string() });
        position = if source[next..].starts_with('\n') { next + 1 } else { next };
    }

    if position < source.len() {
        source[position..].split('\n').for_each(|line| split_tags(line, &mut lines));
    }

    lines
}

/// `h3. Title` → (3, "Title").
fn heading(line: &str) -> Option<(usize, &str)> {
    let rest = line.strip_prefix('h')?;
    let level = rest.chars().next()?.to_digit(10).filter(|level| (1..=6).contains(level))? as usize;
    let title = rest[1..].strip_prefix('.')?;

    (title.is_empty() || title.starts_with(' ')).then(|| (level, title.trim()))
}

/// `** item` → ("**", "item"). A dash is a bullet only on its own; `--` is a dash in prose.
fn list_item(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let length = trimmed.chars().take_while(|character| matches!(character, '*' | '#' | '-')).count();
    let (marker, rest) = trimmed.split_at(length);
    let valid = length > 0 && length <= 12 && (marker == "-" || !marker.contains('-'));

    (valid && rest.starts_with(' ') && !rest.trim().is_empty()).then(|| (marker, rest.trim()))
}

fn is_rule(line: &str) -> bool {
    line.len() >= 4 && line.chars().all(|character| character == '-')
}

fn tag_title(tag: &str) -> Option<String> {
    let params = tag.trim_start_matches("{panel").trim_end_matches('}').trim_start_matches(':');

    params.split('|').find_map(|part| part.trim().strip_prefix("title=")).map(|title| title.trim().to_string()).filter(|title| !title.is_empty())
}

/// Whether a line of markup starts something other than a paragraph.
fn starts_block(line: &str) -> bool {
    let trimmed = line.trim();

    trimmed.is_empty()
        || trimmed == "{quote}"
        || trimmed.starts_with("{panel")
        || trimmed.starts_with('|')
        || trimmed.starts_with("bq. ")
        || heading(trimmed).is_some()
        || is_rule(trimmed)
        || list_item(line).is_some()
}

fn render(lines: &[Line], depth: usize) -> String {
    let mut blocks: Vec<String> = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut index = 0;

    fn flush(paragraph: &mut Vec<String>, blocks: &mut Vec<String>) {
        if paragraph.is_empty() {
            return;
        }

        let text = paragraph.join("\n");
        let lines: Vec<String> = text.split('\n').map(str::trim_end).filter(|line| !line.trim().is_empty()).map(md::guard_line).collect();

        blocks.push(lines.join("\\\n"));
        paragraph.clear();
    }

    while index < lines.len() {
        let text = match &lines[index] {
            Line::Code { language, body } => {
                flush(&mut paragraph, &mut blocks);
                blocks.push(md::fence(language, body));
                index += 1;
                continue;
            }
            Line::Text(text) => text,
        };
        let trimmed = text.trim();

        if !starts_block(text) {
            paragraph.push(inline(trimmed, 0));
            index += 1;
            continue;
        }

        flush(&mut paragraph, &mut blocks);

        if trimmed.is_empty() {
            index += 1;
        } else if trimmed == "{quote}" || trimmed.starts_with("{panel") {
            let closing = if trimmed == "{quote}" { "{quote}" } else { "{panel}" };
            let end = lines[index + 1..]
                .iter()
                .position(|line| matches!(line, Line::Text(text) if text.trim() == closing))
                .map_or(lines.len(), |offset| index + 1 + offset);

            // A closing tag with nothing open is dropped.
            if trimmed != "{panel}" || end < lines.len() {
                let inner = if depth < MAX_NESTING { render(&lines[index + 1..end], depth + 1) } else { String::new() };
                let label = if closing == "{quote}" {
                    String::new()
                } else {
                    format!("**{}**", md::escape(&tag_title(trimmed).unwrap_or_else(|| "Note".to_string())))
                };

                blocks.push(md::quote(&md::join_blocks(vec![label, inner])));
                index = end;
            }

            index += 1;
        } else if let Some((level, title)) = heading(trimmed) {
            let title = inline(title, 0).replace('\n', " ");

            if !title.trim().is_empty() {
                blocks.push(format!("{} {}", "#".repeat(level), title.trim()));
            }

            index += 1;
        } else if let Some(quoted) = trimmed.strip_prefix("bq. ") {
            blocks.push(md::quote(&md::guard_line(&inline(quoted.trim(), 0).replace('\n', " "))));
            index += 1;
        } else if is_rule(trimmed) {
            blocks.push("---".to_string());
            index += 1;
        } else if trimmed.starts_with('|') {
            let end = lines[index..]
                .iter()
                .position(|line| !matches!(line, Line::Text(text) if text.trim().starts_with('|')))
                .map_or(lines.len(), |offset| index + offset);
            let rows: Vec<(Vec<String>, bool)> =
                lines[index..end].iter().filter_map(|line| if let Line::Text(text) = line { Some(row(text.trim())) } else { None }).collect();
            let has_header = rows.first().is_some_and(|(_, header)| *header);

            blocks.push(md::table(&rows.into_iter().map(|(cells, _)| cells).collect::<Vec<_>>(), has_header));
            index = end;
        } else {
            let (list, end) = list(lines, index);

            blocks.push(list);
            index = end;
        }
    }

    flush(&mut paragraph, &mut blocks);

    md::join_blocks(blocks)
}

/// The list that starts at `start`, and the index of the first line after it.
fn list(lines: &[Line], start: usize) -> (String, usize) {
    let mut out: Vec<String> = Vec::new();
    // For each open level: whether it is numbered, and how many items it has had.
    let mut levels: Vec<(bool, usize)> = Vec::new();
    let mut continuation = 0;
    let mut index = start;

    while let Some(Line::Text(text)) = lines.get(index) {
        if let Some((marker, body)) = list_item(text) {
            let kinds: Vec<bool> = marker.chars().map(|character| character == '#').collect();

            levels.truncate(kinds.len());

            for (level, numbered) in kinds.iter().enumerate() {
                match levels.get_mut(level) {
                    Some(open) if open.0 == *numbered => {}
                    Some(open) => *open = (*numbered, 0),
                    None => levels.push((*numbered, 0)),
                }
            }

            let last = levels.len() - 1;
            levels[last].1 += 1;

            let indent: usize = levels[..last].iter().map(|(numbered, count)| if *numbered { count.to_string().len() + 2 } else { 2 }).sum();
            let bullet = if levels[last].0 { format!("{}. ", levels[last].1) } else { "- ".to_string() };

            continuation = indent + bullet.len();
            out.push(format!("{}{bullet}{}", " ".repeat(indent), inline(body, 0).replace('\n', " ")));
        } else if !starts_block(text) && !out.is_empty() {
            // Text right under an item belongs to it.
            let last = out.len() - 1;
            out[last].push_str(&format!("\\\n{}{}", " ".repeat(continuation), inline(text.trim(), 0).replace('\n', " ")));
        } else {
            break;
        }

        index += 1;
    }

    (out.join("\n"), index)
}

/// The cells of a table row, and whether it is a header row (`||a||b||`).
fn row(line: &str) -> (Vec<String>, bool) {
    let characters: Vec<char> = line.chars().collect();
    let header = line.starts_with("||");
    let mut cells: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut nested = 0usize;
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];

        match character {
            '[' | '{' => nested += 1,
            ']' | '}' => nested = nested.saturating_sub(1),
            _ => {}
        }

        if character == '|' && nested == 0 {
            cells.push(std::mem::take(&mut cell));

            if characters.get(index + 1) == Some(&'|') {
                index += 1;
            }
        } else {
            cell.push(character);
        }

        index += 1;
    }

    if !cell.trim().is_empty() {
        cells.push(cell);
    }

    // The separator that opens the row leaves an empty first cell behind.
    (cells.iter().skip(1).map(|cell| inline(cell.trim(), 0).replace('\n', " ")).collect(), header)
}

fn starts_with(characters: &[char], index: usize, text: &str) -> bool {
    text.chars().enumerate().all(|(offset, wanted)| characters.get(index + offset) == Some(&wanted))
}

/// The index of `text` at or after `from`, on the same line and not too far away.
fn find(characters: &[char], from: usize, text: &str) -> Option<usize> {
    let limit = characters.len().min(from + MAX_SPAN);

    (from..limit).take_while(|index| characters[*index] != '\n').find(|index| starts_with(characters, *index, text))
}

fn is_linkable(target: &str) -> bool {
    let lower = target.to_lowercase();

    ["http://", "https://", "mailto:", "ftp://", "/", "#"].iter().any(|prefix| lower.starts_with(prefix)) && !target.contains(char::is_whitespace)
}

const EMOTICONS: [(&str, &str); 11] = [
    ("(/)", "✅"),
    ("(x)", "❌"),
    ("(!)", "⚠️"),
    ("(i)", "ℹ️"),
    ("(?)", "❓"),
    ("(y)", "👍"),
    ("(n)", "👎"),
    ("(on)", "💡"),
    ("(*)", "⭐"),
    (":)", "🙂"),
    (":(", "🙁"),
];

/// The text between a pair of emphasis markers that opens at `index`, and where it ends.
/// Jira only reads a marker as emphasis at the edge of a word.
fn delimited(characters: &[char], index: usize, marker: char) -> Option<(String, usize)> {
    // Superscript follows its word (x^2^); everything else opens at the edge of one.
    let before_ok = marker == '^' || index == 0 || !(characters[index - 1].is_alphanumeric() || characters[index - 1] == marker);
    let next = *characters.get(index + 1)?;

    if !before_ok || next.is_whitespace() || next == marker {
        return None;
    }

    let limit = characters.len().min(index + MAX_SPAN);
    let mut end = index + 2;

    while end < limit && characters[end] != '\n' {
        let closes = characters[end] == marker
            && !characters[end - 1].is_whitespace()
            && characters.get(end + 1).is_none_or(|after| !after.is_alphanumeric() && *after != marker);

        if closes {
            return Some((characters[index + 1..end].iter().collect(), end + 1));
        }

        end += 1;
    }

    None
}

/// Inline markup. A forced line break (`\\`) comes out as a newline for the caller to place.
fn inline(text: &str, nesting: usize) -> String {
    if nesting > MAX_NESTING {
        return md::escape(text);
    }

    let characters: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    let mut plain = String::new();
    let mut index = 0;

    macro_rules! emit {
        ($text:expr, $next:expr) => {{
            out.push_str(&md::escape(&std::mem::take(&mut plain)));
            out.push_str(&$text);
            index = $next;
            continue;
        }};
    }

    while index < characters.len() {
        let character = characters[index];
        let after_space = index == 0 || characters[index - 1].is_whitespace();

        if let Some(length) = md::url_at(&characters, index) {
            let url: String = characters[index..index + length].iter().collect();
            emit!(url, index + length);
        }

        match character {
            '\\' => match characters.get(index + 1) {
                Some('\\') => emit!("\n", index + 2),
                Some(next) if next.is_ascii_punctuation() => {
                    plain.push(*next);
                    index += 2;
                    continue;
                }
                _ => {}
            },
            '{' => {
                if starts_with(&characters, index, "{{") {
                    if let Some(end) = find(&characters, index + 2, "}}") {
                        let code: String = characters[index + 2..end].iter().collect();

                        if !code.trim().is_empty() {
                            emit!(md::code_span(code.trim()), end + 2);
                        }
                    }
                }

                // Colours and anchors have no counterpart; the text between them stays.
                if starts_with(&characters, index, "{color") || starts_with(&characters, index, "{anchor") {
                    if let Some(end) = find(&characters, index + 1, "}") {
                        emit!("", end + 1);
                    }
                }
            }
            '[' => {
                if let Some(end) = find(&characters, index + 1, "]") {
                    let inside: String = characters[index + 1..end].iter().collect();
                    let parts: Vec<&str> = inside.split('|').map(str::trim).collect();

                    if let Some(user) = inside.strip_prefix('~') {
                        let user = user.trim().trim_start_matches("accountid:");

                        if !user.is_empty() && !user.contains(char::is_whitespace) {
                            emit!(format!("@{user}"), end + 1);
                        }
                    } else if let Some(file) = inside.strip_prefix('^') {
                        emit!(format!("\\[attachment: {}\\]", md::escape(file.trim())), end + 1);
                    } else if parts.len() >= 2 && is_linkable(parts[1]) {
                        emit!(md::link(&inline(parts[0], nesting + 1).replace('\n', " "), parts[1]), end + 1);
                    } else if parts.len() == 1 && is_linkable(parts[0]) && !parts[0].starts_with('#') && !parts[0].starts_with('/') {
                        emit!(md::link(&md::escape(parts[0]), parts[0]), end + 1);
                    }
                }
            }
            '!' if after_space || !characters[index - 1].is_alphanumeric() => {
                if let Some(end) = find(&characters, index + 1, "!") {
                    let inside: String = characters[index + 1..end].iter().collect();
                    let name = inside.split('|').next().unwrap_or("").trim();
                    let is_file = name.rsplit_once('.').is_some_and(|(stem, extension)| {
                        !stem.is_empty() && (2..=5).contains(&extension.len()) && extension.chars().all(char::is_alphanumeric)
                    });

                    if !name.is_empty() && !name.contains(char::is_whitespace) && is_file {
                        if is_linkable(name) && !name.starts_with('/') {
                            emit!(format!("![]({})", md::link_target(name)), end + 1);
                        }

                        emit!(format!("\\[attachment: {}\\]", md::escape(name)), end + 1);
                    }
                }
            }
            '*' | '_' | '+' | '^' | '-' => {
                // A dash opens strike-through only after a space, or dates and ranges would count.
                let allowed = character != '-' || after_space;

                if let Some((inner, end)) = delimited(&characters, index, character).filter(|_| allowed) {
                    let inner = inline(&inner, nesting + 1);
                    let wrapped = match character {
                        '*' => md::wrap("**", &inner),
                        '_' => md::wrap("*", &inner),
                        '-' => md::wrap("~~", &inner),
                        _ => inner,
                    };

                    emit!(wrapped, end);
                }
            }
            '?' if starts_with(&characters, index, "??") => {
                if let Some(end) = find(&characters, index + 2, "??").filter(|end| *end > index + 2) {
                    let cited: String = characters[index + 2..end].iter().collect();
                    emit!(md::wrap("*", &inline(&cited, nesting + 1)), end + 2);
                }
            }
            '(' | ':' if after_space => {
                let found = EMOTICONS.iter().find(|(code, _)| starts_with(&characters, index, code));

                if let Some((code, emoji)) = found {
                    let end = index + code.chars().count();

                    if characters.get(end).is_none_or(|after| !after.is_alphanumeric()) {
                        emit!(*emoji, end);
                    }
                }
            }
            _ => {}
        }

        plain.push(character);
        index += 1;
    }

    out.push_str(&md::escape(&plain));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_paragraphs_and_line_breaks() {
        let markdown = wiki_to_markdown("h1. Title\r\nh3. Sub *title*\n\nFirst line\nsecond line\\\\third line\n\nh7. not a heading\nh2.also not");

        assert_eq!(markdown, "# Title\n\n### Sub **title**\n\nFirst line\\\nsecond line\\\nthird line\n\nh7. not a heading\\\nh2.also not");
    }

    #[test]
    fn inline_markup() {
        assert_eq!(wiki_to_markdown("*bold* _italic_ -gone- +under+ x^2^ ??cited??"), "**bold** *italic* ~~gone~~ under x2 *cited*");
        assert_eq!(wiki_to_markdown("*bold with _italic_ inside*"), "**bold with *italic* inside**");
        assert_eq!(wiki_to_markdown("{{let x = a*b;}} and {{}}"), "`let x = a*b;` and {{}}");
        assert_eq!(wiki_to_markdown("{color:red}urgent{color} and {color:#ff0000}*hot*{color}"), "urgent and **hot**");
    }

    #[test]
    fn markers_inside_words_are_not_markup() {
        assert_eq!(
            wiki_to_markdown("2 * 3 * 4, snake_case_name, a - b - c, 2024-01-02, well-known"),
            "2 \\* 3 \\* 4, snake_case_name, a - b - c, 2024-01-02, well-known"
        );
        assert_eq!(wiki_to_markdown("\\*not bold\\* and <b>"), "\\*not bold\\* and \\<b>");
        assert_eq!(wiki_to_markdown("wow! great! and 1 + 2 + 3"), "wow! great! and 1 + 2 + 3");
    }

    #[test]
    fn links_mentions_and_images() {
        assert_eq!(
            wiki_to_markdown("[the docs|https://example.com/a_b] and [https://example.com]"),
            "[the docs](https://example.com/a_b) and [https://example.com](https://example.com)"
        );
        assert_eq!(wiki_to_markdown("[*bold* link|https://example.com|a tip]"), "[**bold** link](https://example.com)");
        assert_eq!(wiki_to_markdown("cc [~jkowalski] and [~accountid:5b10ac8d]"), "cc @jkowalski and @5b10ac8d");
        assert_eq!(
            wiki_to_markdown("see [^spec.pdf], [WEB-12] and [not a link]"),
            "see \\[attachment: spec.pdf\\], \\[WEB-12\\] and \\[not a link\\]"
        );
        assert_eq!(
            wiki_to_markdown("!screenshot.png! !shot 2.png|thumbnail! !https://example.com/a.png|width=200!"),
            "\\[attachment: screenshot.png\\] !shot 2.png|thumbnail! ![](https://example.com/a.png)"
        );
        assert_eq!(wiki_to_markdown("bare https://example.com/a_b*c, done"), "bare https://example.com/a_b*c, done");
        assert_eq!(
            wiki_to_markdown("[mail|mailto:a@example.com] [x|javascript:alert(1)]"),
            "[mail](mailto:a@example.com) \\[x|javascript:alert(1)\\]"
        );
    }

    #[test]
    fn code_and_noformat_blocks_are_not_markup() {
        let markdown = wiki_to_markdown("Before\n{code:java}\nint x = a * b; // *not bold*\n{code}\nAfter {noformat}h1. raw{noformat} end\n{code:title=Foo.py|language=python}print('x'){code}\n{code}\nnever closed *x*");

        assert_eq!(
            markdown,
            "Before\n\n```java\nint x = a * b; // *not bold*\n```\n\nAfter\n\n```\nh1. raw\n```\n\nend\n\n```python\nprint('x')\n```\n\n```\nnever closed *x*\n```"
        );
    }

    #[test]
    fn quotes_and_panels() {
        let markdown = wiki_to_markdown("bq. A single quoted line\n{quote}\nFirst\n\n* item\n{quote}\n{panel:title=Heads up|borderStyle=dashed}\nInside the panel\n{panel}\n{panel}\nNo title\n{panel}\n{quote}inline quote{quote}");

        assert_eq!(
            markdown,
            "> A single quoted line\n\n> First\n>\n> - item\n\n> **Heads up**\n>\n> Inside the panel\n\n> **Note**\n>\n> No title\n\n> inline quote"
        );
    }

    #[test]
    fn nested_lists() {
        let markdown = wiki_to_markdown("* one\n** nested\n*** deeper\n* two\n# first\n# second\n#* mixed\n#* mixed again\n# third\n- dash item\ncontinues here\n\n*bold* at the start is not a list");

        assert_eq!(
            markdown,
            "- one\n  - nested\n    - deeper\n- two\n1. first\n2. second\n   - mixed\n   - mixed again\n3. third\n- dash item\\\n  continues here\n\n**bold** at the start is not a list"
        );
    }

    #[test]
    fn tables() {
        let markdown = wiki_to_markdown("||Browser||Works||\n|Firefox|yes|\n|Safari {{x|y}}|[docs|https://example.com]|\n\n|no|header|");

        assert_eq!(
            markdown,
            "| Browser | Works |\n| --- | --- |\n| Firefox | yes |\n| Safari `x\\|y` | [docs](https://example.com) |\n\n|  |  |\n| --- | --- |\n| no | header |"
        );
    }

    #[test]
    fn rules_emoticons_and_text_that_looks_like_markdown() {
        assert_eq!(wiki_to_markdown("above\n----\nbelow\n--\n(/) done (x) failed :) ok"), "above\n\n---\n\nbelow\\\n\\--\\\n✅ done ❌ failed 🙂 ok");
        assert_eq!(wiki_to_markdown("# not a heading in Markdown terms"), "1. not a heading in Markdown terms");
        assert_eq!(
            wiki_to_markdown("> quoted in Markdown, not in Jira\n1. neither a list"),
            "\\> quoted in Markdown, not in Jira\\\n1\\. neither a list"
        );
        assert_eq!(wiki_to_markdown(""), "");
        assert_eq!(wiki_to_markdown("   \n\n  "), "");
    }

    struct Random(u64);

    impl Random {
        fn next(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;

            (self.0 % below.max(1) as u64) as usize
        }
    }

    const PIECES: [&str; 44] = [
        "*",
        "_",
        "-",
        "+",
        "^",
        "~",
        "??",
        "{{",
        "}}",
        "{code}",
        "{code:java}",
        "{noformat}",
        "{quote}",
        "{panel:title=x}",
        "{panel}",
        "{color:red}",
        "{color}",
        "[",
        "]",
        "|",
        "||",
        "!",
        "\\",
        "\\\\",
        "h1. ",
        "bq. ",
        "# ",
        "* ",
        "** ",
        "----",
        "\n",
        "\n\n",
        " ",
        "a",
        "żółć",
        "👍",
        "http://x.io/a_b",
        "[~",
        "[^",
        "(/)",
        "{",
        "}",
        "\r\n",
        "x.png",
    ];

    #[test]
    fn never_panics_on_random_markup() {
        let mut random = Random(0x2545_f491_4f6c_dd1d);

        for _ in 0..6000 {
            let source: String = (0..random.next(60)).map(|_| PIECES[random.next(PIECES.len())]).collect();
            let _ = wiki_to_markdown(&source);
        }

        // Arbitrary bytes read as text, and pathological runs of markers.
        for seed in 0..500u32 {
            let source: String = (0..200).map(|index| char::from_u32((seed * 7919 + index * 104_729) % 0x2ff).unwrap_or('?')).collect();
            let _ = wiki_to_markdown(&source);
        }

        let _ = wiki_to_markdown(&"*a ".repeat(20_000));
        let _ = wiki_to_markdown(&"*_".repeat(20_000));
        let _ = wiki_to_markdown(&"{quote}".repeat(5_000));
        let _ = wiki_to_markdown(&"[".repeat(20_000));
    }
}
