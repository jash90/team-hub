//! SQLite: one writing connection behind a lock, and a read-only connection per thread.
//!
//! Reads run on whichever thread asks, usually a Rayon worker, and several can run at once
//! (WAL). A read inside a write does not see the write; code running in a transaction must use
//! the transaction it was given.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Params, Row, Transaction};

use crate::error::{ApiError, ApiResult};

static NEXT_DB: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static READERS: RefCell<HashMap<u64, Rc<Connection>>> = RefCell::new(HashMap::new());
}

pub struct Db {
    id: u64,
    path: PathBuf,
    writer: Mutex<Connection>,
}

fn configure(connection: &Connection) -> rusqlite::Result<()> {
    connection.busy_timeout(std::time::Duration::from_secs(10))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;

    Ok(())
}

impl Db {
    pub fn open(path: &Path) -> ApiResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let writer = Connection::open(path)?;
        writer.pragma_update(None, "journal_mode", "WAL")?;
        configure(&writer)?;
        writer.execute_batch(
            "CREATE TABLE IF NOT EXISTS migration (
                namespace TEXT NOT NULL,
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL,
                PRIMARY KEY (namespace, version)
            )",
        )?;

        Ok(Self { id: NEXT_DB.fetch_add(1, Ordering::Relaxed), path: path.to_path_buf(), writer: Mutex::new(writer) })
    }

    fn reader(&self) -> ApiResult<Rc<Connection>> {
        READERS.with(|readers| {
            if let Some(connection) = readers.borrow().get(&self.id) {
                return Ok(connection.clone());
            }

            let connection = Connection::open_with_flags(
                &self.path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
            )?;
            configure(&connection)?;
            let connection = Rc::new(connection);
            readers.borrow_mut().insert(self.id, connection.clone());

            Ok(connection)
        })
    }

    pub fn read<T>(&self, work: impl FnOnce(&Connection) -> ApiResult<T>) -> ApiResult<T> {
        let connection = self.reader()?;

        work(&connection)
    }

    /// Runs `work` in a transaction; an error rolls everything back.
    pub fn write<T>(&self, work: impl FnOnce(&Transaction) -> ApiResult<T>) -> ApiResult<T> {
        let mut writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let transaction = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let result = work(&transaction)?;
        transaction.commit()?;

        Ok(result)
    }

    /// Applies the migrations of `namespace` that have not run yet, in order.
    pub fn migrate(&self, namespace: &str, migrations: &[&str]) -> ApiResult<()> {
        for (index, sql) in migrations.iter().enumerate() {
            let version = index as i64 + 1;

            self.write(|tx| {
                let applied: Option<i64> = tx
                    .query_row("SELECT 1 FROM migration WHERE namespace = ?1 AND version = ?2", (namespace, version), |row| row.get(0))
                    .optional()?;

                if applied.is_some() {
                    return Ok(());
                }

                tx.execute_batch(sql)
                    .map_err(|error| ApiError::internal(format!("migration {namespace}/{version}: {error}")))?;
                tx.execute("INSERT INTO migration (namespace, version, applied_at) VALUES (?1, ?2, ?3)", (namespace, version, now_iso()))?;

                Ok(())
            })?;
        }

        Ok(())
    }
}

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn query_all<T>(
    connection: &Connection,
    sql: &str,
    params: impl Params,
    map: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> ApiResult<Vec<T>> {
    let mut statement = connection.prepare_cached(sql)?;
    let rows = statement.query_map(params, map)?.collect::<rusqlite::Result<Vec<T>>>()?;

    Ok(rows)
}

pub fn query_one<T>(
    connection: &Connection,
    sql: &str,
    params: impl Params,
    map: impl FnOnce(&Row) -> rusqlite::Result<T>,
) -> ApiResult<Option<T>> {
    let mut statement = connection.prepare_cached(sql)?;

    Ok(statement.query_row(params, map).optional()?)
}

/// `?, ?, ?` for an `IN (...)` list of `count` values.
pub fn placeholders(count: usize) -> String {
    vec!["?"; count.max(1)].join(", ")
}
