//! Dates for the demo's epics, so the timeline of the shop has bars around today. The API
//! project's epic is left without any: its row shows the span estimated from its children.

use chrono::{Duration, SecondsFormat, Utc};
use hub_app::db::query_one;
use hub_app::{Actor, ApiResult, App};
use hub_core::Id;

use crate::span::set_span;

/// An epic and the days, relative to today, on which its work starts and is due.
const PLANS: &[(&str, i64, i64)] = &[("WEB-1", -20, 16), ("WEB-2", -17, 30), ("WEB-3", 10, 45)];

/// The epics were planned a few days after they were written down.
const PLANNED: i64 = -27;

fn date(day: i64) -> String {
    (Utc::now() + Duration::days(day)).format("%Y-%m-%d").to_string()
}

pub fn seed_demo(app: &App) -> ApiResult<()> {
    let planned = (Utc::now() + Duration::days(PLANNED)).to_rfc3339_opts(SecondsFormat::Millis, true);

    for (key, start, due) in PLANS {
        let found: Option<(Id, Option<Id>)> = app.read(|connection| {
            query_one(connection, "SELECT id, reporter_id FROM issue WHERE key = ?1", [key], |row| Ok((row.get(0)?, row.get(1)?)))
        })?;
        let Some((issue_id, reporter)) = found else { continue };
        let actor = reporter.map(Actor::user).unwrap_or_else(|| Actor::system("Timeline"));

        app.write_at(actor, planned.clone(), |tx| set_span(tx, issue_id, Some(&date(*start)), Some(&date(*due))))?;
    }

    Ok(())
}
