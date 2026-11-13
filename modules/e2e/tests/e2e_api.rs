mod common;

use axum::http::{Method, StatusCode};
use common::{events, found, issue, member, person, project, test_app, transition, Client};
use hub_app::services::issues::{delete_issue, issue_id_by_key};
use hub_app::{App, Module};
use hub_core::{ProjectRole, Role};
use hub_mod_e2e::E2eModule;
use serde_json::{json, Value};

/// The framework's own example of a valid report (`packages/e2e/schema/fixtures/report-v1.valid.json`).
const FIXTURE: &str = include_str!("fixtures/report-v1.valid.json");

/// One result of a report, as the framework writes it. `status` decides the attempts.
fn result(file: &str, titles: &[&str], status: &str, tags: &[&str]) -> Value {
    let failure = json!({ "category": "test", "code": "ASSERTION_FAILED", "message": "expect.toHaveText failed", "retryable": false,
                          "details": { "expected": "text \"OK\"", "observed": "text \"500\"" }, "source": { "file": file, "line": 17, "column": 3 } });
    let attempt = |index: i64, status: &str| json!({ "index": index, "status": status, "durationMs": 1200 + index * 100, "steps": [], "error": if status == "failed" { failure.clone() } else { Value::Null } });
    let attempts = match status {
        "passed" => vec![attempt(0, "passed")],
        "failed" => vec![attempt(0, "failed"), attempt(1, "failed")],
        "flaky" => vec![attempt(0, "failed"), attempt(1, "passed")],
        _ => Vec::new(),
    };

    json!({
        "testId": format!("{file}::{}", titles.join("::")),
        "titlePath": titles,
        "file": file,
        "source": { "file": file, "line": 5, "column": 1 },
        "targetId": "chromium",
        "agent": "default",
        "repeat": 0,
        "tags": tags,
        "selected": true,
        "status": status,
        "skip": if status == "skipped" { json!({ "cause": "explicit", "reason": "not today" }) } else { Value::Null },
        "attempts": attempts,
    })
}

fn report(id: &str, started_at: &str, branch: &str, results: Vec<Value>) -> Value {
    let failed = results.iter().any(|result| result["status"] == "failed");

    json!({
        "schemaVersion": "report-1",
        "run": {
            "id": id,
            "runner": { "name": "e2e", "version": "0.23.0" },
            "status": if failed { "failed" } else { "passed" },
            "startedAt": started_at,
            "finishedAt": started_at,
            "vcs": { "commit": "0123456789abcdef0123456789abcdef01234567", "branch": branch },
            "serialGroups": [],
            "errors": [],
            "results": results,
        }
    })
}

struct Hub {
    app: App,
    admin: Client,
    ci: Client,
}

/// A project WEB with four issues, an administrator and a member whose token CI uses.
fn hub() -> Hub {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let ci = person(&app, "Cezary CI", Role::Member);
    project(&app, &admin, "WEB", false);

    for summary in ["Cart", "Login", "Card payments", "Password reset"] {
        issue(&app, &admin, "WEB", summary);
    }

    Hub { app, admin, ci }
}

fn test<'a>(page: &'a Value, title: &str) -> &'a Value {
    page["tests"].as_array().expect("tests").iter().find(|test| test["title"] == title).unwrap_or_else(|| panic!("no test titled {title} in {page}"))
}

fn issue_keys(test: &Value) -> Vec<&str> {
    test["issues"].as_array().expect("issues").iter().map(|issue| issue["key"].as_str().unwrap_or("")).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn posting_needs_a_token_and_the_right_to_edit() {
    let Hub { app, admin, ci } = hub();
    let body = report("run-1", "2026-11-12T10:00:00Z", "main", vec![result("a.e2e.ts", &["WEB-1 adds to cart"], "passed", &[])]);

    let mut stranger = ci.clone();
    stranger.bearer = None;
    assert_eq!(stranger.refused(Method::POST, "/projects/WEB/runs", Some(body.clone())).await.0, StatusCode::UNAUTHORIZED);
    stranger.bearer = Some("th_not-a-token".to_string());
    assert_eq!(stranger.refused(Method::POST, "/projects/WEB/runs", Some(body.clone())).await.0, StatusCode::UNAUTHORIZED);

    // A viewer reads the page but cannot post runs or change links.
    let viewer = person(&app, "Wanda Viewer", Role::Member);
    member(&app, &admin, "WEB", &viewer, ProjectRole::Viewer);
    assert_eq!(viewer.refused(Method::POST, "/projects/WEB/runs", Some(body.clone())).await.0, StatusCode::FORBIDDEN);

    let posted = ci.post("/projects/WEB/runs", body).await;
    assert_eq!(posted["created"], true);
    let page = viewer.get("/projects/WEB").await;
    let test_id = page["tests"][0]["id"].as_i64().unwrap();
    assert_eq!((page["canEdit"].clone(), page["counts"]["total"].clone()), (json!(false), json!(1)));
    assert_eq!(viewer.refused(Method::PUT, &format!("/tests/{test_id}/issues/WEB-2"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(viewer.refused(Method::DELETE, &format!("/tests/{test_id}"), None).await.0, StatusCode::FORBIDDEN);
    // Removing a run is the project administrator's.
    assert_eq!(ci.refused(Method::DELETE, &format!("/runs/{}", posted["run"]["id"]), None).await.0, StatusCode::FORBIDDEN);

    // A private project does not exist for outsiders, nor do its runs.
    project(&app, &admin, "SEC", true);
    let secret =
        admin.post("/projects/SEC/runs", report("s-1", "2026-11-12T10:00:00Z", "main", vec![result("s.e2e.ts", &["secret"], "passed", &[])])).await;
    assert_eq!(ci.refused(Method::GET, "/projects/SEC", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ci.refused(Method::POST, "/projects/SEC/runs", Some(json!({}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ci.refused(Method::GET, &format!("/runs/{}", secret["run"]["id"]), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ci.refused(Method::GET, "/projects/NOPE", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn takes_the_frameworks_report_as_it_is_and_only_once() {
    let Hub { ci, .. } = hub();
    let (status, posted) = ci.send_raw(Method::POST, "/projects/WEB/runs", Some("application/json"), FIXTURE.as_bytes().to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{posted}");

    let run = &posted["run"];
    assert_eq!(posted["created"], true);
    assert_eq!((run["format"].as_str(), run["status"].as_str(), run["projectKey"].as_str()), (Some("json"), Some("failed"), Some("WEB")));
    // Three of the four results count: the one a test filter left out says nothing about its test.
    assert_eq!(
        (run["total"].clone(), run["passed"].clone(), run["failed"].clone(), run["skipped"].clone()),
        (json!(3), json!(1), json!(1), json!(1))
    );
    assert_eq!((run["agentSteps"].clone(), run["modelCalls"].clone()), (json!(3), json!(2)));
    assert_eq!(posted["url"], format!("http://localhost/p/WEB/tests?run={}", run["id"]));

    // The same report again changes nothing.
    let (_, again) = ci.send_raw(Method::POST, "/projects/WEB/runs", Some("application/json"), FIXTURE.as_bytes().to_vec()).await;
    assert_eq!((again["created"].clone(), again["run"]["id"].clone()), (json!(false), run["id"].clone()));

    let page = ci.get("/projects/WEB").await;
    assert_eq!(page["runs"].as_array().unwrap().len(), 1);
    assert_eq!(page["latestRun"]["id"], run["id"]);
    assert_eq!(page["ingestUrl"], "http://localhost/api/m/e2e/projects/WEB/runs");
    assert_eq!(page["counts"], json!({ "total": 3, "passing": 1, "failing": 1, "flaky": 0, "skipped": 1, "unlinked": 3 }));
    // Failing tests come first.
    assert_eq!(page["tests"][0]["title"], "checkout > retains serial verdict");
    assert_eq!((page["tests"][0]["status"].as_str(), page["tests"][0]["attempts"].clone()), (Some("failed"), json!(2)));

    let detail = ci.get(&format!("/runs/{}", run["id"])).await;
    let results = detail["results"].as_array().unwrap();
    assert_eq!(results.iter().map(|result| result["status"].as_str().unwrap()).collect::<Vec<_>>(), vec!["failed", "passed", "skipped"]);
    assert_eq!(results[2]["message"], "the demo tenant has a single organization");
    assert_eq!(
        (results[1]["file"].as_str(), results[1]["line"].clone(), results[1]["durationMs"].clone()),
        (Some("tests/example.e2e.ts"), json!(3), json!(1000))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_what_it_cannot_read() {
    let Hub { ci, .. } = hub();

    for (content_type, body, expected) in [
        ("application/json", "", "no body"),
        ("application/json", "{\"results\": []}", "no \"run\""),
        ("application/json", "{ not json", "not valid JSON"),
        ("application/xml", "<testsuites><testsuite></testsuites>", "not valid XML"),
        ("text/xml", "<report/>", "not a JUnit file"),
    ] {
        let (status, value) = ci.send_raw(Method::POST, "/projects/WEB/runs", Some(content_type), body.as_bytes().to_vec()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {value}");
        assert!(value["error"]["message"].as_str().unwrap().contains(expected), "{body}: {value}");
    }

    let mut document = report("m-1", "2026-11-12T10:00:00Z", "main", Vec::new());
    document["meta"] = json!("main");
    assert!(ci.refused(Method::POST, "/projects/WEB/runs", Some(document)).await.1.contains("\"meta\""));
    assert_eq!(ci.get("/projects/WEB").await["runs"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_are_linked_to_the_issues_they_name() {
    let Hub { app, admin, ci } = hub();
    project(&app, &admin, "SEC", true);
    issue(&app, &admin, "SEC", "Hidden");

    let results = vec![
        result("cart.e2e.ts", &["Cart", "WEB-1 a customer adds a product"], "passed", &[]),
        // A key in a describe title, one in a tag (any case), one in an annotation.
        result("login.e2e.ts", &["WEB-2 Login", "signs in with a password"], "failed", &["@web-4", "smoke"]),
        result("pay.e2e.ts", &["pays by card"], "flaky", &[]),
        // Not keys: an unknown project, a key glued to a word, lower case in a title, a missing issue,
        // and an issue of a private project the poster cannot see.
        result("misc.e2e.ts", &["UTF-8 names, XWEB-1, web-1, WEB-99 and SEC-1"], "passed", &[]),
    ];
    let mut document = report("run-1", "2026-11-12T10:00:00Z", "main", results);
    document["run"]["results"][2]["annotations"] = json!([{ "type": "issue", "description": "WEB-3" }]);

    let posted = ci.post("/projects/WEB/runs", document.clone()).await;
    assert_eq!(posted["linkedIssues"], json!(["WEB-1", "WEB-2", "WEB-3", "WEB-4"]));

    let page = ci.get("/projects/WEB").await;
    assert_eq!(issue_keys(test(&page, "Cart > WEB-1 a customer adds a product")), vec!["WEB-1"]);
    assert_eq!(issue_keys(test(&page, "WEB-2 Login > signs in with a password")), vec!["WEB-2", "WEB-4"]);
    assert_eq!(issue_keys(test(&page, "pays by card")), vec!["WEB-3"]);
    assert!(issue_keys(test(&page, "UTF-8 names, XWEB-1, web-1, WEB-99 and SEC-1")).is_empty());
    assert_eq!(page["counts"]["unlinked"], 1);

    // The administrator, who may see SEC, gets that link when posting.
    document["run"]["id"] = json!("run-2");
    document["run"]["startedAt"] = json!("2026-11-12T11:00:00Z");
    let posted = admin.post("/projects/WEB/runs", document.clone()).await;
    assert!(posted["linkedIssues"].as_array().unwrap().contains(&json!("SEC-1")));
    // ...and only those who may see the issue are shown it.
    assert_eq!(issue_keys(test(&admin.get("/projects/WEB").await, "UTF-8 names, XWEB-1, web-1, WEB-99 and SEC-1")), vec!["SEC-1"]);
    assert!(issue_keys(test(&ci.get("/projects/WEB").await, "UTF-8 names, XWEB-1, web-1, WEB-99 and SEC-1")).is_empty());

    // By hand: a link the title does not carry, and the removal of one it does.
    let pay = test(&page, "pays by card")["id"].as_i64().unwrap();
    let login = test(&page, "WEB-2 Login > signs in with a password")["id"].as_i64().unwrap();
    ci.put(&format!("/tests/{pay}/issues/web-1")).await;
    ci.delete(&format!("/tests/{login}/issues/WEB-4")).await;
    assert_eq!(ci.refused(Method::PUT, &format!("/tests/{pay}/issues/WEB-99"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ci.refused(Method::PUT, &format!("/tests/{pay}/issues/SEC-1"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ci.refused(Method::DELETE, &format!("/tests/{pay}/issues/WEB-2"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ci.refused(Method::PUT, "/tests/9999/issues/WEB-1", None).await.0, StatusCode::NOT_FOUND);

    // Both survive the next run; a title that no longer names an issue drops its link.
    document["run"]["id"] = json!("run-3");
    document["run"]["startedAt"] = json!("2026-11-12T12:00:00Z");
    document["run"]["results"][2]["annotations"] = Value::Null;
    ci.post("/projects/WEB/runs", document).await;

    let page = ci.get("/projects/WEB").await;
    let pay = test(&page, "pays by card");
    assert_eq!(issue_keys(pay), vec!["WEB-1"]);
    assert_eq!(pay["issues"][0]["manual"], true);
    assert_eq!(issue_keys(test(&page, "WEB-2 Login > signs in with a password")), vec!["WEB-2"]);
    assert_eq!(test(&page, "Cart > WEB-1 a customer adds a product")["issues"][0]["manual"], false);

    // Linking again by hand lifts the removal.
    ci.put(&format!("/tests/{login}/issues/WEB-4")).await;
    assert_eq!(issue_keys(test(&ci.get("/projects/WEB").await, "WEB-2 Login > signs in with a password")), vec!["WEB-2", "WEB-4"]);
}

const JUNIT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="e2e" tests="4" failures="1" errors="0" skipped="1" time="9.500">
  <testsuite name="tests/cart.e2e.ts" tests="4" failures="1" errors="0" skipped="1" time="9.500">
    <testcase name="Cart &gt; WEB-1 a customer adds a product [chromium]" classname="tests/cart.e2e.ts" time="3.250"/>
    <testcase name="Cart &gt; WEB-2 an empty cart shows a message [chromium]" classname="tests/cart.e2e.ts" time="5.000">
      <failure message="expect.toHaveText failed" type="ASSERTION_FAILED">ASSERTION_FAILED (body): expect.toHaveText failed</failure>
    </testcase>
    <testcase name="WEB-4 reset sends a link [chromium]" classname="tests/cart.e2e.ts" time="1.250">
      <system-out>flaky: 1 failed attempt before passing</system-out>
    </testcase>
    <testcase name="WEB-3 pays by BLIK [chromium]" classname="tests/cart.e2e.ts" time="0.000">
      <skipped message="no sandbox yet"/>
    </testcase>
  </testsuite>
</testsuites>
"#;

#[tokio::test(flavor = "multi_thread")]
async fn takes_junit_xml_with_what_ci_knows_in_the_query() {
    let Hub { ci, .. } = hub();
    let path = "/projects/web/runs?runId=gha-77-1&name=CI%20%2377&branch=refs/heads/feature/WEB-2-cart&commit=abc1234&pr=12&ciUrl=https://ci.example/runs/77";
    let posted = ci.post_xml(path, JUNIT).await;
    let run = &posted["run"];

    assert_eq!((run["format"].as_str(), run["status"].as_str(), run["name"].as_str()), (Some("junit"), Some("failed"), Some("CI #77")));
    assert_eq!((run["branch"].as_str(), run["commit"].as_str(), run["prNumber"].clone()), (Some("feature/WEB-2-cart"), Some("abc1234"), json!(12)));
    assert_eq!((run["ciUrl"].as_str(), run["durationMs"].clone()), (Some("https://ci.example/runs/77"), json!(9500)));
    assert_eq!(
        (run["total"].clone(), run["passed"].clone(), run["failed"].clone(), run["flaky"].clone(), run["skipped"].clone()),
        (json!(4), json!(1), json!(1), json!(1), json!(1))
    );
    assert_eq!(posted["linkedIssues"], json!(["WEB-1", "WEB-2", "WEB-3", "WEB-4"]));

    // The run id from the query makes a JUnit post idempotent too.
    assert_eq!(ci.post_xml(path, JUNIT).await["created"], false);
    // An address that is not a web address is not kept as a link.
    let other = ci.post_xml("/projects/WEB/runs?runId=gha-78-1&ciUrl=javascript:alert(1)", JUNIT).await;
    assert_eq!(other["run"]["ciUrl"], Value::Null);

    let page = ci.get("/projects/WEB").await;
    let failed = test(&page, "Cart > WEB-2 an empty cart shows a message");
    assert_eq!(
        (failed["status"].as_str(), failed["message"].as_str(), failed["errorCode"].as_str()),
        (Some("failed"), Some("expect.toHaveText failed"), Some("ASSERTION_FAILED"))
    );
    assert_eq!(test(&page, "WEB-4 reset sends a link")["attempts"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_run_that_ran_a_test_decides_and_history_shows_the_rest() {
    let Hub { ci, .. } = hub();
    let post = |id: &'static str, at: &'static str, branch: &'static str, statuses: [&'static str; 2]| {
        let ci = ci.clone();

        async move {
            let results = vec![result("a.e2e.ts", &["WEB-1 steady"], statuses[0], &[]), result("a.e2e.ts", &["WEB-2 wobbly"], statuses[1], &[])];

            ci.post("/projects/WEB/runs", report(id, at, branch, results)).await["run"]["id"].as_i64().unwrap()
        }
    };

    let first = post("r1", "2026-11-10T10:00:00Z", "main", ["passed", "passed"]).await;
    let third = post("r3", "2026-11-12T10:00:00Z", "main", ["failed", "passed"]).await;
    // A report that arrives late does not overwrite what a newer run said.
    let second = post("r2", "2026-11-11T10:00:00Z", "main", ["passed", "failed"]).await;

    let page = ci.get("/projects/WEB").await;
    assert_eq!(page["latestRun"]["id"], third);
    assert_eq!(page["runs"].as_array().unwrap().iter().map(|run| run["id"].as_i64().unwrap()).collect::<Vec<_>>(), vec![third, second, first]);

    let steady = test(&page, "WEB-1 steady");
    assert_eq!((steady["status"].as_str(), steady["lastRunId"].clone(), steady["unstable"].clone()), (Some("failed"), json!(third), json!(false)));
    assert_eq!(
        steady["history"].as_array().unwrap().iter().map(|point| point["status"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["passed", "passed", "failed"]
    );
    assert_eq!(
        (steady["message"].as_str().map(|message| message.lines().next().unwrap()), steady["branch"].as_str()),
        (Some("expect.toHaveText failed"), Some("main"))
    );

    // Passing, failing, passing on one branch: unstable, though its newest result is a pass.
    let wobbly = test(&page, "WEB-2 wobbly");
    assert_eq!((wobbly["status"].as_str(), wobbly["unstable"].clone()), (Some("passed"), json!(true)));
    assert_eq!(page["counts"]["flaky"], 1);

    // A run that ran only one of the tests (`e2e run --grep`) leaves the other as it was.
    let partial = report("r4", "2026-11-13T10:00:00Z", "main", vec![result("a.e2e.ts", &["WEB-2 wobbly"], "flaky", &[])]);
    ci.post("/projects/WEB/runs", partial).await;
    let page = ci.get("/projects/WEB").await;
    assert_eq!((test(&page, "WEB-1 steady")["status"].as_str(), test(&page, "WEB-2 wobbly")["status"].as_str()), (Some("failed"), Some("flaky")));
    assert_eq!(page["canAdmin"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn issues_show_their_tests_on_the_panel_the_card_and_in_queries() {
    let Hub { app, admin, ci } = hub();
    transition(&app, &admin, "WEB-1", "Done");
    transition(&app, &admin, "WEB-2", "Done");

    let module = E2eModule;
    let panel_of = |key: &str| {
        app.read(|connection| {
            let (issue_id, _) = issue_id_by_key(connection, key)?.expect("the issue");
            let row = hub_app::services::issues::row_by_id(&app, connection, issue_id)?;

            Ok((module.issue_panel(&app, connection, &row, &ci.user)?, row))
        })
        .expect("a panel")
    };

    // Nothing has been posted: no panel, no badge, and every issue is without tests.
    assert_eq!(panel_of("WEB-1").0, None);
    assert_eq!(found(&app, &ci, "tests is empty"), vec!["WEB-1", "WEB-2", "WEB-3", "WEB-4"]);

    let results = vec![
        result("cart.e2e.ts", &["WEB-1 adds a product"], "passed", &[]),
        result("cart.e2e.ts", &["WEB-1 removes a product"], "passed", &[]),
        result("login.e2e.ts", &["WEB-2 signs in"], "failed", &[]),
        result("login.e2e.ts", &["WEB-2 signs out"], "flaky", &[]),
        result("pay.e2e.ts", &["WEB-3 pays by card"], "flaky", &[]),
        result("pay.e2e.ts", &["WEB-3 pays by BLIK"], "skipped", &[]),
    ];
    let mut document = report("run-1", "2026-11-12T10:00:00Z", "main", results);
    document["meta"] = json!({ "name": "CI #5", "pr": 7, "ciUrl": "https://ci.example/5" });
    let run_id = ci.post("/projects/WEB/runs", document).await["run"]["id"].as_i64().unwrap();

    // The panel: last result, when, on which branch and commit, and the run.
    let (panel, row) = panel_of("WEB-2");
    let panel = panel.expect("a panel");
    assert_eq!(panel["state"], "failing");
    assert_eq!(panel["tests"].as_array().unwrap().len(), 2);
    let failing = &panel["tests"][0];
    assert_eq!(
        (failing["title"].as_str(), failing["status"].as_str(), failing["projectKey"].as_str()),
        (Some("WEB-2 signs in"), Some("failed"), Some("WEB"))
    );
    assert_eq!((failing["runId"].clone(), failing["runName"].as_str(), failing["branch"].as_str()), (json!(run_id), Some("CI #5"), Some("main")));
    assert_eq!(
        (failing["commit"].as_str(), failing["at"].as_str(), failing["ciUrl"].as_str()),
        (Some("0123456789abcdef0123456789abcdef01234567"), Some("2026-11-12T10:00:00.000Z"), Some("https://ci.example/5"))
    );
    assert_eq!(failing["history"][0]["status"], "failed");
    // The badge on its card.
    assert_eq!(row.ext["e2e"], json!({ "state": "failing", "total": 2, "passing": 0, "failing": 1, "flaky": 1 }));
    assert_eq!(panel_of("WEB-1").1.ext["e2e"]["state"], "passing");
    assert_eq!(panel_of("WEB-3").1.ext["e2e"]["state"], "flaky");
    // An issue without tests has no badge, but its panel is there now that the project uses tests.
    let (panel, row) = panel_of("WEB-4");
    assert_eq!(panel, Some(json!({ "state": null, "tests": [] })));
    assert!(!row.ext.contains_key("e2e"));

    // The query field says what an issue's tests say together.
    assert_eq!(found(&app, &ci, "tests = failing"), vec!["WEB-2"]);
    assert_eq!(found(&app, &ci, "tests = flaky"), vec!["WEB-3"]);
    assert_eq!(found(&app, &ci, "tests = passing"), vec!["WEB-1"]);
    assert_eq!(found(&app, &ci, "tests = none"), vec!["WEB-4"]);
    assert_eq!(found(&app, &ci, "tests is empty"), vec!["WEB-4"]);
    assert_eq!(found(&app, &ci, "tests is not empty"), vec!["WEB-1", "WEB-2", "WEB-3"]);
    assert_eq!(found(&app, &ci, "tests in (failing, flaky)"), vec!["WEB-2", "WEB-3"]);
    assert_eq!(found(&app, &ci, "tests != passing"), vec!["WEB-2", "WEB-3", "WEB-4"]);
    assert_eq!(found(&app, &ci, "category = done AND tests = failing"), vec!["WEB-2"]);
    assert_eq!(found(&app, &ci, "project = WEB AND TESTS = Failed"), vec!["WEB-2"]);

    let bad = hub_app::services::search::search(&app, &ci.user, "tests = broken", 10, 0).unwrap();
    assert!(bad.error.unwrap().message.contains("not one of: passing, failing, flaky, skipped, none"));
    let bad = hub_app::services::search::search(&app, &ci.user, "tests > failing", 10, 0).unwrap();
    assert!(bad.error.unwrap().message.contains("operator"));
    assert_eq!(module.query_fields()[0].name, "tests");

    // "My issues with failing tests": the administrator is assigned every issue, the CI user none.
    let mine = app.read(|connection| module.dashboard(connection, &admin.user)).unwrap().expect("a section");
    assert_eq!(mine["issues"].as_array().unwrap().len(), 1);
    assert_eq!(
        (mine["issues"][0]["key"].as_str(), mine["issues"][0]["done"].clone(), mine["issues"][0]["tests"][0]["title"].as_str()),
        (Some("WEB-2"), json!(true), Some("WEB-2 signs in"))
    );
    assert_eq!(app.read(|connection| module.dashboard(connection, &ci.user)).unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_can_ask_about_tests() {
    let Hub { app, ci, .. } = hub();
    let results = vec![
        result("login.e2e.ts", &["WEB-2 signs in"], "failed", &[]),
        result("pay.e2e.ts", &["WEB-3 pays by card"], "flaky", &[]),
        result("cart.e2e.ts", &["WEB-1 adds"], "passed", &[]),
    ];
    ci.post("/projects/WEB/runs", report("run-1", "2026-11-12T10:00:00Z", "main", results)).await;

    let module = E2eModule;
    let names: Vec<&str> = module.tools().iter().map(|tool| tool.name).collect();
    assert_eq!(names, vec!["get_issue_tests", "list_failing_tests"]);
    assert!(module.tools().iter().all(|tool| tool.read_only));

    let tests = module.call_tool(&app, &ci.user, "get_issue_tests", &json!({ "key": "WEB-2" })).unwrap().unwrap();
    assert_eq!(
        (tests["state"].as_str(), tests["tests"][0]["status"].as_str(), tests["tests"][0]["file"].as_str()),
        (Some("failing"), Some("failed"), Some("login.e2e.ts"))
    );
    let none = module.call_tool(&app, &ci.user, "get_issue_tests", &json!({ "key": "WEB-4" })).unwrap().unwrap();
    assert_eq!((none["state"].clone(), none["tests"].clone()), (Value::Null, json!([])));

    let failing = module.call_tool(&app, &ci.user, "list_failing_tests", &json!({ "projectKey": "web" })).unwrap().unwrap();
    assert_eq!((failing["tests"].clone(), failing["latestRun"]["status"].as_str()), (json!(3), Some("failed")));
    assert_eq!(
        (failing["failing"][0]["title"].as_str(), failing["failing"][0]["line"].clone(), failing["failing"][0]["issues"][0]["key"].as_str()),
        (Some("WEB-2 signs in"), json!(5), Some("WEB-2"))
    );
    assert_eq!(failing["flaky"][0]["title"], "WEB-3 pays by card");

    assert_eq!(module.call_tool(&app, &ci.user, "get_issue_tests", &json!({})).unwrap().unwrap_err().status, StatusCode::BAD_REQUEST);
    assert_eq!(module.call_tool(&app, &ci.user, "get_issue_tests", &json!({ "key": "NOPE-1" })).unwrap().unwrap_err().status, StatusCode::NOT_FOUND);
    assert_eq!(
        module.call_tool(&app, &ci.user, "list_failing_tests", &json!({ "projectKey": "NOPE" })).unwrap().unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert!(module.call_tool(&app, &ci.user, "something_else", &json!({})).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_announces_itself_and_the_issues_it_touches() {
    let Hub { app, ci, .. } = hub();
    let results = vec![
        result("cart.e2e.ts", &["WEB-1 adds"], "passed", &[]),
        result("login.e2e.ts", &["WEB-2 signs in"], "failed", &[]),
        result("x.e2e.ts", &["no issue"], "passed", &[]),
    ];
    let mut stream = app.subscribe();

    ci.post("/projects/WEB/runs", report("run-1", "2026-11-12T10:00:00Z", "main", results.clone())).await;
    assert_eq!(
        events(&app),
        vec![
            ("e2e.run".to_string(), None),
            ("e2e.updated".to_string(), Some("WEB-1".to_string())),
            ("e2e.updated".to_string(), Some("WEB-2".to_string()))
        ]
    );

    // The same events reach open pages.
    let live = stream.recv().await.unwrap();
    assert_eq!((live.kind.as_str(), live.payload["failed"].clone(), live.payload["total"].clone()), ("e2e.run", json!(1), json!(3)));
    assert_eq!(stream.recv().await.unwrap().kind, "e2e.updated");

    // A duplicate announces nothing; a link set by hand announces its issue.
    ci.post("/projects/WEB/runs", report("run-1", "2026-11-12T10:00:00Z", "main", results)).await;
    assert_eq!(events(&app).len(), 3);
    let test_id = test(&ci.get("/projects/WEB").await, "no issue")["id"].as_i64().unwrap();
    ci.put(&format!("/tests/{test_id}/issues/WEB-3")).await;
    assert_eq!(events(&app).last().unwrap(), &("e2e.updated".to_string(), Some("WEB-3".to_string())));
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_tests_and_issues_can_be_removed() {
    let Hub { app, admin, ci } = hub();
    let first =
        ci.post("/projects/WEB/runs", report("r1", "2026-11-11T10:00:00Z", "main", vec![result("a.e2e.ts", &["WEB-1 old"], "passed", &[])])).await;
    let second = ci
        .post(
            "/projects/WEB/runs",
            report(
                "r2",
                "2026-11-12T10:00:00Z",
                "main",
                vec![result("a.e2e.ts", &["WEB-1 old"], "failed", &[]), result("a.e2e.ts", &["WEB-2 new"], "passed", &[])],
            ),
        )
        .await;
    assert_eq!(found(&app, &ci, "tests = failing"), vec!["WEB-1"]);

    // Without its newest run a test says what the run before said; a test only that run knew is gone.
    admin.delete(&format!("/runs/{}", second["run"]["id"])).await;
    let page = ci.get("/projects/WEB").await;
    assert_eq!(page["runs"].as_array().unwrap().len(), 1);
    assert_eq!(page["tests"].as_array().unwrap().len(), 1);
    assert_eq!((page["tests"][0]["status"].as_str(), page["tests"][0]["lastRunId"].clone()), (Some("passed"), first["run"]["id"].clone()));
    assert_eq!(found(&app, &ci, "tests = passing"), vec!["WEB-1"]);
    assert_eq!(found(&app, &ci, "tests is empty"), vec!["WEB-2", "WEB-3", "WEB-4"]);
    assert_eq!(ci.refused(Method::GET, &format!("/runs/{}", second["run"]["id"]), None).await.0, StatusCode::NOT_FOUND);

    // Forgetting a test takes its links along.
    ci.delete(&format!("/tests/{}", page["tests"][0]["id"])).await;
    assert_eq!(ci.get("/projects/WEB").await["tests"], json!([]));
    assert_eq!(found(&app, &ci, "tests is not empty"), Vec::<String>::new());

    // A deleted issue leaves no link behind for a later issue to inherit.
    ci.post("/projects/WEB/runs", report("r3", "2026-11-13T10:00:00Z", "main", vec![result("a.e2e.ts", &["WEB-3 pays"], "passed", &[])])).await;
    let links = |app: &App| {
        app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM link WHERE from_kind = 'e2e_test'", [], |row| row.get::<_, i64>(0))?))
            .unwrap()
    };
    assert_eq!(links(&app), 1);
    app.write(admin.user.actor(), |tx| {
        let (issue_id, _) = issue_id_by_key(tx, "WEB-3")?.expect("the issue");

        delete_issue(tx, issue_id).map(|_| ())
    })
    .unwrap();
    assert_eq!(links(&app), 0);
    assert!(issue_keys(&ci.get("/projects/WEB").await["tests"][0]).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demo_has_a_done_issue_with_a_failing_test() {
    let app = test_app();
    let owner = person(&app, "Olga Owner", Role::Admin);
    hub_app::seed::seed_demo(&app, owner.user.id).expect("demo data");

    let page = owner.get("/projects/WEB").await;
    let tests = page["tests"].as_array().unwrap();
    assert_eq!(page["runs"].as_array().unwrap().len(), 8);
    assert_eq!(page["counts"], json!({ "total": 14, "passing": 11, "failing": 1, "flaky": 1, "skipped": 1, "unlinked": 1 }));
    assert_eq!(
        (page["latestRun"]["name"].as_str(), page["latestRun"]["status"].as_str(), page["latestRun"]["branch"].as_str()),
        (Some("CI #236"), Some("failed"), Some("main"))
    );
    assert!(page["latestRun"]["agentSteps"].as_i64().unwrap() > 0 && page["latestRun"]["costUsd"].as_f64().unwrap() > 0.0);
    assert!(page["bugTypeId"].is_i64());

    // The one failing test verifies an issue that is done.
    let failing = &tests[0];
    assert_eq!(failing["title"], "Koszyk > WEB-11 pusty koszyk pokazuje komunikat zamiast błędu 500");
    assert_eq!(
        (failing["status"].as_str(), failing["issues"][0]["key"].as_str(), failing["issues"][0]["done"].clone()),
        (Some("failed"), Some("WEB-11"), json!(true))
    );
    assert_eq!(found(&app, &owner, "category = done AND tests = failing"), vec!["WEB-11"]);
    assert_eq!(found(&app, &owner, "tests = flaky"), vec!["WEB-14"]);

    let flaky = &tests[1];
    assert_eq!(
        (flaky["status"].as_str(), flaky["unstable"].clone(), flaky["issues"][0]["key"].as_str()),
        (Some("flaky"), json!(true), Some("WEB-14"))
    );
    assert_eq!(
        tests.iter().filter(|test| test["issues"].as_array().unwrap().is_empty()).map(|test| test["title"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["strona główna pokazuje katalog produktów"]
    );
    // A tag links too, and a test that last ran on a branch says so.
    assert_eq!(issue_keys(test(&page, "Adres dostawy > formularz wymaga ulicy, miasta i kodu pocztowego")), vec!["WEB-5"]);
    assert_eq!(test(&page, "Płatności > WEB-9 odrzucona karta pokazuje czytelny komunikat")["branch"], "feature/WEB-9-platnosc-karta");

    // The drill-down into the newest run shows the failure.
    let detail = owner.get(&format!("/runs/{}", page["latestRun"]["id"])).await;
    let first = &detail["results"][0];
    assert_eq!((first["status"].as_str(), first["line"].clone(), first["attempts"].clone()), (Some("failed"), json!(53), json!(2)));
    assert!(first["message"].as_str().unwrap().contains("Twój koszyk jest pusty"));
}
