#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::{Extension, Router};
use chrono::{Duration, SecondsFormat, Utc};
use http_body_util::BodyExt;
use hub_app::auth::ModuleScope;
use hub_app::services::issues::{create_issue, update_issue};
use hub_app::services::projects::{create_project, set_member, NewProject};
use hub_app::services::search::search;
use hub_app::services::sprints::{create_sprint, SprintInput};
use hub_app::services::users::{create_token, create_user, NewUser};
use hub_app::{access, Actor, App, AuthUser, Config, Module};
use hub_core::{Id, IssuePatch, NewIssue, ProjectRole, Role};
use hub_mod_timeline::TimelineModule;
use serde_json::Value;
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn test_app() -> App {
    let directory = std::env::temp_dir().join(format!("team-hub-timeline-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&directory);

    App::new(Config::for_tests(directory), vec![Arc::new(TimelineModule) as Arc<dyn Module>]).expect("the test application starts")
}

/// The module's routes where the server mounts them, marked as the module's the way the
/// server marks them: that mark is what makes a project that switched the module off vanish.
pub fn router(app: &App) -> Router {
    let routes = TimelineModule.routes().expect("the module has routes").layer(Extension(ModuleScope("timeline")));

    Router::new().nest("/api/m/timeline", routes).with_state(app.clone())
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

    Client { router: router(app), bearer: Some(token), user: AuthUser { id: user.id, name: user.name, email, role, via_token: true, through: Vec::new() } }
}

pub fn project(app: &App, owner: &Client, key: &str, is_private: bool) -> Id {
    let new =
        NewProject { key: key.to_string(), name: format!("Project {key}"), description: String::new(), board_mode: None, is_private, lead_id: None };

    app.write(owner.user.actor(), |tx| create_project(tx, &owner.user, &new)).expect("a project").summary.id
}

pub fn member(app: &App, owner: &Client, key: &str, who: &Client, role: ProjectRole) {
    app.write(owner.user.actor(), |tx| set_member(tx, &access::project_access(tx, &owner.user, key)?, who.user.id, Some(role)))
        .expect("a membership");
}

/// A day relative to today, `YYYY-MM-DD`.
pub fn date(day: i64) -> String {
    (Utc::now() + Duration::days(day)).format("%Y-%m-%d").to_string()
}

/// Noon of a day relative to today, so that an hour either way stays within the day.
pub fn noon(day: i64) -> String {
    format!("{}T12:00:00.000Z", date(day))
}

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// What an issue is created with, beyond its summary.
#[derive(Default)]
pub struct Fields<'a> {
    pub kind: &'a str,
    pub parent: Option<&'a str>,
    pub sprint: Option<Id>,
    pub points: Option<f64>,
    pub due: Option<String>,
}

/// Creates an issue at the instant `at` and returns its key.
pub fn issue_at(app: &App, who: &Client, project_key: &str, summary: &str, at: String, fields: Fields) -> String {
    app.write_at(Actor::user(who.user.id), at, |tx| {
        let type_id: Id = tx.query_row(
            "SELECT t.id FROM issue_type t JOIN project p ON p.id = t.project_id WHERE p.key = ?1 AND t.name = ?2",
            (project_key, if fields.kind.is_empty() { "Task" } else { fields.kind }),
            |row| row.get(0),
        )?;
        let patch = IssuePatch {
            summary: Some(summary.to_string()),
            type_id: Some(type_id),
            parent_key: Some(fields.parent.map(String::from)),
            sprint_id: Some(fields.sprint),
            points: Some(fields.points),
            due_date: Some(fields.due.clone()),
            ..Default::default()
        };
        let id = create_issue(tx, &NewIssue { project_key: project_key.to_string(), fields: patch })?;

        Ok(tx.query_row("SELECT key FROM issue WHERE id = ?1", [id], |row| row.get(0))?)
    })
    .expect("an issue")
}

pub fn issue(app: &App, who: &Client, project_key: &str, summary: &str, fields: Fields) -> String {
    issue_at(app, who, project_key, summary, now(), fields)
}

pub fn epic(app: &App, who: &Client, project_key: &str, summary: &str) -> String {
    issue(app, who, project_key, summary, Fields { kind: "Epic", ..Default::default() })
}

/// Moves an issue to the status called `status`, at the instant `at`.
pub fn transition_at(app: &App, who: &Client, key: &str, status: &str, at: String) {
    app.write_at(Actor::user(who.user.id), at, |tx| {
        let (id, status_id): (Id, Id) = tx.query_row(
            "SELECT i.id, s.id FROM issue i JOIN status s ON s.project_id = i.project_id WHERE i.key = ?1 AND s.name = ?2",
            (key, status),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        update_issue(tx, id, &IssuePatch { status_id: Some(status_id), ..Default::default() }).map(|_| ())
    })
    .expect("a transition");
}

pub fn sprint(app: &App, owner: &Client, project_key: &str, name: &str, start: Option<String>, end: Option<String>) -> Id {
    app.write(owner.user.actor(), |tx| {
        let input = SprintInput { name: Some(name.to_string()), start_date: start, end_date: end, ..Default::default() };

        create_sprint(tx, &access::project_access(tx, &owner.user, project_key)?, &input)
    })
    .expect("a sprint")
    .id
}

/// The keys a query finds, in key order.
pub fn found(app: &App, who: &Client, query: &str) -> Vec<String> {
    let result = search(app, &who.user, query, 200, 0).expect("a search");
    assert!(result.error.is_none(), "query \"{query}\" failed: {:?}", result.error);

    let mut keys: Vec<String> = result.issues.into_iter().map(|issue| issue.key).collect();
    keys.sort();

    keys
}

/// `(kind, issue key)` of the events of a kind starting with `prefix`, oldest first.
pub fn events(app: &App, prefix: &str) -> Vec<(String, Option<String>)> {
    app.read(|connection| {
        hub_app::db::query_all(
            connection,
            "SELECT e.kind, i.key FROM event e LEFT JOIN issue i ON i.id = e.issue_id WHERE e.kind LIKE ?1 || '%' ORDER BY e.id",
            [prefix],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    })
    .expect("events")
}

impl Client {
    pub async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(format!("/api/m/timeline{path}"));

        if let Some(bearer) = &self.bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }

        let request = match body {
            Some(body) => request.header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .expect("a valid request");
        let response = self.router.clone().oneshot(request).await.expect("the router answers");
        let status = response.status();
        let bytes = response.into_body().collect().await.expect("a readable body").to_bytes();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));

        (status, value)
    }

    pub async fn get(&self, path: &str) -> Value {
        let (status, value) = self.send(Method::GET, path, None).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {value}");

        value
    }

    pub async fn put(&self, path: &str, body: Value) -> Value {
        let (status, value) = self.send(Method::PUT, path, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "PUT {path}: {value}");

        value
    }

    /// The status and the message of a request that is expected to be refused.
    pub async fn refused(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, String) {
        let (status, value) = self.send(method, path, body).await;
        assert!(!status.is_success(), "{path} was expected to be refused: {value}");

        (status, value["error"]["message"].as_str().unwrap_or_default().to_string())
    }
}
