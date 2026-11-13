//! Confluence storage format to Markdown.
//!
//! The storage format is XHTML with Confluence's own elements (`ac:` for macros, links and
//! tasks, `ri:` for what they point at). The parser here is small and forgiving: it never
//! fails, tags it does not know are dropped and their text is kept.

use hub_core::keys::is_project_key;
use hub_core::text::fold;

/// Stands for a line break inside inline text until the surrounding block decides how to write it.
const BREAK: char = '\u{e000}';
/// Elements nested deeper than this are flattened into their parent.
const MAX_DEPTH: usize = 100;

const VOID: &[&str] = &[
    "br", "hr", "img", "col", "input", "meta", "link", "wbr", "area", "base", "embed", "source", "track", "param",
];
const CONTAINERS: &[&str] = &[
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "details",
    "summary",
    "address",
    "dl",
    "dt",
    "dd",
    "center",
    "form",
    "fieldset",
    "body",
    "html",
    "tbody",
    "thead",
    "tfoot",
    "tr",
    "td",
    "th",
    "li",
    "ac:layout",
    "ac:layout-section",
    "ac:layout-cell",
    "ac:rich-text-body",
    "ac:adf-content",
    "ac:task",
    "ac:task-body",
];
const HIDDEN: &[&str] = &[
    "ac:parameter",
    "ac:placeholder",
    "ac:task-id",
    "ac:task-status",
    "ac:adf-attribute",
    "ac:adf-fallback",
    "script",
    "style",
    "head",
    "title",
    "colgroup",
    "caption",
];
const CODE: &[&str] = &["code", "tt", "kbd", "samp", "var"];
const UNSAFE_SCHEMES: &[&str] = &["javascript:", "vbscript:", "data:"];

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Text(String),
    /// A CDATA section: text exactly as written.
    Raw(String),
    Element(Element),
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Element {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            _ => None,
        })
    }

    fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|element| element.name == name)
    }

    /// The value of the macro parameter `name` (`<ac:parameter ac:name="...">`).
    fn parameter(&self, name: &str) -> Option<String> {
        self.elements()
            .find(|element| element.name == "ac:parameter" && element.attribute("ac:name") == Some(name))
            .map(|element| plain_text(&element.children))
            .filter(|value| !value.is_empty())
    }
}

// ---------- parsing ----------

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "ndash" => "–",
        "mdash" => "—",
        "hellip" => "…",
        "laquo" => "«",
        "raquo" => "»",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "bdquo" => "„",
        "sbquo" => "‚",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "deg" => "°",
        "euro" => "€",
        "pound" => "£",
        "sect" => "§",
        "para" => "¶",
        "times" => "×",
        "divide" => "÷",
        "plusmn" => "±",
        "micro" => "µ",
        "middot" => "·",
        "bull" => "•",
        "rarr" => "→",
        "larr" => "←",
        "uarr" => "↑",
        "darr" => "↓",
        "harr" => "↔",
        "le" => "≤",
        "ge" => "≥",
        "ne" => "≠",
        "frac12" => "½",
        "oacute" => "ó",
        "Oacute" => "Ó",
        "shy" => "",
        "zwnj" | "zwj" => "",
        _ => return None,
    })
}

/// Replaces `&name;`, `&#123;` and `&#x7b;`; what is not an entity stays as written.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }

    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];

        let end = rest[1..]
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '#'))
            .map(|index| index + 1);
        let replacement = end.filter(|end| rest[*end..].starts_with(';') && *end <= 12).and_then(|end| {
            let name = &rest[1..end];
            let character = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(String::from)
            } else if let Some(decimal) = name.strip_prefix('#') {
                decimal.parse::<u32>().ok().and_then(char::from_u32).map(String::from)
            } else {
                named_entity(name).map(String::from)
            };

            character.map(|character| (character, end + 1))
        });

        match replacement {
            Some((character, consumed)) => {
                decoded.push_str(&character);
                rest = &rest[consumed..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }

    decoded.push_str(rest);

    decoded
}

/// The index of the `>` that ends the tag starting at the beginning of `text`, skipping quoted values.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;

    for (index, character) in text.char_indices() {
        match (quote, character) {
            (None, '"' | '\'') => quote = Some(character),
            (Some(open), close) if open == close => quote = None,
            (None, '>') => return Some(index),
            _ => {}
        }
    }

    None
}

fn parse_attributes(text: &str) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    let mut rest = text.trim_start();

    while !rest.is_empty() {
        let name_end = rest
            .find(|character: char| character.is_whitespace() || character == '=')
            .unwrap_or(rest.len());
        let name = rest[..name_end].trim_end_matches('/').to_lowercase();
        rest = rest[name_end..].trim_start();

        let value = match rest.strip_prefix('=') {
            Some(after) => {
                let after = after.trim_start();
                let (value, remaining) = match after.chars().next() {
                    Some(quote @ ('"' | '\'')) => match after[1..].find(quote) {
                        Some(end) => (&after[1..end + 1], &after[end + 2..]),
                        None => (&after[1..], ""),
                    },
                    _ => {
                        let end = after.find(char::is_whitespace).unwrap_or(after.len());

                        (&after[..end], &after[end..])
                    }
                };
                rest = remaining.trim_start();

                decode_entities(value)
            }
            None => String::new(),
        };

        if !name.is_empty() {
            attributes.push((name, value));
        }
    }

    attributes
}

fn close(stack: &mut Vec<Element>, name: &str) {
    let Some(depth) = stack.iter().rposition(|element| element.name == name).filter(|depth| *depth > 0) else {
        return;
    };

    while stack.len() > depth {
        let Some(finished) = stack.pop() else { break };

        if let Some(parent) = stack.last_mut() {
            parent.children.push(Node::Element(finished));
        }
    }
}

fn parse(input: &str) -> Vec<Node> {
    let mut stack = vec![Element::default()];
    let mut rest = input;

    let push = |stack: &mut Vec<Element>, node: Node| {
        if let Some(top) = stack.last_mut() {
            top.children.push(node);
        }
    };

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>");
            push(&mut stack, Node::Raw(after[..end.unwrap_or(after.len())].to_string()));
            rest = end.map_or("", |end| &after[end + 3..]);
        } else if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
        } else if let Some(after) = rest.strip_prefix("</") {
            let end = after.find('>');
            close(&mut stack, after[..end.unwrap_or(after.len())].trim().to_lowercase().as_str());
            rest = end.map_or("", |end| &after[end + 1..]);
        } else if rest.starts_with('<') && rest[1..].starts_with(|character: char| character.is_ascii_alphabetic()) {
            let Some(end) = tag_end(rest) else {
                push(&mut stack, Node::Text(decode_entities(rest)));
                break;
            };
            let inner = &rest[1..end];
            let self_closing = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
            let element = Element {
                name: inner[..name_end].to_lowercase(),
                attributes: parse_attributes(&inner[name_end..]),
                children: Vec::new(),
            };

            if self_closing || VOID.contains(&element.name.as_str()) || stack.len() > MAX_DEPTH {
                push(&mut stack, Node::Element(element));
            } else {
                stack.push(element);
            }

            rest = &rest[end + 1..];
        } else {
            let end = rest[1..].find('<').map_or(rest.len(), |index| index + 1);
            push(&mut stack, Node::Text(decode_entities(&rest[..end])));
            rest = &rest[end..];
        }
    }

    while stack.len() > 1 {
        let Some(finished) = stack.pop() else { break };

        if let Some(parent) = stack.last_mut() {
            parent.children.push(Node::Element(finished));
        }
    }

    stack.pop().map(|root| root.children).unwrap_or_default()
}

// ---------- text ----------

/// The text of the nodes without any markup, with runs of white space collapsed.
fn plain_text(nodes: &[Node]) -> String {
    fn collect(nodes: &[Node], text: &mut String) {
        for node in nodes {
            match node {
                Node::Text(part) | Node::Raw(part) => text.push_str(part),
                Node::Element(element) if element.name == "br" => text.push(' '),
                Node::Element(element) => collect(&element.children, text),
            }
        }
    }

    let mut text = String::new();
    collect(nodes, &mut text);

    text.split(|character: char| character.is_whitespace() || character == '\u{a0}')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text of the nodes exactly as written, for code.
fn raw_text(nodes: &[Node]) -> String {
    let mut text = String::new();

    for node in nodes {
        match node {
            Node::Text(part) | Node::Raw(part) => text.push_str(part),
            Node::Element(element) if element.name == "br" => text.push('\n'),
            Node::Element(element) => text.push_str(&raw_text(&element.children)),
        }
    }

    text
}

fn looks_like_entity(rest: &str) -> bool {
    let name = rest
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '#')
        .count();

    name > 0 && rest[name..].starts_with(';')
}

/// Appends prose to inline Markdown: white space collapses, and characters that would start
/// formatting are escaped.
fn push_text(buffer: &mut String, text: &str) {
    let mut previous: Option<char> = None;

    for (index, character) in text.char_indices() {
        let rest = &text[index + character.len_utf8()..];

        if character.is_whitespace() || character == '\u{a0}' {
            if !buffer.ends_with(' ') && !buffer.ends_with(BREAK) {
                buffer.push(' ');
            }
        } else {
            let escape = match character {
                '\\' | '*' | '`' | '[' | ']' | '<' => true,
                '_' => !(previous.is_some_and(char::is_alphanumeric) && rest.starts_with(char::is_alphanumeric)),
                '&' => looks_like_entity(rest),
                '~' => rest.starts_with('~') || previous == Some('~'),
                _ => false,
            };

            if escape {
                buffer.push('\\');
            }

            buffer.push(character);
        }

        previous = Some(character);
    }
}

/// Appends already formatted inline Markdown without doubling the space at the seam.
fn push_markdown(buffer: &mut String, markdown: &str) {
    if (buffer.ends_with(' ') || buffer.ends_with(BREAK)) && markdown.starts_with(' ') {
        buffer.push_str(&markdown[1..]);
    } else {
        buffer.push_str(markdown);
    }
}

/// Wraps inline Markdown in an emphasis marker, keeping surrounding spaces outside of it.
fn wrap(inner: &str, marker: &str) -> String {
    let trimmed = inner.trim_matches(|character: char| character == ' ' || character == BREAK);
    let wrapped_already = trimmed
        .strip_prefix(marker)
        .and_then(|rest| rest.strip_suffix(marker))
        .is_some_and(|core| !core.is_empty() && !core.starts_with('*') && !core.ends_with('*') && !core.contains(marker));

    if trimmed.is_empty() || wrapped_already {
        return inner.to_string();
    }

    let lead = if inner.starts_with(' ') { " " } else { "" };
    let trail = if inner.ends_with(' ') { " " } else { "" };

    format!("{lead}{marker}{trimmed}{marker}{trail}")
}

fn code_span(code: &str) -> String {
    let code = code.split_whitespace().collect::<Vec<_>>().join(" ");

    if code.is_empty() {
        String::new()
    } else if code.contains('`') {
        format!("`` {code} ``")
    } else {
        format!("`{code}`")
    }
}

fn link_target(href: &str) -> Option<String> {
    let href = href.trim();
    let lower = href.to_lowercase();

    if href.is_empty() || UNSAFE_SCHEMES.iter().any(|scheme| lower.starts_with(scheme)) {
        return None;
    }

    Some(
        href.replace(' ', "%20")
            .replace('(', "%28")
            .replace(')', "%29")
            .replace('<', "%3C")
            .replace('>', "%3E"),
    )
}

fn escaped(text: &str) -> String {
    let mut buffer = String::new();
    push_text(&mut buffer, text);

    buffer.trim().to_string()
}

fn image(source: Option<&str>, alt: Option<&str>, filename: Option<&str>) -> String {
    let name = alt.map(str::trim).filter(|alt| !alt.is_empty()).or(filename).unwrap_or("image");
    let remote = source
        .map(str::trim)
        .filter(|source| source.starts_with("https://") || source.starts_with("http://"))
        .and_then(link_target);

    match remote {
        Some(url) => format!("![{}]({url})", escaped(name)),
        // An attachment stays in Confluence; what remains here is its name.
        None => format!("*(image: {})*", escaped(name)),
    }
}

/// `<ac:link>`: a link to a page becomes `[[Title]]`, anything else keeps its text.
fn confluence_link(element: &Element) -> String {
    let label = element
        .child("ac:plain-text-link-body")
        .or_else(|| element.child("ac:link-body"))
        .map(|body| plain_text(&body.children))
        .filter(|label| !label.is_empty());
    let fits = |text: &str| !text.contains("]]") && !text.contains("[[") && !text.contains('|');

    if let Some(title) = element
        .child("ri:page")
        .and_then(|page| page.attribute("ri:content-title"))
        .map(str::trim)
        .filter(|title| !title.is_empty() && fits(title))
    {
        let space = element
            .child("ri:page")
            .and_then(|page| page.attribute("ri:space-key"))
            .filter(|key| is_project_key(key));
        let target = match space {
            Some(space) => format!("{space}:{title}"),
            None => title.to_string(),
        };

        return match label.filter(|label| fits(label) && fold(label) != fold(title)) {
            Some(label) => format!("[[{target}|{label}]]"),
            None => format!("[[{target}]]"),
        };
    }

    let fallback = element.elements().find_map(|target| {
        target
            .attribute("ri:content-title")
            .or_else(|| target.attribute("ri:filename"))
            .or_else(|| target.attribute("ri:value"))
    });

    escaped(&label.or_else(|| fallback.map(String::from)).unwrap_or_default())
}

fn emoticon(element: &Element) -> String {
    match element.attribute("ac:name") {
        Some("tick") => "✓".to_string(),
        Some("cross") => "✗".to_string(),
        _ => element
            .attribute("ac:emoji-fallback")
            .filter(|fallback| !fallback.starts_with(':'))
            .unwrap_or("")
            .to_string(),
    }
}

/// Inline Markdown for the nodes. Block elements met here are flattened to their text.
fn inline_of(nodes: &[Node]) -> String {
    let mut buffer = String::new();

    for node in nodes {
        match node {
            Node::Text(text) | Node::Raw(text) => push_text(&mut buffer, text),
            Node::Element(element) => inline_element(&mut buffer, element),
        }
    }

    buffer
}

fn inline_element(buffer: &mut String, element: &Element) {
    let name = element.name.as_str();

    match name {
        "br" => {
            let kept = buffer.trim_end_matches(' ').len();
            buffer.truncate(kept);
            buffer.push(BREAK);
        }
        "strong" | "b" => push_markdown(buffer, &wrap(&inline_of(&element.children), "**")),
        "em" | "i" => push_markdown(buffer, &wrap(&inline_of(&element.children), "*")),
        "s" | "del" | "strike" => push_markdown(buffer, &wrap(&inline_of(&element.children), "~~")),
        "a" => {
            let text = inline_of(&element.children).replace(BREAK, " ");
            let shown = text.trim();
            let lead = if text.starts_with(' ') { " " } else { "" };
            let trail = if text.ends_with(' ') { " " } else { "" };

            match element.attribute("href").and_then(link_target) {
                Some(href) if shown.is_empty() => push_markdown(buffer, &format!("<{href}>")),
                Some(href) => push_markdown(buffer, &format!("{lead}[{shown}]({href}){trail}")),
                None => push_markdown(buffer, &text),
            }
        }
        "img" => {
            let source = element.attribute("src");
            let filename = source
                .and_then(|source| source.split(['?', '#']).next())
                .and_then(|path| path.rsplit('/').next())
                .filter(|file| !file.is_empty());

            buffer.push_str(&image(source, element.attribute("alt").or_else(|| element.attribute("title")), filename));
        }
        "ac:image" => {
            let source = element.child("ri:url").and_then(|url| url.attribute("ri:value"));
            let filename = element.child("ri:attachment").and_then(|attachment| attachment.attribute("ri:filename"));

            buffer.push_str(&image(
                source,
                element.attribute("ac:alt").or_else(|| element.attribute("ac:title")),
                filename,
            ));
        }
        "ac:link" => buffer.push_str(&confluence_link(element)),
        "ac:emoticon" => buffer.push_str(&emoticon(element)),
        "time" => push_text(buffer, element.attribute("datetime").unwrap_or("")),
        "ac:structured-macro" | "ac:macro" => match element.attribute("ac:name") {
            // A Jira issue macro names an issue; keeping the key lets the page link to ours.
            Some("jira") => push_text(buffer, &element.parameter("key").unwrap_or_default()),
            Some("status") => push_markdown(buffer, &wrap(&escaped(&element.parameter("title").unwrap_or_default()), "**")),
            Some("code" | "noformat") => buffer.push_str(&code_span(
                &element
                    .child("ac:plain-text-body")
                    .map(|body| raw_text(&body.children))
                    .unwrap_or_default(),
            )),
            _ => {
                if let Some(body) = element.child("ac:rich-text-body") {
                    push_markdown(buffer, &inline_of(&body.children));
                }
            }
        },
        _ if CODE.contains(&name) => buffer.push_str(&code_span(&raw_text(&element.children))),
        _ if HIDDEN.contains(&name) => {}
        _ => {
            let inner = inline_of(&element.children);
            let block = CONTAINERS.contains(&name)
                || matches!(
                    name,
                    "p" | "ul" | "ol" | "table" | "blockquote" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                );

            if block && !buffer.is_empty() && !buffer.ends_with(' ') && !buffer.ends_with(BREAK) {
                buffer.push(' ');
            }

            push_markdown(buffer, &inner);

            if block && !buffer.ends_with(' ') {
                buffer.push(' ');
            }
        }
    }
}

// ---------- blocks ----------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Paragraph,
    List,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
struct Block {
    kind: Kind,
    text: String,
}

fn other(text: String) -> Block {
    Block { kind: Kind::Other, text }
}

/// Stops a line of prose from reading as a heading, a quote, a list item or a rule.
fn guard_line(line: &str) -> String {
    let Some(first) = line.chars().next() else { return String::new() };
    let marks_alone = line.chars().all(|character| character == first);

    match first {
        '#' | '>' => format!("\\{line}"),
        '-' | '+' if line[1..].starts_with(' ') || marks_alone => format!("\\{line}"),
        '=' if marks_alone => format!("\\{line}"),
        _ if first.is_ascii_digit() => {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            let after = &line[digits..];

            if after.starts_with(". ") || after.starts_with(") ") {
                format!("{}\\{after}", &line[..digits])
            } else {
                line.to_string()
            }
        }
        _ => line.to_string(),
    }
}

/// Inline Markdown as a paragraph: line breaks written out, nothing mistaken for block syntax.
fn paragraph(inline: &str) -> String {
    inline
        .split(BREAK)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(guard_line)
        .collect::<Vec<_>>()
        .join("\\\n")
}

fn join(blocks: &[Block]) -> String {
    blocks.iter().map(|block| block.text.as_str()).collect::<Vec<_>>().join("\n\n")
}

fn quote(text: &str) -> String {
    text.lines()
        .map(|line| if line.is_empty() { ">".to_string() } else { format!("> {line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

fn language_name(language: &str) -> String {
    let language: String = language
        .trim()
        .to_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "+#-_.".contains(*character))
        .collect();

    match language.as_str() {
        "none" | "text" | "plain" | "plaintext" => String::new(),
        "c#" => "csharp".to_string(),
        "js" => "javascript".to_string(),
        "py" => "python".to_string(),
        _ => language,
    }
}

fn fence(code: &str, language: &str) -> String {
    let code = code.replace("\r\n", "\n");
    let code = code.trim_matches('\n');
    let mut longest = 0;
    let mut run = 0;

    for character in code.chars() {
        run = if character == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }

    let fence = "`".repeat((longest + 1).max(3));

    format!("{fence}{}\n{code}\n{fence}", language_name(language))
}

/// One list item: the marker, then its blocks, with every further line indented under the text.
fn list_item(marker: &str, indent: usize, blocks: &[Block]) -> String {
    let mut text = String::new();

    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            let tight = block.kind == Kind::List && blocks[index - 1].kind != Kind::Other;
            text.push_str(if tight { "\n" } else { "\n\n" });
        }

        text.push_str(&block.text);
    }

    let indent = " ".repeat(indent);
    let mut item = marker.to_string();

    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            item.push('\n');

            if !line.is_empty() {
                item.push_str(&indent);
            }
        }

        item.push_str(line);
    }

    item
}

fn list(element: &Element, ordered: bool) -> String {
    let mut items: Vec<String> = Vec::new();
    let mut number = element.attribute("start").and_then(|start| start.parse::<u64>().ok()).unwrap_or(1);

    for child in element.elements() {
        match child.name.as_str() {
            "li" => {
                let blocks = blocks_of(&child.children);

                if blocks.is_empty() {
                    continue;
                }

                let marker = if ordered { format!("{number}. ") } else { "- ".to_string() };
                items.push(list_item(&marker, marker.len(), &blocks));
                number += 1;
            }
            // A list directly inside a list, without an item around it, belongs to the item before.
            "ul" | "ol" => {
                let nested = list(child, child.name == "ol");
                let indent = if ordered { 3 } else { 2 };
                let nested = nested
                    .lines()
                    .map(|line| format!("{}{line}", " ".repeat(indent)))
                    .collect::<Vec<_>>()
                    .join("\n");

                match items.last_mut() {
                    Some(last) => {
                        last.push('\n');
                        last.push_str(&nested);
                    }
                    None => items.push(nested),
                }
            }
            _ => {}
        }
    }

    items.join("\n")
}

fn task_list(element: &Element) -> String {
    element
        .elements()
        .filter(|task| task.name == "ac:task")
        .map(|task| {
            let done = task
                .child("ac:task-status")
                .is_some_and(|status| plain_text(&status.children) == "complete");
            let blocks = task.child("ac:task-body").map(|body| blocks_of(&body.children)).unwrap_or_default();

            list_item(if done { "- [x] " } else { "- [ ] " }, 2, &blocks).trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn rows_of<'a>(element: &'a Element, rows: &mut Vec<&'a Element>) {
    for child in element.elements() {
        match child.name.as_str() {
            "tr" => rows.push(child),
            "thead" | "tbody" | "tfoot" => rows_of(child, rows),
            _ => {}
        }
    }
}

fn table(element: &Element) -> String {
    let mut rows = Vec::new();
    rows_of(element, &mut rows);

    let mut cells: Vec<(bool, Vec<String>)> = Vec::new();

    for row in rows {
        let mut header = false;
        let mut texts = Vec::new();

        for cell in row.elements().filter(|cell| cell.name == "td" || cell.name == "th") {
            header |= cell.name == "th";

            let blocks = blocks_of(&cell.children);
            let text = blocks
                .iter()
                .map(|block| block.text.replace("\\\n", "<br>").replace('\n', "<br>"))
                .collect::<Vec<_>>()
                .join("<br>");
            texts.push(text.replace('|', "\\|"));

            let span = cell.attribute("colspan").and_then(|span| span.parse::<usize>().ok()).unwrap_or(1).min(50);
            texts.extend(std::iter::repeat_n(String::new(), span.saturating_sub(1)));
        }

        if !texts.is_empty() {
            cells.push((header, texts));
        }
    }

    let width = cells.iter().map(|(_, texts)| texts.len()).max().unwrap_or(0);

    if width == 0 {
        return String::new();
    }

    let line = |texts: &[String]| {
        let mut line = String::from("|");

        for index in 0..width {
            match texts.get(index).filter(|text| !text.is_empty()) {
                Some(text) => line.push_str(&format!(" {text} |")),
                None => line.push_str(" |"),
            }
        }

        line
    };
    let header = if cells.first().is_some_and(|(header, _)| *header) {
        cells.remove(0).1
    } else {
        Vec::new()
    };
    let mut lines = vec![line(&header), format!("|{}", " --- |".repeat(width))];
    lines.extend(cells.iter().map(|(_, texts)| line(texts)));

    lines.join("\n")
}

fn panel_label(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "info" => "Info",
        "note" => "Note",
        "warning" | "error" => "Warning",
        "tip" | "success" => "Tip",
        _ => return None,
    })
}

/// A callout: a blockquote that opens with a bold label, and the title if there is one.
fn callout(label: Option<&str>, title: Option<&str>, body: &[Node]) -> String {
    let title = title.map(escaped).filter(|title| !title.is_empty());
    let heading = match (label, title) {
        (Some(label), Some(title)) => Some(format!("**{label}:** {title}")),
        (Some(label), None) => Some(format!("**{label}**")),
        (None, Some(title)) => Some(format!("**{title}**")),
        (None, None) => None,
    };
    let mut blocks: Vec<Block> = heading.into_iter().map(other).collect();
    blocks.extend(blocks_of(body));

    quote(&join(&blocks))
}

#[derive(Default)]
struct Flow {
    blocks: Vec<Block>,
    inline: String,
}

impl Flow {
    fn flush(&mut self) {
        let text = paragraph(&std::mem::take(&mut self.inline));

        if !text.is_empty() {
            self.blocks.push(Block { kind: Kind::Paragraph, text });
        }
    }

    fn block(&mut self, kind: Kind, text: String) {
        self.flush();

        if !text.is_empty() {
            self.blocks.push(Block { kind, text });
        }
    }

    fn container(&mut self, nodes: &[Node]) {
        self.flush();
        self.walk(nodes);
        self.flush();
    }

    fn walk(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Text(text) | Node::Raw(text) => push_text(&mut self.inline, text),
                Node::Element(element) => self.element(element),
            }
        }
    }

    fn element(&mut self, element: &Element) {
        let name = element.name.as_str();

        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name[1..].parse::<usize>().unwrap_or(1);
                let text = inline_of(&element.children).replace(BREAK, " ");
                let text = text.split(' ').filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ");

                self.block(
                    Kind::Other,
                    if text.is_empty() {
                        text
                    } else {
                        format!("{} {text}", "#".repeat(level))
                    },
                );
            }
            "p" => self.container(&element.children),
            "hr" => self.block(Kind::Other, "---".to_string()),
            "ul" | "ol" => self.block(Kind::List, list(element, name == "ol")),
            "ac:task-list" => self.block(Kind::List, task_list(element)),
            "blockquote" => self.block(Kind::Other, quote(&join(&blocks_of(&element.children)))),
            "pre" => self.block(Kind::Other, fence(&raw_text(&element.children), "")),
            "table" => self.block(Kind::Other, table(element)),
            "ac:structured-macro" | "ac:macro" => self.structured_macro(element),
            "ac:adf-extension" => self.adf_extension(element),
            _ if CONTAINERS.contains(&name) => self.container(&element.children),
            _ if HIDDEN.contains(&name) => {}
            "br" | "strong" | "b" | "em" | "i" | "s" | "del" | "strike" | "a" | "img" | "ac:image" | "ac:link" | "ac:emoticon" | "time" => {
                inline_element(&mut self.inline, element);
            }
            _ if CODE.contains(&name) => inline_element(&mut self.inline, element),
            _ => self.walk(&element.children),
        }
    }

    fn structured_macro(&mut self, element: &Element) {
        let name = element.attribute("ac:name").unwrap_or("").to_lowercase();
        let rich = element.child("ac:rich-text-body").map(|body| body.children.as_slice());
        let plain = element.child("ac:plain-text-body").map(|body| raw_text(&body.children));
        let title = element.parameter("title");

        match name.as_str() {
            "code" => self.block(
                Kind::Other,
                fence(&plain.unwrap_or_default(), &element.parameter("language").unwrap_or_default()),
            ),
            "noformat" => self.block(Kind::Other, fence(&plain.unwrap_or_default(), "")),
            "info" | "note" | "warning" | "tip" => self.block(Kind::Other, callout(panel_label(&name), title.as_deref(), rich.unwrap_or_default())),
            "panel" => self.block(Kind::Other, callout(None, title.as_deref(), rich.unwrap_or_default())),
            "expand" => {
                self.block(Kind::Other, title.map(|title| format!("**{}**", escaped(&title))).unwrap_or_default());
                self.container(rich.unwrap_or_default());
            }
            "jira" | "status" => inline_element(&mut self.inline, element),
            _ => match (rich, plain) {
                (Some(body), _) => self.container(body),
                (None, Some(text)) if !text.trim().is_empty() => self.block(Kind::Other, fence(&text, "")),
                _ => {}
            },
        }
    }

    /// The new editor wraps panels in `ac:adf-extension`, with the same content a second time
    /// as a fallback for old clients; only one of the two is kept.
    fn adf_extension(&mut self, element: &Element) {
        let Some(node) = element.child("ac:adf-node") else {
            if let Some(fallback) = element.child("ac:adf-fallback") {
                self.container(&fallback.children);
            }

            return;
        };
        let content = node
            .child("ac:adf-content")
            .map(|content| content.children.as_slice())
            .unwrap_or_default();
        let kind = node
            .elements()
            .find(|attribute| attribute.name == "ac:adf-attribute" && attribute.attribute("key") == Some("panel-type"))
            .map(|attribute| plain_text(&attribute.children));

        if node.attribute("type") == Some("panel") {
            self.block(Kind::Other, callout(kind.as_deref().and_then(panel_label), None, content));
        } else {
            self.container(content);
        }
    }
}

fn blocks_of(nodes: &[Node]) -> Vec<Block> {
    let mut flow = Flow::default();
    flow.container(nodes);

    flow.blocks
}

/// Converts a page body in Confluence storage format to Markdown.
pub fn storage_to_markdown(storage: &str) -> String {
    join(&blocks_of(&parse(storage)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(storage: &str) -> String {
        storage_to_markdown(storage)
    }

    #[test]
    fn headings_paragraphs_and_line_breaks() {
        assert_eq!(
            md("<h1>Title</h1><p>First   line<br/>second\n line</p><h3> Sub <strong>bold</strong></h3><p></p><p> </p>"),
            "# Title\n\nFirst line\\\nsecond line\n\n### Sub **bold**"
        );
        assert_eq!(md("<h2>Two<br/>lines</h2>"), "## Two lines");
        assert_eq!(md("<h2></h2><p>Alone</p>"), "Alone");
        assert_eq!(
            md("loose text<p>then a paragraph</p>and more"),
            "loose text\n\nthen a paragraph\n\nand more"
        );
    }

    #[test]
    fn inline_formatting() {
        assert_eq!(
            md("<p>A <strong>bold</strong>, <em>italic</em>, <b>b</b>, <i>i</i>, <s>gone</s> and <code>let x = 1;</code>.</p>"),
            "A **bold**, *italic*, **b**, *i*, ~~gone~~ and `let x = 1;`."
        );
        assert_eq!(
            md("<p>Spaces<strong> stay outside </strong>the markers, <em></em>empty ones vanish.</p>"),
            "Spaces **stay outside** the markers, empty ones vanish."
        );
        assert_eq!(
            md("<p><strong><em>both</em></strong> <u>underlined</u> <span style=\"color: red\">red</span> x<sup>2</sup></p>"),
            "***both*** underlined red x2"
        );
        assert_eq!(md("<p><code>a `tick` b</code> <code>&lt;T&gt;</code></p>"), "`` a `tick` b `` `<T>`");
    }

    #[test]
    fn text_that_looks_like_markdown_is_escaped() {
        assert_eq!(
            md("<p>2 * 3 * 4, snake_case, _private, [1], a &lt;b&gt; c\\d, `tick`</p>"),
            "2 \\* 3 \\* 4, snake_case, \\_private, \\[1\\], a \\<b> c\\\\d, \\`tick\\`"
        );
        assert_eq!(
            md("<p># not a heading</p><p>- not a list</p><p>1. not numbered</p><p>&gt; not a quote</p><p>---</p>"),
            "\\# not a heading\n\n\\- not a list\n\n1\\. not numbered\n\n\\> not a quote\n\n\\---"
        );
        assert_eq!(md("<p>AT&amp;T, &amp;amp; and R&amp;D</p>"), "AT&T, \\&amp; and R&D");
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(
            md("<p>Zaż&oacute;łć &amp; gęślą&nbsp;jaźń &mdash; &#261;&#x105; &hellip; &bogus; &#xZZ; 5 &lt; 6 &euro;</p>"),
            "Zażółć & gęślą jaźń — ąą … \\&bogus; \\&#xZZ; 5 \\< 6 €"
        );
        assert_eq!(decode_entities("a &amp;&amp; b & c &"), "a && b & c &");
    }

    #[test]
    fn links() {
        assert_eq!(
            md("<p>See <a href=\"https://example.com/a b?(x)\">the <strong>docs</strong></a>.</p>"),
            "See [the **docs**](https://example.com/a%20b?%28x%29)."
        );
        assert_eq!(
            md("<p><a href=\"https://example.com\"></a> <a name=\"anchor\">just text</a> <a href=\"javascript:alert(1)\">click</a></p>"),
            "<https://example.com> just text click"
        );
    }

    #[test]
    fn nested_lists() {
        let storage = "<ul><li>one</li><li><p>two</p><ol><li>first</li><li>second<ul><li>deep</li></ul></li></ol></li><li>three<br/>continued</li><li></li></ul>";

        assert_eq!(md(storage), "- one\n- two\n  1. first\n  2. second\n     - deep\n- three\\\n  continued");
        assert_eq!(
            md("<ol start=\"3\"><li>c</li><li><p>d</p><p>more d</p></li></ol>"),
            "3. c\n4. d\n\n   more d"
        );
        assert_eq!(md("<ul><li>a</li><ul><li>stray</li></ul><li>b</li></ul>"), "- a\n  - stray\n- b");
        assert_eq!(md("<p>Before</p><ul><li>item</li></ul><p>After</p>"), "Before\n\n- item\n\nAfter");
    }

    #[test]
    fn blockquotes_rules_and_preformatted_text() {
        assert_eq!(
            md("<blockquote><p>Quoted</p><p>twice</p><ul><li>x</li></ul></blockquote><hr/><p>after</p>"),
            "> Quoted\n>\n> twice\n>\n> - x\n\n---\n\nafter"
        );
        assert_eq!(md("<pre>line 1\n  indented &lt;tag&gt;\n</pre>"), "```\nline 1\n  indented <tag>\n```");
    }

    #[test]
    fn tables_become_gfm_tables() {
        let storage = "<table><colgroup><col/><col/></colgroup><tbody><tr><th>Name</th><th><p>Value</p></th></tr><tr><td>a|b</td><td><p>one</p><p>two<br/>three</p></td></tr><tr><td colspan=\"2\">wide</td></tr><tr><td>short</td></tr></tbody></table>";

        assert_eq!(
            md(storage),
            "| Name | Value |\n| --- | --- |\n| a\\|b | one<br>two<br>three |\n| wide | |\n| short | |"
        );
        assert_eq!(
            md("<table><tr><td>no</td><td>header</td></tr><tr><td><ul><li>x</li><li>y</li></ul></td><td></td></tr></table>"),
            "| | |\n| --- | --- |\n| no | header |\n| - x<br>- y | |"
        );
        assert_eq!(md("<table></table><p>x</p>"), "x");
    }

    #[test]
    fn images_keep_their_alt_text_or_file_name() {
        assert_eq!(
            md("<p><img src=\"https://example.com/a.png\" alt=\"A diagram\"/></p>"),
            "![A diagram](https://example.com/a.png)"
        );
        assert_eq!(
            md("<p><img src=\"/download/attachments/1/shot.png?version=2\"/></p>"),
            "*(image: shot.png)*"
        );
        assert_eq!(
            md("<p><ac:image ac:alt=\"Flow\"><ri:attachment ri:filename=\"flow.png\"/></ac:image></p>"),
            "*(image: Flow)*"
        );
        assert_eq!(
            md("<ac:image ac:width=\"200\"><ri:attachment ri:filename=\"my_flow.png\" /></ac:image>"),
            "*(image: my_flow.png)*"
        );
        assert_eq!(
            md("<ac:image><ri:url ri:value=\"https://example.com/x.png\"/></ac:image>"),
            "![image](https://example.com/x.png)"
        );
    }

    #[test]
    fn the_code_macro_becomes_a_fenced_block() {
        let storage = "<ac:structured-macro ac:name=\"code\" ac:schema-version=\"1\"><ac:parameter ac:name=\"language\">Python</ac:parameter><ac:parameter ac:name=\"title\">example.py</ac:parameter><ac:plain-text-body><![CDATA[def f(x):\n    return x * 2  # <not> &amp; escaped\n]]></ac:plain-text-body></ac:structured-macro>";

        assert_eq!(md(storage), "```python\ndef f(x):\n    return x * 2  # <not> &amp; escaped\n```");
        assert_eq!(
            md("<ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[a ``` b]]></ac:plain-text-body></ac:structured-macro>"),
            "````\na ``` b\n````"
        );
        assert_eq!(md("<ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">none</ac:parameter><ac:plain-text-body><![CDATA[x]]]]><![CDATA[>y]]></ac:plain-text-body></ac:structured-macro>"), "```\nx]]>y\n```");
        assert_eq!(
            md("<ac:structured-macro ac:name=\"noformat\"><ac:plain-text-body><![CDATA[plain]]></ac:plain-text-body></ac:structured-macro>"),
            "```\nplain\n```"
        );
        assert_eq!(md("<ul><li>Run:<ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">bash</ac:parameter><ac:plain-text-body><![CDATA[make\n\nmake install]]></ac:plain-text-body></ac:structured-macro></li></ul>"), "- Run:\n\n  ```bash\n  make\n\n  make install\n  ```");
    }

    #[test]
    fn panels_become_quotes_with_a_bold_label() {
        assert_eq!(
            md("<ac:structured-macro ac:name=\"info\"><ac:rich-text-body><p>Read this.</p></ac:rich-text-body></ac:structured-macro>"),
            "> **Info**\n>\n> Read this."
        );
        assert_eq!(
            md("<ac:structured-macro ac:name=\"warning\"><ac:parameter ac:name=\"title\">Careful</ac:parameter><ac:rich-text-body><p>One</p><ul><li>two</li></ul></ac:rich-text-body></ac:structured-macro>"),
            "> **Warning:** Careful\n>\n> One\n>\n> - two"
        );
        assert_eq!(md("<ac:structured-macro ac:name=\"note\"><ac:rich-text-body><p>n</p></ac:rich-text-body></ac:structured-macro><ac:structured-macro ac:name=\"tip\"><ac:rich-text-body><p>t</p></ac:rich-text-body></ac:structured-macro>"), "> **Note**\n>\n> n\n\n> **Tip**\n>\n> t");
        assert_eq!(md("<ac:structured-macro ac:name=\"panel\"><ac:parameter ac:name=\"title\">Box</ac:parameter><ac:rich-text-body><p>in</p></ac:rich-text-body></ac:structured-macro>"), "> **Box**\n>\n> in");

        let adf = "<ac:adf-extension><ac:adf-node type=\"panel\"><ac:adf-attribute key=\"panel-type\">note</ac:adf-attribute><ac:adf-content><p>New editor</p></ac:adf-content></ac:adf-node><ac:adf-fallback><div class=\"panel\"><p>New editor</p></div></ac:adf-fallback></ac:adf-extension>";
        assert_eq!(md(adf), "> **Note**\n>\n> New editor");
    }

    #[test]
    fn other_macros_keep_their_content_and_drop_their_parameters() {
        assert_eq!(
            md("<ac:structured-macro ac:name=\"toc\"><ac:parameter ac:name=\"maxLevel\">2</ac:parameter></ac:structured-macro><p>x</p>"),
            "x"
        );
        assert_eq!(md("<ac:structured-macro ac:name=\"expand\"><ac:parameter ac:name=\"title\">Details</ac:parameter><ac:rich-text-body><p>Hidden</p></ac:rich-text-body></ac:structured-macro>"), "**Details**\n\nHidden");
        assert_eq!(md("<ac:structured-macro ac:name=\"excerpt\"><ac:parameter ac:name=\"hidden\">true</ac:parameter><ac:rich-text-body><p>Kept</p></ac:rich-text-body></ac:structured-macro>"), "Kept");
        assert_eq!(md("<p>Tracked in <ac:structured-macro ac:name=\"jira\"><ac:parameter ac:name=\"server\">Jira</ac:parameter><ac:parameter ac:name=\"key\">WEB-12</ac:parameter></ac:structured-macro> <ac:structured-macro ac:name=\"status\"><ac:parameter ac:name=\"colour\">Green</ac:parameter><ac:parameter ac:name=\"title\">DONE</ac:parameter></ac:structured-macro></p>"), "Tracked in WEB-12 **DONE**");
        assert_eq!(md("<ac:layout><ac:layout-section ac:type=\"two_equal\"><ac:layout-cell><p>left</p></ac:layout-cell><ac:layout-cell><p>right</p></ac:layout-cell></ac:layout-section></ac:layout>"), "left\n\nright");
    }

    #[test]
    fn page_links_become_wiki_links() {
        assert_eq!(
            md("<p>See <ac:link><ri:page ri:content-title=\"Release plan\"/></ac:link>.</p>"),
            "See [[Release plan]]."
        );
        assert_eq!(md("<p><ac:link><ri:page ri:content-title=\"Release plan\" ri:space-key=\"OPS\"/><ac:plain-text-link-body><![CDATA[the plan]]></ac:plain-text-link-body></ac:link></p>"), "[[OPS:Release plan|the plan]]");
        assert_eq!(md("<p><ac:link><ri:page ri:content-title=\"Płatności\"/><ac:link-body>płatności</ac:link-body></ac:link> <ac:link><ri:page ri:content-title=\"Home\" ri:space-key=\"~jan\"/></ac:link></p>"), "[[Płatności]] [[Home]]");
        assert_eq!(md("<p><ac:link><ri:page ri:content-title=\"A | B\"/></ac:link>, <ac:link><ri:attachment ri:filename=\"spec_v2.pdf\"/></ac:link>, <ac:link ac:anchor=\"top\"><ac:plain-text-link-body><![CDATA[up]]></ac:plain-text-link-body></ac:link>, <ac:link><ri:user ri:account-id=\"42\"/></ac:link>!</p>"), "A | B, spec_v2.pdf, up, !");
    }

    #[test]
    fn task_lists_become_checkboxes() {
        let storage = "<ac:task-list>\n<ac:task><ac:task-id>1</ac:task-id><ac:task-status>incomplete</ac:task-status><ac:task-body><span class=\"placeholder-inline-tasks\">Order <strong>keys</strong></span></ac:task-body></ac:task>\n<ac:task><ac:task-id>2</ac:task-id><ac:task-status>complete</ac:task-status><ac:task-body>Book the window<ac:task-list><ac:task><ac:task-status>incomplete</ac:task-status><ac:task-body>Tell support</ac:task-body></ac:task></ac:task-list></ac:task-body></ac:task>\n</ac:task-list>";

        assert_eq!(md(storage), "- [ ] Order **keys**\n- [x] Book the window\n  - [ ] Tell support");
    }

    #[test]
    fn unknown_tags_are_dropped_and_their_text_kept() {
        assert_eq!(md("<p>Some <blink>old</blink> <ac:unknown-thing>markup</ac:unknown-thing><ac:placeholder>type here</ac:placeholder> <time datetime=\"2026-03-01\"/> <ac:emoticon ac:name=\"tick\"/></p>"), "Some old markup 2026-03-01 ✓");
        assert_eq!(
            md("<div><span>a</span><custom>b</custom></div><section>c</section><!-- comment --><?xml version=\"1.0\"?><!DOCTYPE html>"),
            "ab\n\nc"
        );
    }

    #[test]
    fn broken_markup_does_not_break_the_converter() {
        assert_eq!(md("<p>unclosed <strong>bold<p>next</p>"), "unclosed **bold next**");
        assert_eq!(md("</p>stray close</div> text <"), "stray close text \\<");
        assert_eq!(md("<p>a < b and <3</p>"), "a \\< b and \\<3");
        assert_eq!(md("<p attr=\"never closed"), "\\<p attr=\"never closed");
        assert_eq!(md("<![CDATA[unterminated"), "unterminated");
        assert_eq!(md(""), "");
        assert_eq!(md(&"<div>".repeat(5000)), "");
        assert_eq!(
            md(&format!("{}deep{}", "<ul><li>".repeat(300), "</li></ul>".repeat(300))).lines().count(),
            1
        );
    }

    #[test]
    fn a_whole_page() {
        let storage = r#"<h1>Wdrożenie</h1>
<p>Opis procesu dla <ac:link><ri:page ri:content-title="Architektura sklepu" /></ac:link>, zob. WEB-13.</p>
<ac:structured-macro ac:name="warning" ac:schema-version="1"><ac:rich-text-body><p>Tylko w oknie serwisowym.</p></ac:rich-text-body></ac:structured-macro>
<ol>
  <li>Oznacz wydanie</li>
  <li>Zatwierdź w potoku</li>
</ol>
<ac:task-list><ac:task><ac:task-status>incomplete</ac:task-status><ac:task-body>Zapowiedź na kanale</ac:task-body></ac:task></ac:task-list>
<table><tbody><tr><th>Krok</th><th>Kto</th></tr><tr><td>Tag</td><td>Dyżurny</td></tr></tbody></table>"#;

        assert_eq!(
            md(storage),
            "# Wdrożenie\n\nOpis procesu dla [[Architektura sklepu]], zob. WEB-13.\n\n> **Warning**\n>\n> Tylko w oknie serwisowym.\n\n1. Oznacz wydanie\n2. Zatwierdź w potoku\n\n- [ ] Zapowiedź na kanale\n\n| Krok | Kto |\n| --- | --- |\n| Tag | Dyżurny |"
        );
    }
}
