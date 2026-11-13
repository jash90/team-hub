//! The desktop app as a runner for both kinds of work (docs/runners.md): it takes the test
//! jobs sent to it by name and runs them on the computer, shows the same runners as the
//! web, and hands work to another runner when asked to.

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hub_desktop_core::commands;
use hub_desktop_core::config::Paths;
use hub_desktop_core::model::{Delegation, NewTestJob, SandboxState, ThreadKind};
use hub_desktop_core::secrets::MemoryKeychain;
use hub_desktop_core::{policy, Desktop, OpenOptions};
use serde_json::{json, Value};
use support::cloud::{github, open_headless};
use support::{fake_agent, git, repo, write, FakeHub, Repo, TempDir, TOKEN};

/// A suite whose `e2e` writes a report with one file: what its process was given and could read.
const SUITE_E2E: &str = r#"#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const probe = (file) => { try { fs.readdirSync(file); return 'listed'; } catch (error) { try { fs.readFileSync(file); return 'read'; } catch (again) { return again.code; } } };
const attempt = path.join('.e2e', 'results', 'smoke-works-0123456789abcdef', 'attempt-1');
fs.mkdirSync(attempt, { recursive: true });
fs.writeFileSync(path.join(attempt, 'seen.json'), JSON.stringify({
  env: Object.keys(process.env), home: process.env.HOME,
  probes: Object.fromEntries((process.env.SUITE_PROBE || '').split(',').filter(Boolean).map((file) => [file, probe(file)])),
  outside: (() => { try { fs.writeFileSync(path.join(process.env.SUITE_OUTSIDE, 'planted.txt'), 'x'); return 'written'; } catch (error) { return error.code; } })(),
}));
fs.writeFileSync(path.join('.e2e', 'report.json'), JSON.stringify({ schemaVersion: 'report-1', run: { id: 'r', results: [{ id: '0123456789abcdef', attempts: [{ artifacts: [
  { kind: 'download', mediaType: 'application/json', path: 'smoke-works-0123456789abcdef/attempt-1/seen.json' },
] }] }] } }));
console.log('suite ran');
"#;

struct Computer {
    desktop: Desktop,
    data: TempDir,
    repo: Repo,
}

/// The desktop core on a computer with a clone of the project's repository, which holds a suite.
async fn computer(label: &str, hub: &FakeHub, sandbox: SandboxState) -> Computer {
    let data = TempDir::new(label);
    let mut options = OpenOptions::new(Paths::under(data.path()), Arc::new(MemoryKeychain::default()));

    options.helper = Some(fake_agent());
    options.stop_grace = Duration::from_millis(400);
    options.poll_every = Duration::from_millis(100);
    options.sandbox = Some(sandbox);
    options.github_cli = Some(false);

    let desktop = Desktop::open(options).unwrap();
    let repo = repo(label);
    let (url, path) = (hub.url.clone(), repo.clone.to_string_lossy().into_owned());

    write(&repo.clone, "e2e/package.json", "{ \"name\": \"suite\", \"private\": true }\n");
    write(&repo.clone, "e2e/install.sh", "mkdir -p node_modules/.bin && ln -sf ../../tools/e2e.js node_modules/.bin/e2e\n");
    write(&repo.clone, "e2e/tools/e2e.js", SUITE_E2E);
    std::fs::set_permissions(repo.clone.join("e2e/tools/e2e.js"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    git(&repo.clone, &["add", "--all"]);
    git(&repo.clone, &["commit", "-m", "A suite"]);
    git(&repo.clone, &["push", "origin", "HEAD:main"]);
    git(&repo.clone, &["fetch", "origin"]);

    desktop.run(move |core| core.connect(&url, TOKEN)).await.unwrap();
    desktop.run(move |core| core.map_repo("WEB", None, &path)).await.unwrap();

    // The project's tests run from the repository that clone is of, with a browser of the suite's own.
    let name = hub_desktop_core::git::repo_of_remote(&repo.origin.to_string_lossy()).unwrap();

    hub.set_e2e(
        json!({
            "repo": name, "ref": "main", "directory": "e2e", "installCommand": "sh install.sh", "runArgs": "", "video": "off", "trace": "off",
            "env": { "CHROMIUM_EXECUTABLE": "/usr/bin/chromium", "SUITE_OUTSIDE": data.path().to_str().unwrap() }, "timeoutMinutes": 5,
        }),
        &[],
    );

    Computer { desktop, data, repo }
}

fn sandbox_here() -> SandboxState {
    policy::sandbox_probe()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_desktop_app_runs_the_test_jobs_sent_to_it_by_name_on_the_computer() {
    std::env::set_var("TEAM_HUB_TOKEN", TOKEN);

    let hub = FakeHub::start().await;
    let computer = computer("desktop-job", &hub, sandbox_here()).await;
    let runner = computer.desktop.job_runner();
    let (_stop, mut stopped) = tokio::sync::watch::channel(false);
    let here = computer.desktop.core().settings().runner_id;
    let head_before = git(&computer.repo.clone, &["rev-parse", "HEAD"]);

    // The team's queue is not this computer's business unless its owner says so.
    let queued = hub.queue_job(None, None, "off", None);

    computer.desktop.tick(true).await.unwrap();
    assert!(!runner.run_next(&mut stopped).await.unwrap());
    assert_eq!(hub.job(queued)["state"], "queued");

    let claim = hub.state().e2e.claims[0].clone();
    let heartbeat = hub.state().runners[0].clone();

    assert_eq!(claim["runnerId"], heartbeat["runnerId"]);
    assert_eq!((&claim["interactive"], &claim["pinnedOnly"], &claim["shared"]), (&json!(true), &json!(true), &json!(false)));
    assert!(claim.get("projects").is_none());

    // Started in the app for this computer: it is carried out here.
    let state_file = computer.data.path().join("state/desktop.db");
    let home_keys = dirs::home_dir().unwrap().join(".ssh");

    hub.state().e2e.settings["env"]["SUITE_PROBE"] = json!(format!("{},{}", state_file.display(), home_keys.display()));

    let new = NewTestJob { project_key: "web".into(), reference: None, filter: None, on_runner: Some(here.clone()) };
    let job = computer.desktop.run(move |core| core.start_test_job(&new)).await.unwrap();
    let id = job["id"].as_i64().unwrap();

    assert_eq!(job["onRunner"], here.as_str());
    assert!(runner.run_next(&mut stopped).await.unwrap());

    let detail = computer.desktop.run(move |core| core.test_job(id, 0)).await.unwrap();

    assert_eq!(detail["job"]["state"], "succeeded", "{:#?}", detail["log"]);
    assert_eq!(detail["job"]["commit"], head_before.as_str());
    assert!(detail["log"].as_array().unwrap().iter().any(|line| line["text"] == "suite ran"));

    // The person's checkout was not touched: same commit, nothing changed, no working tree added.
    assert_eq!(git(&computer.repo.clone, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(&computer.repo.clone, &["status", "--porcelain"]), "");
    assert_eq!(git(&computer.repo.clone, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 1);

    // The suite got the settings' variables and none of the app's own.
    let files = hub.artifacts_of(detail["job"]["runId"].as_i64().unwrap());
    let seen: Value = serde_json::from_slice(&files["smoke-works-0123456789abcdef/attempt-1/seen.json"].1).unwrap();
    let names: Vec<&str> = seen["env"].as_array().unwrap().iter().filter_map(Value::as_str).collect();

    assert!(names.contains(&"CHROMIUM_EXECUTABLE") && !names.contains(&"TEAM_HUB_TOKEN"), "{names:?}");
    assert_eq!(seen["home"], computer.data.path().join(format!("jobs/{id}/home")).to_str().unwrap());

    // Where the system has a sandbox, the job cannot read the app's state or the person's
    // keys and cannot write outside its own folder; without one it is the person's own user.
    // Test jobs are confined on Linux only (`JobOptions::for_desktop`).
    if cfg!(target_os = "linux") && sandbox_here() == SandboxState::Available {
        assert_eq!(seen["probes"][state_file.to_str().unwrap()], "ENOENT");
        assert_ne!(seen["probes"][home_keys.to_str().unwrap()], "read");
        // What it wrote beside its folder went to a /tmp of its own, not to the disk.
        assert!(!computer.data.path().join("planted.txt").exists());
    } else {
        assert_eq!(seen["probes"][state_file.to_str().unwrap()], "read");
    }

    // Its owner lets it take the team's queue too: the claim says so, and the job is taken.
    let mut settings = computer.desktop.core().settings();

    settings.test_jobs_from_queue = true;
    computer.desktop.run(move |core| core.save_settings(settings)).await.unwrap();
    assert!(runner.run_next(&mut stopped).await.unwrap());
    assert_eq!(hub.job(queued)["state"], "succeeded");
    assert_eq!(hub.state().e2e.claims.last().unwrap()["pinnedOnly"], false);

    // Test jobs switched off altogether: the computer asks for none.
    let mut settings = computer.desktop.core().settings();

    settings.test_jobs = false;
    computer.desktop.run(move |core| core.save_settings(settings)).await.unwrap();
    assert!(!computer.desktop.core().takes_test_jobs());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_app_shows_the_same_runners_and_hands_work_to_another_one() {
    let hub = FakeHub::start().await;
    let computer = computer("runners", &hub, SandboxState::Unavailable).await;
    let host = github("runners").await;
    let cloud = open_headless("runners-cloud", &hub, &host, &[("RUNNER_NAME", "Cloud runner 1")], |_| {}).await;

    computer.desktop.tick(true).await.unwrap();
    cloud.desktop.tick(true).await.unwrap();

    let (_stop, mut stopped) = tokio::sync::watch::channel(false);

    computer.desktop.job_runner().run_next(&mut stopped).await.unwrap();

    let here = computer.desktop.core().settings().runner_id;
    let there = cloud.desktop.core().settings().runner_id;
    let runners = commands::invoke(&computer.desktop, "runners", json!({})).await.unwrap();
    let of = |id: &str| runners.as_array().unwrap().iter().find(|runner| runner["id"] == id).cloned().unwrap();

    assert_eq!(runners.as_array().unwrap().len(), 2);
    assert_eq!((&of(&here)["interactive"], of(&here)["work"].as_array().unwrap().len()), (&json!(true), 2));
    assert_eq!((&of(&there)["interactive"], &of(&there)["name"]), (&json!(false), &json!("Cloud runner 1")));
    assert_eq!(
        of(&here)["work"][1],
        json!({ "module": "e2e", "what": ["tests"], "projects": null, "pinnedOnly": true, "online": true, "lastSeenAt": support::NOW, "active": [] })
    );

    // Work on an issue sent to the cloud runner: no thread starts here, and this app never claims it.
    let new = Delegation {
        issue_key: "web-1".into(),
        kind: ThreadKind::Implement,
        instructions: String::new(),
        model: None,
        repo: Some("acme/web".into()),
        on_runner: Some(there.clone()),
    };
    let run = commands::invoke(&computer.desktop, "delegate", serde_json::to_value(&new).unwrap()).await.unwrap();
    let run_id = run["id"].as_i64().unwrap();

    assert_eq!((&run["state"], &run["onRunner"], &run["repo"]), (&json!("queued"), &json!(there), &json!("acme/web")));

    for _ in 0..3 {
        computer.desktop.tick(true).await.unwrap();
    }

    assert_eq!(hub.run(run_id)["run"]["state"], "queued");
    assert!(computer.desktop.run(|core| core.threads()).await.unwrap().is_empty());

    // The cloud runner carries it out; the app follows it and decides its push like the web does.
    for _ in 0..200 {
        cloud.desktop.tick(true).await.unwrap();

        if hub.run(run_id)["run"]["state"] == "waiting" && hub.run(run_id)["proposals"].as_array().unwrap().len() == 3 {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let followed = commands::invoke(&computer.desktop, "run_detail", json!({ "id": run_id })).await.unwrap();
    let push = followed["proposals"].as_array().unwrap().iter().find(|proposal| proposal["kind"] == "push").cloned().unwrap();

    assert_eq!(followed["run"]["runnerName"], "Cloud runner 1");
    assert!(followed["events"].as_array().unwrap().len() > 3);

    let decided = commands::invoke(&computer.desktop, "decide_on_run", json!({ "id": push["id"], "approve": true })).await.unwrap();

    assert!(decided["proposals"].as_array().unwrap().iter().any(|proposal| proposal["id"] == push["id"] && proposal["state"] == "approved"));

    for _ in 0..200 {
        cloud.desktop.tick(true).await.unwrap();

        if !hub.proposal_on_run(run_id, "push")["result"].is_null() {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(hub.proposal_on_run(run_id, "push")["result"]["commit"], push["payload"]["commit"]);

    // The project's jobs, started for any runner, followed and cancelled through the same commands.
    let started =
        commands::invoke(&computer.desktop, "start_test_job", json!({ "projectKey": "WEB", "reference": "main", "filter": "tests/a.e2e.ts" }))
            .await
            .unwrap();
    let listed = commands::invoke(&computer.desktop, "test_jobs", json!({ "key": "WEB" })).await.unwrap();

    assert_eq!((&started["onRunner"], &started["filter"]), (&Value::Null, &json!("tests/a.e2e.ts")));
    assert_eq!(listed[0]["id"], started["id"]);
    assert_eq!(commands::invoke(&computer.desktop, "cancel_test_job", json!({ "id": started["id"] })).await.unwrap()["state"], "cancelled");
    assert_eq!(commands::invoke(&computer.desktop, "test_settings", json!({ "key": "WEB" })).await.unwrap()["directory"], "e2e");

    // A runner the tests module has not heard of cannot be named.
    let unknown = commands::invoke(&computer.desktop, "start_test_job", json!({ "projectKey": "WEB", "onRunner": "runner-nobody" })).await;

    assert!(unknown.unwrap_err().to_string().contains("does not take test jobs"));
    drop(PathBuf::new());
}
