//! What the module shows, and the queries that read it.

use std::collections::HashMap;

use hub_app::db::query_all;
use hub_app::ApiResult;
use hub_core::dev::{CheckState, DevSummary, DriftKind, PrBadge, PrState, ReviewState};
use hub_core::{Id, IssueRow};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What moves an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GithubTrigger {
    BranchCreated,
    PrOpened,
    ChangesRequested,
    PrApproved,
    PrMerged,
}

impl GithubTrigger {
    pub const ALL: [GithubTrigger; 5] = [Self::BranchCreated, Self::PrOpened, Self::ChangesRequested, Self::PrApproved, Self::PrMerged];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BranchCreated => "branch_created",
            Self::PrOpened => "pr_opened",
            Self::ChangesRequested => "changes_requested",
            Self::PrApproved => "pr_approved",
            Self::PrMerged => "pr_merged",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|trigger| trigger.as_str() == text)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubRule {
    pub trigger: GithubTrigger,
    pub status_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubRepo {
    pub id: Id,
    pub project_id: Id,
    pub full_name: String,
    pub default_branch: String,
    pub last_synced_at: Option<String>,
    pub last_event_at: Option<String>,
    pub sync_error: Option<String>,
    pub open_prs: i64,
    pub branches: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubPr {
    pub id: Id,
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub state: PrState,
    pub draft: bool,
    pub author: Option<String>,
    pub head_ref: String,
    pub base_ref: String,
    pub url: String,
    pub review: Option<ReviewState>,
    pub checks: Option<CheckState>,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: String,
    pub merged_at: Option<String>,
    /// Issues this pull request is linked to.
    pub issue_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubBranch {
    pub id: Id,
    pub repo: String,
    pub name: String,
    pub url: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubCommit {
    pub id: Id,
    pub repo: String,
    pub sha: String,
    pub message: String,
    pub author_name: Option<String>,
    pub author_login: Option<String>,
    pub url: String,
    pub committed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubRepoRef {
    pub id: Id,
    pub full_name: String,
}

/// The "Development" panel of an issue.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubIssuePanel {
    pub branches: Vec<GithubBranch>,
    pub commits: Vec<GithubCommit>,
    pub prs: Vec<GithubPr>,
    pub drift: Vec<DriftKind>,
    pub suggested_branch: String,
    /// Repositories a branch for this issue can be created in.
    pub repos: Vec<GithubRepoRef>,
    pub can_create_branch: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubDriftItem {
    pub issue: IssueRow,
    pub kinds: Vec<DriftKind>,
}

/// A project's "Code" page.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubProjectCode {
    pub repos: Vec<GithubRepo>,
    pub rules: Vec<GithubRule>,
    pub pulls: Vec<GithubPr>,
    pub drift: Vec<GithubDriftItem>,
    pub can_admin: bool,
    pub token_configured: bool,
    pub webhook_configured: bool,
    pub webhook_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubSettings {
    pub api_url: String,
    pub token_hint: Option<String>,
    pub webhook_secret_hint: Option<String>,
    pub webhook_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubDelivery {
    pub id: Id,
    pub event: String,
    pub repo: Option<String>,
    pub received_at: String,
    pub outcome: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubSyncReport {
    pub pulls: i64,
    pub branches: i64,
    pub commits: i64,
    pub linked_issues: i64,
}

pub fn id_list(ids: &[Id]) -> String {
    if ids.is_empty() {
        "NULL".to_string()
    } else {
        ids.iter().map(Id::to_string).collect::<Vec<_>>().join(", ")
    }
}

const REPO_SELECT: &str = "SELECT r.id, r.project_id, r.owner || '/' || r.name, r.default_branch, r.last_synced_at, r.last_event_at, r.sync_error,
        (SELECT count(*) FROM gh_pr p WHERE p.repo_id = r.id AND p.state = 'open'),
        (SELECT count(*) FROM gh_branch b WHERE b.repo_id = r.id AND b.deleted = 0)
    FROM gh_repo r";

fn map_repo(row: &rusqlite::Row) -> rusqlite::Result<GithubRepo> {
    Ok(GithubRepo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        full_name: row.get(2)?,
        default_branch: row.get(3)?,
        last_synced_at: row.get(4)?,
        last_event_at: row.get(5)?,
        sync_error: row.get(6)?,
        open_prs: row.get(7)?,
        branches: row.get(8)?,
    })
}

pub fn project_repos(connection: &Connection, project_id: Id) -> ApiResult<Vec<GithubRepo>> {
    query_all(connection, &format!("{REPO_SELECT} WHERE r.project_id = ?1 ORDER BY r.owner, r.name"), [project_id], map_repo)
}

pub fn all_repos(connection: &Connection) -> ApiResult<Vec<GithubRepo>> {
    query_all(connection, &format!("{REPO_SELECT} ORDER BY r.id"), [], map_repo)
}

pub fn repo_by_id(connection: &Connection, id: Id) -> ApiResult<Option<GithubRepo>> {
    Ok(query_all(connection, &format!("{REPO_SELECT} WHERE r.id = ?1"), [id], map_repo)?.pop())
}

const PR_SELECT: &str = "SELECT p.id, r.owner || '/' || r.name, p.number, p.title, p.state, p.draft, p.author, p.head_ref, p.base_ref, p.url,
        p.review, p.checks, p.additions, p.deletions, p.created_at, p.updated_at, p.merged_at,
        (SELECT group_concat(i.key) FROM gh_ref f JOIN issue i ON i.id = f.issue_id WHERE f.kind = 'pr' AND f.ref_id = p.id)
    FROM gh_pr p JOIN gh_repo r ON r.id = p.repo_id";

fn map_pr(row: &rusqlite::Row) -> rusqlite::Result<GithubPr> {
    let state: String = row.get(4)?;
    let review: Option<String> = row.get(10)?;
    let checks: Option<String> = row.get(11)?;
    let keys: Option<String> = row.get(17)?;

    Ok(GithubPr {
        id: row.get(0)?,
        repo: row.get(1)?,
        number: row.get(2)?,
        title: row.get(3)?,
        state: PrState::parse(&state).unwrap_or(PrState::Open),
        draft: row.get(5)?,
        author: row.get(6)?,
        head_ref: row.get(7)?,
        base_ref: row.get(8)?,
        url: row.get(9)?,
        review: review.as_deref().and_then(ReviewState::parse),
        checks: checks.as_deref().and_then(CheckState::parse),
        additions: row.get(12)?,
        deletions: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        merged_at: row.get(16)?,
        issue_keys: keys.map(|keys| keys.split(',').map(String::from).collect()).unwrap_or_default(),
    })
}

/// Pull requests matching `condition` (over `p` and `r`), most recently updated first.
pub fn pulls_where(connection: &Connection, condition: &str, limit: i64) -> ApiResult<Vec<GithubPr>> {
    query_all(connection, &format!("{PR_SELECT} WHERE {condition} ORDER BY p.updated_at DESC LIMIT {limit}"), [], map_pr)
}

pub fn issue_pulls(connection: &Connection, issue_id: Id) -> ApiResult<Vec<GithubPr>> {
    pulls_where(connection, &format!("p.id IN (SELECT ref_id FROM gh_ref WHERE kind = 'pr' AND issue_id = {issue_id})"), 50)
}

pub fn issue_branches(connection: &Connection, issue_id: Id) -> ApiResult<Vec<GithubBranch>> {
    query_all(
        connection,
        "SELECT b.id, r.owner || '/' || r.name, b.name, b.url, b.updated_at
         FROM gh_ref f JOIN gh_branch b ON b.id = f.ref_id JOIN gh_repo r ON r.id = b.repo_id
         WHERE f.kind = 'branch' AND f.issue_id = ?1 AND b.deleted = 0 ORDER BY b.updated_at DESC",
        [issue_id],
        |row| Ok(GithubBranch { id: row.get(0)?, repo: row.get(1)?, name: row.get(2)?, url: row.get(3)?, updated_at: row.get(4)? }),
    )
}

pub fn issue_commits(connection: &Connection, issue_id: Id) -> ApiResult<Vec<GithubCommit>> {
    query_all(
        connection,
        "SELECT c.id, r.owner || '/' || r.name, c.sha, c.message, c.author_name, c.author_login, c.url, c.committed_at
         FROM gh_ref f JOIN gh_commit c ON c.id = f.ref_id JOIN gh_repo r ON r.id = c.repo_id
         WHERE f.kind = 'commit' AND f.issue_id = ?1 ORDER BY c.committed_at DESC LIMIT 50",
        [issue_id],
        |row| {
            Ok(GithubCommit {
                id: row.get(0)?,
                repo: row.get(1)?,
                sha: row.get(2)?,
                message: row.get(3)?,
                author_name: row.get(4)?,
                author_login: row.get(5)?,
                url: row.get(6)?,
                committed_at: row.get(7)?,
            })
        },
    )
}

/// What the code says about each of `issue_ids`, without drift; issues with nothing linked are absent.
pub fn dev_summaries(connection: &Connection, issue_ids: &[Id]) -> ApiResult<HashMap<Id, DevSummary>> {
    let ids = id_list(issue_ids);
    let mut summaries: HashMap<Id, DevSummary> = HashMap::new();

    let badges = query_all(
        connection,
        &format!(
            "SELECT f.issue_id, r.owner || '/' || r.name, p.number, p.title, p.url, p.state, p.draft, p.review, p.checks, p.updated_at
             FROM gh_ref f JOIN gh_pr p ON p.id = f.ref_id JOIN gh_repo r ON r.id = p.repo_id
             WHERE f.kind = 'pr' AND f.issue_id IN ({ids}) ORDER BY p.state = 'open' DESC, p.updated_at DESC"
        ),
        [],
        |row| {
            let state: String = row.get(5)?;
            let review: Option<String> = row.get(7)?;
            let checks: Option<String> = row.get(8)?;

            Ok((
                row.get::<_, Id>(0)?,
                PrBadge {
                    repo: row.get(1)?,
                    number: row.get(2)?,
                    title: row.get(3)?,
                    url: row.get(4)?,
                    state: PrState::parse(&state).unwrap_or(PrState::Open),
                    draft: row.get(6)?,
                    review: review.as_deref().and_then(ReviewState::parse),
                    checks: checks.as_deref().and_then(CheckState::parse),
                    updated_at: row.get(9)?,
                },
            ))
        },
    )?;

    for (issue_id, badge) in badges {
        summaries.entry(issue_id).or_default().prs.push(badge);
    }

    let counts = query_all(
        connection,
        &format!(
            "SELECT f.issue_id,
                    sum(f.kind = 'branch' AND EXISTS (SELECT 1 FROM gh_branch b WHERE b.id = f.ref_id AND b.deleted = 0)),
                    sum(f.kind = 'commit')
             FROM gh_ref f WHERE f.issue_id IN ({ids}) AND f.kind != 'pr' GROUP BY f.issue_id"
        ),
        [],
        |row| Ok((row.get::<_, Id>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
    )?;

    for (issue_id, branches, commits) in counts {
        if branches > 0 || commits > 0 {
            let summary = summaries.entry(issue_id).or_default();
            summary.branches = branches;
            summary.commits = commits;
        }
    }

    Ok(summaries)
}

pub fn stored_drift(connection: &Connection, issue_ids: &[Id]) -> ApiResult<HashMap<Id, Vec<DriftKind>>> {
    let rows = query_all(
        connection,
        &format!("SELECT issue_id, kind FROM gh_drift WHERE issue_id IN ({}) ORDER BY issue_id, kind", id_list(issue_ids)),
        [],
        |row| Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?)),
    )?;
    let mut drift: HashMap<Id, Vec<DriftKind>> = HashMap::new();

    for (issue_id, kind) in rows {
        if let Some(kind) = DriftKind::parse(&kind) {
            drift.entry(issue_id).or_default().push(kind);
        }
    }

    Ok(drift)
}

pub fn project_rules(connection: &Connection, project_id: Id) -> ApiResult<Vec<GithubRule>> {
    let stored = query_all(connection, "SELECT trigger, status_id FROM gh_rule WHERE project_id = ?1", [project_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Id>(1)?))
    })?;

    Ok(GithubTrigger::ALL
        .into_iter()
        .map(|trigger| GithubRule { trigger, status_id: stored.iter().find(|(name, _)| name == trigger.as_str()).map(|(_, status_id)| *status_id) })
        .collect())
}
