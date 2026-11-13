#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use hub_app::services::projects::{create_project, set_member, NewProject};
use hub_app::services::users::{create_token, create_user, NewUser};
use hub_app::{access, Actor, App, AuthUser, Config, Module};
use hub_core::{Id, IssuePatch, NewIssue, ProjectRole, Role};
use hub_mod_wiki::WikiModule;
use serde_json::Value;
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn test_app() -> App {
    let directory = std::env::temp_dir().join(format!("team-hub-wiki-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&directory);

    App::new(Config::for_tests(directory), vec![Arc::new(WikiModule) as Arc<dyn Module>]).expect("the test application starts")
}

pub fn router(app: &App) -> Router {
    Router::new().nest("/api/m/wiki", WikiModule.routes().expect("the wiki has routes")).with_state(app.clone())
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

pub fn project(app: &App, owner: &Client, key: &str, is_private: bool) -> Id {
    let new =
        NewProject { key: key.to_string(), name: format!("Project {key}"), description: String::new(), board_mode: None, is_private, lead_id: None };

    app.write(owner.user.actor(), |tx| create_project(tx, &owner.user, &new)).expect("a project").summary.id
}

pub fn member(app: &App, owner: &Client, key: &str, who: &Client, role: ProjectRole) {
    app.write(owner.user.actor(), |tx| set_member(tx, &access::project_access(tx, &owner.user, key)?, who.user.id, Some(role)))
        .expect("a membership");
}

/// Creates an issue and returns its key.
pub fn issue(app: &App, who: &Client, project_key: &str, summary: &str) -> String {
    let new = NewIssue { project_key: project_key.to_string(), fields: IssuePatch { summary: Some(summary.to_string()), ..Default::default() } };

    app.write(Actor::user(who.user.id), |tx| {
        let id = hub_app::services::issues::create_issue(tx, &new)?;

        Ok(tx.query_row("SELECT key FROM issue WHERE id = ?1", [id], |row| row.get(0))?)
    })
    .expect("an issue")
}

impl Client {
    pub async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(format!("/api/m/wiki{path}"));

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

    pub async fn patch(&self, path: &str, body: Value) -> Value {
        self.expect_ok(Method::PATCH, path, Some(body)).await
    }

    pub async fn put(&self, path: &str) -> Value {
        self.expect_ok(Method::PUT, path, None).await
    }

    pub async fn delete(&self, path: &str) -> Value {
        self.expect_ok(Method::DELETE, path, None).await
    }

    /// The status and message of a request expected to be refused.
    pub async fn refused(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, String) {
        let (status, value) = self.send(method, path, body).await;

        (status, value["error"]["message"].as_str().unwrap_or("").to_string())
    }
}

pub fn titles(value: &Value) -> Vec<String> {
    value.as_array().expect("an array").iter().map(|item| item["title"].as_str().unwrap_or("").to_string()).collect()
}

pub fn id_of(value: &Value) -> Id {
    value["id"].as_i64().expect("an id")
}
