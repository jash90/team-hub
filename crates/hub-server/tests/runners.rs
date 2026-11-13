//! Runners as one list, and the keys people store for the team's runners (`docs/runners.md`).

mod common;

use axum::http::{Method, StatusCode};
use common::{test_app, Client};
use hub_server::{build_router, modules};
use serde_json::{json, Value};

/// A client that sends the person's API token instead of their session.
async fn with_token(person: &mut Client) -> Client {
    let token = person.post("/api/me/tokens", json!({ "name": "runner" })).await["token"].as_str().expect("a token").to_string();
    let mut client = Client::new(person.router.clone());
    client.bearer = Some(token);

    client
}

fn named<'a>(list: &'a Value, name: &str) -> &'a Value {
    list.as_array().expect("a list").iter().find(|entry| entry["name"] == name || entry["id"] == name).unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_persons_keys_go_in_and_never_come_back() {
    let app = test_app(modules());
    let mut admin = Client::new(build_router(app.clone()));
    admin.setup(false).await;
    let (mut anna, _) = admin.member("Anna Nowak", "anna@example.com").await;

    let keys = anna.get("/api/me/keys").await;
    let mut names: Vec<&str> = keys.as_array().unwrap().iter().map(|key| key["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(names, ["anthropic_api_key", "github_token"], "each key once, though two modules name the token: {keys}");
    assert_eq!((&named(&keys, "github_token")["set"], &named(&keys, "github_token")["tail"]), (&json!(false), &Value::Null));

    assert_eq!(anna.refused(Method::PUT, "/api/me/keys/anthropic_api_key", Some(json!({ "value": "two words" }))).await, StatusCode::BAD_REQUEST);
    assert_eq!(anna.refused(Method::PUT, "/api/me/keys/ssh_key", Some(json!({ "value": "x" }))).await, StatusCode::NOT_FOUND, "a key no module names");

    let after = anna.put("/api/me/keys/anthropic_api_key", json!({ "value": "  sk-ant-api03-anna-secret-7Qx2  " })).await;
    let stored = named(&after, "anthropic_api_key");
    assert_eq!((&stored["set"], &stored["tail"], stored["updatedAt"].is_string()), (&json!(true), &json!("…7Qx2"), true));
    assert!(!after.to_string().contains("anna-secret"), "{after}");
    assert!(!anna.get("/api/me/keys").await.to_string().contains("anna-secret"));

    // One person's key is nobody else's, and a token is not enough to change it.
    assert_eq!(named(&admin.get("/api/me/keys").await, "anthropic_api_key")["set"], false);
    let mut by_token = with_token(&mut anna).await;
    assert_eq!(by_token.get("/api/me/keys").await.as_array().map(Vec::len), Some(2), "reading which are set is fine");
    assert_eq!(by_token.refused(Method::PUT, "/api/me/keys/anthropic_api_key", Some(json!({ "value": "sk-ant-other" }))).await, StatusCode::FORBIDDEN);
    assert_eq!(by_token.refused(Method::DELETE, "/api/me/keys/anthropic_api_key", None).await, StatusCode::FORBIDDEN);

    // At rest it is not readable.
    let at_rest: Vec<u8> = app.read(|connection| Ok(connection.query_row("SELECT sealed FROM secret WHERE name LIKE 'user:%:anthropic_api_key'", [], |row| row.get(0))?)).unwrap();
    assert!(!String::from_utf8_lossy(&at_rest).contains("anna-secret"));

    let cleared = anna.delete("/api/me/keys/anthropic_api_key").await;
    assert_eq!(named(&cleared, "anthropic_api_key")["set"], false);
    assert_eq!(app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM secret WHERE name LIKE 'user:%'", [], |row| row.get::<_, i64>(0))?)).unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runner_that_does_both_kinds_of_work_is_listed_once_and_the_teams_takes_work_with_the_requesters_keys() {
    let app = test_app(modules());
    let mut admin = Client::new(build_router(app.clone()));
    admin.setup(false).await;
    admin.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    admin.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Login fails on Safari" })).await;
    admin.put("/api/m/e2e/projects/WEB/settings", json!({ "repo": "acme/shop" })).await;
    let (mut anna, anna_id) = admin.member("Anna Nowak", "anna@example.com").await;
    let mut cloud = with_token(&mut admin).await;
    let mut laptop = with_token(&mut anna).await;

    // The team's runner announces itself to both modules with one id; Anna's computer too.
    cloud
        .post("/api/m/agents/runners/heartbeat", json!({ "runnerId": "cloud-1", "name": "Cloud runner 1", "kinds": ["implement"], "version": "0.1.0", "interactive": false, "shared": true }))
        .await;
    let idle = cloud.post("/api/m/e2e/jobs/claim", json!({ "runnerId": "cloud-1", "runnerName": "Cloud runner 1", "interactive": false, "shared": true, "version": "0.1.0" })).await;
    assert_eq!(idle, Value::Null, "nothing is queued yet");
    laptop.post("/api/m/agents/runners/heartbeat", json!({ "runnerId": "anna-laptop", "name": "Anna's laptop", "kinds": ["implement"], "version": "0.1.0" })).await;
    laptop.post("/api/m/e2e/jobs/claim", json!({ "runnerId": "anna-laptop", "runnerName": "Anna's laptop", "pinnedOnly": true, "version": "0.1.0" })).await;

    let listed = anna.get("/api/runners").await;
    assert_eq!(listed.as_array().map(Vec::len), Some(2), "{listed}");
    let team = named(&listed, "cloud-1");
    let modules_of = |runner: &Value| runner["work"].as_array().unwrap().iter().map(|work| work["module"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!((&team["shared"], &team["interactive"], &team["online"]), (&json!(true), &json!(false), &json!(true)));
    assert_eq!(modules_of(team).len(), 2, "agent runs and tests: {team}");
    let own = named(&listed, "anna-laptop");
    assert_eq!((&own["shared"], &own["interactive"], &own["userId"]), (&json!(false), &json!(true), &json!(anna_id)));
    let tests = own["work"].as_array().unwrap().iter().find(|work| work["module"] == "e2e").expect("the laptop takes tests");
    assert_eq!((&tests["pinnedOnly"], &tests["what"]), (&json!(true), &json!(["tests"])));

    // Anna sends a test job to the team's runner: it comes with her GitHub token.
    anna.put("/api/me/keys/github_token", json!({ "value": "ghp_anna_0001" })).await;
    let job = anna.post("/api/m/e2e/projects/WEB/jobs", json!({ "onRunner": "cloud-1" })).await;
    assert_eq!((&job["state"], &job["onRunner"], &job["onRunnerName"]), (&json!("queued"), &json!("cloud-1"), &json!("Cloud runner 1")));
    assert_eq!(laptop.post("/api/m/e2e/jobs/claim", json!({ "runnerId": "anna-laptop", "runnerName": "Anna's laptop", "pinnedOnly": true })).await, Value::Null);
    let claimed = cloud.post("/api/m/e2e/jobs/claim", json!({ "runnerId": "cloud-1", "runnerName": "Cloud runner 1", "interactive": false, "shared": true })).await;
    assert_eq!((&claimed["job"]["id"], &claimed["credentials"]["githubToken"]), (&job["id"], &json!("ghp_anna_0001")), "{claimed}");

    // What a runner holds shows on the list, with a page to open.
    let busy = anna.get("/api/runners").await;
    let held: Vec<&Value> = named(&busy, "cloud-1")["work"].as_array().unwrap().iter().flat_map(|work| work["active"].as_array().unwrap()).collect();
    assert_eq!(held.len(), 1, "{busy}");
    assert!(held[0]["href"].as_str().unwrap().starts_with('/') && !held[0].to_string().contains("ghp_"));

    // A module switched off takes its half of the list with it.
    admin.put("/api/admin/modules/e2e", json!({ "enabled": false })).await;
    let without = anna.get("/api/runners").await;
    assert_eq!(modules_of(named(&without, "cloud-1")), ["agents"]);
}
