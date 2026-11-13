#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use http_body_util::BodyExt;
use hub_app::services::users::{create_token, create_user, NewUser};
use hub_app::{App, AuthUser, Config, Module};
use hub_core::Role;
use hub_mod_confluence::ConfluenceModule;
use hub_mod_wiki::WikiModule;
use serde_json::{json, Value};
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn test_app() -> App {
    let directory = std::env::temp_dir().join(format!("team-hub-confluence-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&directory);
    let modules: Vec<Arc<dyn Module>> = vec![Arc::new(WikiModule), Arc::new(ConfluenceModule)];

    App::new(Config::for_tests(directory), modules).expect("the test application starts")
}

pub fn router(app: &App) -> Router {
    Router::new()
        .nest("/api/m/wiki", WikiModule.routes().expect("the wiki has routes"))
        .nest("/api/m/confluence", ConfluenceModule.routes().expect("the import has routes"))
        .with_state(app.clone())
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
    pub async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(format!("/api/m{path}"));

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

    pub async fn put(&self, path: &str, body: Value) -> Value {
        self.expect_ok(Method::PUT, path, Some(body)).await
    }

    pub async fn patch(&self, path: &str, body: Value) -> Value {
        self.expect_ok(Method::PATCH, path, Some(body)).await
    }

    /// The status and message of a request expected to be refused.
    pub async fn refused(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, String) {
        let (status, value) = self.send(method, path, body).await;

        (status, value["error"]["message"].as_str().unwrap_or("").to_string())
    }
}

#[derive(Clone)]
pub struct FakePage {
    pub id: String,
    pub title: String,
    pub version: i64,
    pub storage: String,
    pub parent: Option<String>,
}

#[derive(Default)]
pub struct Upstream {
    pub pages: Vec<FakePage>,
    pub spaces: Vec<(String, String)>,
    /// The only `Authorization` header the server accepts; anything else gets a 401.
    pub accepts: String,
    /// The most results the server returns at once, whatever the client asks for.
    pub page_size: usize,
    /// Whether answers carry `_links.next`, as Confluence Cloud's do.
    pub next_links: bool,
    /// Ids of pages served without their body.
    pub without_body: Vec<String>,
    /// Statuses to answer the coming requests with, before answering properly again.
    pub failures: Vec<u16>,
    /// Every request as (path, start, limit, Authorization).
    pub requests: Vec<(String, usize, usize, String)>,
}

/// A stand-in for Confluence on a port of this machine, serving `/wiki/rest/api`.
#[derive(Clone)]
pub struct Fake {
    pub state: Arc<Mutex<Upstream>>,
    pub base_url: String,
}

impl Fake {
    pub async fn start(accepts: &str) -> Self {
        let state = Arc::new(Mutex::new(Upstream { accepts: accepts.to_string(), page_size: 2, next_links: true, ..Default::default() }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let base_url = format!("http://{}/wiki", listener.local_addr().expect("an address"));
        let routes = Router::new().route("/wiki/rest/api/space", get(spaces)).route("/wiki/rest/api/content", get(content)).with_state(state.clone());

        tokio::spawn(async move {
            axum::serve(listener, routes).await.expect("the fake Confluence serves");
        });

        Self { state, base_url }
    }

    pub fn with<T>(&self, change: impl FnOnce(&mut Upstream) -> T) -> T {
        change(&mut self.state.lock().expect("the fake's state"))
    }

    pub fn add(&self, id: &str, title: &str, parent: Option<&str>, storage: &str) {
        self.with(|upstream| {
            upstream.pages.push(FakePage {
                id: id.to_string(),
                title: title.to_string(),
                version: 1,
                storage: storage.to_string(),
                parent: parent.map(String::from),
            })
        });
    }

    /// A new version of a page, as an edit in Confluence makes one.
    pub fn edit(&self, id: &str, storage: &str) {
        self.with(|upstream| {
            let page = upstream.pages.iter_mut().find(|page| page.id == id).expect("the page");
            page.version += 1;
            page.storage = storage.to_string();
        });
    }

    /// The requests made since the last call, as (path, start, limit, Authorization).
    pub fn take_requests(&self) -> Vec<(String, usize, usize, String)> {
        self.with(|upstream| std::mem::take(&mut upstream.requests))
    }
}

fn number(query: &HashMap<String, String>, name: &str, default: usize) -> usize {
    query.get(name).and_then(|value| value.parse().ok()).unwrap_or(default)
}

/// Records the request; returns the refusal when it is not to be answered properly.
fn admit(upstream: &mut Upstream, path: &str, query: &HashMap<String, String>, headers: &HeaderMap) -> Option<Response> {
    let authorization = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).unwrap_or("").to_string();
    upstream.requests.push((path.to_string(), number(query, "start", 0), number(query, "limit", 25), authorization.clone()));

    if !upstream.failures.is_empty() {
        let status = StatusCode::from_u16(upstream.failures.remove(0)).expect("a status");

        return Some((status, "try later").into_response());
    }

    if authorization != upstream.accepts {
        return Some((StatusCode::UNAUTHORIZED, Json(json!({ "message": "Basic authentication with passwords is deprecated" }))).into_response());
    }

    None
}

fn paged(upstream: &Upstream, path: &str, query: &HashMap<String, String>, all: Vec<Value>) -> Response {
    let start = number(query, "start", 0);
    let limit = number(query, "limit", 25).min(upstream.page_size);
    let results: Vec<Value> = all.iter().skip(start).take(limit).cloned().collect();
    let mut links = json!({ "context": "/wiki", "self": format!("{path}?start={start}") });

    if upstream.next_links && start + results.len() < all.len() {
        let mut next: Vec<String> =
            query.iter().filter(|(name, _)| *name != "start" && *name != "limit").map(|(name, value)| format!("{name}={value}")).collect();
        next.sort();
        next.push(format!("limit={limit}"));
        next.push(format!("start={}", start + results.len()));
        links["next"] = json!(format!("{path}?{}", next.join("&")));
    }

    Json(json!({ "results": results, "start": start, "limit": limit, "size": results.len(), "_links": links })).into_response()
}

async fn spaces(State(state): State<Arc<Mutex<Upstream>>>, Query(query): Query<HashMap<String, String>>, headers: HeaderMap) -> Response {
    let mut upstream = state.lock().expect("the fake's state");

    if let Some(refusal) = admit(&mut upstream, "/rest/api/space", &query, &headers) {
        return refusal;
    }

    let all: Vec<Value> = upstream
        .spaces
        .iter()
        .filter(|(key, _)| query.get("spaceKey").is_none_or(|wanted| wanted == key))
        .map(|(key, name)| json!({ "id": 1, "key": key, "name": name, "type": "global" }))
        .collect();

    paged(&upstream, "/rest/api/space", &query, all)
}

async fn content(State(state): State<Arc<Mutex<Upstream>>>, Query(query): Query<HashMap<String, String>>, headers: HeaderMap) -> Response {
    let mut upstream = state.lock().expect("the fake's state");

    if let Some(refusal) = admit(&mut upstream, "/rest/api/content", &query, &headers) {
        return refusal;
    }

    if query.get("type").map(String::as_str) != Some("page") || query.get("expand").map(String::as_str) != Some("body.storage,version,ancestors") {
        return (StatusCode::BAD_REQUEST, "the import must ask for pages with body, version and ancestors").into_response();
    }

    if !upstream.spaces.iter().any(|(key, _)| Some(key) == query.get("spaceKey")) {
        return (StatusCode::NOT_FOUND, Json(json!({ "message": "No space with that key" }))).into_response();
    }

    let by_id: HashMap<&str, &FakePage> = upstream.pages.iter().map(|page| (page.id.as_str(), page)).collect();
    let all: Vec<Value> = upstream
        .pages
        .iter()
        .map(|page| {
            let mut ancestors = Vec::new();
            let mut cursor = page.parent.as_deref();

            while let Some(parent) = cursor.and_then(|id| by_id.get(id)) {
                ancestors.insert(0, json!({ "id": parent.id, "type": "page", "title": parent.title }));
                cursor = parent.parent.as_deref();
            }

            let mut listed = json!({
                "id": page.id,
                "type": "page",
                "status": "current",
                "title": page.title,
                "version": { "number": page.version, "when": "2026-10-01T10:00:00.000Z" },
                "ancestors": ancestors,
                "_links": { "webui": format!("/spaces/DOC/pages/{}", page.id) },
            });

            if !upstream.without_body.contains(&page.id) {
                listed["body"] = json!({ "storage": { "value": page.storage, "representation": "storage" } });
            }

            listed
        })
        .collect();

    paged(&upstream, "/rest/api/content", &query, all)
}
