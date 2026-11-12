//! Rules and types of team-hub. No network, no disk: everything here is a function of its
//! arguments, so the server, the modules and the tests share one definition of each rule.

pub mod dev;
pub mod diff;
pub mod keys;
pub mod query;
pub mod rank;
pub mod release_notes;
pub mod smart_commits;
pub mod text;
pub mod types;
pub mod workflow;

pub use types::*;
