//! Atlassian Document Format (the JSON Jira Cloud stores descriptions and comments in) to
//! Markdown. A pure function of the document: anything it does not know keeps its text.

use hub_core::text::handle_of;
use serde_json::Value;

use crate::md;

/// Deeper nesting than any real document has; beyond it only the text is kept.
const MAX_DEPTH: usize = 60;
/// Marks in the order they wrap each other, outermost first. `code` is handled at the text itself.
const MARKS: [&str; 4] = ["link", "strong", "em", "strike"];

pub fn adf_to_markdown(document: &Value) -> String {
    let text = match document {
        Value::String(text) => md::escape(text),
        Value::Object(_) => block(document, 0),
        Value::Array(nodes) => blocks(nodes, 0),
        _ => String::new(),
    };

    text.trim_matches('\n').to_string()
}

fn kind(node: &Value) -> &str {
    node["type"].as_str().unwrap_or("")
}

fn children(node: &Value) -> &[Value] {
    node["content"].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn attr<'a>(node: &'a Value, name: &str) -> &'a str {
    node["attrs"][name].as_str().unwrap_or("")
}

fn is_inline(node: &Value) -> bool {
    matches!(
        kind(node),
        "text" | "hardBreak" | "mention" | "emoji" | "inlineCard" | "date" | "status" | "placeholder" | "mediaInline" | "inlineExtension"
    )
}

/// All the text under a node, for places where structure cannot be kept. Walks with its own
/// stack, so it works at any depth.
fn plain(node: &Value, out: &mut String) {
    let mut pending = vec![node];

    while let Some(node) = pending.pop() {
        if let Some(text) = node["text"].as_str() {
            out.push_str(text);
        }

        pending.extend(children(node).iter().rev());
    }
}

fn blocks(nodes: &[Value], depth: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut run: Vec<&Value> = Vec::new();

    // Inline nodes that sit directly among blocks (it happens in list items) form a paragraph.
    for node in nodes {
        if is_inline(node) {
            run.push(node);
            continue;
        }

        if !run.is_empty() {
            parts.push(paragraph(&std::mem::take(&mut run)));
        }

        parts.push(block(node, depth));
    }

    if !run.is_empty() {
        parts.push(paragraph(&run));
    }

    md::join_blocks(parts)
}

fn paragraph(nodes: &[&Value]) -> String {
    let mut text = String::new();
    inline(nodes, 0, &mut text);

    text.lines().map(md::guard_line).collect::<Vec<_>>().join("\n").trim().to_string()
}

fn inline_of(node: &Value) -> String {
    paragraph(&children(node).iter().collect::<Vec<_>>())
}

fn block(node: &Value, depth: usize) -> String {
    if depth > MAX_DEPTH {
        let mut text = String::new();
        plain(node, &mut text);

        return md::escape(&text);
    }

    match kind(node) {
        "doc" | "layoutSection" | "layoutColumn" | "bodiedExtension" | "decisionList" | "mediaGroup" => blocks(children(node), depth + 1),
        "paragraph" => inline_of(node),
        "heading" => {
            let level = node["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as usize;
            let title = inline_of(node).replace("\\\n", " ").replace('\n', " ");

            if title.is_empty() {
                String::new()
            } else {
                format!("{} {}", "#".repeat(level), title.trim_start_matches('\\'))
            }
        }
        "bulletList" => list(node, depth, |_| "- ".to_string()),
        "orderedList" => {
            let first = node["attrs"]["order"].as_u64().unwrap_or(1);

            list(node, depth, |index| format!("{}. ", first + index as u64))
        }
        "taskList" => list(node, depth, |_| "- ".to_string()),
        "listItem" | "taskItem" | "decisionItem" => item(node, depth),
        "codeBlock" => {
            let mut code = String::new();
            plain(node, &mut code);

            md::fence(attr(node, "language"), &code)
        }
        "blockquote" => md::quote(&blocks(children(node), depth + 1)),
        "panel" => {
            let label = match attr(node, "panelType") {
                "info" => "Info",
                "warning" => "Warning",
                "error" => "Error",
                "success" => "Success",
                "tip" => "Tip",
                _ => "Note",
            };

            md::quote(&md::join_blocks(vec![format!("**{label}**"), blocks(children(node), depth + 1)]))
        }
        "expand" | "nestedExpand" => {
            let title = attr(node, "title").trim();
            let heading = if title.is_empty() { String::new() } else { format!("**{}**", md::escape(title)) };

            md::join_blocks(vec![heading, blocks(children(node), depth + 1)])
        }
        "table" => table(node, depth),
        "rule" => "---".to_string(),
        "mediaSingle" => children(node).iter().map(media).collect::<Vec<_>>().join(" "),
        "media" => media(node),
        "blockCard" | "embedCard" => {
            let url = attr(node, "url");

            if url.is_empty() {
                String::new()
            } else {
                md::link(&md::escape(url), url)
            }
        }
        _ if is_inline(node) => paragraph(&[node]),
        // Unknown: keep whatever is inside it.
        _ => {
            let inside = children(node);

            if inside.is_empty() {
                node["text"].as_str().map(md::escape).unwrap_or_default()
            } else {
                blocks(inside, depth + 1)
            }
        }
    }
}

fn list(node: &Value, depth: usize, marker: impl Fn(usize) -> String) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut index = 0;

    for child in children(node) {
        // A task list nested in a task list is a child of the list, not of an item.
        if matches!(kind(child), "taskList" | "bulletList" | "orderedList") {
            let nested = block(child, depth + 1);

            lines.push(nested.lines().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n"));
            continue;
        }

        let marker = marker(index);
        let check = match (kind(child), attr(child, "state")) {
            ("taskItem", "DONE") => "[x] ",
            ("taskItem", _) => "[ ] ",
            _ => "",
        };
        let body = item(child, depth);
        let body = if body.is_empty() { String::new() } else { md::indent_rest(&body, marker.chars().count()) };

        lines.push(format!("{marker}{check}{body}").trim_end().to_string());
        index += 1;
    }

    lines.join("\n")
}

/// What is inside a list item: its blocks, with a nested list directly under its paragraph.
fn item(node: &Value, depth: usize) -> String {
    let inside = children(node);

    if inside.iter().all(is_inline) {
        return paragraph(&inside.iter().collect::<Vec<_>>());
    }

    let mut text = String::new();

    for child in inside {
        let rendered = if is_inline(child) { paragraph(&[child]) } else { block(child, depth + 1) };

        if rendered.trim().is_empty() {
            continue;
        }

        if !text.is_empty() {
            text.push_str(if matches!(kind(child), "bulletList" | "orderedList" | "taskList") { "\n" } else { "\n\n" });
        }

        text.push_str(&rendered);
    }

    text
}

fn table(node: &Value, depth: usize) -> String {
    let rows: Vec<Vec<String>> = children(node)
        .iter()
        .filter(|row| kind(row) == "tableRow")
        .map(|row| children(row).iter().map(|cell| blocks(children(cell), depth + 1).replace("\\\n", " ").replace("\n\n", " ")).collect())
        .collect();
    let has_header = children(node).first().is_some_and(|row| children(row).iter().any(|cell| kind(cell) == "tableHeader"));

    md::table(&rows, has_header)
}

fn media(node: &Value) -> String {
    let attrs = &node["attrs"];
    let text = |name: &str| attrs[name].as_str().filter(|text| !text.trim().is_empty());
    let name = text("alt").or(text("__fileName")).or(text("title"));

    match (kind(node), text("url")) {
        ("media" | "mediaInline", Some(url)) if attrs["type"] == "external" => {
            format!("![{}]({})", md::escape(name.unwrap_or("")), md::link_target(url))
        }
        ("media" | "mediaInline", _) => format!("\\[attachment: {}\\]", md::escape(name.or(text("id")).unwrap_or("file"))),
        _ => children(node).iter().map(media).collect::<Vec<_>>().join(" "),
    }
}

/// What a node's mark of this kind says: the address for a link, empty for the others.
fn mark<'a>(node: &'a Value, wanted: &str) -> Option<&'a str> {
    node["marks"].as_array()?.iter().find(|mark| mark["type"] == wanted).map(|mark| {
        if wanted == "link" {
            mark["attrs"]["href"].as_str().unwrap_or("")
        } else {
            ""
        }
    })
}

fn has_code(node: &Value) -> bool {
    mark(node, "code").is_some()
}

/// Neighbours that share a mark are wrapped together, so "**a** **b**" never comes out as
/// "**a****b**". Code spans take no other emphasis inside, only around them.
fn inline(nodes: &[&Value], level: usize, out: &mut String) {
    let Some(wanted) = MARKS.get(level) else {
        for node in nodes {
            leaf(node, out);
        }

        return;
    };
    let mut start = 0;

    while start < nodes.len() {
        let key = mark(nodes[start], wanted);
        let mut end = start + 1;

        while end < nodes.len() && mark(nodes[end], wanted) == key {
            end += 1;
        }

        let mut inner = String::new();
        inline(&nodes[start..end], level + 1, &mut inner);

        match (key, *wanted) {
            (None, _) => out.push_str(&inner),
            (Some(href), "link") => out.push_str(&md::link(&inner, href)),
            (Some(_), "strong") => out.push_str(&md::wrap("**", &inner)),
            (Some(_), "em") => out.push_str(&md::wrap("*", &inner)),
            (Some(_), _) => out.push_str(&md::wrap("~~", &inner)),
        }

        start = end;
    }
}

fn leaf(node: &Value, out: &mut String) {
    match kind(node) {
        "text" => {
            let text = node["text"].as_str().unwrap_or("");

            if has_code(node) {
                out.push_str(&md::code_span(text));
            } else {
                out.push_str(&md::escape(text).replace('\n', "\\\n"));
            }
        }
        "hardBreak" => out.push_str("\\\n"),
        "mention" => {
            let name = attr(node, "text").trim_start_matches('@').trim();
            let name = if name.is_empty() { attr(node, "id") } else { name };

            out.push('@');
            out.push_str(&handle_of(name));
        }
        "emoji" => {
            let text = attr(node, "text");

            out.push_str(&md::escape(if text.is_empty() { attr(node, "shortName") } else { text }));
        }
        "inlineCard" => {
            let url = attr(node, "url");

            if !url.is_empty() {
                out.push_str(&md::link(&md::escape(url), url));
            }
        }
        "date" => {
            let millis = node["attrs"]["timestamp"].as_str().and_then(|text| text.parse::<i64>().ok()).or(node["attrs"]["timestamp"].as_i64());

            if let Some(date) = millis.and_then(chrono::DateTime::from_timestamp_millis) {
                out.push_str(&date.format("%Y-%m-%d").to_string());
            }
        }
        "status" => out.push_str(&md::code_span(attr(node, "text"))),
        "media" | "mediaInline" => out.push_str(&media(node)),
        _ => {
            let mut text = String::new();
            plain(node, &mut text);
            out.push_str(&md::escape(&text));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn text(text: &str) -> Value {
        json!({ "type": "text", "text": text })
    }

    fn marked(content: &str, marks: &[&str]) -> Value {
        json!({ "type": "text", "text": content, "marks": marks.iter().map(|mark| json!({ "type": mark })).collect::<Vec<_>>() })
    }

    fn node(kind: &str, content: Vec<Value>) -> Value {
        json!({ "type": kind, "content": content })
    }

    fn paragraph(content: Vec<Value>) -> Value {
        node("paragraph", content)
    }

    fn doc(content: Vec<Value>) -> Value {
        json!({ "type": "doc", "version": 1, "content": content })
    }

    fn convert(content: Vec<Value>) -> String {
        adf_to_markdown(&doc(content))
    }

    #[test]
    fn paragraphs_and_headings() {
        let markdown = convert(vec![
            json!({ "type": "heading", "attrs": { "level": 2 }, "content": [text("Steps to reproduce")] }),
            paragraph(vec![text("Open the cart.")]),
            paragraph(vec![text("Second paragraph"), json!({ "type": "hardBreak" }), text("after a break")]),
            json!({ "type": "heading", "attrs": { "level": 9 }, "content": [text("Deep")] }),
        ]);

        assert_eq!(markdown, "## Steps to reproduce\n\nOpen the cart.\n\nSecond paragraph\\\nafter a break\n\n###### Deep");
    }

    #[test]
    fn marks_wrap_text_and_merge_across_neighbours() {
        let link = json!({ "type": "text", "text": "the docs", "marks": [{ "type": "link", "attrs": { "href": "https://example.com/a b" } }, { "type": "strong" }] });
        let markdown = convert(vec![paragraph(vec![
            marked("bold", &["strong"]),
            marked(" and bold italic", &["strong", "em"]),
            text(", "),
            marked("code_x*", &["code"]),
            text(", "),
            marked("gone", &["strike"]),
            text(", "),
            marked("under", &["underline"]),
            text(" and "),
            link,
            text("."),
        ])]);

        assert_eq!(markdown, "**bold *and bold italic***, `code_x*`, ~~gone~~, under and [**the docs**](https://example.com/a%20b).");
    }

    #[test]
    fn spaces_stay_outside_the_markers() {
        assert_eq!(convert(vec![paragraph(vec![text("a"), marked(" b ", &["strong"]), text("c")])]), "a **b** c");
        assert_eq!(convert(vec![paragraph(vec![marked("   ", &["em"]), text("x")])]), "x");
    }

    #[test]
    fn text_that_looks_like_markdown_is_escaped() {
        let markdown = convert(vec![paragraph(vec![text("# not a heading *x* [y](z) <b>")]), paragraph(vec![text("1. not a list")])]);

        assert_eq!(markdown, "\\# not a heading \\*x\\* \\[y\\](z) \\<b>\n\n1\\. not a list");
    }

    #[test]
    fn nested_lists() {
        let markdown = convert(vec![node(
            "bulletList",
            vec![
                node("listItem", vec![paragraph(vec![text("one")])]),
                node(
                    "listItem",
                    vec![
                        paragraph(vec![text("two")]),
                        json!({ "type": "orderedList", "attrs": { "order": 3 }, "content": [
                                node("listItem", vec![paragraph(vec![text("first")])]),
                                node("listItem", vec![paragraph(vec![text("second")]), node("bulletList", vec![node("listItem", vec![paragraph(vec![text("deep")])])])]),
                            ] }),
                    ],
                ),
                node("listItem", vec![paragraph(vec![text("three")]), paragraph(vec![text("with a second paragraph")])]),
            ],
        )]);

        assert_eq!(markdown, "- one\n- two\n  3. first\n  4. second\n     - deep\n- three\n\n  with a second paragraph");
    }

    #[test]
    fn task_lists_keep_their_state_and_nesting() {
        let task = |state: &str, label: &str| json!({ "type": "taskItem", "attrs": { "localId": "x", "state": state }, "content": [text(label)] });
        let markdown = convert(vec![node(
            "taskList",
            vec![task("DONE", "ordered the keys"), task("TODO", "install them"), node("taskList", vec![task("TODO", "front door")])],
        )]);

        assert_eq!(markdown, "- [x] ordered the keys\n- [ ] install them\n  - [ ] front door");
    }

    #[test]
    fn code_blocks_keep_their_language_and_their_text() {
        let markdown = convert(vec![
            json!({ "type": "codeBlock", "attrs": { "language": "rust" }, "content": [text("fn main() {\n    let x = *y;\n}")] }),
            json!({ "type": "codeBlock", "content": [text("```\nfenced inside\n```")] }),
        ]);

        assert_eq!(markdown, "```rust\nfn main() {\n    let x = *y;\n}\n```\n\n````\n```\nfenced inside\n```\n````");
    }

    #[test]
    fn quotes_and_panels() {
        let markdown = convert(vec![
            node("blockquote", vec![paragraph(vec![text("quoted")]), paragraph(vec![text("twice")])]),
            json!({ "type": "panel", "attrs": { "panelType": "warning" }, "content": [paragraph(vec![text("Do not deploy on Friday.")])] }),
            json!({ "type": "panel", "attrs": { "panelType": "custom" }, "content": [node("bulletList", vec![node("listItem", vec![paragraph(vec![text("a")])])])] }),
        ]);

        assert_eq!(markdown, "> quoted\n>\n> twice\n\n> **Warning**\n>\n> Do not deploy on Friday.\n\n> **Note**\n>\n> - a");
    }

    #[test]
    fn tables_become_gfm_tables() {
        let cell = |kind: &str, content: &str| json!({ "type": kind, "attrs": {}, "content": [paragraph(vec![text(content)])] });
        let markdown = convert(vec![node(
            "table",
            vec![
                node("tableRow", vec![cell("tableHeader", "Browser"), cell("tableHeader", "Works")]),
                node("tableRow", vec![cell("tableCell", "Firefox | ESR"), cell("tableCell", "yes")]),
                node("tableRow", vec![cell("tableCell", "Safari")]),
            ],
        )]);

        assert_eq!(markdown, "| Browser | Works |\n| --- | --- |\n| Firefox \\| ESR | yes |\n| Safari |  |");

        let headless = convert(vec![node("table", vec![node("tableRow", vec![cell("tableCell", "a"), cell("tableCell", "b")])])]);
        assert_eq!(headless, "|  |  |\n| --- | --- |\n| a | b |");
    }

    #[test]
    fn rules_mentions_emoji_cards_and_media() {
        let markdown = convert(vec![
            paragraph(vec![
                json!({ "type": "mention", "attrs": { "id": "5b10ac8d82e05b22cc7d4ef5", "text": "@Żaneta Łoś" } }),
                text(" "),
                json!({ "type": "emoji", "attrs": { "shortName": ":thumbsup:", "text": "👍" } }),
                text(" "),
                json!({ "type": "emoji", "attrs": { "shortName": ":custom:" } }),
                text(" see "),
                json!({ "type": "inlineCard", "attrs": { "url": "https://example.atlassian.net/browse/WEB-7" } }),
                text(" due "),
                json!({ "type": "date", "attrs": { "timestamp": "1730419200000" } }),
                text(" "),
                json!({ "type": "status", "attrs": { "text": "IN REVIEW", "color": "blue" } }),
            ]),
            json!({ "type": "rule" }),
            json!({ "type": "mediaSingle", "attrs": { "layout": "center" }, "content": [{ "type": "media", "attrs": { "id": "abc-1", "type": "file", "collection": "x", "alt": "screenshot.png" } }] }),
            json!({ "type": "mediaGroup", "content": [{ "type": "media", "attrs": { "id": "abc-2", "type": "file", "collection": "x" } }] }),
            json!({ "type": "mediaSingle", "content": [{ "type": "media", "attrs": { "type": "external", "url": "https://example.com/a.png" } }] }),
        ]);

        assert_eq!(
            markdown,
            "@zaneta.los 👍 :custom: see [https://example.atlassian.net/browse/WEB-7](https://example.atlassian.net/browse/WEB-7) due 2024-11-01 `IN REVIEW`\n\n---\n\n\\[attachment: screenshot.png\\]\n\n\\[attachment: abc-2\\]\n\n![](https://example.com/a.png)"
        );
    }

    #[test]
    fn unknown_nodes_keep_their_text() {
        let markdown = convert(vec![
            json!({ "type": "futureBlock", "attrs": { "x": 1 }, "content": [paragraph(vec![text("inside a block nobody knows")])] }),
            paragraph(vec![text("before "), json!({ "type": "futureInline", "content": [text("inline text")] }), text(" after")]),
            json!({ "type": "expand", "attrs": { "title": "Details" }, "content": [paragraph(vec![text("hidden")])] }),
            json!({ "type": "strange", "text": "bare text" }),
        ]);

        assert_eq!(markdown, "inside a block nobody knows\n\nbefore inline text after\n\n**Details**\n\nhidden\n\nbare text");
    }

    #[test]
    fn what_is_not_a_document_is_not_a_problem() {
        assert_eq!(adf_to_markdown(&Value::Null), "");
        assert_eq!(adf_to_markdown(&json!(42)), "");
        assert_eq!(adf_to_markdown(&json!("plain *text*")), "plain \\*text\\*");
        assert_eq!(adf_to_markdown(&json!({ "type": "doc" })), "");
        assert_eq!(adf_to_markdown(&json!({ "type": "doc", "content": "wrong" })), "");
        assert_eq!(adf_to_markdown(&json!({ "type": "paragraph", "content": [{ "type": "text" }, { "text": 5 }, 7, null] })), "");
        assert_eq!(adf_to_markdown(&json!([{ "type": "paragraph", "content": [{ "type": "text", "text": "in an array" }] }])), "in an array");
    }

    /// A small deterministic generator, so a failure can be repeated.
    struct Random(u64);

    impl Random {
        fn next(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;

            (self.0 % below.max(1) as u64) as usize
        }
    }

    const KINDS: [&str; 30] = [
        "doc",
        "paragraph",
        "heading",
        "bulletList",
        "orderedList",
        "listItem",
        "taskList",
        "taskItem",
        "codeBlock",
        "blockquote",
        "panel",
        "table",
        "tableRow",
        "tableHeader",
        "tableCell",
        "rule",
        "mediaSingle",
        "media",
        "mention",
        "emoji",
        "hardBreak",
        "inlineCard",
        "text",
        "date",
        "status",
        "expand",
        "mediaGroup",
        "unknownThing",
        "",
        "decisionList",
    ];
    const TEXTS: [&str; 12] =
        ["", " ", "plain", "*_`[]<>|\\~#", "żółć 👍", "line\nbreak", "```", "- x", "1. y", "> z", "https://x.io/a_b", "\t\u{0}"];

    fn random_node(random: &mut Random, depth: usize) -> Value {
        match random.next(12) {
            0 => return Value::Null,
            1 => return json!(random.next(1000)),
            2 => return json!(TEXTS[random.next(TEXTS.len())]),
            _ => {}
        }

        let mut node = json!({ "type": KINDS[random.next(KINDS.len())] });

        if random.next(2) == 0 {
            node["text"] = json!(TEXTS[random.next(TEXTS.len())]);
        }

        if random.next(2) == 0 {
            node["attrs"] = json!({
                "level": random.next(9), "order": random.next(5), "state": (["TODO", "DONE", ""][random.next(3)]), "language": TEXTS[random.next(TEXTS.len())],
                "panelType": (["info", "x", "warning"][random.next(3)]), "text": TEXTS[random.next(TEXTS.len())], "url": TEXTS[random.next(TEXTS.len())],
                "timestamp": (["1730419200000", "x", "-99999999999999999"][random.next(3)]), "type": (["file", "external"][random.next(2)]),
            });
        }

        if random.next(3) == 0 {
            let marks: Vec<Value> = (0..random.next(4))
                .map(|_| json!({ "type": (["strong", "em", "code", "link", "strike", "weird"][random.next(6)]), "attrs": { "href": TEXTS[random.next(TEXTS.len())] } }))
                .collect();
            node["marks"] = json!(marks);
        }

        if depth < 6 && random.next(4) != 0 {
            let content: Vec<Value> = (0..random.next(5)).map(|_| random_node(random, depth + 1)).collect();
            node["content"] = if random.next(15) == 0 { json!("not an array") } else { json!(content) };
        }

        node
    }

    #[test]
    fn never_panics_on_random_documents() {
        let mut random = Random(0x9e37_79b9_7f4a_7c15);

        for _ in 0..4000 {
            let document = random_node(&mut random, 0);
            let _ = adf_to_markdown(&document);
        }

        // Deeper than anything real: the text survives, the stack does too.
        let mut deep = text("bottom");

        for _ in 0..100 {
            deep = node("blockquote", vec![node("bulletList", vec![node("listItem", vec![deep])])]);
        }

        assert!(adf_to_markdown(&doc(vec![deep])).contains("bottom"));
    }
}
