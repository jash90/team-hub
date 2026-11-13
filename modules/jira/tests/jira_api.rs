mod common;

use axum::http::{Method, StatusCode};
use common::{connected, fake, person, small, test_app, Setup, BASIC, BEARER, EMAIL, TOKEN};
use hub_app::db::{query_all, query_one};
use hub_app::App;
use hub_core::Role;
use serde_json::{json, Value};

fn count(app: &App, sql: &str) -> i64 {
    app.read(|connection| Ok(connection.query_row(sql, [], |row| row.get(0))?)).unwrap()
}

fn text(app: &App, sql: &str) -> Option<String> {
    app.read(|connection| query_one(connection, sql, [], |row| row.get(0))).unwrap().flatten()
}

#[tokio::test(flavor = "multi_thread")]
async fn everything_is_for_administrators_and_the_token_is_never_shown() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let member = person(&app, "Marek Member", Role::Member);
    let mut stranger = member.clone();
    stranger.bearer = None;

    assert_eq!(stranger.refused(Method::GET, "/jira/settings", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(stranger.refused(Method::GET, "/jira/jobs", None).await.0, StatusCode::UNAUTHORIZED);

    for (method, path, body) in [
        (Method::GET, "/jira/settings", None),
        (Method::PUT, "/jira/settings", Some(json!({ "baseUrl": "https://example.atlassian.net" }))),
        (Method::POST, "/jira/test", None),
        (Method::GET, "/jira/projects", None),
        (Method::GET, "/jira/projects/WEB/preview", None),
        (Method::POST, "/jira/import", Some(json!({ "projectKey": "WEB" }))),
        (Method::GET, "/jira/jobs", None),
        (Method::GET, "/jira/jobs/1", None),
        (Method::POST, "/jira/jobs/1/cancel", None),
        (Method::POST, "/jira/csv", Some(json!({}))),
        (Method::POST, "/jira/csv/1/preview", Some(json!({}))),
        (Method::POST, "/jira/csv/1/import", Some(json!({}))),
    ] {
        assert_eq!(member.refused(method.clone(), path, body).await.0, StatusCode::FORBIDDEN, "{method} {path}");
    }

    assert_eq!(admin.get("/jira/settings").await, json!({ "baseUrl": "", "email": "", "tokenHint": null, "configured": false }));
    assert_eq!(admin.get("/jira/jobs").await, json!([]));
    assert_eq!(admin.refused(Method::GET, "/jira/jobs/7", None).await.0, StatusCode::NOT_FOUND);

    // Nothing can be asked of Jira before it is configured, and no job is left behind.
    for (method, path, body) in [
        (Method::POST, "/jira/test", None),
        (Method::GET, "/jira/projects", None),
        (Method::GET, "/jira/projects/WEB/preview", None),
        (Method::POST, "/jira/import", Some(json!({ "projectKey": "WEB" }))),
    ] {
        assert_eq!(admin.refused(method.clone(), path, body).await.0, StatusCode::CONFLICT, "{method} {path}");
    }

    assert_eq!(admin.get("/jira/jobs").await, json!([]));

    for refused in
        ["http://jira.example.com", "jira.example.com", "ftp://example.com", "https://user:pw@example.com", "http://127.0.0.1.evil.example"]
    {
        let (status, message) = admin.refused(Method::PUT, "/jira/settings", Some(json!({ "baseUrl": refused, "token": "x" }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}: {message}");
    }

    let saved = admin
        .put("/jira/settings", json!({ "baseUrl": " https://example.atlassian.net/ ", "email": " anna@example.com ", "token": "secret-token-9876" }))
        .await;

    assert_eq!(saved, json!({ "baseUrl": "https://example.atlassian.net", "email": "anna@example.com", "tokenHint": "…9876", "configured": true }));
    assert!(!admin.get("/jira/settings").await.to_string().contains("secret-token"));

    let at_rest: Vec<Vec<u8>> = app.read(|connection| query_all(connection, "SELECT sealed FROM secret", [], |row| row.get(0))).unwrap();

    assert_eq!(at_rest.len(), 1);
    assert!(!String::from_utf8_lossy(&at_rest[0]).contains("secret-token"));

    // A blank token keeps the stored one; clearing it is said out loud.
    let kept = admin.put("/jira/settings", json!({ "baseUrl": "https://example.atlassian.net/jira", "email": "", "token": "  " })).await;

    assert_eq!(kept, json!({ "baseUrl": "https://example.atlassian.net/jira", "email": "", "tokenHint": "…9876", "configured": true }));

    // The stored token is not sent to another server just because the address changed.
    let (status, message) = admin.refused(Method::PUT, "/jira/settings", Some(json!({ "baseUrl": "https://jira.example.com" }))).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("enter the token"), "{message}");
    assert_eq!(admin.get("/jira/settings").await["baseUrl"], "https://example.atlassian.net/jira");

    let moved = admin.put("/jira/settings", json!({ "baseUrl": "https://jira.example.com", "token": "another-token-5555" })).await;

    assert_eq!((&moved["baseUrl"], &moved["tokenHint"]), (&json!("https://jira.example.com"), &json!("…5555")));

    let cleared = admin.put("/jira/settings", json!({ "baseUrl": "https://jira.example.com", "clearToken": true })).await;

    assert_eq!((&cleared["tokenHint"], &cleared["configured"]), (&Value::Null, &json!(false)));
    assert_eq!(admin.refused(Method::POST, "/jira/test", None).await.0, StatusCode::CONFLICT);

    // A project key goes into addresses and queries: only what Jira allows in one is taken.
    for key in ["", "W\"EB", "1WEB", "WEB OR 1=1"] {
        assert_eq!(admin.refused(Method::POST, "/jira/import", Some(json!({ "projectKey": key }))).await.0, StatusCode::BAD_REQUEST, "{key}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_connection_test_says_which_jira_answered() {
    let Setup { admin, fake, .. } = connected(small(true)).await;

    fake.take_requests();

    let cloud = admin.post("/jira/test", json!({})).await;

    assert_eq!(
        (cloud["ok"].as_bool(), cloud["deployment"].as_str(), cloud["version"].as_str()),
        (Some(true), Some("cloud"), Some("1001.0.0-SNAPSHOT"))
    );
    assert_eq!(cloud["message"], format!("Connected to Jira at {}", fake.base_url));
    assert_eq!(fake.take_requests(), vec!["GET /rest/api/2/serverInfo", "GET /rest/api/3/myself"]);

    // A personal access token of a Jira Server: no e-mail, Bearer, REST v2.
    let Setup { admin, fake, .. } = connected(small(false)).await;

    fake.take_requests();

    let server = admin.post("/jira/test", json!({})).await;

    assert_eq!((server["ok"].as_bool(), server["deployment"].as_str(), server["version"].as_str()), (Some(true), Some("server"), Some("9.12.4")));
    assert_eq!(fake.take_requests(), vec!["GET /rest/api/2/serverInfo", "GET /rest/api/2/myself"]);

    // The wrong kind of credentials for this server: Jira's refusal is the answer.
    admin.put("/jira/settings", json!({ "baseUrl": fake.base_url, "email": EMAIL, "token": TOKEN })).await;

    let refused = admin.post("/jira/test", json!({})).await;

    assert_eq!(
        (refused["ok"].as_bool(), refused["message"].as_str(), refused["deployment"].as_str()),
        (Some(false), Some("Jira rejected the token"), None)
    );
    assert_eq!(fake.take_requests().len(), 2);
    assert!(!refused.to_string().contains(TOKEN));
    assert_eq!(admin.refused(Method::GET, "/jira/projects", None).await, (StatusCode::BAD_GATEWAY, "Jira rejected the token".to_string()));

    // An address where no Jira lives.
    admin.put("/jira/settings", json!({ "baseUrl": format!("{}/nothing-here", fake.base_url), "token": TOKEN })).await;
    assert_eq!(admin.post("/jira/test", json!({})).await["message"], "Jira has nothing at that address; check the address and the project key");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_jira_is_asked_again_after_the_pause_it_names() {
    let Setup { admin, fake, .. } = connected(small(true)).await;

    fake.with(|upstream| upstream.failures = vec![(429, Some(1)), (503, None)]);
    fake.take_requests();

    let started = std::time::Instant::now();
    let result = admin.post("/jira/test", json!({})).await;

    assert_eq!(result["ok"], true, "{result}");
    // 429 with "Retry-After: 1", then a 503, then the two proper answers.
    assert_eq!(fake.take_requests().len(), 4);
    assert!(started.elapsed() >= std::time::Duration::from_millis(1500), "{:?}", started.elapsed());

    // A Jira that stays down is given up on, and the message says what it kept answering.
    fake.with(|upstream| upstream.failures = vec![(502, None); 4]);

    let down = admin.post("/jira/test", json!({})).await;

    assert_eq!((down["ok"].as_bool(), down["message"].as_str()), (Some(false), Some("Jira kept answering with status 502; try again later")));
    assert_eq!(fake.take_requests().len(), 4);
    assert!(BASIC.starts_with("Basic ") && BEARER.starts_with("Bearer "));
}

async fn the_preview_proposes_a_mapping(cloud: bool) {
    let Setup { admin, member, fake, .. } = connected(small(cloud)).await;
    let projects = admin.get("/jira/projects").await;

    assert_eq!(projects, json!([{ "key": "WEB", "name": "Web shop", "issueCount": 7, "exists": false, "importedAt": null }]));

    let preview = admin.get("/jira/projects/web/preview").await;
    let account = |name: &str| if cloud { format!("acc-{name}") } else { name.to_string() };

    assert_eq!(
        (preview["projectKey"].as_str(), preview["projectName"].as_str(), preview["issueCount"].as_i64()),
        (Some("WEB"), Some("Web shop"), Some(7))
    );
    assert_eq!((preview["sprints"].as_i64(), preview["versions"].as_i64(), preview["components"].as_i64()), (Some(3), Some(2), Some(2)));
    assert_eq!(preview["deployment"], if cloud { "cloud" } else { "server" });
    assert_eq!((preview["agile"].as_bool(), preview["storyPoints"].as_bool(), &preview["local"]), (Some(true), Some(true), &Value::Null));
    assert_eq!(
        preview["mapping"],
        json!({
            "boardMode": "scrum",
            "types": [
                { "jiraName": "Epic", "name": "Epic", "level": "epic" },
                { "jiraName": "Story", "name": "Story", "level": "standard" },
                { "jiraName": "Bug", "name": "Bug", "level": "standard" },
                { "jiraName": "Sub-task", "name": "Subtask", "level": "subtask" },
            ],
            // In the order of the board's columns, not the order Jira lists them in.
            "statuses": [
                { "jiraName": "Open", "jiraCategory": "new", "name": "Open", "category": "todo" },
                { "jiraName": "In Progress", "jiraCategory": "indeterminate", "name": "In Progress", "category": "in_progress" },
                { "jiraName": "In Review", "jiraCategory": "indeterminate", "name": "In Review", "category": "in_progress" },
                { "jiraName": "Done", "jiraCategory": "done", "name": "Done", "category": "done" },
            ],
            "priorities": [
                { "jiraName": "Highest", "priority": 1 },
                { "jiraName": "High", "priority": 2 },
                { "jiraName": "Medium", "priority": 3 },
                { "jiraName": "Low", "priority": 4 },
                { "jiraName": "Lowest", "priority": 5 },
            ],
            // Matched by e-mail address, whatever its case; the others have nobody here yet.
            "users": [
                { "account": account("ada"), "name": "Ada Admin", "email": "ada.admin@example.com", "userId": admin.user.id },
                { "account": account("marek"), "name": "Marek Member", "email": "MAREK.MEMBER@example.com", "userId": member.user.id },
                { "account": account("zofia"), "name": "Zofia Zewnętrzna", "email": null, "userId": null },
                { "account": account("olaf"), "name": "Olaf Odszedł", "email": "olaf@old.example.com", "userId": null },
            ],
            "createUsers": true,
        })
    );

    // A preview reads and writes nothing.
    assert_eq!(admin.get("/jira/jobs").await, json!([]));
    assert_eq!(admin.refused(Method::GET, "/jira/projects/NOPE/preview", None).await.0, StatusCode::BAD_GATEWAY);

    // After an import the list and the preview say so.
    admin.import(json!({ "projectKey": "WEB" })).await;

    let projects = admin.get("/jira/projects").await;

    assert_eq!((projects[0]["exists"].as_bool(), projects[0]["importedAt"].is_string()), (Some(true), true));

    let preview = admin.get("/jira/projects/WEB/preview").await;

    assert_eq!((preview["local"]["name"].as_str(), preview["local"]["issueCount"].as_i64()), (Some("Web shop"), Some(7)));
    assert!(preview["local"]["importedAt"].is_string());
    // The people who got an account in the meantime are now matched to it.
    assert!(preview["mapping"]["users"].as_array().unwrap().iter().all(|entry| entry["userId"].is_number()), "{preview}");
    assert!(!fake.take_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_preview_proposes_a_mapping_for_cloud() {
    the_preview_proposes_a_mapping(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_preview_proposes_a_mapping_for_server() {
    the_preview_proposes_a_mapping(false).await;
}

/// Key, summary, type, status, parent key, sprint name.
type IssueLine = (String, String, String, String, Option<String>, Option<String>);
/// Key, field, old value, new value, when.
type ChangeLine = (String, String, Option<String>, Option<String>, String);

const EXPORT: &str = "Summary,Issue key,Issue id,Issue Type,Status,Status Category,Project key,Project name,Priority,Assignee,Reporter,Created,Updated,Resolved,Description,Labels,Labels,Sprint,Sprint,Comment,Comment,Custom field (Story Points),Parent id,Outward issue link (Blocks),Log Work,Attachment,Component/s,Fix Version/s,Team\r\n\
\"Checkout, the \"\"new\"\" one\",SHOP-1,10001,Epic,In Progress,In Progress,SHOP,Web shop,High,Ada Admin,Jan Kowalski,01/Mar/24 9:00 AM,05/Mar/24 2:30 PM,,\"h2. Goal\n\nA *better* checkout,\nwith fewer steps.\",checkout,q2,,,\"02/Mar/24 10:15 AM;jkowalski;First line\nsecond line; with a semicolon\",Just text,,,,,,Cart,,Blue\r\n\
Pay by card,SHOP-2,10002,Story,Done,Done,SHOP,Web shop,Medium,Jan Kowalski,Ada Admin,02/Mar/24 9:00 AM,09/Mar/24 4:00 PM,09/Mar/24 4:00 PM,,,,Sprint 1,,\"03/Mar/24 8:00 AM;ada.admin@example.com;See [the docs|https://example.com/pay]\",,5.0,10001,SHOP-4,Done at last;08/Mar/24 1:00 PM;jkowalski;5400,03/Mar/24 9:00 AM;anowak;spec.pdf;https://jira.example.com/secure/attachment/7/spec.pdf,Payments,1.0,Blue\r\n\
Card form,SHOP-3,10003,Sub-task,To Do,To Do,SHOP,Web shop,,,,2024-03-03 08:00,2024-03-03 08:00,,,,,Sprint 2,,,,,10002,,,,,,Red\r\n\
Refunds,SHOP-4,10004,Story,To Do,To Do,SHOP,Web shop,Low,,,04/Mar/24 9:00 AM,04/Mar/24 9:00 AM,,,,,Sprint 1,Sprint 2,,,,10001,,,,,,\r\n\
Other project,OPS-9,20009,Task,Open,To Do,OPS,Operations,Low,,,03/Mar/24 9:00 AM,03/Mar/24 9:00 AM,,,,,,,,,,,,,,,,\r\n";

#[tokio::test(flavor = "multi_thread")]
async fn a_csv_export_is_previewed_mapped_and_imported() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);

    assert_eq!(admin.upload("/jira/csv", "empty.csv", "  ").await.0, StatusCode::BAD_REQUEST);

    let (status, refused) = admin.upload("/jira/csv", "people.csv", "Name,Age\nAda,36\n").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused["error"]["message"].as_str().unwrap().contains("no column with issue keys"), "{refused}");
    assert_eq!(count(&app, "SELECT count(*) FROM jira_upload"), 0);

    let (status, preview) = admin.upload("/jira/csv", "Jira%20export%20(SHOP).csv", EXPORT).await;

    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!((preview["filename"].as_str(), preview["rows"].as_i64()), (Some("Jira export (SHOP).csv"), Some(5)));
    assert_eq!(preview["projects"], json!([{ "key": "SHOP", "issues": 4 }, { "key": "OPS", "issues": 1 }]));
    assert_eq!(
        (preview["projectKey"].as_str(), preview["projectName"].as_str(), preview["issueCount"].as_i64()),
        (Some("SHOP"), Some("Web shop"), Some(4))
    );
    assert_eq!((preview["sprints"].as_i64(), preview["versions"].as_i64(), preview["components"].as_i64()), (Some(2), Some(1), Some(2)));
    assert_eq!(preview["problems"], json!([]));

    let columns = preview["columns"].as_array().unwrap();
    let target_of = |name: &str| columns.iter().find(|column| column["name"] == name).unwrap()["target"].as_str().unwrap().to_string();

    assert_eq!(columns.len(), 29);
    assert_eq!(
        (target_of("Issue key"), target_of("Custom field (Story Points)"), target_of("Outward issue link (Blocks)"), target_of("Team")),
        ("key".to_string(), "points".to_string(), "linkOutward".to_string(), "ignore".to_string())
    );
    assert_eq!(columns[0]["sample"], "Checkout, the \"new\" one");
    assert!(preview["targets"].as_array().unwrap().contains(&json!("labels")));
    assert_eq!(
        preview["mapping"]["statuses"],
        json!([
            { "jiraName": "To Do", "jiraCategory": "new", "name": "To Do", "category": "todo" },
            { "jiraName": "In Progress", "jiraCategory": "indeterminate", "name": "In Progress", "category": "in_progress" },
            { "jiraName": "Done", "jiraCategory": "done", "name": "Done", "category": "done" },
        ])
    );
    assert_eq!(preview["mapping"]["users"][0], json!({ "account": "Ada Admin", "name": "Ada Admin", "email": null, "userId": null }));

    // The administrator reads the "Team" column as labels, and maps a name to an account.
    let id = preview["uploadId"].as_i64().unwrap();
    let team = columns.iter().find(|column| column["name"] == "Team").unwrap()["index"].clone();
    let choice = json!({
        "columns": [{ "index": team, "name": "Team", "target": "labels" }],
        "projectName": "Shop",
        "mapping": { "users": [{ "account": "Ada Admin", "userId": admin.user.id }], "createUsers": true },
    });
    let adjusted = admin.post(&format!("/jira/csv/{id}/preview"), choice.clone()).await;

    assert_eq!(adjusted["columns"].as_array().unwrap().iter().find(|column| column["name"] == "Team").unwrap()["target"], "labels");
    assert_eq!((adjusted["projectName"].as_str(), adjusted["mapping"]["users"][0]["userId"].as_i64()), (Some("Shop"), Some(admin.user.id)));
    assert_eq!(admin.post(&format!("/jira/csv/{id}/preview"), json!({ "projectKey": "ops" })).await["issueCount"], 1);
    assert_eq!(admin.refused(Method::POST, "/jira/csv/999/preview", Some(json!({}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        admin.refused(Method::POST, &format!("/jira/csv/{id}/import"), Some(json!({ "projectKey": "NOPE" }))).await.0,
        StatusCode::BAD_REQUEST
    );

    let job = admin.finished(&admin.post(&format!("/jira/csv/{id}/import"), choice.clone()).await).await;
    let report = &job["report"];

    assert_eq!((job["state"].as_str(), job["kind"].as_str(), job["projectKey"].as_str()), (Some("done"), Some("csv"), Some("SHOP")), "{job}");
    assert_eq!(
        (report["created"].as_i64(), report["failed"].as_array().map(Vec::len), report["skipped"].as_array().map(Vec::len)),
        (Some(4), Some(0), Some(0)),
        "{job}"
    );
    assert_eq!((report["comments"].as_i64(), report["worklogs"].as_i64(), report["links"].as_i64()), (Some(3), Some(1), Some(1)));
    // Names and logins have nobody here; an author given by e-mail address is matched by it.
    assert_eq!(report["usersCreated"], json!(["Jan Kowalski", "jkowalski"]));
    assert_eq!(report["usersMatched"], 2);

    // Keys kept; epic → story → subtask through Jira's parent ids.
    let issues: Vec<IssueLine> = app
        .read(|connection| {
            query_all(
                connection,
                "SELECT i.key, i.summary, t.name, s.name, p.key, sp.name FROM issue i JOIN issue_type t ON t.id = i.type_id JOIN status s ON s.id = i.status_id
                 LEFT JOIN issue p ON p.id = i.parent_id LEFT JOIN sprint sp ON sp.id = i.sprint_id ORDER BY i.number",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
        })
        .unwrap();
    let row = |key: &str, summary: &str, kind: &str, status: &str, parent: Option<&str>, sprint: Option<&str>| {
        (key.to_string(), summary.to_string(), kind.to_string(), status.to_string(), parent.map(String::from), sprint.map(String::from))
    };

    assert_eq!(
        issues,
        vec![
            row("SHOP-1", "Checkout, the \"new\" one", "Epic", "In Progress", None, None),
            row("SHOP-2", "Pay by card", "Story", "Done", Some("SHOP-1"), Some("Sprint 1")),
            row("SHOP-3", "Card form", "Subtask", "To Do", Some("SHOP-2"), Some("Sprint 2")),
            row("SHOP-4", "Refunds", "Story", "To Do", Some("SHOP-1"), Some("Sprint 2")),
        ]
    );
    assert_eq!(text(&app, "SELECT name FROM project WHERE key = 'SHOP'").as_deref(), Some("Shop"));
    assert_eq!(count(&app, "SELECT count(*) FROM project"), 1);

    // Cells with line breaks, quotes and delimiters arrive whole; markup becomes Markdown.
    assert_eq!(
        text(&app, "SELECT description FROM issue WHERE key = 'SHOP-1'").as_deref(),
        Some("## Goal\n\nA **better** checkout,\\\nwith fewer steps.")
    );

    let comments: Vec<(String, String, Option<String>)> = app
        .read(|connection| {
            query_all(
                connection,
                "SELECT c.body, c.created_at, u.name FROM comment c JOIN issue i ON i.id = c.issue_id LEFT JOIN user u ON u.id = c.author_id ORDER BY i.number, c.id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .unwrap();

    assert_eq!(
        comments,
        vec![
            ("First line\\\nsecond line; with a semicolon".to_string(), "2024-03-02T10:15:00.000Z".to_string(), Some("jkowalski".to_string())),
            ("Just text".to_string(), "2024-03-05T14:30:00.000Z".to_string(), None),
            ("See [the docs](https://example.com/pay)".to_string(), "2024-03-03T08:00:00.000Z".to_string(), Some("Ada Admin".to_string())),
        ]
    );

    // Repeated columns are collected, and the column read as labels joins them.
    let labels = |key: &str| -> Vec<String> {
        app.read(|connection| {
            query_all(
                connection,
                "SELECT l.label FROM issue_label l JOIN issue i ON i.id = l.issue_id WHERE i.key = ?1 ORDER BY l.label",
                [key],
                |row| row.get(0),
            )
        })
        .unwrap()
    };

    assert_eq!(labels("SHOP-1"), vec!["Blue", "checkout", "q2"]);
    assert_eq!(labels("SHOP-3"), vec!["Red"]);
    assert_eq!(count(&app, "SELECT assignee_id FROM issue WHERE key = 'SHOP-1'"), admin.user.id);
    assert_eq!(count(&app, "SELECT CAST(points AS INTEGER) FROM issue WHERE key = 'SHOP-2'"), 5);
    assert_eq!(text(&app, "SELECT resolved_at FROM issue WHERE key = 'SHOP-2'").as_deref(), Some("2024-03-09T16:00:00.000Z"));
    assert_eq!(text(&app, "SELECT v.name FROM issue i JOIN version v ON v.id = i.version_id WHERE i.key = 'SHOP-2'").as_deref(), Some("1.0"));
    assert_eq!(count(&app, "SELECT minutes FROM worklog"), 90);
    assert!(text(&app, "SELECT description FROM issue WHERE key = 'SHOP-2'")
        .unwrap()
        .ends_with("## Attachments in Jira\n\n- [spec.pdf](https://jira.example.com/secure/attachment/7/spec.pdf)"));
    assert_eq!(count(&app, "SELECT count(*) FROM issue_link l JOIN issue s ON s.id = l.source_id JOIN issue t ON t.id = l.target_id WHERE s.key = 'SHOP-2' AND t.key = 'SHOP-4' AND l.type = 'blocks'"), 1);

    // A file has no history: one step from the first status to where the issue is, at the
    // time it was resolved, so the reports do not see it as done on the day it was created.
    let changes: Vec<ChangeLine> = app
        .read(|connection| {
            query_all(
                connection,
                "SELECT i.key, c.field, c.old_value, c.new_value, c.created_at FROM change c JOIN issue i ON i.id = c.issue_id ORDER BY i.number, c.id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
        })
        .unwrap();

    assert_eq!(
        changes,
        vec![
            (
                "SHOP-1".to_string(),
                "status".to_string(),
                Some("To Do".to_string()),
                Some("In Progress".to_string()),
                "2024-03-05T14:30:00.000Z".to_string()
            ),
            ("SHOP-2".to_string(), "status".to_string(), Some("To Do".to_string()), Some("Done".to_string()), "2024-03-09T16:00:00.000Z".to_string()),
        ]
    );
    // The sprint whose work is all done counts as completed; the other waits to be started.
    let sprints: Vec<(String, String)> =
        app.read(|connection| query_all(connection, "SELECT name, state FROM sprint ORDER BY id", [], |row| Ok((row.get(0)?, row.get(1)?)))).unwrap();

    assert_eq!(sprints, vec![("Sprint 1".to_string(), "closed".to_string()), ("Sprint 2".to_string(), "future".to_string())]);

    // No Jira address is configured, so the panel names the original without a link.
    let panel = hub_app::services::issues::issue_detail(&app, &admin.user, "SHOP-2").unwrap().panels["jira"].clone();

    assert_eq!((panel["key"].as_str(), &panel["url"]), (Some("SHOP-2"), &Value::Null));

    // The same file again changes nothing; a newer export updates what changed.
    let totals = |app: &App| {
        ["issue", "comment", "change", "worklog", "issue_link", "user", "sprint"].map(|table| count(app, &format!("SELECT count(*) FROM {table}")))
    };
    let before = totals(&app);
    let again = admin.finished(&admin.post(&format!("/jira/csv/{id}/import"), choice.clone()).await).await;

    assert_eq!(
        (again["report"]["created"].as_i64(), again["report"]["updated"].as_i64(), again["report"]["unchanged"].as_i64()),
        (Some(0), Some(0), Some(4)),
        "{again}"
    );
    assert_eq!(totals(&app), before);

    let newer = EXPORT
        .replace("Refunds,SHOP-4,10004,Story,To Do,To Do", "Refunds and returns,SHOP-4,10004,Story,In Progress,In Progress")
        .replace("04/Mar/24 9:00 AM,04/Mar/24 9:00 AM", "04/Mar/24 9:00 AM,11/Mar/24 9:00 AM");
    let (_, second) = admin.upload("/jira/csv", "newer.csv", &newer).await;
    let updated = admin.finished(&admin.post(&format!("/jira/csv/{}/import", second["uploadId"]), choice).await).await;

    assert_eq!(
        (updated["report"]["created"].as_i64(), updated["report"]["updated"].as_i64(), updated["report"]["unchanged"].as_i64()),
        (Some(0), Some(1), Some(3)),
        "{updated}"
    );
    assert_eq!(text(&app, "SELECT summary FROM issue WHERE key = 'SHOP-4'").as_deref(), Some("Refunds and returns"));
    assert_eq!(
        text(&app, "SELECT c.old_value || ' → ' || c.new_value || ' @ ' || c.created_at FROM change c JOIN issue i ON i.id = c.issue_id WHERE i.key = 'SHOP-4'").as_deref(),
        Some("To Do → In Progress @ 2024-03-11T09:00:00.000Z")
    );
    assert_eq!(totals(&app), [before[0], before[1], before[2] + 1, before[3], before[4], before[5], before[6]]);
    let _ = fake::POINTS_FIELD;
}
