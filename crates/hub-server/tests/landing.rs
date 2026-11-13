//! The page about team-hub at `/`: who sees it, and who sees the application instead.

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use common::Client;
use http_body_util::BodyExt;
use hub_app::{App, Config};
use hub_server::build_router;
use serde_json::json;
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// An instance with a built interface and a landing page of one line each, so that a test
/// can tell which of them it was given.
fn instance(landing: bool) -> (Router, PathBuf) {
    let root = std::env::temp_dir().join(format!("team-hub-landing-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&root);

    let web = root.join("web");
    let page = root.join("landing");

    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::create_dir_all(page.join("pl")).unwrap();
    std::fs::create_dir_all(page.join("landing-assets").join("img")).unwrap();
    std::fs::write(web.join("index.html"), "the application").unwrap();
    std::fs::write(page.join("index.html"), "the page in English").unwrap();
    std::fs::write(page.join("pl").join("index.html"), "the page in Polish").unwrap();
    std::fs::write(page.join("robots.txt"), "User-agent: *").unwrap();
    std::fs::write(page.join("landing-assets").join("site.css"), "body {}").unwrap();
    std::fs::write(page.join("landing-assets").join("img").join("board.webp"), "a picture").unwrap();

    let mut config = Config::for_tests(root.join("data"));
    config.web_dir = web;
    config.landing_dir = landing.then_some(page);

    (build_router(App::new(config, Vec::new()).expect("the test application starts")), root)
}

struct Page {
    status: StatusCode,
    body: String,
    cache: String,
    policy: bool,
    location: String,
}

async fn open(client: &Client, path: &str) -> Page {
    let mut request = Request::builder().method(Method::GET).uri(path);

    if let Some(cookie) = &client.cookie {
        request = request.header(header::COOKIE, cookie);
    }

    let response = client.router.clone().oneshot(request.body(Body::empty()).unwrap()).await.expect("the router answers");
    let text = |name: header::HeaderName| response.headers().get(name).and_then(|value| value.to_str().ok()).unwrap_or_default().to_string();

    Page {
        status: response.status(),
        cache: text(header::CACHE_CONTROL),
        policy: response.headers().contains_key(header::CONTENT_SECURITY_POLICY),
        location: text(header::LOCATION),
        body: String::from_utf8(response.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap(),
    }
}

async fn set_up(client: &mut Client) {
    let (status, _) = client
        .send(Method::POST, "/api/setup", Some(json!({ "name": "Ada", "email": "ada@example.com", "password": "correct horse battery" })))
        .await;

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_visitor_sees_the_page_and_somebody_signed_in_sees_the_application() {
    let (router, _) = instance(true);
    let mut owner = Client::new(router.clone());

    // Nobody has set the instance up: whoever opens it is its owner, and gets the setup form.
    let fresh = open(&owner, "/").await;
    assert_eq!((fresh.status, fresh.body.as_str()), (StatusCode::OK, "the application"));

    set_up(&mut owner).await;

    let signed_in = open(&owner, "/").await;
    assert_eq!(signed_in.body, "the application");
    assert_eq!(signed_in.cache, "no-store");
    assert!(!signed_in.policy, "the page's policy would stop the application's own scripts");

    let visitor = open(&Client::new(router.clone()), "/").await;
    assert_eq!((visitor.status, visitor.body.as_str()), (StatusCode::OK, "the page in English"));
    // A browser that kept it would show it to the same person after they sign in.
    assert_eq!(visitor.cache, "no-store");
    assert!(visitor.policy);

    // A cookie that is no session is a visitor's.
    let mut stale = Client::new(router.clone());
    stale.cookie = Some("th_session=nothing".to_string());
    assert_eq!(open(&stale, "/").await.body, "the page in English");

    // Signing out makes a visitor of the owner again.
    let (status, _) = owner.send(Method::POST, "/api/logout", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(open(&owner, "/").await.body, "the page in English");

    // The sign-in the page links to, and every other address, is the application's.
    assert_eq!(open(&Client::new(router), "/login").await.body, "the application");
}

#[tokio::test]
async fn the_polish_page_its_files_and_robots_are_for_anyone() {
    let (router, _) = instance(true);
    let mut owner = Client::new(router.clone());
    set_up(&mut owner).await;

    for client in [&owner, &Client::new(router.clone())] {
        let polish = open(client, "/pl/").await;
        assert_eq!((polish.status, polish.body.as_str()), (StatusCode::OK, "the page in Polish"));
        assert!(polish.policy);

        let bare = open(client, "/pl").await;
        assert_eq!((bare.status, bare.location.as_str()), (StatusCode::PERMANENT_REDIRECT, "/pl/"));

        assert_eq!(open(client, "/robots.txt").await.body, "User-agent: *");

        let picture = open(client, "/landing-assets/img/board.webp").await;
        assert_eq!((picture.status, picture.body.as_str()), (StatusCode::OK, "a picture"));
        assert_eq!(picture.cache, "public, max-age=31536000, immutable");

        assert_eq!(open(client, "/landing-assets/site.css?v=3").await.cache, "public, max-age=31536000, immutable");
        assert_eq!(open(client, "/landing-assets/site.css").await.cache, "public, max-age=3600");

        // A file the page does not have is missing, not the application.
        assert_eq!(open(client, "/landing-assets/img/none.webp").await.status, StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn an_instance_without_the_page_shows_the_sign_in_right_away() {
    for (router, root) in [instance(false), {
        // Switched on, but the folder is not there: the same as switched off.
        let (_, root) = instance(true);
        std::fs::remove_dir_all(root.join("landing")).unwrap();

        let mut config = Config::for_tests(root.join("data2"));
        config.web_dir = root.join("web");
        config.landing_dir = Some(root.join("landing"));

        (build_router(App::new(config, Vec::new()).unwrap()), root)
    }] {
        let mut owner = Client::new(router.clone());
        set_up(&mut owner).await;

        let visitor = Client::new(router);

        assert_eq!(open(&visitor, "/").await.body, "the application", "{}", root.display());
        // No page in Polish either: the address is the application's, like any other.
        assert_eq!(open(&visitor, "/pl/").await.body, "the application");
    }
}
