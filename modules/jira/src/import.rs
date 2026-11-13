//! The import: a project's configuration first, then its issues page by page, then what
//! connects them (parents, links). It runs on its own thread as a job, can be run again at
//! any time, and only ever adds to or updates what it brought itself.
//!
//! Each page of issues is fetched on the job's thread (network), converted to Markdown on
//! the worker pool (computation) and written in one transaction.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;

use hub_app::access::ProjectAccess;
use hub_app::db::{now_iso, query_all, query_one};
use hub_app::services::history::FieldChange;
use hub_app::services::issues::{import_change, import_comment, import_issue, import_worklog, reindex_issue, update_comment, ImportedIssue};
use hub_app::services::projects::{
    add_component, add_status, add_type, create_project, delete_status, delete_type, update_status, ComponentInput, NewProject, StatusInput,
    TypeInput,
};
use hub_app::services::sprints::{create_sprint, SprintInput};
use hub_app::services::users::{create_user, update_user, NewUser, UserPatch};
use hub_app::services::versions::{create_version, VersionInput};
use hub_app::{Actor, ApiError, ApiResult, App, AuthUser, Tx};
use hub_core::keys::{is_project_key, parse_issue_key};
use hub_core::text::{slugify, truncate};
use hub_core::{Id, LinkType, ProjectRole, Role, StatusCategory, TypeLevel, DEFAULT_PRIORITY};
use rayon::prelude::*;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::client::{load_settings, Client, SearchCursor, PAGE_SIZE};
use crate::csv::Parsed;
use crate::jobs;
use crate::mapping::{self, clean_name, default_priority, default_type, fold, guess_category, type_icon, Structure};
use crate::remote::{self, Fields, RemoteHistory, RemoteIssue, RemoteSprint, RemoteType, RemoteUser, RemoteVersion};
use crate::types::{JiraJobState, JiraJobStep, JiraMapping, JiraNote, JiraReport};

const LABEL: &str = "Jira";
const MAX_SUMMARY: usize = 255;
/// Lists in a report stop here; the counters keep counting.
const MAX_NOTES: usize = 500;

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub into_existing: bool,
    pub overwrite_local: bool,
    pub full: bool,
}

pub struct Request {
    pub job_id: Id,
    pub admin: AuthUser,
    pub project_key: String,
    pub options: Options,
    pub mapping: Option<JiraMapping>,
}

pub enum Source {
    /// Jira's REST API, with the stored address and token.
    Api,
    /// An uploaded CSV export, already read.
    Csv(Box<Parsed>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum End {
    Done,
    Cancelled,
}

/// What is remembered about a project that was imported before.
#[derive(Debug, Clone, Default)]
pub struct Stored {
    /// The newest `updated` Jira reported, as Jira wrote it.
    pub cursor: Option<String>,
    pub mapping: Option<JiraMapping>,
}

pub fn stored(connection: &Connection, project_key: &str) -> ApiResult<Option<Stored>> {
    query_one(
        connection,
        "SELECT j.cursor, j.mapping FROM jira_project j JOIN project p ON p.id = j.project_id WHERE p.key = ?1 COLLATE NOCASE",
        [project_key],
        |row| {
            let mapping: String = row.get(1)?;

            Ok(Stored { cursor: row.get(0)?, mapping: serde_json::from_str(&mapping).ok() })
        },
    )
}

/// Runs the job on a thread of its own: it blocks on the network and may take long.
pub fn spawn(app: &App, request: Request, source: Source) -> ApiResult<()> {
    let app = app.clone();
    let job_id = request.job_id;

    std::thread::Builder::new()
        .name("jira-import".to_string())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| match source {
                Source::Api => run_api(&app, &request),
                Source::Csv(parsed) => run_csv(&app, &request, *parsed),
            }));
            let (state, error) = match outcome {
                Ok(Ok(End::Done)) => (JiraJobState::Done, None),
                Ok(Ok(End::Cancelled)) => (JiraJobState::Cancelled, None),
                Ok(Err(error)) => (JiraJobState::Failed, Some(error.message)),
                Err(_) => (JiraJobState::Failed, Some("The import stopped unexpectedly; what it had imported is kept".to_string())),
            };

            if let Err(error) = jobs::finish(&app, job_id, state, error.as_deref()) {
                tracing::error!(job = job_id, "could not record the end of a Jira import: {}", error.message);
            }
        })
        .map(|_| ())
        .map_err(|error| ApiError::internal(format!("could not start the import: {error}")))
}

fn announce(app: &App, request: &Request, step: JiraJobStep) -> ApiResult<()> {
    app.write(Actor::system(LABEL), |tx| jobs::progress(tx, request.job_id, step, None, 0, &JiraReport::default()))
}

fn jql(project_key: &str, since: Option<&str>) -> String {
    // The cursor is Jira's own text (`2024-03-12T09:15:30.000+0100`), in the time zone JQL
    // reads dates in: the one of the account behind the token.
    let since = since.and_then(|cursor| cursor.get(..16)).map(|minute| minute.replace('T', " ")).filter(|minute| remote::instant(minute).is_some());

    match since {
        Some(minute) => format!("project = \"{project_key}\" AND updated >= \"{minute}\" ORDER BY key ASC"),
        None => format!("project = \"{project_key}\" ORDER BY key ASC"),
    }
}

/// Fetches what a search result left out: Jira caps the comments, work logs and changelog
/// entries it embeds. A failure here is reported on the issue rather than stopping the import.
fn complete(client: &Client, issue: &mut RemoteIssue) -> Option<String> {
    let mut problem = None;

    if issue.comment_total > issue.comments.len() {
        match client.comments(&issue.key) {
            Ok(comments) => issue.comments = comments,
            Err(error) => problem = Some(format!("only the first {} comments came through: {}", issue.comments.len(), error.message)),
        }
    }

    if issue.worklog_total > issue.worklogs.len() {
        match client.worklogs(&issue.key) {
            Ok(worklogs) => issue.worklogs = worklogs,
            Err(error) => problem = Some(format!("only the first {} work logs came through: {}", issue.worklogs.len(), error.message)),
        }
    }

    if issue.history_total > issue.history.len() {
        match client.changelog(&issue.key) {
            Ok(history) => issue.history = history,
            Err(error) => problem = Some(format!("its history is incomplete: {}", error.message)),
        }
    }

    problem
}

fn run_api(app: &App, request: &Request) -> ApiResult<End> {
    announce(app, request, JiraJobStep::Connecting)?;

    let mut client = app.read(|connection| Client::new(&load_settings(connection, &app.config.secret_key)?))?;
    client.detect()?;

    announce(app, request, JiraJobStep::Structure)?;

    let structure = mapping::fetch(&client, &request.project_key)?;
    let fields = client.fields()?;
    let known = app.read(|connection| stored(connection, &structure.key))?.unwrap_or_default();
    let mut importer = Importer::prepare(app, request, &structure, known.mapping.as_ref(), fields.clone(), client.base(), true)?;
    let query = jql(&structure.key, if request.options.full { None } else { known.cursor.as_deref() });

    importer.total = client.count(&query);
    importer.save(app, JiraJobStep::Issues)?;

    let pages = (|| {
        let mut cursor = SearchCursor::default();

        while !cursor.done {
            if app.read(|connection| jobs::cancel_requested(connection, request.job_id))? {
                return Ok(End::Cancelled);
            }

            let page = client.search(&query, &fields, &mut cursor)?;
            let mut issues = Vec::with_capacity(page.issues.len());

            importer.total = importer.total.or(page.total);

            for value in &page.issues {
                let key = value["key"].as_str().unwrap_or("?").to_string();

                match remote::issue(value, &fields) {
                    Some(mut issue) => {
                        if let Some(problem) = complete(&client, &mut issue) {
                            importer.note(format!("{key}: {problem}"));
                        }

                        issues.push(issue);
                    }
                    None => importer.fail(&key, "Jira sent it without a key or a creation date"),
                }
            }

            importer.store_page(app, &issues)?;
        }

        Ok(End::Done)
    })();

    importer.close(app, pages)
}

fn run_csv(app: &App, request: &Request, parsed: Parsed) -> ApiResult<End> {
    announce(app, request, JiraJobStep::Structure)?;

    let known = app.read(|connection| stored(connection, &parsed.structure.key))?.unwrap_or_default();
    // With an address configured the issues link back to Jira; without one they still import.
    let base_url = app.read(|connection| load_settings(connection, &app.config.secret_key))?.base_url;
    let mut importer = Importer::prepare(app, request, &parsed.structure, known.mapping.as_ref(), Fields::default(), &base_url, false)?;

    parsed.problems.iter().take(50).for_each(|problem| importer.note(problem.clone()));
    importer.total = Some(parsed.issues.len() as i64);
    importer.save(app, JiraJobStep::Issues)?;

    let pages = (|| {
        for page in parsed.issues.chunks(PAGE_SIZE) {
            if app.read(|connection| jobs::cancel_requested(connection, request.job_id))? {
                return Ok(End::Cancelled);
            }

            importer.store_page(app, page)?;
        }

        Ok(End::Done)
    })();

    importer.close(app, pages)
}

/// The Markdown of one issue's texts; made for a whole page of issues at once.
struct Converted {
    description: String,
    comments: Vec<String>,
    worklogs: Vec<String>,
}

impl Converted {
    fn of(issue: &RemoteIssue) -> Self {
        Self {
            description: issue.description.to_markdown(),
            comments: issue.comments.iter().map(|comment| comment.body.to_markdown()).collect(),
            worklogs: issue.worklogs.iter().map(|worklog| worklog.comment.to_markdown()).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LocalStatus {
    id: Id,
    name: String,
    category: StatusCategory,
}

/// Ids looked up while importing. Dropped whenever an issue is rolled back, because what it
/// created on the way (a person, a sprint) is gone with it.
#[derive(Default)]
struct Cache {
    types: HashMap<String, (Id, TypeLevel)>,
    statuses: HashMap<String, LocalStatus>,
    users: HashMap<String, Option<Id>>,
    components: HashMap<String, Id>,
    versions: HashMap<String, Id>,
    sprints: HashMap<String, Option<(Id, String)>>,
}

/// One entry of an issue's history, ready to be written.
struct Step {
    /// Jira's changelog id; zero for a step the import worked out itself.
    history: i64,
    at: String,
    author: Option<RemoteUser>,
    field: &'static str,
    old: Option<String>,
    new: Option<String>,
    old_ref: Option<String>,
    new_ref: Option<String>,
}

impl Step {
    fn text(entry: &RemoteHistory, field: &'static str, old: Option<String>, new: Option<String>) -> Self {
        Self { history: entry.id, at: entry.created.clone(), author: entry.author.clone(), field, old, new, old_ref: None, new_ref: None }
    }

    fn reference(
        history: i64,
        at: &str,
        author: Option<RemoteUser>,
        field: &'static str,
        old: Option<(Id, String)>,
        new: Option<(Id, String)>,
    ) -> Self {
        Self {
            history,
            at: at.to_string(),
            author,
            field,
            old_ref: old.as_ref().map(|(id, _)| id.to_string()),
            new_ref: new.as_ref().map(|(id, _)| id.to_string()),
            old: old.map(|(_, name)| name),
            new: new.map(|(_, name)| name),
        }
    }
}

/// A move between sprints, by Jira's sprint ids: the changelog id (zero when the import worked
/// it out), when, who, from where and to where.
type SprintMove = (i64, String, Option<RemoteUser>, Option<String>, Option<String>);

/// An issue's past and what follows from it for its present.
struct Timeline {
    steps: Vec<Step>,
    sprint: Option<(Id, String)>,
    status_since: String,
    resolved_at: Option<String>,
}

/// An issue whose parent had not been imported yet when it was.
struct Deferred {
    issue: ImportedIssue,
    id: Id,
    key: String,
    parent_key: String,
    updated: String,
}

enum Outcome {
    Created,
    Updated,
    Unchanged,
    Skipped(&'static str),
}

/// How far the report had got, to take back what a rolled-back issue added to it.
struct Marks {
    users_created: usize,
    unmapped: usize,
    counters: [i64; 7],
    deferred: usize,
    links: usize,
}

/// What `jira_issue` says about an issue that was imported before.
struct Known {
    remote_updated: Option<String>,
    fingerprint: String,
    last_history: i64,
    history_through: String,
}

fn link_kind(type_name: &str) -> (LinkType, bool) {
    let name = fold(type_name);

    if name.contains("block") {
        (LinkType::Blocks, true)
    } else if name.contains("duplicat") {
        (LinkType::Duplicates, true)
    } else {
        (LinkType::Relates, name.contains("relate"))
    }
}

/// The sprint an issue is in according to Jira's list of all it has been in: the running
/// one, else a planned one, else the last completed one.
fn pick(sprints: &[RemoteSprint]) -> Option<&RemoteSprint> {
    let last = |state: &str| sprints.iter().rev().find(|sprint| sprint.state == state);

    last("active")
        .or_else(|| last("future"))
        .or_else(|| sprints.iter().filter(|sprint| sprint.state == "closed").max_by(|left, right| left.completed.cmp(&right.completed)))
}

/// The last id of a changelog's `from`/`to` for sprints, which lists every sprint so far.
fn last_id(ids: &Option<String>) -> Option<String> {
    ids.as_deref().and_then(|ids| ids.split(',').map(str::trim).rfind(|id| !id.is_empty())).map(String::from)
}

fn fits(level: TypeLevel, parent: TypeLevel) -> bool {
    match level {
        TypeLevel::Epic => false,
        TypeLevel::Standard => parent == TypeLevel::Epic,
        TypeLevel::Subtask => parent == TypeLevel::Standard,
    }
}

fn file_size(bytes: i64) -> String {
    match bytes {
        ..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", (bytes + 512) / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// A digest of the fields an import writes. When it no longer matches what the import left,
/// somebody has edited the issue here.
fn fingerprint(connection: &Connection, issue_id: Id) -> ApiResult<String> {
    let values: Vec<rusqlite::types::Value> = connection.query_row(
        "SELECT type_id, status_id, priority, summary, description, assignee_id, reporter_id, parent_id, sprint_id, version_id, points,
                due_date, estimate_minutes,
                (SELECT group_concat(label, char(31)) FROM (SELECT label FROM issue_label WHERE issue_id = i.id ORDER BY label COLLATE NOCASE)),
                (SELECT group_concat(component_id) FROM (SELECT component_id FROM issue_component WHERE issue_id = i.id ORDER BY component_id))
         FROM issue i WHERE i.id = ?1",
        [issue_id],
        |row| (0..15).map(|index| row.get(index)).collect(),
    )?;
    let mut hasher = Sha256::new();

    for value in values {
        hasher.update(format!("{value:?}").as_bytes());
        hasher.update([0]);
    }

    Ok(hex::encode(&hasher.finalize()[..16]))
}

struct Importer {
    job_id: Id,
    admin: AuthUser,
    overwrite_local: bool,
    create_users: bool,
    /// False for a CSV file, which says where an issue is but not how it got there.
    has_history: bool,
    project_id: Id,
    project_key: String,
    base_url: String,
    fields: Fields,
    /// Jira's name (folded) to the name and meaning here.
    type_names: HashMap<String, (String, TypeLevel)>,
    status_names: HashMap<String, (String, StatusCategory)>,
    priorities: HashMap<String, i64>,
    user_choices: HashMap<String, Id>,
    sprint_info: HashMap<String, RemoteSprint>,
    off_board: Option<HashSet<String>>,
    cache: Cache,
    matched: HashSet<String>,
    report: JiraReport,
    total: Option<i64>,
    processed: i64,
    deferred: Vec<Deferred>,
    links: Vec<(String, String, LinkType)>,
    /// The newest change Jira reported in this run: as an instant, and as Jira wrote it.
    newest: Option<(String, String)>,
}

impl Importer {
    /// Creates or finds the project and gives it what Jira's project has: statuses in
    /// workflow order, types, components, versions and sprints.
    fn prepare(
        app: &App,
        request: &Request,
        structure: &Structure,
        stored: Option<&JiraMapping>,
        fields: Fields,
        base_url: &str,
        has_history: bool,
    ) -> ApiResult<Self> {
        let key = structure.key.trim().to_uppercase();

        if !is_project_key(&key) {
            return Err(ApiError::bad_request(format!(
                "\"{key}\" cannot be a project key here (2 to 10 letters and digits, starting with a letter), and issue keys must stay as they are"
            )));
        }

        let local_users: Vec<(Id, String)> =
            app.read(|connection| query_all(connection, "SELECT id, email FROM user", [], |row| Ok((row.get(0)?, row.get(1)?))))?;
        let mapping = mapping::resolve(structure, &local_users, stored, request.mapping.as_ref());
        let mut importer = Self {
            job_id: request.job_id,
            admin: request.admin.clone(),
            overwrite_local: request.options.overwrite_local,
            create_users: mapping.create_users,
            has_history,
            project_id: 0,
            project_key: key,
            base_url: base_url.trim_end_matches('/').to_string(),
            fields,
            type_names: mapping.types.iter().map(|entry| (fold(&entry.jira_name), (entry.name.clone(), entry.level))).collect(),
            status_names: mapping.statuses.iter().map(|entry| (fold(&entry.jira_name), (entry.name.clone(), entry.category))).collect(),
            priorities: mapping.priorities.iter().map(|entry| (fold(&entry.jira_name), entry.priority.clamp(1, 5))).collect(),
            user_choices: mapping.users.iter().filter_map(|entry| Some((entry.account.clone(), entry.user_id?))).collect(),
            sprint_info: structure.sprints.iter().map(|sprint| (sprint.id.clone(), sprint.clone())).collect(),
            off_board: structure.off_board.clone(),
            cache: Cache::default(),
            matched: HashSet::new(),
            report: JiraReport::default(),
            total: None,
            processed: 0,
            deferred: Vec::new(),
            links: Vec::new(),
            newest: None,
        };

        app.write(Actor::user_via(request.admin.id, LABEL), |tx| importer.structure(tx, request, structure, &mapping))?;

        Ok(importer)
    }

    fn access(&self) -> ProjectAccess {
        ProjectAccess { id: self.project_id, key: self.project_key.clone(), role: ProjectRole::Admin }
    }

    fn note(&mut self, text: String) {
        if self.report.unmapped.len() < MAX_NOTES && !self.report.unmapped.contains(&text) {
            self.report.unmapped.push(text);
        }
    }

    fn fail(&mut self, key: &str, reason: &str) {
        self.processed += 1;
        self.report.failed.push(JiraNote { key: key.to_string(), reason: reason.to_string() });
    }

    fn save(&self, app: &App, step: JiraJobStep) -> ApiResult<()> {
        app.write(Actor::system(LABEL), |tx| jobs::progress(tx, self.job_id, step, self.total, self.processed, &self.report))
    }

    fn structure(&mut self, tx: &Tx, request: &Request, structure: &Structure, mapping: &JiraMapping) -> ApiResult<()> {
        let key = self.project_key.clone();
        let existing: Option<Id> = query_one(tx, "SELECT id FROM project WHERE key = ?1", [&key], |row| row.get(0))?;

        self.project_id = match existing {
            Some(id) => {
                let imported: Option<Id> = query_one(tx, "SELECT project_id FROM jira_project WHERE project_id = ?1", [id], |row| row.get(0))?;

                if imported.is_none() && !request.options.into_existing {
                    return Err(ApiError::conflict(format!(
                        "A project with the key {key} already exists here. Choose “import into the existing project” to add Jira's issues to it."
                    )));
                }

                id
            }
            None => {
                let lead_email = structure.lead.as_ref().and_then(|lead| lead.email.clone()).unwrap_or_default();
                let lead: Option<Id> = query_one(tx, "SELECT id FROM user WHERE email = ?1 AND active = 1", [lead_email.trim()], |row| row.get(0))?;
                let name = clean_name(&structure.name);
                let new = NewProject {
                    key: key.clone(),
                    name: if name.is_empty() { key.clone() } else { name },
                    description: truncate(structure.description.trim(), 2000),
                    board_mode: mapping.board_mode,
                    is_private: false,
                    lead_id: lead,
                };

                self.report.project_created = true;

                create_project(tx, &self.admin, &new)?.summary.id
            }
        };

        let access = self.access();
        let created = self.report.project_created;

        // Statuses, in the order of the mapping, which is Jira's workflow order. Several of
        // Jira's statuses may share one here.
        let mut targets: Vec<(String, StatusCategory, bool)> = Vec::new();

        for entry in &mapping.statuses {
            let on_board = self.off_board.as_ref().is_none_or(|off_board| !off_board.contains(&fold(&entry.jira_name)));
            let name = clean_name(&entry.name);

            match targets.iter_mut().find(|(known, _, _)| fold(known) == fold(&name)) {
                Some(target) => target.2 |= on_board,
                None if !name.is_empty() => targets.push((name, entry.category, on_board)),
                None => {}
            }
        }

        for (position, (name, category, on_board)) in targets.iter().enumerate() {
            let existing: Option<Id> =
                query_one(tx, "SELECT id FROM status WHERE project_id = ?1 AND name = ?2", (self.project_id, name), |row| row.get(0))?;
            let position = created.then_some(position as i64);
            let input = StatusInput { name: Some(name.clone()), category: Some(*category), on_board: Some(*on_board), wip_limit: None, position };

            match existing {
                // A status the new project came with: it takes Jira's place and meaning.
                Some(id) if created => update_status(tx, &access, id, &StatusInput { name: None, ..input })?,
                Some(_) => {}
                None => {
                    add_status(tx, &access, &input)?;
                }
            }
        }

        if created && !targets.is_empty() {
            let defaults: Vec<(Id, String)> =
                query_all(tx, "SELECT id, name FROM status WHERE project_id = ?1", [self.project_id], |row| Ok((row.get(0)?, row.get(1)?)))?;

            for (id, name) in defaults {
                if !targets.iter().any(|(target, _, _)| fold(target) == fold(&name)) {
                    delete_status(tx, &access, id, None)?;
                }
            }
        }

        // Types. A new project keeps its epic and subtask types even when Jira's project has
        // none, so the levels stay available; unused ordinary types go.
        let wanted: Vec<String> = mapping.types.iter().map(|entry| entry.jira_name.clone()).collect();

        for jira_name in &wanted {
            self.issue_type(tx, jira_name)?;
        }

        if created && mapping.types.iter().any(|entry| entry.level == TypeLevel::Standard) {
            let defaults: Vec<(Id, String)> =
                query_all(tx, "SELECT id, name FROM issue_type WHERE project_id = ?1 AND level = 'standard'", [self.project_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?;

            for (id, name) in defaults {
                if !mapping.types.iter().any(|entry| fold(&entry.name) == fold(&name)) {
                    delete_type(tx, &access, id, None)?;
                }
            }

            self.cache.types.clear();
        }

        for (name, description) in &structure.components {
            self.component(tx, name, description)?;
        }

        for version in &structure.versions {
            self.version(tx, version)?;
        }

        let mut sprints: Vec<&RemoteSprint> = structure.sprints.iter().collect();
        sprints
            .sort_by_key(|sprint| (sprint.start.is_none() && sprint.state != "closed", sprint.start.clone(), sprint.id.parse::<i64>().unwrap_or(0)));

        for sprint in sprints {
            let local = self.ensure_sprint(tx, sprint)?;
            self.cache.sprints.insert(sprint.id.clone(), Some(local));
        }

        tx.execute(
            "INSERT INTO jira_project (project_id, remote_key, base_url, mapping) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (project_id) DO UPDATE SET remote_key = ?2, base_url = CASE WHEN ?3 = '' THEN base_url ELSE ?3 END, mapping = ?4",
            (self.project_id, &structure.key, &self.base_url, serde_json::to_string(mapping)?),
        )?;

        Ok(())
    }

    /// The person here behind a person of Jira: the administrator's choice, whoever they
    /// were matched to before, the account with their e-mail address, or (when the import may
    /// create accounts) a new inactive one. None means only the name can be kept.
    fn user(&mut self, tx: &Tx, person: &RemoteUser) -> ApiResult<Option<Id>> {
        if let Some(known) = self.cache.users.get(&person.account) {
            return Ok(*known);
        }

        let exists = |id: Id| -> ApiResult<Option<Id>> { query_one(tx, "SELECT id FROM user WHERE id = ?1", [id], |row| row.get(0)) };
        let mut found = match self.user_choices.get(&person.account) {
            Some(id) => exists(*id)?,
            None => None,
        };

        if found.is_none() {
            found = query_one(tx, "SELECT user_id FROM jira_user WHERE account = ?1", [&person.account], |row| row.get(0))?;
        }

        if found.is_none() {
            if let Some(email) = person.email.as_deref().map(str::trim).filter(|email| !email.is_empty()) {
                found = query_one(tx, "SELECT id FROM user WHERE email = ?1", [email], |row| row.get(0))?;
            }
        }

        if found.is_some() && self.matched.insert(person.account.clone()) {
            self.report.users_matched += 1;
        }

        if found.is_none() && self.create_users {
            found = Some(self.create_account(tx, person)?);
            self.matched.insert(person.account.clone());
        }

        if let Some(id) = found {
            tx.execute(
                "INSERT INTO jira_user (account, user_id, name) VALUES (?1, ?2, ?3) ON CONFLICT (account) DO UPDATE SET user_id = ?2, name = ?3",
                (&person.account, id, &person.name),
            )?;
        }

        self.cache.users.insert(person.account.clone(), found);

        Ok(found)
    }

    /// An account for somebody who only exists in Jira: no password and switched off, so
    /// nobody can sign in with it, but their name stays on what they wrote and did.
    fn create_account(&mut self, tx: &Tx, person: &RemoteUser) -> ApiResult<Id> {
        let name: String = person.name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
        let name = if name.is_empty() { person.account.chars().take(80).collect() } else { name };
        let new = |email: String| NewUser { name: name.clone(), email, password: None, role: Some(Role::Member), github_login: None };
        let slug = slugify(&person.account, 48);
        // Jira Cloud rarely tells e-mail addresses; an address that cannot exist stands in.
        let placeholder =
            format!("jira-{}@imported.invalid", if slug.is_empty() { hex::encode(&Sha256::digest(person.account.as_bytes())[..8]) } else { slug });
        let user = match person.email.as_deref() {
            Some(email) => create_user(tx, &new(email.to_string())).or_else(|_| create_user(tx, &new(placeholder)))?,
            None => create_user(tx, &new(placeholder))?,
        };

        update_user(tx, &self.admin, user.id, &UserPatch { active: Some(false), ..Default::default() })?;
        self.report.users_created.push(user.name);

        Ok(user.id)
    }

    /// Who did something, for a comment or a history entry.
    fn actor(&mut self, tx: &Tx, person: Option<&RemoteUser>) -> ApiResult<Actor> {
        let Some(person) = person else { return Ok(Actor::system(LABEL)) };

        Ok(match self.user(tx, person)? {
            Some(id) => Actor::user(id),
            None => Actor::system(format!("{LABEL} · {}", person.name)),
        })
    }

    fn issue_type(&mut self, tx: &Tx, jira_name: &str) -> ApiResult<(Id, TypeLevel)> {
        let jira_name = if jira_name.trim().is_empty() { "Task" } else { jira_name.trim() };
        let folded = fold(jira_name);

        if let Some(found) = self.cache.types.get(&folded) {
            return Ok(*found);
        }

        let (name, level) = match self.type_names.get(&folded) {
            Some(known) => known.clone(),
            None => {
                let default = default_type(&RemoteType { name: jira_name.to_string(), subtask: false, epic: folded == "epic" });

                self.note(format!("The issue type “{jira_name}” was not in the mapping; it was added as “{}”", default.name));
                self.type_names.insert(folded.clone(), (default.name.clone(), default.level));

                (default.name, default.level)
            }
        };
        let name = if clean_name(&name).is_empty() { "Task".to_string() } else { clean_name(&name) };
        let existing: Option<(Id, String)> =
            query_one(tx, "SELECT id, level FROM issue_type WHERE project_id = ?1 AND name = ?2", (self.project_id, &name), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        let found = match existing {
            Some((id, actual)) => {
                let actual = TypeLevel::parse(&actual).unwrap_or(TypeLevel::Standard);

                if actual != level {
                    self.note(format!("The type “{name}” already exists here on the {actual} level; that level is kept"));
                }

                (id, actual)
            }
            None => {
                let input = TypeInput { name: Some(name.clone()), level: Some(level), icon: Some(type_icon(&name, level).to_string()) };

                (add_type(tx, &self.access(), &input)?, level)
            }
        };

        self.cache.types.insert(folded, found);

        Ok(found)
    }

    fn status(&mut self, tx: &Tx, jira_name: &str) -> ApiResult<LocalStatus> {
        let jira_name = if jira_name.trim().is_empty() { "To Do" } else { jira_name.trim() };
        let folded = fold(jira_name);

        if let Some(found) = self.cache.statuses.get(&folded) {
            return Ok(found.clone());
        }

        let mapped = self.status_names.get(&folded).cloned();
        let (name, category) = mapped.clone().unwrap_or_else(|| (clean_name(jira_name), guess_category(jira_name)));
        let name = if name.is_empty() { "To Do".to_string() } else { name };
        let existing: Option<(Id, String, String)> =
            query_one(tx, "SELECT id, name, category FROM status WHERE project_id = ?1 AND name = ?2", (self.project_id, &name), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        let found = match existing {
            Some((id, name, category)) => LocalStatus { id, name, category: StatusCategory::parse(&category).unwrap_or(StatusCategory::InProgress) },
            None => {
                // A status that only the history knows (Jira's workflow has dropped it since)
                // is kept, off the board, so the history still says where the issue was.
                let input = StatusInput {
                    name: Some(name.clone()),
                    category: Some(category),
                    on_board: Some(mapped.is_some()),
                    wip_limit: None,
                    position: None,
                };
                let id = add_status(tx, &self.access(), &input)?;

                if mapped.is_none() {
                    self.note(format!(
                        "The status “{jira_name}” is not in the project's workflow any more; it was kept for the history, off the board"
                    ));
                }

                LocalStatus { id, name, category }
            }
        };

        self.cache.statuses.insert(folded, found.clone());

        Ok(found)
    }

    fn component(&mut self, tx: &Tx, name: &str, description: &str) -> ApiResult<Option<Id>> {
        let name = clean_name(name);
        let folded = fold(&name);

        if name.is_empty() {
            return Ok(None);
        }

        if let Some(found) = self.cache.components.get(&folded) {
            return Ok(Some(*found));
        }

        let existing: Option<Id> =
            query_one(tx, "SELECT id FROM component WHERE project_id = ?1 AND name = ?2", (self.project_id, &name), |row| row.get(0))?;
        let id = match existing {
            Some(id) => id,
            None => {
                self.report.components += 1;

                add_component(tx, &self.access(), &ComponentInput { name: Some(name), description: Some(truncate(description, 500)), lead_id: None })?
            }
        };

        self.cache.components.insert(folded, id);

        Ok(Some(id))
    }

    fn version(&mut self, tx: &Tx, version: &RemoteVersion) -> ApiResult<Option<Id>> {
        let name = version.name.split_whitespace().collect::<Vec<_>>().join(" ");
        let folded = fold(&name);

        if name.is_empty() {
            return Ok(None);
        }

        if let Some(found) = self.cache.versions.get(&folded) {
            return Ok(Some(*found));
        }

        let existing: Option<Id> =
            query_one(tx, "SELECT id FROM version WHERE project_id = ?1 AND name = ?2", (self.project_id, &name), |row| row.get(0))?;
        let id = match existing {
            Some(id) => {
                // Released in Jira since the last import.
                if version.released {
                    let released_at = version.release_date.as_ref().map(|date| format!("{date}T12:00:00.000Z")).unwrap_or_else(|| tx.now.clone());

                    tx.execute("UPDATE version SET released = 1, released_at = ?1 WHERE id = ?2 AND released = 0", (released_at, id))?;
                }

                id
            }
            None => {
                let input = VersionInput {
                    name: Some(name),
                    description: Some(truncate(&version.description, 500)),
                    start_date: version.start_date.clone(),
                    release_date: version.release_date.clone(),
                    archived: None,
                };
                let id = create_version(tx, &self.access(), &input)?.id;

                // A release that happened in Jira happened then, not now: the service would
                // stamp it with the present, so the imported state is written as it was.
                if version.released || version.archived {
                    let released_at = version.release_date.as_ref().map(|date| format!("{date}T12:00:00.000Z")).unwrap_or_else(|| tx.now.clone());

                    tx.execute(
                        "UPDATE version SET released = ?1, released_at = ?2, archived = ?3 WHERE id = ?4",
                        (version.released, version.released.then_some(released_at), version.archived, id),
                    )?;
                }

                self.report.versions += 1;

                id
            }
        };

        self.cache.versions.insert(folded, id);

        Ok(Some(id))
    }

    /// The sprint here for one of Jira's, created or brought up to date with Jira's state.
    fn ensure_sprint(&mut self, tx: &Tx, sprint: &RemoteSprint) -> ApiResult<(Id, String)> {
        let name = sprint.name.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = if name.is_empty() { format!("Sprint {}", sprint.id) } else { name };
        let mapped: Option<Id> =
            query_one(tx, "SELECT sprint_id FROM jira_sprint WHERE project_id = ?1 AND remote_id = ?2", (self.project_id, &sprint.id), |row| {
                row.get(0)
            })?;
        let id = match mapped {
            Some(id) => id,
            None => {
                // In a project that existed before, a sprint of the same name is the same sprint.
                let same_name: Option<Id> = query_one(
                    tx,
                    "SELECT id FROM sprint WHERE project_id = ?1 AND name = ?2 AND id NOT IN (SELECT sprint_id FROM jira_sprint WHERE project_id = ?1)",
                    (self.project_id, &name),
                    |row| row.get(0),
                )?;
                let id = match same_name {
                    Some(id) => id,
                    None => {
                        self.report.sprints += 1;

                        create_sprint(tx, &self.access(), &SprintInput { name: Some(name.clone()), ..Default::default() })?.id
                    }
                };

                tx.execute("INSERT INTO jira_sprint (project_id, remote_id, sprint_id) VALUES (?1, ?2, ?3)", (self.project_id, &sprint.id, id))?;

                id
            }
        };
        let running: Option<String> =
            query_one(tx, "SELECT name FROM sprint WHERE project_id = ?1 AND state = 'active' AND id != ?2 LIMIT 1", (self.project_id, id), |row| {
                row.get(0)
            })?;
        let state = match (sprint.state.as_str(), running) {
            ("active", Some(running)) => {
                self.note(format!("Jira runs “{running}” and “{name}” at the same time; a project here runs one sprint at a time, so “{name}” is planned, not running"));
                "future"
            }
            ("active", None) => "active",
            ("closed", _) => "closed",
            _ => "future",
        };
        let day = |instant: &Option<String>| instant.as_deref().and_then(remote::day);
        let completed = (state == "closed").then(|| sprint.completed.clone().or(sprint.end.clone())).flatten();

        // The services start and complete sprints at the present moment; a sprint that ran
        // in Jira keeps Jira's state and dates, so they are written as they were.
        tx.execute(
            "UPDATE sprint SET name = ?1, goal = ?2, state = ?3, start_date = ?4, end_date = ?5, completed_at = ?6 WHERE id = ?7",
            (&name, sprint.goal.trim(), state, day(&sprint.start), day(&sprint.end), completed, id),
        )?;

        Ok((id, name))
    }

    fn sprint(&mut self, tx: &Tx, remote_id: &str) -> ApiResult<Option<(Id, String)>> {
        if let Some(found) = self.cache.sprints.get(remote_id) {
            return Ok(found.clone());
        }

        let mapped: Option<(Id, String)> = query_one(
            tx,
            "SELECT s.id, s.name FROM jira_sprint j JOIN sprint s ON s.id = j.sprint_id WHERE j.project_id = ?1 AND j.remote_id = ?2",
            (self.project_id, remote_id),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let found = match (mapped, self.sprint_info.get(remote_id).cloned()) {
            (Some(found), _) => Some(found),
            // A sprint of another board that the issue has been in.
            (None, Some(info)) => Some(self.ensure_sprint(tx, &info)?),
            (None, None) => None,
        };

        self.cache.sprints.insert(remote_id.to_string(), found.clone());

        Ok(found)
    }

    /// When the sprint was completed, if it has been.
    fn closed_at(&self, remote_id: &str) -> Option<String> {
        let sprint = self.sprint_info.get(remote_id).filter(|sprint| sprint.state == "closed")?;

        sprint.completed.clone().or(sprint.end.clone())
    }

    /// Converts a page of issues (in parallel) and writes it in one transaction. An issue
    /// that cannot be stored is rolled back on its own and reported; the others go through.
    fn store_page(&mut self, app: &App, issues: &[RemoteIssue]) -> ApiResult<()> {
        let converted: Vec<Converted> = issues.par_iter().map(Converted::of).collect();

        app.write(Actor::system(LABEL), |tx| {
            for (issue, converted) in issues.iter().zip(&converted) {
                self.store_one(tx, issue, converted)?;
            }

            jobs::progress(tx, self.job_id, JiraJobStep::Issues, self.total, self.processed, &self.report)
        })
    }

    fn marks(&self) -> Marks {
        let report = &self.report;

        Marks {
            users_created: report.users_created.len(),
            unmapped: report.unmapped.len(),
            counters: [report.comments, report.worklogs, report.changes, report.sprints, report.versions, report.components, report.users_matched],
            deferred: self.deferred.len(),
            links: self.links.len(),
        }
    }

    fn rewind(&mut self, marks: Marks) {
        let report = &mut self.report;

        report.users_created.truncate(marks.users_created);
        report.unmapped.truncate(marks.unmapped);
        [report.comments, report.worklogs, report.changes, report.sprints, report.versions, report.components, report.users_matched] = marks.counters;
        self.deferred.truncate(marks.deferred);
        self.links.truncate(marks.links);
        self.matched.clear();
        self.cache = Cache::default();
    }

    fn store_one(&mut self, tx: &Tx, issue: &RemoteIssue, converted: &Converted) -> ApiResult<()> {
        let marks = self.marks();

        tx.execute_batch("SAVEPOINT jira_issue")?;

        match self.store_issue(tx, issue, converted) {
            Ok(outcome) => {
                tx.execute_batch("RELEASE jira_issue")?;

                match outcome {
                    Outcome::Created => self.report.created += 1,
                    Outcome::Updated => self.report.updated += 1,
                    Outcome::Unchanged => self.report.unchanged += 1,
                    Outcome::Skipped(reason) => self.report.skipped.push(JiraNote { key: issue.key.clone(), reason: reason.to_string() }),
                }
            }
            Err(error) => {
                tx.execute_batch("ROLLBACK TO jira_issue; RELEASE jira_issue")?;
                self.rewind(marks);
                self.report.failed.push(JiraNote { key: issue.key.clone(), reason: error.message });
            }
        }

        self.processed += 1;

        if self.newest.as_ref().is_none_or(|(newest, _)| *newest < issue.updated) && !issue.updated_raw.is_empty() {
            self.newest = Some((issue.updated.clone(), issue.updated_raw.clone()));
        }

        Ok(())
    }

    /// The issue's history as entries to write, the sprint it ends up in, and since when it
    /// has been in its status.
    fn timeline(&mut self, tx: &Tx, issue: &RemoteIssue, status: &LocalStatus, local: Option<Id>) -> ApiResult<Timeline> {
        let done = status.category == StatusCategory::Done;
        let mut steps: Vec<Step> = Vec::new();
        // Every change of status: when, and whether the issue was done after it.
        let mut moves: Vec<(String, bool)> = Vec::new();
        let mut initially_done = false;
        let mut sprint_changes: Vec<(&RemoteHistory, Option<String>, Option<String>)> = Vec::new();

        for entry in &issue.history {
            for item in &entry.items {
                let field = item.field.to_lowercase();

                if field == "status" {
                    let (Some(from), Some(to)) = (&item.from_text, &item.to_text) else { continue };
                    let (old, new) = (self.status(tx, from)?, self.status(tx, to)?);

                    if moves.is_empty() {
                        initially_done = old.category == StatusCategory::Done;
                    }

                    moves.push((entry.created.clone(), new.category == StatusCategory::Done));

                    if old.id != new.id {
                        steps.push(Step::reference(
                            entry.id,
                            &entry.created,
                            entry.author.clone(),
                            "status",
                            Some((old.id, old.name)),
                            Some((new.id, new.name)),
                        ));
                    }
                } else if field == "sprint" {
                    sprint_changes.push((entry, last_id(&item.from), last_id(&item.to)));
                } else if field == "assignee" {
                    let mut side = |account: &Option<String>, name: &Option<String>| -> ApiResult<(Option<String>, Option<String>)> {
                        let Some(account) = account else { return Ok((name.clone(), None)) };
                        let person = RemoteUser { account: account.clone(), name: name.clone().unwrap_or_else(|| account.clone()), email: None };
                        let id = self.user(tx, &person)?;

                        Ok((Some(person.name), id.map(|id| id.to_string())))
                    };
                    let (old, old_ref) = side(&item.from, &item.from_text)?;
                    let (new, new_ref) = side(&item.to, &item.to_text)?;

                    if old != new || old_ref != new_ref {
                        steps.push(Step { old_ref, new_ref, ..Step::text(entry, "assignee", old, new) });
                    }
                } else if item.from_text != item.to_text {
                    let field = match field.as_str() {
                        _ if self.fields.is_points(item) => "points",
                        "summary" => "summary",
                        "priority" => "priority",
                        "issuetype" => "type",
                        "fix version" => "version",
                        _ => continue,
                    };
                    let short = |text: &Option<String>| text.as_deref().map(|text| truncate(text, 200));

                    steps.push(Step::text(entry, field, short(&item.from_text), short(&item.to_text)));
                }
            }
        }

        // Whether the issue was done at an instant: what decides if it stayed in a sprint
        // that was completed, or went back to the backlog.
        let finished = issue.resolved.clone().unwrap_or_else(|| issue.updated.clone());
        let done_at = |instant: &str| match moves.iter().take_while(|(at, _)| at.as_str() <= instant).last() {
            Some((_, done)) => *done,
            None if moves.is_empty() => done && finished.as_str() <= instant,
            None => initially_done,
        };

        let mut status_since = moves.last().map_or_else(|| issue.created.clone(), |(at, _)| at.clone());
        let mut resolved_at = initially_done.then(|| issue.created.clone());

        for (at, done) in &moves {
            match (resolved_at.is_some(), done) {
                (false, true) => resolved_at = Some(at.clone()),
                (true, false) => resolved_at = None,
                _ => {}
            }
        }

        if !self.has_history {
            // A file says where the issue is, not how it got there. One step from where it
            // was (the first status for a new issue) keeps the reports from showing work as
            // finished on the day it was created.
            let from: Option<(Id, String, String)> = match local {
                Some(id) => query_one(
                    tx,
                    "SELECT s.id, s.name, i.status_since FROM issue i JOIN status s ON s.id = i.status_id WHERE i.id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?,
                None => {
                    query_one(tx, "SELECT id, name, '' FROM status WHERE project_id = ?1 ORDER BY position, id LIMIT 1", [self.project_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?
                }
            };

            match from {
                Some((id, name, _)) if id != status.id => {
                    let at = if done { finished.clone() } else { issue.updated.clone() };

                    steps.push(Step::reference(0, &at, None, "status", Some((id, name)), Some((status.id, status.name.clone()))));
                    status_since = at;
                }
                Some((_, _, since)) if !since.is_empty() => status_since = since,
                _ => {}
            }

            resolved_at = None;
        }

        let resolved_at = done.then(|| resolved_at.or(issue.resolved.clone()).unwrap_or_else(|| status_since.clone()));

        // Sprints. Jira lists every sprint an issue has been in and never says "the sprint
        // ended and this went back to the backlog"; here an issue is in one sprint or none.
        let field = pick(&issue.sprints).map(|sprint| sprint.id.clone());
        let mut current = match sprint_changes.first() {
            Some((_, from, _)) => from.clone(),
            None => field.clone(),
        };
        let mut moved: Vec<SprintMove> = Vec::new();
        let left_behind = |importer: &Self, sprint: &Option<String>, before: Option<&str>| {
            sprint
                .as_deref()
                .and_then(|id| importer.closed_at(id))
                .filter(|closed| before.is_none_or(|before| closed.as_str() < before) && !done_at(closed))
        };

        for (entry, _, to) in &sprint_changes {
            if *to == current {
                continue;
            }

            if let Some(closed) = left_behind(self, &current, Some(&entry.created)) {
                moved.push((0, closed, None, current.take(), None));
            }

            if *to != current {
                moved.push((entry.id, entry.created.clone(), entry.author.clone(), current.clone(), to.clone()));
                current = to.clone();
            }
        }

        if current != field {
            moved.push((0, issue.updated.clone(), None, current.clone(), field.clone()));
            current = field;
        }

        if let Some(closed) = left_behind(self, &current, None) {
            moved.push((0, closed, None, current.take(), None));
        }

        for (history, at, author, old, new) in moved {
            let old = match old {
                Some(id) => self.sprint(tx, &id)?,
                None => None,
            };
            let new = match new {
                Some(id) => self.sprint(tx, &id)?,
                None => None,
            };

            if old.as_ref().map(|(id, _)| id) != new.as_ref().map(|(id, _)| id) {
                steps.push(Step::reference(history, &at, author, "sprint", old, new));
            }
        }

        let sprint = match current {
            Some(id) => self.sprint(tx, &id)?,
            None => None,
        };

        steps.sort_by(|left, right| left.at.cmp(&right.at));

        Ok(Timeline { steps, sprint, status_since, resolved_at })
    }

    fn store_issue(&mut self, tx: &Tx, issue: &RemoteIssue, converted: &Converted) -> ApiResult<Outcome> {
        let local: Option<Id> =
            query_one(tx, "SELECT id FROM issue WHERE project_id = ?1 AND number = ?2", (self.project_id, issue.number), |row| row.get(0))?;
        let known: Option<Known> = match local {
            Some(id) => {
                query_one(tx, "SELECT remote_updated, fingerprint, last_history, history_through FROM jira_issue WHERE issue_id = ?1", [id], |row| {
                    Ok(Known { remote_updated: row.get(0)?, fingerprint: row.get(1)?, last_history: row.get(2)?, history_through: row.get(3)? })
                })?
            }
            None => None,
        };

        match (local, &known) {
            (Some(_), None) if !self.overwrite_local => {
                return Ok(Outcome::Skipped("An issue with this key already exists here and did not come from Jira"))
            }
            (Some(id), Some(known)) => {
                let edited = fingerprint(tx, id)? != known.fingerprint;
                let changed = known.remote_updated.as_deref() != Some(issue.updated.as_str());

                if !changed && !(edited && self.overwrite_local) {
                    return Ok(Outcome::Unchanged);
                }

                if edited && !self.overwrite_local {
                    return Ok(Outcome::Skipped("Edited here after it was imported; the newer version in Jira was not applied"));
                }
            }
            _ => {}
        }

        for sprint in &issue.sprints {
            self.sprint_info.insert(sprint.id.clone(), sprint.clone());
        }

        let (type_id, level) = self.issue_type(tx, &issue.type_name)?;
        let status = self.status(tx, &issue.status_name)?;
        let timeline = self.timeline(tx, issue, &status, local)?;
        let mut parent_id = None;
        let mut waits_for_parent = false;

        if let Some(parent_key) = &issue.parent_key {
            match parse_issue_key(parent_key) {
                Some((project, number)) if project == self.project_key => {
                    let parent: Option<(Id, String)> = query_one(
                        tx,
                        "SELECT i.id, t.level FROM issue i JOIN issue_type t ON t.id = i.type_id WHERE i.project_id = ?1 AND i.number = ?2",
                        (self.project_id, number),
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?;

                    match parent {
                        Some((id, parent_level)) if fits(level, TypeLevel::parse(&parent_level).unwrap_or(TypeLevel::Standard)) => {
                            parent_id = Some(id)
                        }
                        Some(_) => self.note(format!(
                            "{}: {parent_key} cannot be its parent here (epics hold stories, tasks and bugs; those hold subtasks)",
                            issue.key
                        )),
                        None => waits_for_parent = true,
                    }
                }
                _ => self.note(format!("{}: its parent {parent_key} is in another project", issue.key)),
            }
        }

        let mut component_ids = Vec::new();

        for name in &issue.components {
            component_ids.extend(self.component(tx, name, "")?);
        }

        let version_id = match issue.fix_versions.first() {
            Some(name) => self.version(tx, &RemoteVersion { name: name.clone(), ..Default::default() })?,
            None => None,
        };

        if issue.fix_versions.len() > 1 {
            self.note(format!("{}: it has {} fix versions; the first, “{}”, is kept", issue.key, issue.fix_versions.len(), issue.fix_versions[0]));
        }

        let assignee_id = match &issue.assignee {
            Some(person) => self.user(tx, person)?,
            None => None,
        };
        let reporter_id = match &issue.reporter {
            Some(person) => self.user(tx, person)?,
            None => None,
        };
        let priority = match &issue.priority {
            Some(name) => self.priorities.get(&fold(name)).copied().unwrap_or_else(|| default_priority(name, 0, 1)),
            None => DEFAULT_PRIORITY,
        };
        let points = issue.points.filter(|points| (0.0..=1000.0).contains(points));

        if issue.points.is_some() && points.is_none() {
            self.note(format!("{}: {} story points is outside what an estimate can be here", issue.key, issue.points.unwrap_or_default()));
        }

        let mut description = converted.description.clone();

        if !issue.attachments.is_empty() {
            let files: Vec<String> = issue
                .attachments
                .iter()
                .map(|file| {
                    let size = file.size.map(|bytes| format!(" ({})", file_size(bytes))).unwrap_or_default();
                    let name = file.filename.replace(['[', ']'], "");

                    if file.url.is_empty() {
                        format!("- {name}{size}")
                    } else {
                        format!("- [{name}]({}){size}", file.url.replace(' ', "%20").replace(')', "%29"))
                    }
                })
                .collect();

            description = format!("{description}\n\n## Attachments in Jira\n\n{}", files.join("\n")).trim().to_string();
        }

        let summary = issue.summary.split_whitespace().collect::<Vec<_>>().join(" ");
        let imported = ImportedIssue {
            project_id: self.project_id,
            number: issue.number,
            type_id,
            status_id: status.id,
            priority,
            summary: if summary.is_empty() { "(no summary)".to_string() } else { truncate(&summary, MAX_SUMMARY) },
            description,
            assignee_id,
            reporter_id,
            parent_id,
            sprint_id: timeline.sprint.as_ref().map(|(id, _)| *id),
            version_id,
            points,
            due_date: issue.due_date.clone(),
            estimate_minutes: issue.estimate_seconds.map(|seconds| (seconds + 30) / 60),
            labels: issue.labels.clone(),
            component_ids,
            created_at: issue.created.clone(),
            updated_at: issue.updated.clone(),
            status_since: timeline.status_since.clone(),
            resolved_at: timeline.resolved_at.clone(),
        };
        let id = import_issue(tx, &imported)?;

        // History: only what is new since the last import of this issue.
        let last_history = known.as_ref().map_or(0, |known| known.last_history);
        let history_through = known.as_ref().map_or(String::new(), |known| known.history_through.clone());
        let (mut newest_history, mut newest_step) = (last_history, history_through.clone());

        for step in &timeline.steps {
            let fresh = if step.history > 0 { step.history > last_history } else { step.at > history_through };

            if !fresh {
                continue;
            }

            let actor = self.actor(tx, step.author.as_ref())?;
            let change = FieldChange {
                field: step.field,
                old: step.old.clone(),
                new: step.new.clone(),
                old_ref: step.old_ref.clone(),
                new_ref: step.new_ref.clone(),
            };

            import_change(tx, id, &actor, &change, &step.at)?;
            self.report.changes += 1;
            newest_history = newest_history.max(step.history);

            if step.at > newest_step {
                newest_step = step.at.clone();
            }
        }

        let mut touched_comments = false;

        for (comment, body) in issue.comments.iter().zip(&converted.comments) {
            if body.trim().is_empty() {
                continue;
            }

            let before: Option<(Id, Option<String>)> = query_one(
                tx,
                "SELECT local_id, remote_updated FROM jira_item WHERE issue_id = ?1 AND kind = 'comment' AND remote_id = ?2",
                (id, &comment.id),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

            match before {
                None => {
                    let author = self.actor(tx, comment.author.as_ref())?;
                    let local_id = import_comment(tx, id, &author, body, &comment.created, &comment.updated)?;

                    tx.execute(
                        "INSERT INTO jira_item (issue_id, kind, remote_id, local_id, remote_updated) VALUES (?1, 'comment', ?2, ?3, ?4)",
                        (id, &comment.id, local_id, &comment.updated),
                    )?;
                    self.report.comments += 1;
                    touched_comments = true;
                }
                // Edited in Jira since. A comment deleted here stays deleted.
                Some((local_id, seen)) if seen.as_deref() != Some(comment.updated.as_str()) => {
                    let exists: Option<Id> = query_one(tx, "SELECT id FROM comment WHERE id = ?1", [local_id], |row| row.get(0))?;

                    if exists.is_some() {
                        update_comment(tx, &self.admin, local_id, body)?;
                    }

                    tx.execute(
                        "UPDATE jira_item SET remote_updated = ?1 WHERE issue_id = ?2 AND kind = 'comment' AND remote_id = ?3",
                        (&comment.updated, id, &comment.id),
                    )?;
                }
                Some(_) => {}
            }
        }

        for (worklog, note) in issue.worklogs.iter().zip(&converted.worklogs) {
            let before: Option<Id> = query_one(
                tx,
                "SELECT local_id FROM jira_item WHERE issue_id = ?1 AND kind = 'worklog' AND remote_id = ?2",
                (id, &worklog.id),
                |row| row.get(0),
            )?;

            if before.is_none() {
                let user_id = match &worklog.author {
                    Some(person) => self.user(tx, person)?,
                    None => None,
                };
                let local_id = import_worklog(tx, id, user_id, ((worklog.seconds + 30) / 60).max(1), &worklog.started, note)?;

                tx.execute(
                    "INSERT INTO jira_item (issue_id, kind, remote_id, local_id) VALUES (?1, 'worklog', ?2, ?3)",
                    (id, &worklog.id, local_id),
                )?;
                self.report.worklogs += 1;
            }
        }

        if touched_comments {
            reindex_issue(tx, id)?;
        }

        for link in &issue.links {
            let (kind, known_type) = link_kind(&link.type_name);
            let (source, target) =
                if link.outward { (issue.key.clone(), link.other_key.clone()) } else { (link.other_key.clone(), issue.key.clone()) };

            if !known_type {
                self.note(format!("Links of the type “{}” were imported as “relates to”", link.type_name));
            }

            self.links.push((source, target, kind));
        }

        let browse_url = if self.base_url.is_empty() { String::new() } else { format!("{}/browse/{}", self.base_url, issue.key) };

        // Until its parent is in, the issue counts as not imported, so that a run that
        // stops half-way picks it up again.
        tx.execute(
            "INSERT INTO jira_issue (issue_id, remote_id, remote_key, self_url, browse_url, imported_at, remote_updated, fingerprint, last_history, history_through)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (issue_id) DO UPDATE SET remote_id = ?2, remote_key = ?3, self_url = ?4,
                 browse_url = CASE WHEN ?5 = '' THEN browse_url ELSE ?5 END, imported_at = ?6, remote_updated = ?7, fingerprint = ?8,
                 last_history = ?9, history_through = ?10",
            rusqlite::params![
                id,
                issue.id,
                issue.key,
                issue.self_url,
                browse_url,
                tx.now,
                (!waits_for_parent).then_some(&issue.updated),
                fingerprint(tx, id)?,
                newest_history,
                newest_step
            ],
        )?;

        if waits_for_parent {
            let parent_key = issue.parent_key.clone().unwrap_or_default();

            self.deferred.push(Deferred { issue: imported, id, key: issue.key.clone(), parent_key, updated: issue.updated.clone() });
        }

        Ok(if local.is_some() { Outcome::Updated } else { Outcome::Created })
    }

    /// Parents that were imported after their children, and the links between issues.
    fn relations(&mut self, tx: &Tx) -> ApiResult<()> {
        for mut deferred in std::mem::take(&mut self.deferred) {
            let parent: Option<(Id, String)> = match parse_issue_key(&deferred.parent_key) {
                Some((_, number)) => query_one(
                    tx,
                    "SELECT i.id, t.level FROM issue i JOIN issue_type t ON t.id = i.type_id WHERE i.project_id = ?1 AND i.number = ?2",
                    (self.project_id, number),
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?,
                None => None,
            };
            let level: String = tx.query_row("SELECT level FROM issue_type WHERE id = ?1", [deferred.issue.type_id], |row| row.get(0))?;
            let level = TypeLevel::parse(&level).unwrap_or(TypeLevel::Standard);

            match parent {
                Some((parent_id, parent_level)) if fits(level, TypeLevel::parse(&parent_level).unwrap_or(TypeLevel::Standard)) => {
                    deferred.issue.parent_id = Some(parent_id);
                    import_issue(tx, &deferred.issue)?;
                }
                Some(_) => self.note(format!(
                    "{}: {} cannot be its parent here (epics hold stories, tasks and bugs; those hold subtasks)",
                    deferred.key, deferred.parent_key
                )),
                None => self.note(format!("{}: its parent {} was not imported", deferred.key, deferred.parent_key)),
            }

            tx.execute(
                "UPDATE jira_issue SET remote_updated = ?1, fingerprint = ?2 WHERE issue_id = ?3",
                (&deferred.updated, fingerprint(tx, deferred.id)?, deferred.id),
            )?;
        }

        let mut elsewhere = 0;

        for (source, target, kind) in std::mem::take(&mut self.links) {
            let id_of = |key: &str| -> ApiResult<Option<Id>> { query_one(tx, "SELECT id FROM issue WHERE key = ?1", [key], |row| row.get(0)) };

            match (id_of(&source)?, id_of(&target)?) {
                (Some(source_id), Some(target_id)) if source_id != target_id => {
                    // "Relates" has no direction: one row whichever side it was read from.
                    let mirrored: Option<Id> = match kind {
                        LinkType::Relates => query_one(
                            tx,
                            "SELECT id FROM issue_link WHERE source_id = ?1 AND target_id = ?2 AND type = 'relates'",
                            (target_id, source_id),
                            |row| row.get(0),
                        )?,
                        _ => None,
                    };

                    if mirrored.is_none() {
                        // The link service records a history entry and two events per link,
                        // as for a link made by hand; an imported link is written quietly.
                        self.report.links += tx.execute(
                            "INSERT OR IGNORE INTO issue_link (source_id, target_id, type, created_at) VALUES (?1, ?2, ?3, ?4)",
                            (source_id, target_id, kind.as_str(), &tx.now),
                        )? as i64;
                    }
                }
                (Some(_), Some(_)) => {}
                _ => elsewhere += 1,
            }
        }

        if elsewhere > 0 {
            self.note(format!("{elsewhere} links point to issues that are not here (other projects, or issues that were not imported)"));
        }

        Ok(())
    }

    /// Ends the run: relations for whatever was imported (also when the run was cancelled or
    /// failed half-way), and, after a complete run, the mark from which the next one continues.
    fn close(&mut self, app: &App, pages: ApiResult<End>) -> ApiResult<End> {
        let complete = matches!(pages, Ok(End::Done));

        self.save(app, JiraJobStep::Relations)?;

        app.write(Actor::system(LABEL), |tx| {
            self.relations(tx)?;

            if complete {
                let cursor = self.newest.as_ref().map(|(_, raw)| raw.clone());

                tx.execute(
                    "UPDATE jira_project SET cursor = coalesce(?1, cursor), last_import_at = ?2 WHERE project_id = ?3",
                    (cursor, now_iso(), self.project_id),
                )?;
            }

            jobs::progress(tx, self.job_id, JiraJobStep::Finished, self.total, self.processed, &self.report)
        })?;

        pages
    }
}
