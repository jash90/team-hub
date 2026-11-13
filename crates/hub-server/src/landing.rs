//! The page a visitor meets first: what team-hub is, with the way to the sign-in and to the
//! desktop app. It is the `landing/` folder of the repository, served as it is.
//!
//! `/` is that page for somebody who is not signed in, and the application for somebody who
//! is. An instance nobody has set up yet shows its setup form instead: whoever opens it
//! first is its owner, not a visitor. `/pl/` is the same page in Polish, for anyone.

use std::path::{Path, PathBuf};

use axum::extract::{Request, State};
use axum::http::header::{HeaderValue, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, VARY};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use hub_app::auth::MaybeUser;
use hub_app::services::users;
use hub_app::{ApiError, App};
use tower_http::services::ServeDir;

/// The folder the pages' pictures, fonts, stylesheet and script are in, and the address
/// they are served under. Named so that it cannot meet the interface's own `/assets`.
const ASSETS: &str = "landing-assets";

/// The pages load nothing from anywhere else and have no inline script or style, so the
/// policy can say so.
const POLICY: &str = "default-src 'none'; img-src 'self'; style-src 'self'; script-src 'self'; font-src 'self'; \
                      base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The folder to serve, when this instance shows the page and the page is there.
pub fn directory(app: &App) -> Option<PathBuf> {
    let directory = app.config.landing_dir.clone()?;

    if directory.join("index.html").is_file() {
        Some(directory)
    } else {
        tracing::info!("no landing page in {}: visitors get the sign-in right away", directory.display());
        None
    }
}

async fn page(path: &Path, cache: &'static str, policy: Option<&'static str>) -> Response {
    let Ok(html) = tokio::fs::read(path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = html.into_response();
    let headers = response.headers_mut();

    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));

    if let Some(policy) = policy {
        headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(policy));
    }

    response
}

/// Whether this request should see the page about team-hub and not the application.
async fn is_visitor(app: &App, caller: Result<MaybeUser, ApiError>) -> bool {
    if caller.is_ok_and(|caller| caller.user.is_some()) {
        return false;
    }

    // When that cannot be told, the application says what is wrong better than this page.
    app.run(|app| app.read(users::setup_required)).await.is_ok_and(|setup_required| !setup_required)
}

pub fn routes(app: &App, directory: PathBuf) -> Router<App> {
    let application = app.config.web_dir.join("index.html");
    let english = directory.join("index.html");
    let polish = directory.join("pl").join("index.html");
    let robots = directory.join("robots.txt");

    // What `/` answers depends on the session, so nothing may keep it: a browser that kept
    // the page would show it to somebody who has signed in since.
    let front = move |State(app): State<App>, caller: Result<MaybeUser, ApiError>| async move {
        let mut response = if is_visitor(&app, caller).await {
            page(&english, "no-store", Some(POLICY)).await
        } else {
            page(&application, "no-store", None).await
        };

        response.headers_mut().insert(VARY, HeaderValue::from_static("cookie"));
        response
    };

    let assets = Router::new().fallback_service(ServeDir::new(directory.join(ASSETS))).layer(middleware::from_fn(cached));

    Router::new()
        .route("/", get(front))
        .route("/pl", get(|| async { Redirect::permanent("/pl/") }))
        .route("/pl/", get(move || async move { page(&polish, "public, max-age=300, must-revalidate", Some(POLICY)).await }))
        .route(
            "/robots.txt",
            get(move || async move {
                match tokio::fs::read(&robots).await {
                    Ok(text) => ([(CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response(),
                    Err(_) => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .nest(&format!("/{ASSETS}"), assets)
}

/// Pictures and fonts never change under the same name, and the stylesheet and the script
/// are linked with `?v=N`, raised when they change. Anything else is checked again soon.
async fn cached(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let named = path.starts_with("/img/") || path.starts_with("/fonts/");
    let versioned = request.uri().query().is_some_and(|query| query.split('&').any(|part| part.starts_with("v=")));
    let mut response = next.run(request).await;

    if response.status().is_success() {
        let cache = if named || versioned { "public, max-age=31536000, immutable" } else { "public, max-age=3600" };

        response.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    }

    response
}
