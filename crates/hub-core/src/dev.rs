//! What the code says about an issue, whichever host the code lives on, and where that
//! disagrees with what the issue's status says ("drift").

use chrono::{DateTime, Duration, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::{string_enum, StatusCategory};

string_enum!(PrState { Open => "open", Merged => "merged", Closed => "closed" });
string_enum!(ReviewState { Approved => "approved", ChangesRequested => "changes_requested", Pending => "pending" });
string_enum!(CheckState { Passing => "passing", Failing => "failing", Running => "running" });

string_enum!(
    /// A way an issue's status and its code disagree.
    DriftKind {
        DoneWithOpenPr => "done_with_open_pr",
        MergedNotDone => "merged_not_done",
        NoCodeYet => "no_code_yet",
        TodoWithCode => "todo_with_code",
        ChecksFailing => "checks_failing",
        ChangesRequested => "changes_requested",
        StalePr => "stale_pr",
        ClosedUnmerged => "closed_unmerged",
    }
);

/// A pull request reduced to what board cards and drift detection need.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrBadge {
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    pub draft: bool,
    pub review: Option<ReviewState>,
    pub checks: Option<CheckState>,
    pub updated_at: String,
}

/// Everything linked to one issue, summarised.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevSummary {
    pub branches: i64,
    pub commits: i64,
    pub prs: Vec<PrBadge>,
    pub drift: Vec<DriftKind>,
}

#[derive(Debug, Clone, Copy)]
pub struct DriftConfig {
    /// Days an issue may sit in progress with no branch, commit or pull request.
    pub idle_days: i64,
    /// Days an open pull request may go without an update.
    pub stale_days: i64,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self { idle_days: 3, stale_days: 7 }
    }
}

pub struct DriftInput<'a> {
    pub category: StatusCategory,
    /// When the issue entered its current status.
    pub in_status_since: &'a str,
    pub dev: &'a DevSummary,
}

fn older_than(timestamp: &str, days: i64, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(timestamp).is_ok_and(|at| now - at.with_timezone(&Utc) > Duration::days(days))
}

/// The ways one issue's status disagrees with its code. An empty list means they agree.
pub fn classify_drift(input: &DriftInput, config: DriftConfig, now: DateTime<Utc>) -> Vec<DriftKind> {
    let prs = &input.dev.prs;
    let open: Vec<&PrBadge> = prs.iter().filter(|pr| pr.state == PrState::Open).collect();
    let merged = prs.iter().filter(|pr| pr.state == PrState::Merged).count();
    let has_code = input.dev.branches > 0 || input.dev.commits > 0 || !prs.is_empty();
    let mut kinds = Vec::new();

    match input.category {
        StatusCategory::Done => {
            if !open.is_empty() {
                kinds.push(DriftKind::DoneWithOpenPr);
            }
        }
        StatusCategory::Todo => {
            if !open.is_empty() || merged > 0 {
                kinds.push(DriftKind::TodoWithCode);
            }
        }
        StatusCategory::InProgress => {
            if merged > 0 && open.is_empty() {
                kinds.push(DriftKind::MergedNotDone);
            }

            if !has_code && older_than(input.in_status_since, config.idle_days, now) {
                kinds.push(DriftKind::NoCodeYet);
            }

            if !prs.is_empty() && open.is_empty() && merged == 0 {
                kinds.push(DriftKind::ClosedUnmerged);
            }
        }
    }

    if input.category != StatusCategory::Done {
        if open.iter().any(|pr| pr.checks == Some(CheckState::Failing)) {
            kinds.push(DriftKind::ChecksFailing);
        }

        if open.iter().any(|pr| pr.review == Some(ReviewState::ChangesRequested)) {
            kinds.push(DriftKind::ChangesRequested);
        }

        if open.iter().any(|pr| !pr.draft && older_than(&pr.updated_at, config.stale_days, now)) {
            kinds.push(DriftKind::StalePr);
        }
    }

    kinds
}

/// Drift for a whole board or project at once.
pub fn classify_drift_many(inputs: &[DriftInput], config: DriftConfig, now: DateTime<Utc>) -> Vec<Vec<DriftKind>> {
    inputs.par_iter().map(|input| classify_drift(input, config, now)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(state: PrState) -> PrBadge {
        PrBadge {
            repo: "acme/web".into(),
            number: 1,
            title: "WEB-1 login".into(),
            url: String::new(),
            state,
            draft: false,
            review: None,
            checks: None,
            updated_at: "2026-11-10T10:00:00Z".into(),
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-11-12T10:00:00Z".parse().unwrap()
    }

    fn drift(category: StatusCategory, since: &str, dev: DevSummary) -> Vec<DriftKind> {
        classify_drift(&DriftInput { category, in_status_since: since, dev: &dev }, DriftConfig::default(), now())
    }

    #[test]
    fn agreement_is_silent() {
        let dev = DevSummary { branches: 1, prs: vec![pr(PrState::Open)], ..Default::default() };

        assert!(drift(StatusCategory::InProgress, "2026-11-01T00:00:00Z", dev).is_empty());
        assert!(drift(StatusCategory::Done, "2026-11-01T00:00:00Z", DevSummary { prs: vec![pr(PrState::Merged)], ..Default::default() }).is_empty());
        assert!(drift(StatusCategory::Todo, "2026-01-01T00:00:00Z", DevSummary::default()).is_empty());
    }

    #[test]
    fn status_ahead_of_or_behind_the_code() {
        assert_eq!(drift(StatusCategory::Done, "", DevSummary { prs: vec![pr(PrState::Open)], ..Default::default() }), vec![DriftKind::DoneWithOpenPr]);
        assert_eq!(
            drift(StatusCategory::InProgress, "", DevSummary { prs: vec![pr(PrState::Merged)], ..Default::default() }),
            vec![DriftKind::MergedNotDone]
        );
        assert_eq!(drift(StatusCategory::Todo, "", DevSummary { prs: vec![pr(PrState::Open)], ..Default::default() }), vec![DriftKind::TodoWithCode]);
        assert_eq!(
            drift(StatusCategory::InProgress, "", DevSummary { prs: vec![pr(PrState::Closed)], ..Default::default() }),
            vec![DriftKind::ClosedUnmerged]
        );
    }

    #[test]
    fn in_progress_without_code_only_after_a_few_days() {
        assert_eq!(drift(StatusCategory::InProgress, "2026-11-05T10:00:00Z", DevSummary::default()), vec![DriftKind::NoCodeYet]);
        assert!(drift(StatusCategory::InProgress, "2026-11-11T10:00:00Z", DevSummary::default()).is_empty());
    }

    #[test]
    fn open_pull_request_trouble() {
        let mut failing = pr(PrState::Open);
        failing.checks = Some(CheckState::Failing);
        failing.review = Some(ReviewState::ChangesRequested);
        failing.updated_at = "2026-10-01T00:00:00Z".into();

        assert_eq!(
            drift(StatusCategory::InProgress, "", DevSummary { prs: vec![failing], ..Default::default() }),
            vec![DriftKind::ChecksFailing, DriftKind::ChangesRequested, DriftKind::StalePr]
        );
    }

    #[test]
    fn many_inputs_keep_their_order() {
        let merged = DevSummary { prs: vec![pr(PrState::Merged)], ..Default::default() };
        let empty = DevSummary::default();
        let inputs = [
            DriftInput { category: StatusCategory::InProgress, in_status_since: "", dev: &merged },
            DriftInput { category: StatusCategory::Todo, in_status_since: "", dev: &empty },
        ];

        assert_eq!(classify_drift_many(&inputs, DriftConfig::default(), now()), vec![vec![DriftKind::MergedNotDone], vec![]]);
    }
}
