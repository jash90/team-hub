//! The reports themselves: pure functions from a project's history to what a chart shows.
//! Days, issues and sprints are independent of each other, so each report fans out over
//! them on the Rayon pool.

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use hub_core::{Id, Sprint, SprintState, StatusCategory, TypeLevel};
use rayon::prelude::*;

use crate::history::{day_end, day_of, days_between, days_from, iso_day, last_days, IssueHistory, ProjectHistory};
use crate::types::{
    ReportBurndown, ReportBurndownDay, ReportCreatedResolved, ReportCreatedResolvedDay, ReportCumulativeFlow, ReportCycleIssue, ReportCycleTime,
    ReportFlowDay, ReportPercentiles, ReportUnit, ReportVelocity, ReportVelocitySprint, ReportWorkload, ReportWorkloadRow,
};

/// Later than any instant in the database.
const END_OF_TIME: &str = "9999";
const UNASSIGNED: &str = "Unassigned";

fn round2(value: f64) -> f64 {
    // An empty sum is negative zero; adding zero makes it the plain one, which is what JSON should carry.
    (value * 100.0).round() / 100.0 + 0.0
}

/// One issue's part of a sprint at one instant: nothing while it is not in the sprint.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Share {
    amount: f64,
    done: bool,
}

fn scope(shares: &[Share]) -> f64 {
    shares.iter().map(|share| share.amount).sum()
}

fn completed(shares: &[Share]) -> f64 {
    shares.iter().filter(|share| share.done).map(|share| share.amount).sum()
}

/// Points live on stories, tasks and bugs: an epic's or a subtask's would count the same
/// work twice. Counting issues leaves out only epics.
fn counts(unit: ReportUnit, issue: &IssueHistory) -> bool {
    match unit {
        ReportUnit::Points => issue.level == TypeLevel::Standard,
        ReportUnit::Issues => issue.level != TypeLevel::Epic,
    }
}

fn members(history: &ProjectHistory, sprint_id: Id, unit: ReportUnit) -> Vec<&IssueHistory> {
    history.issues.iter().filter(|issue| counts(unit, issue) && issue.was_ever_in(sprint_id)).collect()
}

fn share(history: &ProjectHistory, issue: &IssueHistory, sprint_id: Id, unit: ReportUnit, instant: &str) -> Share {
    if !issue.exists_before(instant) || issue.sprint_before(instant) != Some(sprint_id) {
        return Share::default();
    }

    let amount = match unit {
        ReportUnit::Points => issue.points_before(instant),
        ReportUnit::Issues => 1.0,
    };

    Share { amount, done: history.is_done(issue.status_before(instant)) }
}

fn snapshot(history: &ProjectHistory, members: &[&IssueHistory], sprint_id: Id, unit: ReportUnit, instant: &str) -> Vec<Share> {
    members.iter().map(|issue| share(history, issue, sprint_id, unit, instant)).collect()
}

/// Scope that appeared and scope that went away between two snapshots of the same issues.
fn scope_changes(before: &[Share], after: &[Share]) -> (f64, f64) {
    before.iter().zip(after).fold((0.0, 0.0), |(added, removed), (before, after)| {
        let change = after.amount - before.amount;

        (added + change.max(0.0), removed + (-change).max(0.0))
    })
}

/// The days a sprint's chart covers, each with the instant at which the day is measured:
/// its end, or on the day the sprint was completed the moment of completion. Completing a
/// sprint moves unfinished work out of it at that very instant, and measuring just before
/// keeps that work on the chart.
fn sprint_days(sprint: &Sprint, today: NaiveDate) -> Vec<(NaiveDate, String)> {
    let Some(first) = sprint.start_date.as_deref().and_then(day_of) else {
        return Vec::new();
    };
    let completed_at = sprint.completed_at.as_deref().filter(|_| sprint.state == SprintState::Closed);
    let last = match sprint.state {
        SprintState::Future => return Vec::new(),
        SprintState::Active => today,
        SprintState::Closed => completed_at.and_then(day_of).or_else(|| sprint.end_date.as_deref().and_then(day_of)).unwrap_or(today).max(first),
    };

    days_between(first, last)
        .into_iter()
        .map(|day| {
            let end = day_end(day);
            let instant = completed_at.filter(|completed_at| *completed_at < end.as_str()).map_or(end.clone(), String::from);

            (day, instant)
        })
        .collect()
}

fn ideal(committed: f64, span: i64, elapsed: i64) -> f64 {
    if span <= 0 {
        return 0.0;
    }

    committed * (span - elapsed).max(0) as f64 / span as f64
}

fn burndown_series(history: &ProjectHistory, sprint: &Sprint, unit: ReportUnit, days: &[(NaiveDate, String)]) -> (f64, Vec<ReportBurndownDay>) {
    let members = members(history, sprint.id, unit);
    let snapshots: Vec<Vec<Share>> = days.par_iter().map(|(_, instant)| snapshot(history, &members, sprint.id, unit, instant)).collect();
    let committed = snapshots.first().map_or(0.0, |shares| scope(shares));
    let span = match (days.first(), sprint.end_date.as_deref().and_then(day_of)) {
        (Some((first, _)), Some(end)) => (end - *first).num_days(),
        _ => days.len() as i64 - 1,
    };
    let mut previous: Option<&Vec<Share>> = None;
    let mut entries = Vec::with_capacity(days.len());

    for (elapsed, ((day, _), shares)) in days.iter().zip(&snapshots).enumerate() {
        let (added, removed) = previous.map_or((0.0, 0.0), |before| scope_changes(before, shares));
        let done = completed(shares);

        entries.push(ReportBurndownDay {
            date: iso_day(*day),
            remaining: round2(scope(shares) - done),
            ideal: round2(ideal(committed, span, elapsed as i64)),
            completed: round2(done),
            added: round2(added),
            removed: round2(removed),
        });
        previous = Some(shares);
    }

    (round2(committed), entries)
}

/// A sprint's burndown. Without a `unit` it is drawn in points, or in issues when nothing in
/// the sprint was ever estimated.
pub fn burndown(history: &ProjectHistory, sprint: &Sprint, unit: Option<ReportUnit>, today: NaiveDate) -> ReportBurndown {
    let days = sprint_days(sprint, today);
    let series = |unit| burndown_series(history, sprint, unit, &days);
    let (unit, (committed, days)) = match unit {
        Some(unit) => (unit, series(unit)),
        None => {
            let points = series(ReportUnit::Points);

            if points.1.iter().any(|day| day.remaining + day.completed > 0.0) {
                (ReportUnit::Points, points)
            } else {
                (ReportUnit::Issues, series(ReportUnit::Issues))
            }
        }
    };

    ReportBurndown { sprint: Some(sprint.clone()), unit, committed, days }
}

/// The sprint a burndown shows when none is asked for: the running one, else the one
/// completed last.
pub fn default_sprint(sprints: &[Sprint]) -> Option<&Sprint> {
    sprints
        .iter()
        .find(|sprint| sprint.state == SprintState::Active)
        .or_else(|| sprints.iter().filter(|sprint| sprint.state == SprintState::Closed).max_by(|a, b| a.completed_at.cmp(&b.completed_at)))
}

/// The `count` sprints completed last, oldest first.
pub fn recent_closed(sprints: &[Sprint], count: usize) -> Vec<Sprint> {
    let mut closed: Vec<Sprint> = sprints.iter().filter(|sprint| sprint.state == SprintState::Closed).cloned().collect();
    closed.sort_by(|a, b| a.completed_at.cmp(&b.completed_at).then(a.id.cmp(&b.id)));

    closed.split_off(closed.len().saturating_sub(count))
}

fn sprint_velocity(history: &ProjectHistory, sprint: &Sprint) -> ReportVelocitySprint {
    let completion = sprint.completed_at.clone().unwrap_or_else(|| END_OF_TIME.to_string());
    // What the team took on is what was in the sprint when its first day ended.
    let commitment =
        sprint.start_date.as_deref().and_then(day_of).map(day_end).filter(|instant| *instant < completion).unwrap_or_else(|| completion.clone());
    let shares = |unit, instant: &str| snapshot(history, &members(history, sprint.id, unit), sprint.id, unit, instant);

    ReportVelocitySprint {
        sprint: sprint.clone(),
        committed_points: round2(scope(&shares(ReportUnit::Points, &commitment))),
        completed_points: round2(completed(&shares(ReportUnit::Points, &completion))),
        committed_issues: scope(&shares(ReportUnit::Issues, &commitment)).round() as i64,
        completed_issues: completed(&shares(ReportUnit::Issues, &completion)).round() as i64,
    }
}

/// What each of `sprints` (completed, oldest first) took on and what it finished.
pub fn velocity(history: &ProjectHistory, sprints: &[Sprint]) -> ReportVelocity {
    let sprints: Vec<ReportVelocitySprint> = sprints.par_iter().map(|sprint| sprint_velocity(history, sprint)).collect();
    let average = match sprints.len() {
        0 => 0.0,
        count => sprints.iter().map(|sprint| sprint.completed_points).sum::<f64>() / count as f64,
    };

    ReportVelocity { sprints, average_completed_points: round2(average) }
}

/// How many issues were in each status category at the end of each of the last `days` days.
pub fn cumulative_flow(history: &ProjectHistory, days: i64, today: NaiveDate) -> ReportCumulativeFlow {
    let work = history.work();
    let days = last_days(today, days)
        .par_iter()
        .map(|day| {
            let instant = day_end(*day);
            let mut entry = ReportFlowDay { date: iso_day(*day), todo: 0, in_progress: 0, done: 0 };

            for issue in work.iter().filter(|issue| issue.exists_before(&instant)) {
                match history.category(issue.status_before(&instant)) {
                    Some(StatusCategory::Done) => entry.done += 1,
                    Some(StatusCategory::InProgress) => entry.in_progress += 1,
                    Some(StatusCategory::Todo) | None => entry.todo += 1,
                }
            }

            entry
        })
        .collect();

    ReportCumulativeFlow { days }
}

/// Issues created and issues resolved on each of the last `days` days. An issue counts as
/// resolved once, on the day it entered the done statuses it is still in.
pub fn created_vs_resolved(history: &ProjectHistory, days: i64, today: NaiveDate) -> ReportCreatedResolved {
    let work = history.work();
    let marks: Vec<(Option<NaiveDate>, Option<NaiveDate>)> =
        work.par_iter().map(|issue| (day_of(&issue.created_at), history.resolved_at(issue).and_then(day_of))).collect();
    let counted: Vec<(NaiveDate, i64, i64)> = last_days(today, days)
        .par_iter()
        .map(|day| {
            let on_day = |mark: &Option<NaiveDate>| i64::from(*mark == Some(*day));

            (*day, marks.iter().map(|(created, _)| on_day(created)).sum(), marks.iter().map(|(_, resolved)| on_day(resolved)).sum())
        })
        .collect();
    let (mut created_total, mut resolved_total) = (0, 0);
    let days = counted
        .into_iter()
        .map(|(day, created, resolved)| {
            created_total += created;
            resolved_total += resolved;

            ReportCreatedResolvedDay { date: iso_day(day), created, resolved, created_total, resolved_total }
        })
        .collect();

    ReportCreatedResolved { days, created: created_total, resolved: resolved_total }
}

/// The smallest value with at least `percent` percent of the values at or below it.
fn percentile(sorted: &[f64], percent: usize) -> Option<f64> {
    let rank = (sorted.len() * percent).div_ceil(100);

    sorted.get(rank.checked_sub(1)?).copied()
}

fn percentiles(mut values: Vec<f64>) -> ReportPercentiles {
    values.sort_by(f64::total_cmp);

    ReportPercentiles { p50: percentile(&values, 50), p85: percentile(&values, 85), p95: percentile(&values, 95) }
}

/// How long the issues resolved in the last `days` days took.
pub fn cycle_time(history: &ProjectHistory, days: i64, today: NaiveDate) -> ReportCycleTime {
    let from = last_days(today, days).first().map(|day| iso_day(*day)).unwrap_or_default();
    let until = day_end(today);
    let mut issues: Vec<ReportCycleIssue> = history
        .work()
        .par_iter()
        .filter_map(|issue| {
            let resolved_at = history.resolved_at(issue).filter(|at| *at >= from.as_str() && *at < until.as_str())?;
            let cycle_days = history.started_at(issue).and_then(|started_at| days_from(started_at, resolved_at)).filter(|days| *days >= 0.0);

            Some(ReportCycleIssue {
                key: issue.key.clone(),
                summary: issue.summary.clone(),
                resolved_at: resolved_at.to_string(),
                lead_days: round2(days_from(&issue.created_at, resolved_at)?),
                cycle_days: cycle_days.map(round2),
            })
        })
        .collect();
    issues.sort_by(|a, b| a.resolved_at.cmp(&b.resolved_at).then_with(|| a.key.cmp(&b.key)));

    ReportCycleTime {
        lead: percentiles(issues.iter().map(|issue| issue.lead_days).collect()),
        cycle: percentiles(issues.iter().filter_map(|issue| issue.cycle_days).collect()),
        issues,
    }
}

/// Open work per assignee, as it stands now. `names` holds the people who may be assigned.
pub fn workload(history: &ProjectHistory, names: &HashMap<Id, String>) -> ReportWorkload {
    let mut rows: BTreeMap<Option<Id>, ReportWorkloadRow> = BTreeMap::new();

    for issue in history.work() {
        let category = history.category(Some(issue.status_id));

        if category == Some(StatusCategory::Done) {
            continue;
        }

        let row = rows.entry(issue.assignee_id).or_insert_with(|| ReportWorkloadRow {
            assignee_id: issue.assignee_id,
            name: match issue.assignee_id {
                Some(id) => names.get(&id).cloned().unwrap_or_else(|| format!("User {id}")),
                None => UNASSIGNED.to_string(),
            },
            open_issues: 0,
            open_points: 0.0,
            in_progress: 0,
        });
        row.open_issues += 1;
        row.in_progress += i64::from(category == Some(StatusCategory::InProgress));

        if counts(ReportUnit::Points, issue) {
            row.open_points = round2(row.open_points + issue.points.unwrap_or(0.0));
        }
    }

    let mut assignees: Vec<ReportWorkloadRow> = rows.into_values().collect();
    assignees.sort_by(|a, b| {
        (a.assignee_id.is_none(), -a.open_issues, a.name.to_lowercase()).cmp(&(b.assignee_id.is_none(), -b.open_issues, b.name.to_lowercase()))
    });

    ReportWorkload { assignees }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Step;

    const TODO: Id = 1;
    const DOING: Id = 2;
    const DONE: Id = 3;
    const RETIRED: Id = 4;
    const SPRINT: Id = 7;

    fn at(day: u32, hour: u32) -> String {
        format!("2026-03-{day:02}T{hour:02}:00:00.000Z")
    }

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, day).unwrap()
    }

    fn step<T>(day: u32, hour: u32, old: T, new: T) -> Step<T> {
        Step { at: at(day, hour), old, new }
    }

    /// A story created on the first of the month, as it would be had nothing ever happened to it.
    fn issue(id: Id) -> IssueHistory {
        IssueHistory {
            id,
            key: format!("WEB-{id}"),
            summary: format!("Issue {id}"),
            level: TypeLevel::Standard,
            assignee_id: None,
            created_at: at(1, 9),
            status_id: TODO,
            status_since: at(1, 9),
            sprint_id: None,
            points: None,
            status_steps: Vec::new(),
            sprint_steps: Vec::new(),
            points_steps: Vec::new(),
        }
    }

    /// An issue that went through the given statuses, starting in "to do".
    fn moved(mut issue: IssueHistory, moves: &[(u32, u32, Id)]) -> IssueHistory {
        for (day, hour, status) in moves {
            issue.status_steps.push(step(*day, *hour, Some(issue.status_id), Some(*status)));
            issue.status_id = *status;
            issue.status_since = at(*day, *hour);
        }

        issue
    }

    fn project(issues: Vec<IssueHistory>) -> ProjectHistory {
        let categories = HashMap::from([(TODO, StatusCategory::Todo), (DOING, StatusCategory::InProgress), (DONE, StatusCategory::Done)]);

        ProjectHistory { issues, categories }
    }

    fn sprint(state: SprintState, start: u32, end: u32, completed_at: Option<String>) -> Sprint {
        Sprint {
            id: SPRINT,
            project_id: 1,
            name: "Sprint".to_string(),
            goal: String::new(),
            state,
            start_date: Some(iso_day(day(start))),
            end_date: Some(iso_day(day(end))),
            completed_at,
            position: 1,
        }
    }

    fn running() -> Sprint {
        sprint(SprintState::Active, 2, 6, None)
    }

    fn column(report: &ReportBurndown, value: impl Fn(&ReportBurndownDay) -> f64) -> Vec<f64> {
        report.days.iter().map(value).collect()
    }

    #[test]
    fn a_field_without_changes_has_always_had_its_current_value_and_one_with_changes_starts_at_the_first_old_value() {
        let untouched = IssueHistory { sprint_id: Some(SPRINT), points: Some(5.0), ..issue(1) };
        let changed = IssueHistory {
            sprint_id: None,
            sprint_steps: vec![step(4, 12, Some(SPRINT), None)],
            points: Some(8.0),
            points_steps: vec![step(3, 12, Some(3.0), Some(8.0))],
            ..issue(2)
        };

        assert_eq!(untouched.sprint_before(&at(1, 10)), Some(SPRINT));
        assert_eq!(untouched.points_before(&at(1, 10)), 5.0);
        assert!(!untouched.exists_before(&at(1, 9)));
        assert_eq!(changed.sprint_before(&at(4, 12)), Some(SPRINT));
        assert_eq!(changed.sprint_before(&at(4, 13)), None);
        assert_eq!(changed.points_before(&at(2, 0)), 3.0);
        assert_eq!(changed.points_before(&at(9, 0)), 8.0);
    }

    #[test]
    fn scope_added_mid_sprint_raises_remaining_and_is_reported_as_added_not_as_commitment() {
        let planned = moved(IssueHistory { sprint_id: Some(SPRINT), points: Some(5.0), ..issue(1) }, &[(3, 10, DOING), (4, 15, DONE)]);
        let late = IssueHistory { sprint_id: Some(SPRINT), points: Some(3.0), sprint_steps: vec![step(3, 11, None, Some(SPRINT))], ..issue(2) };
        let report = burndown(&project(vec![planned, late]), &running(), None, day(5));

        assert_eq!(report.unit, ReportUnit::Points);
        assert_eq!(report.committed, 5.0);
        assert_eq!(report.days.iter().map(|day| day.date.as_str()).collect::<Vec<_>>(), ["2026-03-02", "2026-03-03", "2026-03-04", "2026-03-05"]);
        assert_eq!(column(&report, |day| day.remaining), [5.0, 8.0, 3.0, 3.0]);
        assert_eq!(column(&report, |day| day.completed), [0.0, 0.0, 5.0, 5.0]);
        assert_eq!(column(&report, |day| day.added), [0.0, 3.0, 0.0, 0.0]);
        assert_eq!(column(&report, |day| day.removed), [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(column(&report, |day| day.ideal), [5.0, 3.75, 2.5, 1.25]);
    }

    #[test]
    fn a_reopened_issue_goes_back_to_remaining() {
        let reopened = moved(IssueHistory { sprint_id: Some(SPRINT), points: Some(5.0), ..issue(1) }, &[(3, 10, DONE), (4, 10, DOING)]);
        let report = burndown(&project(vec![reopened]), &running(), Some(ReportUnit::Points), day(5));

        assert_eq!(column(&report, |day| day.completed), [0.0, 5.0, 0.0, 0.0]);
        assert_eq!(column(&report, |day| day.remaining), [5.0, 0.0, 5.0, 5.0]);
        assert_eq!(column(&report, |day| day.added), [0.0; 4]);
    }

    #[test]
    fn a_change_of_points_moves_scope_by_the_difference() {
        let estimated_twice = IssueHistory {
            sprint_id: Some(SPRINT),
            points: Some(5.0),
            points_steps: vec![step(3, 10, Some(3.0), Some(8.0)), step(4, 10, Some(8.0), Some(5.0))],
            ..issue(1)
        };
        let report = burndown(&project(vec![estimated_twice]), &running(), None, day(5));

        assert_eq!(report.committed, 3.0);
        assert_eq!(column(&report, |day| day.remaining), [3.0, 8.0, 5.0, 5.0]);
        assert_eq!(column(&report, |day| day.added), [0.0, 5.0, 0.0, 0.0]);
        assert_eq!(column(&report, |day| day.removed), [0.0, 0.0, 3.0, 0.0]);
    }

    #[test]
    fn an_issue_whose_first_sprint_change_leaves_the_sprint_was_in_it_from_creation_and_is_removed_that_day() {
        let dropped = IssueHistory { sprint_id: None, points: Some(2.0), sprint_steps: vec![step(4, 10, Some(SPRINT), None)], ..issue(1) };
        let kept = IssueHistory { sprint_id: Some(SPRINT), points: Some(1.0), ..issue(2) };
        let report = burndown(&project(vec![dropped, kept]), &running(), None, day(5));

        assert_eq!(report.committed, 3.0);
        assert_eq!(column(&report, |day| day.remaining), [3.0, 3.0, 1.0, 1.0]);
        assert_eq!(column(&report, |day| day.removed), [0.0, 0.0, 2.0, 0.0]);
    }

    #[test]
    fn a_completed_sprint_stops_at_completion_and_still_shows_the_work_it_handed_on() {
        let completed_at = at(5, 10);
        let finished = moved(IssueHistory { sprint_id: Some(SPRINT), points: Some(5.0), ..issue(1) }, &[(4, 10, DONE)]);
        let carried_over = IssueHistory {
            sprint_id: Some(SPRINT + 1),
            points: Some(3.0),
            sprint_steps: vec![Step { at: completed_at.clone(), old: Some(SPRINT), new: Some(SPRINT + 1) }],
            ..issue(2)
        };
        let closed = sprint(SprintState::Closed, 2, 6, Some(completed_at));
        let report = burndown(&project(vec![finished, carried_over]), &closed, None, day(20));

        assert_eq!(report.days.last().map(|day| day.date.as_str()), Some("2026-03-05"));
        assert_eq!(column(&report, |day| day.remaining), [8.0, 8.0, 3.0, 3.0]);
        assert_eq!(column(&report, |day| day.removed), [0.0; 4]);
    }

    #[test]
    fn points_come_from_stories_only_while_counting_issues_includes_subtasks_but_never_epics() {
        let story = IssueHistory { sprint_id: Some(SPRINT), points: Some(5.0), ..issue(1) };
        let subtask = moved(IssueHistory { level: TypeLevel::Subtask, sprint_id: Some(SPRINT), points: Some(2.0), ..issue(2) }, &[(3, 10, DONE)]);
        let epic = IssueHistory { level: TypeLevel::Epic, sprint_id: Some(SPRINT), points: Some(40.0), ..issue(3) };
        let history = project(vec![story, subtask, epic]);
        let points = burndown(&history, &running(), Some(ReportUnit::Points), day(3));
        let issues = burndown(&history, &running(), Some(ReportUnit::Issues), day(3));

        assert_eq!(points.committed, 5.0);
        assert_eq!(column(&points, |day| day.completed), [0.0, 0.0]);
        assert_eq!(issues.committed, 2.0);
        assert_eq!(column(&issues, |day| day.remaining), [2.0, 1.0]);
    }

    #[test]
    fn nothing_completed_is_a_plain_zero_and_never_a_negative_one() {
        let history = project(vec![IssueHistory { sprint_id: Some(SPRINT), points: Some(5.0), ..issue(1) }]);
        let report = burndown(&history, &running(), None, day(3));
        let empty = velocity(&project(Vec::new()), std::slice::from_ref(&sprint(SprintState::Closed, 2, 6, Some(at(6, 16)))));

        assert!(report.days.iter().all(|day| day.completed == 0.0 && day.completed.is_sign_positive() && day.removed.is_sign_positive()));
        assert!(empty.sprints[0].committed_points.is_sign_positive() && empty.sprints[0].completed_points.is_sign_positive());
        assert!(empty.average_completed_points.is_sign_positive());
    }

    #[test]
    fn a_sprint_without_any_points_is_drawn_in_issues_unless_points_are_asked_for() {
        let history = project(vec![IssueHistory { sprint_id: Some(SPRINT), ..issue(1) }, IssueHistory { sprint_id: Some(SPRINT), ..issue(2) }]);

        let fallback = burndown(&history, &running(), None, day(3));
        let asked = burndown(&history, &running(), Some(ReportUnit::Points), day(3));

        assert_eq!((fallback.unit, fallback.committed), (ReportUnit::Issues, 2.0));
        assert_eq!((asked.unit, asked.committed), (ReportUnit::Points, 0.0));
    }

    #[test]
    fn a_sprint_that_has_not_started_has_no_days_and_a_running_one_past_its_end_date_keeps_going_with_an_ideal_of_zero() {
        let history = project(vec![IssueHistory { sprint_id: Some(SPRINT), points: Some(4.0), ..issue(1) }]);
        let planned = sprint(SprintState::Future, 2, 6, None);
        let overdue = burndown(&history, &running(), None, day(8));

        assert!(burndown(&history, &planned, None, day(5)).days.is_empty());
        assert_eq!(overdue.days.len(), 7);
        assert_eq!(column(&overdue, |day| day.ideal), [4.0, 3.0, 2.0, 1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn velocity_commits_what_was_in_the_sprint_on_its_first_day_and_completes_what_was_done_at_completion() {
        let completed_at = at(6, 16);
        let planned_and_done = moved(IssueHistory { sprint_id: Some(SPRINT), points: Some(5.0), ..issue(1) }, &[(3, 10, DONE)]);
        let added_and_done = moved(
            IssueHistory { sprint_id: Some(SPRINT), points: Some(2.0), sprint_steps: vec![step(4, 9, None, Some(SPRINT))], ..issue(2) },
            &[(5, 10, DONE)],
        );
        let planned_and_carried_over = IssueHistory {
            sprint_id: None,
            points: Some(8.0),
            sprint_steps: vec![Step { at: completed_at.clone(), old: Some(SPRINT), new: None }],
            ..issue(3)
        };
        let done_after_completion = moved(IssueHistory { sprint_id: Some(SPRINT), points: Some(1.0), ..issue(4) }, &[(7, 10, DONE)]);
        let done_subtask = moved(IssueHistory { level: TypeLevel::Subtask, sprint_id: Some(SPRINT), ..issue(5) }, &[(3, 10, DONE)]);
        let history = project(vec![planned_and_done, added_and_done, planned_and_carried_over, done_after_completion, done_subtask]);
        let closed = sprint(SprintState::Closed, 2, 6, Some(completed_at));
        let report = velocity(&history, std::slice::from_ref(&closed));

        assert_eq!(report.sprints.len(), 1);
        assert_eq!(report.sprints[0].committed_points, 14.0);
        assert_eq!(report.sprints[0].completed_points, 7.0);
        assert_eq!(report.sprints[0].committed_issues, 4);
        assert_eq!(report.sprints[0].completed_issues, 3);
        assert_eq!(report.average_completed_points, 7.0);
        assert_eq!(velocity(&history, &[]).average_completed_points, 0.0);
    }

    #[test]
    fn the_default_sprint_is_the_running_one_else_the_one_completed_last() {
        let closed = |id: Id, day: u32| Sprint { id, ..sprint(SprintState::Closed, 2, 6, Some(at(day, 10))) };
        let done = vec![closed(1, 20), closed(2, 10), Sprint { id: 3, ..sprint(SprintState::Future, 2, 6, None) }];
        let mut with_active = done.clone();
        with_active.push(Sprint { id: 4, ..running() });

        assert_eq!(default_sprint(&done).map(|sprint| sprint.id), Some(1));
        assert_eq!(default_sprint(&with_active).map(|sprint| sprint.id), Some(4));
        assert_eq!(default_sprint(&done[2..]).map(|sprint| sprint.id), None);
        assert_eq!(recent_closed(&with_active, 6).iter().map(|sprint| sprint.id).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(recent_closed(&with_active, 1).iter().map(|sprint| sprint.id).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn cumulative_flow_follows_each_issue_through_the_categories_from_the_day_it_was_created() {
        let finished = moved(issue(1), &[(2, 10, DOING), (4, 10, DONE)]);
        let late = IssueHistory { created_at: at(3, 9), ..issue(2) };
        let epic = IssueHistory { level: TypeLevel::Epic, ..issue(3) };
        let report = cumulative_flow(&project(vec![finished, late, epic]), 4, day(4));
        let counts: Vec<(i64, i64, i64)> = report.days.iter().map(|day| (day.todo, day.in_progress, day.done)).collect();

        assert_eq!(report.days[0].date, "2026-03-01");
        assert_eq!(counts, [(1, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 1)]);
    }

    #[test]
    fn time_spent_in_a_status_that_was_deleted_since_is_not_done() {
        let through_retired = moved(issue(1), &[(2, 10, RETIRED), (4, 10, DONE)]);
        let report = cumulative_flow(&project(vec![through_retired]), 4, day(4));

        assert_eq!(report.days.iter().map(|day| (day.todo, day.done)).collect::<Vec<_>>(), [(1, 0), (1, 0), (1, 0), (0, 1)]);
    }

    #[test]
    fn a_move_without_a_history_row_is_dated_by_when_the_issue_entered_its_current_status() {
        let mut swept = moved(issue(1), &[(2, 10, RETIRED)]);
        swept.status_id = DONE;
        swept.status_since = at(3, 12);
        swept.reconcile_status();

        let mut recorded = moved(issue(2), &[(2, 10, DONE)]);
        recorded.reconcile_status();

        assert_eq!(swept.status_steps.last(), Some(&step(3, 12, Some(RETIRED), Some(DONE))));
        assert_eq!(recorded.status_steps.len(), 1);
        assert_eq!(project(vec![swept.clone()]).resolved_at(&swept), Some(at(3, 12).as_str()));
    }

    #[test]
    fn an_issue_resolved_twice_counts_once_on_the_day_of_its_last_resolution() {
        let twice = moved(issue(1), &[(2, 10, DONE), (3, 10, DOING), (4, 10, DONE)]);
        let open = IssueHistory { created_at: at(3, 9), ..issue(2) };
        let report = created_vs_resolved(&project(vec![twice, open]), 4, day(4));
        let rows: Vec<(i64, i64, i64, i64)> =
            report.days.iter().map(|day| (day.created, day.resolved, day.created_total, day.resolved_total)).collect();

        assert_eq!(rows, [(1, 0, 1, 0), (0, 0, 1, 0), (1, 0, 2, 0), (0, 1, 2, 1)]);
        assert_eq!((report.created, report.resolved), (2, 1));
    }

    #[test]
    fn lead_time_runs_from_creation_and_cycle_time_from_the_first_move_into_progress() {
        let worked = moved(issue(1), &[(3, 9, DOING), (4, 9, TODO), (5, 9, DOING), (7, 21, DONE)]);
        let closed_unstarted = moved(issue(2), &[(2, 9, DONE)]);
        let between_done_statuses = {
            let mut history = project(vec![moved(issue(3), &[(2, 9, DOING), (3, 9, DONE), (6, 9, RETIRED)])]);
            history.categories.insert(RETIRED, StatusCategory::Done);

            cycle_time(&history, 30, day(10))
        };
        let report = cycle_time(&project(vec![worked, closed_unstarted, issue(4)]), 30, day(10));

        assert_eq!(report.issues.iter().map(|issue| issue.key.as_str()).collect::<Vec<_>>(), ["WEB-2", "WEB-1"]);
        assert_eq!((report.issues[0].lead_days, report.issues[0].cycle_days), (1.0, None));
        assert_eq!((report.issues[1].lead_days, report.issues[1].cycle_days), (6.5, Some(4.5)));
        assert_eq!(report.lead, ReportPercentiles { p50: Some(1.0), p85: Some(6.5), p95: Some(6.5) });
        assert_eq!(report.cycle, ReportPercentiles { p50: Some(4.5), p85: Some(4.5), p95: Some(4.5) });
        assert_eq!((between_done_statuses.issues[0].resolved_at.as_str(), between_done_statuses.issues[0].lead_days), (at(3, 9).as_str(), 2.0));
    }

    #[test]
    fn cycle_time_only_lists_issues_resolved_inside_the_window() {
        let old = moved(issue(1), &[(2, 9, DONE)]);
        let recent = moved(issue(2), &[(9, 9, DONE)]);
        let report = cycle_time(&project(vec![old, recent]), 3, day(10));

        assert_eq!(report.issues.iter().map(|issue| issue.key.as_str()).collect::<Vec<_>>(), ["WEB-2"]);
        assert_eq!(cycle_time(&project(Vec::new()), 3, day(10)).lead, ReportPercentiles::default());
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let values: Vec<f64> = (1..=20).map(f64::from).collect();

        assert_eq!(percentiles(values), ReportPercentiles { p50: Some(10.0), p85: Some(17.0), p95: Some(19.0) });
        assert_eq!(percentiles(vec![4.0]), ReportPercentiles { p50: Some(4.0), p85: Some(4.0), p95: Some(4.0) });
    }

    #[test]
    fn workload_counts_open_work_per_person_with_the_unassigned_row_last() {
        let anna = |id: Id| IssueHistory { assignee_id: Some(10), ..issue(id) };
        let history = project(vec![
            moved(IssueHistory { points: Some(5.0), ..anna(1) }, &[(2, 9, DOING)]),
            IssueHistory { level: TypeLevel::Subtask, points: Some(1.0), ..anna(2) },
            moved(IssueHistory { points: Some(8.0), ..anna(3) }, &[(2, 9, DONE)]),
            IssueHistory { points: Some(3.0), ..issue(4) },
            IssueHistory { points: Some(2.0), ..issue(5) },
            IssueHistory { points: Some(1.0), ..issue(6) },
            IssueHistory { level: TypeLevel::Epic, assignee_id: Some(11), ..issue(7) },
            IssueHistory { assignee_id: Some(12), points: Some(13.0), ..issue(8) },
        ]);
        let names = HashMap::from([(10, "Anna".to_string()), (12, "Piotr".to_string())]);
        let rows: Vec<(Option<Id>, String, i64, f64, i64)> = workload(&history, &names)
            .assignees
            .into_iter()
            .map(|row| (row.assignee_id, row.name, row.open_issues, row.open_points, row.in_progress))
            .collect();

        assert_eq!(
            rows,
            [(Some(10), "Anna".to_string(), 2, 5.0, 1), (Some(12), "Piotr".to_string(), 1, 13.0, 0), (None, "Unassigned".to_string(), 3, 6.0, 0),]
        );
    }
}
