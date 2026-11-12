//! The application behind the API: storage, sign-in, the services that change things, and
//! the host that modules plug into. Request work runs on the Rayon pool (`App::run`).

pub mod access;
pub mod app;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod module;
pub mod schema;
pub mod secrets;
pub mod seed;
pub mod services;

pub use app::{Actor, App, Event, Tx};
pub use auth::AuthUser;
pub use config::Config;
pub use error::{ApiError, ApiResult};
pub use module::{FieldSql, Module, ModuleMeta, QueryScope, ToolSpec};
