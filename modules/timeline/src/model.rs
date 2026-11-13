//! Reads the rows of a timeline: issues with their children, and when each one runs. An end
//! of a span that nobody set is estimated from the children (`estimate`).

use std::collections::HashMap;

use hub_app::db::query_all;
use hub_app::{ApiError, ApiResult};
use hub_core::{Id, StatusCategory, WorkCounts};
use rusqlite::{Connection, Row};

use crate::types::{TimelineChild, TimelineDerived, TimelineEpic, TimelineSpan, TimelineStatus};

/// Which issues are the rows: the epics of a project, or one issue of any kind.
#[derive(Debug, Clone, Copy)]
pub enum Scope {
    Epics { project_id: Id },
    Issue(Id),
}

impl Scope {
    /// A condition on the row's issue, aliased `e`, with its type as `t`; `?1` is `id()`.
    fn condition(self) -> &'static str {
        match self {
            Scope::Epics { .. } => "e.project_id = ?1 AND t.level = 'epic'",
            Scope::Issue(_) => "e.id = ?1",
        }
    }

    fn id(self) -> Id {
        match self {
            Scope::Epics { project_id } => project_id,
            Scope::Issue(id) => id,
        }
    }
}

/// A row before its children are known.
pub struct Parent {
    id: Id,
    key: String,
    summary: String,
    status: TimelineStatus,
    assignee_id: Option<Id>,
    start: Option<String>,
    due: Option<String>,
}

/// A child with what it says about when its parent's work runs.
pub struct Child {
    parent_id: Id,
    issue: TimelineChild,
    points: Option<f64>,
    created: String,
    resolved: Option<String>,
    /// The day it first entered an in-progress status.
    begun: Option<String>,
    sprint_start: Option<String>,
    sprint_end: Option<String>,
}

fn status(row: &Row, first: usize) -> rusqlite::Result<TimelineStatus> {
    let category: String = row.get(first + 2)?;

    Ok(TimelineStatus { id: row.get(first)?, name: row.get(first + 1)?, category: StatusCategory::parse(&category).unwrap_or(StatusCategory::Todo) })
}

/// The rows of `scope`, in rank order.
pub fn parents(connection: &Connection, scope: Scope) -> ApiResult<Vec<Parent>> {
    query_all(
        connection,
        &format!(
            "SELECT e.id, e.key, e.summary, s.id, s.name, s.category, e.assignee_id, span.start_date, e.due_date
             FROM issue e
             JOIN issue_type t ON t.id = e.type_id
             JOIN status s ON s.id = e.status_id
             LEFT JOIN timeline_span span ON span.issue_id = e.id
             WHERE {} ORDER BY e.rank, e.id",
            scope.condition()
        ),
        [scope.id()],
        |row| {
            Ok(Parent {
                id: row.get(0)?,
                key: row.get(1)?,
                summary: row.get(2)?,
                status: status(row, 3)?,
                assignee_id: row.get(6)?,
                start: row.get(7)?,
                due: row.get(8)?,
            })
        },
    )
}

/// The children of the rows of `scope`, in rank order. A child began on the day of its first
/// recorded move into an in-progress status; one created in such a status and still there
/// has no such move, and began when it got there.
pub fn children(connection: &Connection, scope: Scope) -> ApiResult<Vec<Child>> {
    query_all(
        connection,
        &format!(
            "SELECT c.parent_id, c.id, c.key, c.summary, s.id, s.name, s.category, c.assignee_id, span.start_date, c.due_date, c.points,
                    substr(c.created_at, 1, 10), substr(c.resolved_at, 1, 10),
                    coalesce(
                        (SELECT substr(min(moved.created_at), 1, 10) FROM change moved JOIN status entered ON entered.id = CAST(moved.new_ref AS INTEGER)
                         WHERE moved.issue_id = c.id AND moved.field = 'status' AND entered.category = 'in_progress'),
                        CASE WHEN s.category = 'in_progress' THEN substr(c.status_since, 1, 10) END),
                    substr(sprint.start_date, 1, 10), substr(coalesce(sprint.end_date, sprint.completed_at), 1, 10)
             FROM issue c
             JOIN issue e ON e.id = c.parent_id
             JOIN issue_type t ON t.id = e.type_id
             JOIN status s ON s.id = c.status_id
             LEFT JOIN timeline_span span ON span.issue_id = c.id
             LEFT JOIN sprint ON sprint.id = c.sprint_id
             WHERE {} ORDER BY c.rank, c.id",
            scope.condition()
        ),
        [scope.id()],
        |row| {
            Ok(Child {
                parent_id: row.get(0)?,
                issue: TimelineChild {
                    id: row.get(1)?,
                    key: row.get(2)?,
                    summary: row.get(3)?,
                    status: status(row, 4)?,
                    assignee_id: row.get(7)?,
                    start: row.get(8)?,
                    due: row.get(9)?,
                },
                points: row.get(10)?,
                created: row.get(11)?,
                resolved: row.get(12)?,
                begun: row.get(13)?,
                sprint_start: row.get(14)?,
                sprint_end: row.get(15)?,
            })
        },
    )
}

/// When the children's work runs, as far as they say. It starts with the earliest of their
/// sprints; without a dated sprint, on the first day one of them was in progress; while none
/// has begun, on the day the first of them was created. It ends on the latest of their due
/// dates, the ends of their sprints and the days they were resolved.
fn estimate(children: &[&Child]) -> (Option<String>, Option<String>) {
    let earliest = |date: fn(&Child) -> Option<&str>| children.iter().filter_map(|child| date(child)).min().map(String::from);
    let start = earliest(|child| child.sprint_start.as_deref())
        .or_else(|| earliest(|child| child.begun.as_deref()))
        .or_else(|| earliest(|child| Some(child.created.as_str())));
    let due = children
        .iter()
        .flat_map(|child| [child.issue.due.as_deref(), child.sprint_end.as_deref(), child.resolved.as_deref()])
        .flatten()
        .max()
        .map(String::from);

    (start, due)
}

/// The span of an issue with the dates `start` and `due` and these children. An estimated
/// end gives way to the other one, so the span never runs backwards: an estimated start is
/// no later than the due date, an estimated due date no earlier than the start.
fn span(start: Option<String>, due: Option<String>, children: &[&Child]) -> TimelineSpan {
    let derived = TimelineDerived { start: start.is_none(), due: due.is_none() };

    if !derived.start && !derived.due {
        return TimelineSpan { start, due, derived };
    }

    let (estimated_start, estimated_due) = estimate(children);
    let start = match (start, &due) {
        (Some(start), _) => Some(start),
        (None, Some(due)) => estimated_start.map(|start| start.min(due.clone())),
        (None, None) => estimated_start,
    };
    let due = match (due, &start) {
        (Some(due), _) => Some(due),
        (None, Some(start)) => estimated_due.map(|due| due.max(start.clone())),
        (None, None) => estimated_due,
    };

    TimelineSpan { derived: TimelineDerived { start: derived.start && start.is_some(), due: derived.due && due.is_some() }, start, due }
}

fn tally(children: &[&Child]) -> WorkCounts {
    let count = |category: StatusCategory| children.iter().filter(|child| child.issue.status.category == category).count() as i64;

    WorkCounts {
        total: children.len() as i64,
        todo: count(StatusCategory::Todo),
        in_progress: count(StatusCategory::InProgress),
        done: count(StatusCategory::Done),
    }
}

fn points(children: &[&Child], counted: impl Fn(&Child) -> bool) -> f64 {
    children.iter().filter(|child| counted(child)).filter_map(|child| child.points).sum()
}

/// Puts the children under their rows and works out each row's span and progress.
pub fn assemble(parents: Vec<Parent>, children: Vec<Child>) -> Vec<TimelineEpic> {
    let mut under: HashMap<Id, Vec<&Child>> = HashMap::new();

    for child in &children {
        under.entry(child.parent_id).or_default().push(child);
    }

    parents
        .into_iter()
        .map(|parent| {
            let own = under.remove(&parent.id).unwrap_or_default();

            TimelineEpic {
                id: parent.id,
                key: parent.key,
                summary: parent.summary,
                status: parent.status,
                assignee_id: parent.assignee_id,
                span: span(parent.start, parent.due, &own),
                counts: tally(&own),
                points_done: points(&own, |child| child.issue.status.category == StatusCategory::Done),
                points_total: points(&own, |_| true),
                children: own.into_iter().map(|child| child.issue.clone()).collect(),
            }
        })
        .collect()
}

/// One issue as a row of the timeline, whatever its type; its children are what is under it.
pub fn row(connection: &Connection, issue_id: Id) -> ApiResult<TimelineEpic> {
    let scope = Scope::Issue(issue_id);

    assemble(parents(connection, scope)?, children(connection, scope)?).pop().ok_or_else(|| ApiError::not_found("Issue"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    fn child(created: &str) -> Child {
        let status = TimelineStatus { id: 1, name: "To Do".to_string(), category: StatusCategory::Todo };

        Child {
            parent_id: 1,
            issue: TimelineChild { id: 2, key: "WEB-2".to_string(), summary: String::new(), status, assignee_id: None, start: None, due: None },
            points: None,
            created: created.to_string(),
            resolved: None,
            begun: None,
            sprint_start: None,
            sprint_end: None,
        }
    }

    #[test]
    fn the_start_is_estimated_from_sprints_before_work_before_creation() {
        let waiting = child("2026-03-01");
        let begun = Child { begun: text("2026-03-09"), ..child("2026-03-05") };
        let planned = Child { sprint_start: text("2026-03-16"), sprint_end: text("2026-03-29"), ..child("2026-03-02") };

        assert_eq!(estimate(&[&waiting]), (text("2026-03-01"), None));
        assert_eq!(estimate(&[&waiting, &begun]).0, text("2026-03-09"), "work that has begun says more than the day an issue was written down");
        assert_eq!(estimate(&[&waiting, &begun, &planned]), (text("2026-03-16"), text("2026-03-29")));
        assert_eq!(estimate(&[]), (None, None));
    }

    #[test]
    fn the_end_is_the_latest_of_due_dates_sprint_ends_and_resolutions() {
        let mut due = child("2026-03-01");
        due.issue.due = text("2026-04-10");
        let resolved = Child { resolved: text("2026-04-12"), ..child("2026-03-01") };
        let planned = Child { sprint_start: text("2026-03-16"), sprint_end: text("2026-03-29"), ..child("2026-03-02") };

        assert_eq!(estimate(&[&due, &planned]).1, text("2026-04-10"));
        assert_eq!(estimate(&[&due, &planned, &resolved]).1, text("2026-04-12"));
    }

    #[test]
    fn dates_that_are_set_are_kept_and_only_the_missing_end_is_marked() {
        let planned = Child { sprint_start: text("2026-03-16"), sprint_end: text("2026-03-29"), ..child("2026-03-02") };
        let both = span(text("2026-01-01"), text("2026-02-01"), &[&planned]);
        let start_only = span(text("2026-03-20"), None, &[&planned]);
        let nothing = span(None, None, &[]);

        assert_eq!((both.start, both.due, both.derived), (text("2026-01-01"), text("2026-02-01"), TimelineDerived::default()));
        assert_eq!((start_only.due, start_only.derived), (text("2026-03-29"), TimelineDerived { start: false, due: true }));
        assert_eq!((nothing.start, nothing.due, nothing.derived), (None, None, TimelineDerived::default()));
    }

    #[test]
    fn an_estimated_end_never_makes_the_span_run_backwards() {
        let planned = Child { sprint_start: text("2026-03-16"), sprint_end: text("2026-03-29"), ..child("2026-03-02") };
        let late_start = span(text("2026-05-01"), None, &[&planned]);
        let early_due = span(None, text("2026-03-10"), &[&planned]);

        assert_eq!((late_start.start, late_start.due), (text("2026-05-01"), text("2026-05-01")));
        assert_eq!((early_due.start, early_due.due), (text("2026-03-10"), text("2026-03-10")));
    }
}
