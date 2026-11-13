//! `e2e.job`, the task kind of the e2e module: a scheduled task that runs the project's test
//! suite on a runner and takes the job's end as its own.

mod common;

use axum::http::StatusCode;
use common::{test_app, Client};
use hub_server::{build_router, modules};
use serde_json::{json, Value};

const REPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="e2e" tests="3" failures="1" errors="0" skipped="0" time="4.000">
  <testsuite name="tests/cart.e2e.ts" tests="3" failures="1" errors="0" skipped="0" time="4.000">
    <testcase name="adds a product" classname="tests/cart.e2e.ts" time="1.000"/>
    <testcase name="removes a product" classname="tests/cart.e2e.ts" time="1.000"/>
    <testcase name="pays by card" classname="tests/cart.e2e.ts" time="2.000"><failure message="expected 200">trace</failure></testcase>
  </testsuite>
</testsuites>"#;

struct Scene {
    admin: Client,
    runner: Client,
}

async fn scene() -> Scene {
    let app = test_app(modules());
    let mut admin = Client::new(build_router(app));
    admin.setup(false).await;
    admin.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    admin.put("/api/m/e2e/projects/WEB/settings", json!({ "repo": "acme/shop", "ref": "main" })).await;
    let token = admin.post("/api/me/tokens", json!({ "name": "runner" })).await["token"].as_str().unwrap().to_string();
    let mut runner = Client::new(admin.router.clone());
    runner.bearer = Some(token);

    Scene { admin, runner }
}

async fn task(scene: &mut Scene, id: i64) -> Value {
    scene.admin.get("/api/projects/WEB/tasks").await.as_array().unwrap().iter().find(|task| task["id"] == id).expect("the task").clone()
}

/// Presses the task's button and waits until the job it queues is there to be claimed.
async fn claimed(scene: &mut Scene, id: i64) -> Value {
    assert_eq!(scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await["state"], "running");

    for _ in 0..200 {
        let claim = scene.runner.post("/api/m/e2e/jobs/claim", json!({ "runnerId": "cloud-1", "runnerName": "Cloud runner 1", "interactive": false })).await;

        if !claim.is_null() {
            return claim;
        }

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    panic!("the task queued no job");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_suite_stays_running_until_its_job_ends_and_reports_what_the_tests_did() {
    let mut scene = scene().await;
    let kinds = scene.admin.get("/api/task-kinds").await;
    let kind = kinds.as_array().unwrap().iter().find(|kind| kind["id"] == "e2e.job").expect("the kind");
    let fields: Vec<&str> = kind["fields"].as_array().unwrap().iter().map(|field| field["name"].as_str().unwrap()).collect();
    assert_eq!((kind["module"].as_str(), fields), (Some("e2e"), vec!["ref", "filter", "video", "onRunner"]));

    let made = scene
        .admin
        .post("/api/projects/WEB/tasks", json!({ "kind": "e2e.job", "name": "Nightly suite", "slot": "on_demand", "config": { "ref": "release", "filter": "tests/cart.e2e.ts" } }))
        .await;
    let id = made["id"].as_i64().expect("a task id");

    let claim = claimed(&mut scene, id).await;
    let job = claim["job"]["id"].as_i64().unwrap();
    assert_eq!((&claim["job"]["ref"], &claim["job"]["filter"], &claim["job"]["repo"]), (&json!("release"), &json!("tests/cart.e2e.ts"), &json!("acme/shop")));
    let waiting = task(&mut scene, id).await;
    assert_eq!((&waiting["state"], &waiting["progress"]), (&json!("running"), &json!("Tests of release queued for a runner")));

    // The runner sends the report and finishes: the task's attempt ends with the job.
    let (status, stored) =
        scene.runner.send_raw(&format!("/api/m/e2e/projects/WEB/runs?job={job}"), &[("content-type", "application/xml".to_string())], REPORT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(scene.runner.post(&format!("/api/m/e2e/jobs/{job}/finish"), json!({})).await["state"], "succeeded");

    let done = task(&mut scene, id).await;
    assert_eq!(done["state"], "ok", "{done}");
    assert_eq!(
        (&done["result"]["summary"], &done["result"]["link"], &done["progress"]),
        (&json!("1 of 3 tests failed, 0 flaky"), &json!(format!("/p/WEB/tests/jobs/{job}")), &Value::Null)
    );

    // A job that could not be run is the task's failure; the older result stays on show.
    let claim = claimed(&mut scene, id).await;
    let job = claim["job"]["id"].as_i64().unwrap();
    scene.runner.post(&format!("/api/m/e2e/jobs/{job}/finish"), json!({ "error": "npm ci exited with 1" })).await;
    let failed = task(&mut scene, id).await;
    assert_eq!((&failed["state"], &failed["lastError"], &failed["result"]["summary"]), (&json!("failed"), &json!("npm ci exited with 1"), &json!("1 of 3 tests failed, 0 flaky")));

    // Cancelled before any runner took it.
    assert_eq!(scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await["state"], "running");
    let mut queued = Value::Null;

    for _ in 0..200 {
        queued = scene.admin.get("/api/m/e2e/projects/WEB/jobs").await[0].clone();

        if queued["state"] == "queued" {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    scene.admin.post(&format!("/api/m/e2e/jobs/{}/cancel", queued["id"]), json!({})).await;
    let cancelled = task(&mut scene, id).await;
    assert_eq!((&cancelled["state"], &cancelled["lastError"]), (&json!("failed"), &json!("The test job was cancelled")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_without_a_repository_to_test_fails_the_task_with_where_to_set_one() {
    let mut scene = scene().await;
    scene.admin.post("/api/projects", json!({ "key": "API", "name": "Api" })).await;
    let id = scene.admin.post("/api/projects/API/tasks", json!({ "kind": "e2e.job", "name": "Suite", "slot": "on_demand", "config": {} })).await["id"].as_i64().unwrap();
    scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await;
    let mut seen = Value::Null;

    for _ in 0..200 {
        seen = scene.admin.get("/api/projects/API/tasks").await[0].clone();

        if seen["state"] != "running" {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    assert_eq!(seen["state"], "failed", "{seen}");
    assert!(seen["lastError"].as_str().unwrap().contains("Project settings → Tests"), "{seen}");
}
