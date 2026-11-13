//! The application: database, configuration, modules, and the two ways work gets done:
//! `run` (move a request onto the Rayon pool) and `write` (a transaction that emits events).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Deref;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, RwLock};

use hub_core::{Id, ModuleInfo};
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::db::{now_iso, query_one, Db};
use crate::error::{ApiError, ApiResult};
use crate::module::Module;
use crate::schema;

/// Who is doing something: a signed-in person, or a system acting under a label.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Actor {
    pub user_id: Option<Id>,
    pub label: Option<String>,
}

impl Actor {
    pub fn user(id: Id) -> Self {
        Self { user_id: Some(id), label: None }
    }

    pub fn system(label: impl Into<String>) -> Self {
        Self { user_id: None, label: Some(label.into()) }
    }

    /// A known person acting through another system, e.g. a commit author matched to a user.
    pub fn user_via(id: Id, label: impl Into<String>) -> Self {
        Self { user_id: Some(id), label: Some(label.into()) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: Id,
    pub at: String,
    pub actor_id: Option<Id>,
    pub actor_label: Option<String>,
    pub kind: String,
    pub project_id: Option<Id>,
    pub issue_id: Option<Id>,
    pub payload: serde_json::Value,
    /// True for an event only administrators may hear on the live stream. An event about a
    /// project needs no such mark: it reaches whoever can see the project.
    #[serde(skip)]
    pub admins_only: bool,
}

/// A transaction in progress. Dereferences to the connection, and collects the events to
/// announce once it commits.
pub struct Tx<'a> {
    connection: &'a Connection,
    pub actor: Actor,
    pub now: String,
    events: RefCell<Vec<Event>>,
}

impl Deref for Tx<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.connection
    }
}

impl Tx<'_> {
    pub fn emit(&self, kind: &str, project_id: Option<Id>, issue_id: Option<Id>, payload: serde_json::Value) -> ApiResult<()> {
        self.announce(kind, project_id, issue_id, payload, false)
    }

    /// An event about the instance's administration (an import under way, say), which is
    /// about no project and is nobody else's business.
    pub fn emit_to_admins(&self, kind: &str, payload: serde_json::Value) -> ApiResult<()> {
        self.announce(kind, None, None, payload, true)
    }

    fn announce(&self, kind: &str, project_id: Option<Id>, issue_id: Option<Id>, payload: serde_json::Value, admins_only: bool) -> ApiResult<()> {
        self.connection.execute(
            "INSERT INTO event (at, actor_id, actor_label, kind, project_id, issue_id, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (&self.now, self.actor.user_id, &self.actor.label, kind, project_id, issue_id, payload.to_string()),
        )?;
        self.events.borrow_mut().push(Event {
            id: self.connection.last_insert_rowid(),
            at: self.now.clone(),
            actor_id: self.actor.user_id,
            actor_label: self.actor.label.clone(),
            kind: kind.to_string(),
            project_id,
            issue_id,
            payload,
            admins_only,
        });

        Ok(())
    }
}

pub struct AppInner {
    pub db: Db,
    pub config: Config,
    pub modules: Vec<Arc<dyn Module>>,
    enabled: RwLock<HashMap<String, bool>>,
    stream: broadcast::Sender<Event>,
}

#[derive(Clone)]
pub struct App(Arc<AppInner>);

impl Deref for App {
    type Target = AppInner;

    fn deref(&self) -> &AppInner {
        &self.0
    }
}

fn module_setting(id: &str) -> String {
    format!("module:{id}")
}

impl App {
    pub fn new(config: Config, modules: Vec<Arc<dyn Module>>) -> ApiResult<Self> {
        let db = Db::open(&config.database_path())?;
        db.migrate("core", schema::MIGRATIONS)?;

        let mut enabled = HashMap::new();

        for module in &modules {
            let meta = module.meta();
            db.migrate(meta.id, module.migrations())?;

            let stored = db.read(|connection| get_setting::<bool>(connection, &module_setting(meta.id)))?;
            enabled.insert(meta.id.to_string(), stored.unwrap_or(meta.default_enabled));
        }

        let (stream, _) = broadcast::channel(256);

        Ok(Self(Arc::new(AppInner { db, config, modules, enabled: RwLock::new(enabled), stream })))
    }

    /// Runs request work on the Rayon pool, off the async runtime. A panic becomes an error
    /// for this request only.
    pub async fn run<T, F>(&self, work: F) -> ApiResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&App) -> ApiResult<T> + Send + 'static,
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let app = self.clone();

        rayon::spawn(move || {
            let outcome =
                std::panic::catch_unwind(AssertUnwindSafe(|| work(&app))).unwrap_or_else(|_| Err(ApiError::internal("the request handler panicked")));
            let _ = sender.send(outcome);
        });

        receiver.await.map_err(|_| ApiError::internal("the worker pool dropped the request"))?
    }

    pub fn read<T>(&self, work: impl FnOnce(&Connection) -> ApiResult<T>) -> ApiResult<T> {
        self.db.read(work)
    }

    /// One transaction on behalf of `actor`. Events emitted inside reach the modules and the
    /// live stream only after it has committed.
    pub fn write<T>(&self, actor: Actor, work: impl FnOnce(&Tx) -> ApiResult<T>) -> ApiResult<T> {
        self.write_at(actor, now_iso(), work)
    }

    /// `write` with the clock set to `now`; for imports and example data that happened earlier.
    pub fn write_at<T>(&self, actor: Actor, now: String, work: impl FnOnce(&Tx) -> ApiResult<T>) -> ApiResult<T> {
        let (result, events) = self.db.write(|transaction| {
            let tx = Tx { connection: transaction, actor, now, events: RefCell::new(Vec::new()) };
            let result = work(&tx)?;

            Ok((result, tx.events.into_inner()))
        })?;

        self.dispatch(events);

        Ok(result)
    }

    fn dispatch(&self, events: Vec<Event>) {
        for event in events {
            // A module hears nothing about a project that has switched it off.
            let off = event
                .project_id
                .and_then(|project_id| self.read(|connection| crate::services::project_modules::disabled_modules(connection, project_id)).ok())
                .unwrap_or_default();

            for module in self.enabled_modules().filter(|module| !off.iter().any(|id| id == module.meta().id)) {
                let handled = std::panic::catch_unwind(AssertUnwindSafe(|| module.on_event(self, &event)));

                if handled.is_err() {
                    tracing::error!(module = module.meta().id, kind = event.kind, "module panicked on an event");
                }
            }

            let _ = self.stream.send(event);
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.stream.subscribe()
    }

    pub fn module_enabled(&self, id: &str) -> bool {
        self.enabled.read().map(|enabled| enabled.get(id).copied().unwrap_or(false)).unwrap_or(false)
    }

    pub fn enabled_modules(&self) -> impl Iterator<Item = &Arc<dyn Module>> {
        self.modules.iter().filter(|module| self.module_enabled(module.meta().id))
    }

    pub fn module_infos(&self) -> Vec<ModuleInfo> {
        self.modules
            .iter()
            .map(|module| {
                let meta = module.meta();

                ModuleInfo {
                    id: meta.id.to_string(),
                    name: meta.name.to_string(),
                    description: meta.description.to_string(),
                    version: meta.version.to_string(),
                    enabled: self.module_enabled(meta.id),
                    depends_on: meta.depends_on.iter().map(|id| id.to_string()).collect(),
                }
            })
            .collect()
    }

    /// Switches a module on or off. A module cannot run without the ones it depends on, and
    /// one that others depend on cannot be switched off under them.
    pub fn set_module_enabled(&self, id: &str, enabled: bool) -> ApiResult<()> {
        let module = self.modules.iter().find(|module| module.meta().id == id).ok_or_else(|| ApiError::not_found("Module"))?;

        if enabled {
            if let Some(missing) = module.meta().depends_on.iter().find(|dependency| !self.module_enabled(dependency)) {
                return Err(ApiError::conflict(format!("Enable the \"{missing}\" module first")));
            }
        } else if let Some(dependant) = self.enabled_modules().find(|other| other.meta().depends_on.contains(&id)) {
            return Err(ApiError::conflict(format!("The \"{}\" module depends on this one", dependant.meta().id)));
        }

        self.db.write(|tx| set_setting(tx, &module_setting(id), &enabled))?;

        if let Ok(mut map) = self.enabled.write() {
            map.insert(id.to_string(), enabled);
        }

        Ok(())
    }

    pub fn start_modules(&self) {
        for module in self.enabled_modules() {
            module.start(self);
        }
    }
}

pub fn get_setting<T: DeserializeOwned>(connection: &Connection, key: &str) -> ApiResult<Option<T>> {
    let text: Option<String> = query_one(connection, "SELECT value FROM setting WHERE key = ?1", [key], |row| row.get(0))?;

    Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
}

pub fn set_setting<T: Serialize>(connection: &Connection, key: &str, value: &T) -> ApiResult<()> {
    connection.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
        (key, serde_json::to_string(value)?),
    )?;

    Ok(())
}
