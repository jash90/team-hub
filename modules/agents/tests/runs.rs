//! Agent runs through the module's router: people starting and deciding, runners carrying
//! out, and the server as a runner against a model endpoint faked on this machine.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use hub_app::access::project_access;
use hub_app::services::issues::{create_issue, issue_access, issue_detail, update_issue};
use hub_app::services::overview::dashboard;
use hub_app::services::projects::{create_project, set_member, NewProject};
use hub_app::services::search::search;
use hub_app::services::sprints::{create_sprint, SprintInput};
use hub_app::services::users::{create_token, create_user, update_user, NewUser, UserPatch};
use hub_app::{Actor, App, AuthUser, Config, Module};
use hub_core::{IssuePatch, NewIssue, ProjectRole, Role};
use hub_mod_agents::model::RETRY_BASE_MS;
use hub_mod_agents::{runs, worker, AgentsModule};
use serde_json::{json, Value};
use tokio::sync::Notify;
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

const OLD_DESCRIPTION: &str = "Tapping Sign in does nothing.\nSeen on iOS 18.";
const NEW_DESCRIPTION: &str = "## Goal\nSigning in works on Safari.\n\nTapping Sign in does nothing.\nSeen on iOS 18.\n\n## Acceptance criteria\n- [ ] Sign in works on Safari 18";
const SUMMARY: &str = "The issue asks for a working sign-in on Safari. Nothing is blocking it.";

// ---------- a model endpoint on this machine ----------

#[derive(Default)]
struct FakeModel {
    /// Every request received: the headers that matter and the body.
    requests: Mutex<Vec<Value>>,
    overloads: AtomicUsize,
    arrived: Notify,
    release: Notify,
}

fn text(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({ "type": "tool_use", "id": id, "name": name, "input": input })
}

/// What a model would say, decided from the tools it was offered and from markers in the
/// requester's instructions.
fn script(body: &Value) -> (Vec<Value>, &'static str) {
    let messages = body["messages"].as_array().expect("messages");
    let first = messages[0]["content"].as_str().unwrap_or("");
    let last = &messages[messages.len() - 1];
    let tools: Vec<&str> = body["tools"].as_array().map(|tools| tools.iter().filter_map(|tool| tool["name"].as_str()).collect()).unwrap_or_default();
    let results: Vec<&Value> =
        last["content"].as_array().map(|blocks| blocks.iter().filter(|block| block["type"] == "tool_result").collect()).unwrap_or_default();
    let rewrite = || {
        (
            vec![text("Rewriting the description."), tool_use("toolu_1", "propose_update_fields", json!({ "description": NEW_DESCRIPTION }))],
            "tool_use",
        )
    };

    if results.iter().any(|result| result["is_error"] == true) {
        return (vec![tool_use("toolu_fixed", "propose_transition", json!({ "status": "in review" }))], "tool_use");
    }

    if !results.is_empty() {
        return (vec![text("Done. My proposals are above.")], "end_turn");
    }

    if last["content"].as_str().is_some_and(|text| text.starts_with("You have not made a proposal yet")) {
        return rewrite();
    }

    if first.contains("[refuse]") {
        return (Vec::new(), "refusal");
    }

    if first.contains("[no-tool]") {
        return (vec![text("The description could be clearer.")], "end_turn");
    }

    if first.contains("[bad-status]") {
        return (vec![tool_use("toolu_bad", "propose_transition", json!({ "status": "Shipped" }))], "tool_use");
    }

    match tools.as_slice() {
        ["propose_update_fields"] => rewrite(),
        ["propose_subtasks"] => {
            let items = json!([{ "summary": "Reproduce on Safari 18" }, { "summary": "Fix the tap handler", "description": "The handler never fires." }, { "summary": "Add a regression test" }]);

            (vec![tool_use("toolu_1", "propose_subtasks", json!({ "items": items }))], "tool_use")
        }
        [] => (vec![text(SUMMARY)], "end_turn"),
        ["propose_update_fields", "propose_comment"] => (
            vec![
                tool_use("toolu_1", "propose_update_fields", json!({ "priority": "High", "labels": ["auth", "safari"] })),
                tool_use("toolu_2", "propose_comment", json!({ "body": "Which Safari versions are affected?" })),
            ],
            "tool_use",
        ),
        _ => (vec![tool_use("toolu_1", "propose_comment", json!({ "body": "I looked into it." }))], "tool_use"),
    }
}

async fn messages(State(fake): State<Arc<FakeModel>>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).map(String::from);
    let first = body["messages"][0]["content"].as_str().unwrap_or("").to_string();

    fake.requests
        .lock()
        .unwrap()
        .push(json!({ "key": header("x-api-key"), "version": header("anthropic-version"), "beta": header("anthropic-beta"), "body": body }));

    if first.contains("[overloaded]") && fake.overloads.fetch_add(1, Ordering::SeqCst) < 2 {
        let error = json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } });

        return (StatusCode::from_u16(529).unwrap(), Json(error)).into_response();
    }

    if first.contains("[slow]") {
        fake.arrived.notify_one();
        fake.release.notified().await;
    }

    let (content, stop_reason) = script(&body);

    Json(json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "model": body["model"],
        "content": content,
        "stop_reason": stop_reason,
        "usage": { "input_tokens": 1000, "output_tokens": 200 },
    }))
    .into_response()
}

async fn fake_model() -> (Arc<FakeModel>, String) {
    let fake = Arc::new(FakeModel::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    let router = Router::new().route("/v1/messages", post(messages)).with_state(fake.clone());

    tokio::spawn(async move { axum::serve(listener, router).await.expect("the fake model serves") });

    (fake, address)
}

// ---------- the instance under test ----------

struct World {
    app: App,
    router: Router,
    admin: AuthUser,
    token: String,
}

fn person(app: &App, name: &str, email: &str, role: Role) -> (AuthUser, String) {
    let new = NewUser { name: name.to_string(), email: email.to_string(), password: None, role: Some(role), github_login: None };
    let (user, token) = app
        .db
        .write(|tx| {
            let user = create_user(tx, &new)?;
            let token = create_token(tx, user.id, "agent")?;

            Ok((user, token))
        })
        .expect("a user with a token");

    (AuthUser { id: user.id, name: user.name, email: user.email, role, via_token: true, through: Vec::new() }, token)
}

fn issue(world: &World, project: &str, summary: &str, description: &str) {
    let fields = IssuePatch { summary: Some(summary.to_string()), description: Some(description.to_string()), ..Default::default() };

    world.app.write(world.admin.actor(), |tx| create_issue(tx, &NewIssue { project_key: project.to_string(), fields })).expect("an issue");
}

fn project(world: &World, key: &str, is_private: bool) {
    let new =
        NewProject { key: key.to_string(), name: format!("Project {key}"), description: String::new(), board_mode: None, is_private, lead_id: None };

    world.app.write(world.admin.actor(), |tx| create_project(tx, &world.admin, &new)).expect("a project");
}

/// An instance with an administrator and a project WEB holding WEB-1 (a task with a
/// two-line description) and WEB-2.
fn world() -> World {
    let directory = std::env::temp_dir().join(format!("team-hub-runs-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&directory);
    let app = App::new(Config::for_tests(directory), vec![Arc::new(AgentsModule)]).expect("the test application starts");
    let (admin, token) = person(&app, "Bartek Admin", "admin@example.com", Role::Admin);
    let router = Router::new().nest("/api/m/agents", AgentsModule.routes().unwrap()).with_state(app.clone());
    let world = World { app, router, admin, token };

    RETRY_BASE_MS.store(1, Ordering::Relaxed);
    project(&world, "WEB", false);
    issue(&world, "WEB", "Login fails on Safari", OLD_DESCRIPTION);
    issue(&world, "WEB", "Checkout page", "");

    world
}

impl World {
    async fn send(&self, token: &str, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/m/agents{path}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
            .expect("a valid request");
        let response = self.router.clone().oneshot(request).await.expect("the router answers");
        let status = response.status();
        let bytes = response.into_body().collect().await.expect("a readable body").to_bytes();

        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// A request as the administrator that is expected to succeed.
    async fn ok(&self, method: Method, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.send(&self.token, method.clone(), path, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");

        value
    }

    async fn get(&self, path: &str) -> Value {
        self.ok(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        self.ok(Method::POST, path, Some(body)).await
    }

    /// Points the server at the fake model.
    async fn configure(&self, address: &str) {
        self.ok(Method::PUT, "/settings", Some(json!({ "apiKey": "sk-test-key-1234", "apiUrl": address }))).await;
    }

    async fn start(&self, key: &str, kind: &str, instructions: &str) -> i64 {
        let run = self.post(&format!("/issues/{key}/runs"), json!({ "kind": kind, "instructions": instructions })).await;
        assert_eq!(run["state"], "queued");

        run["id"].as_i64().expect("a run id")
    }

    /// Lets the server's worker carry out one queued run; false when it took none.
    async fn work(&self) -> bool {
        let app = self.app.clone();

        tokio::task::spawn_blocking(move || worker::run_next(&app).expect("the worker")).await.expect("the worker finishes")
    }

    async fn run(&self, id: i64) -> Value {
        self.get(&format!("/runs/{id}")).await
    }

    async fn decide(&self, proposal: &Value, body: Value) -> Value {
        self.post(&format!("/proposals/{}/decide", proposal["id"]), body).await
    }

    fn text(&self, sql: &str) -> Option<String> {
        self.app.read(|connection| Ok(connection.query_row(sql, [], |row| row.get(0))?)).expect("a value")
    }

    fn count(&self, sql: &str) -> i64 {
        self.app.read(|connection| Ok(connection.query_row(sql, [], |row| row.get(0))?)).expect("a count")
    }
}

fn kinds(events: &Value) -> Vec<&str> {
    events.as_array().expect("an array").iter().filter_map(|event| event["kind"].as_str()).collect()
}

fn texts(events: &Value) -> Vec<&str> {
    events.as_array().expect("an array").iter().filter_map(|event| event["text"].as_str()).collect()
}

// ---------- the server as a runner ----------

#[tokio::test(flavor = "multi_thread")]
async fn refine_proposes_a_new_description_that_is_applied_as_the_approver_and_labelled_as_the_agents_work() {
    let world = world();
    let (fake, address) = fake_model().await;
    world.configure(&address).await;

    let id = world.start("web-1", "refine", "Keep it short").await;
    assert!(world.work().await);
    let detail = world.run(id).await;
    let proposal = &detail["proposals"][0];
    let diff: Vec<(&str, &str)> = proposal["preview"]["descriptionDiff"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| (line["kind"].as_str().unwrap(), line["text"].as_str().unwrap()))
        .collect();

    assert_eq!(
        (&detail["run"]["state"], &detail["run"]["runnerId"], &detail["run"]["runnerName"]),
        (&json!("waiting"), &json!("server"), &json!("Server"))
    );
    assert_eq!((&detail["run"]["model"], &detail["run"]["pendingProposals"]), (&json!("claude-sonnet-5-5"), &json!(1)));
    assert_eq!(
        (&detail["run"]["inputTokens"], &detail["run"]["outputTokens"], &detail["run"]["costUsd"]),
        (&json!(2000), &json!(400), &json!(0.008))
    );
    assert_eq!(detail["run"]["summary"], "Done. My proposals are above.");
    assert_eq!(kinds(&detail["events"]), ["status", "status", "status", "message", "tool", "status", "message"]);
    assert_eq!(texts(&detail["events"])[..3], ["Taken by Server", "Reading the issue", "Asking Claude Sonnet 5.5"]);
    assert_eq!(
        (&proposal["kind"], &proposal["state"], &proposal["payload"]),
        (&json!("update_fields"), &json!("pending"), &json!({ "description": NEW_DESCRIPTION }))
    );
    assert_eq!(diff[..4], [("added", "## Goal"), ("added", "Signing in works on Safari."), ("added", ""), ("same", "Tapping Sign in does nothing.")]);

    let requests = fake.requests.lock().unwrap().clone();
    let prompt = requests[0]["body"]["messages"][0]["content"].as_str().unwrap();

    assert_eq!(requests.len(), 2, "one answer with the proposal, one to wrap up");
    assert_eq!((&requests[0]["key"], &requests[0]["version"]), (&json!("sk-test-key-1234"), &json!("2026-10-21")));
    assert_eq!(requests[0]["body"]["model"], "claude-sonnet-5-5");
    assert!(prompt.contains("<requester_instructions>\nKeep it short\n</requester_instructions>"));
    assert!(prompt.contains("Login fails on Safari") && prompt.contains("Seen on iOS 18.") && prompt.contains("\"statuses\""));
    assert!(requests[0]["body"]["system"].as_str().unwrap().contains("not addressed to you"));
    assert_eq!(requests[1]["body"]["messages"][2]["content"][0]["type"], "tool_result");

    let approved = world.decide(proposal, json!({ "approve": true })).await;

    assert_eq!((&approved["run"]["state"], &approved["run"]["pendingProposals"]), (&json!("succeeded"), &json!(0)));
    assert_eq!((&approved["proposals"][0]["state"], &approved["proposals"][0]["decidedBy"]), (&json!("approved"), &json!(world.admin.id)));
    assert_eq!(approved["proposals"][0]["result"], json!({ "changed": ["description"], "previousDescription": OLD_DESCRIPTION }));
    assert!(approved["proposals"][0]["preview"]["descriptionDiff"].is_array(), "the picture it was decided on is kept");
    assert_eq!(world.text("SELECT description FROM issue WHERE key = 'WEB-1'").as_deref(), Some(NEW_DESCRIPTION));
    assert_eq!(world.text("SELECT actor_label FROM change WHERE field = 'description'").as_deref(), Some("AI agent"));
    assert_eq!(world.count(&format!("SELECT count(*) FROM change WHERE field = 'description' AND actor_id = {}", world.admin.id)), 1);
    assert!(world.count("SELECT count(*) FROM event WHERE kind = 'agent.run' AND issue_id IS NOT NULL AND project_id IS NOT NULL") > 6);
    assert_eq!(world.count("SELECT count(*) FROM notification WHERE kind = 'agent' AND text LIKE 'WEB-1: the agent proposes%'"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_decided_proposal_cannot_be_decided_again() {
    let world = world();
    let (_fake, address) = fake_model().await;
    world.configure(&address).await;
    let id = world.start("WEB-1", "refine", "").await;
    world.work().await;
    let proposal = world.run(id).await["proposals"][0].clone();
    world.decide(&proposal, json!({ "approve": false, "note": "Too long" })).await;

    let again = world.send(&world.token, Method::POST, &format!("/proposals/{}/decide", proposal["id"]), Some(json!({ "approve": true }))).await;
    let detail = world.run(id).await;

    assert_eq!(again.0, StatusCode::CONFLICT);
    assert_eq!((&detail["proposals"][0]["state"], &detail["proposals"][0]["note"]), (&json!("rejected"), &json!("Too long")));
    assert_eq!(detail["run"]["state"], "succeeded", "nothing is left to decide");
    assert_eq!(world.text("SELECT description FROM issue WHERE key = 'WEB-1'").as_deref(), Some(OLD_DESCRIPTION));
}

#[tokio::test(flavor = "multi_thread")]
async fn breakdown_proposes_subtasks_that_the_person_may_edit_before_they_are_created() {
    let world = world();
    let (_fake, address) = fake_model().await;
    world.configure(&address).await;

    world
        .app
        .write(world.admin.actor(), |tx| {
            let sprint = create_sprint(tx, &project_access(tx, &world.admin, "WEB")?, &SprintInput::default())?;
            let (issue_id, _) = issue_access(tx, &world.admin, "WEB-1")?;

            update_issue(tx, issue_id, &IssuePatch { sprint_id: Some(Some(sprint.id)), ..Default::default() })
        })
        .expect("WEB-1 in a sprint");

    let id = world.start("WEB-1", "breakdown", "").await;
    assert!(world.work().await);
    let detail = world.run(id).await;
    let proposal = &detail["proposals"][0];
    let edited = json!({ "items": [{ "summary": "Reproduce on Safari 18 and 17" }, { "summary": "Fix the tap handler" }] });

    assert_eq!((&proposal["kind"], proposal["payload"]["items"].as_array().map(Vec::len)), (&json!("create_subtasks"), Some(3)));
    assert_eq!(proposal["preview"]["issueType"], "Subtask");

    let approved = world.decide(proposal, json!({ "approve": true, "payload": edited })).await;

    assert_eq!(approved["proposals"][0]["result"], json!({ "keys": ["WEB-3", "WEB-4"] }));
    assert_eq!(approved["proposals"][0]["payload"], edited);
    assert_eq!(approved["run"]["state"], "succeeded");
    assert_eq!(world.text("SELECT summary FROM issue WHERE key = 'WEB-3'").as_deref(), Some("Reproduce on Safari 18 and 17"));
    assert_eq!(
        world.count("SELECT count(*) FROM issue c JOIN issue p ON p.id = c.parent_id WHERE p.key = 'WEB-1' AND c.sprint_id = p.sprint_id"),
        2,
        "in their parent's sprint"
    );
    assert_eq!(world.count("SELECT count(*) FROM event WHERE kind = 'issue.created' AND actor_label = 'AI agent'"), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn summarize_ends_with_a_summary_and_no_proposal() {
    let world = world();
    let (fake, address) = fake_model().await;
    world.configure(&address).await;
    world.ok(Method::PUT, "/settings", Some(json!({ "defaultModel": "claude-haiku-5-5" }))).await;

    let id = world.start("WEB-1", "summarize", "").await;
    assert!(world.work().await);
    let detail = world.run(id).await;
    let requests = fake.requests.lock().unwrap().clone();

    assert_eq!((&detail["run"]["state"], &detail["run"]["summary"], &detail["proposals"]), (&json!("succeeded"), &json!(SUMMARY), &json!([])));
    assert_eq!((&detail["run"]["model"], &detail["run"]["costUsd"]), (&json!("claude-haiku-5-5"), &json!(0.0002)));
    assert!(detail["run"]["finishedAt"].is_string());
    assert_eq!(texts(&detail["events"]).last(), Some(&SUMMARY));
    assert!(requests[0]["body"].get("tools").is_none() && requests[0]["body"].get("fallbacks").is_none() && requests[0]["beta"].is_null());
    assert_eq!(world.count("SELECT count(*) FROM notification WHERE kind = 'agent' AND text LIKE 'WEB-1: the agent finished: The issue asks%'"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn triage_proposes_fields_and_a_comment_and_each_is_decided_on_its_own() {
    let world = world();
    let (_fake, address) = fake_model().await;
    world.configure(&address).await;

    let id = world.start("WEB-1", "triage", "").await;
    assert!(world.work().await);
    let detail = world.run(id).await;
    let (fields, comment) = (&detail["proposals"][0], &detail["proposals"][1]);

    assert_eq!((&fields["kind"], &comment["kind"], &detail["run"]["pendingProposals"]), (&json!("update_fields"), &json!("comment"), &json!(2)));
    assert_eq!(
        fields["preview"]["changes"],
        json!([{ "field": "priority", "before": "Medium", "after": "High" }, { "field": "labels", "before": null, "after": "auth, safari" }])
    );

    let rejected = world.decide(comment, json!({ "approve": false, "note": "We know the versions" })).await;
    assert_eq!((&rejected["run"]["state"], &rejected["run"]["pendingProposals"]), (&json!("waiting"), &json!(1)));

    let approved = world.decide(fields, json!({ "approve": true })).await;

    assert_eq!(approved["run"]["state"], "succeeded");
    assert_eq!((&approved["proposals"][1]["state"], &approved["proposals"][1]["note"]), (&json!("rejected"), &json!("We know the versions")));
    assert_eq!(world.count("SELECT priority FROM issue WHERE key = 'WEB-1'"), 2);
    assert_eq!(world.count("SELECT count(*) FROM issue_label l JOIN issue i ON i.id = l.issue_id WHERE i.key = 'WEB-1'"), 2);
    assert_eq!(world.count("SELECT count(*) FROM comment"), 0);
    assert_eq!(world.count("SELECT count(*) FROM change WHERE actor_label = 'AI agent' AND field IN ('priority', 'labels')"), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_the_tracker_refuses_goes_back_to_the_model_which_corrects_it() {
    let world = world();
    let (fake, address) = fake_model().await;
    world.configure(&address).await;

    let id = world.start("WEB-1", "custom", "[bad-status] move it along").await;
    assert!(world.work().await);
    let detail = world.run(id).await;
    let requests = fake.requests.lock().unwrap().clone();
    let refusal = &requests[1]["body"]["messages"][2]["content"][0];

    assert_eq!(
        (&refusal["is_error"], refusal["content"].as_str()),
        (&json!(true), Some("Unknown status \"Shipped\" (valid: To Do, In Progress, In Review, Done)"))
    );
    assert_eq!(detail["proposals"].as_array().map(Vec::len), Some(1));
    assert_eq!(detail["proposals"][0]["payload"], json!({ "status": "In Review" }), "spelled the way the project spells it");
    assert_eq!(
        detail["proposals"][0]["preview"],
        json!({ "changes": [{ "field": "status", "before": "To Do", "after": "In Review" }], "descriptionDiff": null, "allowed": true, "issueType": null })
    );
    assert!(texts(&detail["events"]).contains(&"A proposal was refused: Unknown status \"Shipped\" (valid: To Do, In Progress, In Review, Done)"));

    world.decide(&detail["proposals"][0], json!({ "approve": true })).await;
    assert_eq!(world.text("SELECT s.name FROM issue i JOIN status s ON s.id = i.status_id WHERE i.key = 'WEB-1'").as_deref(), Some("In Review"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_that_forgets_to_propose_is_asked_once_more_and_overload_is_retried() {
    let world = world();
    let (fake, address) = fake_model().await;
    world.configure(&address).await;

    let forgetful = world.start("WEB-1", "refine", "[no-tool]").await;
    assert!(world.work().await);
    let overloaded = world.start("WEB-2", "summarize", "[overloaded]").await;
    assert!(world.work().await);
    let requests = fake.requests.lock().unwrap().clone();

    assert_eq!(world.run(forgetful).await["proposals"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        requests[1]["body"]["messages"][2]["content"],
        "You have not made a proposal yet. Call propose_update_fields now with what you suggest."
    );
    assert_eq!(world.run(overloaded).await["run"]["state"], "succeeded");
    assert_eq!(requests.len(), 3 + 3, "three for the first run; two refusals and an answer for the second");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_fails_with_the_reason_when_the_model_declines_or_cannot_be_reached() {
    let world = world();
    let (_fake, address) = fake_model().await;
    world.configure(&address).await;

    let declined = world.start("WEB-1", "refine", "[refuse]").await;
    assert!(world.work().await);
    world.ok(Method::PUT, "/settings", Some(json!({ "apiUrl": "http://127.0.0.1:9" }))).await;
    let unreachable = world.start("WEB-1", "summarize", "").await;
    assert!(world.work().await);
    let (declined, unreachable) = (world.run(declined).await, world.run(unreachable).await);

    assert_eq!(
        (&declined["run"]["state"], texts(&declined["events"]).last()),
        (&json!("failed"), Some(&"The model declined to work on this request"))
    );
    assert_eq!(kinds(&declined["events"]).last(), Some(&"error"));
    assert_eq!(unreachable["run"]["state"], "failed");
    assert!(texts(&unreachable["events"]).last().is_some_and(|text| text.starts_with("The model provider kept failing")));
    assert_eq!(world.count("SELECT count(*) FROM notification WHERE text LIKE '%the agent run failed'"), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_key_or_past_the_monthly_cap_the_server_leaves_runs_in_the_queue() {
    let world = world();
    let (_fake, address) = fake_model().await;
    let first = world.start("WEB-1", "summarize", "").await;

    assert!(!world.work().await, "no key yet");
    assert_eq!(world.get("/settings").await["serverStatus"], "no_key");
    assert_eq!(world.run(first).await["run"]["state"], "queued");

    world.configure(&address).await;
    world.ok(Method::PUT, "/settings", Some(json!({ "monthlyCapUsd": 0.004 }))).await;
    assert!(world.work().await);
    let second = world.start("WEB-2", "summarize", "").await;
    let settings = world.get("/settings").await;

    assert!(!world.work().await, "the first run cost 0.004 dollars, which is the cap");
    assert_eq!(
        (&settings["serverStatus"], &settings["spentThisMonthUsd"], &settings["monthlyCapUsd"]),
        (&json!("cap_reached"), &json!(0.004), &json!(0.004))
    );
    assert_eq!(world.run(second).await["run"]["state"], "queued");

    world.ok(Method::PUT, "/settings", Some(json!({ "monthlyCapUsd": null, "serverKinds": ["refine"] }))).await;
    assert!(!world.work().await, "summaries are no longer the server's to do");
    world.ok(Method::PUT, "/settings", Some(json!({ "serverKinds": ["refine", "summarize", "implement"] }))).await;

    assert_eq!(world.get("/settings").await["serverKinds"], json!(["refine", "summarize"]), "the server never takes work that needs a repository");
    assert!(world.work().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_for_administrators_and_never_give_the_key_back() {
    let world = world();
    let (_, member) = person(&world.app, "Maria Member", "maria@example.com", Role::Member);
    world.configure("http://localhost:4010").await;
    let as_admin = world.get("/settings").await;
    let as_member = world.send(&member, Method::GET, "/settings", None).await.1;

    assert_eq!(
        (&as_admin["keyConfigured"], &as_admin["keyHint"], &as_admin["apiUrl"]),
        (&json!(true), &json!("…1234"), &json!("http://localhost:4010"))
    );
    assert_eq!((&as_member["keyConfigured"], &as_member["keyHint"]), (&json!(true), &Value::Null));
    assert_eq!(as_admin["defaultModel"], "claude-sonnet-5-5");
    assert_eq!(as_admin["models"].as_array().map(Vec::len), Some(4));
    assert!(!as_admin.to_string().contains("sk-test-key"));
    assert_eq!(world.send(&member, Method::PUT, "/settings", Some(json!({ "apiKey": "x" }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(world.send(&world.token, Method::PUT, "/settings", Some(json!({ "apiUrl": "http://example.com" }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(world.send(&world.token, Method::PUT, "/settings", Some(json!({ "defaultModel": "gpt-4" }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(world.ok(Method::PUT, "/settings", Some(json!({ "clearApiKey": true }))).await["keyConfigured"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_run_the_server_is_working_on_stops_it_before_anything_is_proposed() {
    let world = world();
    let (fake, address) = fake_model().await;
    world.configure(&address).await;

    let waiting_in_queue = world.start("WEB-2", "implement", "").await;
    assert_eq!(world.post(&format!("/runs/{waiting_in_queue}/cancel"), json!({})).await["state"], "cancelled", "nobody had taken it");

    let id = world.start("WEB-1", "refine", "[slow]").await;
    let app = world.app.clone();
    let worker = tokio::task::spawn_blocking(move || worker::run_next(&app).expect("the worker"));
    fake.arrived.notified().await;
    let asked = world.post(&format!("/runs/{id}/cancel"), json!({})).await;

    assert_eq!((&asked["state"], &asked["cancelRequested"]), (&json!("running"), &json!(true)));

    fake.release.notify_one();
    assert!(worker.await.expect("the worker finishes"));
    let detail = world.run(id).await;

    assert_eq!((&detail["run"]["state"], &detail["proposals"]), (&json!("cancelled"), &json!([])));
    assert_eq!(texts(&detail["events"]).last(), Some(&"Cancelled"));
    assert_eq!(detail["run"]["inputTokens"], 1000, "what was used is still accounted for");
}

// ---------- runners elsewhere ----------

async fn claim(world: &World, token: &str, runner: &str, kinds: Value) -> Value {
    let (status, value) = world.send(token, Method::POST, "/runs/claim", Some(json!({ "runnerId": runner, "kinds": kinds }))).await;
    assert_eq!(status, StatusCode::OK, "{value}");

    value
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_goes_to_its_requesters_runner_and_a_shared_one_to_exactly_one_of_two_competing_runners() {
    let world = world();
    let (other, other_token) = person(&world.app, "Piotr Runner", "piotr@example.com", Role::Member);
    let beat = |runner: &str, name: &str| json!({ "runnerId": runner, "name": name, "kinds": ["implement", "review"], "projects": ["WEB"], "version": "1.2.0" });

    assert_eq!(world.post("/runners/heartbeat", beat("desk-1", "Desktop · Bartek's MacBook")).await, json!({ "ok": true }));
    assert_eq!(
        world.send(&other_token, Method::POST, "/runners/heartbeat", Some(beat("desk-2", "Desktop · Piotr's ThinkPad"))).await.0,
        StatusCode::OK
    );
    assert_eq!(world.send(&other_token, Method::POST, "/runners/heartbeat", Some(beat("desk-1", "Stolen"))).await.0, StatusCode::CONFLICT);

    let runners = world.get("/runners").await;
    assert_eq!(runners.as_array().map(Vec::len), Some(2), "one's own, and others that are online");
    let own = runners.as_array().unwrap().iter().find(|runner| runner["id"] == "desk-1").expect("the administrator's runner");
    assert_eq!(
        (&own["name"], &own["online"], &own["kinds"], &own["projects"]),
        (&json!("Desktop · Bartek's MacBook"), &json!(true), &json!(["implement", "review"]), &json!(["WEB"]))
    );

    let private = world.post("/issues/WEB-1/runs", json!({ "kind": "implement", "repo": "acme/shop" })).await;
    let private_id = private["id"].as_i64().unwrap();

    assert_eq!(claim(&world, &other_token, "desk-2", json!(["implement"])).await, Value::Null, "not Piotr's to take");
    assert_eq!(claim(&world, &world.token, "desk-1", json!(["review"])).await, Value::Null, "not a kind this runner does");

    let taken = claim(&world, &world.token, "desk-1", json!(["implement"])).await;
    assert_eq!(
        (&taken["run"]["id"], &taken["run"]["state"], &taken["run"]["runnerName"], &taken["run"]["repo"]),
        (&private["id"], &json!("claimed"), &json!("Desktop · Bartek's MacBook"), &json!("acme/shop"))
    );
    assert_eq!(claim(&world, &world.token, "desk-1", json!(["implement"])).await, Value::Null, "nothing else is queued");

    let shared = world.post("/issues/WEB-2/runs", json!({ "kind": "implement", "shared": true })).await;
    let (first, second) =
        tokio::join!(claim(&world, &world.token, "desk-1", json!(["implement"])), claim(&world, &other_token, "desk-2", json!(["implement"])));
    let winners: Vec<&Value> = [&first, &second].into_iter().filter(|answer| !answer.is_null()).collect();

    assert_eq!(winners.len(), 1, "two runners never get the same run");
    assert_eq!(winners[0]["run"]["id"], shared["id"]);

    // Only the runner that claimed a run may write to it.
    let events = format!("/runs/{private_id}/events");
    assert_eq!(world.send(&other_token, Method::POST, &events, Some(json!({ "kind": "message", "text": "Hello" }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        world.send(&other_token, Method::PATCH, &format!("/runs/{private_id}"), Some(json!({ "state": "succeeded" }))).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        world
            .send(
                &other_token,
                Method::POST,
                &format!("/runs/{private_id}/proposals"),
                Some(json!({ "kind": "comment", "payload": { "body": "x" } }))
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // What the person tells the agent in the runner's app is recorded as said by them.
    world.post(&events, json!({ "kind": "answer", "text": "Keep it small." })).await;
    let said = world.run(private_id).await["events"].as_array().unwrap().last().cloned().unwrap();
    assert_eq!((&said["kind"], &said["text"], &said["actorId"]), (&json!("answer"), &json!("Keep it small."), &json!(world.admin.id)));

    let event = world.post(&events, json!({ "kind": "tool", "text": "cargo test", "data": { "exitCode": 0 } })).await;
    assert!(event["id"].is_i64());
    assert_eq!(world.run(private_id).await["run"]["state"], "running");

    world.post(&events, json!({ "kind": "question", "text": "Should the fix cover Safari 17 too?" })).await;
    let asked = world.run(private_id).await;
    assert_eq!((&asked["run"]["state"], &asked["run"]["pendingQuestion"]), (&json!("waiting"), &json!(true)));

    // A pending question is answered with the call that lets the run go on, not with an event.
    assert_eq!(world.send(&world.token, Method::POST, &events, Some(json!({ "kind": "answer", "text": "Yes" }))).await.0, StatusCode::CONFLICT);

    let answered = world.post(&format!("/runs/{private_id}/answer"), json!({ "text": "Yes, both." })).await;
    let last = answered["events"].as_array().unwrap().last().unwrap();
    assert_eq!((&answered["run"]["state"], &answered["run"]["pendingQuestion"]), (&json!("running"), &json!(false)));
    assert_eq!((&last["kind"], &last["text"], &last["actorId"]), (&json!("answer"), &json!("Yes, both."), &json!(world.admin.id)));

    let refused = world
        .send(
            &world.token,
            Method::POST,
            &format!("/runs/{private_id}/proposals"),
            Some(json!({ "kind": "update_fields", "payload": { "assignee": "Nobody" } })),
        )
        .await;
    assert_eq!(
        (refused.0, refused.1["error"]["message"].as_str()),
        (StatusCode::BAD_REQUEST, Some("Unknown person \"Nobody\" (valid: Bartek Admin, Piotr Runner)"))
    );

    let proposal = world
        .post(&format!("/runs/{private_id}/proposals"), json!({ "kind": "comment", "payload": { "body": " The fix is in the pull request. " } }))
        .await;
    assert_eq!((&proposal["state"], &proposal["payload"]), (&json!("pending"), &json!({ "body": "The fix is in the pull request." })));
    assert_eq!(world.run(private_id).await["run"]["state"], "waiting");

    world.post(&format!("/runs/{private_id}/cancel"), json!({})).await;
    assert_eq!(
        world.send(&world.token, Method::PATCH, &format!("/runs/{private_id}"), Some(json!({ "pullRequestUrl": "javascript:alert(1)" }))).await.0,
        StatusCode::BAD_REQUEST
    );
    let patch = json!({ "state": "succeeded", "summary": "Fixed the handler.", "branch": "web-1-login", "pullRequestUrl": "https://github.com/acme/shop/pull/7", "inputTokens": 5000, "outputTokens": 900, "costUsd": 0.02 });
    let finished = world.ok(Method::PATCH, &format!("/runs/{private_id}"), Some(patch)).await;

    assert_eq!((&finished["state"], &finished["cancelRequested"], &finished["branch"]), (&json!("succeeded"), &json!(true), &json!("web-1-login")));
    assert_eq!(
        (&finished["pullRequestUrl"], &finished["costUsd"], &finished["pendingProposals"]),
        (&json!("https://github.com/acme/shop/pull/7"), &json!(0.02), &json!(1))
    );
    assert_eq!(world.send(&world.token, Method::POST, &events, Some(json!({ "kind": "message", "text": "More" }))).await.0, StatusCode::CONFLICT);

    // A proposal can still be decided after the run has ended.
    let decided = world.decide(&proposal, json!({ "approve": true })).await;
    assert_eq!((&decided["proposals"][0]["state"], &decided["run"]["state"]), (&json!("approved"), &json!("succeeded")));
    assert_eq!(world.text("SELECT body FROM comment").as_deref(), Some("The fix is in the pull request."));
    assert_eq!(world.count(&format!("SELECT count(*) FROM comment WHERE author_id = {} AND author_label = 'AI agent'", world.admin.id)), 1);
    assert_eq!(other.name, "Piotr Runner");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runner_keeps_the_run_it_started_itself_and_can_name_the_pull_request_after_the_end() {
    let world = world();
    let beat = |runner: &str| json!({ "runnerId": runner, "name": runner, "kinds": ["implement"], "version": "1" });
    let start = |runner: &str| json!({ "kind": "implement", "runnerId": runner });

    // A runner nobody has heard of cannot be given a run.
    assert_eq!(world.send(&world.token, Method::POST, "/issues/WEB-1/runs", Some(start("app"))).await.0, StatusCode::BAD_REQUEST);

    world.post("/runners/heartbeat", beat("app")).await;
    world.post("/runners/heartbeat", beat("cli")).await;

    let (_, other_token) = person(&world.app, "Piotr Runner", "piotr@example.com", Role::Member);
    assert_eq!(world.send(&other_token, Method::POST, "/issues/WEB-1/runs", Some(start("app"))).await.0, StatusCode::CONFLICT, "not Piotr's runner");

    // Born claimed: the same person's second runner finds nothing to take.
    let own = world.post("/issues/WEB-1/runs", start("app")).await;
    let id = own["id"].as_i64().expect("a run id");
    assert_eq!((&own["state"], &own["runnerId"]), (&json!("claimed"), &json!("app")));
    assert_eq!(claim(&world, &world.token, "cli", json!(["implement"])).await, Value::Null);

    // Claiming by id takes that run and no older one.
    let older = world.start("WEB-1", "implement", "").await;
    let newer = world.start("WEB-2", "implement", "").await;
    let by_id = |run: i64| json!({ "runnerId": "cli", "kinds": ["implement"], "runId": run });
    assert_eq!(world.post("/runs/claim", by_id(newer)).await["run"]["id"], newer);
    assert_eq!(world.post("/runs/claim", by_id(newer)).await, Value::Null, "already taken");
    assert_eq!(world.post("/runs/claim", by_id(id)).await, Value::Null, "held by the other runner");
    assert_eq!(world.run(older).await["run"]["state"], "queued");

    // After the end only the branch and the pull request can still be recorded.
    let path = format!("/runs/{id}");
    world.send(&world.token, Method::PATCH, &path, Some(json!({ "state": "succeeded", "summary": "Done on a branch." }))).await;
    assert_eq!(world.send(&world.token, Method::PATCH, &path, Some(json!({ "summary": "Rewritten" }))).await.0, StatusCode::CONFLICT);
    assert_eq!(world.send(&world.token, Method::PATCH, &path, Some(json!({ "state": "running" }))).await.0, StatusCode::CONFLICT);
    assert_eq!(
        world.send(&world.token, Method::POST, &format!("{path}/events"), Some(json!({ "kind": "message", "text": "Late" }))).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        world.send(&other_token, Method::PATCH, &path, Some(json!({ "branch": "stolen" }))).await.0,
        StatusCode::FORBIDDEN,
        "still only its own runner"
    );

    let (status, named) = world
        .send(&world.token, Method::PATCH, &path, Some(json!({ "branch": "feature/WEB-1", "pullRequestUrl": "https://github.com/acme/shop/pull/7" })))
        .await;
    assert_eq!(status, StatusCode::OK, "{named}");
    assert_eq!(
        (&named["state"], &named["summary"], &named["branch"], &named["pullRequestUrl"]),
        (&json!("succeeded"), &json!("Done on a branch."), &json!("feature/WEB-1"), &json!("https://github.com/acme/shop/pull/7"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_and_a_pull_request_are_shown_here_and_decided_in_the_runners_app() {
    let world = world();
    let (_, other_token) = person(&world.app, "Piotr Runner", "piotr@example.com", Role::Member);

    world.post("/runners/heartbeat", json!({ "runnerId": "app", "name": "Desktop", "kinds": ["implement"], "version": "1" })).await;
    let id = world.post("/issues/WEB-1/runs", json!({ "kind": "implement", "runnerId": "app" })).await["id"].as_i64().unwrap();
    let proposals = format!("/runs/{id}/proposals");

    // What leaves the computer cannot be trusted in advance, whatever the project is told.
    let trust = world.send(&world.token, Method::PUT, "/projects/WEB/trust", Some(json!({ "kinds": ["comment", "push", "pull_request"] }))).await.1;
    assert_eq!(trust["kinds"], json!(["comment"]));
    world.send(&world.token, Method::PUT, "/projects/WEB/trust", Some(json!({ "kinds": [] }))).await;

    for refused in [json!({ "kind": "push", "payload": { "branch": "two words" } }), json!({ "kind": "pull_request", "payload": { "title": " " } })] {
        assert_eq!(world.send(&world.token, Method::POST, &proposals, Some(refused.clone())).await.0, StatusCode::BAD_REQUEST, "{refused}");
    }

    let push = world.post(&proposals, json!({ "kind": "push", "payload": { "branch": "feature/WEB-1", "note": "One commit." } })).await;
    let pull = world.post(&proposals, json!({ "kind": "pull_request", "payload": { "title": "WEB-1 Login", "body": "Adds validation.", "head": "feature/WEB-1" } })).await;
    let (push_id, pull_id) = (push["id"].as_i64().unwrap(), pull["id"].as_i64().unwrap());

    assert_eq!((&push["state"], &push["payload"]), (&json!("pending"), &json!({ "branch": "feature/WEB-1", "note": "One commit." })));

    let waiting = world.run(id).await;
    assert_eq!((&waiting["run"]["state"], &waiting["run"]["pendingProposals"]), (&json!("waiting"), &json!(2)));

    // Nobody decides them on the web, not even the person whose run it is.
    for approve in [true, false] {
        let decision = Some(json!({ "approve": approve }));
        assert_eq!(world.send(&world.token, Method::POST, &format!("/proposals/{push_id}/decide"), decision).await.0, StatusCode::CONFLICT);
    }

    // Only the run's runner says how it went, and only for these kinds.
    let done = |commit: &str| json!({ "approve": true, "result": { "commit": commit } });
    assert_eq!(world.send(&other_token, Method::POST, &format!("/proposals/{push_id}/settle"), Some(done("abc"))).await.0, StatusCode::FORBIDDEN);

    let comment = world.post(&proposals, json!({ "kind": "comment", "payload": { "body": "Done." } })).await["id"].as_i64().unwrap();
    assert_eq!(world.send(&world.token, Method::POST, &format!("/proposals/{comment}/settle"), Some(done("abc"))).await.0, StatusCode::BAD_REQUEST);
    world.post(&format!("/proposals/{comment}/decide"), json!({ "approve": false })).await;

    let settled = world.post(&format!("/proposals/{push_id}/settle"), done("abc123")).await;
    let pushed = settled["proposals"].as_array().unwrap().iter().find(|proposal| proposal["id"] == push_id).cloned().unwrap();
    assert_eq!(
        (&pushed["state"], &pushed["result"], &pushed["decidedBy"]),
        (&json!("approved"), &json!({ "commit": "abc123" }), &json!(world.admin.id))
    );
    assert_eq!(settled["run"]["state"], "waiting", "the pull request is still undecided");
    assert_eq!(world.send(&world.token, Method::POST, &format!("/proposals/{push_id}/settle"), Some(done("again"))).await.0, StatusCode::CONFLICT);

    let declined = world.post(&format!("/proposals/{pull_id}/settle"), json!({ "approve": false, "note": "Not yet." })).await;
    let pull = declined["proposals"].as_array().unwrap().iter().find(|proposal| proposal["id"] == pull_id).cloned().unwrap();
    assert_eq!((&pull["state"], &pull["note"]), (&json!("rejected"), &json!("Not yet.")));
    assert_eq!((&declined["run"]["state"], &declined["run"]["pendingProposals"]), (&json!("running"), &json!(0)));
    assert_eq!(texts(&declined["events"]).last(), Some(&"Declined in the runner's app: opening a pull request"));

    // The server's own runs have no code to push.
    let own = world.start("WEB-2", "refine", "").await;
    let refused = world.app.write(Actor::system("test"), |tx| {
        Ok(hub_mod_agents::proposals::add(tx, &runs::load(tx, own)?, hub_mod_agents::types::AgentProposalKind::Push, &json!({ "branch": "x" })).map(|_| ()))
    });
    assert!(refused.unwrap().is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runner_with_nobody_at_it_has_its_pushes_decided_on_the_web_from_the_changes_it_uploads() {
    let world = world();
    let (_, other_token) = person(&world.app, "Piotr Runner", "piotr@example.com", Role::Member);
    let beat = json!({ "runnerId": "cloud", "name": "Cloud runner", "kinds": ["implement"], "version": "1", "interactive": false });

    world.post("/runners/heartbeat", beat).await;
    let runner = world.get("/runners").await.as_array().unwrap().iter().find(|runner| runner["id"] == "cloud").cloned().unwrap();
    assert_eq!(runner["interactive"], false);

    let run = world.post("/issues/WEB-1/runs", json!({ "kind": "implement", "runnerId": "cloud" })).await;
    let id = run["id"].as_i64().unwrap();
    assert_eq!((&run["runnerInteractive"], &run["hasChanges"]), (&json!(false), &json!(false)));

    // What the thread committed, for a person to look at before anything is pushed.
    let changes = json!({
        "base": "1111111", "head": "abc1234",
        "commits": [{ "sha": "abc1234", "subject": "WEB-1: validate the form", "author": "Agent" }],
        "files": [{ "path": "src/login.ts", "status": "modified", "additions": 12, "deletions": 3 }],
        "patch": "--- a/src/login.ts\n+++ b/src/login.ts\n@@ -1 +1 @@\n-old\n+new\n", "truncated": false,
    });
    let path = format!("/runs/{id}/changes");

    assert_eq!(world.send(&world.token, Method::GET, &path, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(world.send(&other_token, Method::PUT, &path, Some(changes.clone())).await.0, StatusCode::FORBIDDEN);
    let mut bad = changes.clone();
    bad["head"] = json!("not a commit");
    assert_eq!(world.send(&world.token, Method::PUT, &path, Some(bad)).await.0, StatusCode::BAD_REQUEST);

    let (status, updated) = world.send(&world.token, Method::PUT, &path, Some(changes)).await;
    assert_eq!((status, &updated["hasChanges"]), (StatusCode::OK, &json!(true)), "{updated}");

    let (status, shown) = world.send(&other_token, Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK, "anyone who sees the run sees its changes");
    assert_eq!((&shown["head"], &shown["files"][0]["additions"], shown["updatedAt"].is_string()), (&json!("abc1234"), &json!(12), true));

    // The push and the pull request: decided here, carried out there.
    let proposals = format!("/runs/{id}/proposals");
    let push = world.post(&proposals, json!({ "kind": "push", "payload": { "branch": "feature/WEB-1", "commit": "abc1234" } })).await;
    let pull = world.post(&proposals, json!({ "kind": "pull_request", "payload": { "title": "WEB-1 Login", "head": "feature/WEB-1" } })).await;
    let (push_id, pull_id) = (push["id"].as_i64().unwrap(), pull["id"].as_i64().unwrap());
    let of = |detail: &Value, id: i64| detail["proposals"].as_array().unwrap().iter().find(|proposal| proposal["id"] == id).cloned().unwrap();

    assert_eq!(push["payload"]["commit"], "abc1234");
    assert_eq!(
        world.send(&world.token, Method::POST, &format!("/proposals/{push_id}/settle"), Some(json!({ "approve": true }))).await.0,
        StatusCode::CONFLICT,
        "nobody is at this runner to settle it"
    );
    assert_eq!(
        world.send(&world.token, Method::POST, &format!("/proposals/{push_id}/result"), Some(json!({ "result": { "commit": "abc1234" } }))).await.0,
        StatusCode::CONFLICT,
        "not approved yet"
    );

    let approved = world.post(&format!("/proposals/{push_id}/decide"), json!({ "approve": true, "note": "Looks right." })).await;
    let on_run = of(&approved, push_id);
    assert_eq!((&on_run["state"], &on_run["result"], &on_run["decidedBy"]), (&json!("approved"), &Value::Null, &json!(world.admin.id)));
    assert_eq!(texts(&approved["events"]).last(), Some(&"Approved: pushing the branch; the runner carries it out"));
    assert_eq!(approved["run"]["state"], "waiting", "the pull request is still undecided");

    assert_eq!(
        world.send(&other_token, Method::POST, &format!("/proposals/{push_id}/result"), Some(json!({ "result": { "commit": "abc1234" } }))).await.0,
        StatusCode::FORBIDDEN
    );
    let done = world.post(&format!("/proposals/{push_id}/result"), json!({ "result": { "commit": "abc1234" } })).await;
    assert_eq!((&of(&done, push_id)["state"], &of(&done, push_id)["result"]["commit"]), (&json!("approved"), &json!("abc1234")));
    assert_eq!(
        world.send(&world.token, Method::POST, &format!("/proposals/{push_id}/result"), Some(json!({ "result": { "commit": "again" } }))).await.0,
        StatusCode::CONFLICT
    );

    // The pull request is approved, and opening it fails: the person sees that it was not done.
    world.post(&format!("/proposals/{pull_id}/decide"), json!({ "approve": true })).await;
    let failed = world.post(&format!("/proposals/{pull_id}/result"), json!({ "error": "GitHub answered 422: no commits between main and feature/WEB-1" })).await;
    assert_eq!(
        (&of(&failed, pull_id)["state"], &of(&failed, pull_id)["note"]),
        (&json!("failed"), &json!("GitHub answered 422: no commits between main and feature/WEB-1"))
    );
    assert_eq!(failed["run"]["state"], "running", "nothing is left to decide");

    // One that succeeds puts its address on the run; a rejected one is simply rejected.
    let second = world.post(&proposals, json!({ "kind": "pull_request", "payload": { "title": "WEB-1 Login" } })).await["id"].as_i64().unwrap();
    world.post(&format!("/proposals/{second}/decide"), json!({ "approve": true })).await;
    let opened = world.post(&format!("/proposals/{second}/result"), json!({ "result": { "url": "https://github.com/acme/shop/pull/9" } })).await;
    assert_eq!(opened["run"]["pullRequestUrl"], "https://github.com/acme/shop/pull/9");

    let third = world.post(&proposals, json!({ "kind": "push", "payload": { "branch": "feature/WEB-1" } })).await["id"].as_i64().unwrap();
    let rejected = world.post(&format!("/proposals/{third}/decide"), json!({ "approve": false })).await;
    assert_eq!(of(&rejected, third)["state"], "rejected");

    // A runner with a person at it keeps deciding in its own app.
    world.post("/runners/heartbeat", json!({ "runnerId": "desk", "name": "Desktop", "kinds": ["implement"], "version": "1" })).await;
    let local = world.post("/issues/WEB-2/runs", json!({ "kind": "implement", "runnerId": "desk" })).await;
    assert_eq!(local["runnerInteractive"], true);
    let own = world.post(&format!("/runs/{}/proposals", local["id"]), json!({ "kind": "push", "payload": { "branch": "x" } })).await["id"].as_i64().unwrap();
    assert_eq!(world.send(&world.token, Method::POST, &format!("/proposals/{own}/decide"), Some(json!({ "approve": true }))).await.0, StatusCode::CONFLICT);
    assert_eq!(
        world.send(&world.token, Method::POST, &format!("/proposals/{own}/result"), Some(json!({ "result": { "commit": "abc1234" } }))).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_sent_to_one_runner_by_name_waits_for_it_and_a_computer_takes_such_work_from_its_owner_only() {
    let world = world();
    let (_, other_token) = person(&world.app, "Piotr Runner", "piotr@example.com", Role::Member);
    let beat = |runner: &str, name: &str, more: Value| {
        let mut beat = json!({ "runnerId": runner, "name": name, "kinds": ["implement"], "version": "1" });
        beat.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        beat
    };
    let start = |token: &str, body: Value| {
        let token = token.to_string();
        let world = &world;
        async move { world.send(&token, Method::POST, "/issues/WEB-1/runs", Some(body)).await }
    };

    world.post("/runners/heartbeat", beat("laptop", "Bartek's MacBook", json!({ "pinnedOnly": true }))).await;
    world.post("/runners/heartbeat", beat("cloud", "Cloud runner", json!({ "interactive": false, "projects": ["WEB"] }))).await;
    assert_eq!(world.send(&other_token, Method::POST, "/runners/heartbeat", Some(beat("thinkpad", "Piotr's ThinkPad", json!({})))).await.0, StatusCode::OK);

    let listed = world.get("/runners").await;
    let laptop = listed.as_array().unwrap().iter().find(|runner| runner["id"] == "laptop").unwrap();
    assert_eq!(laptop["pinnedOnly"], true);

    // Who may be named, and for what.
    let refused = |body: Value| start(&world.token, body);
    assert_eq!(refused(json!({ "kind": "implement", "onRunner": "nobody" })).await.0, StatusCode::BAD_REQUEST, "an unknown runner");
    assert_eq!(refused(json!({ "kind": "review", "onRunner": "cloud" })).await.0, StatusCode::BAD_REQUEST, "a kind it does not take");
    assert_eq!(refused(json!({ "kind": "implement", "onRunner": "cloud", "runnerId": "cloud" })).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(refused(json!({ "kind": "implement", "onRunner": "thinkpad" })).await.0, StatusCode::FORBIDDEN, "somebody else's computer");
    assert_eq!(world.count("SELECT count(*) FROM agent_run"), 0, "a refused run is not left behind");

    // A run nobody was named for: the computer that takes only named work is not offered it.
    let loose = world.post("/issues/WEB-2/runs", json!({ "kind": "implement" })).await;
    assert_eq!((&loose["onRunner"], &loose["onRunnerName"]), (&Value::Null, &Value::Null));
    assert_eq!(claim(&world, &world.token, "laptop", json!(["implement"])).await, Value::Null);

    // Sent to the computer: it waits for it, and the other runner of the same person passes it by.
    let (status, here) = start(&world.token, json!({ "kind": "implement", "onRunner": "laptop" })).await;
    assert_eq!(status, StatusCode::OK, "{here}");
    assert_eq!((&here["state"], &here["onRunner"], &here["onRunnerName"], &here["runnerId"]), (&json!("queued"), &json!("laptop"), &json!("Bartek's MacBook"), &Value::Null));
    assert_eq!(texts(&world.run(here["id"].as_i64().unwrap()).await["events"]), ["Sent to Bartek's MacBook"]);

    let by_cloud = claim(&world, &world.token, "cloud", json!(["implement"])).await;
    assert_eq!(by_cloud["run"]["id"], loose["id"], "the cloud runner takes the one sent to nobody");
    assert_eq!(claim(&world, &world.token, "cloud", json!(["implement"])).await, Value::Null, "and not the one sent to the computer");

    let by_laptop = claim(&world, &world.token, "laptop", json!(["implement"])).await;
    assert_eq!((&by_laptop["run"]["id"], &by_laptop["run"]["runnerId"], &by_laptop["run"]["onRunner"]), (&here["id"], &json!("laptop"), &json!("laptop")));

    // A service takes named work from anyone who may start it, shared or not; and what is
    // sent to it comes before the older run in the queue.
    let older = world.post("/issues/WEB-2/runs", json!({ "kind": "implement" })).await;
    let (status, theirs) = start(&other_token, json!({ "kind": "implement", "onRunner": "cloud" })).await;
    assert_eq!((status, &theirs["shared"]), (StatusCode::OK, &json!(false)), "{theirs}");
    assert_eq!(claim(&world, &other_token, "thinkpad", json!(["implement"])).await, Value::Null, "Piotr's own computer is not offered what he sent elsewhere");
    assert_eq!(claim(&world, &world.token, "cloud", json!(["implement"])).await["run"]["id"], theirs["id"]);
    assert_eq!(claim(&world, &world.token, "cloud", json!(["implement"])).await["run"]["id"], older["id"]);

    // Queued again after the runner went silent, it still waits for the same one.
    let id = here["id"].as_i64().unwrap();
    let later = Utc::now() + Duration::minutes(11);
    world.app.write(Actor::system("test"), |tx| runs::requeue_stale(tx, later)).unwrap();
    let again = world.run(id).await;
    assert_eq!((&again["run"]["state"], &again["run"]["onRunner"], &again["run"]["runnerId"]), (&json!("queued"), &json!("laptop"), &Value::Null));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claimed_run_whose_runner_went_silent_for_ten_minutes_returns_to_the_queue() {
    let world = world();
    world.post("/runners/heartbeat", json!({ "runnerId": "desk-1", "name": "Desktop", "kinds": ["implement"], "version": "1" })).await;
    let id = world.start("WEB-1", "implement", "").await;
    claim(&world, &world.token, "desk-1", json!(["implement"])).await;
    let sweep = |minutes: i64| {
        world.app.write(Actor::system("test"), |tx| runs::requeue_stale(tx, Utc::now() + Duration::minutes(minutes))).expect("a sweep")
    };

    assert_eq!(sweep(9), 0, "heard from nine minutes ago");
    assert_eq!(world.run(id).await["run"]["state"], "claimed");
    assert_eq!(sweep(11), 1);

    let detail = world.run(id).await;
    assert_eq!((&detail["run"]["state"], &detail["run"]["runnerId"]), (&json!("queued"), &Value::Null));
    assert_eq!(texts(&detail["events"]).last(), Some(&"The runner stopped answering; the run is queued again"));
    assert_eq!(claim(&world, &world.token, "desk-1", json!(["implement"])).await["run"]["id"], id, "and can be taken again");

    // A run that waits for a person is not the runner's to keep alive.
    world.post(&format!("/runs/{id}/events"), json!({ "kind": "question", "text": "Which branch?" })).await;
    assert_eq!(sweep(60), 0);

    // The server's own runs were interrupted by a restart, not by silence.
    let (_fake, address) = fake_model().await;
    world.configure(&address).await;
    let summary = world.start("WEB-2", "summarize", "").await;
    world.app.write(Actor::system("test"), |tx| runs::hold(tx, summary, "server", "Server", None)).expect("held");

    assert_eq!(sweep(60), 0);
    assert_eq!(world.app.write(Actor::system("test"), runs::requeue_interrupted).expect("requeued"), 1);
    assert!(world.work().await);
    assert_eq!(world.run(summary).await["run"]["state"], "succeeded");
}

// ---------- proposals ----------

/// A run held by the administrator's runner, to post proposals to.
async fn held_run(world: &World, key: &str) -> i64 {
    let id = world.start(key, "custom", "").await;
    world.post("/runners/heartbeat", json!({ "runnerId": "desk-1", "name": "Desktop", "kinds": ["custom"], "version": "1" })).await;
    claim(world, &world.token, "desk-1", json!(["custom"])).await;

    id
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_that_can_no_longer_be_applied_fails_with_the_reason_and_changes_nothing() {
    let world = world();
    let (vera, _) = person(&world.app, "Vera Leaving", "vera@example.com", Role::Member);
    let id = held_run(&world, "WEB-1").await;
    let proposal = world
        .post(
            &format!("/runs/{id}/proposals"),
            json!({ "kind": "update_fields", "payload": { "assignee": "vera@example.com", "summary": "Login fails" } }),
        )
        .await;

    assert_eq!(proposal["payload"], json!({ "summary": "Login fails", "assignee": "Vera Leaving" }));
    world
        .app
        .write(world.admin.actor(), |tx| update_user(tx, &world.admin, vera.id, &UserPatch { active: Some(false), ..Default::default() }))
        .expect("deactivated");

    let decided = world.decide(&proposal, json!({ "approve": true })).await;

    assert_eq!(
        (&decided["proposals"][0]["state"], &decided["proposals"][0]["note"]),
        (&json!("failed"), &json!("That person does not exist or is no longer active"))
    );
    assert_eq!(
        world.text("SELECT summary FROM issue WHERE key = 'WEB-1'").as_deref(),
        Some("Login fails on Safari"),
        "the summary was rolled back with the rest"
    );
    assert_eq!(decided["run"]["state"], "running", "back with its runner");
    assert!(texts(&decided["events"]).last().is_some_and(|text| text.starts_with("Could not apply changes to the issue's fields")));
}

#[tokio::test(flavor = "multi_thread")]
async fn every_kind_of_proposal_is_checked_on_arrival_and_applied_on_approval() {
    let world = world();
    let id = held_run(&world, "WEB-1").await;
    let path = format!("/runs/{id}/proposals");
    let refused = |kind: &'static str, payload: Value| {
        let (world, path) = (&world, path.clone());

        async move { world.send(&world.token, Method::POST, &path, Some(json!({ "kind": kind, "payload": payload }))).await }
    };

    for (kind, payload) in [
        ("comment", json!({ "body": "  " })),
        ("transition", json!({ "status": "Shipped" })),
        ("update_fields", json!({})),
        ("update_fields", json!({ "priority": "urgent" })),
        ("update_fields", json!({ "dueDate": "next week" })),
        ("create_subtasks", json!({ "items": [] })),
        ("create_issue", json!({ "projectKey": "NOPE", "summary": "x" })),
        ("create_issue", json!({ "projectKey": "WEB", "summary": "x", "type": "Feature" })),
        ("log_work", json!({ "spent": "a while" })),
        ("comment", json!({ "text": "wrong field" })),
    ] {
        let (status, value) = refused(kind, payload.clone()).await;
        assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND, "{kind} {payload}: {status} {value}");
    }

    assert_eq!(world.count("SELECT count(*) FROM agent_proposal"), 0);

    let mut proposals = Vec::new();

    for (kind, payload) in [
        ("create_issue", json!({ "projectKey": "web", "summary": "Follow-up: Safari 17", "type": "bug" })),
        ("log_work", json!({ "spent": "90m", "note": "investigation" })),
        ("update_fields", json!({ "points": 3, "dueDate": "2026-12-01", "assignee": "me" })),
        ("transition", json!({ "status": "done" })),
    ] {
        proposals.push(world.post(&path, json!({ "kind": kind, "payload": payload })).await);
    }

    assert_eq!(proposals[1]["payload"], json!({ "spent": "1h 30m", "note": "investigation" }));
    assert_eq!(proposals[0]["preview"]["issueType"], "bug");

    for proposal in &proposals {
        let decided = world.decide(proposal, json!({ "approve": true })).await;
        let state =
            decided["proposals"].as_array().unwrap().iter().find(|candidate| candidate["id"] == proposal["id"]).map(|found| found["state"].clone());

        assert_eq!(state, Some(json!("approved")), "{proposal}");
    }

    assert_eq!(world.text("SELECT summary FROM issue WHERE key = 'WEB-3'").as_deref(), Some("Follow-up: Safari 17"));
    assert_eq!(world.count("SELECT minutes FROM worklog"), 90);
    assert_eq!(
        world.count(&format!(
            "SELECT count(*) FROM issue WHERE key = 'WEB-1' AND points = 3 AND due_date = '2026-12-01' AND assignee_id = {}",
            world.admin.id
        )),
        1
    );
    assert_eq!(world.text("SELECT s.name FROM issue i JOIN status s ON s.id = i.status_id WHERE i.key = 'WEB-1'").as_deref(), Some("Done"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_can_trust_a_kind_of_proposal_which_is_then_applied_without_a_click() {
    let world = world();
    let (_, member) = person(&world.app, "Maria Member", "maria@example.com", Role::Member);

    assert_eq!(world.get("/projects/WEB/trust").await, json!({ "kinds": [] }), "nothing is trusted by default");
    assert_eq!(world.send(&member, Method::PUT, "/projects/WEB/trust", Some(json!({ "kinds": ["comment"] }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        world.ok(Method::PUT, "/projects/WEB/trust", Some(json!({ "kinds": ["log_work", "comment"] }))).await,
        json!({ "kinds": ["comment", "log_work"] })
    );

    let id = held_run(&world, "WEB-1").await;
    let comment = world.post(&format!("/runs/{id}/proposals"), json!({ "kind": "comment", "payload": { "body": "Starting on this." } })).await;
    let fields = world.post(&format!("/runs/{id}/proposals"), json!({ "kind": "update_fields", "payload": { "points": 2 } })).await;

    assert_eq!((&comment["state"], &comment["decidedBy"]), (&json!("approved"), &json!(world.admin.id)));
    assert_eq!(comment["note"], "Applied at once: the project trusts this kind of proposal");
    assert_eq!(fields["state"], "pending");
    assert_eq!(world.text("SELECT body FROM comment").as_deref(), Some("Starting on this."));
    assert_eq!(world.run(id).await["run"]["state"], "waiting", "the other proposal still needs a person");
}

// ---------- who may do what ----------

#[tokio::test(flavor = "multi_thread")]
async fn starting_answering_deciding_and_cancelling_need_edit_access_and_reading_needs_to_see_the_project() {
    let world = world();
    let (viewer, viewer_token) = person(&world.app, "Vera Viewer", "vera@example.com", Role::Member);
    let (_, outsider_token) = person(&world.app, "Olga Outside", "olga@example.com", Role::Member);

    project(&world, "SEC", true);
    issue(&world, "SEC", "For members only", "");
    world
        .app
        .write(world.admin.actor(), |tx| set_member(tx, &project_access(tx, &world.admin, "WEB")?, viewer.id, Some(ProjectRole::Viewer)))
        .expect("a viewer");

    let id = held_run(&world, "WEB-1").await;
    let secret = world.start("SEC-1", "implement", "").await;
    let proposal = world.post(&format!("/runs/{id}/proposals"), json!({ "kind": "comment", "payload": { "body": "Hello" } })).await;
    world.post(&format!("/runs/{id}/events"), json!({ "kind": "question", "text": "Why?" })).await;
    let as_viewer = |method: Method, path: String, body: Value| {
        let (world, token) = (&world, viewer_token.clone());

        async move { world.send(&token, method, &path, Some(body)).await.0 }
    };

    assert_eq!(world.send(&viewer_token, Method::GET, &format!("/runs/{id}"), None).await.0, StatusCode::OK, "a viewer may read");
    assert_eq!(world.send(&viewer_token, Method::GET, "/issues/WEB-1/runs", None).await.1.as_array().map(Vec::len), Some(1));
    assert_eq!(as_viewer(Method::POST, "/issues/WEB-1/runs".to_string(), json!({ "kind": "refine" })).await, StatusCode::FORBIDDEN);
    assert_eq!(as_viewer(Method::POST, format!("/runs/{id}/cancel"), json!({})).await, StatusCode::FORBIDDEN);
    assert_eq!(as_viewer(Method::POST, format!("/runs/{id}/answer"), json!({ "text": "Because" })).await, StatusCode::FORBIDDEN);
    assert_eq!(as_viewer(Method::POST, format!("/proposals/{}/decide", proposal["id"]), json!({ "approve": true })).await, StatusCode::FORBIDDEN);

    for path in [format!("/runs/{secret}"), "/issues/SEC-1/runs".to_string(), "/projects/SEC/trust".to_string()] {
        assert_eq!(world.send(&outsider_token, Method::GET, &path, None).await.0, StatusCode::NOT_FOUND, "{path}");
    }

    assert_eq!(world.send(&outsider_token, Method::POST, &format!("/runs/{secret}/cancel"), Some(json!({}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(world.send(&outsider_token, Method::POST, "/issues/SEC-1/runs", Some(json!({ "kind": "refine" }))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(world.send(&outsider_token, Method::GET, "/runs", None).await.1.as_array().map(Vec::len), Some(1), "only the run in the open project");
    assert_eq!(world.get("/runs").await.as_array().map(Vec::len), Some(2));
    assert_eq!(world.get("/runs?project=sec").await[0]["id"], secret);
    assert_eq!(world.get("/runs?state=waiting,running&mine=true").await[0]["id"], id);
    assert_eq!(world.get("/runs?state=queued&limit=1").await[0]["issueKey"], "SEC-1");
    assert_eq!(world.send(&world.token, Method::GET, "/runs?state=sleeping", None).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(world.send("th_wrong", Method::GET, "/runs", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        world.send(&world.token, Method::POST, "/issues/WEB-1/runs", Some(json!({ "kind": "dance" }))).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

// ---------- what the rest of the application sees ----------

#[tokio::test(flavor = "multi_thread")]
async fn issues_carry_a_mark_a_panel_and_a_dashboard_entry_while_an_agent_needs_someone() {
    let world = world();
    let id = held_run(&world, "WEB-1").await;
    let ext = |world: &World| {
        let found = search(&world.app, &world.admin, "key = WEB-1", 10, 0).expect("a search");

        found.issues[0].ext.get("agents").cloned()
    };

    assert_eq!(ext(&world), Some(json!({ "working": 1, "waiting": 0 })));

    world.post(&format!("/runs/{id}/events"), json!({ "kind": "question", "text": "Which browser?" })).await;
    let panel = issue_detail(&world.app, &world.admin, "WEB-1").expect("the issue").panels.get("agents").cloned().expect("a panel");
    let mine = dashboard(&world.app, &world.admin).expect("my work").ext.get("agents").cloned().expect("a section");

    assert_eq!(ext(&world), Some(json!({ "working": 0, "waiting": 1 })));
    assert_eq!((&panel["runs"][0]["id"], &panel["runs"][0]["pendingQuestion"]), (&json!(id), &json!(true)));
    assert_eq!(mine["waiting"][0]["issueKey"], "WEB-1");

    world.post(&format!("/runs/{id}/answer"), json!({ "text": "Safari" })).await;
    world.ok(Method::PATCH, &format!("/runs/{id}"), Some(json!({ "state": "succeeded", "summary": "Done" }))).await;

    assert_eq!(ext(&world), None);
    assert!(!dashboard(&world.app, &world.admin).expect("my work").ext.contains_key("agents"));
}

// ---------- a plain MCP client as a runner ----------

async fn tool(world: &World, name: &str, arguments: Value) -> (bool, Value) {
    let message =
        world.post("/mcp", json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": arguments } })).await;
    let text = message["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{name}: {message}"));

    (message["result"]["isError"] == true, serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())))
}

#[tokio::test(flavor = "multi_thread")]
async fn an_mcp_client_can_take_a_run_report_ask_propose_and_finish() {
    let world = world();
    let (_, other_token) = person(&world.app, "Piotr Runner", "piotr@example.com", Role::Member);
    let id = world.start("WEB-1", "implement", "Fix the tap handler").await;
    let listed = tool(&world, "list_my_runs", json!({})).await.1;

    assert_eq!(listed.as_array().map(Vec::len), Some(1));
    assert_eq!(
        (&listed[0]["runId"], &listed[0]["issueKey"], &listed[0]["instructions"], &listed[0]["state"]),
        (&json!(id), &json!("WEB-1"), &json!("Fix the tap handler"), &json!("queued"))
    );
    assert_eq!(
        world
            .send(
                &other_token,
                Method::POST,
                "/mcp",
                Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "list_my_runs" } }))
            )
            .await
            .1["result"]["content"][0]["text"],
        "[]"
    );

    let reported = tool(&world, "report_progress", json!({ "runId": id, "text": "Reading the handler." })).await;
    assert_eq!(reported, (false, json!({ "runId": id, "state": "running", "cancelRequested": false })));
    assert_eq!(world.run(id).await["run"]["runnerName"], "MCP · Bartek Admin");

    assert_eq!(tool(&world, "ask_question", json!({ "runId": id, "text": "Touch events too?" })).await.1["state"], "waiting");
    world.post(&format!("/runs/{id}/answer"), json!({ "text": "Yes." })).await;

    let refused = tool(&world, "propose", json!({ "runId": id, "kind": "transition", "payload": { "status": "Shipped" } })).await;
    assert_eq!(refused, (true, json!("Unknown status \"Shipped\" (valid: To Do, In Progress, In Review, Done)")));

    let proposed = tool(&world, "propose", json!({ "runId": id, "kind": "transition", "payload": { "status": "In Review" } })).await.1;
    assert_eq!(proposed["state"], "pending");
    world.post(&format!("/proposals/{}/decide", proposed["proposalId"]), json!({ "approve": true })).await;

    let read = tool(&world, "get_run", json!({ "runId": id })).await.1;
    let answers: Vec<&str> =
        read["events"].as_array().unwrap().iter().filter(|event| event["kind"] == "answer").filter_map(|event| event["text"].as_str()).collect();

    assert_eq!((answers, &read["proposals"][0]["state"]), (vec!["Yes."], &json!("approved")));

    let finished = tool(
        &world,
        "finish_run",
        json!({ "runId": id, "summary": "Handler fixed.", "branch": "web-1-tap", "pullRequestUrl": "https://github.com/acme/shop/pull/9" }),
    )
    .await
    .1;
    let detail = world.run(id).await;

    assert_eq!(finished, json!({ "runId": id, "state": "succeeded", "pendingProposals": 0 }));
    assert_eq!((&detail["run"]["summary"], &detail["run"]["branch"]), (&json!("Handler fixed."), &json!("web-1-tap")));
    assert_eq!(tool(&world, "report_progress", json!({ "runId": id, "text": "One more thing" })).await, (true, json!("The run has already ended")));
    assert_eq!(tool(&world, "get_run", json!({ "runId": 999 })).await, (true, json!("Run not found")));
    assert_eq!(tool(&world, "list_my_runs", json!({})).await.1, json!([]));
}
