//! Example runs for a demo instance: a week of CI for the WEB project, written as the
//! framework's own JSON report and fed through the same code that takes posted reports.
//!
//! The suite is written in the language of the demo: files, groups, titles, tags and what
//! a failure saw on the page. Outcomes, durations and ids are the same in both languages.

use chrono::{Duration, SecondsFormat, Utc};
use hub_app::db::query_one;
use hub_app::{Actor, ApiResult, App};
use hub_core::demo::{DemoLanguage, DemoText as Text};
use hub_core::Id;
use serde_json::{json, Value};

use crate::ingest::{project_keys, store_run, RunMeta};
use crate::report::parse_json;

struct Run {
    day: i64,
    hour: i64,
    number: i64,
    /// The branches are the ones the demo's pull requests are on.
    branch: Text,
    pull: Option<i64>,
}

/// The repository the runs are of, as the demo's code hosting names it.
const REPO: Text = Text::new("acme/store-web", "acme/sklep-web");

const RUNS: &[Run] = &[
    Run { day: -5, hour: 9, number: 212, branch: Text::same("main"), pull: None },
    Run { day: -4, hour: 10, number: 215, branch: Text::same("main"), pull: None },
    Run { day: -3, hour: 14, number: 219, branch: Text::new("fix/WEB-11-empty-cart", "fix/WEB-11-pusty-koszyk"), pull: Some(53) },
    Run { day: -2, hour: 9, number: 224, branch: Text::same("main"), pull: None },
    Run { day: -2, hour: 15, number: 226, branch: Text::new("feature/WEB-9-card-payment", "feature/WEB-9-platnosc-karta"), pull: Some(52) },
    Run { day: -1, hour: 10, number: 231, branch: Text::same("main"), pull: None },
    Run { day: -1, hour: 16, number: 233, branch: Text::new("feature/WEB-9-card-payment", "feature/WEB-9-platnosc-karta"), pull: Some(52) },
    Run { day: 0, hour: 2, number: 236, branch: Text::same("main"), pull: None },
];

struct Failure {
    code: &'static str,
    message: &'static str,
    locator: Option<Text>,
    expected: Option<Text>,
    observed: Option<Text>,
    line: i64,
}

struct Case {
    file: Text,
    group: Text,
    title: Text,
    line: i64,
    tags: &'static [Text],
    /// One letter per run: p passed, f failed, k flaky (failed, then passed on the retry),
    /// s skipped, - not in the suite of that run.
    outcomes: &'static str,
    failure: Option<Failure>,
    skip: Option<Text>,
    /// Steps an agent carries out in this test.
    agent_steps: i64,
    seconds: f64,
}

const BASE: Case = Case {
    file: Text::EMPTY,
    group: Text::EMPTY,
    title: Text::EMPTY,
    line: 1,
    tags: &[],
    outcomes: "pppppppp",
    failure: None,
    skip: None,
    agent_steps: 0,
    seconds: 3.0,
};

const CASES: &[Case] = &[
    Case {
        file: Text::new("tests/cart.e2e.ts", "tests/koszyk.e2e.ts"),
        group: Text::new("Cart", "Koszyk"),
        title: Text::new("WEB-4 a customer adds a product to the cart", "WEB-4 klient dodaje produkt do koszyka"),
        line: 8,
        seconds: 4.2,
        ..BASE
    },
    Case {
        file: Text::new("tests/cart.e2e.ts", "tests/koszyk.e2e.ts"),
        group: Text::new("Cart", "Koszyk"),
        title: Text::new("WEB-4 a customer removes a product from the cart", "WEB-4 klient usuwa produkt z koszyka"),
        line: 21,
        seconds: 3.6,
        ..BASE
    },
    Case {
        file: Text::new("tests/cart.e2e.ts", "tests/koszyk.e2e.ts"),
        group: Text::new("Cart", "Koszyk"),
        title: Text::new("WEB-6 the cart total includes the item's discount", "WEB-6 suma koszyka uwzględnia rabat pozycji"),
        line: 34,
        tags: &[Text::new("regression", "regresja")],
        seconds: 3.1,
        ..BASE
    },
    Case {
        file: Text::new("tests/cart.e2e.ts", "tests/koszyk.e2e.ts"),
        group: Text::new("Cart", "Koszyk"),
        title: Text::new("WEB-11 an empty cart shows a message instead of error 500", "WEB-11 pusty koszyk pokazuje komunikat zamiast błędu 500"),
        line: 47,
        tags: &[Text::new("regression", "regresja")],
        outcomes: "--pkpppk",
        failure: Some(Failure {
            code: "ASSERTION_FAILED",
            message: "expect.toHaveText failed",
            locator: Some(Text::same("getByRole(\"status\")")),
            expected: Some(Text::new("text \"Your cart is empty\"", "text \"Twój koszyk jest pusty\"")),
            observed: Some(Text::new("text \"Loading the summary…\"", "text \"Wczytywanie podsumowania…\"")),
            line: 53,
        }),
        seconds: 2.4,
        ..BASE
    },
    Case {
        file: Text::new("tests/account.e2e.ts", "tests/konto.e2e.ts"),
        group: Text::new("Customer account", "Konto klienta"),
        title: Text::new("WEB-8 a new customer signs up by e-mail", "WEB-8 rejestracja nowego klienta e-mailem"),
        line: 9,
        seconds: 5.8,
        ..BASE
    },
    Case {
        file: Text::new("tests/account.e2e.ts", "tests/konto.e2e.ts"),
        group: Text::new("Customer account", "Konto klienta"),
        title: Text::new("WEB-8 sign-in with the correct password", "WEB-8 logowanie poprawnym hasłem"),
        line: 24,
        tags: &[Text::same("smoke")],
        seconds: 2.9,
        ..BASE
    },
    Case {
        file: Text::new("tests/account.e2e.ts", "tests/konto.e2e.ts"),
        group: Text::new("Customer account", "Konto klienta"),
        title: Text::new("WEB-14 a password reset sends a link and sets the new password", "WEB-14 reset hasła wysyła link i ustawia nowe hasło"),
        line: 37,
        outcomes: "-ppppfpf",
        failure: Some(Failure {
            code: "ASSERTION_FAILED",
            message: "expect.toBeVisible failed",
            locator: Some(Text::new("getByText(\"Your password has been changed\")", "getByText(\"Hasło zostało zmienione\")")),
            expected: Some(Text::same("visible")),
            observed: Some(Text::new(
                "no matching node; the page shows \"The link has expired or is not valid\"",
                "no matching node; the page shows \"Link wygasł lub jest nieprawidłowy\"",
            )),
            line: 44,
        }),
        seconds: 7.5,
        ..BASE
    },
    Case {
        file: Text::new("tests/payments.e2e.ts", "tests/platnosci.e2e.ts"),
        group: Text::new("Payments", "Płatności"),
        title: Text::new("WEB-9 a payment with a test card goes through", "WEB-9 płatność kartą testową przechodzi"),
        line: 11,
        tags: &[Text::new("payments", "płatności")],
        outcomes: "----p-p-",
        seconds: 9.4,
        ..BASE
    },
    Case {
        file: Text::new("tests/payments.e2e.ts", "tests/platnosci.e2e.ts"),
        group: Text::new("Payments", "Płatności"),
        title: Text::new("WEB-9 a declined card shows a clear message", "WEB-9 odrzucona karta pokazuje czytelny komunikat"),
        line: 29,
        tags: &[Text::new("payments", "płatności")],
        outcomes: "----f-p-",
        failure: Some(Failure {
            code: "ASSERTION_FAILED",
            message: "expect.toBeVisible failed",
            locator: Some(Text::new("getByText(\"Your card was declined\")", "getByText(\"Karta została odrzucona\")")),
            expected: Some(Text::same("visible")),
            observed: Some(Text::same("no matching node; the page shows \"Unexpected token < in JSON\"")),
            line: 38,
        }),
        seconds: 6.3,
        ..BASE
    },
    Case {
        file: Text::new("tests/payments.e2e.ts", "tests/platnosci.e2e.ts"),
        group: Text::new("Payments", "Płatności"),
        title: Text::new("WEB-10 Pay by bank with the test bank", "WEB-10 płatność BLIK kodem testowym"),
        line: 46,
        tags: &[Text::new("payments", "płatności")],
        outcomes: "---s-s-s",
        skip: Some(Text::new("Pay by bank is not available in the gateway's sandbox yet", "BLIK w sandboxie bramki jest jeszcze niedostępny")),
        seconds: 0.0,
        ..BASE
    },
    Case {
        file: Text::new("tests/address.e2e.ts", "tests/adres.e2e.ts"),
        group: Text::new("Delivery address", "Adres dostawy"),
        title: Text::new("the form requires a street, a city and a postcode", "formularz wymaga ulicy, miasta i kodu pocztowego"),
        line: 7,
        tags: &[Text::same("@WEB-5")],
        seconds: 3.3,
        ..BASE
    },
    Case {
        file: Text::new("tests/address.e2e.ts", "tests/adres.e2e.ts"),
        group: Text::new("Delivery address", "Adres dostawy"),
        title: Text::new("WEB-5 a saved address is suggested on the next order", "WEB-5 zapisany adres podpowiada się przy kolejnym zamówieniu"),
        line: 22,
        seconds: 4.0,
        ..BASE
    },
    Case {
        file: Text::new("tests/checkout.e2e.ts", "tests/zakup.e2e.ts"),
        group: Text::new("Checkout flow", "Ścieżka zakupowa"),
        title: Text::new("WEB-17 a customer places an order and pays on delivery", "WEB-17 klient składa zamówienie z płatnością przy odbiorze"),
        line: 6,
        tags: &[Text::same("agent")],
        outcomes: "-ppppppp",
        agent_steps: 4,
        seconds: 38.0,
        ..BASE
    },
    Case {
        file: Text::same("tests/smoke.e2e.ts"),
        group: Text::EMPTY,
        title: Text::new("the home page shows the product catalogue", "strona główna pokazuje katalog produktów"),
        line: 4,
        tags: &[Text::same("smoke")],
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

fn error(language: DemoLanguage, failure: &Failure, file: &str) -> Value {
    let pick = |text: Option<Text>| text.map(|text| text.pick(language));

    json!({
        "category": "test",
        "code": failure.code,
        "message": failure.message,
        "retryable": false,
        "phase": "body",
        "details": { "locator": pick(failure.locator), "expected": pick(failure.expected), "observed": pick(failure.observed) },
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

/// One attempt at a test. Ids are made from the Polish texts, so they are the same in both languages.
fn attempt(language: DemoLanguage, case: &Case, index: i64, status: &str, started: &str) -> Value {
    let failed = status == "failed";

    json!({
        "id": format!("{}-{index}", sha(case.title.pl)),
        "index": index,
        "status": status,
        "startedAt": started,
        "durationMs": (case.seconds * 1000.0 * if failed { 1.4 } else { 1.0 }) as i64 + index * 137,
        "artifacts": [],
        "secondaryErrors": [],
        "cleanup": "complete",
        "error": if failed { case.failure.as_ref().map(|failure| error(language, failure, case.file.pick(language))) } else { None },
        "steps": steps(case),
    })
}

/// The report of one run, in the shape `packages/e2e/src/report/build.ts` of the framework writes.
fn report(language: DemoLanguage, run: &Run, position: usize) -> Value {
    let started = at(run.day, run.hour);
    // The commit a run tested has the same SHA whatever the language.
    let commit = sha(&format!("sklep-web-{}", run.number));
    let mut results = Vec::new();
    let mut total_ms = 0;

    for case in CASES {
        let try_once = |index: i64, status: &str| attempt(language, case, index, status, &started);
        let (status, attempts): (&str, Vec<Value>) = match case.outcomes.as_bytes().get(position) {
            Some(b'p') => ("passed", vec![try_once(0, "passed")]),
            Some(b'f') => ("failed", vec![try_once(0, "failed"), try_once(1, "failed")]),
            Some(b'k') => ("flaky", vec![try_once(0, "failed"), try_once(1, "passed")]),
            Some(b's') => ("skipped", Vec::new()),
            _ => continue,
        };
        let file = case.file.pick(language);
        let title_path: Vec<&str> = [case.group.pick(language), case.title.pick(language)].into_iter().filter(|part| !part.is_empty()).collect();
        let tags: Vec<&str> = case.tags.iter().map(|tag| tag.pick(language)).collect();

        total_ms += attempts.iter().filter_map(|attempt| attempt["durationMs"].as_i64()).sum::<i64>();
        results.push(json!({
            "id": sha(&format!("{}{}", case.file.pl, case.title.pl)),
            "testId": format!("{file}::{}", title_path.join("::")),
            "kind": "test",
            "titlePath": title_path,
            "file": file,
            "source": { "file": file, "line": case.line, "column": 1 },
            "targetId": "chromium",
            "platform": "web",
            "agent": "default",
            "repeat": 0,
            "tags": tags,
            "selected": true,
            "status": status,
            "skip": case.skip.filter(|_| status == "skipped").map(|reason| json!({ "cause": "explicit", "reason": reason.pick(language) })),
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
            "id": format!("demo-{}-{}", REPO.pick(language).trim_start_matches("acme/"), run.number),
            "specVersion": "0.1",
            "runner": { "name": "e2e", "version": "0.23.0" },
            "status": if failed { "failed" } else { "passed" },
            "exitCode": if failed { 1 } else { 0 },
            "startedAt": started,
            "finishedAt": finished,
            "environment": { "ci": true, "os": "linux", "arch": "x64", "runtime": "node 24.8.0" },
            "vcs": { "commit": commit, "branch": run.branch.pick(language), "dirty": false },
            "serialGroups": [],
            "errors": [],
            "results": results,
        }
    })
}

pub fn seed_demo(app: &App, language: DemoLanguage) -> ApiResult<()> {
    let web: Option<Id> = app.read(|connection| query_one(connection, "SELECT id FROM project WHERE key = 'WEB'", [], |row| row.get(0)))?;
    let Some(web) = web else { return Ok(()) };
    let projects = app.read(project_keys)?;
    let repo = REPO.pick(language);

    for (position, run) in RUNS.iter().enumerate() {
        let document = report(language, run, position);
        let mut parsed = parse_json(document.to_string().as_bytes())?;
        parsed.find_issue_keys(&projects);

        let meta = RunMeta {
            name: Some(format!("CI #{}", run.number)),
            pr_number: run.pull,
            ci_url: Some(format!("https://github.com/{repo}/actions/runs/{}", 7_300_000_000 + run.number * 1_117)),
            ..Default::default()
        };

        app.write_at(Actor::system("CI"), document["run"]["finishedAt"].as_str().map(String::from).unwrap_or_else(|| at(run.day, run.hour)), |tx| {
            store_run(tx, None, web, &parsed, meta).map(|_| ())
        })?;
    }

    Ok(())
}
