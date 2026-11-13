//! Connecting a desktop app to a person's account (`docs/desktop-pairing.md`).

mod common;

use axum::http::{Method, StatusCode};
use common::{test_app, Client};
use hub_app::App;
use hub_server::{build_router, modules};
use serde_json::{json, Value};

/// What the app does first: nobody is signed in.
async fn ask(router: &axum::Router, name: &str) -> Value {
    Client::new(router.clone()).post("/api/desktop/pairings", json!({ "name": name, "runnerId": "desktop-1a2b" })).await
}

/// What the app does next, with the secret it kept.
async fn collect(router: &axum::Router, pairing: &Value, secret: &str) -> (StatusCode, Value) {
    let path = format!("/api/desktop/pairings/{}/token", pairing["id"].as_str().unwrap());

    Client::new(router.clone()).send(Method::POST, &path, Some(json!({ "secret": secret }))).await
}

fn secret_of(pairing: &Value) -> &str {
    pairing["secret"].as_str().expect("a secret")
}

fn waiting(app: &App) -> i64 {
    app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM desktop_pairing", [], |row| row.get(0))?)).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_asks_a_person_says_yes_and_the_app_collects_its_token_once() {
    let app = test_app(modules());
    let router = build_router(app.clone());
    let mut admin = Client::new(router.clone());
    admin.setup(false).await;
    let (mut anna, anna_id) = admin.member("Anna Nowak", "anna@example.com").await;

    let pairing = ask(&router, "  Desktop ·  Anna's laptop ").await;
    let code = pairing["code"].as_str().expect("a code").to_string();
    assert_eq!((code.len(), secret_of(&pairing).len(), pairing["id"].as_str().map(str::len)), (8, 64, Some(32)), "{pairing}");
    assert!(pairing["expiresAt"].as_str().is_some_and(|at| at > hub_app::db::now_iso().as_str()));

    // The secret is kept as a digest, like a token.
    let stored: String = app.read(|connection| Ok(connection.query_row("SELECT secret_hash FROM desktop_pairing", [], |row| row.get(0))?)).unwrap();
    assert_eq!(stored, hub_app::auth::digest(secret_of(&pairing)));

    // Until somebody says yes the app is told to wait, and there is no token to be had.
    let (status, answer) = collect(&router, &pairing, secret_of(&pairing)).await;
    assert_eq!((status, &answer["status"], &answer["token"]), (StatusCode::ACCEPTED, &json!("pending"), &Value::Null));
    assert_eq!(anna.get("/api/me/tokens").await, json!([]));

    // Anna is shown the app under the code, typed as people type it, and says yes.
    let typed = format!("{}-{}", code[..4].to_lowercase(), &code[4..]);
    let shown = anna.get(&format!("/api/desktop/pairings/{typed}")).await;
    assert_eq!((&shown["name"], &shown["code"], &shown["approved"]), (&json!("Desktop · Anna's laptop"), &json!(code), &json!(false)));
    assert!(shown.get("secret").is_none() && shown.get("id").is_none(), "the person sees neither what the app holds: {shown}");
    let approved = anna.post(&format!("/api/desktop/pairings/{code}/approve"), json!({})).await;
    assert_eq!(approved["approved"], true);
    assert_eq!(anna.get("/api/me/tokens").await, json!([]), "saying yes makes no token yet");

    // Somebody else cannot take over an app that Anna has said yes to.
    assert_eq!(admin.refused(Method::POST, &format!("/api/desktop/pairings/{code}/approve"), Some(json!({}))).await, StatusCode::CONFLICT);
    assert_eq!(admin.refused(Method::POST, &format!("/api/desktop/pairings/{code}/decline"), Some(json!({}))).await, StatusCode::CONFLICT);

    // The app collects: a token of Anna's, named after the app.
    let (status, granted) = collect(&router, &pairing, secret_of(&pairing)).await;
    assert_eq!(status, StatusCode::OK, "{granted}");
    let token = granted["token"].as_str().expect("a token").to_string();
    let tokens = anna.get("/api/me/tokens").await;
    assert_eq!((tokens.as_array().map(Vec::len), &tokens[0]["name"], &tokens[0]["id"]), (Some(1), &json!("Desktop · Anna's laptop"), &granted["tokenId"]));
    let mut desktop = Client::new(router.clone());
    desktop.bearer = Some(token);
    assert_eq!(desktop.get("/api/bootstrap").await["user"]["id"], anna_id);

    // Once. After that the request is no more, for the app and for the person.
    assert_eq!(collect(&router, &pairing, secret_of(&pairing)).await.0, StatusCode::GONE);
    assert_eq!(anna.refused(Method::GET, &format!("/api/desktop/pairings/{code}"), None).await, StatusCode::GONE);
    assert_eq!((waiting(&app), anna.get("/api/me/tokens").await.as_array().map(Vec::len)), (0, Some(1)));

    // The app disconnects by removing its own token.
    let (status, _) = desktop.send(Method::DELETE, &format!("/api/me/tokens/{}", granted["tokenId"]), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(desktop.refused(Method::GET, "/api/dashboard", None).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_person_signed_in_in_a_browser_can_say_yes() {
    let app = test_app(modules());
    let router = build_router(app.clone());
    let mut admin = Client::new(router.clone());
    admin.setup(false).await;

    let pairing = ask(&router, "Desktop").await;
    let approve = format!("/api/desktop/pairings/{}/approve", pairing["code"].as_str().unwrap());
    let show = format!("/api/desktop/pairings/{}", pairing["code"].as_str().unwrap());

    // Nobody at all.
    assert_eq!(Client::new(router.clone()).refused(Method::POST, &approve, Some(json!({}))).await, StatusCode::UNAUTHORIZED);
    assert_eq!(Client::new(router.clone()).refused(Method::GET, &show, None).await, StatusCode::UNAUTHORIZED);

    // An API token: a token must not be able to make itself more tokens.
    let token = admin.post("/api/me/tokens", json!({ "name": "script" })).await["token"].as_str().unwrap().to_string();
    let mut by_token = Client::new(router.clone());
    by_token.bearer = Some(token);
    assert_eq!(by_token.refused(Method::POST, &approve, Some(json!({}))).await, StatusCode::FORBIDDEN);
    assert_eq!(by_token.refused(Method::GET, &show, None).await, StatusCode::FORBIDDEN);
    assert_eq!(by_token.refused(Method::POST, &approve.replace("approve", "decline"), Some(json!({}))).await, StatusCode::FORBIDDEN);

    // The session's cookie without its CSRF token: another site's form.
    let mut forged = admin.clone();
    forged.csrf = None;
    assert_eq!(forged.refused(Method::POST, &approve, Some(json!({}))).await, StatusCode::FORBIDDEN);

    // None of that got the app anything.
    let (status, answer) = collect(&router, &pairing, secret_of(&pairing)).await;
    assert_eq!((status, &answer["token"]), (StatusCode::ACCEPTED, &Value::Null));

    // The person themselves.
    admin.post(&approve, json!({})).await;
    assert_eq!(collect(&router, &pairing, secret_of(&pairing)).await.0, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_wrong_secret_gets_nothing_and_spoils_nothing() {
    let app = test_app(modules());
    let router = build_router(app.clone());
    let mut admin = Client::new(router.clone());
    admin.setup(false).await;

    let pairing = ask(&router, "Desktop").await;
    let other = ask(&router, "Another desktop").await;
    admin.post(&format!("/api/desktop/pairings/{}/approve", pairing["code"].as_str().unwrap()), json!({})).await;

    // A guess, another app's secret, and the code, which the person's screen shows.
    for wrong in ["", "0000", secret_of(&other), pairing["code"].as_str().unwrap()] {
        let (status, answer) = collect(&router, &pairing, wrong).await;
        assert_eq!((status, &answer["token"]), (StatusCode::FORBIDDEN, &Value::Null), "{wrong}: {answer}");
    }

    assert_eq!(admin.get("/api/me/tokens").await, json!([]));

    // The app that asked still gets its token.
    let (status, granted) = collect(&router, &pairing, secret_of(&pairing)).await;
    assert_eq!((status, granted["token"].is_string()), (StatusCode::OK, true));

    // A request nobody ever made.
    let (status, _) = collect(&router, &json!({ "id": "0".repeat(32) }), secret_of(&pairing)).await;
    assert_eq!(status, StatusCode::GONE);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_lasts_ten_minutes_and_a_declined_one_not_that() {
    let app = test_app(modules());
    let router = build_router(app.clone());
    let mut admin = Client::new(router.clone());
    admin.setup(false).await;

    // Out of time, though somebody had said yes.
    let late = ask(&router, "Desktop").await;
    let code = late["code"].as_str().unwrap();
    admin.post(&format!("/api/desktop/pairings/{code}/approve"), json!({})).await;
    app.db.write(|tx| Ok(tx.execute("UPDATE desktop_pairing SET expires_at = '2020-01-01T00:00:00.000Z' WHERE id = ?1", [late["id"].as_str()])?)).unwrap();
    assert_eq!(admin.refused(Method::GET, &format!("/api/desktop/pairings/{code}"), None).await, StatusCode::GONE);
    assert_eq!(admin.refused(Method::POST, &format!("/api/desktop/pairings/{code}/approve"), Some(json!({}))).await, StatusCode::GONE);
    let (status, answer) = collect(&router, &late, secret_of(&late)).await;
    assert_eq!((status, &answer["error"]["code"]), (StatusCode::GONE, &json!("gone")));
    assert_eq!((waiting(&app), admin.get("/api/me/tokens").await), (0, json!([])));

    // Declined: the app hears the same, at once.
    let declined = ask(&router, "Desktop").await;
    admin.post(&format!("/api/desktop/pairings/{}/decline", declined["code"].as_str().unwrap()), json!({})).await;
    assert_eq!(collect(&router, &declined, secret_of(&declined)).await.0, StatusCode::GONE);
    assert_eq!(admin.get("/api/me/tokens").await, json!([]));

    // Somebody deactivated between saying yes and the app collecting.
    let (mut anna, anna_id) = admin.member("Anna Nowak", "anna@example.com").await;
    let orphan = ask(&router, "Desktop").await;
    anna.post(&format!("/api/desktop/pairings/{}/approve", orphan["code"].as_str().unwrap()), json!({})).await;
    admin.patch(&format!("/api/users/{anna_id}"), json!({ "active": false })).await;
    assert_eq!(collect(&router, &orphan, secret_of(&orphan)).await.0, StatusCode::GONE);
    assert_eq!(app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM api_token", [], |row| row.get::<_, i64>(0))?)).unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_so_many_apps_wait_at_once_and_old_requests_make_room() {
    let app = test_app(modules());
    let router = build_router(app.clone());
    let mut admin = Client::new(router.clone());
    admin.setup(false).await;

    let mut codes = std::collections::HashSet::new();

    for _ in 0..hub_app::services::pairing::MAX_OPEN {
        codes.insert(ask(&router, "Desktop").await["code"].as_str().unwrap().to_string());
    }

    assert_eq!(codes.len() as i64, hub_app::services::pairing::MAX_OPEN, "every code is its own");

    let (status, answer) = Client::new(router.clone()).send(Method::POST, "/api/desktop/pairings", Some(json!({ "name": "One too many" }))).await;
    assert_eq!((status, &answer["error"]["code"]), (StatusCode::TOO_MANY_REQUESTS, &json!("too_many_pairings")));

    // Those that ran out of time do not count.
    app.db.write(|tx| Ok(tx.execute("UPDATE desktop_pairing SET expires_at = '2020-01-01T00:00:00.000Z' WHERE rowid % 2 = 0", [])?)).unwrap();
    ask(&router, "Room again").await;
    assert_eq!(waiting(&app), hub_app::services::pairing::MAX_OPEN / 2 + 1);

    // A request without a usable name is not taken.
    for name in ["", "   ", &"x".repeat(81)] {
        assert_eq!(Client::new(router.clone()).refused(Method::POST, "/api/desktop/pairings", Some(json!({ "name": name }))).await, StatusCode::BAD_REQUEST);
    }
}
