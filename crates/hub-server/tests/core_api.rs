mod common;

use axum::http::{Method, StatusCode};
use common::{keys, test_app, Client};
use hub_server::build_router;
use serde_json::{json, Value};

async fn fresh() -> Client {
    let mut client = Client::new(build_router(test_app(Vec::new())));
    client.setup(false).await;

    client
}

fn id_named(items: &Value, name: &str) -> i64 {
    items.as_array().expect("an array").iter().find(|item| item["name"] == name).unwrap_or_else(|| panic!("no \"{name}\" in {items}"))["id"]
        .as_i64()
        .expect("an id")
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_works_once_and_signs_in() {
    let mut client = Client::new(build_router(test_app(Vec::new())));

    assert_eq!(client.get("/api/bootstrap").await["setupRequired"], true);
    assert_eq!(client.refused(Method::GET, "/api/projects", None).await, StatusCode::UNAUTHORIZED);

    client.setup(false).await;
    let bootstrap = client.get("/api/bootstrap").await;

    assert_eq!(bootstrap["setupRequired"], false);
    assert_eq!(bootstrap["user"]["role"], "admin");
    assert_eq!(
        client.refused(Method::POST, "/api/setup", Some(json!({ "name": "x", "email": "x@example.com", "password": "12345678" }))).await,
        StatusCode::CONFLICT
    );

    let mut stranger = Client::new(client.router.clone());
    assert_eq!(
        stranger.refused(Method::POST, "/api/login", Some(json!({ "email": "admin@example.com", "password": "wrong password" }))).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_without_its_csrf_token_cannot_write() {
    let mut client = fresh().await;
    client.csrf = None;

    assert_eq!(client.refused(Method::POST, "/api/projects", Some(json!({ "key": "WEB", "name": "Web" }))).await, StatusCode::FORBIDDEN);
    assert_eq!(client.get("/api/projects").await, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn issues_are_created_changed_and_remember_their_history() {
    let mut client = fresh().await;
    let project = client.post("/api/projects", json!({ "key": "web", "name": "Web shop" })).await;

    assert_eq!(project["key"], "WEB");
    assert_eq!(project["statuses"].as_array().unwrap().len(), 4);

    let bug = id_named(&project["types"], "Bug");
    let in_progress = id_named(&project["statuses"], "In Progress");
    let done = id_named(&project["statuses"], "Done");
    let me = client.get("/api/bootstrap").await["user"]["id"].as_i64().unwrap();

    let first = client
        .post("/api/issues", json!({ "projectKey": "WEB", "summary": "  Login   fails ", "typeId": bug, "labels": ["auth", "Auth", "safari bug"] }))
        .await;
    let second = client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Checkout page" })).await;

    assert_eq!(first["key"], "WEB-1");
    assert_eq!(first["summary"], "Login fails");
    assert_eq!(first["labels"], json!(["auth", "safari-bug"]));
    assert_eq!(first["reporterId"], me);
    assert_eq!(second["key"], "WEB-2");
    assert!(first["rank"].as_str().unwrap() < second["rank"].as_str().unwrap());

    let updated = client
        .patch("/api/issues/web-1", json!({ "statusId": in_progress, "assigneeId": me, "points": 3, "priority": 1, "dueDate": "2026-12-01" }))
        .await;

    assert_eq!(updated["statusId"], in_progress);
    assert_eq!(updated["points"], 3.0);
    assert!(updated["resolvedAt"].is_null());

    let resolved = client.patch("/api/issues/WEB-1", json!({ "statusId": done })).await;
    assert!(resolved["resolvedAt"].is_string());

    let cleared = client.patch("/api/issues/WEB-1", json!({ "assigneeId": null, "statusId": in_progress })).await;
    assert!(cleared["assigneeId"].is_null() && cleared["resolvedAt"].is_null());

    client.post("/api/issues/WEB-1/comments", json!({ "body": "Reproduced on Safari 18" })).await;
    client.post("/api/issues/WEB-1/links", json!({ "type": "blocks", "targetKey": "WEB-2" })).await;
    client.post("/api/issues/WEB-1/worklogs", json!({ "spent": "1h 30m", "note": "debugging" })).await;

    let detail = client.get("/api/issues/WEB-1").await;
    let fields: Vec<&str> = detail["changes"].as_array().unwrap().iter().map(|change| change["field"].as_str().unwrap()).collect();

    assert_eq!(fields, ["status", "priority", "assignee", "points", "due", "status", "status", "assignee", "link"]);
    assert_eq!(detail["changes"][0]["oldValue"], "To Do");
    assert_eq!(detail["changes"][0]["newValue"], "In Progress");
    assert_eq!(detail["comments"][0]["body"], "Reproduced on Safari 18");
    assert_eq!(detail["links"][0]["direction"], "outward");
    assert_eq!(detail["links"][0]["issue"]["key"], "WEB-2");
    assert_eq!(detail["spentMinutes"], 90);
    assert_eq!(detail["commentCount"], 1);
    assert_eq!(client.get("/api/issues/WEB-2").await["links"][0]["direction"], "inward");

    assert_eq!(client.refused(Method::PATCH, "/api/issues/WEB-1", Some(json!({ "summary": "  " }))).await, StatusCode::BAD_REQUEST);
    assert_eq!(client.refused(Method::GET, "/api/issues/WEB-99", None).await, StatusCode::NOT_FOUND);

    client.delete("/api/issues/WEB-2").await;
    assert_eq!(client.get("/api/issues/WEB-1").await["links"], json!([]));

    let third = client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Numbers are never reused" })).await;
    assert_eq!(third["key"], "WEB-3");
}

#[tokio::test(flavor = "multi_thread")]
async fn hierarchy_is_epic_then_issue_then_subtask() {
    let mut client = fresh().await;
    let project = client.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    let epic = id_named(&project["types"], "Epic");
    let subtask = id_named(&project["types"], "Subtask");

    client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Checkout", "typeId": epic })).await;
    let story = client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Cart", "parentKey": "WEB-1" })).await;
    let child = client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Cart API", "parentKey": "WEB-2" })).await;

    assert_eq!(story["parentKey"], "WEB-1");
    assert_eq!(child["typeId"], subtask, "an issue created under a story is a subtask");

    let refused = [
        json!({ "projectKey": "WEB", "summary": "Orphan", "typeId": subtask }),
        json!({ "projectKey": "WEB", "summary": "Under a subtask", "parentKey": "WEB-3" }),
        json!({ "projectKey": "WEB", "summary": "Nested epic", "typeId": epic, "parentKey": "WEB-1" }),
    ];

    for body in refused {
        assert_eq!(client.refused(Method::POST, "/api/issues", Some(body)).await, StatusCode::BAD_REQUEST);
    }

    assert_eq!(client.refused(Method::PATCH, "/api/issues/WEB-2", Some(json!({ "parentKey": "WEB-2" }))).await, StatusCode::BAD_REQUEST);

    let done = id_named(&project["statuses"], "Done");
    client.patch("/api/issues/WEB-3", json!({ "statusId": done })).await;

    assert_eq!(client.get("/api/issues/WEB-2").await["subtasks"], json!({ "total": 1, "done": 1 }));
    assert_eq!(keys(&client.get("/api/issues/WEB-1").await["children"]), ["WEB-2"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sprint_is_planned_run_and_completed() {
    let mut client = fresh().await;
    let project = client.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    let done = id_named(&project["statuses"], "Done");
    let in_progress = id_named(&project["statuses"], "In Progress");

    for summary in ["One", "Two", "Three", "Four"] {
        client.post("/api/issues", json!({ "projectKey": "WEB", "summary": summary })).await;
    }

    let sprint = client.post("/api/projects/WEB/sprints", json!({})).await;
    let next = client.post("/api/projects/WEB/sprints", json!({ "name": "Later" })).await;
    let (sprint_id, next_id) = (sprint["id"].as_i64().unwrap(), next["id"].as_i64().unwrap());

    assert_eq!(sprint["name"], "WEB Sprint 1");

    client.post("/api/issues/WEB-3/move", json!({ "sprintId": sprint_id })).await;
    client.post("/api/issues/WEB-1/move", json!({ "sprintId": sprint_id, "afterKey": "WEB-3" })).await;
    client.post("/api/issues/WEB-4/move", json!({ "sprintId": sprint_id, "beforeKey": "WEB-3" })).await;

    let backlog = client.get("/api/projects/WEB/backlog").await;

    assert_eq!(keys(&backlog["sprints"][0]["issues"]), ["WEB-4", "WEB-3", "WEB-1"], "the order follows the drags");
    assert_eq!(keys(&backlog["backlog"]), ["WEB-2"]);
    assert!(client.get("/api/projects/WEB/board").await["sprint"].is_null());

    client.post(&format!("/api/sprints/{sprint_id}/start"), json!({ "startDate": "2026-11-02", "endDate": "2026-11-16" })).await;
    assert_eq!(
        client.refused(Method::POST, &format!("/api/sprints/{next_id}/start"), Some(json!({}))).await,
        StatusCode::CONFLICT,
        "one sprint runs at a time"
    );

    client.post("/api/issues/WEB-4/move", json!({ "statusId": done })).await;
    client.post("/api/issues/WEB-3/move", json!({ "statusId": in_progress })).await;

    let board = client.get("/api/projects/WEB/board").await;
    let columns: Vec<(String, Vec<String>)> = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| (column["status"]["name"].as_str().unwrap().to_string(), keys(&column["issues"])))
        .collect();

    assert_eq!(board["sprint"]["state"], "active");
    assert_eq!(columns[0], ("To Do".to_string(), vec!["WEB-1".to_string()]));
    assert_eq!(columns[1].1, ["WEB-3"]);
    assert_eq!(columns[3].1, ["WEB-4"]);

    let completion = client.post(&format!("/api/sprints/{sprint_id}/complete"), json!({ "moveTo": next_id })).await;

    assert_eq!((completion["done"].as_i64(), completion["moved"].as_i64()), (Some(1), Some(2)));
    assert_eq!(client.get("/api/issues/WEB-4").await["sprintId"], sprint_id, "finished work stays in its sprint");
    assert_eq!(client.get("/api/issues/WEB-3").await["sprintId"], next_id);
    assert_eq!(client.refused(Method::DELETE, &format!("/api/sprints/{sprint_id}"), None).await, StatusCode::CONFLICT);
    assert_eq!(
        client.refused(Method::PATCH, "/api/issues/WEB-2", Some(json!({ "sprintId": sprint_id }))).await,
        StatusCode::CONFLICT,
        "a completed sprint takes no new issues"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn queries_filter_and_order() {
    let mut client = fresh().await;
    let project = client.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    client.post("/api/projects", json!({ "key": "API", "name": "Api" })).await;
    let bug = id_named(&project["types"], "Bug");
    let done = id_named(&project["statuses"], "Done");
    let in_progress = id_named(&project["statuses"], "In Progress");
    let me = client.get("/api/bootstrap").await["user"]["id"].as_i64().unwrap();
    let (_, anna) = client.member("Anna Nowak", "anna@example.com").await;

    client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Błąd logowania w Safari", "typeId": bug, "priority": 1, "assigneeId": me, "labels": ["auth"], "points": 2 })).await;
    client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Koszyk: żółta wstążka promocji", "priority": 4, "assigneeId": anna, "points": 8, "statusId": in_progress })).await;
    client
        .post("/api/issues", json!({ "projectKey": "WEB", "summary": "Stopka", "statusId": done, "description": "Zawiera link do logowania" }))
        .await;
    client
        .post("/api/issues", json!({ "projectKey": "API", "summary": "Rate limiting", "labels": ["auth", "backend"], "dueDate": "2020-01-01" }))
        .await;
    client.post("/api/issues/WEB-2/comments", json!({ "body": "Wstążka zasłania przycisk zamówienia" })).await;

    let cases: [(&str, &[&str]); 22] = [
        ("project = WEB ORDER BY key", &["WEB-1", "WEB-2", "WEB-3"]),
        ("project = web AND category != done ORDER BY key", &["WEB-1", "WEB-2"]),
        ("status = \"In Progress\"", &["WEB-2"]),
        ("type = Bug OR project = API ORDER BY key", &["API-1", "WEB-1"]),
        ("assignee = currentUser()", &["WEB-1"]),
        ("assignee = \"Anna Nowak\"", &["WEB-2"]),
        ("assignee = anna.nowak", &["WEB-2"]),
        ("assignee is empty ORDER BY key", &["API-1", "WEB-3"]),
        ("assignee != currentUser() ORDER BY key", &["API-1", "WEB-2", "WEB-3"]),
        ("label = auth ORDER BY key", &["API-1", "WEB-1"]),
        ("label in (backend, nothing)", &["API-1"]),
        ("label is empty ORDER BY key", &["WEB-2", "WEB-3"]),
        ("priority >= High", &["WEB-1"]),
        ("priority < Medium", &["WEB-2"]),
        ("points > 3", &["WEB-2"]),
        ("due < now()", &["API-1"]),
        ("created >= -1d AND project = API", &["API-1"]),
        ("text ~ logowania ORDER BY key", &["WEB-1", "WEB-3"]),
        ("summary ~ \"zolta wstazka\"", &["WEB-2"]),
        ("comment ~ zamówienia", &["WEB-2"]),
        ("NOT (project = WEB OR label = backend)", &[]),
        ("key in (WEB-3, api-1) ORDER BY priority DESC, key DESC", &["WEB-3", "API-1"]),
    ];

    for (query, expected) in cases {
        assert_eq!(client.search_keys(query).await, expected, "query: {query}");
    }

    assert_eq!(client.search_keys("logow").await.len(), 2, "plain words search the text");
    assert_eq!(client.search_keys("web-2").await, ["WEB-2"], "a bare key finds the issue");
    assert_eq!(client.search_keys("project = WEB ORDER BY priority DESC").await[0], "WEB-1");

    let bad = client.get("/api/search?q=status%20%3D%20Nope").await;
    assert_eq!(bad["error"]["message"], "There is no status called \"Nope\"");

    let malformed = client.get("/api/search?q=status%20%3D").await;
    assert_eq!(malformed["error"]["position"], 8);
    assert_eq!(client.get("/api/search?q=nonsense%20%3D%201").await["error"]["message"], "Unknown field \"nonsense\"");
}

#[tokio::test(flavor = "multi_thread")]
async fn private_projects_and_viewers() {
    let mut admin = fresh().await;
    let (mut anna, anna_id) = admin.member("Anna Nowak", "anna@example.com").await;
    let (mut piotr, _) = admin.member("Piotr Zieliński", "piotr@example.com").await;

    admin.post("/api/projects", json!({ "key": "SEC", "name": "Secret", "isPrivate": true })).await;
    admin.post("/api/projects", json!({ "key": "PUB", "name": "Public" })).await;
    admin.post("/api/issues", json!({ "projectKey": "SEC", "summary": "Hidden plan" })).await;
    admin.post("/api/issues", json!({ "projectKey": "PUB", "summary": "Open plan" })).await;
    admin.put(&format!("/api/projects/SEC/members/{anna_id}"), json!({ "role": "viewer" })).await;

    assert_eq!(keys(&anna.get("/api/projects").await), ["PUB", "SEC"]);
    assert_eq!(keys(&piotr.get("/api/projects").await), ["PUB"]);
    assert_eq!(piotr.refused(Method::GET, "/api/issues/SEC-1", None).await, StatusCode::NOT_FOUND);
    assert_eq!(piotr.refused(Method::GET, "/api/projects/SEC/board", None).await, StatusCode::NOT_FOUND);
    assert_eq!(piotr.search_keys("plan").await, ["PUB-1"]);
    assert_eq!(anna.search_keys("plan").await.len(), 2);

    assert_eq!(anna.get("/api/issues/SEC-1").await["summary"], "Hidden plan");
    assert_eq!(anna.refused(Method::PATCH, "/api/issues/SEC-1", Some(json!({ "summary": "Mine now" }))).await, StatusCode::FORBIDDEN);
    assert_eq!(anna.refused(Method::POST, "/api/issues", Some(json!({ "projectKey": "SEC", "summary": "New" }))).await, StatusCode::FORBIDDEN);

    piotr.patch("/api/issues/PUB-1", json!({ "summary": "Anyone signed in works in an open project" })).await;
    assert_eq!(piotr.refused(Method::PATCH, "/api/projects/PUB", Some(json!({ "name": "Taken over" }))).await, StatusCode::FORBIDDEN);
    assert_eq!(piotr.refused(Method::POST, "/api/users", Some(json!({ "name": "X", "email": "x@example.com" }))).await, StatusCode::FORBIDDEN);
    assert_eq!(piotr.refused(Method::PUT, "/api/admin/modules/github", Some(json!({ "enabled": false }))).await, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_strict_workflow_allows_only_listed_moves() {
    let mut client = fresh().await;
    let project = client.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    let todo = id_named(&project["statuses"], "To Do");
    let in_progress = id_named(&project["statuses"], "In Progress");
    let done = id_named(&project["statuses"], "Done");

    client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Careful" })).await;
    client
        .put(
            "/api/projects/WEB/transitions",
            json!([{ "fromStatusId": todo, "toStatusId": in_progress }, { "fromStatusId": in_progress, "toStatusId": done }, { "toStatusId": todo, "name": "Reopen" }]),
        )
        .await;
    client.patch("/api/projects/WEB", json!({ "strictWorkflow": true })).await;

    assert_eq!(client.refused(Method::PATCH, "/api/issues/WEB-1", Some(json!({ "statusId": done }))).await, StatusCode::CONFLICT);
    client.patch("/api/issues/WEB-1", json!({ "statusId": in_progress })).await;
    client.patch("/api/issues/WEB-1", json!({ "statusId": done })).await;
    client.patch("/api/issues/WEB-1", json!({ "statusId": todo })).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn statuses_can_be_added_reordered_and_removed() {
    let mut client = fresh().await;
    let project = client.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    let todo = id_named(&project["statuses"], "To Do");

    client.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Waiting" })).await;
    let project = client.post("/api/projects/WEB/statuses", json!({ "name": "QA", "category": "in_progress", "position": 3, "wipLimit": 2 })).await;
    let names: Vec<&str> = project["statuses"].as_array().unwrap().iter().map(|status| status["name"].as_str().unwrap()).collect();
    let qa = id_named(&project["statuses"], "QA");

    assert_eq!(names, ["To Do", "In Progress", "In Review", "QA", "Done"]);
    assert_eq!(client.refused(Method::DELETE, &format!("/api/projects/WEB/statuses/{todo}"), None).await, StatusCode::CONFLICT);

    client.delete(&format!("/api/projects/WEB/statuses/{todo}?moveTo={qa}")).await;
    assert_eq!(client.get("/api/issues/WEB-1").await["statusId"], qa);
}

#[tokio::test(flavor = "multi_thread")]
async fn people_hear_about_what_concerns_them() {
    let mut admin = fresh().await;
    let (mut anna, anna_id) = admin.member("Anna Nowak", "anna@example.com").await;
    let project = admin.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;
    let done = id_named(&project["statuses"], "Done");

    admin.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Pay by card", "assigneeId": anna_id })).await;
    admin.post("/api/issues/WEB-1/comments", json!({ "body": "Please check the sandbox keys" })).await;
    admin.post("/api/issues", json!({ "projectKey": "WEB", "summary": "Other" })).await;
    admin.post("/api/issues/WEB-2/comments", json!({ "body": "@anna.nowak can you look?" })).await;

    let kinds: Vec<String> =
        anna.get("/api/notifications").await.as_array().unwrap().iter().map(|item| item["kind"].as_str().unwrap().to_string()).collect();

    assert_eq!(kinds, ["mention", "comment", "assigned"]);
    assert_eq!(anna.get("/api/bootstrap").await["unreadNotifications"], 3);
    assert_eq!(admin.get("/api/notifications").await, json!([]), "nobody is told about their own actions");

    anna.patch("/api/issues/WEB-1", json!({ "statusId": done })).await;
    assert_eq!(admin.get("/api/notifications").await[0]["kind"], "status");

    anna.post("/api/notifications/read", json!({})).await;
    assert_eq!(anna.get("/api/bootstrap").await["unreadNotifications"], 0);

    let dashboard = anna.get("/api/dashboard").await;
    assert_eq!(keys(&dashboard["recentlyDone"]), ["WEB-1"]);
    assert_eq!(keys(&dashboard["watching"]), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn api_tokens_work_without_a_session() {
    let mut client = fresh().await;
    let token = client.post("/api/me/tokens", json!({ "name": "ci" })).await["token"].as_str().unwrap().to_string();
    let mut machine = Client::new(client.router.clone());

    machine.bearer = Some(token.clone());
    machine.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;

    let tokens = client.get("/api/me/tokens").await;
    assert_eq!(tokens[0]["hint"], format!("…{}", &token[token.len() - 4..]));
    assert!(!tokens.to_string().contains(&token), "the token itself is never returned again");

    client.delete(&format!("/api/me/tokens/{}", tokens[0]["id"])).await;
    assert_eq!(machine.refused(Method::GET, "/api/projects", None).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demo_instance_has_weeks_of_history() {
    let mut client = Client::new(build_router(test_app(Vec::new())));
    client.setup(true).await;

    assert_eq!(keys(&client.get("/api/projects").await), ["API", "WEB"]);

    let board = client.get("/api/projects/WEB/board").await;
    let on_board: usize = board["columns"].as_array().unwrap().iter().map(|column| column["issues"].as_array().unwrap().len()).sum();

    assert_eq!(board["sprint"]["name"], "WEB Sprint 2");
    assert_eq!(on_board, 12, "nine sprint issues and three subtasks");

    let backlog = client.get("/api/projects/WEB/backlog").await;
    assert_eq!(backlog["sprints"].as_array().unwrap().len(), 2);
    assert_eq!(backlog["backlog"].as_array().unwrap().len(), 6);

    assert_eq!(client.search_keys("project = WEB AND sprint = \"WEB Sprint 1\" AND category = done").await.len(), 5);
    assert_eq!(client.search_keys("project = WEB AND sprint in openSprints() AND assignee = currentUser() ORDER BY key").await, ["WEB-12", "WEB-14"]);

    let project = client.get("/api/projects/WEB").await;
    assert_eq!(project["versions"][2]["released"], true);
    assert_eq!(project["versions"][2]["counts"]["done"], 5);

    let kanban = client.get("/api/projects/API/board").await;
    assert_eq!(kanban["mode"], "kanban");
    assert!(kanban["sprint"].is_null());
    assert_eq!(keys(&client.get("/api/projects/API/backlog").await["backlog"]), ["API-6", "API-7"]);

    let dashboard = client.get("/api/dashboard").await;
    assert!(keys(&dashboard["assigned"]).contains(&"WEB-12".to_string()));
    assert_eq!(dashboard["activeSprints"][0]["sprint"]["name"], "WEB Sprint 2");
    assert!(client.get("/api/activity?limit=5").await.as_array().unwrap().len() == 5);
    assert_eq!(client.get("/api/quick?q=blik").await[0]["id"], "WEB-10");
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_your_own_password_needs_the_current_one() {
    let mut admin = fresh().await;
    let (mut anna, anna_id) = admin.member("Anna Nowak", "anna@example.com").await;
    let path = format!("/api/users/{anna_id}");

    assert_eq!(anna.refused(Method::PATCH, &path, Some(json!({ "password": "a new password" }))).await, StatusCode::FORBIDDEN);
    assert_eq!(
        anna.refused(Method::PATCH, &path, Some(json!({ "password": "a new password", "currentPassword": "not this one" }))).await,
        StatusCode::FORBIDDEN
    );
    anna.patch(&path, json!({ "password": "a new password", "currentPassword": "long enough" })).await;
    assert_eq!(anna.refused(Method::PATCH, &path, Some(json!({ "role": "admin" }))).await, StatusCode::FORBIDDEN);

    let mut again = Client::new(admin.router.clone());
    again.post("/api/login", json!({ "email": "anna@example.com", "password": "a new password" })).await;

    admin.patch(&path, json!({ "password": "reset by admin" })).await;
    admin.patch(&path, json!({ "active": false })).await;
    assert_eq!(again.refused(Method::GET, "/api/projects", None).await, StatusCode::UNAUTHORIZED, "deactivating ends their sessions");
}
