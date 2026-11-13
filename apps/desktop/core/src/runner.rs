//! The loops a runner runs for as long as its process does: `hub runner` beside a person,
//! and the headless runner of docs/cloud-runner.md. Both drive the same rounds of the core
//! (`Core::tick`); what differs is who decides what leaves the machine, and that lives there.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::app::{Desktop, OpenOptions};
use crate::error::CoreResult;
use crate::headless::RunnerEnv;
use crate::jobs::{JobOptions, JobRunner};
use crate::model::{AppEvent, EventKind, SandboxState, ThreadEvent};

const UNCONFIGURED_EVERY: Duration = Duration::from_secs(60);
const STATUS_EVERY: Duration = Duration::from_secs(30);
/// How long a job gets to tell the server it was stopped, inside the ten seconds a platform
/// gives a process that it asked to end.
const JOBS_STOP_WITHIN: Duration = Duration::from_secs(4);

/// One line of the runner's own log: standard output, with the time, for a platform's log view.
pub fn say(text: &str) {
    println!("[{}] {text}", crate::store::now());
}

/// A thread's event as one line of text.
pub fn line_of(event: &ThreadEvent) -> String {
    let mark = match event.kind {
        EventKind::Message => "agent ",
        EventKind::Tool => "tool  ",
        EventKind::Status => "      ",
        EventKind::Question => "ASKS  ",
        EventKind::Answer => "you   ",
        EventKind::Error => "ERROR ",
    };

    format!("{mark}{}", event.text.replace('\n', "\n      "))
}

/// Resolves when the process is asked to end: Ctrl-C, or the SIGTERM a container platform
/// sends before it replaces the process.
pub async fn asked_to_stop() {
    #[cfg(unix)]
    {
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());

        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// `hub runner`: claims and carries out delegated runs beside a person, printing what the
/// threads say, until interrupted (or, with `once`, until nothing is left to do).
pub async fn attended(desktop: &Desktop, once: bool) -> CoreResult<()> {
    let mut events = desktop.subscribe();
    let mut idle_rounds = 0;

    println!("Runner {} is looking for runs. Ctrl-C to stop.", desktop.core().settings().runner_name);

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(AppEvent::ThreadEvent { thread_id, event }) => println!("#{thread_id} {}", line_of(&event)),
                Ok(AppEvent::Notify { title, .. }) => println!("* {title}"),
                _ => {}
            },
            () = tokio::time::sleep(Duration::from_millis(500)) => {
                desktop.tick(true).await?;

                let busy = desktop.run(|core| Ok(core.threads()?.iter().any(|thread| thread.state.is_active()))).await?;

                idle_rounds = if busy { 0 } else { idle_rounds + 1 };

                if once && idle_rounds >= 3 {
                    break;
                }
            }
            () = asked_to_stop() => {
                println!("Stopping…");
                desktop.shutdown().await;

                break;
            }
        }
    }

    Ok(())
}

/// A runner that lacks its address or its token stays up and says so, instead of ending and
/// being started again in a loop by whatever runs it.
async fn wait_unconfigured(env: &RunnerEnv) {
    loop {
        say(&format!(
            "Not configured: set {}. The runner does nothing until then (docs/cloud-runner.md lists every variable).",
            env.missing().join(" and ")
        ));

        tokio::select! {
            () = asked_to_stop() => return,
            () = tokio::time::sleep(UNCONFIGURED_EVERY) => {}
        }
    }
}

fn status_line(desktop: &Desktop) -> String {
    let status = desktop.core().status();

    match (&status.user, status.online, status.error.or(status.runner.error)) {
        (_, _, Some(error)) => format!("Server: {error}"),
        (Some(user), true, None) => format!("Connected to {} as {}.", status.server_url.unwrap_or_default(), user.name),
        (_, _, None) => format!("{} cannot be reached; trying again.", status.server_url.unwrap_or_default()),
    }
}

/// The headless runner: opens the core as the environment says, keeps its rounds and the
/// live stream going, runs test jobs beside them, and ends within seconds of being asked to,
/// leaving its runs to be continued when it comes back.
pub async fn headless(env: &RunnerEnv, adjust: impl FnOnce(&mut OpenOptions, &mut JobOptions)) -> CoreResult<()> {
    env.remarks.iter().for_each(|remark| say(remark));

    let Some(mut options) = env.open_options() else {
        wait_unconfigured(env).await;

        return Ok(());
    };
    let mut job_options = JobOptions::new(env.folders(), env.job_user());

    adjust(&mut options, &mut job_options);
    env.limits().iter().for_each(|limit| say(limit));

    let desktop = Desktop::open(options)?;
    let _ = desktop.run(|core| Ok(core.reconnect())).await;
    let status = desktop.core().status();
    let mut last_status = status_line(&desktop);

    say(&format!("Runner \"{}\" ({}), data in {}.", status.runner.name, status.runner.id, env.data_dir.display()));
    say(&last_status);
    say(match (status.sandbox, env.allow_shell) {
        (SandboxState::Available, _) => "Agent threads get a shell inside the operating system's sandbox.",
        (SandboxState::Unavailable, true) => "Agent threads get an unconfined shell (RUNNER_ALLOW_SHELL): only for repositories and people you trust.",
        (SandboxState::Unavailable, false) => "Agent threads work without a shell: there is no working sandbox here and RUNNER_ALLOW_SHELL is off.",
    });

    // The first round at once: what was interrupted goes on without waiting for a poll.
    let _ = desktop.tick(true).await;

    desktop.start_background();

    let (stop, stopped) = watch::channel(false);
    let jobs = env.test_jobs.then(|| {
        say(&job_options.user.describe());

        let runner = JobRunner::new(Arc::clone(desktop.core()), job_options);

        tokio::spawn(async move { runner.run(stopped).await })
    });
    let mut events = desktop.subscribe();

    loop {
        tokio::select! {
            () = asked_to_stop() => break,
            event = events.recv() => match event {
                Ok(AppEvent::ThreadEvent { thread_id, event }) if matches!(event.kind, EventKind::Status | EventKind::Error | EventKind::Question) => {
                    say(&format!("#{thread_id} {}", line_of(&event).trim_start()));
                }
                Ok(AppEvent::Notify { title, .. }) => say(&title),
                _ => {}
            },
            () = tokio::time::sleep(STATUS_EVERY) => {
                let now = status_line(&desktop);

                if now != last_status {
                    say(&now);
                    last_status = now;
                }
            }
        }
    }

    say("Asked to stop: ending helpers and jobs. Runs stay this runner's and continue when it is back.");

    let _ = stop.send(true);

    desktop.suspend().await;

    if let Some(jobs) = jobs {
        let _ = tokio::time::timeout(JOBS_STOP_WITHIN, jobs).await;
    }

    say("Stopped.");

    Ok(())
}
