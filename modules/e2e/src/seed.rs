//! Example runs for a demo instance: a week of CI for the WEB project, written as the
//! framework's own JSON report and fed through the same code that takes posted reports.

use chrono::{Duration, SecondsFormat, Utc};
use hub_app::db::query_one;
use hub_app::{Actor, ApiResult, App};
use hub_core::Id;
use serde_json::{json, Value};

use crate::ingest::{project_keys, store_run, RunMeta};
use crate::report::parse_json;

struct Run {
    day: i64,
    hour: i64,
    number: i64,
    branch: &'static str,
    pull: Option<i64>,
}

const RUNS: &[Run] = &[
    Run { day: -5, hour: 9, number: 212, branch: "main", pull: None },
    Run { day: -4, hour: 10, number: 215, branch: "main", pull: None },
    Run { day: -3, hour: 14, number: 219, branch: "fix/WEB-11-pusty-koszyk", pull: Some(53) },
    Run { day: -2, hour: 9, number: 224, branch: "main", pull: None },
    Run { day: -2, hour: 15, number: 226, branch: "feature/WEB-9-platnosc-karta", pull: Some(52) },
    Run { day: -1, hour: 10, number: 231, branch: "main", pull: None },
    Run { day: -1, hour: 16, number: 233, branch: "feature/WEB-9-platnosc-karta", pull: Some(52) },
    Run { day: 0, hour: 2, number: 236, branch: "main", pull: None },
];

struct Failure {
    code: &'static str,
    message: &'static str,
    locator: Option<&'static str>,
    expected: Option<&'static str>,
    observed: Option<&'static str>,
    line: i64,
}

struct Case {
    file: &'static str,
    group: &'static str,
    title: &'static str,
    line: i64,
    tags: &'static [&'static str],
    /// One letter per run: p passed, f failed, k flaky (failed, then passed on the retry),
    /// s skipped, - not in the suite of that run.
    outcomes: &'static str,
    failure: Option<Failure>,
    skip: Option<&'static str>,
    /// Steps an agent carries out in this test.
    agent_steps: i64,
    seconds: f64,
}

const BASE: Case =
    Case { file: "", group: "", title: "", line: 1, tags: &[], outcomes: "pppppppp", failure: None, skip: None, agent_steps: 0, seconds: 3.0 };

const CASES: &[Case] = &[
    Case { file: "tests/koszyk.e2e.ts", group: "Koszyk", title: "WEB-4 klient dodaje produkt do koszyka", line: 8, seconds: 4.2, ..BASE },
    Case { file: "tests/koszyk.e2e.ts", group: "Koszyk", title: "WEB-4 klient usuwa produkt z koszyka", line: 21, seconds: 3.6, ..BASE },
    Case {
        file: "tests/koszyk.e2e.ts",
        group: "Koszyk",
        title: "WEB-6 suma koszyka uwzględnia rabat pozycji",
        line: 34,
        tags: &["regresja"],
        seconds: 3.1,
        ..BASE
    },
    Case {
        file: "tests/koszyk.e2e.ts",
        group: "Koszyk",
        title: "WEB-11 pusty koszyk pokazuje komunikat zamiast błędu 500",
        line: 47,
        tags: &["regresja"],
        outcomes: "--pppfpf",
        failure: Some(Failure {
            code: "ASSERTION_FAILED",
            message: "expect.toHaveText failed",
            locator: Some("getByRole(\"status\")"),
            expected: Some("text \"Twój koszyk jest pusty\""),
            observed: Some("text \"Błąd 500: nie udało się wczytać podsumowania\""),
            line: 53,
        }),
        seconds: 2.4,
        ..BASE
    },
    Case { file: "tests/konto.e2e.ts", group: "Konto klienta", title: "WEB-8 rejestracja nowego klienta e-mailem", line: 9, seconds: 5.8, ..BASE },
    Case {
        file: "tests/konto.e2e.ts",
        group: "Konto klienta",
        title: "WEB-8 logowanie poprawnym hasłem",
        line: 24,
        tags: &["smoke"],
        seconds: 2.9,
        ..BASE
    },
    Case {
        file: "tests/konto.e2e.ts",
        group: "Konto klienta",
        title: "WEB-14 reset hasła wysyła link i ustawia nowe hasło",
        line: 37,
        outcomes: "-ppkpppk",
        failure: Some(Failure {
            code: "LOCATOR_NOT_FOUND",
            message: "No node matches getByRole(\"link\", \"Ustaw nowe hasło\") within 5000 ms: the reset e-mail had not arrived yet",
            locator: Some("getByRole(\"link\", \"Ustaw nowe hasło\")"),
            expected: None,
            observed: None,
            line: 44,
        }),
        seconds: 7.5,
        ..BASE
    },
    Case {
        file: "tests/platnosci.e2e.ts",
        group: "Płatności",
        title: "WEB-9 płatność kartą testową przechodzi",
        line: 11,
        tags: &["płatności"],
        outcomes: "----p-p-",
        seconds: 9.4,
        ..BASE
    },
    Case {
        file: "tests/platnosci.e2e.ts",
        group: "Płatności",
        title: "WEB-9 odrzucona karta pokazuje czytelny komunikat",
        line: 29,
        tags: &["płatności"],
        outcomes: "----f-p-",
        failure: Some(Failure {
            code: "ASSERTION_FAILED",
            message: "expect.toBeVisible failed",
            locator: Some("getByText(\"Karta została odrzucona\")"),
            expected: Some("visible"),
            observed: Some("no matching node; the page shows \"Unexpected token < in JSON\""),
            line: 38,
        }),
        seconds: 6.3,
        ..BASE
    },
    Case {
        file: "tests/platnosci.e2e.ts",
        group: "Płatności",
        title: "WEB-10 płatność BLIK kodem testowym",
        line: 46,
        tags: &["płatności"],
        outcomes: "---s-s-s",
        skip: Some("BLIK w sandboxie bramki jest jeszcze niedostępny"),
        seconds: 0.0,
        ..BASE
    },
    Case {
        file: "tests/adres.e2e.ts",
        group: "Adres dostawy",
        title: "formularz wymaga ulicy, miasta i kodu pocztowego",
        line: 7,
        tags: &["@WEB-5"],
        seconds: 3.3,
        ..BASE
    },
    Case {
        file: "tests/adres.e2e.ts",
        group: "Adres dostawy",
        title: "WEB-5 zapisany adres podpowiada się przy kolejnym zamówieniu",
        line: 22,
        seconds: 4.0,
        ..BASE
    },
    Case {
        file: "tests/zakup.e2e.ts",
        group: "Ścieżka zakupowa",
        title: "WEB-17 klient składa zamówienie z płatnością przy odbiorze",
        line: 6,
        tags: &["agent"],
        outcomes: "-ppppppp",
        agent_steps: 4,
        seconds: 38.0,
        ..BASE
    },
    Case {
        file: "tests/smoke.e2e.ts",
        group: "",
        title: "strona główna pokazuje katalog produktów",
        line: 4,
        tags: &["smoke"],
        seconds: 1.8,
        ..BASE
    },
];

/// A run of today happened `hour` hours ago; an earlier one at `hour` o'clock of its day.
fn at(day: i64, hour: i64) -> String {
    let now = Utc::now();
    let instant = match now.date_naive().and_hms_opt(hour.clamp(0, 23) as u32, 7, 0) {
        Some(time) if day < 0 => time.and_utc() + Duration::days(day),
        _ => now - Duration::hours(hour),
    };

    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn sha(seed: &str) -> String {
    hub_app::auth::digest(seed)[..40].to_string()
}

fn error(failure: &Failure, file: &str) -> Value {
    json!({
        "category": "test",
        "code": failure.code,
        "message": failure.message,
        "retryable": false,
        "phase": "body",
        "details": { "locator": failure.locator, "expected": failure.expected, "observed": failure.observed },
        "source": { "file": file, "line": failure.line, "column": 3 },
    })
}

fn steps(case: &Case) -> Vec<Value> {
    (0..case.agent_steps)
        .map(|index| {
            json!({
                "index": index,
                "kind": "agent",
                "api": if index + 1 == case.agent_steps { "agent.assert" } else { "agent.act" },
                "status": "passed",
                "metrics": { "modelCalls": 2 + index % 2, "actionSteps": 3, "observationBytes": 4096, "contextBytes": 2048, "ledgerBytes": 512 },
                "model": { "provider": "gateway", "model": "openai/gpt-6-luna-fast", "calls": 2 + index % 2, "inputTokens": 4200 + index * 310, "outputTokens": 260, "estimatedCostUsd": 0.0041 + index as f64 * 0.0004 },
            })
        })
        .collect()
}

fn attempt(case: &Case, index: i64, status: &str, started: &str) -> Value {
    let failed = status == "failed";

    json!({
        "id": format!("{}-{index}", sha(case.title)),
        "index": index,
        "status": status,
        "startedAt": started,
        "durationMs": (case.seconds * 1000.0 * if failed { 1.4 } else { 1.0 }) as i64 + index * 137,
        "artifacts": [],
        "secondaryErrors": [],
        "cleanup": "complete",
        "error": if failed { case.failure.as_ref().map(|failure| error(failure, case.file)) } else { None },
        "steps": steps(case),
    })
}

/// The report of one run, in the shape `packages/e2e/src/report/build.ts` of the framework writes.
fn report(run: &Run, position: usize) -> Value {
    let started = at(run.day, run.hour);
    let commit = sha(&format!("sklep-web-{}", run.number));
    let mut results = Vec::new();
    let mut total_ms = 0;

    for case in CASES {
        let (status, attempts): (&str, Vec<Value>) = match case.outcomes.as_bytes().get(position) {
            Some(b'p') => ("passed", vec![attempt(case, 0, "passed", &started)]),
            Some(b'f') => ("failed", vec![attempt(case, 0, "failed", &started), attempt(case, 1, "failed", &started)]),
            Some(b'k') => ("flaky", vec![attempt(case, 0, "failed", &started), attempt(case, 1, "passed", &started)]),
            Some(b's') => ("skipped", Vec::new()),
            _ => continue,
        };
        let title_path: Vec<&str> = [case.group, case.title].into_iter().filter(|part| !part.is_empty()).collect();

        total_ms += attempts.iter().filter_map(|attempt| attempt["durationMs"].as_i64()).sum::<i64>();
        results.push(json!({
            "id": sha(&format!("{}{}", case.file, case.title)),
            "testId": format!("{}::{}", case.file, title_path.join("::")),
            "kind": "test",
            "titlePath": title_path,
            "file": case.file,
            "source": { "file": case.file, "line": case.line, "column": 1 },
            "targetId": "chromium",
            "platform": "web",
            "agent": "default",
            "repeat": 0,
            "tags": case.tags,
            "selected": true,
            "status": status,
            "skip": case.skip.filter(|_| status == "skipped").map(|reason| json!({ "cause": "explicit", "reason": reason })),
            "attempts": attempts,
        }));
    }

    let failed = results.iter().any(|result| result["status"] == "failed");
    let finished = (chrono::DateTime::parse_from_rfc3339(&started).map(|at| at.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now())
        + Duration::milliseconds(total_ms + 21_000))
    .to_rfc3339_opts(SecondsFormat::Millis, true);

    json!({
        "schemaVersion": "report-1",
        "run": {
            "id": format!("demo-sklep-web-{}", run.number),
            "specVersion": "0.1",
            "runner": { "name": "e2e", "version": "0.23.0" },
            "status": if failed { "failed" } else { "passed" },
            "exitCode": if failed { 1 } else { 0 },
            "startedAt": started,
            "finishedAt": finished,
            "environment": { "ci": true, "os": "linux", "arch": "x64", "runtime": "node 24.8.0" },
            "vcs": { "commit": commit, "branch": run.branch, "dirty": false },
            "serialGroups": [],
            "errors": [],
            "results": results,
        }
    })
}

pub fn seed_demo(app: &App) -> ApiResult<()> {
    let web: Option<Id> = app.read(|connection| query_one(connection, "SELECT id FROM project WHERE key = 'WEB'", [], |row| row.get(0)))?;
    let Some(web) = web else { return Ok(()) };
    let projects = app.read(project_keys)?;

    for (position, run) in RUNS.iter().enumerate() {
        let document = report(run, position);
        let mut parsed = parse_json(document.to_string().as_bytes())?;
        parsed.find_issue_keys(&projects);

        let meta = RunMeta {
            name: Some(format!("CI #{}", run.number)),
            pr_number: run.pull,
            ci_url: Some(format!("https://github.com/acme/sklep-web/actions/runs/{}", 7_300_000_000 + run.number * 1_117)),
            ..Default::default()
        };

        app.write_at(Actor::system("CI"), document["run"]["finishedAt"].as_str().map(String::from).unwrap_or_else(|| at(run.day, run.hour)), |tx| {
            store_run(tx, None, web, &parsed, meta).map(|_| ())
        })?;
    }

    Ok(())
}
