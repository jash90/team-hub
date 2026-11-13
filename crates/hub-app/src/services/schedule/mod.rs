//! The schedule (`docs/schedule.md`): recurring work of a project, run with one of two daily
//! slots or on demand. The core owns the tasks, their state and when they run; what a task
//! does is its kind's business, and kinds belong to modules (`Module::task_kinds`,
//! `Module::start_task`).
//!
//! One file per part: the tasks themselves, running them, the instance's settings, and the
//! minute-by-minute check. Everything public is re-exported here.

mod running;
mod settings;
mod tasks;
mod ticker;

pub use running::{finish, run_slot, run_task, Outcome};
pub use settings::{save_settings, settings};
pub use tasks::{create, delete, list, task_kinds, update, NewTask, TaskPatch};
pub use ticker::Ticker;
