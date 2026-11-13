//! The reports on the demo data. The expected numbers follow from `hub_app::seed`: every
//! issue of the demo is listed there with its sprint, points and the days of its moves.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use hub_app::seed::seed_demo;
use hub_app::services::projects::{create_project, NewProject};
use hub_app::services::users::{create_token, create_user, NewUser};
use hub_app::{App, AuthUser, Config, Module};
use hub_core::Role;
use hub_mod_reports::ReportsModule;
use serde_json::Value;
use tower::ServiceExt;

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Demo {
    app: App,
    router: Router,
    admin: AuthUser,
    token: String,
}

fn person(app: &App, name: &str, email: &str, role: Role) -> (AuthUser, String) {
    let new = NewUser {
        name: name.to_string(),
        email: email.to_string(),
        password: None,
        role: Some(role),
        github_login: None,
    };
    let (user, token) = app
        .db
        .write(|tx| {
            let user = create_user(tx, &new)?;
            let token = create_token(tx, user.id, "tests")?;

            Ok((user, token))
        })
        .expect("a user with a token");

    (
        AuthUser {
            id: user.id,
            name: user.name,
            email: user.email,
            role,
            via_token: true,
        },
        token,
    )
}

fn demo() -> Demo {
    let directory = std::env::temp_dir().join(format!(
        "team-hub-reports-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let app = App::new(Config::for_tests(directory), vec![Arc::new(ReportsModule)]).expect("the test application starts");
    let (admin, token) = person(&app, "Bartek Admin", "admin@example.com", Role::Admin);
    seed_demo(&app, admin.id).expect("the demo data");
    let router = Router::new()
        .nest("/api/m/reports", ReportsModule.routes().expect("the module has routes"))
        .with_state(app.clone());

    Demo { app, router, admin, token }
}

async fn send(router: &Router, token: Option<&str>, path: &str) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(format!("/api/m/reports{path}"));

    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }

    let response = router
        .clone()
        .oneshot(request.body(Body::empty()).expect("a valid request"))
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response.into_body().collect().await.expect("a readable body").to_bytes();

    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl Demo {
    async fn get(&self, path: &str) -> Value {
        let (status, value) = send(&self.router, Some(&self.token), path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {value}");

        value
    }
}

fn days(report: &Value) -> &Vec<Value> {
    report["days"].as_array().expect("days")
}

fn total(report: &Value, field: &str) -> f64 {
    days(report).iter().map(|day| day[field].as_f64().expect("a number")).sum()
}

fn close(value: &Value, expected: f64) -> bool {
    value.as_f64().is_some_and(|value| (value - expected).abs() < 0.011)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_running_sprint_committed_its_nine_stories_35_points_and_has_burned_the_5_of_the_two_that_are_done() {
    let demo = demo();
    let report = demo.get("/projects/WEB/burndown").await;
    let (first, last) = (&days(&report)[0], &days(&report)[5]);

    assert_eq!(report["sprint"]["name"], "WEB Sprint 2");
    assert_eq!(report["sprint"]["state"], "active");
    assert_eq!(report["unit"], "points");
    assert_eq!(report["committed"], 35.0);
    assert_eq!(days(&report).len(), 6, "the sprint started five days ago and runs until today");
    assert_eq!(first["date"], report["sprint"]["startDate"]);
    assert_eq!(
        (&first["remaining"], &first["completed"], &first["ideal"], &first["added"]),
        (&35.0.into(), &0.0.into(), &35.0.into(), &0.0.into())
    );
    assert_eq!((&last["remaining"], &last["completed"]), (&30.0.into(), &5.0.into()));
    assert_eq!(last["ideal"], 22.5, "five of the fourteen steps from 35 down to zero");
    assert_eq!((total(&report, "added"), total(&report, "removed")), (0.0, 0.0));
}

#[tokio::test(flavor = "multi_thread")]
async fn counted_in_issues_the_running_sprint_has_its_nine_stories_and_three_subtasks_of_which_three_are_done() {
    let demo = demo();
    let report = demo.get("/projects/web/burndown?unit=issues").await;
    let last = days(&report).last().expect("today");

    assert_eq!(report["unit"], "issues");
    assert_eq!(report["committed"], 12.0);
    assert_eq!((&last["remaining"], &last["completed"]), (&9.0.into(), &3.0.into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn sprint_1_committed_15_points_took_on_a_2_point_bug_three_days_in_and_completed_all_17() {
    let demo = demo();
    let velocity = demo.get("/projects/WEB/velocity").await;
    let sprint = &velocity["sprints"][0];

    assert_eq!(velocity["sprints"].as_array().map(Vec::len), Some(1), "only sprint 1 has been completed");
    assert_eq!(sprint["sprint"]["name"], "WEB Sprint 1");
    assert_eq!((&sprint["committedPoints"], &sprint["completedPoints"]), (&15.0.into(), &17.0.into()));
    assert_eq!((&sprint["committedIssues"], &sprint["completedIssues"]), (&4.into(), &5.into()));
    assert_eq!(velocity["averageCompletedPoints"], 17.0);

    let report = demo.get(&format!("/projects/WEB/burndown?sprint={}", sprint["sprint"]["id"])).await;
    let (first, last) = (&days(&report)[0], days(&report).last().expect("the day of completion"));
    let ideals: Vec<f64> = days(&report).iter().map(|day| day["ideal"].as_f64().expect("a number")).collect();

    assert_eq!(report["unit"], "points");
    assert_eq!(report["committed"], 15.0);
    assert_eq!(first["date"], report["sprint"]["startDate"]);
    assert_eq!((&first["remaining"], &first["ideal"]), (&15.0.into(), &15.0.into()));
    assert_eq!(last["date"].as_str(), report["sprint"]["completedAt"].as_str().map(|at| &at[..10]));
    assert_eq!((&last["remaining"], &last["completed"]), (&0.0.into(), &17.0.into()));
    assert_eq!((total(&report, "added"), total(&report, "removed")), (2.0, 0.0));
    assert!(
        ideals.windows(2).all(|pair| pair[0] > pair[1]),
        "the ideal line only goes down: {ideals:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn of_the_26_web_issues_that_are_not_epics_8_are_done_6_in_progress_and_12_still_to_do() {
    let demo = demo();
    let flow = demo.get("/projects/WEB/cumulative-flow").await;
    let (first, last) = (&days(&flow)[0], &days(&flow)[29]);
    let totals: Vec<i64> = days(&flow)
        .iter()
        .map(|day| ["todo", "inProgress", "done"].iter().filter_map(|field| day[field].as_i64()).sum())
        .collect();

    assert_eq!(days(&flow).len(), 30);
    assert_eq!((&last["todo"], &last["inProgress"], &last["done"]), (&12.into(), &6.into(), &8.into()));
    assert_eq!(
        (&first["todo"], &first["inProgress"], &first["done"]),
        (&0.into(), &0.into(), &0.into()),
        "the first story is 24 days old"
    );
    assert!(totals.windows(2).all(|pair| pair[0] <= pair[1]), "issues are only ever added: {totals:?}");

    let pace = demo.get("/projects/WEB/created-vs-resolved?days=30").await;
    let today = &days(&pace)[29];

    assert_eq!((&pace["created"], &pace["resolved"]), (&26.into(), &8.into()));
    assert_eq!((&today["createdTotal"], &today["resolvedTotal"]), (&26.into(), &8.into()));
    assert_eq!(total(&pace, "created"), 26.0);
    assert_eq!(days(&demo.get("/projects/WEB/created-vs-resolved?days=7").await).len(), 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_eight_resolved_web_issues_took_3_to_16_days_from_creation_and_2_to_9_from_the_start_of_work() {
    let demo = demo();
    let report = demo.get("/projects/WEB/cycle-time").await;
    let issues = report["issues"].as_array().expect("issues");
    let keys: Vec<&str> = issues.iter().filter_map(|issue| issue["key"].as_str()).collect();
    let login = issues.iter().find(|issue| issue["key"] == "WEB-8").expect("WEB-8 was resolved");

    assert_eq!(
        keys,
        ["WEB-7", "WEB-4", "WEB-5", "WEB-6", "WEB-8", "WEB-14", "WEB-27", "WEB-11"],
        "in the order they were resolved"
    );
    assert!(
        close(&login["leadDays"], 16.0) && close(&login["cycleDays"], 9.0),
        "created 24 days ago, started 17 and done 8 days ago: {login}"
    );
    assert_eq!(login["summary"], "Rejestracja i logowanie e-mailem");
    assert!(login["resolvedAt"].as_str().is_some_and(|at| at.ends_with('Z')));

    let lead = &report["lead"];
    let cycle = &report["cycle"];

    assert!(
        close(&lead["p50"], 7.0) && close(&lead["p85"], 12.0) && close(&lead["p95"], 16.0),
        "leads are 3, 4, 6, 7, 9, 9, 12, 16 days: {lead}"
    );
    assert!(
        close(&cycle["p50"], 3.0) && close(&cycle["p85"], 6.0) && close(&cycle["p95"], 9.0),
        "cycles are 2, 2, 2, 3, 3, 4, 6, 9 days: {cycle}"
    );
    assert_eq!(
        demo.get("/projects/WEB/cycle-time?days=1").await["issues"],
        Value::Array(Vec::new()),
        "nothing was resolved since yesterday"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn open_web_work_is_spread_over_five_people_and_eight_unassigned_issues_worth_29_points() {
    let demo = demo();
    let report = demo.get("/projects/WEB/workload").await;
    let rows: Vec<(&str, i64, f64, i64)> = report["assignees"]
        .as_array()
        .expect("assignees")
        .iter()
        .map(|row| {
            (
                row["name"].as_str().expect("a name"),
                row["openIssues"].as_i64().expect("a count"),
                row["openPoints"].as_f64().expect("points"),
                row["inProgress"].as_i64().expect("a count"),
            )
        })
        .collect();

    assert_eq!(
        rows,
        [
            ("Anna Nowak", 3, 11.0, 2),
            ("Tomasz Kowalczyk", 3, 5.0, 0),
            ("Piotr Zieliński", 2, 6.0, 2),
            ("Bartek Admin", 1, 5.0, 1),
            ("Maria Wiśniewska", 1, 5.0, 1),
            ("Unassigned", 8, 29.0, 0),
        ]
    );
    assert_eq!(report["assignees"][0]["assigneeId"].as_i64().map(|id| id > demo.admin.id), Some(true));
    assert!(report["assignees"][5]["assigneeId"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_kanban_project_has_no_sprints_one_finished_story_and_two_issues_in_progress() {
    let demo = demo();
    let burndown = demo.get("/projects/API/burndown").await;
    let velocity = demo.get("/projects/API/velocity?sprints=3").await;
    let flow = demo.get("/projects/API/cumulative-flow?days=10").await;
    let cycle = demo.get("/projects/API/cycle-time").await;
    let today = days(&flow).last().expect("today");

    assert!(burndown["sprint"].is_null());
    assert_eq!(burndown["days"], Value::Array(Vec::new()));
    assert_eq!(velocity["sprints"], Value::Array(Vec::new()));
    assert_eq!(velocity["averageCompletedPoints"], 0.0);
    assert_eq!(days(&flow).len(), 10);
    assert_eq!((&today["todo"], &today["inProgress"], &today["done"]), (&4.into(), &2.into(), &1.into()));
    assert_eq!(cycle["issues"].as_array().map(Vec::len), Some(1));
    assert_eq!(cycle["issues"][0]["key"], "API-2");
    assert!(
        close(&cycle["issues"][0]["leadDays"], 11.0) && close(&cycle["issues"][0]["cycleDays"], 5.0),
        "created 18, started 12, done 7 days ago"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_need_a_sign_in_a_project_the_caller_may_see_and_sensible_parameters() {
    let demo = demo();
    let (_, outsider) = person(&demo.app, "Olga Outside", "olga@example.com", Role::Member);
    let private = NewProject {
        key: "SEC".to_string(),
        name: "Secret".to_string(),
        description: String::new(),
        board_mode: None,
        is_private: true,
        lead_id: None,
    };
    demo.app
        .write(demo.admin.actor(), |tx| create_project(tx, &demo.admin, &private))
        .expect("a private project");

    assert_eq!(send(&demo.router, None, "/projects/WEB/velocity").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&demo.router, Some(&outsider), "/projects/WEB/velocity").await.0, StatusCode::OK);
    assert_eq!(send(&demo.router, Some(&demo.token), "/projects/SEC/workload").await.0, StatusCode::OK);

    for report in ["burndown", "velocity", "cumulative-flow", "created-vs-resolved", "cycle-time", "workload"] {
        assert_eq!(
            send(&demo.router, Some(&outsider), &format!("/projects/SEC/{report}")).await.0,
            StatusCode::NOT_FOUND,
            "{report}"
        );
        assert_eq!(
            send(&demo.router, Some(&demo.token), &format!("/projects/NOPE/{report}")).await.0,
            StatusCode::NOT_FOUND,
            "{report}"
        );
    }

    assert_eq!(
        send(&demo.router, Some(&demo.token), "/projects/WEB/burndown?unit=hours").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&demo.router, Some(&demo.token), "/projects/WEB/burndown?sprint=9999").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(days(&demo.get("/projects/WEB/cumulative-flow?days=100000").await).len(), 366);
}
