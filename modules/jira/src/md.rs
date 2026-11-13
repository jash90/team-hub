//! Writing Markdown: the pieces both converters share. Text from Jira is data, so whatever
//! Markdown would read as markup is escaped.

/// Where a bare address ends: at whitespace or a character that cannot be part of one, and
/// before trailing punctuation that belongs to the sentence.
fn url_end(characters: &[char], start: usize) -> usize {
    let mut end = start;

    while end < characters.len() && !characters[end].is_whitespace() && !matches!(characters[end], '<' | '>' | '"' | '|' | '[' | ']' | '`' | '\\') {
        end += 1;
    }

    while end > start && matches!(characters[end - 1], '.' | ',' | ';' | ':' | '!' | '?' | ')' | '\'' | '*' | '_') {
        end -= 1;
    }

    end
}

/// The length of the address starting at `index`, if one starts there.
pub fn url_at(characters: &[char], index: usize) -> Option<usize> {
    let rest = &characters[index..];
    let starts = |prefix: &str| rest.len() > prefix.len() && prefix.chars().zip(rest).all(|(wanted, found)| wanted == *found);
    let boundary = index == 0 || !characters[index - 1].is_alphanumeric();

    if !boundary || !(starts("http://") || starts("https://")) {
        return None;
    }

    let end = url_end(characters, index);

    (end > index + 8).then_some(end - index)
}

/// Plain text as Markdown: markup characters are escaped, addresses are left whole so they
/// still become links.
pub fn escape(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    let mut index = 0;

    while index < characters.len() {
        if let Some(length) = url_at(&characters, index) {
            out.extend(&characters[index..index + length]);
            index += length;
            continue;
        }

        let character = characters[index];
        let word = |position: Option<usize>| position.and_then(|position| characters.get(position)).is_some_and(|other| other.is_alphanumeric());

        match character {
            '\\' | '`' | '*' | '[' | ']' | '<' | '~' => {
                out.push('\\');
                out.push(character);
            }
            // An underscore inside a word (snake_case) is not emphasis.
            '_' if !(word(index.checked_sub(1)) && word(Some(index + 1))) => out.push_str("\\_"),
            '&' if characters.get(index + 1).is_some_and(|next| next.is_ascii_alphabetic() || *next == '#') => out.push_str("\\&"),
            _ => out.push(character),
        }

        index += 1;
    }

    out
}

/// A line of a paragraph that would otherwise start a heading, a quote, a list or a rule.
pub fn guard_line(line: &str) -> String {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let first = trimmed.chars().next();
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    let after_digits: Vec<char> = trimmed.chars().skip(digits).take(2).collect();
    let rule = !trimmed.is_empty() && trimmed.chars().all(|character| matches!(character, '-' | '=' | ' '));

    if matches!(first, Some('#' | '>')) || rule || trimmed.starts_with("- ") || trimmed.starts_with("+ ") || trimmed == "-" || trimmed == "+" {
        return format!("\\{trimmed}");
    }

    if digits > 0 && matches!(after_digits.first(), Some('.' | ')')) && after_digits.get(1).is_none_or(|next| *next == ' ') {
        return format!("{}\\{}", &trimmed[..digits], &trimmed[digits..]);
    }

    trimmed.to_string()
}

/// `text` as inline code, whatever backticks it contains.
pub fn code_span(text: &str) -> String {
    let text = text.replace('\n', " ");
    let mut longest = 0;
    let mut run = 0;

    for character in text.chars() {
        run = if character == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }

    let ticks = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') || text.trim().is_empty() { " " } else { "" };

    format!("{ticks}{pad}{text}{pad}{ticks}")
}

/// A fenced code block; the fence is longer than any run of backticks in the code.
pub fn fence(language: &str, code: &str) -> String {
    let mut longest = 0;

    for line in code.lines() {
        longest = longest.max(line.trim_start().chars().take_while(|character| *character == '`').count());
    }

    let ticks = "`".repeat(longest.max(2) + 1);
    let language: String =
        language.chars().filter(|character| character.is_alphanumeric() || matches!(character, '+' | '-' | '#' | '_')).take(24).collect();

    format!("{ticks}{}\n{}\n{ticks}", language.to_lowercase(), code.trim_matches('\n'))
}

pub fn quote(text: &str) -> String {
    text.lines().map(|line| if line.is_empty() { ">".to_string() } else { format!("> {line}") }).collect::<Vec<_>>().join("\n")
}

/// Every line after the first moved right by `width`, for what continues a list item.
pub fn indent_rest(text: &str, width: usize) -> String {
    let pad = " ".repeat(width);

    text.lines()
        .enumerate()
        .map(|(index, line)| if index == 0 || line.is_empty() { line.to_string() } else { format!("{pad}{line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

fn cell(text: &str) -> String {
    let mut out = String::new();
    let mut escaped = false;

    for character in text.trim().chars() {
        match character {
            '|' if !escaped => out.push_str("\\|"),
            '\n' => out.push(' '),
            other => out.push(other),
        }

        escaped = character == '\\' && !escaped;
    }

    out
}

/// A GFM table. Without a header row of its own the table gets an empty one, because GFM
/// has no table without a header.
pub fn table(rows: &[Vec<String>], has_header: bool) -> String {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);

    if width == 0 {
        return String::new();
    }

    let line = |cells: &[String]| {
        let cells: Vec<String> = (0..width).map(|index| cells.get(index).map(|text| cell(text)).unwrap_or_default()).collect();

        format!("| {} |", cells.join(" | "))
    };
    let mut lines = Vec::with_capacity(rows.len() + 2);
    let body = if has_header { &rows[1..] } else { rows };

    lines.push(if has_header { line(&rows[0]) } else { line(&[]) });
    lines.push(format!("|{}", " --- |".repeat(width)));
    lines.extend(body.iter().map(|row| line(row)));

    lines.join("\n")
}

/// `inner` wrapped in `marker` on both sides, with surrounding spaces kept outside, where
/// Markdown wants them.
pub fn wrap(marker: &str, inner: &str) -> String {
    let core = inner.trim();

    if core.is_empty() {
        return inner.to_string();
    }

    let lead = &inner[..inner.len() - inner.trim_start().len()];
    let tail = &inner[inner.trim_end().len()..];

    format!("{lead}{marker}{core}{marker}{tail}")
}

/// A link target that cannot end the link early or smuggle in a script.
pub fn link_target(href: &str) -> String {
    let href = href.trim();
    let lower = href.to_lowercase();

    if lower.starts_with("javascript:") || lower.starts_with("data:") || lower.starts_with("vbscript:") {
        return "#".to_string();
    }

    href.chars()
        .map(|character| match character {
            ' ' => "%20".to_string(),
            '(' => "%28".to_string(),
            ')' => "%29".to_string(),
            '<' => "%3C".to_string(),
            '>' => "%3E".to_string(),
            '\n' | '\r' | '\t' => String::new(),
            other => other.to_string(),
        })
        .collect()
}

pub fn link(text: &str, href: &str) -> String {
    let target = link_target(href);
    let core = text.trim();

    if core.is_empty() {
        return format!("[{target}]({target})");
    }

    wrap_link(text, &target)
}

fn wrap_link(text: &str, target: &str) -> String {
    let lead = &text[..text.len() - text.trim_start().len()];
    let tail = &text[text.trim_end().len()..];

    format!("{lead}[{}]({target}){tail}", text.trim())
}

/// Blocks joined by blank lines, without the empty ones.
pub fn join_blocks(blocks: Vec<String>) -> String {
    blocks.into_iter().map(|block| block.trim_matches('\n').to_string()).filter(|block| !block.trim().is_empty()).collect::<Vec<_>>().join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_characters_in_text_are_escaped_but_addresses_survive() {
        assert_eq!(escape("2 * 3 = 6, a_b and _c_"), "2 \\* 3 = 6, a_b and \\_c\\_");
        assert_eq!(escape("see https://example.com/a_b*c?x=1, ok"), "see https://example.com/a_b*c?x=1, ok");
        assert_eq!(escape("<script> [x] `y` ~z~ \\ &amp;"), "\\<script> \\[x\\] \\`y\\` \\~z\\~ \\\\ \\&amp;");
        assert_eq!(escape("AT&T & co"), "AT\\&T & co");
    }

    #[test]
    fn lines_that_look_like_blocks_are_guarded() {
        assert_eq!(guard_line("# not a heading"), "\\# not a heading");
        assert_eq!(guard_line("> not a quote"), "\\> not a quote");
        assert_eq!(guard_line("- not a list"), "\\- not a list");
        assert_eq!(guard_line("12. not a list"), "12\\. not a list");
        assert_eq!(guard_line("---"), "\\---");
        assert_eq!(guard_line("      not code"), "not code");
        assert_eq!(guard_line("3.14 is fine"), "3.14 is fine");
    }

    #[test]
    fn code_tables_and_links() {
        assert_eq!(code_span("a `b` c"), "``a `b` c``");
        assert_eq!(code_span("`x`"), "`` `x` ``");
        assert_eq!(fence("Java", "a\n```\nb"), "````java\na\n```\nb\n````");
        assert_eq!(table(&[vec!["a".into(), "b|c".into()], vec!["1".into()]], true), "| a | b\\|c |\n| --- | --- |\n| 1 |  |");
        assert_eq!(table(&[vec!["1".into(), "2".into()]], false), "|  |  |\n| --- | --- |\n| 1 | 2 |");
        assert_eq!(wrap("**", " bold "), " **bold** ");
        assert_eq!(link("docs ", "https://example.com/a (b)"), "[docs](https://example.com/a%20%28b%29) ");
        assert_eq!(link("x", "javascript:alert(1)"), "[x](#)");
    }
}
