//! Serves the window's commands over HTTP on this machine, so the interface can be looked at
//! in a browser: `ui/src/bridge.ts` talks to this when it is not running inside Tauri. The
//! commands go through `commands::invoke`, the very function the Tauri crate forwards to.
//!
//! `POST /invoke` `{ command, args }` → `{ ok, value }` or `{ ok: false, error }`
//! `GET /events?after=N` → `{ last, events }`, waiting up to 20 s for something new;
//! without `after`, only where the journal is now
//!
//! For development only: it listens on 127.0.0.1 and has no authentication of its own.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::header::{ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use hub_desktop_core::config::Paths;
use hub_desktop_core::model::AppEvent;
use hub_desktop_core::secrets::{Keychain, MemoryKeychain, OsKeychain};
use hub_desktop_core::{commands, Desktop, OpenOptions};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Notify;

const KEPT_EVENTS: usize = 500;

#[derive(Parser)]
#[command(name = "hub-dev-server", about = "The desktop core over HTTP, for looking at the interface in a browser")]
struct Arguments {
    #[arg(long, default_value_t = 4455)]
    port: u16,
    /// Keep the token in memory only (for a machine without a keychain).
    #[arg(long)]
    memory_keychain: bool,
}

#[derive(Default)]
struct Journal {
    last: u64,
    events: VecDeque<(u64, AppEvent)>,
}

#[derive(Clone)]
struct Served {
    desktop: Desktop,
    journal: Arc<Mutex<Journal>>,
    arrived: Arc<Notify>,
}

#[derive(Deserialize)]
struct Invoke {
    command: String,
    #[serde(default)]
    args: Value,
}

#[derive(Deserialize)]
struct After {
    /// The number of the last event the page has; none on its first request.
    after: Option<u64>,
}

/// Only pages served from this machine may call.
fn cors(headers: &HeaderMap, mut response: Response) -> Response {
    let origin = headers.get("origin").and_then(|origin| origin.to_str().ok()).unwrap_or_default();
    let local = ["http://localhost:", "http://127.0.0.1:"].iter().any(|prefix| origin.starts_with(prefix));

    if local {
        if let Ok(value) = HeaderValue::from_str(origin) {
            response.headers_mut().insert(ACCESS_CONTROL_ALLOW_ORIGIN, value);
            response.headers_mut().insert(ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type"));
            response.headers_mut().insert(ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
        }
    }

    response
}

async fn invoke(State(served): State<Served>, headers: HeaderMap, Json(request): Json<Invoke>) -> Response {
    let answer = match commands::invoke(&served.desktop, &request.command, request.args).await {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(error) => json!({ "ok": false, "error": error }),
    };

    cors(&headers, Json(answer).into_response())
}

async fn events(State(served): State<Served>, headers: HeaderMap, Query(query): Query<After>) -> Response {
    let Some(after) = query.after else {
        let last = served.journal.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).last;

        return cors(&headers, Json(json!({ "last": last, "events": [] })).into_response());
    };
    let read = |served: &Served| {
        let journal = served.journal.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let newer: Vec<&AppEvent> = journal.events.iter().filter(|(number, _)| *number > after).map(|(_, event)| event).collect();

        (journal.last, serde_json::to_value(newer).unwrap_or_default())
    };
    let (mut last, mut found) = read(&served);

    if found.as_array().is_some_and(Vec::is_empty) {
        let _ = tokio::time::timeout(Duration::from_secs(20), served.arrived.notified()).await;

        (last, found) = read(&served);
    }

    cors(&headers, Json(json!({ "last": last, "events": found })).into_response())
}

async fn preflight(headers: HeaderMap) -> Response {
    cors(&headers, StatusCode::NO_CONTENT.into_response())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    let keychain: Arc<dyn Keychain> = if arguments.memory_keychain { Arc::new(MemoryKeychain::default()) } else { Arc::new(OsKeychain) };
    let desktop = Desktop::open(OpenOptions::new(Paths::from_env()?, keychain))?;
    let served = Served { desktop: desktop.clone(), journal: Arc::new(Mutex::new(Journal::default())), arrived: Arc::new(Notify::new()) };
    let mut stream = desktop.subscribe();
    let keeper = served.clone();

    tokio::spawn(async move {
        loop {
            match stream.recv().await {
                Ok(event) => {
                    let mut journal = keeper.journal.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

                    journal.last += 1;

                    let number = journal.last;

                    journal.events.push_back((number, event));

                    if journal.events.len() > KEPT_EVENTS {
                        journal.events.pop_front();
                    }

                    drop(journal);
                    keeper.arrived.notify_waiters();
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    desktop.run(|core| Ok(core.reconnect())).await?;
    desktop.start_background();

    let app = Router::new().route("/invoke", post(invoke).options(preflight)).route("/events", get(events).options(preflight)).with_state(served);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", arguments.port)).await?;

    println!("The desktop core is served on http://127.0.0.1:{} — for development only.", arguments.port);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    desktop.shutdown().await;

    Ok(())
}
