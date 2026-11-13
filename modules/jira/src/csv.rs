//! Jira's CSV export, for importing without API access. The export repeats a column's name
//! for every value of a multi-value field (`Labels`, `Sprint`, `Comment`, …), and cells may
//! hold line breaks inside quotes. Pure functions: text in, issues out.

use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, NaiveDateTime, SecondsFormat};
use sha2::{Digest, Sha256};

use crate::mapping::{category_of, default_priority, fold, Structure};
use crate::remote::{
    self, Body, RemoteAttachment, RemoteComment, RemoteIssue, RemoteLink, RemoteSprint, RemoteStatus, RemoteType, RemoteUser, RemoteVersion,
    RemoteWorklog,
};
use crate::types::JiraCsvColumn;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// What a column can be read as. `ignore` leaves it out.
pub const TARGETS: [&str; 31] = [
    "ignore",
    "key",
    "id",
    "projectKey",
    "projectName",
    "summary",
    "description",
    "type",
    "status",
    "statusCategory",
    "priority",
    "assignee",
    "assigneeId",
    "reporter",
    "reporterId",
    "created",
    "updated",
    "resolved",
    "due",
    "labels",
    "components",
    "fixVersion",
    "sprint",
    "points",
    "estimate",
    "parent",
    "comment",
    "worklog",
    "attachment",
    "linkInward",
    "linkOutward",
];

/// The delimiter of the file: whichever of the usual ones the header line uses most.
fn delimiter(text: &str) -> char {
    let mut counts = [(',', 0usize), (';', 0), ('\t', 0), ('|', 0)];
    let mut quoted = false;

    for character in text.chars() {
        match character {
            '"' => quoted = !quoted,
            '\n' if !quoted => break,
            _ if !quoted => {
                if let Some(entry) = counts.iter_mut().find(|(candidate, _)| *candidate == character) {
                    entry.1 += 1;
                }
            }
            _ => {}
        }
    }

    counts.iter().max_by_key(|(_, count)| *count).filter(|(_, count)| *count > 0).map_or(',', |(candidate, _)| *candidate)
}

/// Reads CSV as spreadsheets write it: cells in double quotes may contain the delimiter, line
/// breaks and doubled quotes. The first row is the header.
pub fn parse(text: &str) -> Table {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let delimiter = delimiter(text);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        if quoted {
            match character {
                '"' if characters.peek() == Some(&'"') => {
                    characters.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                '\r' if characters.peek() == Some(&'\n') => {}
                other => cell.push(other),
            }

            continue;
        }

        match character {
            '"' if cell.is_empty() => quoted = true,
            '\r' if characters.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            other if other == delimiter => row.push(std::mem::take(&mut cell)),
            other => cell.push(other),
        }
    }

    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }

    rows.retain(|row| row.iter().any(|cell| !cell.trim().is_empty()));

    let mut rows = rows.into_iter();
    let header: Vec<String> = rows.next().unwrap_or_default().into_iter().map(|name| name.trim().to_string()).collect();

    Table { rows: rows.collect(), header }
}

/// What a column of Jira's export holds, going by its name.
pub fn default_target(header: &str) -> &'static str {
    let name = fold(header);

    match name.as_str() {
        "issue key" | "key" => "key",
        "issue id" => "id",
        "project key" => "projectKey",
        "project name" => "projectName",
        "summary" => "summary",
        "description" => "description",
        "issue type" | "issuetype" | "type" => "type",
        "status" => "status",
        "status category" => "statusCategory",
        "priority" => "priority",
        "assignee" => "assignee",
        "assignee id" => "assigneeId",
        "reporter" => "reporter",
        "reporter id" => "reporterId",
        "created" => "created",
        "updated" => "updated",
        "resolved" => "resolved",
        "due date" | "due" => "due",
        "labels" => "labels",
        "component/s" | "components" => "components",
        "fix version/s" | "fix versions" => "fixVersion",
        "sprint" => "sprint",
        "custom field (story points)" | "custom field (story point estimate)" | "story points" => "points",
        "original estimate" => "estimate",
        "parent" | "parent id" | "parent key" | "custom field (epic link)" => "parent",
        "comment" | "comments" => "comment",
        "log work" => "worklog",
        "attachment" | "attachments" => "attachment",
        _ if name.starts_with("inward issue link") => "linkInward",
        _ if name.starts_with("outward issue link") => "linkOutward",
        _ => "ignore",
    }
}

/// The file's columns with what each is read as: the administrator's choice where there is
/// one for that column, the default otherwise.
pub fn columns(table: &Table, chosen: Option<&[JiraCsvColumn]>) -> Vec<JiraCsvColumn> {
    table
        .header
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let choice = chosen.and_then(|chosen| chosen.iter().find(|column| column.index == index)).map(|column| column.target.as_str());
            let target = choice.and_then(|choice| TARGETS.iter().find(|target| **target == choice)).copied().unwrap_or_else(|| default_target(name));
            let sample = table.rows.iter().filter_map(|row| row.get(index)).map(|cell| cell.trim()).find(|cell| !cell.is_empty()).unwrap_or("");

            JiraCsvColumn { index, name: name.clone(), sample: hub_core::text::truncate(&sample.replace('\n', " "), 60), target: target.to_string() }
        })
        .collect()
}

/// Jira's default date format in exports is `12/Mar/24 9:15 AM`; ISO dates work too.
pub fn csv_instant(text: &str) -> Option<String> {
    let text = text.trim();

    if text.is_empty() {
        return None;
    }

    if let Some(instant) = remote::instant(text) {
        return Some(instant);
    }

    const WITH_TIME: [&str; 8] = [
        "%d/%b/%y %I:%M %p",
        "%d/%b/%Y %I:%M %p",
        "%d/%b/%y %H:%M",
        "%d/%b/%Y %H:%M",
        "%d/%b/%y %H:%M:%S",
        "%d.%m.%Y %H:%M",
        "%Y/%m/%d %H:%M",
        "%d/%m/%Y %H:%M",
    ];
    const DATE_ONLY: [&str; 4] = ["%d/%b/%y", "%d/%b/%Y", "%d.%m.%Y", "%Y/%m/%d"];

    WITH_TIME
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
        .or_else(|| DATE_ONLY.iter().find_map(|format| NaiveDate::parse_from_str(text, format).ok()).and_then(|date| date.and_hms_opt(0, 0, 0)))
        .map(|naive| naive.and_utc().to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();

    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }

    hex::encode(&hasher.finalize()[..12])
}

/// The cells of a row by what their columns are read as, without the empty ones.
struct Row<'a> {
    cells: Vec<(&'a str, &'a str, &'a str)>,
}

impl<'a> Row<'a> {
    fn new(columns: &'a [JiraCsvColumn], row: &'a [String]) -> Self {
        Self {
            cells: columns
                .iter()
                .filter_map(|column| Some((column.target.as_str(), column.name.as_str(), row.get(column.index)?.trim())))
                .filter(|(target, _, cell)| *target != "ignore" && !cell.is_empty())
                .collect(),
        }
    }

    fn all(&self, target: &'a str) -> impl Iterator<Item = &'a str> + '_ {
        self.cells.iter().filter(move |(found, _, _)| *found == target).map(|(_, _, cell)| *cell)
    }

    fn first(&self, target: &'a str) -> Option<&'a str> {
        self.all(target).next()
    }
}

fn person(name: Option<&str>, id: Option<&str>) -> Option<RemoteUser> {
    let name = name.or(id)?;

    Some(RemoteUser { account: id.unwrap_or(name).to_string(), name: name.to_string(), email: name.contains('@').then(|| name.to_string()) })
}

/// `12/Mar/24 9:15 AM;jkowalski;The text` → when, who, what. A cell that is not in that
/// shape is all text.
fn comment(cell: &str, fallback: &str, names: &HashMap<String, String>) -> RemoteComment {
    let mut parts = cell.splitn(3, ';');
    let (first, second, third) = (parts.next().unwrap_or(""), parts.next(), parts.next());
    let (created, author, body) = match (csv_instant(first), second, third) {
        (Some(created), Some(author), Some(body)) => (created, Some(author.trim()).filter(|author| !author.is_empty()), body),
        _ => (fallback.to_string(), None, cell),
    };

    RemoteComment {
        id: digest(&[&created, author.unwrap_or(""), body]),
        author: author.map(|author| RemoteUser {
            account: author.to_string(),
            name: names.get(author).cloned().unwrap_or_else(|| author.to_string()),
            // An author written as an e-mail address is matched by it.
            email: author.contains('@').then(|| author.to_string()),
        }),
        body: Body::Wiki(body.trim().to_string()),
        updated: created.clone(),
        created,
    }
}

/// `Fixed the redirect;12/Mar/24 9:15 AM;jkowalski;3600` → a work log.
fn worklog(cell: &str, names: &HashMap<String, String>) -> Option<RemoteWorklog> {
    let mut parts = cell.rsplitn(4, ';');
    let seconds: i64 = parts.next()?.trim().parse().ok()?;
    let author = parts.next()?.trim();
    let started = csv_instant(parts.next()?)?;
    let note = parts.next().unwrap_or("").trim();

    Some(RemoteWorklog {
        id: digest(&[&started, author, &seconds.to_string(), note]),
        author: (!author.is_empty()).then(|| RemoteUser {
            account: author.to_string(),
            name: names.get(author).cloned().unwrap_or_else(|| author.to_string()),
            // An author written as an e-mail address is matched by it.
            email: author.contains('@').then(|| author.to_string()),
        }),
        seconds,
        started,
        comment: if note.is_empty() { Body::None } else { Body::Wiki(note.to_string()) },
    })
}

/// `12/Mar/24 9:15 AM;jkowalski;spec.pdf;https://jira/secure/attachment/1/spec.pdf`.
fn attachment(cell: &str) -> RemoteAttachment {
    let parts: Vec<&str> = cell.splitn(4, ';').collect();

    match parts.as_slice() {
        [_, _, filename, url] => RemoteAttachment { filename: filename.trim().to_string(), url: url.trim().to_string(), size: None },
        _ => RemoteAttachment {
            filename: cell.rsplit('/').next().unwrap_or(cell).to_string(),
            url: if cell.contains("://") { cell.to_string() } else { String::new() },
            size: None,
        },
    }
}

fn link_type(header: &str) -> String {
    header
        .split_once('(')
        .and_then(|(_, rest)| rest.rsplit_once(')'))
        .map(|(name, _)| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Relates".to_string())
}

fn category_key(text: &str) -> String {
    match fold(text).as_str() {
        "to do" | "new" => "new",
        "in progress" | "indeterminate" => "indeterminate",
        "done" => "done",
        _ => "",
    }
    .to_string()
}

/// The projects the file has issues of, with how many, most first.
pub fn projects(table: &Table, columns: &[JiraCsvColumn]) -> Vec<(String, i64)> {
    let mut counts: Vec<(String, i64)> = Vec::new();

    for row in &table.rows {
        let Some((project, _)) = Row::new(columns, row).first("key").and_then(hub_core::keys::parse_issue_key) else { continue };

        match counts.iter_mut().find(|(key, _)| *key == project) {
            Some(entry) => entry.1 += 1,
            None => counts.push((project, 1)),
        }
    }

    counts.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));

    counts
}

#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub structure: Structure,
    pub issues: Vec<RemoteIssue>,
    /// Rows that could not be read, and things that had to be guessed.
    pub problems: Vec<String>,
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !value.trim().is_empty() && !list.iter().any(|known| fold(known) == fold(value)) {
        list.push(value.trim().to_string());
    }
}

/// The issues of one project of the file, and the project's structure as far as the file shows it.
pub fn read(table: &Table, columns: &[JiraCsvColumn], project_key: &str, project_name: Option<&str>, now: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let rows: Vec<Row> = table.rows.iter().map(|row| Row::new(columns, row)).collect();
    // Parents are given by Jira's issue id; names of people by their account id.
    let mut keys_by_id: HashMap<&str, &str> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();

    for row in &rows {
        if let (Some(id), Some(key)) = (row.first("id"), row.first("key")) {
            keys_by_id.insert(id, key);
        }

        for (name, id) in [("assignee", "assigneeId"), ("reporter", "reporterId")] {
            if let (Some(name), Some(id)) = (row.first(name), row.first(id)) {
                names.insert(id.to_string(), name.to_string());
            }
        }
    }

    let mut seen: HashSet<i64> = HashSet::new();
    let mut statuses: Vec<RemoteStatus> = Vec::new();
    let mut priorities: Vec<String> = Vec::new();
    let mut types: Vec<String> = Vec::new();
    let mut components: Vec<String> = Vec::new();
    let mut versions: Vec<String> = Vec::new();
    let mut sprints: Vec<String> = Vec::new();
    let mut name = project_name.map(str::trim).filter(|name| !name.is_empty()).map(String::from);

    for (index, row) in rows.iter().enumerate() {
        let line = index + 2;
        let Some(key) = row.first("key") else {
            parsed.problems.push(format!("Row {line} has no issue key"));
            continue;
        };
        let Some((project, number)) = hub_core::keys::parse_issue_key(key) else {
            parsed.problems.push(format!("Row {line}: \"{}\" is not an issue key", hub_core::text::truncate(key, 40)));
            continue;
        };

        if project != project_key {
            continue;
        }

        if !seen.insert(number) {
            parsed.problems.push(format!("{project}-{number} appears more than once; the first row counts"));
            continue;
        }

        let key = format!("{project}-{number}");
        let created = row.first("created").and_then(csv_instant);

        if created.is_none() {
            parsed.problems.push(format!(
                "{key}: the creation date \"{}\" was not understood; the time of the import is used",
                row.first("created").unwrap_or("")
            ));
        }

        let created = created.unwrap_or_else(|| now.to_string());
        let updated = row.first("updated").and_then(csv_instant).unwrap_or_else(|| created.clone());
        let status_name = row.first("status").unwrap_or("To Do").to_string();
        let category = row.first("statusCategory").map(category_key).unwrap_or_default();
        let type_name = row.first("type").unwrap_or("Task").to_string();
        let parent_key = row.first("parent").and_then(|parent| match hub_core::keys::parse_issue_key(parent) {
            Some((project, number)) => Some(format!("{project}-{number}")),
            None => keys_by_id.get(parent).map(|key| key.to_string()),
        });

        if row.first("parent").is_some() && parent_key.is_none() {
            parsed.problems.push(format!("{key}: its parent is not in the file"));
        }

        if !statuses.iter().any(|known| fold(&known.name) == fold(&status_name)) {
            statuses.push(RemoteStatus { id: String::new(), name: status_name.clone(), category });
        }

        if name.is_none() {
            name = row.first("projectName").map(String::from);
        }

        let issue = RemoteIssue {
            id: row.first("id").unwrap_or(&key).to_string(),
            number,
            summary: row.first("summary").unwrap_or("").to_string(),
            description: row.first("description").map_or(Body::None, |text| Body::Wiki(text.to_string())),
            priority: row.first("priority").map(String::from),
            assignee: person(row.first("assignee"), row.first("assigneeId")),
            reporter: person(row.first("reporter"), row.first("reporterId")),
            labels: row.all("labels").flat_map(str::split_whitespace).map(String::from).collect(),
            components: row.all("components").map(String::from).collect(),
            fix_versions: row.all("fixVersion").map(String::from).collect(),
            sprints: row
                .all("sprint")
                .map(|sprint| RemoteSprint { id: format!("name:{}", fold(sprint)), name: sprint.to_string(), ..Default::default() })
                .collect(),
            points: row.first("points").and_then(|points| points.replace(',', ".").parse().ok()),
            due_date: row.first("due").and_then(csv_instant).and_then(|due| remote::day(&due)),
            estimate_seconds: row
                .first("estimate")
                .and_then(|seconds| seconds.parse::<f64>().ok())
                .map(|seconds| seconds as i64)
                .filter(|seconds| *seconds > 0),
            parent_key,
            resolved: row.first("resolved").and_then(csv_instant),
            comments: row.all("comment").map(|cell| comment(cell, &updated, &names)).collect(),
            worklogs: row.all("worklog").filter_map(|cell| worklog(cell, &names)).collect(),
            links: row
                .cells
                .iter()
                .filter(|(target, _, _)| matches!(*target, "linkInward" | "linkOutward"))
                .map(|(target, header, cell)| RemoteLink {
                    type_name: link_type(header),
                    outward: *target == "linkOutward",
                    other_key: cell.to_string(),
                })
                .collect(),
            attachments: row.all("attachment").map(attachment).collect(),
            type_name,
            status_name,
            created,
            updated,
            key,
            ..Default::default()
        };

        push_unique(&mut types, &issue.type_name);
        issue.priority.iter().for_each(|priority| push_unique(&mut priorities, priority));
        issue.components.iter().for_each(|component| push_unique(&mut components, component));
        issue.fix_versions.iter().for_each(|version| push_unique(&mut versions, version));
        issue.sprints.iter().for_each(|sprint| push_unique(&mut sprints, &sprint.name));
        parsed.issues.push(issue);
    }

    parsed.issues.sort_by_key(|issue| issue.number);

    let people = {
        let mut people: Vec<RemoteUser> = Vec::new();
        let authors = parsed.issues.iter().flat_map(|issue| {
            issue.assignee.iter().chain(&issue.reporter).chain(issue.comments.iter().filter_map(|comment| comment.author.as_ref()))
        });

        for person in authors {
            if !people.iter().any(|known| known.account == person.account) {
                people.push(person.clone());
            }
        }

        people
    };
    let is_done = |issue: &RemoteIssue| {
        statuses
            .iter()
            .find(|status| fold(&status.name) == fold(&issue.status_name))
            .is_some_and(|status| category_of(&status.category, &status.name) == hub_core::StatusCategory::Done)
    };
    // The file does not say which sprints are over. One whose issues are all done (or moved
    // on to a later sprint) is taken as completed; the others wait to be started.
    let sprints: Vec<RemoteSprint> = sprints
        .iter()
        .map(|name| {
            let id = format!("name:{}", fold(name));
            let last: Vec<&RemoteIssue> = parsed.issues.iter().filter(|issue| issue.sprints.last().is_some_and(|sprint| sprint.id == id)).collect();
            let closed = last.iter().all(|issue| is_done(issue));

            RemoteSprint {
                id,
                name: name.clone(),
                state: if closed { "closed" } else { "future" }.to_string(),
                completed: if closed { last.iter().filter_map(|issue| issue.resolved.clone().or(Some(issue.updated.clone()))).max() } else { None },
                ..Default::default()
            }
        })
        .collect();

    for issue in &mut parsed.issues {
        for sprint in &mut issue.sprints {
            if let Some(known) = sprints.iter().find(|known| known.id == sprint.id) {
                *sprint = known.clone();
            }
        }
    }

    statuses.sort_by_key(|status| match category_of(&status.category, &status.name) {
        hub_core::StatusCategory::Todo => 0,
        hub_core::StatusCategory::InProgress => 1,
        hub_core::StatusCategory::Done => 2,
    });
    priorities.sort_by_key(|priority| default_priority(priority, 2, 5));

    parsed.structure = Structure {
        key: project_key.to_string(),
        name: name.unwrap_or_else(|| project_key.to_string()),
        types: types
            .iter()
            .map(|name| RemoteType {
                name: name.clone(),
                subtask: fold(name).contains("sub-task") || fold(name).contains("subtask"),
                epic: fold(name) == "epic",
            })
            .collect(),
        statuses,
        priorities,
        components: components.into_iter().map(|name| (name, String::new())).collect(),
        versions: versions.into_iter().map(|name| RemoteVersion { name, ..Default::default() }).collect(),
        scrum: !sprints.is_empty(),
        sprints,
        people,
        ..Default::default()
    };

    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = "\u{feff}Summary,Issue key,Issue id,Issue Type,Status,Project key,Project name,Priority,Assignee,Reporter,Created,Updated,Resolved,Description,Labels,Labels,Sprint,Sprint,Comment,Comment,Custom field (Story Points),Parent id,Outward issue link (Blocks),Log Work,Attachment,Watchers\r\n\
\"Checkout, the \"\"new\"\" one\",SHOP-1,10001,Epic,In Progress,SHOP,Web shop,High,Anna Nowak,Jan Kowalski,01/Mar/24 9:00 AM,05/Mar/24 2:30 PM,,\"h2. Goal\n\nA *better* checkout,\nwith fewer steps.\",checkout,q2 payments,,,\"02/Mar/24 10:15 AM;jkowalski;First line\nsecond line; with a semicolon\",Just text,,,,,,ignored\r\n\
Pay by card,SHOP-2,10002,Story,Done,SHOP,Web shop,Medium,Jan Kowalski,Anna Nowak,02/Mar/24 9:00 AM,09/Mar/24 4:00 PM,09/Mar/24 4:00 PM,,,,Sprint 1,Sprint 2,,,5.0,10001,SHOP-3,Done at last;08/Mar/24 1:00 PM;jkowalski;5400,03/Mar/24 9:00 AM;anowak;spec.pdf;https://jira.example.com/secure/attachment/7/spec.pdf,\r\n\
Refunds,SHOP-3,10003,Story,To Do,SHOP,Web shop,,,,2024-03-03 08:00,2024-03-03 08:00,,,,,Sprint 2,,,,,10001,,,,\r\n\
\r\n\
Other project,OPS-9,20009,Task,Open,OPS,Operations,Low,,,03/Mar/24 9:00 AM,03/Mar/24 9:00 AM,,,,,,,,,,,,,,\r\n\
Broken row,not a key,,,,,,,,,,,,,,,,,,,,,,,,\r\n";

    #[test]
    fn quoted_cells_keep_their_delimiters_quotes_and_line_breaks() {
        let table = parse(EXPORT);

        assert_eq!(table.header.len(), 26);
        assert_eq!(table.header[0], "Summary");
        assert_eq!(table.rows.len(), 5);
        assert_eq!(table.rows[0][0], "Checkout, the \"new\" one");
        assert_eq!(table.rows[0][13], "h2. Goal\n\nA *better* checkout,\nwith fewer steps.");
        assert_eq!(table.rows[0][18], "02/Mar/24 10:15 AM;jkowalski;First line\nsecond line; with a semicolon");

        let semicolons = parse("Summary;Issue key\n\"a;b\";X-1\n");
        assert_eq!(semicolons.rows, vec![vec!["a;b".to_string(), "X-1".to_string()]]);
        assert_eq!(parse("").header.len(), 0);
        assert_eq!(parse("only,a,header").rows.len(), 0);
        assert_eq!(parse("a,b\n\"unclosed,1\n2").rows.len(), 1);
    }

    #[test]
    fn columns_are_read_by_their_names_and_can_be_read_otherwise() {
        let table = parse(EXPORT);
        let defaults = columns(&table, None);
        let targets: Vec<&str> = defaults.iter().map(|column| column.target.as_str()).collect();

        assert_eq!(
            targets,
            vec![
                "summary",
                "key",
                "id",
                "type",
                "status",
                "projectKey",
                "projectName",
                "priority",
                "assignee",
                "reporter",
                "created",
                "updated",
                "resolved",
                "description",
                "labels",
                "labels",
                "sprint",
                "sprint",
                "comment",
                "comment",
                "points",
                "parent",
                "linkOutward",
                "worklog",
                "attachment",
                "ignore",
            ]
        );
        assert_eq!(defaults[1].sample, "SHOP-1");

        let chosen = [
            JiraCsvColumn { index: 25, name: String::new(), sample: String::new(), target: "labels".into() },
            JiraCsvColumn { index: 7, name: String::new(), sample: String::new(), target: "nonsense".into() },
        ];
        let adjusted = columns(&table, Some(&chosen));

        assert_eq!(adjusted[25].target, "labels");
        assert_eq!(adjusted[7].target, "priority");
        assert_eq!(projects(&table, &defaults), vec![("SHOP".to_string(), 3), ("OPS".to_string(), 1)]);
    }

    #[test]
    fn rows_become_issues_with_repeated_columns_collected() {
        let table = parse(EXPORT);
        let parsed = read(&table, &columns(&table, None), "SHOP", None, "2024-06-01T00:00:00.000Z");
        let keys: Vec<&str> = parsed.issues.iter().map(|issue| issue.key.as_str()).collect();

        assert_eq!(keys, vec!["SHOP-1", "SHOP-2", "SHOP-3"]);
        assert_eq!(parsed.problems, vec!["Row 6: \"not a key\" is not an issue key".to_string()]);

        let epic = &parsed.issues[0];
        assert_eq!(epic.summary, "Checkout, the \"new\" one");
        assert_eq!(epic.labels, vec!["checkout", "q2", "payments"]);
        assert_eq!(epic.created, "2024-03-01T09:00:00.000Z");
        assert_eq!(epic.updated, "2024-03-05T14:30:00.000Z");
        assert_eq!(epic.description.to_markdown(), "## Goal\n\nA **better** checkout,\\\nwith fewer steps.");
        assert_eq!(epic.comments.len(), 2);
        assert_eq!(epic.comments[0].created, "2024-03-02T10:15:00.000Z");
        assert_eq!(epic.comments[0].author.as_ref().unwrap().account, "jkowalski");
        assert_eq!(epic.comments[0].body, Body::Wiki("First line\nsecond line; with a semicolon".into()));
        assert_eq!((epic.comments[1].author.clone(), epic.comments[1].created.as_str()), (None, "2024-03-05T14:30:00.000Z"));

        let story = &parsed.issues[1];
        assert_eq!(story.parent_key.as_deref(), Some("SHOP-1"));
        assert_eq!(story.points, Some(5.0));
        assert_eq!(story.resolved.as_deref(), Some("2024-03-09T16:00:00.000Z"));
        assert_eq!(
            story.sprints.iter().map(|sprint| (sprint.name.as_str(), sprint.state.as_str())).collect::<Vec<_>>(),
            vec![("Sprint 1", "closed"), ("Sprint 2", "future")]
        );
        assert_eq!(story.links, vec![RemoteLink { type_name: "Blocks".into(), outward: true, other_key: "SHOP-3".into() }]);
        assert_eq!((story.worklogs[0].seconds, story.worklogs[0].started.as_str()), (5400, "2024-03-08T13:00:00.000Z"));
        assert_eq!(
            story.attachments[0],
            RemoteAttachment { filename: "spec.pdf".into(), url: "https://jira.example.com/secure/attachment/7/spec.pdf".into(), size: None }
        );
        assert_eq!(parsed.issues[2].created, "2024-03-03T08:00:00.000Z");

        let structure = &parsed.structure;
        assert_eq!(structure.name, "Web shop");
        assert_eq!(structure.statuses.iter().map(|status| status.name.as_str()).collect::<Vec<_>>(), vec!["To Do", "In Progress", "Done"]);
        assert_eq!(structure.types.iter().map(|kind| (kind.name.as_str(), kind.epic)).collect::<Vec<_>>(), vec![("Epic", true), ("Story", false)]);
        assert_eq!(structure.priorities, vec!["High", "Medium"]);
        assert_eq!(structure.people.iter().map(|person| person.name.as_str()).collect::<Vec<_>>(), vec!["Anna Nowak", "Jan Kowalski", "jkowalski"]);
        assert_eq!(structure.sprints.len(), 2);
        assert!(structure.scrum);
    }

    #[test]
    fn dates_in_the_formats_jira_exports() {
        assert_eq!(csv_instant("12/Mar/24 9:15 AM").as_deref(), Some("2024-03-12T09:15:00.000Z"));
        assert_eq!(csv_instant("12/Mar/2024 21:15").as_deref(), Some("2024-03-12T21:15:00.000Z"));
        assert_eq!(csv_instant("2024-03-12T09:15:30.000+0100").as_deref(), Some("2024-03-12T08:15:30.000Z"));
        assert_eq!(csv_instant("12/Mar/24").as_deref(), Some("2024-03-12T00:00:00.000Z"));
        assert_eq!(csv_instant("12.03.2024 09:15").as_deref(), Some("2024-03-12T09:15:00.000Z"));
        assert_eq!(csv_instant("soon"), None);
        assert_eq!(csv_instant(""), None);
    }
}
