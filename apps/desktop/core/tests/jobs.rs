//! Test jobs (docs/e2e-jobs.md) carried out by the headless core against the fake team-hub:
//! a repository on the stand-in for GitHub whose "e2e" writes a report and a few files, so
//! the whole life of a job runs without a browser.

mod support;

use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use hub_desktop_core::jobs::JobUser;
use serde_json::{json, Value};
use support::cloud::{github, holds, job_runner, open_headless, GitHost, GITHUB_TOKEN};
use support::{git, group_alive, write, Bench, FakeHub, TOKEN};

const RESULT: &str = "login-signs-in-0123456789abcdef";

/// The suite's `e2e`: writes what the framework would (a report, files under
/// `<output>/results`), prints, and misbehaves on request (`SUITE_MODE`).
const FAKE_E2E: &str = r#"#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');

const args = process.argv.slice(2);
const mode = process.env.SUITE_MODE || 'pass';
const flag = (name) => (args.find((arg) => arg.startsWith(`--${name}=`)) || '').split('=')[1];
const output = flag('output') || (args.includes('--output') ? args[args.indexOf('--output') + 1] : '.e2e');
const results = path.join(output, 'results');
const attempt = path.join(results, 'login-signs-in-0123456789abcdef', 'attempt-1');
const probe = (file) => { try { fs.readFileSync(file); return 'read'; } catch (error) { return error.code; } };

console.log(`e2e ${args.join(' ')}`);
console.log(`pid ${process.pid}`);
console.error('a line on standard error');
console.log(`signing in with ${process.env.SUITE_PASSWORD}`);

if (mode === 'no-report') {
  console.error('COLLECTION_ERROR: tests/login.e2e.ts does not compile');
  process.exit(3);
}

if (mode === 'slow') {
  // Leaves a child behind on purpose: ending the job has to take it along.
  spawn('sleep', ['600'], { stdio: 'ignore' });
  let count = 0;
  setInterval(() => console.log(`step ${++count}`), 100);
} else {
  fs.mkdirSync(path.join(attempt, 'screenshots'), { recursive: true });
  fs.mkdirSync(path.join(attempt, 'video'), { recursive: true });
  fs.mkdirSync(path.join(attempt, 'downloads'), { recursive: true });
  fs.writeFileSync(path.join(attempt, 'screenshots', '001-failure.png'), Buffer.from([0x89, 0x50, 0x4e, 0x47, 1, 2, 3]));
  fs.symlinkSync('/etc/hostname', path.join(attempt, 'downloads', 'link.txt'));
  fs.writeFileSync(path.join(attempt, 'downloads', 'env.json'), JSON.stringify({
    env: process.env, args, cwd: process.cwd(), uid: process.getuid(),
    probes: Object.fromEntries((process.env.SUITE_PROBE || '').split(',').filter(Boolean).map((file) => [file, probe(file)])),
  }));
  const artifacts = [
    { id: 'a:0', kind: 'screenshot', mediaType: 'image/png', path: 'login-signs-in-0123456789abcdef/attempt-1/screenshots/001-failure.png', size: 7 },
    { id: 'a:1', kind: 'download', mediaType: 'application/json', path: 'login-signs-in-0123456789abcdef/attempt-1/downloads/env.json' },
    { id: 'a:2', kind: 'download', mediaType: 'text/plain', path: 'login-signs-in-0123456789abcdef/attempt-1/downloads/never-written.txt' },
    { id: 'a:3', kind: 'download', mediaType: 'text/plain', path: 'login-signs-in-0123456789abcdef/attempt-1/downloads/link.txt' },
    { id: 'a:4', kind: 'download', mediaType: 'text/plain', path: '../../../../../../../../etc/hostname' },
    { id: 'a:5', kind: 'video', mediaType: 'video/mp4', url: 'https://recordings.example/1', startedAt: '2026-11-13T10:00:00.000Z' },
  ];
  if (flag('video') !== 'off') {
    fs.writeFileSync(path.join(attempt, 'video', 'video.webm'), 'WEBM-BYTES');
    artifacts.push({ id: 'a:6', kind: 'video', mediaType: 'video/webm', path: 'login-signs-in-0123456789abcdef/attempt-1/video/video.webm', startedAt: '2026-11-13T10:00:00.100Z' });
  }
  if (flag('trace') !== 'off') {
    fs.writeFileSync(path.join(results, 'login-signs-in-0123456789abcdef', 'trace.md'), '# signs in\n');
  }
  const failed = mode === 'fail';
  fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify({ schemaVersion: 'report-1', run: {
    id: 'run-1', status: failed ? 'failed' : 'passed', exitCode: failed ? 1 : 0,
    results: [{ id: '0123456789abcdef0123456789abcdef', file: 'tests/login.e2e.ts', status: failed ? 'failed' : 'passed', attempts: [{ index: 0, artifacts }] }],
  } }));
  console.log('1 test, report written');
  process.exit(failed ? 1 : 0);
}
"#;

const FAKE_E2E_WEB: &str = r#"#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const folder = process.env.PLAYWRIGHT_BROWSERS_PATH;
const marker = path.join(folder, 'installs.txt');
fs.mkdirSync(folder, { recursive: true });
fs.appendFileSync(marker, `${process.argv.slice(2).join(' ')}\n`);
console.log(`browser ready in ${folder}`);
"#;

/// Makes `npx e2e` and `npx e2e-web` the two scripts above, without asking a registry.
const INSTALL: &str = "set -e\nmkdir -p node_modules/.bin\nln -sf ../../tools/e2e.js node_modules/.bin/e2e\nln -sf ../../tools/e2e-web.js node_modules/.bin/e2e-web\necho installed\n";

static TOKENS: Once = Once::new();

/// The runner's own secrets, in the test process's environment as they are in a container's.
fn tokens_in_the_environment() {
    TOKENS.call_once(|| {
        std::env::set_var("TEAM_HUB_TOKEN", TOKEN);
        std::env::set_var("GITHUB_TOKEN", GITHUB_TOKEN);
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-not-for-test-suites");
    });
}

fn settings(mode: &str) -> Value {
    json!({
        "repo": "acme/web", "ref": "main", "directory": "e2e", "installCommand": "sh install.sh", "runArgs": "--workers 2",
        "video": "on", "trace": "on", "env": { "SUITE_URL": "http://app.test:8080", "SUITE_MODE": mode }, "secretNames": ["SUITE_PASSWORD"],
        "timeoutMinutes": 30,
    })
}

async fn ready(label: &str, mode: &str) -> (Bench, FakeHub, GitHost) {
    tokens_in_the_environment();

    let hub = FakeHub::start().await;
    let host = github(label).await;
    let seed = host.seed();

    write(&seed, "e2e/package.json", "{ \"name\": \"fake-suite\", \"private\": true }\n");
    write(&seed, "e2e/install.sh", INSTALL);
    write(&seed, "e2e/tools/e2e.js", FAKE_E2E);
    write(&seed, "e2e/tools/e2e-web.js", FAKE_E2E_WEB);
    // A report that came with the repository: it must not pass for a run's.
    write(&seed, "e2e/.e2e/report.json", "{ \"schemaVersion\": \"report-1\", \"run\": { \"id\": \"stale\" } }\n");
    git(&seed, &["add", "--all"]);
    git(&seed, &["update-index", "--chmod=+x", "e2e/tools/e2e.js", "e2e/tools/e2e-web.js"]);
    git(&seed, &["commit", "-m", "A suite"]);
    git(&seed, &["push", "origin", "HEAD:main"]);
    hub.set_e2e(settings(mode), &[("SUITE_PASSWORD", "hunter2hunter2")]);

    let bench = open_headless(label, &hub, &host, &[], |_| {}).await;

    (bench, hub, host)
}

fn steps(hub: &FakeHub, job: i64) -> Vec<String> {
    let path = format!("/jobs/{job}");

    hub.state()
        .writes
        .iter()
        .filter(|(method, to, body)| method == "PATCH" && *to == path && body["step"].is_string())
        .map(|(_, _, body)| body["step"].as_str().unwrap().to_string())
        .fold(Vec::new(), |mut steps, step| {
            if steps.last() != Some(&step) {
                steps.push(step);
            }

            steps
        })
}

async fn eventually(what: &str, holds: impl Fn() -> bool) {
    for _ in 0..300 {
        if holds() {
            return;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("never: {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_is_checked_out_run_and_its_report_and_files_come_back() {
    let (bench, hub, host) = ready("job", "pass").await;
    let (runner, _stop, mut stopped) = job_runner(&bench, JobUser::Same);
    let job = hub.queue_job(None, Some("tests/login.e2e.ts --grep \"signs in\""), "retain-on-failure", None);

    assert!(runner.run_next(&mut stopped).await.unwrap());
    assert!(!runner.run_next(&mut stopped).await.unwrap(), "nothing else is queued");

    let done = hub.job(job);
    let run_id = done["runId"].as_i64().expect("the report is tied to the job");

    assert_eq!((&done["state"], &done["error"], &done["commit"]), (&json!("succeeded"), &Value::Null, &json!(host.tip("main"))));
    assert_eq!(steps(&hub, job), vec!["cloning", "installing", "running", "uploading"]);
    assert_eq!(done["runnerName"], "Cloud runner");
    // This run's report, not the one that came with the repository.
    assert_eq!(hub.state().e2e.reports[&run_id]["run"]["id"], "run-1");

    // Every file the report names that is on disk, under the path the report names it by,
    // and the trace page; the one never written, the link out of the folder and the path
    // that climbs out of it are left behind.
    let artifacts = hub.artifacts_of(run_id);
    let under = |rest: &str| format!("{RESULT}/{rest}");
    let mut paths: Vec<&str> = artifacts.keys().map(String::as_str).collect();

    paths.sort_unstable();
    assert_eq!(
        paths,
        vec![
            under("attempt-1/downloads/env.json").as_str(),
            under("attempt-1/screenshots/001-failure.png").as_str(),
            under("attempt-1/video/video.webm").as_str(),
            under("trace.md").as_str(),
        ]
    );
    assert_eq!(artifacts[&under("attempt-1/screenshots/001-failure.png")], ("image/png".to_string(), vec![0x89, 0x50, 0x4e, 0x47, 1, 2, 3]));
    assert_eq!(artifacts[&under("attempt-1/video/video.webm")], ("video/webm".to_string(), b"WEBM-BYTES".to_vec()));
    assert_eq!(artifacts[&under("trace.md")], ("text/markdown".to_string(), b"# signs in\n".to_vec()));

    // What the suite's process was given: the settings' variables and secrets, a home in the
    // job's folder, the browsers' folder, CI, and none of the runner's own.
    let seen: Value = serde_json::from_slice(&artifacts[&under("attempt-1/downloads/env.json")].1).unwrap();
    let env = seen["env"].as_object().unwrap();
    let job_folder = bench.data.path().join("jobs").join(job.to_string());

    for (name, value) in [("SUITE_URL", "http://app.test:8080"), ("SUITE_MODE", "pass"), ("SUITE_PASSWORD", "hunter2hunter2"), ("CI", "1")] {
        assert_eq!(env.get(name), Some(&json!(value)), "{name}");
    }

    assert_eq!(env["HOME"], job_folder.join("home").to_str().unwrap());
    assert_eq!(env["PLAYWRIGHT_BROWSERS_PATH"], bench.data.path().join("browsers").to_str().unwrap());
    assert_eq!(seen["cwd"], job_folder.join("src/e2e").to_str().unwrap());

    let given = ["PATH", "HOME", "CI", "PLAYWRIGHT_BROWSERS_PATH", "SUITE_URL", "SUITE_MODE", "SUITE_PASSWORD"];
    let added_by_npx = |name: &str| name.starts_with("npm_") || ["NODE", "INIT_CWD", "COLOR", "EDITOR", "PWD", "_"].contains(&name);

    assert!(env.keys().all(|name| given.contains(&name.as_str()) || added_by_npx(name)), "{:?}", env.keys().collect::<Vec<_>>());
    assert!(std::env::var("TEAM_HUB_TOKEN").is_ok(), "the runner's own environment has them");
    assert!(!seen.to_string().contains(TOKEN) && !seen.to_string().contains(GITHUB_TOKEN) && !seen.to_string().contains("sk-ant-not-for-test-suites"));

    // The arguments: the filter as typed, the job's recording modes, then the settings' own.
    assert_eq!(seen["args"], json!(["run", "tests/login.e2e.ts", "--grep", "signs in", "--video=retain-on-failure", "--trace=on", "--workers", "2"]));

    // The log has the runner's steps and what the suite printed on both streams; the secret
    // the suite printed is masked.
    let log = hub.job_log(job);
    let has = |text: &str| log.iter().any(|line| line.contains(text));

    assert!(has("$ sh -c sh install.sh") && has("installed") && has("$ npx --no -- e2e-web install chromium") && has("browser ready in"), "{log:#?}");
    assert!(has("a line on standard error") && has("1 test, report written") && has("4 files sent"), "{log:#?}");
    assert!(has("signing in with ***") && !has("hunter2hunter2"), "{log:#?}");
    assert!(has("never-written.txt is named by the report but is not a file") && has("link.txt is named by the report"), "{log:#?}");

    // The job's folder is gone; the browsers' folder stays for the next job.
    assert!(!job_folder.exists());
    assert_eq!(std::fs::read_to_string(bench.data.path().join("browsers/installs.txt")).unwrap(), "install chromium\n");

    // A second job: video off, another ref spelled as a commit, its own browser.
    let commit = host.tip("main");

    hub.state().e2e.settings["env"]["CHROMIUM_EXECUTABLE"] = json!("/opt/chromium");
    hub.state().e2e.settings["env"]["SUITE_MODE"] = json!("fail");

    let second = hub.queue_job(Some(&commit[..12]), None, "off", None);

    assert!(runner.run_next(&mut stopped).await.unwrap());

    let done = hub.job(second);
    let files = hub.artifacts_of(done["runId"].as_i64().unwrap());

    // The suite's tests failed (exit code 1) and the report still arrived: the job succeeded.
    assert_eq!((&done["state"], &done["commit"]), (&json!("succeeded"), &json!(commit)));
    assert!(!files.contains_key(&under("attempt-1/video/video.webm")) && files.contains_key(&under("trace.md")));
    assert!(hub.job_log(second).iter().any(|line| line.contains("e2e run ended with exit code 1")));
    assert!(!hub.job_log(second).iter().any(|line| line.contains("e2e-web install")), "a suite with its own browser installs none");
    assert_eq!(std::fs::read_to_string(bench.data.path().join("browsers/installs.txt")).unwrap(), "install chromium\n");

    // Neither the clone nor anything else under the data folder holds a token.
    assert_eq!(holds(bench.data.path(), GITHUB_TOKEN), Vec::<PathBuf>::new());
    assert_eq!(std::fs::read_dir(bench.data.path().join("jobs")).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_claim_says_who_asks_and_a_job_sent_to_another_runner_is_left_alone() {
    let (bench, hub, _host) = ready("claim", "pass").await;
    let (runner, _stop, mut stopped) = job_runner(&bench, JobUser::Same);
    let elsewhere = hub.queue_job(None, None, "off", Some("desktop-0000"));

    bench.desktop.tick(true).await.unwrap();
    assert!(!runner.run_next(&mut stopped).await.unwrap());
    assert_eq!(hub.job(elsewhere)["state"], "queued");

    let claim = hub.state().e2e.claims[0].clone();
    let heartbeat = hub.state().runners[0].clone();

    assert_eq!(claim["runnerId"], heartbeat["runnerId"]);
    assert_eq!((&claim["runnerName"], &claim["interactive"], &claim["pinnedOnly"]), (&json!("Cloud runner"), &json!(false), &json!(false)));
    assert_eq!(claim["version"], env!("CARGO_PKG_VERSION"));
    assert!(claim.get("projects").is_none());

    let here = hub.queue_job(None, None, "off", claim["runnerId"].as_str());

    assert!(runner.run_next(&mut stopped).await.unwrap());
    assert_eq!(hub.job(here)["state"], "succeeded");

    // A server from before test jobs is asked, answers that it has none, and nothing breaks.
    hub.state().e2e.off = true;
    assert!(runner.run_next(&mut stopped).await.unwrap_err().is_not_found());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_output_reaches_the_log_while_the_suite_runs_and_a_cancel_ends_it_with_everything_it_started() {
    let (bench, hub, _host) = ready("cancel", "slow").await;
    let (runner, _stop, mut stopped) = job_runner(&bench, JobUser::Same);
    let job = hub.queue_job(None, None, "on", None);
    let running = tokio::spawn(async move { runner.run_next(&mut stopped).await });

    // Lines arrive in several small batches while the job is still running.
    eventually("ten steps in the log", || hub.job_log(job).iter().filter(|line| line.starts_with("step ")).count() >= 10).await;
    assert_eq!((&hub.job(job)["state"], &hub.job(job)["step"]), (&json!("running"), &json!("running")));
    assert!(hub.state().e2e.log_batches.len() >= 3 && hub.state().e2e.log_batches.iter().all(|lines| *lines < 40), "{:?}", hub.state().e2e.log_batches);

    let pid: u32 = hub.job_log(job).iter().find_map(|line| line.strip_prefix("pid ").and_then(|pid| pid.parse().ok())).unwrap();
    let group = std::process::Command::new("ps").args(["-o", "pgid=", "-p", &pid.to_string()]).output().unwrap();
    let group: u32 = String::from_utf8_lossy(&group.stdout).trim().parse().unwrap();

    assert!(group_alive(group));
    hub.cancel_job(job);
    assert!(tokio::time::timeout(Duration::from_secs(10), running).await.unwrap().unwrap().unwrap());

    let done = hub.job(job);

    assert_eq!((&done["state"], &done["error"]), (&json!("cancelled"), &json!("Cancelled.")));
    assert!(done["runId"].is_null());
    assert!(!group_alive(group), "the suite and the sleep it started are gone");
    assert!(!bench.data.path().join("jobs").join(job.to_string()).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_past_its_time_is_stopped_and_one_without_a_report_fails_with_the_reason() {
    let (bench, hub, _host) = ready("timeout", "slow").await;
    let folders = hub_desktop_core::headless::Folders::under(bench.data.path());
    let mut options = hub_desktop_core::jobs::JobOptions::new(folders, JobUser::Same);

    // A minute that lasts a tenth of a second: thirty of them are three seconds.
    options.minute = Duration::from_millis(100);
    options.log_every = Duration::from_millis(200);
    options.look_every = Duration::from_millis(500);

    let runner = hub_desktop_core::jobs::JobRunner::new(std::sync::Arc::clone(bench.desktop.core()), options);
    let (_stop, mut stopped) = tokio::sync::watch::channel(false);
    let job = hub.queue_job(None, None, "on", None);
    let started = std::time::Instant::now();

    assert!(runner.run_next(&mut stopped).await.unwrap());

    let done = hub.job(job);

    assert_eq!((&done["state"], &done["error"]), (&json!("failed"), &json!("Stopped after 30 minutes, the project's limit for a job.")));
    assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
    assert!(hub.job_log(job).iter().any(|line| line.starts_with("step ")));
    assert!(hub.job_log(job).last().unwrap().contains("Stopped after 30 minutes"));

    // No report: the suite's exit code and where the report was looked for.
    hub.state().e2e.settings["env"]["SUITE_MODE"] = json!("no-report");

    let job = hub.queue_job(None, None, "on", None);

    assert!(runner.run_next(&mut stopped).await.unwrap());
    assert_eq!(hub.job(job)["state"], "failed");
    assert_eq!(hub.job(job)["error"], "The suite left no report at e2e/.e2e/report.json (e2e run ended with exit code 3). Its output is in the log.");
    assert!(hub.job_log(job).iter().any(|line| line.contains("COLLECTION_ERROR")));
    assert!(hub.state().e2e.reports.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_that_cannot_be_prepared_fails_with_what_went_wrong() {
    let (bench, hub, _host) = ready("unprepared", "pass").await;
    let (runner, _stop, mut stopped) = job_runner(&bench, JobUser::Same);
    let mut failure = async |change: &dyn Fn(&mut Value), reference: Option<&str>| {
        let before = hub.state().e2e.settings.clone();

        change(&mut hub.state().e2e.settings);

        let job = hub.queue_job(reference, None, "off", None);

        assert!(runner.run_next(&mut stopped).await.unwrap());
        hub.state().e2e.settings = before;

        (hub.job(job)["state"].clone(), hub.job(job)["error"].as_str().unwrap().to_string())
    };

    let (state, error) = failure(&|_| {}, Some("no-such-branch")).await;
    assert_eq!(state, "failed");
    assert!(error.starts_with("acme/web could not be checked out at no-such-branch: "), "{error}");

    let (_, error) = failure(&|settings| settings["repo"] = json!("acme/unknown"), None).await;
    assert!(error.starts_with("acme/web could not be checked out") || error.contains("git clone"), "{error}");

    let (_, error) = failure(&|settings| settings["directory"] = json!("suites/web"), None).await;
    assert_eq!(error, "The repository has no folder \"suites/web\" (the suite's directory in the project's test settings).");

    let (_, error) = failure(&|settings| settings["directory"] = json!("../.."), None).await;
    assert_eq!(error, "\"../..\" is not a folder inside the repository.");

    let (_, error) = failure(&|settings| settings["installCommand"] = json!("echo no lockfile >&2; exit 7"), None).await;
    assert_eq!(error, "The install command ended with exit code 7: echo no lockfile >&2; exit 7");

    let (_, error) = failure(&|settings| settings["runArgs"] = json!("--output /tmp/elsewhere"), None).await;
    assert!(error.contains("is not a folder inside the suite"), "{error}");

    // A runner that is root and has no user for jobs runs none, and says what to do.
    let (rootless, _stop, mut stopped) = job_runner(&bench, JobUser::Missing { name: "job".into() });
    let job = hub.queue_job(None, None, "off", None);

    assert!(rootless.run_next(&mut stopped).await.unwrap());
    assert!(hub.job(job)["error"].as_str().unwrap().starts_with("This runner runs as root and has no user \"job\""));
    assert!(!bench.data.path().join("jobs").join(job.to_string()).exists());
}

/// Only where the tests themselves run as root (a container): the job runs as `nobody`.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn as_root_a_job_runs_as_another_user_who_cannot_read_the_runners_state_or_clones() {
    let user = JobUser::for_runner("nobody");
    let JobUser::Other { uid, .. } = user.clone() else {
        eprintln!("skipped: not root, or there is no user called nobody ({user:?})");

        return;
    };
    let (bench, hub, _host) = ready("job-user", "pass").await;
    let data = bench.data.path();
    let state_file = data.join("state/desktop.db");
    let clone_file = data.join("repos/acme/web/.git/config");
    let probe = format!("{},{},{}", state_file.display(), clone_file.display(), data.join("state/settings.json").display());

    hub.state().e2e.settings["env"]["SUITE_PROBE"] = json!(probe);

    let (runner, _stop, mut stopped) = job_runner(&bench, user);
    let job = hub.queue_job(None, None, "off", None);

    assert!(runner.run_next(&mut stopped).await.unwrap());

    let done = hub.job(job);

    assert_eq!(done["state"], "succeeded", "{:#?}", hub.job_log(job));

    let files = hub.artifacts_of(done["runId"].as_i64().unwrap());
    let seen: Value = serde_json::from_slice(&files[&format!("{RESULT}/attempt-1/downloads/env.json")].1).unwrap();

    assert_eq!(seen["uid"], uid);
    assert!(state_file.exists() && clone_file.exists());
    assert_eq!(seen["probes"], json!({ state_file.to_str().unwrap(): "EACCES", clone_file.to_str().unwrap(): "EACCES", data.join("state/settings.json").to_str().unwrap(): "EACCES" }));
    // The browsers it installed are its own, for the next job.
    assert!(data.join("browsers/installs.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_runner_checks_a_job_out_with_the_requesters_token_or_fails_with_the_reason() {
    use support::cloud::PERSON_GITHUB_TOKEN;

    tokens_in_the_environment();

    let hub = FakeHub::start().await;
    let host = github("shared-job").await;
    let seed = host.seed();

    write(&seed, "e2e/package.json", "{ \"name\": \"fake-suite\", \"private\": true }\n");
    write(&seed, "e2e/install.sh", INSTALL);
    write(&seed, "e2e/tools/e2e.js", FAKE_E2E);
    write(&seed, "e2e/tools/e2e-web.js", FAKE_E2E_WEB);
    git(&seed, &["add", "--all"]);
    git(&seed, &["update-index", "--chmod=+x", "e2e/tools/e2e.js", "e2e/tools/e2e-web.js"]);
    git(&seed, &["commit", "-m", "A suite"]);
    git(&seed, &["push", "origin", "HEAD:main"]);
    hub.set_e2e(settings("pass"), &[]);
    hub.state().person_keys = json!({ "githubToken": PERSON_GITHUB_TOKEN });

    let bench = open_headless("shared-job", &hub, &host, &[("GITHUB_TOKEN", ""), ("ANTHROPIC_API_KEY", "")], |_| {}).await;
    let (runner, _stop, mut stopped) = job_runner(&bench, JobUser::Same);
    let job = hub.queue_job(None, None, "off", None);

    assert!(runner.run_next(&mut stopped).await.unwrap());
    assert_eq!(hub.job(job)["state"], "succeeded", "{:#?}", hub.job_log(job));
    assert_eq!((&hub.state().e2e.claims[0]["shared"], &hub.state().e2e.claims[0]["interactive"]), (&json!(true), &json!(false)));
    assert_eq!(holds(bench.data.path(), PERSON_GITHUB_TOKEN), Vec::<PathBuf>::new());

    // Another person's token cannot read the repository: their job fails, although the
    // clone the first job made is right there.
    hub.state().person_keys = json!({ "githubToken": "ghp_cannot_read_this_repository_0000000" });

    let denied = hub.queue_job(None, None, "off", None);

    assert!(runner.run_next(&mut stopped).await.unwrap());

    let error = hub.job(denied)["error"].as_str().unwrap().to_string();

    assert_eq!(hub.job(denied)["state"], "failed");
    assert!(error.contains("Your GitHub token cannot read acme/web") && error.contains("Settings → My keys"), "{error}");

    // A token that is not an administrator's: the claim is refused, and that is an answer, not a crash.
    hub.state().refuses_shared = true;
    assert!(runner.run_next(&mut stopped).await.unwrap_err().to_string().contains("administrator"));
}
