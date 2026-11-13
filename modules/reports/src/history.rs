//! An issue's past as plain data: what its status, sprint and points were at any instant.
//! Nothing here touches the database, so every report is a function of these structs.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate};
use hub_core::{Id, StatusCategory, TypeLevel};

/// One recorded change of a field. Instants are RFC 3339 UTC strings, which order as text.
#[derive(Debug, Clone, PartialEq)]
pub struct Step<T> {
    pub at: String,
    pub old: T,
    pub new: T,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IssueHistory {
    pub id: Id,
    pub key: String,
    pub summary: String,
    pub level: TypeLevel,
    pub assignee_id: Option<Id>,
    pub created_at: String,
    /// The issue as it is now. A field without recorded changes has had its current value
    /// since the issue was created.
    pub status_id: Id,
    pub status_since: String,
    pub sprint_id: Option<Id>,
    pub points: Option<f64>,
    /// Recorded changes, oldest first.
    pub status_steps: Vec<Step<Option<Id>>>,
    pub sprint_steps: Vec<Step<Option<Id>>>,
    pub points_steps: Vec<Step<Option<f64>>>,
}

/// The value of a field once every change before `instant` has been applied.
fn value_before<T: Copy>(steps: &[Step<T>], current: T, instant: &str) -> T {
    let applied = steps.partition_point(|step| step.at.as_str() < instant);

    match applied.checked_sub(1).and_then(|last| steps.get(last)) {
        Some(step) => step.new,
        None => steps.first().map_or(current, |step| step.old),
    }
}

impl IssueHistory {
    pub fn exists_before(&self, instant: &str) -> bool {
        self.created_at.as_str() < instant
    }

    pub fn status_before(&self, instant: &str) -> Option<Id> {
        value_before(&self.status_steps, Some(self.status_id), instant)
    }

    pub fn sprint_before(&self, instant: &str) -> Option<Id> {
        value_before(&self.sprint_steps, self.sprint_id, instant)
    }

    pub fn points_before(&self, instant: &str) -> f64 {
        value_before(&self.points_steps, self.points, instant).unwrap_or(0.0)
    }

    pub fn was_ever_in(&self, sprint_id: Id) -> bool {
        let wanted = Some(sprint_id);

        self.sprint_id == wanted || self.sprint_steps.iter().any(|step| step.old == wanted || step.new == wanted)
    }

    /// Deleting a status moves its issues without a history row, but the issue remembers when
    /// it entered its current status: that is when the unrecorded move happened.
    pub fn reconcile_status(&mut self) {
        let Some(last) = self.status_steps.last() else { return };

        if last.new == Some(self.status_id) {
            return;
        }

        let step = Step { at: self.status_since.clone().max(last.at.clone()), old: last.new, new: Some(self.status_id) };
        self.status_steps.push(step);
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectHistory {
    pub issues: Vec<IssueHistory>,
    /// The project's statuses as they are now.
    pub categories: HashMap<Id, StatusCategory>,
}

impl ProjectHistory {
    /// A status that has been deleted since has no category, so time spent in it is never "done".
    pub fn category(&self, status: Option<Id>) -> Option<StatusCategory> {
        status.and_then(|id| self.categories.get(&id).copied())
    }

    pub fn is_done(&self, status: Option<Id>) -> bool {
        self.category(status) == Some(StatusCategory::Done)
    }

    /// Issues that are work: everything except epics, which only group other issues.
    pub fn work(&self) -> Vec<&IssueHistory> {
        self.issues.iter().filter(|issue| issue.level != TypeLevel::Epic).collect()
    }

    /// When the issue entered the done statuses it is still in; none while it is open.
    /// Reopening forgets the earlier resolution, and moving between two done statuses keeps it.
    pub fn resolved_at<'a>(&self, issue: &'a IssueHistory) -> Option<&'a str> {
        let initial = issue.status_steps.first().map_or(Some(issue.status_id), |step| step.old);
        let mut since = self.is_done(initial).then_some(issue.created_at.as_str());

        for step in &issue.status_steps {
            match (since, self.is_done(step.new)) {
                (None, true) => since = Some(step.at.as_str()),
                (Some(_), false) => since = None,
                _ => {}
            }
        }

        since
    }

    /// When work on the issue first began: its first time in an in-progress status.
    pub fn started_at<'a>(&self, issue: &'a IssueHistory) -> Option<&'a str> {
        let in_progress = |status| self.category(status) == Some(StatusCategory::InProgress);
        let initial = issue.status_steps.first().map_or(Some(issue.status_id), |step| step.old);

        if in_progress(initial) {
            return Some(issue.created_at.as_str());
        }

        issue.status_steps.iter().find(|step| in_progress(step.new)).map(|step| step.at.as_str())
    }
}

/// The UTC day an instant falls on.
pub fn day_of(instant: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(instant.get(..10)?, "%Y-%m-%d").ok()
}

/// The instant a day ends, which is the first instant of the next one.
pub fn day_end(day: NaiveDate) -> String {
    format!("{}T00:00:00.000Z", day.succ_opt().unwrap_or(NaiveDate::MAX).format("%Y-%m-%d"))
}

pub fn iso_day(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

/// Every day from `first` to `last`, both included.
pub fn days_between(first: NaiveDate, last: NaiveDate) -> Vec<NaiveDate> {
    first.iter_days().take_while(|day| *day <= last).collect()
}

/// The `count` days that end with `today`.
pub fn last_days(today: NaiveDate, count: i64) -> Vec<NaiveDate> {
    let first = today.checked_sub_signed(chrono::Duration::days(count.max(1) - 1)).unwrap_or(today);

    days_between(first, today)
}

/// Days from one instant to another, with fractions.
pub fn days_from(start: &str, end: &str) -> Option<f64> {
    let start = DateTime::parse_from_rfc3339(start).ok()?;
    let end = DateTime::parse_from_rfc3339(end).ok()?;

    Some((end - start).num_milliseconds() as f64 / 86_400_000.0)
}
