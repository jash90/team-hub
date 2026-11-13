#![allow(dead_code)]

pub mod fake;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use hub_app::services::users::{create_token, create_user, NewUser};
use hub_app::{App, AuthUser, Config, Module};
use hub_core::Role;
use hub_mod_jira::JiraModule;
use serde_json::Value;
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn test_app() -> App {
    let directory = std::env::temp_dir().join(format!("team-hub-jira-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&directory);
    let modules: Vec<Arc<dyn Module>> = vec![Arc::new(JiraModule)];

    App::new(Config::for_tests(directory), modules).expect("the test application starts")
}

pub fn router(app: &App) -> Router {
    Router::new().nest("/api/m/jira", JiraModule.routes().expect("the import has routes")).with_state(app.clone())
}

/// Somebody signed in with an API token.
#[derive(Clone)]
pub struct Client {
    pub router: Router,
    pub bearer: Option<String>,
    pub user: AuthUser,
}

pub fn person(app: &App, name: &str, role: Role) -> Client {
    let email = format!("{}@example.com", name.to_lowercase().replace(' ', "."));
    let new = NewUser { name: name.to_string(), email: email.clone(), password: None, role: Some(role), github_login: None };
    let user = app.db.write(|tx| create_user(tx, &new)).expect("a user");
    let token = app.db.write(|tx| create_token(tx, user.id, "tests")).expect("a token");

    Client { router: router(app), bearer: Some(token), user: AuthUser { id: user.id, name: user.name, email, role, via_token: true } }
}

impl Client {
    async fn request(&self, method: Method, path: &str, body: Body, headers: &[(&str, &str)]) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(format!("/api/m{path}"));

        if let Some(bearer) = &self.bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }

        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        let response = self.router.clone().oneshot(request.body(body).expect("a valid request")).await.expect("the router answers");
        let status = response.status();
        let bytes = response.into_body().collect().await.expect("a readable body").to_bytes();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));

        (status, value)
    }

    pub async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        match body {
            Some(body) => self.request(method, path, Body::from(body.to_string()), &[("content-type", "application/json")]).await,
            None => self.request(method, path, Body::empty(), &[]).await,
        }
    }

    async fn expect_ok(&self, method: Method, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.send(method.clone(), path, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");

        value
    }

    pub async fn get(&self, path: &str) -> Value {
        self.expect_ok(Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Value {
        self.expect_ok(Method::POST, path, Some(body)).await
    }

    pub async fn put(&self, path: &str, body: Value) -> Value {
        self.expect_ok(Method::PUT, path, Some(body)).await
    }

    /// Sends a file the way the web interface does: as the body, with its name in a header.
    pub async fn upload(&self, path: &str, filename: &str, content: &str) -> (StatusCode, Value) {
        self.request(Method::POST, path, Body::from(content.to_string()), &[("content-type", "text/csv"), ("x-filename", filename)]).await
    }

    /// The status and message of a request expected to be refused.
    pub async fn refused(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, String) {
        let (status, value) = self.send(method, path, body).await;

        (status, value["error"]["message"].as_str().unwrap_or("").to_string())
    }

    /// Waits for an import job to end and returns it.
    pub async fn finished(&self, job: &Value) -> Value {
        let id = job["id"].as_i64().expect("a job id");

        for _ in 0..3000 {
            let job = self.get(&format!("/jira/jobs/{id}")).await;

            if !matches!(job["state"].as_str(), Some("queued" | "running")) {
                return job;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("the import job {id} did not finish");
    }

    /// Starts an import and waits for it; the job must end as `done`.
    pub async fn import(&self, request: Value) -> Value {
        let job = self.finished(&self.post("/jira/import", request).await).await;
        assert_eq!(job["state"], "done", "{job}");

        job
    }
}

pub const EMAIL: &str = "anna@example.com";
pub const TOKEN: &str = "api-token-1234";
pub const BASIC: &str = "Basic YW5uYUBleGFtcGxlLmNvbTphcGktdG9rZW4tMTIzNA==";
pub const BEARER: &str = "Bearer api-token-1234";

/// An application with an administrator and a member, and a fake Jira it is configured for.
pub struct Setup {
    pub app: App,
    pub admin: Client,
    pub member: Client,
    pub fake: fake::Fake,
    pub cloud: bool,
}

pub async fn connected(upstream: fake::Upstream) -> Setup {
    let cloud = upstream.cloud;
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let member = person(&app, "Marek Member", Role::Member);
    let fake = fake::Fake::start(upstream).await;
    let settings = if cloud {
        serde_json::json!({ "baseUrl": fake.base_url, "email": EMAIL, "token": TOKEN })
    } else {
        serde_json::json!({ "baseUrl": fake.base_url, "token": TOKEN })
    };

    admin.put("/jira/settings", settings).await;

    Setup { app, admin, member, fake, cloud }
}

/// Jira's way of writing an instant, here in a zone one hour ahead of UTC.
pub fn local(day_and_time: &str) -> String {
    format!("2024-{day_and_time}:00.000+0100")
}

fn change(field: &str, from: Option<&str>, from_text: Option<&str>, to: Option<&str>, to_text: Option<&str>) -> fake::FakeChange {
    (field.to_string(), from.map(String::from), from_text.map(String::from), to.map(String::from), to_text.map(String::from))
}

fn status_change(from: &str, to: &str) -> fake::FakeChange {
    // The ids are the fake's: 10000 plus the position in the project's list of statuses.
    let id = |name: &str| (10000 + ["Done", "In Progress", "Open", "In Review"].iter().position(|known| *known == name).unwrap()).to_string();

    change("status", Some(&id(from)), Some(from), Some(&id(to)), Some(to))
}

fn entry(id: i64, author: &str, created: &str, items: Vec<fake::FakeChange>) -> fake::FakeHistory {
    fake::FakeHistory { id, author: author.to_string(), created: local(created), items }
}

/// A small project with one of everything, the same in both flavours:
///
/// ```text
/// WEB-1 Epic   Checkout                      In Progress
/// WEB-2 Story  Cart            (in WEB-1)    Done in Sprint 1, with the full history
/// WEB-3 Sub-task Cart API      (in WEB-2)    In Review, Sprint 2
/// WEB-5 Bug    Rounding        (in WEB-1)    In Progress, carried from Sprint 1 to Sprint 2
/// WEB-7 Story  Refunds                       Open, backlog
/// WEB-8 Story  Wishlist        (in WEB-9)    Done after Sprint 1 ended without it
/// WEB-9 Epic   Account                       Open
/// ```
pub fn small(cloud: bool) -> fake::Upstream {
    use fake::{FakeBoard, FakeComment, FakeIssue, FakeProject, FakeSprint, FakeUser, FakeVersion, FakeWorklog, Text};

    let mut upstream = fake::Upstream::new(cloud, if cloud { BASIC } else { BEARER });
    let account = |name: &str| if cloud { format!("acc-{name}") } else { name.to_string() };
    let user = |login: &str, name: &str, email: Option<&str>| FakeUser { account: account(login), name: name.to_string(), email: email.map(String::from) };
    let points = if cloud { "Story point estimate" } else { "Story Points" };

    upstream.page_size = 3;
    upstream.users = vec![
        user("ada", "Ada Admin", Some("ada.admin@example.com")),
        user("marek", "Marek Member", Some("MAREK.MEMBER@example.com")),
        user("zofia", "Zofia Zewnętrzna", None),
        user("olaf", "Olaf Odszedł", Some("olaf@old.example.com")),
    ];
    upstream.projects.push(FakeProject {
        key: "WEB".to_string(),
        name: "Web shop".to_string(),
        description: "The shop.".to_string(),
        lead: Some(account("ada")),
        types: vec![("Epic".to_string(), false, true), ("Story".to_string(), false, false), ("Bug".to_string(), false, false), ("Sub-task".to_string(), true, false)],
        statuses: vec![
            ("Done".to_string(), "done".to_string()),
            ("In Progress".to_string(), "indeterminate".to_string()),
            ("Open".to_string(), "new".to_string()),
            ("In Review".to_string(), "indeterminate".to_string()),
        ],
        components: vec!["Cart".to_string(), "Payments".to_string()],
        versions: vec![
            FakeVersion { name: "1.0".to_string(), released: true, archived: false, start: Some("2024-03-01".to_string()), release: Some("2024-03-15".to_string()) },
            FakeVersion { name: "1.1".to_string(), released: false, archived: false, start: None, release: Some("2024-04-30".to_string()) },
        ],
        boards: vec![FakeBoard {
            id: 7,
            name: "WEB board".to_string(),
            scrum: true,
            columns: vec![vec!["Open".to_string()], vec!["In Progress".to_string()], vec!["In Review".to_string()], vec!["Done".to_string()]],
        }],
        sprints: vec![
            FakeSprint {
                id: 41,
                board: 7,
                name: "Sprint 1".to_string(),
                state: "closed".to_string(),
                start: Some(local("03-01T09:00")),
                end: Some(local("03-15T09:00")),
                complete: Some(local("03-15T16:00")),
                goal: "A cart that works".to_string(),
            },
            FakeSprint {
                id: 42,
                board: 7,
                name: "Sprint 2".to_string(),
                state: "active".to_string(),
                start: Some(local("03-18T09:00")),
                end: Some(local("04-01T09:00")),
                complete: None,
                goal: String::new(),
            },
            FakeSprint { id: 43, board: 7, name: "Sprint 3".to_string(), state: "future".to_string(), ..Default::default() },
        ],
    });

    let rich = Text::new(
        serde_json::json!({ "type": "doc", "version": 1, "content": [
            { "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "Goal" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "A " }, { "type": "text", "text": "better", "marks": [{ "type": "strong" }] }, { "type": "text", "text": " cart" }] },
            { "type": "bulletList", "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "fast" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "simple" }] }] },
            ] },
        ] }),
        "h2. Goal\n\nA *better* cart\n\n* fast\n* simple",
    );
    let comment = |id: i64, author: &str, text: &str, created: &str| FakeComment {
        id,
        author: account(author),
        body: Text::plain(text),
        created: local(created),
        updated: local(created),
    };
    let issue = |key: &str, kind: &str, status: &str, summary: &str, created: &str, updated: &str| FakeIssue {
        key: key.to_string(),
        kind: kind.to_string(),
        status: status.to_string(),
        summary: summary.to_string(),
        priority: Some("Medium".to_string()),
        reporter: Some(account("ada")),
        created: local(created),
        updated: local(updated),
        ..Default::default()
    };

    upstream.issues = vec![
        FakeIssue {
            description: Some(Text::plain("Everything between the cart and the confirmation.")),
            history: vec![entry(101, &account("ada"), "03-04T10:00", vec![status_change("Open", "In Progress")])],
            ..issue("WEB-1", "Epic", "In Progress", "Checkout", "02-20T09:00", "03-04T10:00")
        },
        FakeIssue {
            description: Some(rich),
            assignee: Some(account("marek")),
            labels: vec!["checkout".to_string(), "mvp".to_string()],
            components: vec!["Cart".to_string()],
            versions: vec!["1.0".to_string()],
            sprints: vec![41],
            points: Some(5.0),
            due: Some("2024-03-10".to_string()),
            estimate: Some(28_800),
            parent: Some("WEB-1".to_string()),
            resolved: Some(local("03-08T12:00")),
            comments: vec![comment(901, "zofia", "Looks good on staging.", "03-05T15:30"), comment(902, "ada", "Merged.", "03-07T08:00")],
            worklogs: vec![FakeWorklog {
                id: 801,
                author: account("marek"),
                seconds: 5400,
                started: local("03-05T09:00"),
                comment: Some(Text::plain("Cart endpoints")),
            }],
            attachments: vec![("spec.pdf".to_string(), 204_800)],
            history: vec![
                entry(201, &account("ada"), "03-01T09:30", vec![change("Sprint", None, None, Some("41"), Some("Sprint 1"))]),
                entry(202, &account("ada"), "03-02T09:00", vec![change(points, None, Some("3"), None, Some("5"))]),
                entry(203, &account("ada"), "03-03T09:00", vec![change("assignee", None, None, Some(&account("marek")), Some("Marek Member"))]),
                entry(204, &account("marek"), "03-04T10:00", vec![status_change("Open", "In Progress")]),
                entry(205, &account("marek"), "03-06T10:00", vec![status_change("In Progress", "In Review")]),
                entry(206, &account("ada"), "03-08T12:00", vec![status_change("In Review", "Done"), change("resolution", None, None, Some("1"), Some("Done"))]),
            ],
            ..issue("WEB-2", "Story", "Done", "Cart", "02-21T09:00", "03-08T12:00")
        },
        FakeIssue {
            assignee: Some(account("zofia")),
            sprints: vec![42],
            parent: Some("WEB-2".to_string()),
            history: vec![
                entry(301, &account("zofia"), "03-19T10:00", vec![status_change("Open", "In Progress")]),
                entry(302, &account("zofia"), "03-20T10:00", vec![status_change("In Progress", "In Review")]),
            ],
            ..issue("WEB-3", "Sub-task", "In Review", "Cart API", "03-18T10:00", "03-20T10:00")
        },
        FakeIssue {
            priority: Some("Highest".to_string()),
            assignee: Some(account("olaf")),
            sprints: vec![41, 42],
            parent: Some("WEB-1".to_string()),
            comments: vec![comment(903, "olaf", "Rounds half down instead of half up.", "03-11T11:00")],
            links: vec![("Blocks".to_string(), true, "WEB-7".to_string())],
            history: vec![
                entry(501, &account("ada"), "03-01T09:30", vec![change("Sprint", None, None, Some("41"), Some("Sprint 1"))]),
                entry(502, &account("olaf"), "03-10T10:00", vec![status_change("Open", "In Progress")]),
                entry(503, &account("ada"), "03-18T09:30", vec![change("Sprint", Some("41"), Some("Sprint 1"), Some("41, 42"), Some("Sprint 1, Sprint 2"))]),
            ],
            ..issue("WEB-5", "Bug", "In Progress", "Rounding", "02-25T09:00", "03-18T09:30")
        },
        FakeIssue {
            priority: Some("Low".to_string()),
            reporter: Some(account("zofia")),
            links: vec![
                ("Blocks".to_string(), false, "WEB-5".to_string()),
                ("Cloners".to_string(), true, "WEB-2".to_string()),
                ("Relates".to_string(), true, "OPS-1".to_string()),
            ],
            ..issue("WEB-7", "Story", "Open", "Refunds", "03-02T09:00", "03-02T09:00")
        },
        FakeIssue {
            sprints: vec![41],
            parent: Some("WEB-9".to_string()),
            resolved: Some(local("03-20T12:00")),
            history: vec![
                entry(801, &account("ada"), "03-01T09:30", vec![change("Sprint", None, None, Some("41"), Some("Sprint 1"))]),
                entry(802, &account("marek"), "03-12T10:00", vec![status_change("Open", "In Progress")]),
                entry(803, &account("marek"), "03-20T12:00", vec![status_change("In Progress", "Done")]),
            ],
            ..issue("WEB-8", "Story", "Done", "Wishlist", "02-26T09:00", "03-20T12:00")
        },
        issue("WEB-9", "Epic", "Open", "Account", "03-03T09:00", "03-03T09:00"),
    ];

    upstream
}
