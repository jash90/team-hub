//! The schedule (`docs/schedule.md`): the shapes of a recurring task and of the kinds modules
//! offer, and the rule that says when a slot is due.
//!
//! Catch-up, not cron: a slot is due when its time has passed today, in the schedule's time
//! zone, and it has not been run for today yet. Nothing here reads a clock; "now" is an
//! argument, so the rule is the same in the server and in its tests.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{string_enum, Id};

string_enum!(
    /// When a task runs: with one of the two daily slots, or only from its button.
    TaskSlot { Morning => "morning", Afternoon => "afternoon", OnDemand => "on_demand" }
);
string_enum!(
    /// `ok` and `failed` say how the last attempt ended; `idle` that there has been none.
    TaskState { Idle => "idle", Running => "running", Ok => "ok", Failed => "failed" }
);
string_enum!(
    /// What a field of a kind's form holds. `query` is text in the tracker's query language.
    TaskFieldType { Text => "text", Textarea => "textarea", Query => "query", Select => "select", Boolean => "boolean", Number => "number" }
);

impl Default for TaskFieldType {
    fn default() -> Self {
        Self::Text
    }
}

/// The last attempt that worked. A later failure leaves it as it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskResult {
    pub at: String,
    pub summary: String,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScheduledTask {
    pub id: Id,
    pub project_key: String,
    /// "<module>.<name>", e.g. "agents.run".
    pub kind: String,
    pub name: String,
    /// What the kind's form holds.
    #[ts(type = "Record<string, unknown>")]
    pub config: serde_json::Map<String, serde_json::Value>,
    pub slot: TaskSlot,
    pub enabled: bool,
    /// The person the task runs as: whoever made it, or last changed what it does.
    pub run_as: Id,
    pub state: TaskState,
    pub last_started_at: Option<String>,
    pub last_finished_at: Option<String>,
    /// Why the last attempt failed.
    pub last_error: Option<String>,
    /// What the module said when it started work that is still going on.
    pub progress: Option<String>,
    pub result: Option<TaskResult>,
    /// False when the module is off or gone: the task is shown, never run.
    pub kind_available: bool,
}

/// Something a module can run on a schedule.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskKind {
    /// "<module>.<name>".
    pub id: String,
    pub module: String,
    pub name: String,
    pub description: String,
    /// The form for a task's `config`.
    pub fields: Vec<TaskField>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskField {
    /// The key in `config`.
    pub name: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hint: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(rename = "type")]
    pub kind: TaskFieldType,
    /// The choices of a `select`.
    #[serde(default)]
    pub options: Vec<TaskOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub default: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskOption {
    pub value: String,
    pub label: String,
}

/// What a module answers when asked to start a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum TaskStarted {
    /// The work is over, or was handed on and needs no follow-up.
    Done { summary: String, link: Option<String> },
    /// The work goes on; the module reports its end itself.
    Running { summary: String },
}

/// When the two daily slots run, for the whole instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScheduleSettings {
    /// "HH:MM" in `timezone`.
    pub morning: String,
    pub afternoon: String,
    /// An IANA name, e.g. "Europe/Warsaw".
    pub timezone: String,
    pub weekdays_only: bool,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self { morning: "10:00".to_string(), afternoon: "16:00".to_string(), timezone: "Europe/Warsaw".to_string(), weekdays_only: true }
    }
}

/// "HH:MM" as a time of day; none for anything else.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let (hour, minute) = text.trim().split_once(':')?;
    let digits = |part: &str| (1..=2).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit());

    if !digits(hour) || minute.len() != 2 || !digits(minute) {
        return None;
    }

    NaiveTime::from_hms_opt(hour.parse().ok()?, minute.parse().ok()?, 0)
}

/// The time zone an IANA name stands for; none when there is no such zone.
pub fn parse_zone(name: &str) -> Option<Tz> {
    name.trim().parse().ok()
}

impl ScheduleSettings {
    /// The time of day a slot runs at; none for "on demand", and for a time that is not one.
    pub fn time_of(&self, slot: TaskSlot) -> Option<NaiveTime> {
        match slot {
            TaskSlot::Morning => parse_time(&self.morning),
            TaskSlot::Afternoon => parse_time(&self.afternoon),
            TaskSlot::OnDemand => None,
        }
    }
}

/// The day `slot` is due for at `now`: today's date in the schedule's time zone, once the
/// slot's time has passed and unless it already ran for that day (`last_ran_for`). None when
/// it is not due.
///
/// Only ever today: a server that was down for days owes one run, not one per day missed, and
/// one that was down all day owes nothing for that day the next morning. The comparison is on
/// the wall clock, so a time the clock skips (the night summer time starts) is due as soon as
/// the clock is past it, and a time that comes twice (the night it ends) runs once.
pub fn due_for(settings: &ScheduleSettings, slot: TaskSlot, now: DateTime<Utc>, last_ran_for: Option<NaiveDate>) -> Option<NaiveDate> {
    let at = settings.time_of(slot)?;
    let local = now.with_timezone(&parse_zone(&settings.timezone)?);
    let today = local.date_naive();

    if settings.weekdays_only && matches!(local.weekday(), Weekday::Sat | Weekday::Sun) {
        return None;
    }

    // A day at or after today is also "already run": the zone may have been changed since.
    if local.time() < at || last_ran_for.is_some_and(|last| last >= today) {
        return None;
    }

    Some(today)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn warsaw() -> ScheduleSettings {
        ScheduleSettings::default()
    }

    fn every_day(timezone: &str) -> ScheduleSettings {
        ScheduleSettings { timezone: timezone.to_string(), weekdays_only: false, ..ScheduleSettings::default() }
    }

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).expect("an instant").with_timezone(&Utc)
    }

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    /// An instant given as a wall-clock time in a zone, so that the tests read like a clock.
    fn local(timezone: &str, text: &str) -> DateTime<Utc> {
        let zone: Tz = timezone.parse().expect("a zone");
        let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("a local time");

        zone.from_local_datetime(&naive).single().expect("an unambiguous local time").with_timezone(&Utc)
    }

    #[test]
    fn times_are_hours_and_minutes() {
        assert_eq!(parse_time("10:00"), NaiveTime::from_hms_opt(10, 0, 0));
        assert_eq!(parse_time("9:05"), NaiveTime::from_hms_opt(9, 5, 0));
        assert_eq!(parse_time(" 23:59 "), NaiveTime::from_hms_opt(23, 59, 0));

        for wrong in ["", "10", "24:00", "10:60", "10:5", "ten:00", "10:00:00", "-1:00", "+1:00", "100:00"] {
            assert_eq!(parse_time(wrong), None, "{wrong:?}");
        }
    }

    #[test]
    fn zones_are_iana_names() {
        assert!(parse_zone("Europe/Warsaw").is_some());
        assert!(parse_zone("UTC").is_some());
        assert!(parse_zone("Warsaw").is_none());
        assert!(parse_zone("Europe/Atlantis").is_none());
        assert!(parse_zone("").is_none());
    }

    #[test]
    fn nothing_is_due_before_its_time() {
        // Friday 13 November 2026.
        let settings = warsaw();

        assert_eq!(due_for(&settings, TaskSlot::Morning, local("Europe/Warsaw", "2026-11-13 09:59"), None), None);
        assert_eq!(due_for(&settings, TaskSlot::Afternoon, local("Europe/Warsaw", "2026-11-13 15:59"), None), None);
    }

    #[test]
    fn a_slot_is_due_from_its_minute_on() {
        let settings = warsaw();

        assert_eq!(due_for(&settings, TaskSlot::Morning, local("Europe/Warsaw", "2026-11-13 10:00"), None), Some(day("2026-11-13")));
        assert_eq!(due_for(&settings, TaskSlot::Morning, local("Europe/Warsaw", "2026-11-13 14:20"), None), Some(day("2026-11-13")));
        assert_eq!(due_for(&settings, TaskSlot::Afternoon, local("Europe/Warsaw", "2026-11-13 14:20"), None), None);
        assert_eq!(due_for(&settings, TaskSlot::Afternoon, local("Europe/Warsaw", "2026-11-13 23:59"), None), Some(day("2026-11-13")));
    }

    #[test]
    fn a_slot_runs_once_a_day() {
        let settings = warsaw();
        let noon = local("Europe/Warsaw", "2026-11-13 12:00");

        assert_eq!(due_for(&settings, TaskSlot::Morning, noon, Some(day("2026-11-13"))), None);
        assert_eq!(due_for(&settings, TaskSlot::Morning, noon, Some(day("2026-11-12"))), Some(day("2026-11-13")), "yesterday's run does not count");
    }

    #[test]
    fn on_demand_is_never_due() {
        let settings = every_day("Europe/Warsaw");

        assert_eq!(due_for(&settings, TaskSlot::OnDemand, local("Europe/Warsaw", "2026-11-13 23:00"), None), None);
    }

    #[test]
    fn the_weekend_is_quiet_with_weekdays_only() {
        let settings = warsaw();

        // Saturday 14 and Sunday 15 November 2026.
        for date in ["2026-11-14 12:00", "2026-11-15 18:00"] {
            assert_eq!(due_for(&settings, TaskSlot::Morning, local("Europe/Warsaw", date), None), None, "{date}");
            assert_eq!(due_for(&settings, TaskSlot::Afternoon, local("Europe/Warsaw", date), Some(day("2026-11-13"))), None, "{date}");
        }

        assert_eq!(due_for(&settings, TaskSlot::Morning, local("Europe/Warsaw", "2026-11-16 10:00"), Some(day("2026-11-13"))), Some(day("2026-11-16")));
    }

    #[test]
    fn the_weekend_runs_without_weekdays_only() {
        let settings = every_day("Europe/Warsaw");

        assert_eq!(due_for(&settings, TaskSlot::Morning, local("Europe/Warsaw", "2026-11-14 12:00"), None), Some(day("2026-11-14")));
        assert_eq!(due_for(&settings, TaskSlot::Afternoon, local("Europe/Warsaw", "2026-11-15 18:00"), Some(day("2026-11-14"))), Some(day("2026-11-15")));
    }

    #[test]
    fn today_is_the_day_in_the_schedules_zone_not_utc() {
        // 22:30 UTC on Thursday is 11:30 on Friday in Auckland: Friday's morning slot is due.
        let auckland = ScheduleSettings { timezone: "Pacific/Auckland".to_string(), ..ScheduleSettings::default() };
        let now = utc("2026-01-15T22:30:00Z");

        assert_eq!(due_for(&auckland, TaskSlot::Morning, now, Some(day("2026-01-15"))), Some(day("2026-01-16")));
        assert_eq!(due_for(&auckland, TaskSlot::Morning, now, Some(day("2026-01-16"))), None);

        // 20:30 UTC on Thursday is 09:30 on Friday there: not yet.
        assert_eq!(due_for(&auckland, TaskSlot::Morning, utc("2026-01-15T20:30:00Z"), Some(day("2026-01-15"))), None);
    }

    #[test]
    fn the_weekend_is_the_weekend_in_the_schedules_zone() {
        // 02:00 UTC on Saturday is still Friday evening in Los Angeles: a working day there.
        let angeles = ScheduleSettings { timezone: "America/Los_Angeles".to_string(), ..ScheduleSettings::default() };

        assert_eq!(due_for(&angeles, TaskSlot::Afternoon, utc("2026-01-17T02:00:00Z"), None), Some(day("2026-01-16")));

        // 21:30 UTC on Sunday is Monday morning in Auckland, and a working day already.
        let auckland = ScheduleSettings { timezone: "Pacific/Auckland".to_string(), ..ScheduleSettings::default() };

        assert_eq!(due_for(&auckland, TaskSlot::Morning, utc("2026-01-18T21:30:00Z"), None), Some(day("2026-01-19")));

        // And 22:00 UTC on Friday is Saturday there: quiet, although UTC says Friday.
        assert_eq!(due_for(&auckland, TaskSlot::Morning, utc("2026-01-16T22:00:00Z"), None), None);
    }

    #[test]
    fn the_slot_follows_the_wall_clock_when_summer_time_starts() {
        // In Warsaw the clock goes from 02:00 to 03:00 on Sunday 29 March 2026: ten o'clock
        // is 09:00 UTC the day before and 08:00 UTC on the day.
        let settings = every_day("Europe/Warsaw");

        assert_eq!(due_for(&settings, TaskSlot::Morning, utc("2026-03-28T08:59:00Z"), None), None);
        assert_eq!(due_for(&settings, TaskSlot::Morning, utc("2026-03-28T09:00:00Z"), None), Some(day("2026-03-28")));
        assert_eq!(due_for(&settings, TaskSlot::Morning, utc("2026-03-29T07:59:00Z"), Some(day("2026-03-28"))), None);
        assert_eq!(due_for(&settings, TaskSlot::Morning, utc("2026-03-29T08:00:00Z"), Some(day("2026-03-28"))), Some(day("2026-03-29")));
    }

    #[test]
    fn a_time_the_clock_skips_is_due_once_the_clock_is_past_it() {
        // 02:30 does not exist in Warsaw on 29 March 2026.
        let settings = ScheduleSettings { morning: "02:30".to_string(), ..every_day("Europe/Warsaw") };

        assert_eq!(due_for(&settings, TaskSlot::Morning, utc("2026-03-29T00:59:00Z"), Some(day("2026-03-28"))), None, "01:59, a minute before the jump");
        assert_eq!(due_for(&settings, TaskSlot::Morning, utc("2026-03-29T01:00:00Z"), Some(day("2026-03-28"))), Some(day("2026-03-29")), "03:00, right after");
    }

    #[test]
    fn a_time_that_comes_twice_runs_once() {
        // On Sunday 25 October 2026 the clock in Warsaw goes from 03:00 back to 02:00.
        let settings = ScheduleSettings { morning: "02:30".to_string(), ..every_day("Europe/Warsaw") };
        let first = utc("2026-10-25T00:30:00Z");
        let second = utc("2026-10-25T01:30:00Z");

        assert_eq!(due_for(&settings, TaskSlot::Morning, utc("2026-10-25T00:29:00Z"), Some(day("2026-10-24"))), None);
        assert_eq!(due_for(&settings, TaskSlot::Morning, first, Some(day("2026-10-24"))), Some(day("2026-10-25")));
        assert_eq!(due_for(&settings, TaskSlot::Morning, second, Some(day("2026-10-25"))), None);

        // The afternoon of that day is 15:00 UTC, an hour later than the day before.
        assert_eq!(due_for(&settings, TaskSlot::Afternoon, utc("2026-10-25T14:59:00Z"), None), None);
        assert_eq!(due_for(&settings, TaskSlot::Afternoon, utc("2026-10-25T15:00:00Z"), None), Some(day("2026-10-25")));
    }

    #[test]
    fn after_days_of_downtime_only_todays_slot_is_due() {
        // Last ran on Monday 9 November; the server comes back on Friday at noon.
        let settings = warsaw();
        let back = local("Europe/Warsaw", "2026-11-13 12:00");

        assert_eq!(due_for(&settings, TaskSlot::Morning, back, Some(day("2026-11-09"))), Some(day("2026-11-13")), "one run, for today");
        assert_eq!(due_for(&settings, TaskSlot::Afternoon, back, Some(day("2026-11-09"))), None, "the afternoons missed are gone, and today's is not due yet");
    }

    #[test]
    fn a_day_missed_entirely_is_not_made_up_the_next_morning() {
        // Down all Thursday; back on Friday at eight.
        let settings = warsaw();
        let back = local("Europe/Warsaw", "2026-11-13 08:00");

        assert_eq!(due_for(&settings, TaskSlot::Morning, back, Some(day("2026-11-11"))), None);
        assert_eq!(due_for(&settings, TaskSlot::Afternoon, back, Some(day("2026-11-11"))), None);
    }

    #[test]
    fn a_run_recorded_for_a_later_day_counts_as_done() {
        // The zone was changed from Auckland to Warsaw after Friday's run there.
        let settings = warsaw();

        assert_eq!(due_for(&settings, TaskSlot::Morning, local("Europe/Warsaw", "2026-01-15 12:00"), Some(day("2026-01-16"))), None);
    }

    #[test]
    fn settings_that_make_no_sense_are_never_due() {
        let noon = local("Europe/Warsaw", "2026-11-13 12:00");
        let no_zone = ScheduleSettings { timezone: "Nowhere/Else".to_string(), ..ScheduleSettings::default() };
        let no_time = ScheduleSettings { morning: "soon".to_string(), ..ScheduleSettings::default() };

        assert_eq!(due_for(&no_zone, TaskSlot::Morning, noon, None), None);
        assert_eq!(due_for(&no_time, TaskSlot::Morning, noon, None), None);
    }

    #[test]
    fn a_field_reads_and_writes_the_contracts_shape() {
        let field = TaskField { name: "limit".to_string(), label: "Limit".to_string(), kind: TaskFieldType::Number, default: Some(serde_json::json!(5)), ..TaskField::default() };
        let json = serde_json::to_value(&field).expect("json");

        assert_eq!(json, serde_json::json!({ "name": "limit", "label": "Limit", "required": false, "type": "number", "options": [], "default": 5 }));
        assert_eq!(serde_json::from_value::<TaskField>(serde_json::json!({ "name": "limit", "label": "Limit", "type": "number", "default": 5 })).expect("a field"), field);
    }
}
