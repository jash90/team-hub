//! The `start` field of the query language: `start >= 2026-01-01`, `start < -7d`,
//! `start is empty`. It reads a date the way the core reads one for `due`
//! (`hub_app::services::search`): a day, an instant, an offset from now (`-7d`, `2w`) or one
//! of `now()`, `startOfDay()`, `startOfWeek()`, `startOfMonth()`; and since a start is a day,
//! an instant counts as its day. Only a start that was set matches, never an estimated one.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use hub_app::{ApiError, ApiResult, FieldSql};
use hub_core::query::{Clause, ClauseOp, QueryValue};
use rusqlite::types::Value;

const START: &str = "(SELECT span.start_date FROM timeline_span span WHERE span.issue_id = i.id)";

pub fn is_start(field: &str) -> bool {
    matches!(field.to_lowercase().as_str(), "start" | "startdate")
}

fn unsupported(clause: &Clause) -> ApiError {
    ApiError::bad_request(format!("That operator cannot be used with \"{}\"", clause.field))
}

/// The day a clause's value stands for.
fn day(clause: &Clause, now: DateTime<Utc>) -> ApiResult<NaiveDate> {
    let invalid = || ApiError::bad_request(format!("\"{}\" needs a date such as 2026-11-30, -7d or startOfWeek()", clause.field));
    let today = now.date_naive();

    match clause.values.first().ok_or_else(invalid)? {
        QueryValue::Func(name) => match name.as_str() {
            "now" | "startofday" => Ok(today),
            "startofweek" => Ok(today - Duration::days(today.weekday().num_days_from_monday() as i64)),
            "startofmonth" => Ok(today.with_day(1).unwrap_or(today)),
            _ => Err(ApiError::bad_request(format!("Unknown function {name}()"))),
        },
        QueryValue::Number(_) => Err(invalid()),
        QueryValue::Text(text) => {
            if let Ok(day) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
                return Ok(day);
            }

            if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
                return Ok(instant.with_timezone(&Utc).date_naive());
            }

            let (sign, rest) = match text.strip_prefix('-') {
                Some(rest) => (-1, rest),
                None => (1, text.strip_prefix('+').unwrap_or(text)),
            };
            let unit = rest.chars().last().ok_or_else(invalid)?;
            let amount: i64 = rest[..rest.len() - unit.len_utf8()].parse().map_err(|_| invalid())?;
            let delta = match unit {
                'm' => Duration::minutes(amount),
                'h' => Duration::hours(amount),
                'd' => Duration::days(amount),
                'w' => Duration::weeks(amount),
                _ => return Err(invalid()),
            };

            Ok((now + delta * sign).date_naive())
        }
    }
}

/// SQL for a clause on `start`, as a condition on the issue aliased `i`.
pub fn start_condition(clause: &Clause, now: DateTime<Utc>) -> ApiResult<FieldSql> {
    let sql = match clause.op {
        ClauseOp::IsEmpty => return Ok(FieldSql { sql: format!("{START} IS NULL"), params: Vec::new() }),
        ClauseOp::IsNotEmpty => return Ok(FieldSql { sql: format!("{START} IS NOT NULL"), params: Vec::new() }),
        ClauseOp::Eq => format!("{START} = ?"),
        // An issue without a start does not start that day either, as with the core's `due`.
        ClauseOp::NotEq => format!("coalesce({START} != ?, 1)"),
        ClauseOp::Less => format!("{START} < ?"),
        ClauseOp::LessEq => format!("{START} <= ?"),
        ClauseOp::Greater => format!("{START} > ?"),
        ClauseOp::GreaterEq => format!("{START} >= ?"),
        _ => return Err(unsupported(clause)),
    };

    Ok(FieldSql { sql, params: vec![Value::Text(day(clause, now)?.to_string())] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clause(op: ClauseOp, value: QueryValue) -> Clause {
        Clause { field: "start".to_string(), op, values: vec![value], position: 0 }
    }

    fn day_of(value: QueryValue) -> String {
        // A Wednesday.
        let now = DateTime::parse_from_rfc3339("2026-11-18T15:30:00Z").expect("an instant").with_timezone(&Utc);

        day(&clause(ClauseOp::Eq, value), now).map(|day| day.to_string()).unwrap_or_else(|error| error.message)
    }

    fn text(value: &str) -> QueryValue {
        QueryValue::Text(value.to_string())
    }

    fn function(name: &str) -> QueryValue {
        QueryValue::Func(name.to_string())
    }

    #[test]
    fn a_date_is_a_day_an_offset_from_now_or_a_function() {
        assert_eq!(day_of(text("2026-01-05")), "2026-01-05");
        assert_eq!(day_of(text("2026-01-05T23:30:00+02:00")), "2026-01-05");
        assert_eq!(day_of(text("-7d")), "2026-11-11");
        assert_eq!(day_of(text("2w")), "2026-12-02");
        assert_eq!(day_of(text("-16h")), "2026-11-17", "an instant counts as its day");
        assert_eq!(day_of(function("now")), "2026-11-18");
        assert_eq!(day_of(function("startofweek")), "2026-11-16");
        assert_eq!(day_of(function("startofmonth")), "2026-11-01");
        assert_eq!(day_of(function("opensprints")), "Unknown function opensprints()");
        assert_eq!(day_of(text("soon")), "\"start\" needs a date such as 2026-11-30, -7d or startOfWeek()");
        assert_eq!(day_of(QueryValue::Number(7.0)), "\"start\" needs a date such as 2026-11-30, -7d or startOfWeek()");
    }

    #[test]
    fn lists_and_text_matching_do_not_apply_to_a_date() {
        let now = Utc::now();

        for op in [ClauseOp::In, ClauseOp::NotIn, ClauseOp::Like, ClauseOp::NotLike] {
            let refused = start_condition(&clause(op, text("2026-01-05")), now).expect_err("not an operator for dates");
            assert_eq!(refused.message, "That operator cannot be used with \"start\"");
        }

        assert!(is_start("startDate") && is_start("START") && !is_start("started"));
    }
}
