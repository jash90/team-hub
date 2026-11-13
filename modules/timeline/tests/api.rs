mod common;

use axum::http::{Method, StatusCode};
use common::{date, epic, events, found, issue, issue_at, member, noon, person, project, sprint, test_app, transition_at, Client, Fields};
use hub_app::access::project_access;
use hub_app::seed::seed_demo;
use hub_app::services::issues::{delete_issue, issue_detail, issue_id_by_key};
use hub_app::services::project_modules::set_project_module;
use hub_app::services::versions::{create_version, update_version, VersionInput};
use hub_app::{App, Module};
use hub_core::{ProjectRole, Role};
use hub_mod_timeline::TimelineModule;
use serde_json::{json, Value};

struct Hub {
    app: App,
    admin: Client,
}

/// A project WEB with an administrator in it.
fn hub() -> Hub {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    project(&app, &admin, "WEB", false);

    Hub { app, admin }
}

fn keys(list: &Value) -> Vec<&str> {
    list.as_array().expect("a list").iter().map(|item| item["key"].as_str().expect("a key")).collect()
}

fn epic_of<'a>(timeline: &'a Value, key: &str) -> &'a Value {
    timeline["epics"].as_array().expect("epics").iter().find(|epic| epic["key"] == key).unwrap_or_else(|| panic!("{key} is not on the timeline"))
}

/// `(start, due, start is estimated, due is estimated)` of a row.
fn span(row: &Value) -> (Option<&str>, Option<&str>, bool, bool) {
    (row["start"].as_str(), row["due"].as_str(), row["derived"]["start"] == true, row["derived"]["due"] == true)
}

async fn set(client: &Client, key: &str, start: Option<&str>, due: Option<&str>) -> Value {
    client.put(&format!("/issues/{key}/span"), json!({ "start": start, "due": due })).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_gets_the_epics_with_their_children_progress_sprints_and_versions_in_one_answer() {
    let Hub { app, admin } = hub();
    let running = sprint(&app, &admin, "WEB", "Sprint 1", Some(date(-3)), Some(date(11)));
    let later = sprint(&app, &admin, "WEB", "Sprint 2", None, None);
    let checkout = epic(&app, &admin, "WEB", "Checkout");
    let account = epic(&app, &admin, "WEB", "Account");
    let cart = issue(
        &app,
        &admin,
        "WEB",
        "Cart",
        Fields { kind: "Story", parent: Some(&checkout), sprint: Some(running), points: Some(5.0), ..Default::default() },
    );
    let card =
        issue(&app, &admin, "WEB", "Card payments", Fields { kind: "Story", parent: Some(&checkout), points: Some(8.0), ..Default::default() });
    let blik = issue(&app, &admin, "WEB", "BLIK", Fields { parent: Some(&checkout), ..Default::default() });
    issue(&app, &admin, "WEB", "Not in an epic", Fields::default());
    transition_at(&app, &admin, &cart, "Done", common::now());
    transition_at(&app, &admin, &card, "In Progress", common::now());
    app.write(admin.user.actor(), |tx| {
        let access = project_access(tx, &admin.user, "WEB")?;
        create_version(tx, &access, &VersionInput { name: Some("1.0".to_string()), release_date: Some(date(20)), ..Default::default() })?;
        let old = create_version(tx, &access, &VersionInput { name: Some("0.1".to_string()), ..Default::default() })?;
        update_version(tx, &admin.user, old.id, &VersionInput { archived: Some(true), ..Default::default() })?;

        Ok(())
    })
    .expect("versions");
    set(&admin, &checkout, Some("2026-03-02"), Some("2026-04-17")).await;

    let timeline = admin.get("/projects/web").await;

    assert_eq!(timeline["projectKey"], "WEB");
    assert_eq!(timeline["canEdit"], true);
    assert_eq!(keys(&timeline["epics"]), [checkout.as_str(), account.as_str()], "epics only, in rank order");

    let first = epic_of(&timeline, &checkout);
    assert_eq!(first["summary"], "Checkout");
    assert_eq!(first["status"]["name"], "To Do");
    assert_eq!(first["status"]["category"], "todo");
    assert!(first["status"]["id"].is_i64() && first["id"].is_i64() && first["assigneeId"].is_null());
    assert_eq!(span(first), (Some("2026-03-02"), Some("2026-04-17"), false, false));
    assert_eq!(first["counts"], json!({ "total": 3, "todo": 1, "inProgress": 1, "done": 1 }));
    assert_eq!((&first["pointsDone"], &first["pointsTotal"]), (&json!(5.0), &json!(13.0)));
    assert_eq!(keys(&first["children"]), [cart.as_str(), card.as_str(), blik.as_str()]);
    let second = &first["children"][1];
    assert_eq!(
        (&second["key"], &second["summary"], &second["status"]["name"], &second["status"]["category"]),
        (&json!(card), &json!("Card payments"), &json!("In Progress"), &json!("in_progress"))
    );
    assert!(second["id"].is_i64() && second["assigneeId"].is_null() && second["start"].is_null() && second["due"].is_null());

    let empty = epic_of(&timeline, &account);
    assert_eq!(span(empty), (None, None, false, false), "an epic without dates or children has nothing to estimate from");
    assert_eq!(empty["counts"], json!({ "total": 0, "todo": 0, "inProgress": 0, "done": 0 }));
    assert_eq!(empty["children"], json!([]));

    assert_eq!(
        timeline["sprints"],
        json!([
            { "id": running, "name": "Sprint 1", "state": "future", "start": date(-3), "end": date(11) },
            { "id": later, "name": "Sprint 2", "state": "future", "start": null, "end": null },
        ])
    );
    assert_eq!(timeline["versions"].as_array().map(Vec::len), Some(1), "the archived version stays out");
    assert_eq!(
        (&timeline["versions"][0]["name"], &timeline["versions"][0]["released"], &timeline["versions"][0]["releaseDate"]),
        (&json!("1.0"), &json!(false), &json!(date(20)))
    );

    assert_eq!(admin.refused(Method::GET, "/projects/NOPE", None).await.0, StatusCode::NOT_FOUND);
    let stranger = Client { bearer: None, ..admin.clone() };
    assert_eq!(stranger.refused(Method::GET, "/projects/WEB", None).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_epic_without_dates_gets_them_from_its_children_and_says_so() {
    let Hub { app, admin } = hub();
    let planned = sprint(&app, &admin, "WEB", "Sprint 1", Some(date(-10)), Some(date(4)));
    let story = |parent: &str, day: i64, fields: Fields| issue_at(&app, &admin, "WEB", "Work", noon(day), Fields { parent: Some(parent), ..fields });

    // Nothing has begun: the epic starts when its first issue was written down, and has no end.
    let idea = epic(&app, &admin, "WEB", "Idea");
    story(&idea, -40, Fields::default());
    story(&idea, -30, Fields::default());

    // Work has begun: the first day an issue was in progress counts, not the day it was created.
    let begun = epic(&app, &admin, "WEB", "Begun");
    let first = story(&begun, -40, Fields::default());
    let second = story(&begun, -35, Fields { due: Some(date(25)), ..Default::default() });
    transition_at(&app, &admin, &first, "In Progress", noon(-20));
    transition_at(&app, &admin, &first, "Done", noon(-12));
    transition_at(&app, &admin, &second, "In Progress", noon(-15));

    // A dated sprint says more than either, and its end counts towards the end.
    let sprinted = epic(&app, &admin, "WEB", "Sprinted");
    let in_sprint = story(&sprinted, -40, Fields { sprint: Some(planned), ..Default::default() });
    transition_at(&app, &admin, &in_sprint, "In Progress", noon(-30));
    transition_at(&app, &admin, &in_sprint, "Done", noon(-2));

    // A date that is set stays; only the missing end is estimated, and never before the start.
    let half = epic(&app, &admin, "WEB", "Half planned");
    story(&half, -40, Fields { due: Some(date(7)), ..Default::default() });
    set(&admin, &half, Some(&date(-5)), None).await;
    let late = epic(&app, &admin, "WEB", "Starts late");
    story(&late, -40, Fields { due: Some(date(7)), ..Default::default() });
    set(&admin, &late, Some(&date(30)), None).await;

    let timeline = admin.get("/projects/WEB").await;

    assert_eq!(span(epic_of(&timeline, &idea)), (Some(date(-40).as_str()), None, true, false));
    assert_eq!(span(epic_of(&timeline, &begun)), (Some(date(-20).as_str()), Some(date(25).as_str()), true, true));
    assert_eq!(span(epic_of(&timeline, &sprinted)), (Some(date(-10).as_str()), Some(date(4).as_str()), true, true));
    assert_eq!(span(epic_of(&timeline, &half)), (Some(date(-5).as_str()), Some(date(7).as_str()), false, true));
    assert_eq!(span(epic_of(&timeline, &late)), (Some(date(30).as_str()), Some(date(30).as_str()), false, true));

    // Without a due date or a sprint, the day the last issue was resolved is the end.
    app.write(admin.user.actor(), |tx| Ok(tx.execute("UPDATE issue SET due_date = NULL WHERE key = ?1", [&second])?)).expect("the due date is gone");
    assert_eq!(span(epic_of(&admin.get("/projects/WEB").await, &begun)), (Some(date(-20).as_str()), Some(date(-12).as_str()), true, true));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_span_is_set_changed_and_cleared_and_every_change_is_announced() {
    let Hub { app, admin } = hub();
    let checkout = epic(&app, &admin, "WEB", "Checkout");
    let cart = issue(&app, &admin, "WEB", "Cart", Fields { parent: Some(&checkout), ..Default::default() });

    let row = set(&admin, &checkout, Some("2026-03-02"), Some("2026-04-17")).await;
    assert_eq!(row["key"], checkout);
    assert_eq!(span(&row), (Some("2026-03-02"), Some("2026-04-17"), false, false));
    assert_eq!(keys(&row["children"]), [cart.as_str()], "the answer is the epic's row as the page gets it");
    assert_eq!(row, *epic_of(&admin.get("/projects/WEB").await, &checkout));

    let moved = set(&admin, &checkout, Some("2026-03-09"), Some("2026-04-24")).await;
    assert_eq!(span(&moved), (Some("2026-03-09"), Some("2026-04-24"), false, false));

    // The same dates again change nothing and announce nothing.
    set(&admin, &checkout, Some("2026-03-09"), Some("2026-04-24")).await;
    assert_eq!(events(&app, "timeline.").len(), 2);

    let open_ended = set(&admin, &checkout, Some("2026-03-09"), None).await;
    assert_eq!(span(&open_ended), (Some("2026-03-09"), None, false, false), "nothing under the epic has a date to estimate the end from");

    // Cleared, the epic falls back on what its one issue says: it was written down today.
    let cleared = set(&admin, &checkout, None, None).await;
    assert_eq!(span(&cleared), (Some(date(0).as_str()), None, true, false));
    let stored: i64 = app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM timeline_span", [], |row| row.get(0))?)).expect("a count");
    assert_eq!(stored, 0);

    // Any issue may have a span; under its epic it shows with its own dates.
    let child = set(&admin, &cart, Some("2026-03-10"), Some("2026-03-20")).await;
    assert_eq!(span(&child), (Some("2026-03-10"), Some("2026-03-20"), false, false));
    let epic_row = admin.get("/projects/WEB").await;
    let under = &epic_of(&epic_row, &checkout)["children"][0];
    assert_eq!((under["start"].as_str(), under["due"].as_str()), (Some("2026-03-10"), Some("2026-03-20")));

    let announced = events(&app, "timeline.");
    assert_eq!(announced.len(), 5);
    assert!(announced.iter().all(|(kind, _)| kind == "timeline.updated"));
    assert_eq!(announced.last(), Some(&("timeline.updated".to_string(), Some(cart.clone()))));
    let project_of_event: Option<i64> = app
        .read(|connection| Ok(connection.query_row("SELECT project_id FROM event WHERE kind = 'timeline.updated' LIMIT 1", [], |row| row.get(0))?))
        .expect("an event");
    assert!(project_of_event.is_some(), "the event names its project, so only people who see the project hear it");

    // A deleted issue takes its start with it.
    app.write(admin.user.actor(), |tx| {
        let (id, _) = issue_id_by_key(tx, &cart)?.expect("the issue");

        delete_issue(tx, id).map(|_| ())
    })
    .expect("the issue is deleted");
    let left: i64 = app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM timeline_span", [], |row| row.get(0))?)).expect("a count");
    assert_eq!(left, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn dates_that_make_no_sense_are_refused_and_nothing_is_stored() {
    let Hub { app, admin } = hub();
    let checkout = epic(&app, &admin, "WEB", "Checkout");
    let path = format!("/issues/{checkout}/span");
    set(&admin, &checkout, Some("2026-03-02"), Some("2026-04-17")).await;

    let (status, message) = admin.refused(Method::PUT, &path, Some(json!({ "start": "next week", "due": null }))).await;
    assert_eq!((status, message.as_str()), (StatusCode::BAD_REQUEST, "\"next week\" is not a date (expected YYYY-MM-DD)"));
    assert_eq!(admin.refused(Method::PUT, &path, Some(json!({ "start": null, "due": "2026-02-30" }))).await.0, StatusCode::BAD_REQUEST);

    let (status, message) = admin.refused(Method::PUT, &path, Some(json!({ "start": "2026-05-01", "due": "2026-04-30" }))).await;
    assert_eq!((status, message.as_str()), (StatusCode::BAD_REQUEST, "The start date is after the due date"));

    // A date that is left out is not a date to clear: both ends are stated, or the request is refused.
    assert_eq!(admin.refused(Method::PUT, &path, Some(json!({ "start": "2026-03-09" }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(admin.refused(Method::PUT, "/issues/WEB-999/span", Some(json!({ "start": null, "due": null }))).await.0, StatusCode::NOT_FOUND);

    // One day is a span too.
    assert_eq!(span(&set(&admin, &checkout, Some("2026-03-02"), Some("2026-03-02")).await), (Some("2026-03-02"), Some("2026-03-02"), false, false));
    assert_eq!(events(&app, "timeline.").len(), 2, "a refused request announces nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn viewers_read_members_edit_and_a_private_project_does_not_exist_for_outsiders() {
    let Hub { app, admin } = hub();
    let viewer = person(&app, "Wanda Viewer", Role::Member);
    let editor = person(&app, "Marek Member", Role::Member);
    let outsider = person(&app, "Olga Outsider", Role::Member);
    project(&app, &admin, "SEC", true);
    member(&app, &admin, "SEC", &viewer, ProjectRole::Viewer);
    member(&app, &admin, "SEC", &editor, ProjectRole::Member);
    let secret = epic(&app, &admin, "SEC", "Secret launch");
    let path = format!("/issues/{secret}/span");
    let dates = json!({ "start": "2026-03-02", "due": "2026-04-17" });

    assert_eq!(span(&editor.put(&path, dates.clone()).await), (Some("2026-03-02"), Some("2026-04-17"), false, false));
    assert_eq!(editor.get("/projects/SEC").await["canEdit"], true);

    let seen = viewer.get("/projects/SEC").await;
    assert_eq!(seen["canEdit"], false);
    assert_eq!(span(epic_of(&seen, &secret)), (Some("2026-03-02"), Some("2026-04-17"), false, false));
    let (status, message) = viewer.refused(Method::PUT, &path, Some(json!({ "start": null, "due": null }))).await;
    assert_eq!((status, message.as_str()), (StatusCode::FORBIDDEN, "You can only view this project"));

    assert_eq!(outsider.refused(Method::GET, "/projects/SEC", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(outsider.refused(Method::PUT, &path, Some(dates)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        span(epic_of(&admin.get("/projects/SEC").await, &secret)),
        (Some("2026-03-02"), Some("2026-04-17"), false, false),
        "nobody's refusal changed anything"
    );

    // In an open project every signed-in person is a member.
    let open = epic(&app, &admin, "WEB", "Open work");
    assert_eq!(outsider.put(&format!("/issues/{open}/span"), json!({ "start": "2026-03-02", "due": null })).await["start"], "2026-03-02");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_of_the_due_date_is_in_the_history_of_the_issue() {
    let Hub { app, admin } = hub();
    let editor = person(&app, "Marek Member", Role::Member);
    let checkout = epic(&app, &admin, "WEB", "Checkout");

    set(&editor, &checkout, Some("2026-03-02"), Some("2026-04-17")).await;
    set(&editor, &checkout, Some("2026-03-09"), Some("2026-04-17")).await;
    set(&editor, &checkout, Some("2026-03-09"), None).await;

    let detail = issue_detail(&app, &admin.user, &checkout).expect("the issue");
    let due: Vec<_> = detail.changes.iter().filter(|change| change.field == "due").collect();

    assert_eq!(detail.row.due_date, None);
    assert_eq!(due.len(), 2, "moving only the start is no change of the due date");
    assert_eq!((due[0].old_value.as_deref(), due[0].new_value.as_deref(), due[0].actor_id), (None, Some("2026-04-17"), Some(editor.user.id)));
    assert_eq!((due[1].old_value.as_deref(), due[1].new_value.as_deref()), (Some("2026-04-17"), None));
    assert_eq!(events(&app, "issue.updated").len(), 2, "the tracker announces the due date like any other change of it");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_issue_page_gets_the_span_of_an_epic_or_of_an_issue_with_a_start() {
    let Hub { app, admin } = hub();
    let checkout = epic(&app, &admin, "WEB", "Checkout");
    let cart = issue(&app, &admin, "WEB", "Cart", Fields { parent: Some(&checkout), due: Some(date(30)), ..Default::default() });
    let panel = |key: &str| issue_detail(&app, &admin.user, key).expect("the issue").panels.get("timeline").cloned();

    assert_eq!(panel(&checkout), Some(json!({ "start": date(0), "due": date(30), "derived": { "start": true, "due": true } })));
    assert_eq!(panel(&cart), None, "an issue that is not an epic and has no start has no panel");

    set(&admin, &cart, Some(&date(5)), Some(&date(30))).await;
    set(&admin, &checkout, Some(&date(-3)), None).await;

    assert_eq!(panel(&cart), Some(json!({ "start": date(5), "due": date(30), "derived": { "start": false, "due": false } })));
    assert_eq!(panel(&checkout), Some(json!({ "start": date(-3), "due": date(30), "derived": { "start": false, "due": true } })));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_query_language_finds_issues_by_their_start() {
    let Hub { app, admin } = hub();
    let early = epic(&app, &admin, "WEB", "Early");
    let late = epic(&app, &admin, "WEB", "Late");
    let soon = epic(&app, &admin, "WEB", "Soon");
    let unplanned = epic(&app, &admin, "WEB", "Unplanned");
    set(&admin, &early, Some("2026-01-05"), None).await;
    set(&admin, &late, Some("2026-06-01"), Some("2026-07-01")).await;
    set(&admin, &soon, Some(&date(3)), None).await;
    let all = |keys: &[&String]| keys.iter().map(|key| key.to_string()).collect::<Vec<_>>();

    assert_eq!(found(&app, &admin, "start >= 2026-06-01 AND start < 2026-07-01"), all(&[&late]));
    assert_eq!(found(&app, &admin, "start = 2026-01-05"), all(&[&early]));
    assert_eq!(found(&app, &admin, "start <= 2026-01-05"), all(&[&early]));
    assert_eq!(found(&app, &admin, "start > 2026-01-05 AND start < 2026-06-02"), all(&[&late]));
    assert_eq!(found(&app, &admin, "startDate is empty"), all(&[&unplanned]));
    assert_eq!(found(&app, &admin, "start is not empty").len(), 3);
    assert_eq!(found(&app, &admin, "start != 2026-01-05").len(), 3, "an issue without a start does not start that day either");
    assert_eq!(found(&app, &admin, "start > now() AND start <= 1w"), all(&[&soon]), "relative dates, as for the due date");
    assert_eq!(found(&app, &admin, "start >= startOfMonth() AND start < 60d AND due is empty"), all(&[&soon]));

    let refused = hub_app::services::search::search(&app, &admin.user, "start in (2026-01-05)", 10, 0).expect("an answer").error.expect("an error");
    assert_eq!(refused.message, "That operator cannot be used with \"start\"");
    let refused = hub_app::services::search::search(&app, &admin.user, "start > someday", 10, 0).expect("an answer").error.expect("an error");
    assert_eq!(refused.message, "\"start\" needs a date such as 2026-11-30, -7d or startOfWeek()");

    let fields = TimelineModule.query_fields();
    assert_eq!((fields.len(), fields[0].name.as_str(), fields[0].kind.as_str()), (1, "start", "date"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reads_the_timeline_with_one_tool() {
    let Hub { app, admin } = hub();
    let outsider = person(&app, "Olga Outsider", Role::Member);
    project(&app, &admin, "SEC", true);
    sprint(&app, &admin, "WEB", "Sprint 1", Some(date(-3)), Some(date(11)));
    let checkout = epic(&app, &admin, "WEB", "Checkout");
    let cart = issue(
        &app,
        &admin,
        "WEB",
        "Cart",
        Fields { parent: Some(&checkout), points: Some(3.0), due: Some("2026-04-01".to_string()), ..Default::default() },
    );
    set(&admin, &checkout, Some("2026-03-02"), None).await;
    let call = |who: &Client, arguments: Value| {
        TimelineModule.call_tool(&app, &who.user.through("timeline"), "get_timeline", &arguments).expect("the tool is the module's")
    };

    let tools = TimelineModule.tools();
    assert_eq!((tools.len(), tools[0].name, tools[0].read_only), (1, "get_timeline", true));
    assert_eq!(tools[0].input_schema["required"], json!(["project"]));

    let timeline = call(&admin, json!({ "project": "web" })).expect("the timeline");
    assert_eq!(timeline["projectKey"], "WEB");
    assert_eq!(
        timeline["epics"],
        json!([{
            "key": checkout, "summary": "Checkout", "status": "To Do", "category": "todo", "assigneeId": null,
            "start": "2026-03-02", "due": "2026-04-01", "estimated": ["due"],
            "issues": { "todo": 1, "inProgress": 0, "done": 0 },
            "points": { "done": 0.0, "total": 3.0 },
            "children": [{ "key": cart, "summary": "Cart", "status": "To Do", "start": null, "due": "2026-04-01" }],
        }])
    );
    assert_eq!(timeline["sprints"], json!([{ "name": "Sprint 1", "state": "future", "start": date(-3), "end": date(11) }]));
    assert_eq!(timeline["versions"], json!([]));

    assert_eq!(call(&admin, json!({})).expect_err("no project").message, "\"project\" is required");
    assert_eq!(call(&outsider, json!({ "project": "SEC" })).expect_err("a private project").status, StatusCode::NOT_FOUND);
    assert!(TimelineModule.call_tool(&app, &admin.user, "get_issue", &json!({})).is_none(), "other tools are not this module's");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_that_switched_the_module_off_does_not_exist_for_it() {
    let Hub { app, admin } = hub();
    project(&app, &admin, "API", false);
    let checkout = epic(&app, &admin, "WEB", "Checkout");
    let orders = epic(&app, &admin, "API", "Orders");
    set(&admin, &checkout, Some("2026-03-02"), Some("2026-04-17")).await;
    set(&admin, &orders, Some("2026-03-02"), Some("2026-04-17")).await;
    let switch = |enabled: bool| {
        app.write(admin.user.actor(), |tx| set_project_module(tx, &project_access(tx, &admin.user, "WEB")?, "timeline", enabled)).expect("the switch")
    };
    let tool = |key: &str| {
        TimelineModule.call_tool(&app, &admin.user.through("timeline"), "get_timeline", &json!({ "project": key })).expect("the module's tool")
    };
    let dates = json!({ "start": "2026-05-04", "due": "2026-05-29" });

    switch(false);

    assert_eq!(admin.refused(Method::GET, "/projects/WEB", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(admin.refused(Method::PUT, &format!("/issues/{checkout}/span"), Some(dates.clone())).await.0, StatusCode::NOT_FOUND);
    assert_eq!(tool("WEB").expect_err("the project is not there").status, StatusCode::NOT_FOUND);
    assert_eq!(found(&app, &admin, "start is not empty"), [orders.as_str()], "the field matches nothing in that project");
    assert_eq!(issue_detail(&app, &admin.user, &checkout).expect("the issue").panels.get("timeline"), None);

    // The other project is untouched, and so is what the first one had stored.
    assert_eq!(keys(&admin.get("/projects/API").await["epics"]), [orders.as_str()]);
    assert!(tool("API").is_ok());

    switch(true);

    assert_eq!(span(epic_of(&admin.get("/projects/WEB").await, &checkout)), (Some("2026-03-02"), Some("2026-04-17"), false, false));
    assert_eq!(admin.put(&format!("/issues/{checkout}/span"), dates).await["start"], "2026-05-04");
    assert_eq!(found(&app, &admin, "start is not empty").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demo_has_epics_planned_around_today() {
    let app = test_app();
    let admin = person(&app, "Bartek Admin", Role::Admin);
    seed_demo(&app, admin.user.id).expect("the demo data");

    let web = admin.get("/projects/WEB").await;
    assert_eq!(keys(&web["epics"]), ["WEB-1", "WEB-2", "WEB-3"]);
    assert_eq!(span(epic_of(&web, "WEB-1")), (Some(date(-20).as_str()), Some(date(16).as_str()), false, false));
    assert_eq!(span(epic_of(&web, "WEB-2")), (Some(date(-17).as_str()), Some(date(30).as_str()), false, false));
    assert_eq!(span(epic_of(&web, "WEB-3")), (Some(date(10).as_str()), Some(date(45).as_str()), false, false));

    // WEB-1 has ten issues; the first sprint's three and the fixed bug of the second are done.
    let checkout = epic_of(&web, "WEB-1");
    assert_eq!(checkout["counts"], json!({ "total": 10, "todo": 2, "inProgress": 4, "done": 4 }));
    assert_eq!((&checkout["pointsDone"], &checkout["pointsTotal"]), (&json!(12.0), &json!(39.0)));
    let card = checkout["children"].as_array().expect("children").iter().find(|child| child["key"] == "WEB-9").expect("WEB-9 is under WEB-1");
    assert_eq!(
        (card["start"].as_str(), card["due"].as_str()),
        (Some(date(-5).as_str()), Some(date(3).as_str())),
        "a few issues have a span of their own"
    );
    assert_eq!(web["sprints"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        (&web["sprints"][1]["state"], &web["sprints"][1]["start"], &web["sprints"][1]["end"]),
        (&json!("active"), &json!(date(-5)), &json!(date(9)))
    );
    assert_eq!(web["versions"].as_array().map(Vec::len), Some(3));

    // The epic of the API project was left alone: its span is read off its issues, from the
    // first one in progress twelve days ago to the one resolved a week ago. The demo's moves
    // happen a few hours before "now", which may be the day before.
    let api = admin.get("/projects/API").await;
    let (start, due, start_estimated, due_estimated) = span(epic_of(&api, "API-1"));
    assert!(start_estimated && due_estimated);
    assert!([date(-13), date(-12)].iter().any(|day| Some(day.as_str()) == start), "{start:?}");
    assert!([date(-8), date(-7)].iter().any(|day| Some(day.as_str()) == due), "{due:?}");

    let history = issue_detail(&app, &admin.user, "WEB-1").expect("the epic");
    assert!(history.changes.iter().any(|change| change.field == "due" && change.new_value.as_deref() == Some(date(16).as_str())));
}
