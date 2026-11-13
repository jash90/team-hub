//! The HTTP server: the core API, each module's routes, and the built web interface.

pub mod api;

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{HeaderName, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::Router;
use hub_app::auth::ModuleScope;
use hub_app::{ApiError, App, Module};
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

/// Every module compiled into this build, in the order they are loaded. Each is a Cargo
/// feature of this crate, named by the module's id (see `Cargo.toml`).
// A list literal cannot leave elements out by feature; pushes can.
#[allow(unused_mut, clippy::vec_init_then_push)]
pub fn modules() -> Vec<Arc<dyn Module>> {
    let mut modules: Vec<Arc<dyn Module>> = Vec::new();

    #[cfg(feature = "github")]
    modules.push(Arc::new(hub_mod_github::GithubModule));
    #[cfg(feature = "wiki")]
    modules.push(Arc::new(hub_mod_wiki::WikiModule));
    #[cfg(feature = "confluence")]
    modules.push(Arc::new(hub_mod_confluence::ConfluenceModule));
    #[cfg(feature = "jira")]
    modules.push(Arc::new(hub_mod_jira::JiraModule));
    #[cfg(feature = "timeline")]
    modules.push(Arc::new(hub_mod_timeline::TimelineModule));
    #[cfg(feature = "reports")]
    modules.push(Arc::new(hub_mod_reports::ReportsModule));
    #[cfg(feature = "e2e")]
    modules.push(Arc::new(hub_mod_e2e::E2eModule));
    #[cfg(feature = "agents")]
    modules.push(Arc::new(hub_mod_agents::AgentsModule));

    modules
}

/// Wraps a module's routes so that they do not exist while the module is switched off, and
/// marks the request as the module's, so that a project which has switched the module off
/// does not exist for it either.
fn guarded(app: &App, id: &'static str, routes: Router<App>) -> Router<App> {
    routes.route_layer(middleware::from_fn_with_state(app.clone(), move |State(app): State<App>, mut request: Request, next: Next| async move {
        if app.module_enabled(id) {
            request.extensions_mut().insert(ModuleScope(id));
            next.run(request).await
        } else {
            ApiError::not_found("Module").into_response()
        }
    }))
}

fn fixed_header(name: &'static str, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(HeaderName::from_static(name), HeaderValue::from_static(value))
}

pub fn build_router(app: App) -> Router {
    let mut api = api::router();
    let mut hooks: Router<App> = Router::new();

    for module in &app.modules {
        let id = module.meta().id;

        if let Some(routes) = module.routes() {
            api = api.nest(&format!("/m/{id}"), guarded(&app, id, routes));
        }

        if let Some(routes) = module.hooks() {
            hooks = hooks.nest(&format!("/{id}"), guarded(&app, id, routes));
        }
    }

    let index = app.config.web_dir.join("index.html");
    let web = ServeDir::new(&app.config.web_dir).fallback(ServeFile::new(index));

    Router::new()
        .nest("/api", api.fallback(api::not_found))
        .nest("/hooks", hooks.fallback(api::not_found))
        .fallback_service(web)
        .layer(CompressionLayer::new())
        .layer(fixed_header("x-content-type-options", "nosniff"))
        .layer(fixed_header("x-frame-options", "DENY"))
        .layer(fixed_header("referrer-policy", "same-origin"))
        .with_state(app)
}
