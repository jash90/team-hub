//! What a module is. Everything beyond the tracker's core (code hosting, the knowledge base,
//! imports, reports, agents) plugs in through this trait and can be switched off.

use axum::Router;
use chrono::{DateTime, Utc};
use hub_core::query::{Clause, QueryFieldInfo};
use hub_core::runners::{PersonalKey, Runner};
use hub_core::schedule::{ScheduledTask, TaskKind, TaskStarted};
use hub_core::{DemoLanguage, IssueRow, QuickHit};
use rusqlite::types::Value;
use rusqlite::Connection;

use crate::app::{App, Event};
use crate::auth::AuthUser;
use crate::error::ApiResult;

#[derive(Debug, Clone, Copy)]
pub struct ModuleMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub version: &'static str,
    pub depends_on: &'static [&'static str],
    pub default_enabled: bool,
}

/// A condition on the issue aliased `i`, with the values for its `?` placeholders in order.
#[derive(Debug, Clone, Default)]
pub struct FieldSql {
    pub sql: String,
    pub params: Vec<Value>,
}

/// A tool a module offers to AI agents (served over MCP by the `agents` module).
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the arguments object.
    pub input_schema: serde_json::Value,
    /// True when calling the tool changes nothing.
    pub read_only: bool,
}

pub struct QueryScope<'a> {
    pub user: &'a AuthUser,
    pub now: DateTime<Utc>,
}

pub trait Module: Send + Sync + 'static {
    fn meta(&self) -> ModuleMeta;

    /// The module's own tables; applied in order, each once.
    fn migrations(&self) -> &'static [&'static str] {
        &[]
    }

    /// Routes under `/api/m/<id>`; callers are signed in.
    fn routes(&self) -> Option<Router<App>> {
        None
    }

    /// Routes under `/hooks/<id>` for other systems to call; the module authenticates them itself.
    fn hooks(&self) -> Option<Router<App>> {
        None
    }

    /// Called after the transaction that produced the event has committed.
    fn on_event(&self, _app: &App, _event: &Event) {}

    /// Fields this module adds to the query language.
    fn query_fields(&self) -> Vec<QueryFieldInfo> {
        Vec::new()
    }

    /// SQL for a clause on one of this module's fields; none when the field is not the module's.
    fn compile_field(&self, _connection: &Connection, _clause: &Clause, _scope: &QueryScope) -> Option<ApiResult<FieldSql>> {
        None
    }

    /// Adds this module's entry to `ext` of each row (badges on cards and lists). The host
    /// drops the entry again for rows of a project where the module is switched off.
    fn decorate_issues(&self, _connection: &Connection, _rows: &mut [IssueRow]) -> ApiResult<()> {
        Ok(())
    }

    /// Data for the module's panel on an issue's page. Not asked for issues of a project
    /// where the module is switched off.
    fn issue_panel(&self, _app: &App, _connection: &Connection, _issue: &IssueRow, _viewer: &AuthUser) -> ApiResult<Option<serde_json::Value>> {
        Ok(None)
    }

    fn quick_search(&self, _connection: &Connection, _viewer: &AuthUser, _text: &str) -> ApiResult<Vec<QuickHit>> {
        Ok(Vec::new())
    }

    /// The module's section of "my work".
    fn dashboard(&self, _connection: &Connection, _viewer: &AuthUser) -> ApiResult<Option<serde_json::Value>> {
        Ok(None)
    }

    /// Tools this module offers to AI agents.
    fn tools(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    /// Runs one of this module's tools as `user`; none when the tool is not the module's.
    fn call_tool(&self, _app: &App, _user: &AuthUser, _name: &str, _arguments: &serde_json::Value) -> Option<ApiResult<serde_json::Value>> {
        None
    }

    /// What this module can run on a schedule (`docs/schedule.md`). The id of a kind is
    /// "<module id>.<name>", and its `module` the module's id; anything else is left out.
    fn task_kinds(&self) -> Vec<TaskKind> {
        Vec::new()
    }

    /// Starts a task of one of this module's kinds; none when the kind is not the module's.
    /// `actor` is the person the task runs as, acting through this module, so the usual
    /// access checks apply. Called outside any transaction, and never while the module is
    /// off for the instance or for the task's project. Starting is quick: work that takes
    /// long is handed on, the answer is `TaskStarted::Running`, and the module reports the
    /// end with `services::schedule::finish`.
    fn start_task(&self, _app: &App, _task: &ScheduledTask, _actor: &AuthUser) -> Option<ApiResult<TaskStarted>> {
        None
    }

    /// The runners this module knows (`docs/runners.md`), each with the one kind of work
    /// the module gives out: the viewer's own, and other people's while they are connected.
    /// The core makes one list of what the modules say.
    fn runners(&self, _connection: &Connection, _viewer: &AuthUser, _now: DateTime<Utc>) -> ApiResult<Vec<Runner>> {
        Ok(Vec::new())
    }

    /// Keys a person stores for themselves and this module uses on their behalf, such as
    /// their own model key (`docs/runners.md` → Shared runners). The core keeps the values
    /// (`services::personal_keys`); a key two modules name is listed once.
    fn personal_keys(&self) -> Vec<PersonalKey> {
        Vec::new()
    }

    /// Background work; called once the application is up.
    fn start(&self, _app: &App) {}

    /// Example data for a demo instance, written in `language`: the same data in each
    /// language, only the texts a person reads differ.
    fn seed_demo(&self, _app: &App, _language: DemoLanguage) -> ApiResult<()> {
        Ok(())
    }
}
