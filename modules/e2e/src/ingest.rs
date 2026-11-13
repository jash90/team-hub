//! Stores a parsed run: the run, its tests and results, the links from tests to the issues
//! they name, and the events that tell open pages about it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use hub_app::access::project_access_by_id;
use hub_app::db::{now_iso, query_all, query_one};
use hub_app::services::issues::issue_id_by_key;
use hub_app::{ApiError, ApiResult, AuthUser, Tx};
use hub_core::Id;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use crate::model::{TEST_KIND, VERIFIES, VERIFIES_MANUAL};
use crate::report::{normalize_time, ParsedRun};
use crate::types::E2eStatus;

/// What the poster says about a run beside the report: query parameters of the request, or
/// a `meta` object in the JSON document.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunMeta {
    /// Makes posting idempotent for a report that carries no run id (JUnit).
    pub run_id: Option<String>,
    #[serde(alias = "runName")]
    pub name: Option<String>,
    pub branch: Option<String>,
    #[serde(alias = "sha")]
    pub commit: Option<String>,
    #[serde(alias = "pr", alias = "pullRequest", deserialize_with = "number_or_text")]
    pub pr_number: Option<i64>,
    #[serde(alias = "url")]
    pub ci_url: Option<String>,
    pub started_at: Option<String>,
}

/// A pull request number arrives as a number in JSON and as text in a query string.
fn number_or_text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    let value: Option<serde_json::Value> = Deserialize::deserialize(deserializer)?;

    Ok(match value {
        Some(serde_json::Value::Number(number)) => number.as_i64(),
        Some(serde_json::Value::String(text)) => text.trim().trim_start_matches('#').parse().ok(),
        _ => None,
    }
    .filter(|number| *number > 0))
}

fn tidy(text: Option<String>, max_chars: usize) -> Option<String> {
    text.map(|text| text.trim().chars().take(max_chars).collect::<String>()).filter(|text| !text.is_empty())
}

impl RunMeta {
    /// `self`, with what it leaves unsaid taken from `other`.
    pub fn or(self, other: RunMeta) -> RunMeta {
        RunMeta {
            run_id: self.run_id.or(other.run_id),
            name: self.name.or(other.name),
            branch: self.branch.or(other.branch),
            commit: self.commit.or(other.commit),
            pr_number: self.pr_number.or(other.pr_number),
            ci_url: self.ci_url.or(other.ci_url),
            started_at: self.started_at.or(other.started_at),
        }
    }

    fn cleaned(self) -> RunMeta {
        RunMeta {
            run_id: tidy(self.run_id, 200),
            name: tidy(self.name, 200),
            branch: tidy(self.branch, 200).map(|branch| branch.trim_start_matches("refs/heads/").to_string()),
            commit: tidy(self.commit, 100),
            pr_number: self.pr_number,
            // Shown as a link, so nothing but a web address is kept.
            ci_url: tidy(self.ci_url, 1000).filter(|url| url.starts_with("https://") || url.starts_with("http://")),
            started_at: self.started_at.as_deref().and_then(normalize_time),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub run_id: Id,
    /// False when the run had been stored before; nothing was changed then.
    pub created: bool,
    /// Issues whose tests have a result in this run, or gained or lost a test through it.
    pub issues: Vec<Id>,
}

pub fn project_keys(connection: &Connection) -> ApiResult<HashSet<String>> {
    Ok(query_all(connection, "SELECT key FROM project", [], |row| row.get::<_, String>(0))?.into_iter().collect())
}

/// The issues a test is linked to, with whether each link was set by hand.
fn linked_issues(connection: &Connection, test_id: Id) -> ApiResult<Vec<(Id, bool)>> {
    query_all(
        connection,
        "SELECT CAST(to_id AS INTEGER), relation = ?3 FROM link WHERE from_kind = ?1 AND from_id = ?2 AND to_kind = 'issue' AND relation IN (?4, ?3)",
        (TEST_KIND, test_id.to_string(), VERIFIES_MANUAL, VERIFIES),
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// A result as `refresh_tests` reads it: run, when, status, duration, attempts, message, error code.
type LastResult = (Id, String, String, i64, i64, Option<String>, Option<String>);

/// The newest run that has a result for the test decides what the test's `last_*` columns say.
/// A test on several targets takes its worst result there.
pub fn refresh_tests(connection: &Connection, test_ids: &[Id]) -> ApiResult<()> {
    for test_id in test_ids {
        let results: Vec<LastResult> = query_all(
            connection,
            "SELECT e.run_id, r.started_at, e.status, e.duration_ms, e.attempts, e.message, e.error_code
             FROM e2e_result e JOIN e2e_run r ON r.id = e.run_id
             WHERE e.test_id = ?1 AND e.run_id = (SELECT e2.run_id FROM e2e_result e2 JOIN e2e_run r2 ON r2.id = e2.run_id WHERE e2.test_id = ?1
                                                  ORDER BY r2.started_at DESC, r2.id DESC LIMIT 1)",
            [test_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )?;
        let worst = results.iter().max_by_key(|result| E2eStatus::parse(&result.2).map(E2eStatus::severity).unwrap_or(0));

        match worst {
            Some((run_id, at, status, duration, attempts, message, code)) => connection.execute(
                "UPDATE e2e_test SET last_run_id = ?1, last_at = ?2, last_status = ?3, last_duration_ms = ?4, last_attempts = ?5, last_message = ?6,
                        last_error_code = ?7 WHERE id = ?8",
                (run_id, at, status, duration, attempts, message, code, test_id),
            )?,
            None => connection.execute(
                "UPDATE e2e_test SET last_run_id = NULL, last_at = NULL, last_status = NULL, last_duration_ms = NULL, last_attempts = NULL,
                        last_message = NULL, last_error_code = NULL WHERE id = ?1",
                [test_id],
            )?,
        };
    }

    Ok(())
}

/// Tells open pages that the tests of these issues changed.
pub fn announce_issues(tx: &Tx, issue_ids: &[Id], payload: &serde_json::Value) -> ApiResult<()> {
    for issue_id in issue_ids {
        let project_id: Option<Id> = query_one(tx, "SELECT project_id FROM issue WHERE id = ?1", [issue_id], |row| row.get(0))?;

        if let Some(project_id) = project_id {
            tx.emit("e2e.updated", Some(project_id), Some(*issue_id), payload.clone())?;
        }
    }

    Ok(())
}

/// Stores `run` for the project. `poster` is whoever sent it (none for example data): a test
/// is linked only to issues the poster may see, so a title cannot reach into a private project.
pub fn store_run(tx: &Tx, poster: Option<&AuthUser>, project_id: Id, run: &ParsedRun, meta: RunMeta) -> ApiResult<Stored> {
    let meta = meta.cleaned();
    let external_id = meta.run_id.clone().or_else(|| run.external_id.clone());

    if let Some(external_id) = &external_id {
        let known: Option<Id> =
            query_one(tx, "SELECT id FROM e2e_run WHERE project_id = ?1 AND external_id = ?2", (project_id, external_id), |row| row.get(0))?;

        if let Some(run_id) = known {
            return Ok(Stored { run_id, created: false, issues: Vec::new() });
        }
    }

    let count = |status: E2eStatus| run.results.iter().filter(|result| result.status == status).count() as i64;
    let costs: Vec<f64> = run.results.iter().filter_map(|result| result.cost_usd).collect();
    let started_at = meta.started_at.clone().or_else(|| run.started_at.clone()).unwrap_or_else(now_iso);

    tx.execute(
        "INSERT INTO e2e_run (project_id, external_id, name, format, status, branch, commit_sha, pr_number, ci_url, runner, error, started_at, duration_ms,
                              total, passed, failed, flaky, skipped, agent_steps, model_calls, model_tokens, cost_usd, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
        rusqlite::params![
            project_id,
            external_id,
            meta.name,
            run.format,
            run.status.as_str(),
            meta.branch.clone().or_else(|| run.branch.clone()),
            meta.commit.clone().or_else(|| run.commit.clone()),
            meta.pr_number,
            meta.ci_url,
            run.runner,
            run.error,
            started_at,
            run.duration_ms,
            run.results.len() as i64,
            count(E2eStatus::Passed),
            count(E2eStatus::Failed),
            count(E2eStatus::Flaky),
            count(E2eStatus::Skipped),
            run.results.iter().map(|result| result.agent_steps).sum::<i64>(),
            run.results.iter().map(|result| result.model_calls).sum::<i64>(),
            run.model_tokens.unwrap_or_else(|| run.results.iter().map(|result| result.model_tokens).sum()),
            run.cost_usd.or(if costs.is_empty() { None } else { Some(costs.iter().sum()) }),
            tx.actor.user_id,
            &tx.now,
        ],
    )?;
    let run_id = tx.last_insert_rowid();

    // A test on several targets is one test: it verifies what any of its results names.
    let mut tests: BTreeMap<Id, BTreeSet<String>> = BTreeMap::new();

    for result in &run.results {
        tx.execute(
            "INSERT INTO e2e_test (project_id, file, title, tags, line, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (project_id, file, title) DO UPDATE SET tags = excluded.tags, line = coalesce(excluded.line, e2e_test.line)",
            (project_id, &result.file, &result.title, serde_json::to_string(&result.tags)?, result.declared_line, &tx.now),
        )?;
        let test_id: Id = tx.query_row(
            "SELECT id FROM e2e_test WHERE project_id = ?1 AND file = ?2 AND title = ?3",
            (project_id, &result.file, &result.title),
            |row| row.get(0),
        )?;

        tx.execute(
            "INSERT INTO e2e_result (run_id, test_id, variant, status, duration_ms, attempts, line, message, error_code, agent_steps, model_calls,
                                     model_tokens, cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                run_id,
                test_id,
                result.variant,
                result.status.as_str(),
                result.duration_ms,
                result.attempts,
                result.line,
                result.message,
                result.error_code,
                result.agent_steps,
                result.model_calls,
                result.model_tokens,
                result.cost_usd,
            ],
        )?;

        tests.entry(test_id).or_default().extend(result.issue_keys.iter().cloned());
    }

    let mut may_see: HashMap<Id, bool> = HashMap::new();
    let mut affected: BTreeSet<Id> = BTreeSet::new();

    for (test_id, keys) in &tests {
        let before = linked_issues(tx, *test_id)?;
        let mut named: Vec<Id> = Vec::new();

        for key in keys {
            let Some((issue_id, issue_project)) = issue_id_by_key(tx, key)? else { continue };
            let visible = match poster {
                None => true,
                Some(poster) => *may_see.entry(issue_project).or_insert_with(|| project_access_by_id(tx, poster, issue_project).is_ok()),
            };
            let removed_by_hand: Option<Id> =
                query_one(tx, "SELECT issue_id FROM e2e_unlinked WHERE test_id = ?1 AND issue_id = ?2", (test_id, issue_id), |row| row.get(0))?;

            if visible && removed_by_hand.is_none() {
                named.push(issue_id);
            }
        }

        for (issue_id, manual) in &before {
            if !manual && !named.contains(issue_id) {
                tx.execute(
                    "DELETE FROM link WHERE from_kind = ?1 AND from_id = ?2 AND to_kind = 'issue' AND to_id = ?3 AND relation = ?4",
                    (TEST_KIND, test_id.to_string(), issue_id.to_string(), VERIFIES),
                )?;
            }
        }

        for issue_id in &named {
            tx.execute(
                "INSERT OR IGNORE INTO link (from_kind, from_id, to_kind, to_id, relation, created_at) VALUES (?1, ?2, 'issue', ?3, ?4, ?5)",
                (TEST_KIND, test_id.to_string(), issue_id.to_string(), VERIFIES, &tx.now),
            )?;
        }

        affected.extend(before.iter().map(|(issue_id, _)| *issue_id));
        affected.extend(named);
    }

    refresh_tests(tx, &tests.keys().copied().collect::<Vec<_>>())?;

    let issues: Vec<Id> = affected.into_iter().collect();
    let (total, passed, failed, flaky, skipped) =
        (run.results.len(), count(E2eStatus::Passed), count(E2eStatus::Failed), count(E2eStatus::Flaky), count(E2eStatus::Skipped));

    tx.emit(
        "e2e.run",
        Some(project_id),
        None,
        json!({ "runId": run_id, "status": run.status.as_str(), "total": total, "passed": passed, "failed": failed, "flaky": flaky, "skipped": skipped }),
    )?;
    announce_issues(tx, &issues, &json!({ "runId": run_id }))?;

    Ok(Stored { run_id, created: true, issues })
}

/// Links a test to an issue by hand, or removes the link whatever made it. A removed link
/// stays removed: later runs do not bring it back from the title.
pub fn set_manual_link(tx: &Tx, test_id: Id, issue_id: Id, linked: bool) -> ApiResult<()> {
    if linked {
        tx.execute("DELETE FROM e2e_unlinked WHERE test_id = ?1 AND issue_id = ?2", (test_id, issue_id))?;
        tx.execute(
            "INSERT OR IGNORE INTO link (from_kind, from_id, to_kind, to_id, relation, created_at) VALUES (?1, ?2, 'issue', ?3, ?4, ?5)",
            (TEST_KIND, test_id.to_string(), issue_id.to_string(), VERIFIES_MANUAL, &tx.now),
        )?;
    } else {
        let removed = tx.execute(
            "DELETE FROM link WHERE from_kind = ?1 AND from_id = ?2 AND to_kind = 'issue' AND to_id = ?3 AND relation IN (?4, ?5)",
            (TEST_KIND, test_id.to_string(), issue_id.to_string(), VERIFIES, VERIFIES_MANUAL),
        )?;

        if removed == 0 {
            return Err(ApiError::not_found("Link"));
        }

        tx.execute("INSERT OR IGNORE INTO e2e_unlinked (test_id, issue_id) VALUES (?1, ?2)", (test_id, issue_id))?;
    }

    announce_issues(tx, &[issue_id], &json!({ "testId": test_id }))
}

/// Removes edges whose test or issue no longer exists, so a reused id never inherits them.
pub fn forget_orphans(connection: &Connection) -> ApiResult<()> {
    connection.execute(
        "DELETE FROM link WHERE from_kind = ?1 AND (CAST(from_id AS INTEGER) NOT IN (SELECT id FROM e2e_test)
            OR (to_kind = 'issue' AND CAST(to_id AS INTEGER) NOT IN (SELECT id FROM issue)))",
        [TEST_KIND],
    )?;

    Ok(())
}

/// Forgets a test: its results, its links. The issues it verified are told.
pub fn delete_test(tx: &Tx, test_id: Id) -> ApiResult<()> {
    let issues: Vec<Id> = linked_issues(tx, test_id)?.into_iter().map(|(issue_id, _)| issue_id).collect();

    tx.execute("DELETE FROM link WHERE from_kind = ?1 AND from_id = ?2", (TEST_KIND, test_id.to_string()))?;
    tx.execute("DELETE FROM e2e_test WHERE id = ?1", [test_id])?;

    announce_issues(tx, &issues, &json!({ "testId": test_id }))
}

/// Removes a run. Its tests fall back to what the run before said; a test no other run
/// knows is forgotten with it, unless somebody linked it to an issue by hand.
pub fn delete_run(tx: &Tx, run_id: Id, project_id: Id) -> ApiResult<()> {
    let tests: Vec<Id> = query_all(tx, "SELECT DISTINCT test_id FROM e2e_result WHERE run_id = ?1", [run_id], |row| row.get(0))?;
    let mut affected: BTreeSet<Id> = BTreeSet::new();

    tx.execute("DELETE FROM e2e_run WHERE id = ?1", [run_id])?;

    for test_id in &tests {
        let links = linked_issues(tx, *test_id)?;
        affected.extend(links.iter().map(|(issue_id, _)| *issue_id));

        let remaining: i64 = tx.query_row("SELECT count(*) FROM e2e_result WHERE test_id = ?1", [test_id], |row| row.get(0))?;

        if remaining == 0 && !links.iter().any(|(_, manual)| *manual) {
            tx.execute("DELETE FROM link WHERE from_kind = ?1 AND from_id = ?2", (TEST_KIND, test_id.to_string()))?;
            tx.execute("DELETE FROM e2e_test WHERE id = ?1", [test_id])?;
        }
    }

    refresh_tests(tx, &tests)?;
    tx.emit("e2e.run", Some(project_id), None, json!({ "runId": run_id, "deleted": true }))?;
    announce_issues(tx, &affected.into_iter().collect::<Vec<_>>(), &json!({ "runId": run_id }))
}
