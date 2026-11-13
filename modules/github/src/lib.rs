//! GitHub: repositories connected to projects, pull requests, branches and commits linked to
//! issues by key, workflow rules driven by what happens to the code, commands in commit
//! messages, and drift — where an issue's status and its code disagree.

pub mod actions;
pub mod client;
pub mod drift;
pub mod ingest;
pub mod model;
pub mod routes;
mod schema;
mod seed;

use std::time::Duration;

use axum::Router;
use hub_app::db::query_all;
use hub_app::secrets::secret_hint;
use hub_app::services::issues::{issue_access, row_by_id};
use hub_app::{ApiError, ApiResult, App, AuthUser, Event, FieldSql, Module, ModuleMeta, QueryScope, ToolSpec};
use hub_core::dev::DriftKind;
use hub_core::query::{Clause, ClauseOp, QueryFieldInfo, QueryValue};
use hub_core::{DemoLanguage, Id, IssueRow};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::model::{dev_summaries, pulls_where, stored_drift};

pub struct GithubModule;

const POLL_EVERY: Duration = Duration::from_secs(600);
const ISSUE_PR: &str = "SELECT 1 FROM gh_ref f JOIN gh_pr p ON p.id = f.ref_id WHERE f.kind = 'pr' AND f.issue_id = i.id";

fn clause_texts(clause: &Clause) -> ApiResult<Vec<String>> {
    clause
        .values
        .iter()
        .map(|value| match value {
            QueryValue::Text(text) => Ok(text.to_lowercase().replace([' ', '-'], "_")),
            _ => Err(ApiError::bad_request(format!("\"{}\" takes a word such as open or failing", clause.field))),
        })
        .collect()
}

/// `EXISTS (...)` over the issue's pull requests, for clauses that compare one column to a list of words.
fn pr_condition(clause: &Clause, column: &str, allowed: &[&str], extra: &str) -> ApiResult<FieldSql> {
    let negation = if clause.op.is_negative() { "NOT " } else { "" };

    match clause.op {
        ClauseOp::IsEmpty | ClauseOp::IsNotEmpty => {
            let negation = if clause.op == ClauseOp::IsEmpty { "NOT " } else { "" };

            Ok(FieldSql { sql: format!("{negation}EXISTS ({ISSUE_PR}{extra} AND {column} IS NOT NULL)"), params: Vec::new() })
        }
        ClauseOp::Eq | ClauseOp::NotEq | ClauseOp::In | ClauseOp::NotIn => {
            let words = clause_texts(clause)?;

            if let Some(unknown) = words.iter().find(|word| !allowed.contains(&word.as_str())) {
                return Err(ApiError::bad_request(format!("\"{unknown}\" is not one of: {}", allowed.join(", "))));
            }

            let list = vec!["?"; words.len()].join(", ");

            Ok(FieldSql {
                sql: format!("{negation}EXISTS ({ISSUE_PR}{extra} AND {column} IN ({list}))"),
                params: words.into_iter().map(SqlValue::Text).collect(),
            })
        }
        _ => Err(ApiError::bad_request(format!("That operator cannot be used with \"{}\"", clause.field))),
    }
}

impl Module for GithubModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: "github",
            name: "GitHub",
            description: "Pull requests, branches and commits on issues; workflow rules driven by code; drift between status and code.",
            version: env!("CARGO_PKG_VERSION"),
            depends_on: &[],
            default_enabled: true,
        }
    }

    fn migrations(&self) -> &'static [&'static str] {
        schema::MIGRATIONS
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(routes::routes())
    }

    fn hooks(&self) -> Option<Router<App>> {
        Some(routes::hooks())
    }

    /// An issue that changed status may have just come into, or out of, disagreement with its code.
    fn on_event(&self, app: &App, event: &Event) {
        if event.kind != "issue.transitioned" {
            return;
        }

        let Some(issue_id) = event.issue_id else { return };

        if let Err(error) = app.db.write(|tx| drift::recompute_drift(tx, &[issue_id])) {
            tracing::warn!("drift for issue {issue_id}: {}", error.message);
        }
    }

    fn query_fields(&self) -> Vec<QueryFieldInfo> {
        vec![
            QueryFieldInfo::new("pr", &["pullRequest"], "module", "pr = open"),
            QueryFieldInfo::new("checks", &[], "module", "checks = failing"),
            QueryFieldInfo::new("review", &[], "module", "review = changes_requested"),
            QueryFieldInfo::new("drift", &[], "module", "drift is not empty"),
        ]
    }

    fn compile_field(&self, _connection: &Connection, clause: &Clause, _scope: &QueryScope) -> Option<ApiResult<FieldSql>> {
        match clause.field.to_lowercase().as_str() {
            "pr" | "pullrequest" => Some((|| {
                if matches!(clause.op, ClauseOp::IsEmpty | ClauseOp::IsNotEmpty) {
                    let negation = if clause.op == ClauseOp::IsEmpty { "NOT " } else { "" };

                    return Ok(FieldSql { sql: format!("{negation}EXISTS ({ISSUE_PR})"), params: Vec::new() });
                }

                let words = clause_texts(clause)?;

                if words.iter().any(|word| word == "none") {
                    let negation = if clause.op.is_negative() { "" } else { "NOT " };

                    return Ok(FieldSql { sql: format!("{negation}EXISTS ({ISSUE_PR})"), params: Vec::new() });
                }

                if words.iter().any(|word| word == "draft") {
                    let negation = if clause.op.is_negative() { "NOT " } else { "" };

                    return Ok(FieldSql { sql: format!("{negation}EXISTS ({ISSUE_PR} AND p.state = 'open' AND p.draft = 1)"), params: Vec::new() });
                }

                pr_condition(clause, "p.state", &["open", "merged", "closed"], "")
            })()),
            "checks" => Some(pr_condition(clause, "p.checks", &["passing", "failing", "running"], " AND p.state = 'open'")),
            "review" => Some(pr_condition(clause, "p.review", &["approved", "changes_requested", "pending"], " AND p.state = 'open'")),
            "drift" => Some((|| {
                let any = "SELECT 1 FROM gh_drift d WHERE d.issue_id = i.id";

                match clause.op {
                    ClauseOp::IsEmpty => Ok(FieldSql { sql: format!("NOT EXISTS ({any})"), params: Vec::new() }),
                    ClauseOp::IsNotEmpty => Ok(FieldSql { sql: format!("EXISTS ({any})"), params: Vec::new() }),
                    ClauseOp::Eq | ClauseOp::NotEq | ClauseOp::In | ClauseOp::NotIn => {
                        let words = clause_texts(clause)?;

                        if let Some(unknown) = words.iter().find(|word| DriftKind::parse(word).is_none()) {
                            let kinds: Vec<&str> = DriftKind::ALL.iter().map(|kind| kind.as_str()).collect();

                            return Err(ApiError::bad_request(format!("\"{unknown}\" is not a kind of drift ({})", kinds.join(", "))));
                        }

                        let negation = if clause.op.is_negative() { "NOT " } else { "" };
                        let list = vec!["?"; words.len()].join(", ");

                        Ok(FieldSql {
                            sql: format!("{negation}EXISTS ({any} AND d.kind IN ({list}))"),
                            params: words.into_iter().map(SqlValue::Text).collect(),
                        })
                    }
                    _ => Err(ApiError::bad_request("That operator cannot be used with \"drift\"")),
                }
            })()),
            _ => None,
        }
    }

    fn decorate_issues(&self, connection: &Connection, rows: &mut [IssueRow]) -> ApiResult<()> {
        let ids: Vec<Id> = rows.iter().map(|row| row.id).collect();
        let mut summaries = dev_summaries(connection, &ids)?;
        let mut drift = stored_drift(connection, &ids)?;

        for row in rows {
            let summary = summaries.remove(&row.id);
            let kinds = drift.remove(&row.id);

            if summary.is_none() && kinds.is_none() {
                continue;
            }

            let mut summary = summary.unwrap_or_default();
            summary.drift = kinds.unwrap_or_default();
            row.ext.insert("github".to_string(), serde_json::to_value(summary)?);
        }

        Ok(())
    }

    fn issue_panel(&self, app: &App, connection: &Connection, issue: &IssueRow, _viewer: &AuthUser) -> ApiResult<Option<Value>> {
        Ok(routes::issue_panel(app, connection, issue)?.map(serde_json::to_value).transpose()?)
    }

    /// My open pull requests, and my issues whose status disagrees with their code.
    fn dashboard(&self, connection: &Connection, viewer: &AuthUser) -> ApiResult<Option<Value>> {
        // Projects that have this module switched off stay out of the section.
        let on = |column: &str| hub_app::services::project_modules::on_in_project("github", column);
        let login: Option<String> = connection.query_row("SELECT github_login FROM user WHERE id = ?1", [viewer.id], |row| row.get(0))?;
        let pulls = match login.filter(|login| login.chars().all(|character| character.is_ascii_alphanumeric() || character == '-')) {
            Some(login) => {
                pulls_where(connection, &format!("p.state = 'open' AND p.author = '{login}' COLLATE NOCASE AND {}", on("r.project_id")), 20)?
            }
            None => Vec::new(),
        };
        let drifting = query_all(
            connection,
            &format!(
                "SELECT i.key, i.summary, group_concat(d.kind) FROM gh_drift d JOIN issue i ON i.id = d.issue_id
                 WHERE i.assignee_id = ?1 AND {} GROUP BY i.id ORDER BY i.updated_at DESC LIMIT 20",
                on("i.project_id")
            ),
            [viewer.id],
            |row| {
                let kinds: String = row.get(2)?;

                Ok(json!({ "key": row.get::<_, String>(0)?, "summary": row.get::<_, String>(1)?, "kinds": kinds.split(',').collect::<Vec<_>>() }))
            },
        )?;

        if pulls.is_empty() && drifting.is_empty() {
            return Ok(None);
        }

        Ok(Some(json!({ "pulls": pulls, "drift": drifting })))
    }

    fn tools(&self) -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "get_issue_development",
            description: "Branches, commits and pull requests linked to an issue (with review and check state), the branch name suggested for it, and any drift between its status and its code.",
            input_schema: json!({ "type": "object", "properties": { "key": { "type": "string", "description": "Issue key, e.g. WEB-12" } }, "required": ["key"] }),
            read_only: true,
        }]
    }

    fn call_tool(&self, app: &App, user: &AuthUser, name: &str, arguments: &Value) -> Option<ApiResult<Value>> {
        if name != "get_issue_development" {
            return None;
        }

        Some(app.read(|connection| {
            let key = arguments["key"].as_str().ok_or_else(|| ApiError::bad_request("\"key\" is required"))?;
            let (issue_id, _) = issue_access(connection, user, key)?;
            let issue = row_by_id(app, connection, issue_id)?;

            Ok(serde_json::to_value(routes::issue_panel(app, connection, &issue)?)?)
        }))
    }

    /// Every ten minutes: catch up through the API when a token is configured (webhooks can
    /// be missed), and re-judge drift, some of which appears just by time passing.
    fn start(&self, app: &App) {
        let app = app.clone();

        let spawned = std::thread::Builder::new().name("github-poll".to_string()).spawn(move || loop {
            std::thread::sleep(POLL_EVERY);

            if !app.module_enabled("github") {
                continue;
            }

            let has_token = app.read(|connection| secret_hint(connection, client::TOKEN_SECRET)).ok().flatten().is_some();

            if has_token {
                for repo in app.read(model::all_repos).unwrap_or_default() {
                    let off = app.read(|connection| hub_app::services::project_modules::disabled_modules(connection, repo.project_id)).unwrap_or_default();

                    if off.iter().any(|id| id == "github") {
                        continue;
                    }

                    if let Err(error) = client::sync_and_record(&app, &repo) {
                        tracing::warn!("sync of {}: {}", repo.full_name, error.message);
                    }
                }
            }

            if let Err(error) = app.db.write(|tx| drift::recompute_all(tx)) {
                tracing::warn!("drift: {}", error.message);
            }
        });

        if let Err(error) = spawned {
            tracing::error!("could not start the GitHub poller: {error}");
        }
    }

    fn seed_demo(&self, app: &App, language: DemoLanguage) -> ApiResult<()> {
        seed::seed_demo(app, language)
    }
}
