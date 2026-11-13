//! Reads what the e2e framework writes about a run: its JSON report (`report-1`, the document
//! the `json` reporter prints and `.e2e/report.json` holds) and the JUnit XML of its `junit`
//! reporter. Both become the same [`ParsedRun`]. Nothing here touches the database.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use hub_app::{ApiError, ApiResult};
use hub_core::keys::{extract_issue_keys, KeyCase};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::Value;

use crate::types::{E2eRunStatus, E2eStatus};

const MAX_MESSAGE_CHARS: usize = 4000;
const MAX_TITLE_CHARS: usize = 500;
const MAX_FILE_CHARS: usize = 300;
/// Between the describe titles and the test's own, as the framework's JUnit names join them.
pub const TITLE_SEPARATOR: &str = " > ";

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedResult {
    pub file: String,
    pub title: String,
    /// Target, agent and repeat, when the run had more than one of them.
    pub variant: String,
    pub tags: Vec<String>,
    /// Where the test is declared.
    pub declared_line: Option<i64>,
    /// The line that failed, else where the test is declared.
    pub line: Option<i64>,
    pub status: E2eStatus,
    pub duration_ms: i64,
    pub attempts: i64,
    pub message: Option<String>,
    pub error_code: Option<String>,
    pub agent_steps: i64,
    pub model_calls: i64,
    pub model_tokens: i64,
    pub cost_usd: Option<f64>,
    /// Annotation texts, read for issue keys beside the title.
    pub notes: Vec<String>,
    /// Issue keys the title, tags and annotations mention; filled by [`ParsedRun::find_issue_keys`].
    pub issue_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRun {
    pub format: &'static str,
    pub external_id: Option<String>,
    pub status: E2eRunStatus,
    pub started_at: Option<String>,
    pub duration_ms: i64,
    pub runner: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub error: Option<String>,
    pub model_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub results: Vec<ParsedResult>,
}

impl ParsedRun {
    /// Finds the issue keys each test names: in its title (describe titles included) and
    /// annotations as written (`WEB-12`), in its tags in any case (`@web-12`).
    pub fn find_issue_keys(&mut self, projects: &HashSet<String>) {
        self.results.par_iter_mut().for_each(|result| {
            let mut keys = extract_issue_keys(&result.title, projects, KeyCase::Exact);
            let more = result
                .tags
                .iter()
                .flat_map(|tag| extract_issue_keys(tag, projects, KeyCase::Any))
                .chain(result.notes.iter().flat_map(|note| extract_issue_keys(note, projects, KeyCase::Exact)));

            for key in more {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }

            result.issue_keys = keys;
        });
    }
}

fn cut(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

fn clean(text: Option<&str>, max_chars: usize) -> Option<String> {
    text.map(str::trim).filter(|text| !text.is_empty()).map(|text| cut(text, max_chars))
}

/// A timestamp as UTC in the format the rest of the database uses; none when it cannot be read.
pub fn normalize_time(text: &str) -> Option<String> {
    let text = text.trim();
    let parsed = DateTime::parse_from_rfc3339(text)
        .map(|at| at.with_timezone(&Utc))
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").map(|at| at.and_utc()))
        .ok()?;

    Some(parsed.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn millis_between(start: Option<&str>, end: Option<&str>) -> Option<i64> {
    let start = DateTime::parse_from_rfc3339(start?).ok()?;
    let end = DateTime::parse_from_rfc3339(end?).ok()?;

    Some((end - start).num_milliseconds().max(0))
}

/// Results that would collide (the same test and variant twice in one file) get a counter.
fn tell_apart(results: &mut [ParsedResult]) {
    let mut seen: HashMap<(String, String, String), u32> = HashMap::new();

    for result in results {
        let count = seen.entry((result.file.clone(), result.title.clone(), result.variant.clone())).or_insert(0);
        *count += 1;

        if *count > 1 {
            result.variant = if result.variant.is_empty() { format!("#{count}") } else { format!("{} #{count}", result.variant) };
        }
    }
}

// ---------- the JSON report ----------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Document {
    schema_version: Option<String>,
    run: Option<DocRun>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocRun {
    id: Option<String>,
    status: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    runner: Option<DocRunner>,
    vcs: Option<DocVcs>,
    results: Vec<DocResult>,
    serial_groups: Vec<DocGroup>,
    errors: Vec<DocError>,
    usage: Option<DocUsage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DocRunner {
    name: Option<String>,
    version: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DocVcs {
    commit: Option<String>,
    branch: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocUsage {
    model_tokens: Option<f64>,
    estimated_cost_usd: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocSource {
    file: Option<String>,
    line: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocSkip {
    cause: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocResult {
    test_id: Option<String>,
    title_path: Vec<String>,
    file: Option<String>,
    source: Option<DocSource>,
    target_id: Option<String>,
    agent: Option<String>,
    repeat: Option<f64>,
    tags: Vec<String>,
    annotations: Option<Value>,
    selected: Option<bool>,
    serial_group_id: Option<String>,
    status: Option<String>,
    skip: Option<DocSkip>,
    attempts: Vec<DocAttempt>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocAttempt {
    status: Option<String>,
    duration_ms: Option<f64>,
    error: Option<DocError>,
    steps: Vec<DocStep>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocGroup {
    id: Option<String>,
    attempts: Vec<DocGroupAttempt>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocGroupAttempt {
    status: Option<String>,
    cut_retry: Option<bool>,
    error: Option<DocError>,
    members: Vec<DocMember>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocMember {
    test_id: Option<String>,
    status: Option<String>,
    duration_ms: Option<f64>,
    error: Option<DocError>,
    steps: Vec<DocStep>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocError {
    code: Option<String>,
    message: Option<String>,
    phase: Option<String>,
    source: Option<DocSource>,
    details: Option<DocDetails>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocDetails {
    locator: Option<String>,
    expected: Option<String>,
    observed: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocStep {
    kind: Option<String>,
    metrics: Option<DocMetrics>,
    model: Option<DocModel>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocMetrics {
    model_calls: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DocModel {
    calls: Option<f64>,
    input_tokens: Option<f64>,
    output_tokens: Option<f64>,
    estimated_cost_usd: Option<f64>,
}

/// One attempt of a test, wherever the report keeps it: on the result, or, for a member of a
/// serial group, on the group's attempts.
struct AttemptView<'a> {
    status: &'a str,
    duration_ms: i64,
    /// A retry that was interrupted or never reached this test; it is not the verdict.
    cut: bool,
    error: Option<&'a DocError>,
    steps: &'a [DocStep],
}

fn whole(number: Option<f64>) -> i64 {
    number.filter(|number| number.is_finite()).map(|number| number.round() as i64).unwrap_or(0)
}

fn attempt_views<'a>(result: &'a DocResult, groups: &HashMap<&str, &'a DocGroup>) -> Vec<AttemptView<'a>> {
    let Some(group_id) = result.serial_group_id.as_deref() else {
        return result
            .attempts
            .iter()
            .map(|attempt| AttemptView {
                status: attempt.status.as_deref().unwrap_or(""),
                duration_ms: whole(attempt.duration_ms),
                cut: false,
                error: attempt.error.as_ref(),
                steps: &attempt.steps,
            })
            .collect();
    };
    let Some(group) = groups.get(group_id) else { return Vec::new() };

    group
        .attempts
        .iter()
        .map(|attempt| {
            let member = attempt.members.iter().find(|member| member.test_id.is_some() && member.test_id == result.test_id);

            AttemptView {
                status: member.and_then(|member| member.status.as_deref()).or(attempt.status.as_deref()).unwrap_or(""),
                duration_ms: whole(member.and_then(|member| member.duration_ms)),
                cut: attempt.cut_retry.unwrap_or(false),
                error: member.and_then(|member| member.error.as_ref()).or(attempt.error.as_ref()),
                steps: member.map(|member| member.steps.as_slice()).unwrap_or(&[]),
            }
        })
        .collect()
}

/// Whether an attempt ended the way the result did; a failure and a timeout are one verdict.
fn reached_verdict(result: &str, attempt: &str) -> bool {
    if matches!(result, "failed" | "timed-out") {
        matches!(attempt, "failed" | "timed-out")
    } else {
        attempt == result
    }
}

fn error_text(error: &DocError) -> Option<String> {
    let mut text = error.message.as_deref().map(str::trim).unwrap_or("").to_string();

    if let Some(details) = &error.details {
        for (label, value) in [("locator", &details.locator), ("expected", &details.expected), ("observed", &details.observed)] {
            if let Some(value) = value.as_deref().filter(|value| !value.is_empty() && !text.contains(*value)) {
                text.push_str(&format!("\n{label}: {value}"));
            }
        }
    }

    if text.is_empty() {
        text = error.code.clone().unwrap_or_default();
    }

    clean(Some(&text), MAX_MESSAGE_CHARS)
}

fn run_error_text(error: &DocError) -> String {
    let code = error.code.as_deref().unwrap_or("ERROR");
    let phase = error.phase.as_deref().map(|phase| format!(" ({phase})")).unwrap_or_default();

    cut(&format!("{code}{phase}: {}", error.message.as_deref().unwrap_or("").trim()), MAX_MESSAGE_CHARS)
}

/// Every string inside an `annotations` value, whatever its shape.
fn annotation_texts(value: &Value, texts: &mut Vec<String>) {
    match value {
        Value::String(text) => texts.push(cut(text, MAX_TITLE_CHARS)),
        Value::Array(items) => items.iter().for_each(|item| annotation_texts(item, texts)),
        Value::Object(fields) => fields.values().for_each(|field| annotation_texts(field, texts)),
        _ => {}
    }
}

struct Variants {
    many_targets: bool,
}

fn variant(result: &DocResult, variants: &Variants) -> String {
    let mut parts = Vec::new();

    if variants.many_targets {
        if let Some(target) = result.target_id.as_deref().filter(|target| !target.is_empty()) {
            parts.push(target.to_string());
        }
    }

    if let Some(agent) = result.agent.as_deref().filter(|agent| !agent.is_empty() && *agent != "default") {
        parts.push(agent.to_string());
    }

    let repeat = whole(result.repeat);

    if repeat > 0 {
        parts.push(format!("repeat #{repeat}"));
    }

    parts.join(" · ")
}

/// One result of the report; none for a test the run's selection or a test filter left out,
/// which says nothing new about the test.
fn read_result(result: &DocResult, groups: &HashMap<&str, &DocGroup>, variants: &Variants) -> Option<ParsedResult> {
    let skip_cause = result.skip.as_ref().and_then(|skip| skip.cause.as_deref());
    let raw_status = result.status.as_deref().unwrap_or("");

    if result.selected == Some(false) || (raw_status == "skipped" && skip_cause == Some("filtered")) {
        return None;
    }

    let title = result.title_path.iter().map(|part| part.trim()).filter(|part| !part.is_empty()).collect::<Vec<_>>().join(TITLE_SEPARATOR);
    let file = result.file.as_deref().map(str::trim).filter(|file| !file.is_empty())?;

    if title.is_empty() {
        return None;
    }

    let views = attempt_views(result, groups);
    let last = views.last();
    let verdict = views.iter().rev().find(|view| !view.cut && reached_verdict(raw_status, view.status)).or(last);
    let earlier_failure = views.iter().rev().skip(1).find(|view| view.status != "passed");
    let status = match raw_status {
        "passed" => E2eStatus::Passed,
        "flaky" => E2eStatus::Flaky,
        "failed" | "timed-out" => E2eStatus::Failed,
        _ => E2eStatus::Skipped,
    };
    // What a reader is told: the failure a flaky test hit before it passed, else the verdict's.
    let told = if status == E2eStatus::Flaky { earlier_failure.or(last) } else { verdict };
    let error = told.and_then(|view| view.error).filter(|_| status != E2eStatus::Passed);
    let message = match (status, error) {
        (E2eStatus::Skipped, _) if raw_status == "skipped" => clean(result.skip.as_ref().and_then(|skip| skip.reason.as_deref()), MAX_MESSAGE_CHARS),
        (E2eStatus::Skipped, error) => {
            let why = error.and_then(error_text).unwrap_or_else(|| "the run was stopped".to_string());

            Some(cut(&format!("{raw_status}: {why}"), MAX_MESSAGE_CHARS))
        }
        (_, Some(error)) => error_text(error),
        (E2eStatus::Failed, None) if raw_status == "timed-out" => Some("timed out".to_string()),
        _ => None,
    };
    let declared_line = result.source.as_ref().and_then(|source| source.line).map(|line| whole(Some(line))).filter(|line| *line > 0);
    let failed_line = error
        .and_then(|error| error.source.as_ref())
        .filter(|source| source.file.as_deref().is_none_or(|source_file| source_file == file))
        .and_then(|source| source.line)
        .map(|line| whole(Some(line)))
        .filter(|line| *line > 0);
    let steps = || views.iter().flat_map(|view| view.steps.iter());
    let costs: Vec<f64> = steps().filter_map(|step| step.model.as_ref()?.estimated_cost_usd).filter(|cost| cost.is_finite() && *cost >= 0.0).collect();
    let mut notes = Vec::new();

    if let Some(annotations) = &result.annotations {
        annotation_texts(annotations, &mut notes);
    }

    Some(ParsedResult {
        file: cut(file, MAX_FILE_CHARS),
        title: cut(&title, MAX_TITLE_CHARS),
        variant: variant(result, variants),
        tags: result.tags.iter().map(|tag| cut(tag.trim(), 100)).filter(|tag| !tag.is_empty()).collect(),
        declared_line,
        line: failed_line.or(declared_line),
        status,
        duration_ms: verdict.map(|view| view.duration_ms).unwrap_or(0),
        attempts: views.len() as i64,
        message,
        error_code: error.and_then(|error| clean(error.code.as_deref(), 100)),
        agent_steps: steps().filter(|step| step.kind.as_deref() == Some("agent")).count() as i64,
        model_calls: steps()
            .map(|step| whole(step.model.as_ref().and_then(|model| model.calls).or(step.metrics.as_ref().and_then(|metrics| metrics.model_calls))))
            .sum(),
        model_tokens: steps().filter_map(|step| step.model.as_ref()).map(|model| whole(model.input_tokens) + whole(model.output_tokens)).sum(),
        cost_usd: if costs.is_empty() { None } else { Some(costs.iter().sum()) },
        notes,
        issue_keys: Vec::new(),
    })
}

/// The framework's JSON report. Fields this module does not use are ignored, so a newer
/// report still reads; one without a `run` is refused.
pub fn parse_json(bytes: &[u8]) -> ApiResult<ParsedRun> {
    let document: Document = serde_json::from_slice(bytes).map_err(|error| ApiError::bad_request(format!("The report is not valid JSON: {error}")))?;
    let run = document.run.ok_or_else(|| {
        ApiError::bad_request("This is not an e2e report: it has no \"run\". Post the document the json reporter prints, or .e2e/report.json.")
    })?;

    if let Some(version) = document.schema_version.as_deref().filter(|version| !version.starts_with("report-")) {
        return Err(ApiError::bad_request(format!("Unknown report schema \"{version}\"; expected report-1")));
    }

    let groups: HashMap<&str, &DocGroup> = run.serial_groups.iter().filter_map(|group| Some((group.id.as_deref()?, group))).collect();
    let targets: HashSet<&str> = run.results.iter().filter(|result| result.selected != Some(false)).filter_map(|result| result.target_id.as_deref()).collect();
    let variants = Variants { many_targets: targets.len() > 1 };
    let mut results: Vec<ParsedResult> = run.results.par_iter().filter_map(|result| read_result(result, &groups, &variants)).collect();
    tell_apart(&mut results);

    let status = run.status.as_deref().and_then(E2eRunStatus::parse).unwrap_or_else(|| {
        if results.iter().any(|result| result.status == E2eStatus::Failed) {
            E2eRunStatus::Failed
        } else {
            E2eRunStatus::Passed
        }
    });
    let duration_ms = millis_between(run.started_at.as_deref(), run.finished_at.as_deref()).unwrap_or_else(|| results.iter().map(|result| result.duration_ms).sum());
    let runner = run.runner.as_ref().and_then(|runner| {
        let name = runner.name.as_deref().unwrap_or("e2e");

        Some(match runner.version.as_deref() {
            Some(version) => format!("{name} {version}"),
            None => name.to_string(),
        })
    });

    Ok(ParsedRun {
        format: "json",
        external_id: clean(run.id.as_deref(), 200),
        status,
        started_at: run.started_at.as_deref().and_then(normalize_time),
        duration_ms,
        runner: runner.map(|runner| cut(&runner, 100)),
        branch: run.vcs.as_ref().and_then(|vcs| clean(vcs.branch.as_deref(), 200)),
        commit: run.vcs.as_ref().and_then(|vcs| clean(vcs.commit.as_deref(), 100)),
        error: run.errors.first().map(run_error_text),
        model_tokens: run.usage.as_ref().and_then(|usage| usage.model_tokens).map(|tokens| whole(Some(tokens))),
        cost_usd: run.usage.as_ref().and_then(|usage| usage.estimated_cost_usd).filter(|cost| cost.is_finite() && *cost >= 0.0),
        results,
    })
}

// ---------- JUnit XML ----------

struct JunitName {
    title: String,
    target: Option<String>,
    agent: Option<String>,
    repeat: Option<String>,
}

/// Takes apart a case name as the framework's `junit` reporter writes it:
/// `describe > test (repeat #2) [target] [agent]`, the last two parts optional.
fn split_junit_name(name: &str) -> JunitName {
    let mut rest = name.trim();
    let mut brackets: Vec<&str> = Vec::new();

    while brackets.len() < 2 && rest.ends_with(']') {
        let Some(open) = rest.rfind(" [") else { break };
        brackets.push(&rest[open + 2..rest.len() - 1]);
        rest = &rest[..open];
    }

    // Read from the end: the agent, when there is one, comes after the target.
    let (target, agent) = match brackets.as_slice() {
        [target] => (Some(target.to_string()), None),
        [agent, target] => (Some(target.to_string()), Some(agent.to_string())),
        _ => (None, None),
    };
    let mut repeat = None;

    if rest.ends_with(')') {
        if let Some(open) = rest.rfind(" (repeat #") {
            repeat = Some(rest[open + 2..rest.len() - 1].to_string());
            rest = &rest[..open];
        }
    }

    JunitName { title: rest.trim().to_string(), target, agent, repeat }
}

fn seconds_to_millis(text: Option<&str>) -> i64 {
    text.and_then(|text| text.trim().parse::<f64>().ok()).filter(|seconds| seconds.is_finite() && *seconds >= 0.0).map(|seconds| (seconds * 1000.0).round() as i64).unwrap_or(0)
}

fn element_text(node: roxmltree::Node) -> String {
    node.descendants().filter(|child| child.is_text()).filter_map(|child| child.text()).collect::<String>()
}

struct JunitCase {
    name: JunitName,
    file: String,
    line: Option<i64>,
    status: E2eStatus,
    duration_ms: i64,
    attempts: i64,
    message: Option<String>,
    error_code: Option<String>,
}

/// One `<testcase>`; none for a test a filter left out.
fn read_case(case: roxmltree::Node, suite_name: &str, framework: bool) -> Option<JunitCase> {
    let raw_name = case.attribute("name").unwrap_or("").trim();

    if raw_name.is_empty() {
        return None;
    }

    let name = if framework { split_junit_name(raw_name) } else { JunitName { title: raw_name.to_string(), target: None, agent: None, repeat: None } };
    let file = [case.attribute("file"), case.attribute("classname"), Some(suite_name)].into_iter().flatten().map(str::trim).find(|text| !text.is_empty()).unwrap_or("unknown");
    let child = |tag: &str| case.children().find(|node| node.has_tag_name(tag));
    let output = child("system-out").map(element_text).unwrap_or_default();
    let mut attempts = 1;
    let (status, message, error_code) = if let Some(failure) = child("failure").or_else(|| child("error")) {
        let body = element_text(failure);
        let message = failure.attribute("message").map(str::trim).filter(|message| !message.is_empty()).map(String::from).unwrap_or(body);

        (E2eStatus::Failed, clean(Some(&message), MAX_MESSAGE_CHARS), clean(failure.attribute("type"), 100))
    } else if let Some(skipped) = child("skipped") {
        let message = skipped.attribute("message").map(String::from).unwrap_or_else(|| element_text(skipped));

        if message.trim_start().starts_with("left out by filter") {
            return None;
        }

        attempts = 0;
        (E2eStatus::Skipped, clean(Some(&message), MAX_MESSAGE_CHARS), None)
    } else if let Some(note) = output.lines().find_map(|line| line.trim().strip_prefix("flaky: ")) {
        // "flaky: 2 failed attempts before passing"
        let failed: i64 = note.split_whitespace().next().and_then(|count| count.parse().ok()).unwrap_or(1);
        attempts = failed + 1;

        (E2eStatus::Flaky, Some(format!("flaky: {note}")), None)
    } else {
        (E2eStatus::Passed, None, None)
    };

    Some(JunitCase {
        name,
        file: cut(file, MAX_FILE_CHARS),
        line: case.attribute("line").and_then(|line| line.trim().parse().ok()).filter(|line| *line > 0),
        status,
        duration_ms: seconds_to_millis(case.attribute("time")),
        attempts,
        message,
        error_code,
    })
}

/// JUnit XML: the file the framework's `junit` reporter writes, or any other JUnit file.
pub fn parse_junit(bytes: &[u8]) -> ApiResult<ParsedRun> {
    let text = std::str::from_utf8(bytes).map_err(|_| ApiError::bad_request("The JUnit file is not UTF-8 text"))?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| ApiError::bad_request(format!("The JUnit file is not valid XML: {error}")))?;
    let root = document.root_element();

    if !root.has_tag_name("testsuites") && !root.has_tag_name("testsuite") {
        return Err(ApiError::bad_request("This is not a JUnit file: its root is neither <testsuites> nor <testsuite>"));
    }

    // The framework names its root `e2e` and its cases `title [target]`; other files are read plainly.
    let framework = root.has_tag_name("testsuites") && root.attribute("name") == Some("e2e");
    let mut cases = Vec::new();
    let mut run_error = None;

    for suite in root.descendants().filter(|node| node.has_tag_name("testsuite")) {
        let suite_name = suite.attribute("name").unwrap_or("");

        for case in suite.children().filter(|node| node.has_tag_name("testcase")) {
            // A run-level error (the app never came up) is a case of the `run` suite, not a test.
            if framework && suite_name == "run" && case.attribute("classname") == Some("run") {
                if run_error.is_none() {
                    let error = case.children().find(|node| node.has_tag_name("error") || node.has_tag_name("failure"));
                    let message = error.and_then(|error| error.attribute("message")).unwrap_or("");

                    run_error = Some(cut(&format!("{}: {message}", case.attribute("name").unwrap_or("ERROR")), MAX_MESSAGE_CHARS));
                }

                continue;
            }

            cases.extend(read_case(case, suite_name, framework));
        }
    }

    let targets: HashSet<&str> = cases.iter().filter_map(|case| case.name.target.as_deref()).collect();
    let many_targets = targets.len() > 1;
    let mut results: Vec<ParsedResult> = cases
        .iter()
        .map(|case| {
            let mut parts = Vec::new();

            if many_targets {
                parts.extend(case.name.target.clone());
            }

            parts.extend(case.name.agent.clone());
            parts.extend(case.name.repeat.clone());

            ParsedResult {
                file: case.file.clone(),
                title: cut(&case.name.title, MAX_TITLE_CHARS),
                variant: parts.join(" · "),
                tags: Vec::new(),
                declared_line: case.line,
                line: case.line,
                status: case.status,
                duration_ms: case.duration_ms,
                attempts: case.attempts,
                message: case.message.clone(),
                error_code: case.error_code.clone(),
                agent_steps: 0,
                model_calls: 0,
                model_tokens: 0,
                cost_usd: None,
                notes: Vec::new(),
                issue_keys: Vec::new(),
            }
        })
        .collect();
    tell_apart(&mut results);

    let failed = results.iter().any(|result| result.status == E2eStatus::Failed);
    let timestamp = root.attribute("timestamp").or_else(|| root.descendants().find_map(|node| node.attribute("timestamp")));

    Ok(ParsedRun {
        format: "junit",
        external_id: None,
        status: match (&run_error, failed) {
            (Some(_), _) => E2eRunStatus::Error,
            (None, true) => E2eRunStatus::Failed,
            (None, false) => E2eRunStatus::Passed,
        },
        started_at: timestamp.and_then(normalize_time),
        duration_ms: if root.attribute("time").is_some() { seconds_to_millis(root.attribute("time")) } else { results.iter().map(|result| result.duration_ms).sum() },
        runner: None,
        branch: None,
        commit: None,
        error: run_error,
        model_tokens: None,
        cost_usd: None,
        results,
    })
}

/// Reads a posted report: XML when the content type says so or the body starts like XML, else JSON.
pub fn parse(bytes: &[u8], content_type: Option<&str>) -> ApiResult<ParsedRun> {
    let declared_xml = content_type.is_some_and(|content_type| {
        let media = content_type.split(';').next().unwrap_or("").trim().to_lowercase();

        media == "application/xml" || media == "text/xml" || media.ends_with("+xml")
    });
    let looks_like_xml = bytes.iter().find(|byte| !byte.is_ascii_whitespace() && ![0xef, 0xbb, 0xbf].contains(*byte)) == Some(&b'<');

    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(ApiError::bad_request("The request has no body; post the run report"));
    }

    if declared_xml || looks_like_xml {
        parse_junit(bytes)
    } else {
        parse_json(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/report-v1.valid.json");

    fn projects() -> HashSet<String> {
        ["WEB", "API"].into_iter().map(String::from).collect()
    }

    fn find<'a>(run: &'a ParsedRun, title: &str) -> &'a ParsedResult {
        run.results.iter().find(|result| result.title == title).unwrap_or_else(|| panic!("no result titled {title}"))
    }

    #[test]
    fn reads_the_frameworks_own_report_fixture() {
        let run = parse(FIXTURE, Some("application/json")).unwrap();

        assert_eq!(run.format, "json");
        assert_eq!(run.external_id.as_deref(), Some("018f0f5c-7b2a-7abc-8abc-0123456789ab"));
        assert_eq!(run.status, E2eRunStatus::Failed);
        assert_eq!(run.runner.as_deref().map(|runner| runner.starts_with("e2e ")), Some(true));
        // Four results in the file; the one a test filter left out is not this run's news.
        assert_eq!(run.results.len(), 3);

        let passed = find(&run, "opens app");
        assert_eq!((passed.status, passed.attempts, passed.duration_ms), (E2eStatus::Passed, 1, 1000));
        assert_eq!(passed.file, "tests/example.e2e.ts");
        assert_eq!(passed.tags, vec!["smoke"]);
        assert_eq!((passed.agent_steps, passed.model_calls), (2, 1));
        assert_eq!(passed.message, None);
        assert_eq!(passed.line, Some(3));

        let skipped = find(&run, "switches organizations");
        assert_eq!(skipped.status, E2eStatus::Skipped);
        assert_eq!(skipped.message.as_deref(), Some("the demo tenant has a single organization"));

        // A member of a serial group keeps its attempts on the group; the cut retry is not the verdict.
        let serial = find(&run, "checkout > retains serial verdict");
        assert_eq!((serial.status, serial.attempts, serial.duration_ms), (E2eStatus::Failed, 2, 1250));
    }

    fn report(results: Value) -> Vec<u8> {
        serde_json::json!({
            "schemaVersion": "report-1",
            "run": {
                "id": "run-1",
                "status": "failed",
                "startedAt": "2026-11-12T10:00:00+01:00",
                "finishedAt": "2026-11-12T10:02:30+01:00",
                "runner": { "name": "e2e", "version": "0.23.0" },
                "vcs": { "commit": "0123456789abcdef0123456789abcdef01234567", "branch": "main" },
                "usage": { "modelTokens": 1200, "estimatedCostUsd": 0.0125 },
                "serialGroups": [],
                "errors": [],
                "results": results,
            }
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_failure_tells_its_message_line_and_cost() {
        let bytes = report(serde_json::json!([{
            "testId": "tests/cart.e2e.ts::Cart::WEB-11 empty cart",
            "titlePath": ["Cart", "WEB-11 empty cart shows a message"],
            "file": "tests/cart.e2e.ts",
            "source": { "file": "tests/cart.e2e.ts", "line": 12, "column": 1 },
            "targetId": "chromium", "agent": "default", "repeat": 0, "tags": ["@web-4", "smoke"], "selected": true,
            "annotations": [{ "type": "issue", "description": "API-7" }],
            "status": "failed",
            "attempts": [
                { "index": 0, "status": "failed", "durationMs": 900, "steps": [], "error": { "code": "ASSERTION_FAILED", "message": "first try" } },
                { "index": 1, "status": "timed-out", "durationMs": 2100.4,
                  "error": { "category": "test", "code": "ASSERTION_FAILED", "message": "expect.toHaveText failed",
                             "details": { "locator": "getByRole(\"status\")", "expected": "text \"Koszyk jest pusty\"", "observed": "text \"500\"" },
                             "source": { "file": "tests/cart.e2e.ts", "line": 19, "column": 3 } },
                  "steps": [
                    { "kind": "agent", "metrics": { "modelCalls": 3, "actionSteps": 4 }, "model": { "calls": 3, "inputTokens": 900, "outputTokens": 100, "estimatedCostUsd": 0.01 } },
                    { "kind": "assertion" },
                    { "kind": "agent", "metrics": { "modelCalls": 1 }, "model": { "calls": 1, "inputTokens": 150, "outputTokens": 50, "estimatedCostUsd": 0.0025 } }
                  ] }
            ]
        }]));
        let mut run = parse_json(&bytes).unwrap();
        run.find_issue_keys(&projects());
        let result = &run.results[0];

        assert_eq!(run.started_at.as_deref(), Some("2026-11-12T09:00:00.000Z"));
        assert_eq!(run.duration_ms, 150_000);
        assert_eq!((run.branch.as_deref(), run.model_tokens, run.cost_usd), (Some("main"), Some(1200), Some(0.0125)));
        assert_eq!(result.title, "Cart > WEB-11 empty cart shows a message");
        assert_eq!(result.variant, "");
        assert_eq!((result.status, result.attempts, result.duration_ms), (E2eStatus::Failed, 2, 2100));
        assert_eq!(result.message.as_deref(), Some("expect.toHaveText failed\nlocator: getByRole(\"status\")\nexpected: text \"Koszyk jest pusty\"\nobserved: text \"500\""));
        assert_eq!(result.error_code.as_deref(), Some("ASSERTION_FAILED"));
        assert_eq!((result.declared_line, result.line), (Some(12), Some(19)));
        assert_eq!((result.agent_steps, result.model_calls, result.model_tokens), (2, 4, 1200));
        assert!((result.cost_usd.unwrap() - 0.0125).abs() < 1e-9);
        // The title as written, a tag in any case, an annotation.
        assert_eq!(result.issue_keys, vec!["WEB-11", "WEB-4", "API-7"]);
    }

    #[test]
    fn a_flaky_test_tells_the_failure_it_hit_and_targets_are_told_apart() {
        let attempt = |status: &str, message: Option<&str>| serde_json::json!({ "status": status, "durationMs": 500, "steps": [], "error": message.map(|message| serde_json::json!({ "code": "LOCATOR_NOT_FOUND", "message": message })) });
        let result = |target: &str, status: &str, attempts: Value| serde_json::json!({ "titlePath": ["WEB-14 reset"], "file": "tests/a.e2e.ts", "targetId": target, "agent": "default", "status": status, "attempts": attempts });
        let bytes = report(serde_json::json!([
            result("chromium", "flaky", serde_json::json!([attempt("failed", Some("no button")), attempt("passed", None)])),
            result("firefox", "interrupted", serde_json::json!([attempt("interrupted", Some("SIGINT"))])),
            { "titlePath": ["not chosen"], "file": "tests/b.e2e.ts", "targetId": "chromium", "status": "skipped", "selected": false, "skip": { "cause": "filtered", "reason": "another file" }, "attempts": [] },
        ]));
        let run = parse_json(&bytes).unwrap();

        assert_eq!(run.results.len(), 2);
        assert_eq!((run.results[0].status, run.results[0].variant.as_str(), run.results[0].attempts), (E2eStatus::Flaky, "chromium", 2));
        assert_eq!(run.results[0].message.as_deref(), Some("no button"));
        assert_eq!((run.results[1].status, run.results[1].variant.as_str()), (E2eStatus::Skipped, "firefox"));
        assert_eq!(run.results[1].message.as_deref(), Some("interrupted: SIGINT"));
    }

    #[test]
    fn refuses_what_is_not_a_report() {
        assert_eq!(parse(b"  ", None).unwrap_err().status.as_u16(), 400);
        assert!(parse(b"{\"tests\": []}", None).unwrap_err().message.contains("no \"run\""));
        assert!(parse(b"{\"schemaVersion\": \"other\", \"run\": {}}", None).unwrap_err().message.contains("Unknown report schema"));
        assert!(parse(b"{ nope", None).unwrap_err().message.contains("not valid JSON"));
        assert!(parse(b"<html></html>", None).unwrap_err().message.contains("not a JUnit file"));
        assert!(parse(b"<testsuites><oops></testsuites>", Some("application/xml")).unwrap_err().message.contains("not valid XML"));
        // No DTD, so no entity expansion.
        assert!(parse(b"<!DOCTYPE x [<!ENTITY a \"aaaa\">]><testsuites>&a;</testsuites>", Some("text/xml")).is_err());
    }

    /// As `renderJunitReport` of the framework writes it.
    const JUNIT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="e2e" tests="6" failures="1" errors="1" skipped="2" time="12.500">
  <testsuite name="tests/koszyk.e2e.ts" tests="5" failures="1" errors="0" skipped="2" time="12.500">
    <testcase name="Koszyk &gt; WEB-4 klient dodaje produkt do koszyka [chromium]" classname="tests/koszyk.e2e.ts" time="3.250"/>
    <testcase name="Koszyk &gt; WEB-11 pusty koszyk pokazuje komunikat [chromium]" classname="tests/koszyk.e2e.ts" time="5.000">
      <failure message="expect.toHaveText failed" type="ASSERTION_FAILED">ASSERTION_FAILED (body): expect.toHaveText failed</failure>
      <system-out>[[ATTACHMENT|.e2e/results/x/failure.png]]</system-out>
    </testcase>
    <testcase name="Konto &gt; WEB-14 reset hasła (repeat #1) [chromium] [fast]" classname="tests/koszyk.e2e.ts" time="4.250">
      <system-out>flaky: 2 failed attempts before passing</system-out>
    </testcase>
    <testcase name="WEB-10 płatność BLIK [chromium]" classname="tests/koszyk.e2e.ts" time="0.000">
      <skipped message="BLIK w sandboxie jeszcze niedostępny"/>
    </testcase>
    <testcase name="poza wyborem [chromium]" classname="tests/koszyk.e2e.ts" time="0.000">
      <skipped message="left out by filter areas: the diff reaches none of its tags"/>
    </testcase>
  </testsuite>
  <testsuite name="run" tests="1" failures="0" errors="1" skipped="0" time="0.000">
    <testcase name="APP_UNREACHABLE (launch)" classname="run" time="0.000">
      <error message="http://localhost:3000 did not answer" type="APP_UNREACHABLE">infrastructure error APP_UNREACHABLE (launch): http://localhost:3000 did not answer</error>
    </testcase>
  </testsuite>
</testsuites>
"#;

    #[test]
    fn reads_the_frameworks_junit_file() {
        let mut run = parse(JUNIT.as_bytes(), Some("application/xml; charset=utf-8")).unwrap();
        run.find_issue_keys(&projects());

        assert_eq!((run.format, run.status, run.duration_ms), ("junit", E2eRunStatus::Error, 12_500));
        assert_eq!(run.error.as_deref(), Some("APP_UNREACHABLE (launch): http://localhost:3000 did not answer"));
        assert_eq!(run.results.len(), 4);

        let passed = find(&run, "Koszyk > WEB-4 klient dodaje produkt do koszyka");
        assert_eq!((passed.status, passed.duration_ms, passed.variant.as_str()), (E2eStatus::Passed, 3250, ""));
        assert_eq!(passed.issue_keys, vec!["WEB-4"]);

        let failed = find(&run, "Koszyk > WEB-11 pusty koszyk pokazuje komunikat");
        assert_eq!((failed.status, failed.message.as_deref(), failed.error_code.as_deref()), (E2eStatus::Failed, Some("expect.toHaveText failed"), Some("ASSERTION_FAILED")));

        let flaky = find(&run, "Konto > WEB-14 reset hasła");
        assert_eq!((flaky.status, flaky.attempts, flaky.variant.as_str()), (E2eStatus::Flaky, 3, "fast · repeat #1"));

        let skipped = find(&run, "WEB-10 płatność BLIK");
        assert_eq!((skipped.status, skipped.message.as_deref()), (E2eStatus::Skipped, Some("BLIK w sandboxie jeszcze niedostępny")));
    }

    #[test]
    fn reads_a_plain_junit_file_too() {
        let xml = r#"<testsuite name="pytest" timestamp="2026-11-12T08:00:00" time="1.5">
            <testcase classname="tests.test_cart" name="test_total [eur]" time="0.5" file="tests/test_cart.py" line="14"/>
            <testcase classname="tests.test_cart" name="test_total [eur]" time="1.0"><error type="ValueError">boom
  at line 3</error></testcase>
        </testsuite>"#;
        let run = parse(xml.as_bytes(), None).unwrap();

        assert_eq!((run.status, run.started_at.as_deref(), run.duration_ms), (E2eRunStatus::Failed, Some("2026-11-12T08:00:00.000Z"), 1500));
        assert_eq!((run.results[0].title.as_str(), run.results[0].file.as_str(), run.results[0].line), ("test_total [eur]", "tests/test_cart.py", Some(14)));
        assert_eq!((run.results[1].file.as_str(), run.results[1].message.as_deref()), ("tests.test_cart", Some("boom\n  at line 3")));
    }

    #[test]
    fn the_same_test_twice_is_told_apart() {
        let xml = r#"<testsuite name="s"><testcase classname="a" name="t"/><testcase classname="a" name="t"/></testsuite>"#;
        let run = parse_junit(xml.as_bytes()).unwrap();

        assert_eq!(run.results.iter().map(|result| result.variant.as_str()).collect::<Vec<_>>(), vec!["", "#2"]);
    }
}
