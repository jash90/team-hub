//! When the two daily slots run. One setting for the instance, kept with the other settings.

use hub_core::schedule::{parse_time, parse_zone, ScheduleSettings};
use rusqlite::Connection;
use serde_json::json;

use crate::app::{get_setting, set_setting, Tx};
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};

const SETTING: &str = "schedule";

/// The settings in force: what an administrator saved, or the defaults.
pub fn settings(connection: &Connection) -> ApiResult<ScheduleSettings> {
    Ok(get_setting(connection, SETTING)?.unwrap_or_default())
}

fn time(text: &str, which: &str) -> ApiResult<String> {
    parse_time(text)
        .map(|time| time.format("%H:%M").to_string())
        .ok_or_else(|| ApiError::bad_request(format!("The {which} time has to be written as hours and minutes, like 10:00")))
}

/// Saves the settings as an administrator, and tells open pages. Times come back as "HH:MM".
pub fn save_settings(tx: &Tx, user: &AuthUser, wanted: &ScheduleSettings) -> ApiResult<ScheduleSettings> {
    user.require_admin()?;

    let timezone = wanted.timezone.trim();

    if parse_zone(timezone).is_none() {
        return Err(ApiError::bad_request(format!("\"{timezone}\" is not a time zone; write its IANA name, like Europe/Warsaw")));
    }

    let saved = ScheduleSettings {
        morning: time(&wanted.morning, "morning")?,
        afternoon: time(&wanted.afternoon, "afternoon")?,
        timezone: timezone.to_string(),
        weekdays_only: wanted.weekdays_only,
    };

    if saved.morning >= saved.afternoon {
        return Err(ApiError::bad_request("The morning run has to come before the afternoon one"));
    }

    set_setting(tx, SETTING, &saved)?;
    tx.emit("schedule.settings", None, None, json!({ "timezone": saved.timezone }))?;

    Ok(saved)
}
