#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use hub_app::{App, Config, Module};
use serde_json::{json, Value};
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn test_app(modules: Vec<Arc<dyn Module>>) -> App {
    let directory = std::env::temp_dir().join(format!("team-hub-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&directory);

    App::new(Config::for_tests(directory), modules).expect("the test application starts")
}

/// A browser, more or less: keeps the session cookie and sends the CSRF token.
#[derive(Clone)]
pub struct Client {
    pub router: Router,
    pub cookie: Option<String>,
    pub csrf: Option<String>,
    pub bearer: Option<String>,
}

impl Client {
    pub fn new(router: Router) -> Self {
        Self { router, cookie: None, csrf: None, bearer: None }
    }

    pub async fn send(&mut self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);

        if let Some(cookie) = &self.cookie {
            request = request.header(header::COOKIE, cookie);
        }

        if let Some(csrf) = &self.csrf {
            request = request.header("x-csrf-token", csrf);
        }

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

        if let Some(cookie) = response.headers().get(header::SET_COOKIE).and_then(|value| value.to_str().ok()) {
            self.cookie = cookie.split(';').next().map(String::from);
        }

        let bytes = response.into_body().collect().await.expect("a readable body").to_bytes();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));

        if let Some(csrf) = value.get("csrf").and_then(Value::as_str) {
            self.csrf = Some(csrf.to_string());
        }

        (status, value)
    }

    /// A request with a raw body and extra headers, as another system would send it.
    pub async fn send_raw(&mut self, path: &str, headers: &[(&str, String)], body: Vec<u8>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(Method::POST).uri(path).header(header::CONTENT_TYPE, "application/json");

        for (name, value) in headers {
            request = request.header(*name, value);
        }

        let response = self.router.clone().oneshot(request.body(Body::from(body)).expect("a valid request")).await.expect("the router answers");
        let status = response.status();
        let bytes = response.into_body().collect().await.expect("a readable body").to_bytes();

        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    pub async fn get(&mut self, path: &str) -> Value {
        let (status, value) = self.send(Method::GET, path, None).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {value}");

        value
    }

    pub async fn post(&mut self, path: &str, body: Value) -> Value {
        let (status, value) = self.send(Method::POST, path, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "POST {path}: {value}");

        value
    }

    pub async fn patch(&mut self, path: &str, body: Value) -> Value {
        let (status, value) = self.send(Method::PATCH, path, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "PATCH {path}: {value}");

        value
    }

    pub async fn put(&mut self, path: &str, body: Value) -> Value {
        let (status, value) = self.send(Method::PUT, path, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "PUT {path}: {value}");

        value
    }

    pub async fn delete(&mut self, path: &str) -> Value {
        let (status, value) = self.send(Method::DELETE, path, None).await;
        assert_eq!(status, StatusCode::OK, "DELETE {path}: {value}");

        value
    }

    /// The status of a request expected to be refused.
    pub async fn refused(&mut self, method: Method, path: &str, body: Option<Value>) -> StatusCode {
        self.send(method, path, body).await.0
    }

    pub async fn setup(&mut self, demo: bool) {
        self.post("/api/setup", json!({ "name": "Bartek Admin", "email": "admin@example.com", "password": "correct horse", "demo": demo })).await;
    }

    /// Creates a member as the signed-in administrator and returns a client signed in as them.
    pub async fn member(&mut self, name: &str, email: &str) -> (Client, i64) {
        let user = self.post("/api/users", json!({ "name": name, "email": email, "password": "long enough" })).await;
        let mut client = Client::new(self.router.clone());
        client.post("/api/login", json!({ "email": email, "password": "long enough" })).await;

        (client, user["id"].as_i64().expect("a user id"))
    }

    pub async fn search_keys(&mut self, query: &str) -> Vec<String> {
        let encoded: String = query
            .bytes()
            .map(|byte| if byte.is_ascii_alphanumeric() { (byte as char).to_string() } else { format!("%{byte:02X}") })
            .collect();
        let result = self.get(&format!("/api/search?q={encoded}&limit=200")).await;
        assert!(result["error"].is_null(), "query \"{query}\" failed: {}", result["error"]);

        result["issues"].as_array().expect("issues").iter().map(|issue| issue["key"].as_str().unwrap_or("").to_string()).collect()
    }
}

pub fn keys(value: &Value) -> Vec<String> {
    value.as_array().expect("an array").iter().map(|item| item["key"].as_str().unwrap_or("").to_string()).collect()
}
