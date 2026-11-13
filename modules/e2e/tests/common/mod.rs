#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use hub_app::services::issues::{create_issue, update_issue};
use hub_app::services::projects::{create_project, set_member, NewProject};
use hub_app::services::search::search;
use hub_app::services::users::{create_token, create_user, NewUser};
use hub_app::{access, Actor, App, AuthUser, Config, Module};
use hub_core::{Id, IssuePatch, NewIssue, ProjectRole, Role};
use hub_mod_e2e::E2eModule;
use serde_json::Value;
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn test_app() -> App {
    let directory = std::env::temp_dir().join(format!("team-hub-e2e-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&directory);

    App::new(Config::for_tests(directory), vec![Arc::new(E2eModule) as Arc<dyn Module>]).expect("the test application starts")
}

pub fn router(app: &App) -> Router {
    Router::new().nest("/api/m/e2e", E2eModule.routes().expect("the module has routes")).with_state(app.clone())
}

/// Somebody (or a CI job) signed in with an API token.
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
    let token = app.db.write(|tx| create_token(tx, user.id, "ci")).expect("a token");

    Client { router: router(app), bearer: Some(token), user: AuthUser { id: user.id, name: user.name, email, role, via_token: true } }
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

/// Creates an issue assigned to `who` and returns its key.
pub fn issue(app: &App, who: &Client, project_key: &str, summary: &str) -> String {
    let fields = IssuePatch { summary: Some(summary.to_string()), assignee_id: Some(Some(who.user.id)), ..Default::default() };
    let new = NewIssue { project_key: project_key.to_string(), fields };

    app.write(Actor::user(who.user.id), |tx| {
        let id = create_issue(tx, &new)?;

        Ok(tx.query_row("SELECT key FROM issue WHERE id = ?1", [id], |row| row.get(0))?)
    })
    .expect("an issue")
}

/// Moves an issue to the status called `status`.
pub fn transition(app: &App, who: &Client, key: &str, status: &str) {
    app.write(Actor::user(who.user.id), |tx| {
        let (id, status_id): (Id, Id) = tx.query_row(
            "SELECT i.id, s.id FROM issue i JOIN status s ON s.project_id = i.project_id WHERE i.key = ?1 AND s.name = ?2",
            (key, status),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        update_issue(tx, id, &IssuePatch { status_id: Some(status_id), ..Default::default() }).map(|_| ())
    })
    .expect("a transition");
}

/// The keys a query finds, in key order.
pub fn found(app: &App, who: &Client, query: &str) -> Vec<String> {
    let result = search(app, &who.user, query, 200, 0).expect("a search");
    assert!(result.error.is_none(), "query \"{query}\" failed: {:?}", result.error);

    let mut keys: Vec<String> = result.issues.into_iter().map(|issue| issue.key).collect();
    keys.sort();

    keys
}

/// `(kind, issue key)` of the module's events, oldest first.
pub fn events(app: &App) -> Vec<(String, Option<String>)> {
    app.read(|connection| {
        hub_app::db::query_all(
            connection,
            "SELECT e.kind, i.key FROM event e LEFT JOIN issue i ON i.id = e.issue_id WHERE e.kind LIKE 'e2e.%' ORDER BY e.id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    })
    .expect("events")
}

impl Client {
    pub async fn send_raw(&self, method: Method, path: &str, content_type: Option<&str>, body: Vec<u8>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(format!("/api/m/e2e{path}"));

        if let Some(bearer) = &self.bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }

        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }

        let response = self.router.clone().oneshot(request.body(Body::from(body)).expect("a valid request")).await.expect("the router answers");
        let status = response.status();
        let bytes = response.into_body().collect().await.expect("a readable body").to_bytes();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));

        (status, value)
    }

    pub async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        match body {
            Some(body) => self.send_raw(method, path, Some("application/json"), body.to_string().into_bytes()).await,
            None => self.send_raw(method, path, None, Vec::new()).await,
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

    pub async fn put(&self, path: &str) -> Value {
        self.expect_ok(Method::PUT, path, None).await
    }

    pub async fn delete(&self, path: &str) -> Value {
        self.expect_ok(Method::DELETE, path, None).await
    }

    /// Posts JUnit XML the way `curl --data-binary @junit.xml -H 'content-type: application/xml'` does.
    pub async fn post_xml(&self, path: &str, xml: &str) -> Value {
        let (status, value) = self.send_raw(Method::POST, path, Some("application/xml"), xml.as_bytes().to_vec()).await;
        assert_eq!(status, StatusCode::OK, "POST {path}: {value}");

        value
    }

    /// The status and message of a request expected to be refused.
    pub async fn refused(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, String) {
        let (status, value) = self.send(method, path, body).await;

        (status, value["error"]["message"].as_str().unwrap_or("").to_string())
    }
}
