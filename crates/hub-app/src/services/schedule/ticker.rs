//! The minute-by-minute check that runs the two daily slots.
//!
//! Catch-up, not cron (`hub_core::schedule::due_for`): each check asks, for every project,
//! whether a slot's time has passed today and the slot has not run for today yet. So a server
//! that was down over a slot's time runs it once when it is back. The clock is handed in:
//! the server passes the real one, tests pass their own and never wait.

use chrono::{DateTime, NaiveDate, Utc};
use hub_core::schedule::{due_for, TaskSlot};
use hub_core::Id;
use rusqlite::Connection;

use super::running::{begin_slot, carry_out, fail_interrupted, give_up_on_stuck, iso, ACTOR};
use super::settings::settings;
use crate::app::{Actor, App, Tx};
use crate::db::query_all;
use crate::error::ApiResult;

const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(60);
/// The slots that run by themselves, in the order of the clock: a server that comes back in
/// the evening runs the morning's tasks before the afternoon's.
const DAILY: [TaskSlot; 2] = [TaskSlot::Morning, TaskSlot::Afternoon];

pub struct Ticker {
    app: App,
    clock: Box<dyn Fn() -> DateTime<Utc> + Send>,
}

/// Remembers that the project's slot has run for `day`, a date in the schedule's time zone.
fn mark_ran(tx: &Tx, project_id: Id, slot: TaskSlot, day: NaiveDate) -> ApiResult<()> {
    tx.execute(
        "INSERT INTO schedule_slot_run (project_id, slot, ran_for, ran_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (project_id, slot) DO UPDATE SET ran_for = ?3, ran_at = ?4",
        (project_id, slot.as_str(), day.to_string(), &tx.now),
    )?;

    Ok(())
}

/// Every project that is not archived, with the day `slot` last ran for in it.
fn last_runs(connection: &Connection, slot: TaskSlot) -> ApiResult<Vec<(Id, String, Option<NaiveDate>)>> {
    query_all(
        connection,
        "SELECT p.id, p.key, r.ran_for FROM project p LEFT JOIN schedule_slot_run r ON r.project_id = p.id AND r.slot = ?1
         WHERE p.archived = 0 ORDER BY p.id",
        [slot.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, Option<String>>(2)?.and_then(|day| day.parse().ok()))),
    )
}

impl Ticker {
    pub fn new(app: &App, clock: impl Fn() -> DateTime<Utc> + Send + 'static) -> Self {
        Self { app: app.clone(), clock: Box::new(clock) }
    }

    /// Remembers the slot as run for `day` and starts its tasks, one after another. The day
    /// is remembered first, in the transaction that marks the tasks: a slot never runs twice.
    fn run(&self, project_id: Id, slot: TaskSlot, day: NaiveDate, now: DateTime<Utc>) -> ApiResult<()> {
        let started = self.app.write_at(Actor::system(ACTOR), iso(now), |tx| {
            mark_ran(tx, project_id, slot, day)?;

            begin_slot(&self.app, tx, project_id, slot)
        })?;

        for task in started {
            carry_out(&self.app, task.id, &self.clock);
        }

        Ok(())
    }

    /// One check: gives up on tasks that have been running for too long, then runs every
    /// slot that is due, project by project. Answers with what ran, as (project key, slot).
    /// A project whose slot cannot be run is skipped and tried again at the next check.
    pub fn tick(&self) -> ApiResult<Vec<(String, TaskSlot)>> {
        let now = (self.clock)();
        give_up_on_stuck(&self.app, now)?;

        let settings = self.app.read(settings)?;
        let mut ran = Vec::new();

        for slot in DAILY {
            for (project_id, key, last) in self.app.read(|connection| last_runs(connection, slot))? {
                let Some(day) = due_for(&settings, slot, now, last) else { continue };

                match self.run(project_id, slot, day, now) {
                    Ok(()) => ran.push((key, slot)),
                    Err(error) => tracing::warn!("schedule: the {slot} slot of {key} did not run: {}", error.message),
                }
            }
        }

        Ok(ran)
    }

    /// Remembers every slot that is due right now as run, without running anything. For
    /// example data, which appears all at once: a demo starts as if it had been up all day,
    /// instead of running its morning's tasks the minute it is made.
    pub fn skip_due(&self) -> ApiResult<()> {
        let now = (self.clock)();
        let settings = self.app.read(settings)?;

        self.app.write_at(Actor::system(ACTOR), iso(now), |tx| {
            for slot in DAILY {
                for (project_id, _, last) in last_runs(tx, slot)? {
                    if let Some(day) = due_for(&settings, slot, now, last) {
                        mark_ran(tx, project_id, slot, day)?;
                    }
                }
            }

            Ok(())
        })
    }

    /// For when the server comes up: fails the tasks that its stopping cut off while they
    /// were starting, which would otherwise look as if they were running.
    pub fn recover(&self) -> ApiResult<()> {
        fail_interrupted(&self.app, (self.clock)())
    }

    /// Checks once a minute, on a thread of its own, from now on. The first check is at
    /// once: that is the catch-up after the server was down.
    pub fn start(self) {
        let spawned = std::thread::Builder::new().name("schedule".to_string()).spawn(move || {
            if let Err(error) = self.recover() {
                tracing::warn!("schedule: {}", error.message);
            }

            loop {
                // A check that fails is tried again in a minute; one that stops for good would lose every slot after it.
                if let Err(error) = self.tick() {
                    tracing::warn!("schedule: {}", error.message);
                }

                std::thread::sleep(CHECK_EVERY);
            }
        });

        if let Err(error) = spawned {
            tracing::error!("could not start the schedule: {error}");
        }
    }
}
