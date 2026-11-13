//! `hub-runner`: the core without a person at it, for a container (docs/cloud-runner.md).
//! Everything it needs comes from the environment; it takes no arguments.

use std::process::ExitCode;

use hub_desktop_core::headless::{forget_secrets_in_environment, RunnerEnv};
use hub_desktop_core::runner;

fn main() -> ExitCode {
    // Read once, then taken out of the environment before anything else runs, so that
    // nothing this process starts can find the tokens there.
    let env = RunnerEnv::from_process();

    forget_secrets_in_environment();

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("hub-runner: {error}");

            return ExitCode::FAILURE;
        }
    };
    let outcome = runtime.block_on(runner::headless(&env, |_, _| {}));

    if let Err(error) = &outcome {
        eprintln!("hub-runner: {error}");
    }

    // Left at once: a round still talking to the server or a clone still running must not
    // hold the process past the few seconds a platform gives it to end.
    std::process::exit(i32::from(outcome.is_err()))
}
