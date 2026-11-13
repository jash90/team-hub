//! Test jobs against their contract (`docs/e2e-jobs.md`): settings and secrets, the life of
//! a job, its log, the files of a run, and what a result remembers of its session.

mod common;

use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use common::{events, member, person, project, switch_module, test_app, Client};
use hub_app::{App, Module};
use hub_core::{Id, ProjectRole, Role};
use hub_mod_e2e::{artifacts, jobs, E2eModule};
use serde_json::{json, Value};

/// A report the framework wrote for this repository's own suite (`e2e/`, run with `--trace`),
/// cut down to five results: one that failed twice with screenshots on the way, three that
/// passed, one that was skipped.
const REAL_REPORT: &str = include_str!("fixtures/report-real.trimmed.json");
/// The framework's own example of a valid report, which has a serial group.
const SERIAL_REPORT: &str = include_str!("fixtures/report-v1.valid.json");
const SECRET: &str = "s3cret-value-of-the-payment-sandbox";

struct Hub {
    app: App,
    admin: Client,
    /// A member of every open project: asks for jobs.
    dev: Client,
    /// Another member, whose token a runner uses.
    runner: Client,
    viewer: Client,
}

/// Project WEB with a repository set, an administrator, two members and a viewer.
async fn hub() -> Hub {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let dev = person(&app, "Daria Dev", Role::Member);
    let runner = person(&app, "Robert Runner", Role::Member);
    let viewer = person(&app, "Wanda Viewer", Role::Member);
    project(&app, &admin, "WEB", false);
    member(&app, &admin, "WEB", &viewer, ProjectRole::Viewer);
    admin.put_json("/projects/WEB/settings", json!({ "repo": "acme/shop" })).await;

    Hub { app, admin, dev, runner, viewer }
}

fn claim_body() -> Value {
    json!({ "runnerId": "runner-1", "runnerName": "Cloud runner" })
}

/// Queues a job as `dev` and claims it as `runner`; returns its id.
async fn claimed_job(hub: &Hub) -> Id {
    let job = hub.dev.post("/projects/WEB/jobs", json!({})).await;
    let claimed = hub.runner.post("/jobs/claim", claim_body()).await;
    assert_eq!(claimed["job"]["id"], job["id"]);

    job["id"].as_i64().expect("a job id")
}

fn job_events(app: &App) -> Vec<String> {
    events(app).into_iter().map(|(kind, _)| kind).filter(|kind| kind.starts_with("e2e.job")).collect()
}

async fn post_report(who: &Client, path: &str, report: &str) -> Value {
    let (status, value) = who.send_raw(Method::POST, path, Some("application/json"), report.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "POST {path}: {value}");

    value
}

/// A path as a query parameter carries it: everything but letters, digits and `-._/` escaped.
fn escaped(path: &str) -> String {
    path.bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'/' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

async fn upload(who: &Client, run_id: &Value, path: &str, bytes: &[u8]) -> (StatusCode, Value) {
    who.send_raw(Method::PUT, &format!("/runs/{run_id}/artifacts?path={}", escaped(path)), Some("application/octet-stream"), bytes.to_vec()).await
}

fn run_files(app: &App, run_id: &Value) -> Vec<String> {
    let directory: PathBuf = app.config.data_dir.join("e2e-artifacts").join(run_id.to_string());
    let mut names: Vec<String> =
        std::fs::read_dir(directory).map(|files| files.flatten().map(|file| file.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();

    names
}

// ---------- settings and secrets ----------

#[tokio::test(flavor = "multi_thread")]
async fn settings_start_from_defaults_and_only_an_administrator_changes_them() {
    let Hub { app, admin, dev, viewer, .. } = hub().await;
    project(&app, &admin, "NEW", false);

    let fresh = viewer.get("/projects/NEW/settings").await;
    assert_eq!(
        fresh,
        json!({ "repo": null, "ref": "main", "directory": "", "installCommand": "npm ci", "runArgs": "", "video": "on", "trace": "on",
                "env": {}, "secretNames": [], "timeoutMinutes": 30 })
    );

    assert_eq!(dev.refused(Method::PUT, "/projects/NEW/settings", Some(json!({ "repo": "acme/shop" }))).await.0, StatusCode::FORBIDDEN);

    let saved = admin
        .put_json(
            "/projects/NEW/settings",
            json!({ "repo": "acme/shop.git", "ref": "release/2", "directory": "./e2e/", "installCommand": "pnpm install --frozen-lockfile",
                    "runArgs": "--workers 2", "video": "retain-on-failure", "trace": "off", "env": { "APP_URL": "https://staging.example.com" },
                    "timeoutMinutes": 45 }),
        )
        .await;
    assert_eq!(
        saved,
        json!({ "repo": "acme/shop", "ref": "release/2", "directory": "e2e", "installCommand": "pnpm install --frozen-lockfile",
                "runArgs": "--workers 2", "video": "retain-on-failure", "trace": "off", "env": { "APP_URL": "https://staging.example.com" },
                "secretNames": [], "timeoutMinutes": 45 })
    );

    // What a request leaves out stays as it is; `repo: null` takes the repository away.
    let partly = admin.put_json("/projects/NEW/settings", json!({ "timeoutMinutes": 240, "repo": null })).await;
    assert_eq!((partly["repo"].clone(), partly["ref"].clone(), partly["timeoutMinutes"].clone()), (Value::Null, json!("release/2"), json!(240)));
    assert_eq!(dev.get("/projects/NEW/settings").await, partly);

    for (bad, says) in [
        (json!({ "repo": "https://github.com/acme/shop" }), "owner/name"),
        (json!({ "ref": "--upload-pack=evil" }), "branch"),
        (json!({ "directory": "../outside" }), "inside the repository"),
        (json!({ "video": "sometimes" }), "video mode"),
        (json!({ "trace": "on-first-retry" }), "trace mode"),
        (json!({ "timeoutMinutes": 241 }), "between 1 and 240"),
        (json!({ "env": { "1ST": "x" } }), "cannot name a variable"),
        (json!({ "secrets": { "has space": "x" } }), "cannot name a variable"),
        (json!({ "secrets": { "TOKEN": "" } }), "has no value"),
        (json!({ "env": { "TOKEN": "a" }, "secrets": { "TOKEN": "b" } }), "both a variable and a secret"),
    ] {
        let (status, message) = admin.refused(Method::PUT, "/projects/NEW/settings", Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert!(message.contains(says), "{bad}: {message}");
    }

    // A refused change changed nothing, secrets included.
    assert_eq!(dev.get("/projects/NEW/settings").await, partly);

    // A private project's settings do not exist for outsiders.
    project(&app, &admin, "SEC", true);
    assert_eq!(dev.refused(Method::GET, "/projects/SEC/settings", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(dev.refused(Method::PUT, "/projects/SEC/settings", Some(json!({}))).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_are_write_only_and_reach_only_the_runner_that_claims_a_job() {
    let hub = hub().await;
    let Hub { app, admin, dev, runner, viewer } = &hub;

    let saved = admin.put_json("/projects/WEB/settings", json!({ "env": { "APP_URL": "http://app" }, "secrets": { "PAY_KEY": SECRET, "OLD": "to-go" } })).await;
    assert_eq!(saved["secretNames"], json!(["OLD", "PAY_KEY"]));
    assert!(!saved.to_string().contains(SECRET));

    // Replacing one and removing another leaves the rest alone.
    let changed = admin.put_json("/projects/WEB/settings", json!({ "secrets": { "OLD": null, "EXTRA": "more" } })).await;
    assert_eq!(changed["secretNames"], json!(["EXTRA", "PAY_KEY"]));

    // Encrypted at rest: the value is nowhere in the database file.
    let stored: Vec<Vec<u8>> = app.read(|connection| hub_app::db::query_all(connection, "SELECT sealed FROM secret", [], |row| row.get(0))).unwrap();
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|sealed| !String::from_utf8_lossy(sealed).contains(SECRET)));

    // No read hands it out: not the settings, not the project page, not a job or its list.
    let job = dev.post("/projects/WEB/jobs", json!({})).await;
    let job_path = format!("/jobs/{}", job["id"]);

    for path in ["/projects/WEB/settings", "/projects/WEB", "/projects/WEB/jobs", job_path.as_str()] {
        for who in [admin, dev, viewer] {
            assert!(!who.get(path).await.to_string().contains(SECRET), "{path}");
        }
    }

    // The claim does, once, to the runner: plain variables and secrets apart.
    let claimed = runner.post("/jobs/claim", claim_body()).await;
    assert_eq!(claimed["settings"]["secrets"], json!({ "EXTRA": "more", "PAY_KEY": SECRET }));
    assert_eq!(claimed["settings"]["env"], json!({ "APP_URL": "http://app" }));
    assert_eq!((claimed["settings"]["repo"].as_str(), claimed["settings"]["installCommand"].as_str()), (Some("acme/shop"), Some("npm ci")));
    assert_eq!(claimed["settings"]["secretNames"], json!(["EXTRA", "PAY_KEY"]));

    // And nothing afterwards: the job as its runner reads it back carries none.
    assert!(!runner.patch(&job_path, json!({ "state": "running" })).await.to_string().contains(SECRET));
    assert!(!runner.get(&job_path).await.to_string().contains(SECRET));
}

// ---------- the life of a job ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_job_is_queued_by_a_member_with_the_projects_defaults() {
    let Hub { app, admin, dev, viewer, .. } = hub().await;
    admin.put_json("/projects/WEB/settings", json!({ "ref": "develop", "video": "retain-on-failure" })).await;

    assert_eq!(viewer.refused(Method::POST, "/projects/WEB/jobs", Some(json!({}))).await.0, StatusCode::FORBIDDEN);

    let plain = dev.post("/projects/WEB/jobs", json!({})).await;
    assert_eq!(
        (plain["state"].as_str(), plain["repo"].as_str(), plain["ref"].as_str(), plain["video"].as_str(), plain["trace"].as_str()),
        (Some("queued"), Some("acme/shop"), Some("develop"), Some("retain-on-failure"), Some("on"))
    );
    assert_eq!((plain["requestedBy"].clone(), plain["projectKey"].as_str()), (json!(dev.user.id), Some("WEB")));
    assert_eq!(
        (plain["filter"].clone(), plain["commit"].clone(), plain["runnerName"].clone(), plain["runId"].clone(), plain["startedAt"].clone()),
        (Value::Null, Value::Null, Value::Null, Value::Null, Value::Null)
    );
    assert_eq!((plain["cancelRequested"].clone(), plain["step"].clone(), plain["error"].clone()), (json!(false), Value::Null, Value::Null));

    let chosen = dev.post("/projects/WEB/jobs", json!({ "ref": "fix/WEB-11", "filter": " tests/cart.e2e.ts ", "video": "on", "trace": "off", "taskId": 7 })).await;
    assert_eq!(
        (chosen["ref"].as_str(), chosen["filter"].as_str(), chosen["video"].as_str(), chosen["trace"].as_str()),
        (Some("fix/WEB-11"), Some("tests/cart.e2e.ts"), Some("on"), Some("off"))
    );
    // The scheduled task that asked is remembered with the job.
    let task: Option<Id> = app.read(|connection| Ok(connection.query_row("SELECT task_id FROM e2e_job WHERE id = ?1", [chosen["id"].as_i64()], |row| row.get(0))?)).unwrap();
    assert_eq!(task, Some(7));

    assert_eq!(dev.refused(Method::POST, "/projects/WEB/jobs", Some(json!({ "ref": "-x" }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(dev.refused(Method::POST, "/projects/WEB/jobs", Some(json!({ "video": "always" }))).await.0, StatusCode::BAD_REQUEST);

    // Newest first, and as many as asked for.
    let listed = viewer.get("/projects/WEB/jobs").await;
    assert_eq!(listed.as_array().map(|jobs| jobs.iter().map(|job| job["id"].clone()).collect::<Vec<_>>()), Some(vec![chosen["id"].clone(), plain["id"].clone()]));
    assert_eq!(viewer.get("/projects/WEB/jobs?limit=1").await.as_array().map(Vec::len), Some(1));
    assert_eq!(job_events(&app), vec!["e2e.job", "e2e.job"]);

    // No repository, no jobs: the answer says where to set one.
    project(&app, &admin, "BARE", false);
    let (status, message) = dev.refused(Method::POST, "/projects/BARE/jobs", Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("no repository"), "{message}");

    // The function scheduled tasks call does the same as the route.
    let by_task = app.write(dev.user.actor(), |tx| jobs::create(tx, &dev.user, "WEB", &jobs::NewJob { task_id: Some(9), ..Default::default() })).unwrap();
    assert_eq!((by_task.state.as_str(), by_task.git_ref.as_str()), ("queued", "develop"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runner_claims_the_oldest_job_it_may_run() {
    let Hub { app, admin, dev, runner, viewer } = hub().await;
    project(&app, &admin, "SEC", true);
    admin.put_json("/projects/SEC/settings", json!({ "repo": "acme/vault" })).await;

    let secret_job = admin.post("/projects/SEC/jobs", json!({})).await;
    let first = dev.post("/projects/WEB/jobs", json!({})).await;
    let second = dev.post("/projects/WEB/jobs", json!({})).await;

    assert_eq!(runner.refused(Method::POST, "/jobs/claim", Some(json!({ "runnerId": " " }))).await.0, StatusCode::BAD_REQUEST);
    // Somebody who can only view runs nothing; a runner asked to work for another project waits.
    assert_eq!(viewer.post("/jobs/claim", json!({ "runnerId": "wandas-laptop" })).await, Value::Null);
    assert_eq!(runner.post("/jobs/claim", json!({ "runnerId": "r", "projects": ["OTHER"] })).await, Value::Null);

    // The private project's job is older, but this runner's person is not in that project.
    let claimed = runner.post("/jobs/claim", json!({ "runnerId": "runner-1", "runnerName": "Cloud runner", "projects": ["web"] })).await;
    let job = &claimed["job"];
    assert_eq!((job["id"].clone(), job["state"].as_str(), job["runnerName"].as_str()), (first["id"].clone(), Some("claimed"), Some("Cloud runner")));
    assert!(job["startedAt"].is_string());
    assert_eq!(claimed["settings"]["repo"], "acme/shop");

    assert_eq!(runner.post("/jobs/claim", claim_body()).await["job"]["id"], second["id"]);
    assert_eq!(runner.post("/jobs/claim", claim_body()).await, Value::Null);

    // The administrator's own runner takes the private one.
    assert_eq!(admin.post("/jobs/claim", json!({ "runnerId": "adas-runner" })).await["job"]["id"], secret_job["id"]);
    assert_eq!(dev.refused(Method::GET, &format!("/jobs/{}", secret_job["id"]), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(dev.refused(Method::GET, "/projects/SEC/jobs", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claimed_job_runs_reports_and_succeeds_and_only_its_runner_writes_to_it() {
    let hub = hub().await;
    let Hub { app, admin, dev, runner, viewer } = &hub;
    let id = claimed_job(&hub).await;
    let path = format!("/jobs/{id}");

    // Nobody but the runner that claimed it writes to it: not a member, not an administrator.
    for other in [dev, admin] {
        assert_eq!(other.refused(Method::PATCH, &path, Some(json!({ "state": "running" }))).await.0, StatusCode::FORBIDDEN);
        assert_eq!(other.refused(Method::POST, &format!("{path}/log"), Some(json!({ "lines": ["x"] }))).await.0, StatusCode::FORBIDDEN);
        assert_eq!(other.refused(Method::POST, &format!("{path}/finish"), Some(json!({}))).await.0, StatusCode::FORBIDDEN);
        let (status, _) = other.send_raw(Method::POST, &format!("/projects/WEB/runs?job={id}"), Some("application/json"), REAL_REPORT.as_bytes().to_vec()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    assert_eq!(viewer.refused(Method::PATCH, &path, Some(json!({}))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(runner.refused(Method::PATCH, "/jobs/9999", Some(json!({}))).await.0, StatusCode::NOT_FOUND);

    let cloning = runner.patch(&path, json!({ "state": "running", "step": "cloning" })).await;
    assert_eq!((cloning["state"].as_str(), cloning["step"].as_str(), cloning["cancelRequested"].clone()), (Some("running"), Some("cloning"), json!(false)));
    let installing = runner.patch(&path, json!({ "step": "installing", "commit": "0123456789abcdef0123456789abcdef01234567" })).await;
    assert_eq!((installing["step"].as_str(), installing["commit"].as_str()), (Some("installing"), Some("0123456789abcdef0123456789abcdef01234567")));
    assert_eq!(runner.refused(Method::PATCH, &path, Some(json!({ "state": "succeeded" }))).await.0, StatusCode::BAD_REQUEST);

    // The report, posted as always but naming the job, ties the run to it.
    let posted = post_report(runner, &format!("/projects/WEB/runs?job={id}"), REAL_REPORT).await;
    let run_id = posted["run"]["id"].clone();
    assert_eq!(viewer.get(&path).await["job"]["runId"], run_id);
    assert_eq!(viewer.get(&format!("/runs/{run_id}")).await["jobId"], json!(id));

    let done = runner.post(&format!("{path}/finish"), json!({})).await;
    assert_eq!((done["state"].as_str(), done["error"].clone(), done["step"].clone()), (Some("succeeded"), Value::Null, Value::Null));
    assert!(done["finishedAt"].is_string());

    // Finishing again changes nothing; other writes are refused once a job has ended.
    assert_eq!(runner.post(&format!("{path}/finish"), json!({ "error": "too late" })).await, done);
    assert_eq!(runner.refused(Method::PATCH, &path, Some(json!({ "step": "more" }))).await.0, StatusCode::CONFLICT);
    assert_eq!(runner.refused(Method::POST, &format!("{path}/log"), Some(json!({ "lines": ["more"] }))).await.0, StatusCode::CONFLICT);

    // Every change announced itself with the job and its state.
    let states: Vec<String> = app
        .read(|connection| hub_app::db::query_all(connection, "SELECT payload FROM event WHERE kind = 'e2e.job' ORDER BY id", [], |row| row.get::<_, String>(0)))
        .unwrap()
        .iter()
        .map(|payload| serde_json::from_str::<Value>(payload).unwrap())
        .inspect(|payload| assert_eq!(payload["jobId"], json!(id)))
        .map(|payload| payload["state"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(states, vec!["queued", "claimed", "running", "running", "running", "succeeded"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_can_only_be_tied_to_a_job_of_its_project() {
    let hub = hub().await;
    let Hub { app, admin, runner, .. } = &hub;
    project(app, admin, "API", false);
    let id = claimed_job(&hub).await;

    let (status, message) = runner.send_raw(Method::POST, &format!("/projects/API/runs?job={id}"), Some("application/json"), REAL_REPORT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert_eq!(runner.get("/projects/API").await["runs"], json!([]));

    let (status, _) = runner.send_raw(Method::POST, "/projects/WEB/runs?job=9999", Some("application/json"), REAL_REPORT.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_without_a_report_or_with_an_error_fails() {
    let hub = hub().await;
    let Hub { runner, viewer, .. } = &hub;

    // Finished with nothing to show.
    let empty = claimed_job(&hub).await;
    let done = runner.post(&format!("/jobs/{empty}/finish"), json!({})).await;
    assert_eq!((done["state"].as_str(), done["error"].as_str()), (Some("failed"), Some("the runner finished without sending a report")));

    // Finished with a reason, although a report had arrived: the reason decides.
    let broken = claimed_job(&hub).await;
    post_report(runner, &format!("/projects/WEB/runs?job={broken}"), REAL_REPORT).await;
    let done = runner.post(&format!("/jobs/{broken}/finish"), json!({ "error": "uploading the videos failed: 507" })).await;
    assert_eq!((done["state"].as_str(), done["error"].as_str()), (Some("failed"), Some("uploading the videos failed: 507")));
    assert!(done["runId"].is_i64());

    // Given up before the suite ran.
    let never = claimed_job(&hub).await;
    let failed = runner.patch(&format!("/jobs/{never}"), json!({ "state": "failed", "error": "git clone: repository not found" })).await;
    assert_eq!((failed["state"].as_str(), failed["error"].as_str()), (Some("failed"), Some("git clone: repository not found")));
    assert!(failed["finishedAt"].is_string());
    assert_eq!(viewer.get(&format!("/jobs/{never}")).await["job"], failed);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_ends_a_queued_job_at_once_and_a_running_one_when_its_runner_looks() {
    let hub = hub().await;
    let Hub { dev, runner, viewer, .. } = &hub;

    let queued = dev.post("/projects/WEB/jobs", json!({})).await;
    let queued_path = format!("/jobs/{}", queued["id"]);
    assert_eq!(viewer.refused(Method::POST, &format!("{queued_path}/cancel"), None).await.0, StatusCode::FORBIDDEN);
    let cancelled = dev.post(&format!("{queued_path}/cancel"), json!({})).await;
    assert_eq!((cancelled["state"].as_str(), cancelled["runnerName"].clone()), (Some("cancelled"), Value::Null));
    assert!(cancelled["finishedAt"].is_string());
    // A cancelled job is no longer there to claim.
    assert_eq!(runner.post("/jobs/claim", claim_body()).await, Value::Null);

    // One a runner holds is only marked; it ends when the runner next writes to it.
    let held = claimed_job(&hub).await;
    let path = format!("/jobs/{held}");
    runner.patch(&path, json!({ "state": "running", "step": "running" })).await;
    let marked = dev.post(&format!("{path}/cancel"), json!({})).await;
    assert_eq!((marked["state"].as_str(), marked["cancelRequested"].clone()), (Some("running"), json!(true)));
    let seen = runner.patch(&path, json!({ "step": "running" })).await;
    assert_eq!((seen["state"].as_str(), seen["cancelRequested"].clone(), seen["step"].clone()), (Some("cancelled"), json!(true), Value::Null));
    // The runner tidies up and finishes: the job stays cancelled.
    assert_eq!(runner.post(&format!("{path}/finish"), json!({ "error": "stopped" })).await["state"], "cancelled");

    // A runner that goes straight to finish after a cancel ends the job the same way.
    let direct = claimed_job(&hub).await;
    dev.post(&format!("/jobs/{direct}/cancel"), json!({})).await;
    assert_eq!(runner.post(&format!("/jobs/{direct}/finish"), json!({})).await["state"], "cancelled");

    // Cancelling what has ended changes nothing.
    assert_eq!(dev.post(&format!("/jobs/{direct}/cancel"), json!({})).await["state"], "cancelled");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_whose_runner_went_quiet_fails() {
    let hub = hub().await;
    let Hub { app, dev, runner, viewer, .. } = &hub;
    let quiet = claimed_job(&hub).await;
    let alive = claimed_job(&hub).await;
    let waiting = dev.post("/projects/WEB/jobs", json!({})).await;
    runner.patch(&format!("/jobs/{quiet}"), json!({ "state": "running", "step": "running" })).await;

    let heard = |job: Id, minutes: i64| {
        let at = (chrono::Utc::now() - chrono::Duration::minutes(minutes)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        app.db.write(|tx| Ok(tx.execute("UPDATE e2e_job SET heard_at = ?1, created_at = ?1 WHERE id = ?2", (at, job))?)).unwrap();
    };
    heard(quiet, 11);
    heard(alive, 9);
    // A job nobody has claimed waits as long as it takes.
    heard(waiting["id"].as_i64().unwrap(), 600);

    let failed = viewer.get(&format!("/jobs/{quiet}")).await;
    assert_eq!((failed["job"]["state"].as_str(), failed["job"]["error"].as_str()), (Some("failed"), Some("the runner stopped answering")));
    assert!(failed["job"]["finishedAt"].is_string());
    assert_eq!(viewer.get(&format!("/jobs/{alive}")).await["job"]["state"], "claimed");
    assert_eq!(viewer.get(&format!("/jobs/{}", waiting["id"])).await["job"]["state"], "queued");
    assert_eq!(job_events(app).last().map(String::as_str), Some("e2e.job"));

    // The runner comes back to a job that is over, and is told so.
    assert_eq!(runner.refused(Method::PATCH, &format!("/jobs/{quiet}"), Some(json!({ "step": "uploading" }))).await.0, StatusCode::CONFLICT);

    // Any word from the runner counts: a log line keeps a job alive.
    heard(alive, 11);
    jobs::fail_stale(app).unwrap();
    assert_eq!(viewer.get(&format!("/jobs/{alive}")).await["job"]["state"], "failed");
}

// ---------- runners ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_job_can_be_sent_to_one_runner_and_is_offered_to_no_other() {
    let Hub { app, admin, dev, runner, .. } = hub().await;
    project(&app, &admin, "API", false);
    admin.put_json("/projects/API/settings", json!({ "repo": "acme/api" })).await;

    // Asking for a job is how a runner says it is there: a service of the team, and Daria's computer.
    let cloud = json!({ "runnerId": "cloud-1", "runnerName": "Cloud runner", "interactive": false, "version": "0.3.1", "projects": ["WEB"] });
    let laptop = json!({ "runnerId": "laptop-daria", "runnerName": "Daria's MacBook", "pinnedOnly": true });
    assert_eq!(runner.post("/jobs/claim", cloud.clone()).await, Value::Null);
    assert_eq!(dev.post("/jobs/claim", laptop.clone()).await, Value::Null);

    // An id belongs to the person whose runner used it first.
    let (status, message) = admin.refused(Method::POST, "/jobs/claim", Some(cloud.clone())).await;
    assert_eq!((status, message.as_str()), (StatusCode::CONFLICT, "That runner id belongs to somebody else's runner"));

    // Who may be named: a runner this module knows, that works for the project; somebody's
    // computer only by its owner.
    let named = |runner: &str| json!({ "onRunner": runner });
    assert_eq!(
        dev.refused(Method::POST, "/projects/WEB/jobs", Some(named("never-seen"))).await,
        (StatusCode::BAD_REQUEST, "That runner has not connected to this server".to_string())
    );
    assert_eq!(
        runner.refused(Method::POST, "/projects/WEB/jobs", Some(named("laptop-daria"))).await,
        (StatusCode::FORBIDDEN, "That runner is somebody's computer; only its owner sends work to it by name".to_string())
    );
    assert_eq!(
        dev.refused(Method::POST, "/projects/API/jobs", Some(named("cloud-1"))).await,
        (StatusCode::BAD_REQUEST, "That runner does not work for this project".to_string())
    );

    let open = dev.post("/projects/WEB/jobs", json!({})).await;
    assert_eq!((open["onRunner"].clone(), open["onRunnerName"].clone()), (Value::Null, Value::Null));
    // A service takes work by name from anyone who may start it; the job waits, queued.
    let for_cloud = dev.post("/projects/WEB/jobs", named("cloud-1")).await;
    assert_eq!(
        (for_cloud["state"].as_str(), for_cloud["onRunner"].as_str(), for_cloud["onRunnerName"].as_str()),
        (Some("queued"), Some("cloud-1"), Some("Cloud runner"))
    );
    let for_laptop = dev.post("/projects/WEB/jobs", named("laptop-daria")).await;

    // Another service is offered neither pinned job, only the one anybody may take.
    let other = json!({ "runnerId": "cloud-2", "runnerName": "Second runner", "interactive": false });
    assert_eq!(admin.post("/jobs/claim", other.clone()).await["job"]["id"], open["id"]);
    assert_eq!(admin.post("/jobs/claim", other).await, Value::Null);

    // A runner takes what was sent to it before an older job from the queue.
    let older = dev.post("/projects/WEB/jobs", json!({})).await;
    let newer_pinned = dev.post("/projects/WEB/jobs", named("cloud-1")).await;
    assert_eq!(runner.post("/jobs/claim", cloud.clone()).await["job"]["id"], for_cloud["id"]);
    assert_eq!(runner.post("/jobs/claim", cloud.clone()).await["job"]["id"], newer_pinned["id"]);
    assert_eq!(runner.post("/jobs/claim", cloud.clone()).await["job"]["id"], older["id"]);
    assert_eq!(runner.post("/jobs/claim", cloud).await, Value::Null);

    // The computer that takes only what is sent to it gets its own job, and nothing from the queue.
    dev.post("/projects/WEB/jobs", json!({})).await;
    let claimed = dev.post("/jobs/claim", laptop.clone()).await;
    assert_eq!((claimed["job"]["id"].clone(), claimed["job"]["runnerName"].as_str()), (for_laptop["id"].clone(), Some("Daria's MacBook")));
    assert_eq!(dev.post("/jobs/claim", laptop).await, Value::Null);

    // The function scheduled tasks call pins the same way.
    let pinned = jobs::NewJob { on_runner: Some("cloud-1".to_string()), ..Default::default() };
    let by_task = app.write(dev.user.actor(), |tx| jobs::create(tx, &dev.user, "WEB", &pinned)).unwrap();
    assert_eq!((by_task.on_runner.as_deref(), by_task.on_runner_name.as_deref()), (Some("cloud-1"), Some("Cloud runner")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_runner_works_on_anyones_job_with_that_persons_own_token() {
    let Hub { app, admin, dev, runner, .. } = hub().await;
    // People store their own keys in a browser session, not with an API token.
    let store = |who: &Client, value: &str| {
        let in_person = hub_app::AuthUser { via_token: false, ..who.user.clone() };

        app.db.write(|tx| hub_app::services::personal_keys::set(&app, tx, &in_person, "github_token", value)).unwrap();
    };
    store(&dev, "github_pat_darias-own");
    store(&admin, "github_pat_the-administrators");

    // Only whoever operates the instance operates its shared runners, and a person's computer is never one.
    let shared = json!({ "runnerId": "cloud-shared", "runnerName": "Team runner", "interactive": false, "shared": true });
    assert_eq!(
        runner.refused(Method::POST, "/jobs/claim", Some(shared.clone())).await,
        (StatusCode::FORBIDDEN, "Only an administrator's runner can be shared".to_string())
    );
    let at_a_desk = json!({ "runnerId": "adas-laptop", "shared": true });
    assert_eq!(admin.refused(Method::POST, "/jobs/claim", Some(at_a_desk)).await.0, StatusCode::FORBIDDEN);
    // The refused runners were not taken note of.
    assert!(app.read(|connection| E2eModule.runners(connection, &admin.user.through("e2e"), chrono::Utc::now())).unwrap().is_empty());

    // The shared runner takes Daria's job, and is handed Daria's token for it: not its operator's.
    let darias = dev.post("/projects/WEB/jobs", json!({})).await;
    let claimed = admin.post("/jobs/claim", shared.clone()).await;
    assert_eq!(claimed["job"]["id"], darias["id"]);
    assert_eq!(claimed["credentials"], json!({ "githubToken": "github_pat_darias-own" }));
    // It is in the claim's answer and nowhere else: not on the job as anybody reads it later.
    assert!(!admin.get(&format!("/jobs/{}", darias["id"])).await.to_string().contains("github_pat"));
    assert!(!admin.get("/projects/WEB/jobs").await.to_string().contains("github_pat"));

    // Somebody without a stored token: the credentials are there, and empty.
    runner.post("/projects/WEB/jobs", json!({})).await;
    assert_eq!(admin.post("/jobs/claim", shared.clone()).await["credentials"], json!({}));

    // A shared runner can be named by anyone who may start jobs, and shows as shared in the list.
    let named = dev.post("/projects/WEB/jobs", json!({ "onRunner": "cloud-shared" })).await;
    assert_eq!(named["onRunnerName"], "Team runner");
    let listed = app.read(|connection| E2eModule.runners(connection, &dev.user.through("e2e"), chrono::Utc::now())).unwrap();
    assert_eq!(listed.iter().map(|runner| (runner.id.as_str(), runner.shared, runner.interactive)).collect::<Vec<_>>(), vec![("cloud-shared", true, false)]);

    // A runner that is not shared works with what it has: no credentials at all, even its owner's.
    admin.post("/projects/WEB/jobs", json!({})).await;
    let own = admin.post("/jobs/claim", json!({ "runnerId": "adas-own-runner", "interactive": false })).await;
    assert!(own["job"]["id"].is_i64());
    assert!(own.get("credentials").is_none(), "{own}");

    // The key is declared, so it shows under "My keys" whichever modules are on.
    let keys = E2eModule.personal_keys();
    assert_eq!(keys.iter().map(|key| (key.name.as_str(), key.label.as_str())).collect::<Vec<_>>(), vec![("github_token", "GitHub token")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_of_runners_shows_who_is_connected_and_what_they_carry_out() {
    let Hub { app, admin, dev, runner, viewer } = hub().await;
    project(&app, &admin, "SEC", true);
    admin.put_json("/projects/SEC/settings", json!({ "repo": "acme/vault" })).await;

    let open = dev.post("/projects/WEB/jobs", json!({ "ref": "fix/WEB-11" })).await;
    let secret = admin.post("/projects/SEC/jobs", json!({})).await;
    runner.post("/jobs/claim", json!({ "runnerId": "cloud-1", "runnerName": "Cloud runner", "interactive": false, "version": "0.3.1", "projects": ["WEB"] })).await;
    admin.post("/jobs/claim", json!({ "runnerId": "cloud-2", "runnerName": "Vault runner", "interactive": false, "projects": ["SEC"] })).await;
    dev.post("/jobs/claim", json!({ "runnerId": "laptop-daria", "runnerName": "Daria's MacBook", "pinnedOnly": true })).await;

    let listed = |who: &Client, now: chrono::DateTime<chrono::Utc>| {
        let viewer = who.user.through("e2e");

        app.read(|connection| E2eModule.runners(connection, &viewer, now)).unwrap()
    };
    let now = chrono::Utc::now();

    // Everybody sees who is connected; each runner with the one kind of work this module gives out.
    let seen = listed(&viewer, now);
    assert_eq!(seen.iter().map(|runner| runner.id.as_str()).collect::<Vec<_>>(), vec!["cloud-1", "laptop-daria", "cloud-2"], "by name");
    let cloud = &seen[0];
    assert_eq!((cloud.name.as_str(), cloud.user_id, cloud.interactive, cloud.online, cloud.version.as_deref()), ("Cloud runner", runner.user.id, false, true, Some("0.3.1")));
    assert_eq!(cloud.work.len(), 1);
    assert_eq!((cloud.work[0].module.as_str(), cloud.work[0].what.clone(), cloud.work[0].projects.clone()), ("e2e", vec!["tests".to_string()], Some(vec!["WEB".to_string()])));
    assert_eq!((cloud.work[0].pinned_only, cloud.work[0].online), (false, true));
    // What it is carrying out, with the page that shows it.
    assert_eq!(
        cloud.work[0].active.iter().map(|active| (active.title.clone(), active.href.clone())).collect::<Vec<_>>(),
        vec![("WEB · tests · fix/WEB-11".to_string(), format!("/p/WEB/tests/jobs/{}", open["id"]))]
    );
    let laptop = &seen[1];
    assert_eq!((laptop.interactive, laptop.version.clone(), laptop.work[0].pinned_only, laptop.work[0].projects.clone()), (true, None, true, None));
    assert!(laptop.work[0].active.is_empty());
    // A job of a project the viewer cannot see is not named, though its runner is listed.
    assert!(seen[2].work[0].active.is_empty());
    assert_eq!(listed(&admin, now)[2].work[0].active[0].href, format!("/p/SEC/tests/jobs/{}", secret["id"]));

    // Two minutes of silence: other people's runners leave the list, one's own stay, offline.
    let later = now + chrono::Duration::minutes(3);
    assert!(listed(&viewer, later).is_empty());
    let own = listed(&dev, later);
    assert_eq!(own.iter().map(|runner| (runner.id.as_str(), runner.online, runner.work[0].online)).collect::<Vec<_>>(), vec![("laptop-daria", false, false)]);
}

// ---------- the log ----------

#[tokio::test(flavor = "multi_thread")]
async fn the_log_keeps_its_newest_lines_and_is_read_in_increments() {
    let hub = hub().await;
    let Hub { runner, viewer, .. } = &hub;
    let id = claimed_job(&hub).await;
    let path = format!("/jobs/{id}");

    let first = runner.post(&format!("{path}/log"), json!({ "lines": ["cloning acme/shop", "\u{1b}[32m✓\u{1b}[0m done", "x".repeat(2500)] })).await;
    assert_eq!(first, json!({ "seq": 3 }));

    let detail = viewer.get(&path).await;
    assert_eq!(detail["logTruncated"], false);
    assert_eq!(detail["log"][0], json!({ "seq": 1, "text": "cloning acme/shop" }));
    // Terminal colours are dropped; a line is cut at 2000 characters.
    assert_eq!(detail["log"][1]["text"], "✓ done");
    assert_eq!(detail["log"][2]["text"].as_str().map(|text| text.chars().count()), Some(2000));

    // 2500 more lines, in batches as a runner sends them.
    for batch in 0..5 {
        let lines: Vec<String> = (0..500).map(|line| format!("line {}", 4 + batch * 500 + line)).collect();
        runner.post(&format!("{path}/log"), json!({ "lines": lines })).await;
    }

    let detail = viewer.get(&path).await;
    let log = detail["log"].as_array().unwrap();
    assert_eq!(log.len(), jobs::LOG_LINES_KEPT as usize);
    assert_eq!((log[0]["seq"].clone(), log[0]["text"].as_str()), (json!(504), Some("line 504")));
    assert_eq!(log[1999]["seq"], json!(2503));
    assert_eq!(detail["logTruncated"], true);

    // A page that has read up to a line asks for what came after it.
    let more = viewer.get(&format!("{path}?after=2500")).await;
    assert_eq!(more["log"], json!([{ "seq": 2501, "text": "line 2501" }, { "seq": 2502, "text": "line 2502" }, { "seq": 2503, "text": "line 2503" }]));
    assert_eq!(viewer.get(&format!("{path}?after=2503")).await["log"], json!([]));
    assert_eq!(runner.post(&format!("{path}/log"), json!({ "lines": [] })).await, json!({ "seq": 2503 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn new_log_lines_are_announced_at_most_once_a_second_and_never_left_untold() {
    let hub = hub().await;
    let Hub { app, runner, .. } = &hub;
    let id = claimed_job(&hub).await;
    let announced = || {
        app.read(|connection| hub_app::db::query_all(connection, "SELECT payload FROM event WHERE kind = 'e2e.job.log' ORDER BY id", [], |row| row.get::<_, String>(0)))
            .unwrap()
            .iter()
            .map(|payload| serde_json::from_str::<Value>(payload).unwrap())
            .collect::<Vec<_>>()
    };

    for line in 0..5 {
        runner.post(&format!("/jobs/{id}/log"), json!({ "lines": [format!("line {line}")] })).await;
    }

    // Five posts within a moment: pages heard once, of the first.
    assert_eq!(announced(), vec![json!({ "jobId": id, "seq": 1 })]);

    // The rest is told once the second has passed, though the runner sends nothing more.
    tokio::time::sleep(std::time::Duration::from_millis(1400)).await;
    assert_eq!(announced(), vec![json!({ "jobId": id, "seq": 1 }), json!({ "jobId": id, "seq": 5 })]);
}

// ---------- artifacts ----------

/// A run in WEB (the real report) and in the private SEC, posted by their members.
async fn runs(hub: &Hub) -> (Value, Value) {
    project(&hub.app, &hub.admin, "SEC", true);
    let open = post_report(&hub.runner, "/projects/WEB/runs", REAL_REPORT).await["run"]["id"].clone();
    let private = post_report(&hub.admin, "/projects/SEC/runs", REAL_REPORT).await["run"]["id"].clone();

    (open, private)
}

#[tokio::test(flavor = "multi_thread")]
async fn artifacts_are_uploaded_by_path_and_kept_under_names_of_our_own() {
    let hub = hub().await;
    let Hub { app, runner, viewer, dev, .. } = &hub;
    let (run, private_run) = runs(&hub).await;
    let png = b"\x89PNG\r\n\x1a\n not really a picture".to_vec();

    assert_eq!(upload(viewer, &run, "a/shot.png", &png).await.0, StatusCode::FORBIDDEN);
    assert_eq!(upload(dev, &private_run, "a/shot.png", &png).await.0, StatusCode::NOT_FOUND);
    assert_eq!(upload(runner, &json!(9999), "a/shot.png", &png).await.0, StatusCode::NOT_FOUND);

    // The path is the report's: relative, never one that climbs out or names a place on disk.
    for bad in ["", "../../team-hub.db", "a/../../secret.key", "/etc/passwd", "a//b.png", "a\\b.png", "C:/x.png"] {
        assert_eq!(upload(runner, &run, bad, &png).await.0, StatusCode::BAD_REQUEST, "{bad:?}");
    }

    let (status, _) = runner.send_raw(Method::PUT, &format!("/runs/{run}/artifacts"), Some("image/png"), png.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(run_files(app, &run), Vec::<String>::new());

    let path = "checkout-web-4-132be6b506b3b9cb/attempt-2/screenshots/003-failure.png";
    let (status, first) = upload(runner, &run, path, &png).await;
    assert_eq!(status, StatusCode::OK, "{first}");

    // On disk: a folder per run, a name made here. Nothing of the path the client sent.
    let files = run_files(app, &run);
    assert_eq!(files.len(), 1);
    assert!(files[0].len() == 24 && files[0].chars().all(|character| character.is_ascii_hexdigit()), "{files:?}");

    // The same path again replaces the file: a new artifact, the old one gone.
    let (_, second) = upload(runner, &run, path, b"second").await;
    assert_ne!(first["id"], second["id"]);
    assert_eq!(viewer.request(Method::GET, &format!("/artifacts/{}", first["id"]), &[], Vec::new()).await.0, StatusCode::NOT_FOUND);
    assert_eq!(viewer.request(Method::GET, &format!("/artifacts/{}", second["id"]), &[], Vec::new()).await.2, b"second");
    assert_eq!(run_files(app, &run).len(), 1);

    // An empty file is a file too: a report may name a log nothing was written to.
    assert_eq!(upload(runner, &run, "a/empty.log", b"").await.0, StatusCode::OK);
    assert_eq!(events(app).iter().filter(|(kind, _)| kind == "e2e.artifact").count(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_artifact_and_a_run_have_a_size_limit() {
    let hub = hub().await;
    let Hub { app, runner, .. } = &hub;
    let (run, _) = runs(&hub).await;

    // A body that says it is larger than 200 MB is refused before a byte is read.
    let declared = (artifacts::MAX_ARTIFACT_BYTES + 1).to_string();
    let (status, _, body) = runner
        .request(Method::PUT, &format!("/runs/{run}/artifacts?path=a/video.webm"), &[("content-length", declared.as_str())], b"tiny".to_vec())
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{}", String::from_utf8_lossy(&body));
    assert!(String::from_utf8_lossy(&body).contains("200 MB"));

    // A run that already holds almost 2 GB takes nothing that would go over.
    let almost = (artifacts::MAX_RUN_BYTES - 10) as i64;
    app.db
        .write(|tx| {
            Ok(tx.execute(
                "INSERT INTO e2e_artifact (run_id, path, kind, size, stored, created_at) VALUES (?1, 'a/huge.webm', 'video', ?2, 'nothing-on-disk', '2026-11-13T00:00:00.000Z')",
                (run.as_i64(), almost),
            )?)
        })
        .unwrap();
    let (status, answer) = upload(runner, &run, "a/more.webm", b"eleven bytes").await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{answer}");
    assert!(answer["error"]["message"].as_str().unwrap().contains("2 GB"));
    assert_eq!(upload(runner, &run, "a/fits.webm", b"ten bytes!").await.0, StatusCode::OK);
    // The refused upload left nothing behind.
    assert_eq!(run_files(app, &run).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn artifacts_are_served_whole_or_by_range_with_safe_headers() {
    let hub = hub().await;
    let Hub { runner, viewer, .. } = &hub;
    let (run, _) = runs(&hub).await;
    let video: Vec<u8> = (0..1000u32).map(|byte| (byte % 251) as u8).collect();
    let (_, uploaded) = upload(runner, &run, "t-132be6b506b3b9cb/attempt-1/video/video.webm", &video).await;
    let path = format!("/artifacts/{}", uploaded["id"]);
    let header = |headers: &axum::http::HeaderMap, name: &str| headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or("").to_string();

    let (status, headers, body) = viewer.request(Method::GET, &path, &[], Vec::new()).await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, video.as_slice()));
    assert_eq!(header(&headers, "content-type"), "video/webm");
    assert_eq!(header(&headers, "content-length"), "1000");
    assert_eq!(header(&headers, "accept-ranges"), "bytes");
    assert_eq!(header(&headers, "content-disposition"), "inline; filename=\"video.webm\"");
    assert_eq!(header(&headers, "x-content-type-options"), "nosniff");
    assert!(header(&headers, "content-security-policy").contains("sandbox"));

    // bytes=a-b
    let (status, headers, body) = viewer.request(Method::GET, &path, &[("range", "bytes=100-199")], Vec::new()).await;
    assert_eq!((status, body.as_slice()), (StatusCode::PARTIAL_CONTENT, &video[100..200]));
    assert_eq!((header(&headers, "content-range"), header(&headers, "content-length")), ("bytes 100-199/1000".to_string(), "100".to_string()));
    assert_eq!(header(&headers, "content-type"), "video/webm");

    // bytes=a- : from there to the end, which is how a player asks first (bytes=0-) and seeks.
    let (status, headers, body) = viewer.request(Method::GET, &path, &[("range", "bytes=0-")], Vec::new()).await;
    assert_eq!((status, body.len(), header(&headers, "content-range")), (StatusCode::PARTIAL_CONTENT, 1000, "bytes 0-999/1000".to_string()));
    let (status, headers, body) = viewer.request(Method::GET, &path, &[("range", "bytes=900-")], Vec::new()).await;
    assert_eq!((status, body.as_slice(), header(&headers, "content-range")), (StatusCode::PARTIAL_CONTENT, &video[900..], "bytes 900-999/1000".to_string()));

    // An end past the file is cut to it.
    let (status, headers, body) = viewer.request(Method::GET, &path, &[("range", "bytes=990-5000")], Vec::new()).await;
    assert_eq!((status, body.as_slice(), header(&headers, "content-range")), (StatusCode::PARTIAL_CONTENT, &video[990..], "bytes 990-999/1000".to_string()));

    // bytes=-n : the last n bytes.
    let (status, headers, body) = viewer.request(Method::GET, &path, &[("range", "bytes=-50")], Vec::new()).await;
    assert_eq!((status, body.as_slice(), header(&headers, "content-range")), (StatusCode::PARTIAL_CONTENT, &video[950..], "bytes 950-999/1000".to_string()));

    // A range that starts past the end cannot be satisfied, and the answer says how long the file is.
    let (status, headers, _) = viewer.request(Method::GET, &path, &[("range", "bytes=1000-")], Vec::new()).await;
    assert_eq!((status, header(&headers, "content-range")), (StatusCode::RANGE_NOT_SATISFIABLE, "bytes */1000".to_string()));
    let (status, _, _) = viewer.request(Method::GET, &path, &[("range", "bytes=5000-6000")], Vec::new()).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);

    // HEAD tells the size without the bytes.
    let (status, headers, body) = viewer.request(Method::HEAD, &path, &[], Vec::new()).await;
    assert_eq!((status, header(&headers, "content-length"), body.len()), (StatusCode::OK, "1000".to_string(), 0));

    // What is not a recording or a screenshot is a download, whatever it claims to be.
    for (name, media_type) in [("trace.md", "text/plain; charset=utf-8"), ("page.html", "application/octet-stream"), ("logo.svg", "application/octet-stream")] {
        let (_, uploaded) = upload(runner, &run, &format!("t-132be6b506b3b9cb/{name}"), b"<script>alert(1)</script>").await;
        let (status, headers, _) = viewer.request(Method::GET, &format!("/artifacts/{}", uploaded["id"]), &[], Vec::new()).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(header(&headers, "content-type"), media_type, "{name}");
        assert_eq!(header(&headers, "content-disposition"), format!("attachment; filename=\"{name}\""));
        assert_eq!(header(&headers, "x-content-type-options"), "nosniff");
    }

    let (_, shot) = upload(runner, &run, "t-132be6b506b3b9cb/attempt-1/screenshots/001 koszyk \"x\".png", b"png").await;
    let (_, headers, _) = viewer.request(Method::GET, &format!("/artifacts/{}", shot["id"]), &[], Vec::new()).await;
    assert_eq!((header(&headers, "content-type"), header(&headers, "content-disposition")), ("image/png".to_string(), "inline; filename=\"001koszykx.png\"".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn artifacts_follow_the_projects_access_and_go_with_their_run() {
    let hub = hub().await;
    let Hub { app, admin, dev, runner, viewer } = &hub;
    let (run, private_run) = runs(&hub).await;
    let (_, open) = upload(runner, &run, "a/video.webm", b"open").await;
    let (_, private) = upload(admin, &private_run, "a/video.webm", b"private").await;
    let fetch = |who: &Client, artifact: &Value| {
        let (who, path) = (who.clone(), format!("/artifacts/{}", artifact["id"]));

        async move { who.request(Method::GET, &path, &[], Vec::new()).await.0 }
    };

    assert_eq!(fetch(viewer, &open).await, StatusCode::OK);
    assert_eq!(fetch(admin, &private).await, StatusCode::OK);
    // An outsider of the private project cannot tell its files from ones that do not exist.
    assert_eq!(fetch(dev, &private).await, StatusCode::NOT_FOUND);
    assert_eq!(fetch(dev, &json!({ "id": 9999 })).await, StatusCode::NOT_FOUND);

    let mut stranger = dev.clone();
    stranger.bearer = None;
    assert_eq!(fetch(&stranger, &open).await, StatusCode::UNAUTHORIZED);

    // Removing a run removes its files from the disk.
    assert_eq!(run_files(app, &run).len(), 1);
    admin.delete(&format!("/runs/{run}")).await;
    assert_eq!(fetch(viewer, &open).await, StatusCode::NOT_FOUND);
    assert!(!app.config.data_dir.join("e2e-artifacts").join(run.to_string()).exists());
    assert_eq!(run_files(app, &private_run).len(), 1);

    // A project that goes takes its runs with it; the sweep then takes the files no run names.
    app.db.write(|tx| Ok(tx.execute("DELETE FROM e2e_run WHERE id = ?1", [private_run.as_i64()])?)).unwrap();
    assert_eq!(run_files(app, &private_run).len(), 1);
    artifacts::sweep(app).unwrap();
    assert!(!app.config.data_dir.join("e2e-artifacts").join(private_run.to_string()).exists());
}

// ---------- the session of a result ----------

fn result_titled<'a>(detail: &'a Value, title: &str) -> &'a Value {
    detail["results"].as_array().unwrap().iter().find(|result| result["title"] == title).unwrap_or_else(|| panic!("no result titled {title}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_result_remembers_its_session_from_a_real_report() {
    let hub = hub().await;
    let Hub { app, admin, dev, runner, viewer } = &hub;
    let id = claimed_job(&hub).await;
    let run = post_report(runner, &format!("/projects/WEB/runs?job={id}"), REAL_REPORT).await["run"]["id"].clone();
    let detail = viewer.get(&format!("/runs/{run}")).await;

    // The list knows what each result has to show without reading its session.
    let failed = result_titled(&detail, "Backlog > WEB-4 the backlog lists the sprint and its issues");
    assert_eq!((failed["status"].as_str(), failed["attempts"].clone()), (Some("failed"), json!(2)));
    assert_eq!((failed["videos"].clone(), failed["screenshots"].clone(), failed["hasSession"].clone()), (json!(0), json!(6), json!(true)));
    let passed = result_titled(&detail, "Board > WEB-9 the card of an issue opens the issue beside the board");
    assert_eq!((passed["screenshots"].clone(), passed["hasSession"].clone()), (json!(0), json!(true)));

    let session = viewer.get(&format!("/results/{}", failed["id"])).await;
    assert_eq!((session["run"]["id"].clone(), session["result"]["id"].clone()), (run.clone(), failed["id"].clone()));
    assert_eq!((session["jobId"].clone(), session["jobActive"].clone(), session["canEdit"].clone()), (json!(id), json!(true), json!(false)));
    assert_eq!(session["traceArtifactId"], Value::Null);

    // Both attempts, each with its steps.
    let attempts = session["attempts"].as_array().unwrap();
    assert_eq!(attempts.iter().map(|attempt| (attempt["index"].clone(), attempt["status"].as_str())).collect::<Vec<_>>(), vec![(json!(0), Some("failed")), (json!(1), Some("failed"))]);
    let last = &attempts[1];
    assert_eq!((last["startedAt"].as_str(), last["durationMs"].clone()), (Some("2026-11-13T08:06:29.420Z"), json!(3070)));
    assert_eq!(last["environment"]["browser"], "chromium 153.0.8010.12");

    // The error, with what the assertion expected and what it saw, and where.
    let error = &last["error"];
    assert_eq!((error["code"].as_str(), error["file"].as_str(), error["line"].clone()), (Some("ASSERTION_FAILED"), Some("tests/checkout.e2e.ts"), json!(13)));
    assert_eq!((error["expected"].as_str(), error["observed"].as_str()), (Some("text \"Finish the sprint now\""), Some("text \"Complete sprint\"")));
    assert_eq!(error["locator"], "getByRole(\"button\", name: \"Complete sprint\")");
    assert!(error["message"].as_str().unwrap().starts_with("expect.toHaveText failed"));

    // The steps: api, label, source line, status, when and how long.
    let steps = last["steps"].as_array().unwrap();
    assert_eq!(
        steps.iter().map(|step| (step["index"].as_i64().unwrap(), step["api"].as_str().unwrap(), step["status"].as_str().unwrap())).collect::<Vec<_>>(),
        vec![(0, "app.open", "passed"), (1, "app.screenshot", "passed"), (2, "expect.toBeVisible", "passed"), (3, "app.open", "passed"), (4, "app.screenshot", "passed"), (5, "expect.toHaveText", "failed")]
    );
    assert_eq!((steps[0]["label"].as_str(), steps[0]["file"].as_str(), steps[0]["line"].clone()), (Some("/p/WEB/backlog"), Some("tests/checkout.e2e.ts"), json!(8)));
    assert!(steps[0]["startedAt"].as_str().unwrap() >= last["startedAt"].as_str().unwrap());
    assert!(steps[0]["durationMs"].as_i64().unwrap() > 0);
    assert_eq!(steps[0]["location"], "http://localhost:8195/p/WEB/backlog");
    // What the failed assertion read while it waited, and the error it ended with.
    assert_eq!(steps[5]["events"], json!([{ "kind": "poll", "detail": "text \"Complete sprint\" (1 match) x14", "failed": true, "durationMs": steps[5]["events"][0]["durationMs"] }]));
    assert_eq!(steps[5]["error"]["code"], "ASSERTION_FAILED");

    // The files the report names: the two captures, the screen's text and the screenshot at
    // failure. None has arrived yet.
    let named = last["artifacts"].as_array().unwrap();
    assert_eq!(named.iter().map(|artifact| artifact["kind"].as_str().unwrap()).collect::<Vec<_>>(), vec!["screenshot", "screenshot", "log", "screenshot"]);
    assert!(named.iter().all(|artifact| artifact["artifactId"].is_null()));
    assert_eq!(steps[1]["artifacts"], json!([named[0]["ref"]]));
    assert_eq!((last["failure"]["url"].as_str(), last["failure"]["nodes"].clone()), (Some("http://localhost:8195/p/WEB/board"), json!(222)));
    assert_eq!((last["failure"]["screenshot"].clone(), last["failure"]["screen"].clone()), (named[3]["ref"].clone(), named[2]["ref"].clone()));

    // The runner sends the screenshot at failure and the test's trace page; the session then has them.
    let shot_path = named[3]["path"].as_str().unwrap();
    assert_eq!(shot_path, "checkout-backlog-web-4-the-backlog-lists-132be6b506b3b9cb/attempt-2/screenshots/003-failure.png");
    let (_, shot) = upload(runner, &run, shot_path, b"png bytes").await;
    let (_, trace) = upload(runner, &run, "checkout-backlog-web-4-the-backlog-lists-132be6b506b3b9cb/trace.md", b"# trace").await;
    // Another test's trace page is not this one's.
    upload(runner, &run, "board-web-9-the-card-of-an-issue-opens-1ee7d924694b8941/trace.md", b"# other").await;
    runner.post(&format!("/jobs/{id}/finish"), json!({})).await;

    let session = dev.get(&format!("/results/{}", failed["id"])).await;
    let named = session["attempts"][1]["artifacts"].as_array().unwrap();
    assert_eq!((named[3]["artifactId"].clone(), named[3]["size"].clone()), (shot["id"].clone(), json!(9)));
    assert!(named[0]["artifactId"].is_null());
    assert_eq!((session["traceArtifactId"].clone(), session["jobActive"].clone(), session["canEdit"].clone()), (trace["id"].clone(), json!(false), json!(true)));

    // A passed test: what the engine did under a step, and where the app went.
    let session = viewer.get(&format!("/results/{}", passed["id"])).await;
    let tap = &session["attempts"][0]["steps"][1];
    assert_eq!((tap["api"].as_str(), tap["events"][0]["kind"].as_str(), tap["events"][1]["kind"].as_str()), (Some("locator.tap"), Some("engine"), Some("navigation")));
    assert!(tap["events"][0]["detail"].as_str().unwrap().starts_with("tap button"));
    assert_eq!(tap["events"][1]["detail"], "navigated to http://localhost:8195/p/WEB/board?issue=WEB-9");
    assert_eq!((tap["changes"].as_array().map(Vec::len), tap["moreChanges"].as_i64().map(|more| more > 0)), (Some(12), Some(true)));
    assert_eq!(session["attempts"][0]["error"], Value::Null);
    assert_eq!(session["result"]["issues"], json!([]));

    // What the app logged, each line with the step it belongs to.
    let logged = result_titled(&detail, "Sign-in > a wrong password is refused with a message");
    let session = viewer.get(&format!("/results/{}", logged["id"])).await;
    assert_eq!(
        session["attempts"][0]["appLog"],
        json!([{ "level": "warning", "text": "POST http://localhost:8195/api/login 401 Unauthorized", "at": "2026-11-13T08:06:34.678Z", "step": 4 }])
    );

    // A skipped test never ran an attempt.
    let skipped = result_titled(&detail, "Agent example > an agent comments on an issue");
    assert_eq!(viewer.get(&format!("/results/{}", skipped["id"])).await["attempts"], json!([]));

    // Nothing of a private project's sessions for outsiders, and no result that is not there.
    project(app, admin, "SEC", true);
    let private = post_report(admin, "/projects/SEC/runs", REAL_REPORT).await["run"]["id"].clone();
    let private_result = admin.get(&format!("/runs/{private}")).await["results"][0]["id"].clone();
    assert_eq!(dev.refused(Method::GET, &format!("/results/{private_result}"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(dev.refused(Method::GET, "/results/99999", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn serial_groups_and_junit_runs_have_what_their_reports_hold() {
    let Hub { runner, viewer, .. } = hub().await;

    // A member of a serial group keeps its attempts on the group; both are in its session.
    let run = post_report(&runner, "/projects/WEB/runs", SERIAL_REPORT).await["run"]["id"].clone();
    let detail = viewer.get(&format!("/runs/{run}")).await;
    let serial = result_titled(&detail, "checkout > retains serial verdict");
    assert_eq!((serial["status"].as_str(), serial["attempts"].clone(), serial["hasSession"].clone()), (Some("failed"), json!(2), json!(true)));
    let session = viewer.get(&format!("/results/{}", serial["id"])).await;
    assert_eq!(
        session["attempts"].as_array().unwrap().iter().map(|attempt| (attempt["index"].clone(), attempt["durationMs"].clone())).collect::<Vec<_>>(),
        vec![(json!(0), json!(1250)), (json!(1), json!(0))]
    );
    let opens = result_titled(&detail, "opens app");
    assert_eq!(viewer.get(&format!("/results/{}", opens["id"])).await["tags"], json!(["smoke"]));

    // JUnit tells no steps: the result has no session, and says so.
    let xml = r#"<testsuite name="s" time="1"><testcase classname="a.test" name="adds" time="1"><failure message="boom"/></testcase></testsuite>"#;
    let run = runner.post_xml("/projects/WEB/runs", xml).await["run"]["id"].clone();
    let result = viewer.get(&format!("/runs/{run}")).await["results"][0].clone();
    assert_eq!((result["hasSession"].clone(), result["videos"].clone()), (json!(false), json!(0)));
    let session = viewer.get(&format!("/results/{}", result["id"])).await;
    assert_eq!((session["attempts"].clone(), session["result"]["message"].as_str(), session["run"]["format"].as_str()), (json!([]), Some("boom"), Some("junit")));
}

// ---------- the project's own switch ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_project_that_switched_the_module_off_has_no_jobs_settings_or_files() {
    let hub = hub().await;
    let Hub { app, admin, dev, runner, .. } = &hub;
    let run = post_report(runner, "/projects/WEB/runs", REAL_REPORT).await["run"]["id"].clone();
    let (_, artifact) = upload(runner, &run, "a/video.webm", b"bytes").await;
    let result = admin.get(&format!("/runs/{run}")).await["results"][0]["id"].clone();
    let job = dev.post("/projects/WEB/jobs", json!({})).await;

    switch_module(app, admin, "WEB", false);

    for path in ["/projects/WEB/settings".to_string(), "/projects/WEB/jobs".to_string(), format!("/jobs/{}", job["id"]), format!("/results/{result}"), format!("/artifacts/{}", artifact["id"])] {
        assert_eq!(admin.refused(Method::GET, &path, None).await.0, StatusCode::NOT_FOUND, "{path}");
    }

    assert_eq!(dev.refused(Method::POST, "/projects/WEB/jobs", Some(json!({}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(upload(runner, &run, "a/more.webm", b"bytes").await.0, StatusCode::NOT_FOUND);
    // The queued job is not handed to a runner while the project wants no tests.
    assert_eq!(runner.post("/jobs/claim", claim_body()).await, Value::Null);

    switch_module(app, admin, "WEB", true);
    assert_eq!(runner.post("/jobs/claim", claim_body()).await["job"]["id"], job["id"]);
    assert_eq!(admin.get("/projects/WEB/settings").await["repo"], "acme/shop");
}
