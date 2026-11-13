mod common;

use axum::extract::Path;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::{keys, test_app, Client};
use hmac::{Hmac, Mac};
use hub_server::{build_router, modules};
use serde_json::{json, Value};
use sha2::Sha256;

struct Hub {
    client: Client,
    secret: String,
    deliveries: u32,
}

impl Hub {
    /// A project WEB with the default workflow, the repository acme/web connected, a webhook secret set.
    async fn new() -> Self {
        let mut client = Client::new(build_router(test_app(modules())));
        client.setup(false).await;
        client.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
        client.post("/api/m/github/projects/WEB/repos", json!({ "fullName": "acme/web" })).await;
        let secret = client.post("/api/m/github/settings/webhook-secret", json!({})).await["secret"].as_str().unwrap().to_string();

        Self { client, secret, deliveries: 0 }
    }

    async fn deliver(&mut self, event: &str, payload: Value) -> Value {
        self.deliveries += 1;
        let id = format!("delivery-{}", self.deliveries);
        let (status, value) = self.deliver_as(event, &payload, &id, None).await;
        assert_eq!(status, StatusCode::OK, "{event}: {value}");

        value
    }

    async fn deliver_as(&mut self, event: &str, payload: &Value, id: &str, signature: Option<String>) -> (StatusCode, Value) {
        let body = payload.to_string().into_bytes();
        let signature = signature.unwrap_or_else(|| {
            let mut mac = Hmac::<Sha256>::new_from_slice(self.secret.as_bytes()).unwrap();
            mac.update(&body);

            format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
        });
        let headers = [("x-github-event", event.to_string()), ("x-github-delivery", id.to_string()), ("x-hub-signature-256", signature)];

        self.client.send_raw("/hooks/github", &headers, body).await
    }

    async fn issue(&mut self, summary: &str) -> String {
        self.client.post("/api/issues", json!({ "projectKey": "WEB", "summary": summary })).await["key"].as_str().unwrap().to_string()
    }

    async fn status(&mut self, key: &str) -> String {
        let issue = self.client.get(&format!("/api/issues/{key}")).await;
        let project = self.client.get("/api/projects/WEB").await;
        let status = project["statuses"].as_array().unwrap().iter().find(|status| status["id"] == issue["statusId"]).unwrap();

        status["name"].as_str().unwrap().to_string()
    }
}

fn pull(number: i64, title: &str, branch: &str, action: &str, extra: Value) -> Value {
    let mut pr = json!({
        "number": number,
        "title": title,
        "body": "",
        "state": "open",
        "draft": false,
        "merged": false,
        "user": { "login": "octocat" },
        "head": { "ref": branch, "sha": format!("sha-{number}") },
        "base": { "ref": "main" },
        "html_url": format!("https://github.com/acme/web/pull/{number}"),
        "updated_at": "2026-11-12T10:00:00Z",
    });

    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        pr[key] = value;
    }

    json!({ "action": action, "repository": { "full_name": "acme/web" }, "pull_request": pr })
}

fn push(branch: &str, created: bool, messages: &[&str]) -> Value {
    let commits: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            json!({
                "id": format!("{branch}-{index}"),
                "message": message,
                "timestamp": "2026-11-12T11:00:00+01:00",
                "url": format!("https://github.com/acme/web/commit/{branch}-{index}"),
                "author": { "name": "Mona", "username": "octocat", "email": "admin@example.com" },
            })
        })
        .collect();

    json!({ "ref": format!("refs/heads/{branch}"), "created": created, "deleted": false, "after": "abc", "repository": { "full_name": "acme/web" }, "commits": commits })
}

#[tokio::test(flavor = "multi_thread")]
async fn only_signed_deliveries_are_read_and_each_once() {
    let mut hub = Hub::new().await;
    hub.issue("Login").await;
    let payload = pull(1, "WEB-1 Login", "feature/login", "opened", json!({}));

    assert_eq!(hub.deliver_as("pull_request", &payload, "a", Some("sha256=00".into())).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(hub.deliver_as("pull_request", &payload, "a", Some("nonsense".into())).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(hub.client.get("/api/issues/WEB-1").await["ext"], json!({}));

    assert_eq!(hub.deliver_as("pull_request", &payload, "a", None).await.0, StatusCode::OK);
    assert_eq!(hub.deliver_as("pull_request", &payload, "a", None).await.1["outcome"], "duplicate");
    assert_eq!(hub.deliver("ping", json!({ "zen": "Keep it logically awesome." })).await["outcome"], "pong");

    let unknown = pull(2, "WEB-1 again", "x", "opened", json!({}));
    let mut elsewhere = unknown.clone();
    elsewhere["repository"]["full_name"] = json!("someone/else");
    assert!(hub.deliver("pull_request", elsewhere).await["outcome"].as_str().unwrap().starts_with("ignored"));

    let secrets = hub.client.get("/api/m/github/settings").await;
    assert!(secrets["webhookSecretHint"].as_str().unwrap().starts_with('…'));
    assert!(!secrets.to_string().contains(&hub.secret), "the secret is shown once, when generated");
}

#[tokio::test(flavor = "multi_thread")]
async fn code_moves_the_issue_through_the_workflow() {
    let mut hub = Hub::new().await;
    let key = hub.issue("Card payments").await;

    hub.deliver("push", push("feature/web-1-card-payments", true, &["WEB-1 Card form"])).await;
    assert_eq!(hub.status(&key).await, "In Progress", "a new branch starts the work");

    hub.deliver("pull_request", pull(7, "WEB-1 Card payments", "feature/web-1-card-payments", "opened", json!({ "draft": true }))).await;
    assert_eq!(hub.status(&key).await, "In Progress", "a draft is not yet up for review");

    hub.deliver("pull_request", pull(7, "WEB-1 Card payments", "feature/web-1-card-payments", "ready_for_review", json!({}))).await;
    assert_eq!(hub.status(&key).await, "In Review");

    let mut review = pull(7, "WEB-1 Card payments", "feature/web-1-card-payments", "submitted", json!({}));
    review["review"] = json!({ "state": "changes_requested" });
    hub.deliver("pull_request_review", review).await;
    hub.deliver(
        "check_suite",
        json!({ "repository": { "full_name": "acme/web" }, "check_suite": { "head_sha": "sha-7", "status": "completed", "conclusion": "failure" } }),
    )
    .await;

    let issue = hub.client.get(&format!("/api/issues/{key}")).await;
    let dev = &issue["ext"]["github"];

    assert_eq!((dev["branches"].as_i64(), dev["commits"].as_i64()), (Some(1), Some(1)));
    assert_eq!(dev["prs"][0]["review"], "changes_requested");
    assert_eq!(dev["prs"][0]["checks"], "failing");
    assert_eq!(dev["drift"], json!(["changes_requested", "checks_failing"]));
    assert_eq!(issue["panels"]["github"]["suggestedBranch"], "feature/WEB-1-card-payments");
    assert_eq!(issue["panels"]["github"]["commits"][0]["committedAt"], "2026-11-12T10:00:00.000Z");

    hub.deliver(
        "pull_request",
        pull(
            7,
            "WEB-1 Card payments",
            "feature/web-1-card-payments",
            "closed",
            json!({ "state": "closed", "merged": true, "merged_at": "2026-11-12T12:00:00Z" }),
        ),
    )
    .await;

    assert_eq!(hub.status(&key).await, "Done");
    assert!(hub.client.get(&format!("/api/issues/{key}")).await["ext"]["github"]["drift"].as_array().unwrap().is_empty());

    let history = hub.client.get(&format!("/api/issues/{key}")).await["changes"].clone();
    assert!(history.as_array().unwrap().iter().all(|change| change["actorLabel"] == "GitHub"), "the history says who moved it");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merge_finishes_an_issue_only_when_no_pull_request_is_left_open() {
    let mut hub = Hub::new().await;
    let key = hub.issue("Two parts").await;

    hub.deliver("pull_request", pull(1, "WEB-1 part one", "a", "opened", json!({}))).await;
    hub.deliver("pull_request", pull(2, "WEB-1 part two", "b", "opened", json!({}))).await;
    hub.deliver("pull_request", pull(1, "WEB-1 part one", "a", "closed", json!({ "state": "closed", "merged": true }))).await;
    assert_eq!(hub.status(&key).await, "In Review");

    hub.deliver("pull_request", pull(2, "WEB-1 part two", "b", "closed", json!({ "state": "closed", "merged": true }))).await;
    assert_eq!(hub.status(&key).await, "Done");

    hub.deliver("pull_request", pull(3, "WEB-1 follow-up", "c", "opened", json!({}))).await;
    assert_eq!(hub.status(&key).await, "Done", "finished work is not reopened by new code");
    assert_eq!(hub.client.get(&format!("/api/issues/{key}")).await["ext"]["github"]["drift"], json!(["done_with_open_pr"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn rules_can_be_changed_and_a_closing_phrase_works_without_one() {
    let mut hub = Hub::new().await;
    let key = hub.issue("Typos").await;

    let code = hub.client.get("/api/m/github/projects/WEB").await;
    let triggers: Vec<&str> =
        code["rules"].as_array().unwrap().iter().filter(|rule| !rule["statusId"].is_null()).map(|rule| rule["trigger"].as_str().unwrap()).collect();
    assert_eq!(triggers, ["branch_created", "pr_opened", "pr_merged"], "sensible defaults when the first repository is connected");

    hub.client.put("/api/m/github/projects/WEB/rules", json!([])).await;
    hub.deliver("pull_request", pull(1, "Fix typos", "typos", "opened", json!({ "body": "Fixes WEB-1" }))).await;
    assert_eq!(hub.status(&key).await, "To Do", "without rules, opening a pull request moves nothing");

    hub.deliver("pull_request", pull(1, "Fix typos", "typos", "closed", json!({ "body": "Fixes WEB-1", "state": "closed", "merged": true }))).await;
    assert_eq!(hub.status(&key).await, "Done");
}

#[tokio::test(flavor = "multi_thread")]
async fn commit_messages_can_comment_log_time_and_move() {
    let mut hub = Hub::new().await;
    let key = hub.issue("Search").await;

    hub.deliver("push", push("main", false, &["WEB-1 #comment Index rebuilt nightly #time 1h 30m #in-review", "no key here"])).await;

    let issue = hub.client.get(&format!("/api/issues/{key}")).await;

    assert_eq!(issue["comments"][0]["body"], "Index rebuilt nightly");
    assert_eq!(issue["comments"][0]["authorId"], issue["reporterId"], "the commit's e-mail matched a user");
    assert_eq!(issue["spentMinutes"], 90);
    assert_eq!(hub.status(&key).await, "In Review");
    assert_eq!(issue["panels"]["github"]["commits"].as_array().unwrap().len(), 1);

    hub.deliver("push", push("main", false, &["WEB-1 #comment Index rebuilt nightly #time 1h 30m #in-review"])).await;
    assert_eq!(hub.client.get(&format!("/api/issues/{key}")).await["spentMinutes"], 90, "a commit seen again does nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn queries_see_code() {
    let mut hub = Hub::new().await;

    for summary in ["Open", "Merged", "Plain", "Failing"] {
        hub.issue(summary).await;
    }

    hub.client.put("/api/m/github/projects/WEB/rules", json!([])).await;
    hub.deliver("pull_request", pull(1, "WEB-1 open", "a", "opened", json!({}))).await;
    hub.deliver("pull_request", pull(2, "WEB-2 merged", "b", "closed", json!({ "state": "closed", "merged": true }))).await;
    hub.deliver("pull_request", pull(4, "WEB-4 failing", "d", "opened", json!({}))).await;
    hub.deliver("status", json!({ "repository": { "full_name": "acme/web" }, "sha": "sha-4", "state": "failure" })).await;

    assert_eq!(hub.client.search_keys("pr = open ORDER BY key").await, ["WEB-1", "WEB-4"]);
    assert_eq!(hub.client.search_keys("pr = merged").await, ["WEB-2"]);
    assert_eq!(hub.client.search_keys("pr is empty").await, ["WEB-3"]);
    assert_eq!(hub.client.search_keys("pr = none OR checks = failing ORDER BY key").await, ["WEB-3", "WEB-4"]);
    assert_eq!(hub.client.search_keys("drift = todo_with_code ORDER BY key").await, ["WEB-1", "WEB-2", "WEB-4"]);
    assert_eq!(hub.client.search_keys("drift = checks_failing").await, ["WEB-4"]);
    assert_eq!(hub.client.search_keys("drift is empty").await, ["WEB-3"]);
    assert_eq!(hub.client.get("/api/search?q=pr%20%3D%20sideways").await["error"]["message"], "\"sideways\" is not one of: open, merged, closed");

    let code = hub.client.get("/api/m/github/projects/WEB").await;
    assert_eq!(code["pulls"].as_array().unwrap().len(), 3);
    assert_eq!(keys(&Value::Array(code["drift"].as_array().unwrap().iter().map(|item| item["issue"].clone()).collect())).len(), 3);
    assert!(hub.client.get("/api/search/fields").await.as_array().unwrap().iter().any(|field| field["name"] == "drift"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_switched_off_module_is_gone() {
    let mut hub = Hub::new().await;
    hub.issue("Quiet").await;
    hub.deliver("pull_request", pull(1, "WEB-1 quiet", "a", "opened", json!({}))).await;
    hub.client.put("/api/admin/modules/github", json!({ "enabled": false })).await;

    assert_eq!(hub.client.refused(Method::GET, "/api/m/github/projects/WEB", None).await, StatusCode::NOT_FOUND);
    assert_eq!(hub.deliver_as("ping", &json!({}), "z", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(hub.client.get("/api/issues/WEB-1").await["ext"], json!({}));
    assert_eq!(hub.client.get("/api/search?q=pr%20%3D%20open").await["error"]["message"], "Unknown field \"pr\"");
}

async fn fake_github() -> String {
    async fn authorized(headers: &HeaderMap) -> bool {
        headers.get("authorization").and_then(|value| value.to_str().ok()) == Some("Bearer ghp_test_token")
    }

    let router = Router::new()
        .route("/repos/acme/web", get(|| async { Json(json!({ "default_branch": "trunk" })) }))
        .route(
            "/repos/acme/web/pulls",
            get(|headers: HeaderMap| async move {
                if !authorized(&headers).await {
                    return (StatusCode::UNAUTHORIZED, Json(json!({ "message": "Bad credentials" })));
                }

                (
                    StatusCode::OK,
                    Json(json!([
                        { "number": 5, "title": "WEB-1 Sync me", "state": "open", "draft": false, "user": { "login": "octocat" },
                          "head": { "ref": "feature/sync", "sha": "s5" }, "base": { "ref": "trunk" }, "html_url": "https://github.com/acme/web/pull/5",
                          "updated_at": "2026-11-12T08:00:00Z", "created_at": "2026-11-11T08:00:00Z" },
                        { "number": 4, "title": "Old work", "state": "closed", "merged_at": "2026-11-01T08:00:00Z", "user": { "login": "octocat" },
                          "head": { "ref": "web-2-old", "sha": "s4" }, "base": { "ref": "trunk" }, "html_url": "https://github.com/acme/web/pull/4",
                          "updated_at": "2026-11-01T08:00:00Z" }
                    ])),
                )
            }),
        )
        .route(
            "/repos/acme/web/pulls/{number}/reviews",
            get(|| async { Json(json!([{ "user": { "login": "a" }, "state": "CHANGES_REQUESTED" }, { "user": { "login": "a" }, "state": "APPROVED" }])) }),
        )
        .route(
            "/repos/acme/web/commits/{sha}/check-runs",
            get(|| async { Json(json!({ "check_runs": [{ "status": "completed", "conclusion": "success" }, { "status": "in_progress", "conclusion": null }] })) }),
        )
        .route("/repos/acme/web/branches", get(|| async { Json(json!([{ "name": "trunk", "commit": { "sha": "t" } }, { "name": "feature/WEB-1-sync", "commit": { "sha": "s5" } }])) }))
        .route(
            "/repos/acme/web/commits",
            get(|| async {
                Json(json!([{ "sha": "c1", "html_url": "https://github.com/acme/web/commit/c1", "author": { "login": "octocat" },
                              "commit": { "message": "WEB-1 first #done", "author": { "name": "Mona", "email": "m@example.com", "date": "2026-11-12T07:00:00Z" } } }]))
            }),
        )
        .route("/repos/acme/web/git/ref/heads/{branch}", get(|Path(branch): Path<String>| async move { Json(json!({ "ref": branch, "object": { "sha": "tip" } })) }))
        .route("/repos/acme/web/git/refs", post(|Json(body): Json<Value>| async move { (StatusCode::CREATED, Json(body)) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();

    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    format!("http://{address}")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_api_backfills_what_webhooks_missed_and_creates_branches() {
    let mut hub = Hub::new().await;
    hub.issue("Sync me").await;
    hub.issue("Old work").await;
    let api = fake_github().await;
    let repo = hub.client.get("/api/m/github/projects/WEB").await["repos"][0]["id"].as_i64().unwrap();

    assert_eq!(
        hub.client.refused(Method::POST, &format!("/api/m/github/repos/{repo}/sync"), Some(json!({}))).await,
        StatusCode::CONFLICT,
        "no token yet"
    );
    assert_eq!(
        hub.client.refused(Method::PUT, "/api/m/github/settings", Some(json!({ "apiUrl": "http://evil.example.com" }))).await,
        StatusCode::BAD_REQUEST,
        "the token only ever goes to https or to this machine"
    );

    hub.client.put("/api/m/github/settings", json!({ "apiUrl": api, "token": "wrong" })).await;
    assert_eq!(hub.client.refused(Method::POST, &format!("/api/m/github/repos/{repo}/sync"), Some(json!({}))).await, StatusCode::BAD_GATEWAY);
    assert_eq!(hub.client.get("/api/m/github/projects/WEB").await["repos"][0]["syncError"], "GitHub rejected the token");

    let settings = hub.client.put("/api/m/github/settings", json!({ "token": "ghp_test_token" })).await;
    assert_eq!(settings["tokenHint"], "…oken");

    let report = hub.client.post(&format!("/api/m/github/repos/{repo}/sync"), json!({})).await;
    assert_eq!(report, json!({ "pulls": 2, "branches": 2, "commits": 1, "linkedIssues": 2 }));

    let code = hub.client.get("/api/m/github/projects/WEB").await;
    assert_eq!(code["repos"][0]["defaultBranch"], "trunk");
    assert!(code["repos"][0]["syncError"].is_null());

    let first = hub.client.get("/api/issues/WEB-1").await;
    let pr = &first["panels"]["github"]["prs"][0];

    assert_eq!((pr["review"].as_str(), pr["checks"].as_str()), (Some("approved"), Some("running")), "the latest review of each person counts");
    assert_eq!(first["panels"]["github"]["branches"][0]["name"], "feature/WEB-1-sync");
    assert_eq!(hub.status("WEB-1").await, "To Do", "a backfill links; it does not replay commands or rules");
    assert_eq!(hub.client.get("/api/issues/WEB-2").await["ext"]["github"]["prs"][0]["state"], "merged");

    let panel = hub.client.post("/api/m/github/issues/WEB-2/branch", json!({ "repoId": repo })).await;
    assert_eq!(panel["branches"][0]["name"], "feature/WEB-2-old-work");
    assert_eq!(hub.status("WEB-2").await, "In Progress");
    assert_eq!(
        hub.client.refused(Method::POST, "/api/m/github/issues/WEB-2/branch", Some(json!({ "repoId": repo, "name": "bad name" }))).await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demo_shows_code_next_to_issues() {
    let mut client = Client::new(build_router(test_app(modules())));
    client.setup(true).await;

    let code = client.get("/api/m/github/projects/WEB").await;
    let open: Vec<i64> =
        code["pulls"].as_array().unwrap().iter().filter(|pull| pull["state"] == "open").map(|pull| pull["number"].as_i64().unwrap()).collect();

    assert_eq!(code["repos"][0]["fullName"], "acme/sklep-web");
    assert_eq!(open.len(), 5);
    assert_eq!(client.search_keys("project = WEB AND drift = changes_requested").await, ["WEB-9"]);
    assert_eq!(client.search_keys("project = WEB AND drift = checks_failing").await, ["WEB-15"]);
    assert_eq!(client.search_keys("project = WEB AND drift = no_code_yet").await, ["WEB-17"]);
    assert_eq!(client.search_keys("project = WEB AND drift = todo_with_code").await, ["WEB-13"]);

    let dashboard = client.get("/api/dashboard").await;
    assert_eq!(dashboard["ext"]["github"]["pulls"][0]["number"], 57);
    assert_eq!(client.get("/api/issues/WEB-10").await["panels"]["github"]["branches"][0]["name"], "feature/WEB-10-platnosc-blik");
}
