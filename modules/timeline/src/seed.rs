//! Dates for the demo, so the timeline of the shop has bars around today: its epics, and a
//! few issues the demo already gives a due date. The API project's epic is left without any
//! dates: its row shows the span estimated from its children.

use chrono::{Duration, SecondsFormat, Utc};
use hub_app::db::query_one;
use hub_app::{Actor, ApiResult, App};
use hub_core::Id;

use crate::span::set_span;

struct Plan {
    key: &'static str,
    /// The days, relative to today, on which the work starts and is due.
    start: i64,
    due: i64,
    /// The day the dates were set.
    planned: i64,
}

const PLANS: &[Plan] = &[
    Plan { key: "WEB-1", start: -20, due: 16, planned: -27 },
    Plan { key: "WEB-2", start: -17, due: 30, planned: -27 },
    Plan { key: "WEB-3", start: 10, due: 45, planned: -27 },
    Plan { key: "WEB-9", start: -5, due: 3, planned: -4 },
    Plan { key: "WEB-12", start: -4, due: 6, planned: -4 },
    Plan { key: "WEB-15", start: -2, due: -1, planned: -2 },
];

fn date(day: i64) -> String {
    (Utc::now() + Duration::days(day)).format("%Y-%m-%d").to_string()
}

pub fn seed_demo(app: &App) -> ApiResult<()> {
    for plan in PLANS {
        let found: Option<(Id, Option<Id>)> = app.read(|connection| {
            query_one(connection, "SELECT id, reporter_id FROM issue WHERE key = ?1", [plan.key], |row| Ok((row.get(0)?, row.get(1)?)))
        })?;
        let Some((issue_id, reporter)) = found else { continue };
        let actor = reporter.map(Actor::user).unwrap_or_else(|| Actor::system("Timeline"));
        let planned = (Utc::now() + Duration::days(plan.planned)).to_rfc3339_opts(SecondsFormat::Millis, true);

        app.write_at(actor, planned, |tx| set_span(tx, issue_id, Some(&date(plan.start)), Some(&date(plan.due))))?;
    }

    Ok(())
}
