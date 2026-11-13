//! The live stream of changes.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use futures_util::{Stream, StreamExt};
use hub_app::access::project_access_by_id;
use hub_app::{App, AuthUser};
use tokio_stream::wrappers::BroadcastStream;

pub fn routes() -> Router<App> {
    Router::new().route("/stream", get(stream))
}

/// Server-sent events: every change the caller may see, as it commits.
async fn stream(State(app): State<App>, user: AuthUser) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let events = BroadcastStream::new(app.subscribe()).filter_map(move |event| {
        let app = app.clone();
        let user = user.clone();

        async move {
            let event = event.ok()?;

            if let Some(project_id) = event.project_id {
                app.read(|connection| project_access_by_id(connection, &user, project_id)).ok()?;
            }

            Some(Ok(SseEvent::default().event("change").data(serde_json::to_string(&event).ok()?)))
        }
    });

    Sse::new(events).keep_alive(KeepAlive::default())
}
