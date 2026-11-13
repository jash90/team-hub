//! The team-hub desktop app without its window. Connection and offline copy, repositories,
//! agent threads in git working trees, the runner side of `docs/agent-runs.md`, and the rule
//! that nothing leaves the machine without a person's click.

pub mod api;
pub mod app;
pub mod commands;
pub mod config;
pub mod diff;
pub mod error;
pub mod git;
pub mod headless;
pub mod jobs;
pub mod model;
pub mod policy;
pub mod protocol;
pub mod push_guard;
pub mod repos;
pub mod runner;
pub mod runs;
pub mod secrets;
pub mod store;
pub mod threads;

pub use app::{Core, Desktop, OpenOptions};
pub use error::{CoreError, CoreResult};
