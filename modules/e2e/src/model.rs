//! The queries that read runs, tests and their links to issues.

use std::collections::HashMap;

use hub_app::access::visible_projects;
use hub_app::db::{query_all, query_one};
use hub_app::{ApiResult, AuthUser};
use hub_core::Id;
use rayon::prelude::*;
use rusqlite::types::Value;
use rusqlite::{Connection, Row};

use crate::types::{
    E2eBadge, E2eCounts, E2eHistoryPoint, E2eIssueRef, E2eIssueState, E2eIssueTest, E2eRun, E2eRunResult, E2eRunStatus, E2eStatus, E2eTest,
};

/// Kind of a test in the shared `link` table.
pub const TEST_KIND: &str = "e2e_test";
/// A link read from the test's title, tags or annotations; replaced by every run.
pub const VERIFIES: &str = "verifies";
/// A link set by hand; runs leave it alone.
pub const VERIFIES_MANUAL: &str = "verifies_manual";
/// How many recent runs a test's history strip shows.
pub const HISTORY: usize = 12;
/// History is read from this many of a project's newest runs.
const HISTORY_RUNS: i64 = 60;

/// The tests linked to the issue aliased `i`, for use inside `EXISTS (...)` or an aggregate.
pub const ISSUE_TESTS: &str = "link l JOIN e2e_test t ON t.id = CAST(l.from_id AS INTEGER)
    WHERE l.from_kind = 'e2e_test' AND l.to_kind = 'issue' AND l.to_id = CAST(i.id AS TEXT)";

/// What the tests of the issue aliased `i` say together, as SQL: `none` without tests, else
/// one of the [`E2eIssueState`] words.
pub fn issue_state_sql() -> String {
    format!(
        "(SELECT CASE WHEN count(*) = 0 THEN 'none'
                      WHEN sum(t.last_status = 'failed') > 0 THEN 'failing'
                      WHEN sum(t.last_status = 'flaky') > 0 THEN 'flaky'
                      WHEN sum(t.last_status = 'passed') > 0 THEN 'passing'
                      ELSE 'skipped' END
          FROM {ISSUE_TESTS})"
    )
}

const RUN_COLUMNS: &str = "r.id, p.key, r.name, r.format, r.status, r.branch, r.commit_sha, r.pr_number, r.ci_url, r.runner, r.error, r.started_at,
    r.duration_ms, r.total, r.passed, r.failed, r.flaky, r.skipped, r.agent_steps, r.model_calls, r.model_tokens, r.cost_usd, r.created_by, r.created_at";

fn map_run(row: &Row) -> rusqlite::Result<E2eRun> {
    let status: String = row.get(4)?;

    Ok(E2eRun {
        id: row.get(0)?,
        project_key: row.get(1)?,
        name: row.get(2)?,
        format: row.get(3)?,
        status: E2eRunStatus::parse(&status).unwrap_or(E2eRunStatus::Failed),
        branch: row.get(5)?,
        commit: row.get(6)?,
        pr_number: row.get(7)?,
        ci_url: row.get(8)?,
        runner: row.get(9)?,
        error: row.get(10)?,
        started_at: row.get(11)?,
        duration_ms: row.get(12)?,
        total: row.get(13)?,
        passed: row.get(14)?,
        failed: row.get(15)?,
        flaky: row.get(16)?,
        skipped: row.get(17)?,
        agent_steps: row.get(18)?,
        model_calls: row.get(19)?,
        model_tokens: row.get(20)?,
        cost_usd: row.get(21)?,
        created_by: row.get(22)?,
        created_at: row.get(23)?,
    })
}

/// A run and the id of its project.
pub fn run_by_id(connection: &Connection, id: Id) -> ApiResult<Option<(E2eRun, Id)>> {
    query_one(
        connection,
        &format!("SELECT {RUN_COLUMNS}, r.project_id FROM e2e_run r JOIN project p ON p.id = r.project_id WHERE r.id = ?1"),
        [id],
        |row| Ok((map_run(row)?, row.get(24)?)),
    )
}

/// A project's runs, newest first.
pub fn project_runs(connection: &Connection, project_id: Id, limit: i64) -> ApiResult<Vec<E2eRun>> {
    query_all(
        connection,
        &format!("SELECT {RUN_COLUMNS} FROM e2e_run r JOIN project p ON p.id = r.project_id WHERE r.project_id = ?1 ORDER BY r.started_at DESC, r.id DESC LIMIT ?2"),
        (project_id, limit),
        map_run,
    )
}

fn status_of(text: Option<String>) -> Option<E2eStatus> {
    text.as_deref().and_then(E2eStatus::parse)
}

fn status_from_severity(severity: i64) -> E2eStatus {
    match severity {
        3 => E2eStatus::Failed,
        2 => E2eStatus::Flaky,
        1 => E2eStatus::Passed,
        _ => E2eStatus::Skipped,
    }
}

/// The recent results of the tests selected by `tests` (a subquery yielding test ids), newest
/// first per test. A test that ran on several targets in one run counts once, by its worst result.
fn recent_results(connection: &Connection, tests: &str, params: Vec<Value>) -> ApiResult<HashMap<Id, Vec<E2eHistoryPoint>>> {
    let rows = query_all(
        connection,
        &format!(
            "SELECT test_id, run_id, worst, started_at, branch FROM (
                SELECT x.test_id, x.run_id, x.worst, r.started_at, r.branch,
                       row_number() OVER (PARTITION BY x.test_id ORDER BY r.started_at DESC, r.id DESC) AS position
                FROM (SELECT e.test_id, e.run_id,
                             max(CASE e.status WHEN 'failed' THEN 3 WHEN 'flaky' THEN 2 WHEN 'passed' THEN 1 ELSE 0 END) AS worst
                      FROM e2e_result e
                      WHERE e.test_id IN ({tests})
                        AND e.run_id IN (SELECT id FROM e2e_run WHERE project_id IN (SELECT project_id FROM e2e_test WHERE id IN ({tests}))
                                         ORDER BY started_at DESC, id DESC LIMIT {HISTORY_RUNS})
                      GROUP BY e.test_id, e.run_id) x
                JOIN e2e_run r ON r.id = x.run_id)
             WHERE position <= {HISTORY} ORDER BY test_id, position"
        ),
        rusqlite::params_from_iter(params.iter().chain(params.iter())),
        |row| {
            Ok((
                row.get::<_, Id>(0)?,
                E2eHistoryPoint { run_id: row.get(1)?, status: status_from_severity(row.get(2)?), at: row.get(3)?, branch: row.get(4)? },
            ))
        },
    )?;
    let mut history: HashMap<Id, Vec<E2eHistoryPoint>> = HashMap::new();

    for (test_id, point) in rows {
        history.entry(test_id).or_default().push(point);
    }

    Ok(history)
}

/// A test is unstable when its newest result is flaky, or when it passed and failed by
/// turns in its recent runs on one branch (a change of branch explains a change of result).
fn is_unstable(status: Option<E2eStatus>, newest_first: &[E2eHistoryPoint]) -> bool {
    if status == Some(E2eStatus::Flaky) || newest_first.iter().any(|point| point.status == E2eStatus::Flaky) {
        return true;
    }

    let mut flips: HashMap<Option<&str>, (Option<E2eStatus>, u32)> = HashMap::new();

    for point in newest_first.iter().filter(|point| point.status != E2eStatus::Skipped) {
        let entry = flips.entry(point.branch.as_deref()).or_insert((None, 0));

        if entry.0.is_some_and(|previous| previous != point.status) {
            entry.1 += 1;
        }

        entry.0 = Some(point.status);
    }

    // One change is a regression or a fix; back and forth is instability.
    flips.values().any(|(_, changes)| *changes >= 2)
}

struct TestRow {
    id: Id,
    file: String,
    title: String,
    tags: String,
    line: Option<i64>,
    status: Option<String>,
    last_run_id: Option<Id>,
    last_at: Option<String>,
    duration_ms: Option<i64>,
    attempts: Option<i64>,
    message: Option<String>,
    error_code: Option<String>,
    branch: Option<String>,
    commit: Option<String>,
}

/// The issues each of the tests selected by `tests` verifies, limited to the ones `viewer` may see.
fn test_issues(connection: &Connection, viewer: &AuthUser, tests: &str, mut params: Vec<Value>) -> ApiResult<HashMap<Id, Vec<E2eIssueRef>>> {
    let (visible, visible_params) = visible_projects(viewer, "i.project_id");
    params.extend(visible_params);

    let rows = query_all(
        connection,
        &format!(
            "SELECT CAST(l.from_id AS INTEGER), i.key, i.summary, s.name, s.category = 'done', max(l.relation = '{VERIFIES_MANUAL}')
             FROM link l JOIN issue i ON i.id = CAST(l.to_id AS INTEGER) JOIN status s ON s.id = i.status_id
             WHERE l.from_kind = '{TEST_KIND}' AND l.to_kind = 'issue' AND l.from_id IN (SELECT CAST(id AS TEXT) FROM e2e_test WHERE id IN ({tests}))
               AND {visible}
             GROUP BY l.from_id, i.id ORDER BY i.project_id, i.number"
        ),
        rusqlite::params_from_iter(params),
        |row| {
            Ok((
                row.get::<_, Id>(0)?,
                E2eIssueRef { key: row.get(1)?, summary: row.get(2)?, status: row.get(3)?, done: row.get(4)?, manual: row.get(5)? },
            ))
        },
    )?;
    let mut issues: HashMap<Id, Vec<E2eIssueRef>> = HashMap::new();

    for (test_id, issue) in rows {
        issues.entry(test_id).or_default().push(issue);
    }

    Ok(issues)
}

/// Every test of a project with its latest result, recent history and issues: failing tests
/// first, then flaky ones, then by file and title.
pub fn project_tests(connection: &Connection, viewer: &AuthUser, project_id: Id) -> ApiResult<Vec<E2eTest>> {
    let tests = "SELECT id FROM e2e_test WHERE project_id = ?";
    let params = vec![Value::Integer(project_id)];
    let rows = query_all(
        connection,
        "SELECT t.id, t.file, t.title, t.tags, t.line, t.last_status, t.last_run_id, t.last_at, t.last_duration_ms, t.last_attempts, t.last_message,
                t.last_error_code, r.branch, r.commit_sha
         FROM e2e_test t LEFT JOIN e2e_run r ON r.id = t.last_run_id WHERE t.project_id = ?1",
        [project_id],
        |row| {
            Ok(TestRow {
                id: row.get(0)?,
                file: row.get(1)?,
                title: row.get(2)?,
                tags: row.get(3)?,
                line: row.get(4)?,
                status: row.get(5)?,
                last_run_id: row.get(6)?,
                last_at: row.get(7)?,
                duration_ms: row.get(8)?,
                attempts: row.get(9)?,
                message: row.get(10)?,
                error_code: row.get(11)?,
                branch: row.get(12)?,
                commit: row.get(13)?,
            })
        },
    )?;
    let history = recent_results(connection, tests, params.clone())?;
    let issues = test_issues(connection, viewer, tests, params)?;

    // Putting each test together is independent work; a project can have thousands.
    let mut tests: Vec<E2eTest> = rows
        .into_par_iter()
        .map(|row| {
            let newest_first = history.get(&row.id).map(Vec::as_slice).unwrap_or(&[]);
            let status = status_of(row.status);

            E2eTest {
                id: row.id,
                file: row.file,
                title: row.title,
                tags: serde_json::from_str(&row.tags).unwrap_or_default(),
                line: row.line,
                status,
                last_run_id: row.last_run_id,
                last_at: row.last_at,
                branch: row.branch,
                commit: row.commit,
                duration_ms: row.duration_ms,
                attempts: row.attempts,
                message: row.message,
                error_code: row.error_code,
                unstable: is_unstable(status, newest_first),
                history: newest_first.iter().rev().cloned().collect(),
                issues: issues.get(&row.id).cloned().unwrap_or_default(),
            }
        })
        .collect();

    tests.par_sort_by(|a, b| {
        let weight = |test: &E2eTest| match test.status {
            Some(E2eStatus::Failed) => 0,
            Some(E2eStatus::Flaky) => 1,
            _ => 2,
        };

        (weight(a), &a.file, &a.title).cmp(&(weight(b), &b.file, &b.title))
    });

    Ok(tests)
}

pub fn count_tests(tests: &[E2eTest]) -> E2eCounts {
    let count = |wanted: fn(&E2eTest) -> bool| tests.iter().filter(|test| wanted(test)).count() as i64;

    E2eCounts {
        total: tests.len() as i64,
        passing: count(|test| test.status == Some(E2eStatus::Passed)),
        failing: count(|test| test.status == Some(E2eStatus::Failed)),
        flaky: count(|test| test.unstable),
        skipped: count(|test| matches!(test.status, None | Some(E2eStatus::Skipped))),
        unlinked: count(|test| test.issues.is_empty()),
    }
}

const RESULT_COLUMNS: &str = "e.id, e.test_id, t.title, t.file, coalesce(e.line, t.line), e.variant, e.status, e.duration_ms, e.attempts, e.message, e.error_code,
    e.agent_steps, e.model_calls, e.model_tokens, e.cost_usd, e.videos, e.screenshots, e.session IS NOT NULL";

fn map_result(row: &Row) -> rusqlite::Result<E2eRunResult> {
    let status: String = row.get(6)?;

    Ok(E2eRunResult {
        id: row.get(0)?,
        test_id: row.get(1)?,
        title: row.get(2)?,
        file: row.get(3)?,
        line: row.get(4)?,
        variant: row.get(5)?,
        status: E2eStatus::parse(&status).unwrap_or(E2eStatus::Skipped),
        duration_ms: row.get(7)?,
        attempts: row.get(8)?,
        message: row.get(9)?,
        error_code: row.get(10)?,
        agent_steps: row.get(11)?,
        model_calls: row.get(12)?,
        model_tokens: row.get(13)?,
        cost_usd: row.get(14)?,
        videos: row.get(15)?,
        screenshots: row.get(16)?,
        has_session: row.get(17)?,
        issues: Vec::new(),
    })
}

/// The tests of one run, failures first.
pub fn run_results(connection: &Connection, viewer: &AuthUser, run_id: Id) -> ApiResult<Vec<E2eRunResult>> {
    let tests = "SELECT test_id FROM e2e_result WHERE run_id = ?";
    let issues = test_issues(connection, viewer, tests, vec![Value::Integer(run_id)])?;
    let mut results = query_all(
        connection,
        &format!(
            "SELECT {RESULT_COLUMNS} FROM e2e_result e JOIN e2e_test t ON t.id = e.test_id WHERE e.run_id = ?1
             ORDER BY CASE e.status WHEN 'failed' THEN 0 WHEN 'flaky' THEN 1 WHEN 'passed' THEN 2 ELSE 3 END, t.file, t.title, e.variant"
        ),
        [run_id],
        map_result,
    )?;

    for result in &mut results {
        result.issues = issues.get(&result.test_id).cloned().unwrap_or_default();
    }

    Ok(results)
}

/// One result of a run, with the issues its test verifies.
pub fn run_result(connection: &Connection, viewer: &AuthUser, result_id: Id) -> ApiResult<Option<E2eRunResult>> {
    let found =
        query_one(connection, &format!("SELECT {RESULT_COLUMNS} FROM e2e_result e JOIN e2e_test t ON t.id = e.test_id WHERE e.id = ?1"), [result_id], map_result)?;
    let Some(mut result) = found else { return Ok(None) };

    result.issues = test_issues(connection, viewer, "?", vec![Value::Integer(result.test_id)])?.remove(&result.test_id).unwrap_or_default();

    Ok(Some(result))
}

/// The tests that verify the issues selected by `condition` (on the issue aliased `i`), each
/// with the issue's id, limited to tests of projects `viewer` may see.
pub fn tests_of_issues(connection: &Connection, viewer: &AuthUser, condition: &str, mut params: Vec<Value>) -> ApiResult<Vec<(Id, E2eIssueTest)>> {
    let (visible, visible_params) = visible_projects(viewer, "t.project_id");
    params.extend(visible_params);

    let mut rows = query_all(
        connection,
        &format!(
            "SELECT i.id, t.id, p.key, t.title, t.file, t.line, t.last_status, t.last_at, t.last_run_id, r.name, r.branch, r.commit_sha, r.ci_url,
                    t.last_duration_ms, t.last_message, max(l.relation = '{VERIFIES_MANUAL}')
             FROM issue i
             JOIN link l ON l.to_kind = 'issue' AND l.to_id = CAST(i.id AS TEXT) AND l.from_kind = '{TEST_KIND}'
             JOIN e2e_test t ON t.id = CAST(l.from_id AS INTEGER)
             JOIN project p ON p.id = t.project_id
             LEFT JOIN e2e_run r ON r.id = t.last_run_id
             WHERE ({condition}) AND {visible}
             GROUP BY i.id, t.id
             ORDER BY i.id, CASE t.last_status WHEN 'failed' THEN 0 WHEN 'flaky' THEN 1 WHEN 'passed' THEN 2 ELSE 3 END, t.file, t.title"
        ),
        rusqlite::params_from_iter(params),
        |row| {
            Ok((
                row.get::<_, Id>(0)?,
                E2eIssueTest {
                    test_id: row.get(1)?,
                    project_key: row.get(2)?,
                    title: row.get(3)?,
                    file: row.get(4)?,
                    line: row.get(5)?,
                    status: status_of(row.get(6)?),
                    at: row.get(7)?,
                    run_id: row.get(8)?,
                    run_name: row.get(9)?,
                    branch: row.get(10)?,
                    commit: row.get(11)?,
                    ci_url: row.get(12)?,
                    duration_ms: row.get(13)?,
                    message: row.get(14)?,
                    manual: row.get(15)?,
                    history: Vec::new(),
                },
            ))
        },
    )?;

    if rows.is_empty() {
        return Ok(rows);
    }

    let ids = rows.iter().map(|(_, test)| test.test_id.to_string()).collect::<Vec<_>>().join(", ");
    let history = recent_results(connection, &ids, Vec::new())?;

    for (_, test) in &mut rows {
        test.history = history.get(&test.test_id).map(|points| points.iter().rev().cloned().collect()).unwrap_or_default();
    }

    Ok(rows)
}

pub fn state_of(tests: &[E2eIssueTest]) -> Option<E2eIssueState> {
    if tests.is_empty() {
        return None;
    }

    let count = |wanted: E2eStatus| tests.iter().filter(|test| test.status == Some(wanted)).count() as i64;

    Some(E2eIssueState::of(count(E2eStatus::Passed), count(E2eStatus::Failed), count(E2eStatus::Flaky)))
}

/// The mark of each issue that has tests, by issue id.
pub fn badges(connection: &Connection, issue_ids: &[Id]) -> ApiResult<HashMap<Id, E2eBadge>> {
    if issue_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let ids = issue_ids.iter().map(|id| format!("'{id}'")).collect::<Vec<_>>().join(", ");
    let rows = query_all(
        connection,
        &format!(
            "SELECT CAST(l.to_id AS INTEGER), count(DISTINCT t.id),
                    count(DISTINCT CASE WHEN t.last_status = 'passed' THEN t.id END),
                    count(DISTINCT CASE WHEN t.last_status = 'failed' THEN t.id END),
                    count(DISTINCT CASE WHEN t.last_status = 'flaky' THEN t.id END)
             FROM link l JOIN e2e_test t ON t.id = CAST(l.from_id AS INTEGER)
             WHERE l.from_kind = '{TEST_KIND}' AND l.to_kind = 'issue' AND l.to_id IN ({ids})
             GROUP BY l.to_id"
        ),
        [],
        |row| {
            let (passing, failing, flaky): (i64, i64, i64) = (row.get(2)?, row.get(3)?, row.get(4)?);

            Ok((row.get::<_, Id>(0)?, E2eBadge { state: E2eIssueState::of(passing, failing, flaky), total: row.get(1)?, passing, failing, flaky }))
        },
    )?;

    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(status: E2eStatus, branch: &str) -> E2eHistoryPoint {
        E2eHistoryPoint { run_id: 1, status, at: String::new(), branch: Some(branch.to_string()) }
    }

    #[test]
    fn unstable_means_flaky_or_back_and_forth_on_one_branch() {
        use E2eStatus::{Failed, Flaky, Passed, Skipped};

        assert!(is_unstable(Some(Flaky), &[]));
        assert!(is_unstable(Some(Passed), &[point(Passed, "main"), point(Flaky, "main")]));
        assert!(is_unstable(Some(Passed), &[point(Passed, "main"), point(Failed, "main"), point(Passed, "main")]));
        // A regression is one change, not instability; neither is a failure on another branch.
        assert!(!is_unstable(Some(Failed), &[point(Failed, "main"), point(Failed, "main"), point(Passed, "main")]));
        assert!(!is_unstable(Some(Passed), &[point(Passed, "main"), point(Failed, "fix/a"), point(Passed, "main"), point(Skipped, "main")]));
    }
}
