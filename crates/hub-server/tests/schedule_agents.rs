//! `agents.run`, the task kind of the agents module: a scheduled task that starts an agent
//! run on each issue a query finds.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{test_app, Client};
use hub_app::{App, Event};
use hub_server::{build_router, modules};
use serde_json::{json, Value};
use tokio::sync::broadcast::Receiver;

struct Scene {
    app: App,
    admin: Client,
    events: Receiver<Event>,
}

/// Two projects: WEB with five issues about signing in and one about something else, and
/// API with one about signing in.
async fn scene() -> Scene {
    let app = test_app(modules());
    let mut admin = Client::new(build_router(app.clone()));
    admin.setup(false).await;
    admin.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    admin.post("/api/projects", json!({ "key": "API", "name": "Api" })).await;

    for summary in ["Login fails on Safari", "Login page is slow", "Login with a passkey", "Login remembers me", "Login error is unclear", "Checkout total"] {
        admin.post("/api/issues", json!({ "projectKey": "WEB", "summary": summary })).await;
    }

    admin.post("/api/issues", json!({ "projectKey": "API", "summary": "Login endpoint returns 500" })).await;

    Scene { events: app.subscribe(), app, admin }
}

fn run_task(query: &str, config: Value) -> Value {
    let mut all = json!({ "query": query, "kind": "triage" });
    all.as_object_mut().unwrap().extend(config.as_object().unwrap().clone());

    json!({ "kind": "agents.run", "name": "Triage sign-in", "slot": "on_demand", "config": all })
}

async fn make(scene: &mut Scene, task: Value) -> i64 {
    scene.admin.post("/api/projects/WEB/tasks", task).await["id"].as_i64().expect("a task id")
}

/// Runs the task from its button, waits for the attempt to end, and returns the task.
async fn run(scene: &mut Scene, id: i64) -> Value {
    scene.events = scene.app.subscribe();
    assert_eq!(scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await["state"], "running");

    loop {
        let event = tokio::time::timeout(Duration::from_secs(30), scene.events.recv()).await.expect("the task's attempt ends").expect("an event");

        if event.kind == "task.updated" && event.payload["taskId"] == id && event.payload["state"] != "running" {
            break;
        }
    }

    let tasks = scene.admin.get("/api/projects/WEB/tasks").await;

    tasks.as_array().unwrap().iter().find(|task| task["id"] == id).expect("the task").clone()
}

/// The runs there are, oldest first, as (issue, kind).
async fn runs(scene: &mut Scene) -> Vec<(String, String)> {
    let mut runs: Vec<(i64, String, String)> = scene
        .admin
        .get("/api/m/agents/runs?limit=200")
        .await
        .as_array()
        .expect("runs")
        .iter()
        .map(|run| (run["id"].as_i64().unwrap(), run["issueKey"].as_str().unwrap().to_string(), run["kind"].as_str().unwrap().to_string()))
        .collect();
    runs.sort();

    runs.into_iter().map(|(_, key, kind)| (key, kind)).collect()
}

fn pair(key: &str, kind: &str) -> (String, String) {
    (key.to_string(), kind.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_kind_comes_with_its_form() {
    let mut scene = scene().await;
    let kinds = scene.admin.get("/api/task-kinds").await;
    let kind = kinds.as_array().unwrap().iter().find(|kind| kind["id"] == "agents.run").expect("the kind").clone();
    let fields: Vec<(&str, &str)> = kind["fields"].as_array().unwrap().iter().map(|field| (field["name"].as_str().unwrap(), field["type"].as_str().unwrap())).collect();

    assert_eq!(kind["module"], "agents");
    assert_eq!(fields, [("query", "query"), ("limit", "number"), ("kind", "select"), ("instructions", "textarea"), ("repo", "text"), ("shared", "boolean")]);
    assert_eq!(kind["fields"][0]["required"], true);
    assert_eq!(kind["fields"][1]["default"], 5);

    let run_kinds: Vec<&str> = kind["fields"][2]["options"].as_array().unwrap().iter().map(|option| option["value"].as_str().unwrap()).collect();
    assert_eq!(run_kinds, ["refine", "breakdown", "summarize", "triage", "implement", "review", "custom"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_started_on_each_issue_the_query_finds_in_the_project() {
    let mut scene = scene().await;
    let me = scene.admin.get("/api/bootstrap").await["user"]["id"].clone();
    let config = json!({ "instructions": "Be brief.", "repo": "acme/web", "shared": true });
    let id = make(&mut scene, run_task("summary ~ login ORDER BY key", config)).await;

    let task = run(&mut scene, id).await;
    assert_eq!(task["state"], "ok");
    assert_eq!(task["result"]["summary"], "Started 5 runs: WEB-1, WEB-2, WEB-3, WEB-4, WEB-5");
    assert_eq!(task["result"]["link"], "/agents");

    // The issue of the other project matches the words too, and is not the task's business.
    assert_eq!(runs(&mut scene).await, [pair("WEB-1", "triage"), pair("WEB-2", "triage"), pair("WEB-3", "triage"), pair("WEB-4", "triage"), pair("WEB-5", "triage")]);

    let first = &scene.admin.get("/api/m/agents/issues/WEB-1/runs").await[0];
    assert_eq!(first["state"], "queued");
    assert_eq!(first["requestedBy"], me, "the run belongs to the person the task runs as");
    assert_eq!(first["instructions"], "Be brief.");
    assert_eq!(first["repo"], "acme/web");
    assert_eq!(first["shared"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_project_comes_from_the_task_not_from_the_query() {
    let mut scene = scene().await;

    // The query names the other project: nothing of this one matches, and nothing of that one is touched.
    let id = make(&mut scene, run_task("project = API", json!({}))).await;
    assert_eq!(run(&mut scene, id).await["result"]["summary"], "Nothing matched");

    let id = make(&mut scene, run_task("project = API OR summary ~ checkout", json!({}))).await;
    assert_eq!(run(&mut scene, id).await["result"]["summary"], "Started 1 run: WEB-6");

    // Plain words search the text, in this project only.
    let id = make(&mut scene, run_task("endpoint", json!({}))).await;
    assert_eq!(run(&mut scene, id).await["result"]["summary"], "Nothing matched");
    assert_eq!(runs(&mut scene).await, [pair("WEB-6", "triage")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_limit_is_respected_and_unfinished_runs_are_not_doubled() {
    let mut scene = scene().await;
    let id = make(&mut scene, run_task("summary ~ login ORDER BY key", json!({ "limit": 2 }))).await;

    assert_eq!(run(&mut scene, id).await["result"]["summary"], "Started 2 runs: WEB-1, WEB-2");
    assert_eq!(runs(&mut scene).await.len(), 2);

    // The same again: both issues still have their run.
    assert_eq!(run(&mut scene, id).await["result"]["summary"], "Nothing started: 2 of the issues found already have an unfinished run");
    assert_eq!(runs(&mut scene).await.len(), 2);

    // A wider limit starts the rest and says how many it passed over.
    scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "config": { "query": "summary ~ login ORDER BY key", "kind": "triage", "limit": 4 } })).await;
    assert_eq!(run(&mut scene, id).await["result"]["summary"], "Started 2 runs: WEB-3, WEB-4; skipped 2 with an unfinished run");

    scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "config": { "query": "summary ~ login ORDER BY key", "kind": "triage", "limit": 20 } })).await;
    assert_eq!(run(&mut scene, id).await["result"]["summary"], "Started 1 run: WEB-5; skipped 4 with an unfinished run");
    assert_eq!(runs(&mut scene).await.len(), 5);

    // A run of another kind is another matter, and a run that has ended does not count.
    let other = make(&mut scene, run_task("key = WEB-1", json!({ "kind": "summarize" }))).await;
    assert_eq!(run(&mut scene, other).await["result"]["summary"], "Started 1 run: WEB-1");

    let first = scene.admin.get("/api/m/agents/issues/WEB-1/runs").await.as_array().unwrap().iter().find(|run| run["kind"] == "triage").unwrap()["id"].clone();
    scene.admin.post(&format!("/api/m/agents/runs/{first}/cancel"), json!({})).await;
    let again = make(&mut scene, run_task("key = WEB-1", json!({}))).await;
    assert_eq!(run(&mut scene, again).await["result"]["summary"], "Started 1 run: WEB-1");
    assert_eq!(runs(&mut scene).await.len(), 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_that_cannot_be_carried_out_fail_the_attempt_and_keep_the_result() {
    let mut scene = scene().await;
    let id = make(&mut scene, run_task("key = WEB-6", json!({}))).await;
    let good = run(&mut scene, id).await["result"].clone();
    assert_eq!(good["summary"], "Started 1 run: WEB-6");

    // The schedule checks a query when it is saved...
    let (status, answer) = scene.admin.send(Method::PATCH, &format!("/api/tasks/{id}"), Some(json!({ "config": { "query": "status = ", "kind": "triage" } }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(answer["error"]["message"], "\"Query\": Expected a value");

    // ...but one that was fine then can stop being so: a field of a module that is switched off since.
    let stale = json!({ "query": "status = ", "kind": "triage" }).to_string();
    scene.app.db.write(|tx| Ok(tx.execute("UPDATE scheduled_task SET config = ?1 WHERE id = ?2", (&stale, id))?)).unwrap();

    let failed = run(&mut scene, id).await;
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["lastError"], "The query cannot be understood: Expected a value");
    assert_eq!(failed["result"], good);

    // What the form cannot check, the module does when the task runs.
    scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "config": { "query": "key = WEB-6", "kind": "triage", "limit": 50 } })).await;
    assert_eq!(run(&mut scene, id).await["lastError"], "The limit has to be a whole number from 1 to 20");
    assert_eq!(scene.admin.get("/api/m/agents/runs?limit=200").await.as_array().unwrap().len(), 1, "nothing was started by the attempts that failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_that_switched_the_module_off_runs_none_of_its_tasks() {
    let mut scene = scene().await;
    let id = make(&mut scene, run_task("summary ~ login", json!({}))).await;

    scene.admin.put("/api/projects/WEB/modules/agents", json!({ "enabled": false })).await;

    let task = scene.admin.get("/api/projects/WEB/tasks").await[0].clone();
    assert_eq!(task["kindAvailable"], false);
    assert_eq!(scene.admin.refused(Method::POST, &format!("/api/tasks/{id}/run"), Some(json!({}))).await, StatusCode::CONFLICT);
    assert_eq!(scene.admin.post("/api/projects/WEB/tasks/run", json!({ "slot": "on_demand" })).await, json!([]));

    scene.admin.put("/api/projects/WEB/modules/agents", json!({ "enabled": true })).await;
    assert_eq!(run(&mut scene, id).await["state"], "ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demo_has_a_schedule_to_show() {
    let mut client = Client::new(build_router(test_app(modules())));
    client.setup(true).await;

    let tasks = client.get("/api/projects/WEB/tasks").await;
    let slots: Vec<&str> = tasks.as_array().unwrap().iter().map(|task| task["slot"].as_str().unwrap()).collect();

    assert_eq!(slots, ["morning", "afternoon", "on_demand"]);
    assert!(tasks.as_array().unwrap().iter().all(|task| task["kind"] == "agents.run" && task["kindAvailable"] == true && task["enabled"] == true));
    assert_eq!(tasks[0]["state"], "ok");
    assert_eq!(tasks[0]["result"]["summary"], "Nothing matched");
    assert_eq!(tasks[1]["state"], "idle");
    assert_eq!(client.get("/api/m/agents/runs").await, json!([]), "making the demo started nothing");
}
