//! `hub-runner` as a process, the way a container runs it: configured by its environment
//! alone, up while half-configured, gone within seconds of SIGTERM, and back where it was.
#![cfg(unix)]

mod support;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::cloud::{github, runner_variables, GITHUB_TOKEN, MODEL_KEY};
use support::{group_alive, FakeHub, TempDir, TOKEN};

struct Runner {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
}

fn start(variables: &[(String, String)]) -> Runner {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hub-runner"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .envs(variables.iter().map(|(name, value)| (name.as_str(), value.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let (output, kept) = (child.stdout.take().unwrap(), Arc::clone(&lines));

    std::thread::spawn(move || BufReader::new(output).lines().map_while(Result::ok).for_each(|line| kept.lock().unwrap().push(line)));

    Runner { child, lines }
}

impl Runner {
    fn said(&self, text: &str) -> bool {
        self.lines.lock().unwrap().iter().any(|line| line.contains(text))
    }

    /// Sends SIGTERM and waits; how long the process took to leave, and its exit code.
    fn terminate(mut self) -> (Duration, Option<i32>) {
        let asked = Instant::now();

        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }

        while self.child.try_wait().unwrap().is_none() {
            assert!(asked.elapsed() < Duration::from_secs(20), "the runner ignores SIGTERM: {:#?}", self.lines.lock().unwrap());
            std::thread::sleep(Duration::from_millis(50));
        }

        (asked.elapsed(), self.child.wait().unwrap().code())
    }
}

async fn eventually(what: &str, runner: &Runner, holds: impl Fn() -> bool) {
    for _ in 0..600 {
        if holds() {
            return;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("never: {what}\n{:#?}", runner.lines.lock().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_half_configured_runner_stays_up_says_what_is_missing_and_leaves_when_asked() {
    let runner = start(&[("TEAM_HUB_URL".to_string(), "https://hub.example.com".to_string()), ("RUNNER_MAX_RUNNING".to_string(), "lots".to_string())]);

    eventually("the runner says what is missing", &runner, || runner.said("Not configured: set TEAM_HUB_TOKEN.")).await;
    assert!(runner.said("RUNNER_MAX_RUNNING is a number between 1 and 16"));

    let nothing = start(&[]);

    eventually("both are named", &nothing, || nothing.said("Not configured: set TEAM_HUB_URL and TEAM_HUB_TOKEN.")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut nothing = nothing;

    assert!(nothing.child.try_wait().unwrap().is_none(), "it stays up instead of crash-looping");

    let (took, code) = nothing.terminate();

    assert!(took < Duration::from_secs(2) && code == Some(0), "{took:?} {code:?}");
    runner.terminate();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_runner_leaves_within_seconds_of_sigterm_keeps_its_run_and_continues_it_when_it_is_back() {
    let hub = FakeHub::start().await;
    let host = github("binary").await;
    let data = TempDir::new("binary");
    let mut variables = runner_variables(&hub, &host, data.path());

    variables.push(("TEAM_HUB_SIDECAR".to_string(), env!("CARGO_BIN_EXE_hub-fake-agent").to_string()));
    variables.push(("RUNNER_NAME".to_string(), "Test container".to_string()));

    let runner = start(&variables);

    eventually("a heartbeat", &runner, || !hub.state().runners.is_empty()).await;
    let announced = hub.state().runners[0].clone();

    assert_eq!((&announced["interactive"], &announced["name"]), (&serde_json::json!(false), &serde_json::json!("Test container")));
    assert!(runner.said("Connected to") && runner.said("as Ada Nowak"));

    // The secrets were read and are gone from what the kernel shows of the process's
    // environment: a process of the same user that looks there finds names without values.
    #[cfg(target_os = "linux")]
    {
        let shown = std::fs::read(format!("/proc/{}/environ", runner.child.id())).unwrap_or_default();
        let shown = String::from_utf8_lossy(&shown);

        for secret in [TOKEN, GITHUB_TOKEN, MODEL_KEY] {
            assert!(!shown.contains(secret), "{secret} is readable in /proc/<pid>/environ");
        }
    }

    // A run that takes long is being worked on when the platform replaces the container.
    let run_id = hub.delegate_in("WEB-1", "implement", "[fake:slow]", Some("acme/web"));

    eventually("the run is being worked on", &runner, || hub.run(run_id)["events"].as_array().unwrap().iter().any(|event| event["text"] == "This will take a while.")).await;

    let children = Command::new("ps").args(["-o", "pid=", "--ppid", &runner.child.id().to_string()]).output().unwrap();
    let helpers: Vec<u32> = String::from_utf8_lossy(&children.stdout).lines().filter_map(|pid| pid.trim().parse().ok()).collect();

    assert!(helpers.iter().any(|pid| group_alive(*pid)), "{helpers:?}");

    let (took, code) = runner.terminate();

    assert!(took < Duration::from_secs(10), "it took {took:?} to stop");
    assert_eq!(code, Some(0));
    assert!(helpers.iter().all(|pid| !group_alive(*pid)), "the helper and what it started are gone");
    // The server was not told the run is over: it is still this runner's.
    assert_eq!(hub.run(run_id)["run"]["state"], "running");
    assert_eq!(hub.run(run_id)["run"]["cancelRequested"], false);

    // The container comes back on the same volume and takes the thread up where it was.
    let runner = start(&variables);

    eventually("the run is finished by the runner that came back", &runner, || hub.run(run_id)["run"]["state"] == "succeeded").await;

    let events: Vec<String> = hub.run(run_id)["events"].as_array().unwrap().iter().map(|event| event["text"].as_str().unwrap_or_default().to_string()).collect();

    assert!(events.iter().any(|text| text == "Resumed."), "{events:#?}");
    assert!(events.iter().any(|text| text.starts_with("Continuing.")), "{events:#?}");
    assert_eq!(hub.state().runners.len(), 1, "the same runner id after the restart");

    let (took, _) = runner.terminate();

    assert!(took < Duration::from_secs(10));
}
