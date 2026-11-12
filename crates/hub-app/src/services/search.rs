//! Turns a parsed query into SQL over the issue table (aliased `i`, with its project as `p`).

use chrono::{DateTime, Datelike, Duration, NaiveDate, SecondsFormat, Utc};
use hub_core::keys::parse_issue_key;
use hub_core::query::{looks_like_plain_text, parse_query, Clause, ClauseOp, OrderBy, QueryError, QueryFieldInfo, QueryNode, QueryValue};
use hub_core::text::fold;
use hub_core::{priority_from_name, Id, SearchError, SearchResult};
use rusqlite::types::Value;
use rusqlite::Connection;

use crate::access::visible_projects;
use crate::app::App;
use crate::auth::AuthUser;
use crate::db::query_all;
use crate::error::ApiResult;
use crate::module::QueryScope;
use crate::services::issues::load_rows;
use crate::services::users::resolve_user;

pub struct Compiled {
    pub filter: String,
    pub params: Vec<Value>,
    pub order: String,
}

struct Compiler<'a> {
    app: &'a App,
    connection: &'a Connection,
    scope: QueryScope<'a>,
    params: Vec<Value>,
}

enum DateValue {
    Instant(DateTime<Utc>),
    Day(NaiveDate),
}

fn canonical(field: &str) -> String {
    let lower = field.to_lowercase();

    match lower.as_str() {
        "issue" | "id" | "issuekey" => "key",
        "issuetype" => "type",
        "statuscategory" => "category",
        "labels" => "label",
        "components" => "component",
        "fixversion" => "version",
        "epic" => "parent",
        "storypoints" => "points",
        "duedate" => "due",
        "comments" => "comment",
        other => other,
    }
    .to_string()
}

/// An FTS5 expression matching every word of `text` as a prefix, optionally in one column.
pub fn fts_match(text: &str, column: Option<&str>) -> Option<String> {
    let terms: Vec<String> = fold(text)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\"*"))
        .collect();

    if terms.is_empty() {
        return None;
    }

    let expression = terms.join(" ");

    Some(match column {
        Some(column) => format!("{column} : ({expression})"),
        None => expression,
    })
}

fn value_text(value: &QueryValue) -> Option<String> {
    match value {
        QueryValue::Text(text) => Some(text.clone()),
        QueryValue::Number(number) if number.fract() == 0.0 => Some(format!("{number:.0}")),
        QueryValue::Number(number) => Some(number.to_string()),
        QueryValue::Func(_) => None,
    }
}

fn iso(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn id_list(ids: &[Id]) -> String {
    if ids.is_empty() {
        "NULL".to_string()
    } else {
        ids.iter().map(Id::to_string).collect::<Vec<_>>().join(", ")
    }
}

impl Compiler<'_> {
    fn unsupported(&self, clause: &Clause) -> QueryError {
        QueryError::new(format!("That operator cannot be used with \"{}\"", clause.field), clause.position)
    }

    fn lookup(&self, sql: &str, value: &str, clause: &Clause) -> Result<Vec<Id>, QueryError> {
        query_all(self.connection, sql, [value], |row| row.get(0)).map_err(|error| QueryError::new(error.message, clause.position))
    }

    fn texts(&self, clause: &Clause) -> Result<Vec<String>, QueryError> {
        clause
            .values
            .iter()
            .map(|value| value_text(value).ok_or_else(|| QueryError::new(format!("A function cannot be used with \"{}\"", clause.field), clause.position)))
            .collect()
    }

    /// A condition on a column holding one id, given the ids the clause's values stand for.
    fn id_condition(&self, clause: &Clause, column: &str, ids: &[Id]) -> Result<String, QueryError> {
        match clause.op {
            ClauseOp::Eq | ClauseOp::In => Ok(format!("{column} IN ({})", id_list(ids))),
            ClauseOp::NotEq | ClauseOp::NotIn => Ok(format!("({column} IS NULL OR {column} NOT IN ({}))", id_list(ids))),
            ClauseOp::IsEmpty => Ok(format!("{column} IS NULL")),
            ClauseOp::IsNotEmpty => Ok(format!("{column} IS NOT NULL")),
            _ => Err(self.unsupported(clause)),
        }
    }

    /// Ids found by running `sql` (one text parameter) for each value; a value that finds
    /// nothing is an error naming it, so a typo does not silently match nothing.
    fn named_ids(&self, clause: &Clause, sql: &str, what: &str) -> Result<Vec<Id>, QueryError> {
        let mut ids = Vec::new();

        for text in self.texts(clause)? {
            let found = self.lookup(sql, &text, clause)?;

            if found.is_empty() {
                return Err(QueryError::new(format!("There is no {what} called \"{text}\""), clause.position));
            }

            ids.extend(found);
        }

        Ok(ids)
    }

    fn user_ids(&self, clause: &Clause) -> Result<Vec<Id>, QueryError> {
        let mut ids = Vec::new();

        for value in &clause.values {
            match value {
                QueryValue::Func(name) if name == "currentuser" => ids.push(self.scope.user.id),
                QueryValue::Func(name) => return Err(QueryError::new(format!("Unknown function {name}()"), clause.position)),
                other => {
                    let text = value_text(other).unwrap_or_default();
                    let found = resolve_user(self.connection, &text).map_err(|error| QueryError::new(error.message, clause.position))?;

                    ids.push(found.ok_or_else(|| QueryError::new(format!("There is nobody called \"{text}\""), clause.position))?);
                }
            }
        }

        Ok(ids)
    }

    fn sprint_ids(&self, clause: &Clause) -> Result<Vec<Id>, QueryError> {
        let mut ids = Vec::new();

        for value in &clause.values {
            let found = match value {
                QueryValue::Func(name) => {
                    let state = match name.as_str() {
                        "opensprints" => "active",
                        "futuresprints" => "future",
                        "closedsprints" => "closed",
                        _ => return Err(QueryError::new(format!("Unknown function {name}()"), clause.position)),
                    };

                    self.lookup("SELECT id FROM sprint WHERE state = ?1", state, clause)?
                }
                other => {
                    let text = value_text(other).unwrap_or_default();
                    let found = self.lookup("SELECT id FROM sprint WHERE name = ?1 COLLATE NOCASE OR CAST(id AS TEXT) = ?1", &text, clause)?;

                    if found.is_empty() {
                        return Err(QueryError::new(format!("There is no sprint called \"{text}\""), clause.position));
                    }

                    found
                }
            };

            ids.extend(found);
        }

        Ok(ids)
    }

    /// A condition for a many-to-many field: `subquery` selects matching rows for one text value.
    fn exists_condition(&mut self, clause: &Clause, any: &str, matching: &str) -> Result<String, QueryError> {
        match clause.op {
            ClauseOp::IsEmpty => Ok(format!("NOT EXISTS ({any})")),
            ClauseOp::IsNotEmpty => Ok(format!("EXISTS ({any})")),
            ClauseOp::Eq | ClauseOp::In | ClauseOp::NotEq | ClauseOp::NotIn => {
                let texts = self.texts(clause)?;
                let list = vec!["?"; texts.len()].join(", ");
                self.params.extend(texts.into_iter().map(Value::Text));
                let negation = if clause.op.is_negative() { "NOT " } else { "" };

                Ok(format!("{negation}EXISTS ({})", matching.replace("{list}", &list)))
            }
            _ => Err(self.unsupported(clause)),
        }
    }

    fn date_value(&self, clause: &Clause) -> Result<DateValue, QueryError> {
        let invalid = || QueryError::new(format!("\"{}\" needs a date such as 2026-11-30, -7d or startOfWeek()", clause.field), clause.position);
        let now = self.scope.now;
        let today = now.date_naive();

        match clause.values.first().ok_or_else(invalid)? {
            QueryValue::Func(name) => match name.as_str() {
                "now" => Ok(DateValue::Instant(now)),
                "startofday" => Ok(DateValue::Day(today)),
                "startofweek" => Ok(DateValue::Day(today - Duration::days(today.weekday().num_days_from_monday() as i64))),
                "startofmonth" => Ok(DateValue::Day(today.with_day(1).unwrap_or(today))),
                _ => Err(QueryError::new(format!("Unknown function {name}()"), clause.position)),
            },
            QueryValue::Number(_) => Err(invalid()),
            QueryValue::Text(text) => {
                if let Ok(day) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
                    return Ok(DateValue::Day(day));
                }

                if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
                    return Ok(DateValue::Instant(instant.with_timezone(&Utc)));
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

                Ok(DateValue::Instant(now + delta * sign))
            }
        }
    }

    fn date_condition(&mut self, clause: &Clause, column: &str, day_only: bool) -> Result<String, QueryError> {
        match clause.op {
            ClauseOp::IsEmpty => return Ok(format!("{column} IS NULL")),
            ClauseOp::IsNotEmpty => return Ok(format!("{column} IS NOT NULL")),
            ClauseOp::In | ClauseOp::NotIn | ClauseOp::Like | ClauseOp::NotLike => return Err(self.unsupported(clause)),
            _ => {}
        }

        let value = match self.date_value(clause)? {
            DateValue::Instant(instant) if day_only => DateValue::Day(instant.date_naive()),
            other => other,
        };
        let (start, end, is_day) = match value {
            DateValue::Instant(instant) => (iso(instant), iso(instant), false),
            DateValue::Day(day) if day_only => (day.to_string(), (day + Duration::days(1)).to_string(), true),
            DateValue::Day(day) => (format!("{day}T00:00:00.000Z"), format!("{}T00:00:00.000Z", day + Duration::days(1)), true),
        };

        let (sql, params) = match (clause.op, is_day) {
            (ClauseOp::Eq, true) => (format!("({column} >= ? AND {column} < ?)"), vec![start, end]),
            (ClauseOp::NotEq, true) => (format!("({column} IS NULL OR {column} < ? OR {column} >= ?)"), vec![start, end]),
            (ClauseOp::Eq, false) => (format!("{column} = ?"), vec![start]),
            (ClauseOp::NotEq, false) => (format!("({column} IS NULL OR {column} != ?)"), vec![start]),
            (ClauseOp::Less, _) => (format!("{column} < ?"), vec![start]),
            (ClauseOp::LessEq, true) => (format!("{column} < ?"), vec![end]),
            (ClauseOp::LessEq, false) => (format!("{column} <= ?"), vec![start]),
            (ClauseOp::Greater, true) => (format!("{column} >= ?"), vec![end]),
            (ClauseOp::Greater, false) => (format!("{column} > ?"), vec![start]),
            (ClauseOp::GreaterEq, _) => (format!("{column} >= ?"), vec![start]),
            _ => return Err(self.unsupported(clause)),
        };
        self.params.extend(params.into_iter().map(Value::Text));

        Ok(sql)
    }

    fn text_condition(&mut self, clause: &Clause, column: Option<&str>) -> Result<String, QueryError> {
        if !matches!(clause.op, ClauseOp::Like | ClauseOp::NotLike | ClauseOp::Eq | ClauseOp::NotEq) {
            return Err(self.unsupported(clause));
        }

        let text = self.texts(clause)?.join(" ");
        let Some(expression) = fts_match(&text, column) else { return Ok("0".to_string()) };
        self.params.push(Value::Text(expression));
        let negation = if clause.op.is_negative() { "NOT " } else { "" };

        Ok(format!("i.id {negation}IN (SELECT rowid FROM issue_fts WHERE issue_fts MATCH ?)"))
    }

    fn clause(&mut self, clause: &Clause) -> Result<String, QueryError> {
        let field = canonical(&clause.field);

        match field.as_str() {
            "project" => {
                let ids = self.named_ids(clause, "SELECT id FROM project WHERE key = ?1 COLLATE NOCASE OR name = ?1 COLLATE NOCASE", "project")?;

                self.id_condition(clause, "i.project_id", &ids)
            }
            "type" => {
                let ids = self.named_ids(clause, "SELECT id FROM issue_type WHERE name = ?1", "issue type")?;

                self.id_condition(clause, "i.type_id", &ids)
            }
            "status" => {
                let ids = self.named_ids(clause, "SELECT id FROM status WHERE name = ?1", "status")?;

                self.id_condition(clause, "i.status_id", &ids)
            }
            "category" => {
                let mut categories = Vec::new();

                for text in self.texts(clause)? {
                    categories.push(match fold(&text).replace(['_', ' ', '-'], "").as_str() {
                        "todo" | "new" | "open" => "todo",
                        "inprogress" | "indeterminate" | "started" => "in_progress",
                        "done" | "closed" | "resolved" => "done",
                        _ => return Err(QueryError::new("Category is one of: todo, \"in progress\", done", clause.position)),
                    });
                }

                let list = categories.iter().map(|category| format!("'{category}'")).collect::<Vec<_>>().join(", ");
                let negation = if clause.op.is_negative() { "NOT " } else { "" };

                match clause.op {
                    ClauseOp::Eq | ClauseOp::In | ClauseOp::NotEq | ClauseOp::NotIn => {
                        Ok(format!("i.status_id {negation}IN (SELECT id FROM status WHERE category IN ({list}))"))
                    }
                    _ => Err(self.unsupported(clause)),
                }
            }
            "priority" => {
                let mut priorities = Vec::new();

                for text in self.texts(clause)? {
                    let priority = priority_from_name(&text).or_else(|| text.parse::<i64>().ok().filter(|value| (1..=5).contains(value)));

                    priorities.push(priority.ok_or_else(|| {
                        QueryError::new("Priority is one of: Highest, High, Medium, Low, Lowest", clause.position)
                    })?);
                }

                // Priority 1 is the highest, so "greater than Medium" means a smaller number.
                let comparison = match clause.op {
                    ClauseOp::Less => Some(">"),
                    ClauseOp::LessEq => Some(">="),
                    ClauseOp::Greater => Some("<"),
                    ClauseOp::GreaterEq => Some("<="),
                    _ => None,
                };

                match comparison {
                    Some(operator) => Ok(format!("i.priority {operator} {}", priorities.first().copied().unwrap_or(3))),
                    None => self.id_condition(clause, "i.priority", &priorities).map(|sql| sql.replace("i.priority IS NULL OR ", "")),
                }
            }
            "assignee" => {
                let ids = self.user_ids(clause)?;

                self.id_condition(clause, "i.assignee_id", &ids)
            }
            "reporter" => {
                let ids = self.user_ids(clause)?;

                self.id_condition(clause, "i.reporter_id", &ids)
            }
            "watcher" => {
                let ids = id_list(&self.user_ids(clause)?);
                let negation = if clause.op.is_negative() { "NOT " } else { "" };

                Ok(format!("{negation}EXISTS (SELECT 1 FROM watcher w WHERE w.issue_id = i.id AND w.user_id IN ({ids}))"))
            }
            "label" => self.exists_condition(
                clause,
                "SELECT 1 FROM issue_label l WHERE l.issue_id = i.id",
                "SELECT 1 FROM issue_label l WHERE l.issue_id = i.id AND l.label IN ({list})",
            ),
            "component" => self.exists_condition(
                clause,
                "SELECT 1 FROM issue_component ic WHERE ic.issue_id = i.id",
                "SELECT 1 FROM issue_component ic JOIN component c ON c.id = ic.component_id WHERE ic.issue_id = i.id AND c.name IN ({list})",
            ),
            "sprint" => {
                let ids = self.sprint_ids(clause)?;

                self.id_condition(clause, "i.sprint_id", &ids)
            }
            "version" => {
                let ids = self.named_ids(clause, "SELECT id FROM version WHERE name = ?1", "version")?;

                self.id_condition(clause, "i.version_id", &ids)
            }
            "key" | "parent" => {
                let mut ids = Vec::new();

                for text in self.texts(clause)? {
                    let (project, number) =
                        parse_issue_key(&text).ok_or_else(|| QueryError::new(format!("\"{text}\" is not an issue key"), clause.position))?;

                    ids.extend(self.lookup("SELECT id FROM issue WHERE key = ?1", &format!("{project}-{number}"), clause)?);
                }

                self.id_condition(clause, if field == "key" { "i.id" } else { "i.parent_id" }, &ids)
            }
            "points" => match clause.op {
                ClauseOp::IsEmpty => Ok("i.points IS NULL".to_string()),
                ClauseOp::IsNotEmpty => Ok("i.points IS NOT NULL".to_string()),
                ClauseOp::Eq | ClauseOp::NotEq | ClauseOp::Less | ClauseOp::LessEq | ClauseOp::Greater | ClauseOp::GreaterEq => {
                    let Some(QueryValue::Number(number)) = clause.values.first() else {
                        return Err(QueryError::new("Points are compared with a number", clause.position));
                    };
                    self.params.push(Value::Real(*number));

                    Ok(format!("i.points {} ?", clause.op.sql()))
                }
                _ => Err(self.unsupported(clause)),
            },
            "text" => self.text_condition(clause, None),
            "summary" => self.text_condition(clause, Some("summary")),
            "description" => self.text_condition(clause, Some("description")),
            "comment" => self.text_condition(clause, Some("comments")),
            "created" => self.date_condition(clause, "i.created_at", false),
            "updated" => self.date_condition(clause, "i.updated_at", false),
            "resolved" => self.date_condition(clause, "i.resolved_at", false),
            "due" => self.date_condition(clause, "i.due_date", true),
            _ => {
                for module in self.app.enabled_modules() {
                    if let Some(compiled) = module.compile_field(self.connection, clause, &self.scope) {
                        let compiled = compiled.map_err(|error| QueryError::new(error.message, clause.position))?;
                        self.params.extend(compiled.params);

                        return Ok(format!("({})", compiled.sql));
                    }
                }

                Err(QueryError::new(format!("Unknown field \"{}\"", clause.field), clause.position))
            }
        }
    }

    fn node(&mut self, node: &QueryNode) -> Result<String, QueryError> {
        match node {
            QueryNode::Clause(clause) => self.clause(clause),
            QueryNode::Not(item) => Ok(format!("NOT ({})", self.node(item)?)),
            QueryNode::And(items) | QueryNode::Or(items) => {
                let joiner = if matches!(node, QueryNode::And(_)) { " AND " } else { " OR " };
                let parts = items.iter().map(|item| self.node(item)).collect::<Result<Vec<_>, _>>()?;

                Ok(format!("({})", parts.join(joiner)))
            }
        }
    }
}

fn order_sql(order_by: &[OrderBy]) -> Result<String, QueryError> {
    if order_by.is_empty() {
        return Ok("i.updated_at DESC, i.id DESC".to_string());
    }

    let mut parts = Vec::new();

    for order in order_by {
        let direction = if order.descending { "DESC" } else { "ASC" };
        let reverse = if order.descending { "ASC" } else { "DESC" };

        parts.push(match canonical(&order.field).as_str() {
            "created" => format!("i.created_at {direction}"),
            "updated" => format!("i.updated_at {direction}"),
            "resolved" => format!("i.resolved_at {direction}"),
            "due" => format!("i.due_date IS NULL, i.due_date {direction}"),
            // "priority DESC" reads as "most important first", and 1 is the most important.
            "priority" => format!("i.priority {reverse}"),
            "key" => format!("p.key {direction}, i.number {direction}"),
            "project" => format!("p.key {direction}"),
            "status" => format!("(SELECT position FROM status WHERE id = i.status_id) {direction}"),
            "type" => format!("(SELECT position FROM issue_type WHERE id = i.type_id) {direction}"),
            "assignee" => format!("(SELECT name FROM user WHERE id = i.assignee_id) COLLATE NOCASE {direction}"),
            "summary" => format!("i.summary COLLATE NOCASE {direction}"),
            "points" => format!("i.points {direction}"),
            "rank" => format!("i.rank {direction}"),
            _ => return Err(QueryError::new(format!("Cannot order by \"{}\"", order.field), order.position)),
        });
    }

    parts.push("i.id DESC".to_string());

    Ok(parts.join(", "))
}

pub fn compile(app: &App, connection: &Connection, user: &AuthUser, query: &str) -> Result<Compiled, QueryError> {
    let query = query.trim();

    if looks_like_plain_text(query) {
        let mut params = Vec::new();
        let mut filter = match fts_match(query, None) {
            Some(expression) => {
                params.push(Value::Text(expression));

                "i.id IN (SELECT rowid FROM issue_fts WHERE issue_fts MATCH ?)".to_string()
            }
            None => "0".to_string(),
        };

        if let Some((project, number)) = parse_issue_key(query) {
            params.push(Value::Text(format!("{project}-{number}")));
            filter = format!("({filter} OR i.key = ?)");
        }

        return Ok(Compiled { filter, params, order: order_sql(&[])? });
    }

    let parsed = parse_query(query)?;
    let mut compiler = Compiler { app, connection, scope: QueryScope { user, now: Utc::now() }, params: Vec::new() };
    let filter = match &parsed.filter {
        Some(node) => compiler.node(node)?,
        None => "1 = 1".to_string(),
    };

    Ok(Compiled { filter, params: compiler.params, order: order_sql(&parsed.order_by)? })
}

/// Runs a query. A query that cannot be understood is not a failure of the request: the result
/// carries the message and where in the text it applies.
pub fn search(app: &App, user: &AuthUser, query: &str, limit: i64, offset: i64) -> ApiResult<SearchResult> {
    let compiled = app.read(|connection| Ok(compile(app, connection, user, query)))?;
    let compiled = match compiled {
        Ok(compiled) => compiled,
        Err(error) => {
            return Ok(SearchResult { issues: Vec::new(), total: 0, error: Some(SearchError { message: error.message, position: error.position }) })
        }
    };
    let (visible, mut params) = visible_projects(user, "i.project_id");
    let condition = format!("WHERE {visible} AND ({})", compiled.filter);
    params.extend(compiled.params);

    let count_params = params.clone();
    let mut row_params = params;
    row_params.push(Value::Integer(limit.clamp(1, 500)));
    row_params.push(Value::Integer(offset.max(0)));

    let (total, issues) = rayon::join(
        || {
            app.read(|connection| {
                Ok(connection.query_row(
                    &format!("SELECT count(*) FROM issue i JOIN project p ON p.id = i.project_id {condition}"),
                    rusqlite::params_from_iter(count_params),
                    |row| row.get(0),
                )?)
            })
        },
        || app.read(|connection| load_rows(app, connection, &format!("{condition} ORDER BY {} LIMIT ? OFFSET ?", compiled.order), row_params)),
    );

    Ok(SearchResult { issues: issues?, total: total?, error: None })
}

/// The fields a query may use: the core's and those of every enabled module.
pub fn query_fields(app: &App) -> Vec<QueryFieldInfo> {
    let mut fields = vec![
        QueryFieldInfo::new("project", &[], "project", "project = WEB"),
        QueryFieldInfo::new("key", &["issue", "id"], "issue", "key in (WEB-1, WEB-2)"),
        QueryFieldInfo::new("type", &["issuetype"], "type", "type = Bug"),
        QueryFieldInfo::new("status", &[], "status", "status = \"In Progress\""),
        QueryFieldInfo::new("category", &["statusCategory"], "category", "category != done"),
        QueryFieldInfo::new("priority", &[], "priority", "priority >= High"),
        QueryFieldInfo::new("assignee", &[], "user", "assignee = currentUser()"),
        QueryFieldInfo::new("reporter", &[], "user", "reporter = \"Anna Nowak\""),
        QueryFieldInfo::new("watcher", &[], "user", "watcher = currentUser()"),
        QueryFieldInfo::new("label", &["labels"], "label", "label = backend"),
        QueryFieldInfo::new("component", &["components"], "component", "component = API"),
        QueryFieldInfo::new("sprint", &[], "sprint", "sprint in openSprints()"),
        QueryFieldInfo::new("version", &["fixVersion"], "version", "version = \"1.2\""),
        QueryFieldInfo::new("parent", &["epic"], "issue", "parent = WEB-3"),
        QueryFieldInfo::new("points", &["storyPoints"], "number", "points > 3"),
        QueryFieldInfo::new("text", &[], "text", "text ~ \"login error\""),
        QueryFieldInfo::new("summary", &[], "text", "summary ~ login"),
        QueryFieldInfo::new("description", &[], "text", "description ~ token"),
        QueryFieldInfo::new("comment", &[], "text", "comment ~ reproduced"),
        QueryFieldInfo::new("created", &[], "date", "created >= -7d"),
        QueryFieldInfo::new("updated", &[], "date", "updated < startOfWeek()"),
        QueryFieldInfo::new("resolved", &[], "date", "resolved >= startOfMonth()"),
        QueryFieldInfo::new("due", &["dueDate"], "date", "due < now()"),
    ];

    for module in app.enabled_modules() {
        fields.extend(module.query_fields());
    }

    fields
}
