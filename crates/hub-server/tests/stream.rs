//! The live stream: who hears what.

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{test_app, Client};
use http_body_util::BodyExt;
use hub_app::Actor;
use hub_server::build_router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// An open stream, and the kinds of the events that have arrived on it.
struct Listener {
    body: Body,
}

impl Listener {
    async fn open(client: &Client) -> Self {
        let request = Request::builder()
            .uri("/api/stream")
            .header(header::COOKIE, client.cookie.clone().expect("a signed-in client"))
            .body(Body::empty())
            .expect("a valid request");
        let response = client.router.clone().oneshot(request).await.expect("the router answers");
        assert_eq!(response.status(), StatusCode::OK);

        Self { body: response.into_body() }
    }

    /// The kinds of everything heard until the stream falls quiet.
    async fn heard(&mut self) -> Vec<String> {
        let mut kinds = Vec::new();

        while let Ok(Some(Ok(frame))) = tokio::time::timeout(Duration::from_millis(300), self.body.frame()).await {
            let Some(data) = frame.data_ref() else { continue };

            for line in String::from_utf8_lossy(data).lines() {
                if let Some(event) = line.strip_prefix("data:").and_then(|json| serde_json::from_str::<Value>(json.trim()).ok()) {
                    assert!(event.get("adminsOnly").is_none(), "the mark is the server's own business");
                    kinds.push(event["kind"].as_str().unwrap_or_default().to_string());
                }
            }
        }

        kinds
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_reaches_only_those_who_may_see_what_it_is_about() {
    let app = test_app(Vec::new());
    let mut admin = Client::new(build_router(app.clone()));
    admin.setup(false).await;
    let (member, member_id) = admin.member("Anna Nowak", "anna@example.com").await;
    let (outsider, _) = admin.member("Piotr Zieliński", "piotr@example.com").await;

    admin.post("/api/projects", json!({ "key": "SEC", "name": "Secret", "isPrivate": true })).await;
    admin.post("/api/projects", json!({ "key": "PUB", "name": "Public" })).await;
    admin.put(&format!("/api/projects/SEC/members/{member_id}"), json!({ "role": "viewer" })).await;

    let mut for_admin = Listener::open(&admin).await;
    let mut for_member = Listener::open(&member).await;
    let mut for_outsider = Listener::open(&outsider).await;

    admin.post("/api/issues", json!({ "projectKey": "SEC", "summary": "Hidden plan" })).await;
    admin.post("/api/issues", json!({ "projectKey": "PUB", "summary": "Open plan" })).await;
    app.write(Actor::system("test"), |tx| {
        tx.emit_to_admins("import.progress", json!({ "projectKey": "SEC" }))?;
        tx.emit("instance.note", None, None, json!({}))
    })
    .expect("the events are recorded");

    let created = |heard: &[String]| heard.iter().filter(|kind| *kind == "issue.created").count();
    let has = |heard: &[String], kind: &str| heard.iter().any(|heard| heard == kind);

    let heard = for_admin.heard().await;
    assert_eq!((created(&heard), has(&heard, "import.progress"), has(&heard, "instance.note")), (2, true, true), "{heard:?}");

    let heard = for_member.heard().await;
    assert_eq!((created(&heard), has(&heard, "import.progress"), has(&heard, "instance.note")), (2, false, true), "{heard:?}");

    let heard = for_outsider.heard().await;
    assert_eq!((created(&heard), has(&heard, "import.progress"), has(&heard, "instance.note")), (1, false, true), "{heard:?}");
}
