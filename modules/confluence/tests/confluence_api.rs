mod common;

use axum::http::{Method, StatusCode};
use common::{person, test_app, Client, Fake};
use hub_app::db::query_all;
use hub_app::services::projects::{create_project, NewProject};
use hub_app::{App, Module};
use hub_core::{IssuePatch, NewIssue, Role};
use hub_mod_confluence::ConfluenceModule;
use serde_json::{json, Value};

const BASIC: &str = "Basic YW5uYUBleGFtcGxlLmNvbTphcGktdG9rZW4tMTIzNA==";
const EMAIL: &str = "anna@example.com";
const TOKEN: &str = "api-token-1234";

/// An application with an administrator, and a fake Confluence it is configured for.
async fn connected() -> (App, Client, Fake) {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let fake = Fake::start(BASIC).await;
    fake.with(|upstream| upstream.spaces.push(("DOC".to_string(), "Documentation".to_string())));
    admin.put("/confluence/settings", json!({ "baseUrl": fake.base_url, "email": EMAIL, "token": TOKEN })).await;

    (app, admin, fake)
}

/// Home › (Guide › Install), Notes, Archive: five pages, served two at a time.
fn fill(fake: &Fake) {
    fake.add("1", "Home", None, "<p>Welcome to the docs.</p>");
    fake.add(
        "2",
        "Guide",
        Some("1"),
        "<h2>Steps</h2><ol><li>Read <ac:link><ri:page ri:content-title=\"Install\"/></ac:link></li><li>Run it</li></ol>",
    );
    fake.add("3", "Install", Some("2"), "<ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">bash</ac:parameter><ac:plain-text-body><![CDATA[make install]]></ac:plain-text-body></ac:structured-macro>");
    fake.add("4", "Notes", Some("1"), "<p>Zobacz <ac:link><ri:page ri:content-title=\"Guide\"/></ac:link> i WEB-1.</p><ac:task-list><ac:task><ac:task-status>incomplete</ac:task-status><ac:task-body>Zamówić klucze</ac:task-body></ac:task></ac:task-list>");
    fake.add("5", "Archive", Some("1"), "<p>Old things.</p>");
}

async fn tree(admin: &Client, space_key: &str) -> Vec<(String, Option<String>)> {
    let nodes = admin.get(&format!("/wiki/spaces/{space_key}/tree")).await;
    let nodes = nodes.as_array().expect("a tree");
    let title_of = |id: &Value| nodes.iter().find(|node| &node["id"] == id).map(|node| node["title"].as_str().unwrap().to_string());

    nodes.iter().map(|node| (node["title"].as_str().unwrap().to_string(), title_of(&node["parentId"]))).collect()
}

async fn page(admin: &Client, space_key: &str, title: &str) -> Value {
    let nodes = admin.get(&format!("/wiki/spaces/{space_key}/tree")).await;
    let id = nodes.as_array().unwrap().iter().find(|node| node["title"] == title).unwrap_or_else(|| panic!("no page \"{title}\" in {nodes}"))["id"]
        .as_i64()
        .unwrap();

    admin.get(&format!("/wiki/pages/{id}")).await
}

fn counts(report: &Value) -> (i64, i64, i64, i64, usize) {
    let number = |name: &str| report[name].as_i64().unwrap_or(-1);

    (number("created"), number("updated"), number("unchanged"), number("skipped"), report["failed"].as_array().map_or(99, Vec::len))
}

fn under(parent: &str) -> Option<String> {
    Some(parent.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_for_administrators_and_never_show_the_token() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let member = person(&app, "Marek Member", Role::Member);
    let mut stranger = member.clone();
    stranger.bearer = None;

    assert_eq!(stranger.refused(Method::GET, "/confluence/settings", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(stranger.refused(Method::GET, "/confluence/imports", None).await.0, StatusCode::UNAUTHORIZED);

    for (method, path, body) in [
        (Method::GET, "/confluence/settings", None),
        (Method::PUT, "/confluence/settings", Some(json!({ "baseUrl": "https://example.atlassian.net/wiki" }))),
        (Method::POST, "/confluence/test", None),
        (Method::GET, "/confluence/spaces", None),
        (Method::POST, "/confluence/import", Some(json!({ "spaceKey": "DOC" }))),
        (Method::POST, "/confluence/imports/DOC/sync", None),
    ] {
        assert_eq!(member.refused(method.clone(), path, body).await.0, StatusCode::FORBIDDEN, "{method} {path}");
    }

    assert_eq!(member.get("/confluence/imports").await, json!([]));
    assert_eq!(admin.get("/confluence/settings").await, json!({ "baseUrl": "", "email": "", "tokenHint": null, "configured": false }));

    // Nothing can be asked of Confluence before it is configured.
    for (method, path, body) in [
        (Method::POST, "/confluence/test", None),
        (Method::GET, "/confluence/spaces", None),
        (Method::POST, "/confluence/import", Some(json!({ "spaceKey": "DOC" }))),
    ] {
        assert_eq!(admin.refused(method.clone(), path, body).await.0, StatusCode::CONFLICT, "{method} {path}");
    }

    for refused in [
        "http://confluence.example.com",
        "confluence.example.com",
        "ftp://example.com",
        "https://user:pw@example.com",
        "http://127.0.0.1.evil.example",
    ] {
        let (status, message) = admin.refused(Method::PUT, "/confluence/settings", Some(json!({ "baseUrl": refused, "token": "x" }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}: {message}");
    }

    let saved = admin
        .put(
            "/confluence/settings",
            json!({ "baseUrl": " https://example.atlassian.net/wiki/ ", "email": " anna@example.com ", "token": "secret-token-9876" }),
        )
        .await;
    assert_eq!(
        saved,
        json!({ "baseUrl": "https://example.atlassian.net/wiki", "email": "anna@example.com", "tokenHint": "…9876", "configured": true })
    );
    assert!(!admin.get("/confluence/settings").await.to_string().contains("secret-token"));

    let at_rest: Vec<Vec<u8>> = app.read(|connection| query_all(connection, "SELECT sealed FROM secret", [], |row| row.get(0))).unwrap();
    assert_eq!(at_rest.len(), 1);
    assert!(!String::from_utf8_lossy(&at_rest[0]).contains("secret-token"));

    // A blank token keeps the stored one; clearing it is said out loud.
    let kept = admin.put("/confluence/settings", json!({ "baseUrl": "https://example.atlassian.net", "email": "", "token": "  " })).await;
    assert_eq!(kept, json!({ "baseUrl": "https://example.atlassian.net", "email": "", "tokenHint": "…9876", "configured": true }));
    assert_eq!(admin.put("/confluence/settings", json!({ "baseUrl": "https://example.atlassian.net/wiki" })).await["tokenHint"], "…9876");

    // The stored token is not sent to another server just because the address changed.
    let (status, message) = admin.refused(Method::PUT, "/confluence/settings", Some(json!({ "baseUrl": "https://confluence.example.com" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("enter the token"), "{message}");
    assert_eq!(admin.get("/confluence/settings").await["baseUrl"], "https://example.atlassian.net/wiki");

    let moved = admin.put("/confluence/settings", json!({ "baseUrl": "https://confluence.example.com", "token": "another-token-5555" })).await;
    assert_eq!((&moved["baseUrl"], &moved["tokenHint"]), (&json!("https://confluence.example.com"), &json!("…5555")));

    let cleared = admin.put("/confluence/settings", json!({ "baseUrl": "https://confluence.example.com", "clearToken": true })).await;
    assert_eq!((&cleared["tokenHint"], &cleared["configured"]), (&Value::Null, &json!(false)));
    assert_eq!(admin.refused(Method::POST, "/confluence/test", None).await.0, StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_authorization_header_follows_the_settings() {
    let (_app, admin, fake) = connected().await;

    // With an e-mail: Basic, as Confluence Cloud wants it.
    assert_eq!(admin.post("/confluence/test", json!({})).await, json!({ "ok": true, "message": format!("Connected to {}", fake.base_url) }));
    assert_eq!(fake.take_requests(), [("/rest/api/space".to_string(), 0, 1, BASIC.to_string())]);

    // Without one: Bearer, as a personal access token on Server and Data Center wants it.
    fake.with(|upstream| {
        upstream.accepts = "Bearer pat-abcdef".to_string();
        upstream.spaces.extend([("OPS".to_string(), "Operations".to_string()), ("HR".to_string(), "People".to_string())]);
    });
    admin.put("/confluence/settings", json!({ "baseUrl": fake.base_url, "email": "", "token": "pat-abcdef" })).await;
    assert_eq!(admin.post("/confluence/test", json!({})).await["ok"], true);
    assert_eq!(fake.take_requests()[0].3, "Bearer pat-abcdef");

    // The list of spaces comes in pages too.
    let spaces = admin.get("/confluence/spaces").await;
    assert_eq!(spaces, json!([{ "key": "DOC", "name": "Documentation" }, { "key": "OPS", "name": "Operations" }, { "key": "HR", "name": "People" }]));

    let requests = fake.take_requests();
    assert_eq!(requests.iter().map(|request| (request.1, request.2)).collect::<Vec<_>>(), [(0, 50), (2, 2)]);
    assert!(requests.iter().all(|request| request.3 == "Bearer pat-abcdef"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_token_is_reported_and_not_tried_again() {
    let (_app, admin, fake) = connected().await;
    fill(&fake);
    fake.with(|upstream| upstream.accepts = "Basic something-else".to_string());

    assert_eq!(admin.post("/confluence/test", json!({})).await, json!({ "ok": false, "message": "Confluence rejected the token" }));
    assert_eq!(fake.take_requests().len(), 1);

    let (status, message) = admin.refused(Method::POST, "/confluence/import", Some(json!({ "spaceKey": "DOC" }))).await;
    assert_eq!((status, message.as_str()), (StatusCode::BAD_GATEWAY, "Confluence rejected the token"));
    assert_eq!(fake.take_requests().len(), 1);

    let (status, message) = admin.refused(Method::GET, "/confluence/spaces", None).await;
    assert_eq!((status, message.as_str()), (StatusCode::BAD_GATEWAY, "Confluence rejected the token"));

    // Nothing was created on the way.
    assert_eq!(admin.get("/wiki/spaces").await, json!([]));
    assert_eq!(admin.get("/confluence/imports").await, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn busy_and_failing_answers_are_tried_three_times() {
    let (_app, admin, fake) = connected().await;

    fake.with(|upstream| upstream.failures = vec![429, 503]);
    assert_eq!(admin.post("/confluence/test", json!({})).await["ok"], true);
    assert_eq!(fake.take_requests().len(), 3);

    fake.with(|upstream| upstream.failures = vec![500, 502, 500, 500]);
    assert_eq!(
        admin.post("/confluence/test", json!({})).await,
        json!({ "ok": false, "message": "Confluence kept answering with status 500; try again later" })
    );
    assert_eq!(fake.take_requests().len(), 3);

    // The fourth failure was still waiting; an import meets it and carries on.
    fill(&fake);
    assert_eq!(counts(&admin.post("/confluence/import", json!({ "spaceKey": "DOC" })).await), (5, 0, 0, 0, 0));

    // Other refusals are final at once.
    fake.with(|upstream| upstream.failures = vec![403]);
    let (status, message) = admin.refused(Method::POST, "/confluence/imports/DOC/sync", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(message.contains("refused access"), "{message}");
    assert_eq!(fake.take_requests().iter().filter(|request| request.0 == "/rest/api/content").count(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_space_is_imported_page_by_page_as_a_locked_mirror() {
    let (app, admin, fake) = connected().await;
    let member = person(&app, "Marek Member", Role::Member);
    fill(&fake);

    // An issue for the imported text to mention.
    app.write(admin.user.actor(), |tx| {
        let project = NewProject {
            key: "WEB".to_string(),
            name: "Web".to_string(),
            description: String::new(),
            board_mode: None,
            is_private: false,
            lead_id: None,
        };
        create_project(tx, &admin.user, &project)?;

        hub_app::services::issues::create_issue(
            tx,
            &NewIssue { project_key: "WEB".to_string(), fields: IssuePatch { summary: Some("Klucze".to_string()), ..Default::default() } },
        )
    })
    .unwrap();

    let report = admin.post("/confluence/import", json!({ "spaceKey": "DOC" })).await;
    assert_eq!(report, json!({ "wikiSpaceKey": "DOC", "created": 5, "updated": 0, "unchanged": 0, "skipped": 0, "failed": [], "gone": [] }));

    // Five pages, two at a time: three requests, each following the link of the one before.
    let requests = fake.take_requests();
    let content: Vec<(usize, usize)> =
        requests.iter().filter(|request| request.0 == "/rest/api/content").map(|request| (request.1, request.2)).collect();
    assert_eq!(content, [(0, 50), (2, 2), (4, 2)]);
    assert!(requests.iter().all(|request| request.3 == BASIC));

    let space = admin.get("/wiki/spaces/DOC").await;
    assert_eq!((space["name"].as_str(), space["source"].as_str(), space["pageCount"].as_i64()), (Some("Documentation"), Some("confluence"), Some(5)));
    assert_eq!(
        tree(&admin, "DOC").await,
        [
            ("Home".to_string(), None),
            ("Guide".to_string(), under("Home")),
            ("Install".to_string(), under("Guide")),
            ("Notes".to_string(), under("Home")),
            ("Archive".to_string(), under("Home"))
        ]
    );

    let guide = page(&admin, "DOC", "Guide").await;
    assert_eq!(guide["body"], "## Steps\n\n1. Read [[Install]]\n2. Run it");
    assert_eq!(
        (guide["locked"].as_bool(), guide["version"].as_i64(), guide["sourceId"].as_str(), guide["sourceVersion"].as_i64()),
        (Some(true), Some(1), Some("2"), Some(1))
    );
    assert_eq!(guide["sourceUrl"], format!("{}/spaces/DOC/pages/2", fake.base_url));
    assert_eq!(guide["authorId"], Value::Null);
    assert_eq!(guide["backlinks"], json!([{ "pageId": page(&admin, "DOC", "Notes").await["id"], "spaceKey": "DOC", "title": "Notes" }]));
    assert_eq!(page(&admin, "DOC", "Install").await["body"], "```bash\nmake install\n```");

    // Links to pages and issue keys in the imported text work like in any other page.
    let notes = page(&admin, "DOC", "Notes").await;
    assert_eq!(notes["body"], "Zobacz [[Guide]] i WEB-1.\n\n- [ ] Zamówić klucze");
    assert_eq!(notes["issues"][0]["issue"]["key"], "WEB-1");
    assert_eq!(notes["issues"][0]["relation"], "mentions");

    let revisions = admin.get(&format!("/wiki/pages/{}/revisions", guide["id"])).await;
    assert_eq!((revisions[0]["authorLabel"].as_str(), &revisions[0]["authorId"]), (Some("Confluence"), &Value::Null));

    // The mirror is read-only, and the wiki finds its pages.
    let (status, message) =
        admin.refused(Method::PATCH, &format!("/wiki/pages/{}", guide["id"]), Some(json!({ "body": "mine", "baseVersion": 1 }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("detach it to edit"), "{message}");
    assert_eq!(member.get("/wiki/search?q=zamowic").await[0]["title"], "Notes");

    // The import is remembered with its outcome, for everyone who may see the space.
    let imports = member.get("/confluence/imports").await;
    assert_eq!(imports.as_array().unwrap().len(), 1);
    assert_eq!(imports[0]["lastReport"], report);
    assert!(imports[0]["lastRunAt"].is_string());
    assert_eq!(
        (
            &imports[0]["wikiSpaceKey"],
            &imports[0]["wikiSpaceName"],
            &imports[0]["spaceKey"],
            &imports[0]["rootPageId"],
            &imports[0]["exclude"],
            &imports[0]["baseUrl"]
        ),
        (&json!("DOC"), &json!("Documentation"), &json!("DOC"), &Value::Null, &json!([]), &json!(fake.base_url))
    );

    let announced: Vec<String> =
        app.read(|connection| query_all(connection, "SELECT payload FROM event WHERE kind = 'confluence.imported'", [], |row| row.get(0))).unwrap();
    assert_eq!(announced.len(), 1);
    assert_eq!(serde_json::from_str::<Value>(&announced[0]).unwrap()["created"], 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sync_stores_what_changed_and_does_not_touch_the_rest() {
    let (_app, admin, fake) = connected().await;
    fill(&fake);
    admin.post("/confluence/import", json!({ "spaceKey": "DOC" })).await;

    let mut before = Vec::new();

    for title in ["Home", "Guide", "Install", "Notes", "Archive"] {
        before.push(page(&admin, "DOC", title).await);
    }

    // Nothing changed upstream: nothing changes here, not even a timestamp.
    let report = admin.post("/confluence/imports/doc/sync", json!({})).await;
    assert_eq!(counts(&report), (0, 0, 5, 0, 0));

    for old in &before {
        assert_eq!(&page(&admin, "DOC", old["title"].as_str().unwrap()).await, old);
    }

    // Upstream: Guide is edited, a page is added under it, Archive is deleted. This time the
    // server gives no links to follow, as older ones do not.
    fake.edit("2", "<p>Rewritten, see <ac:link><ri:page ri:content-title=\"Upgrade\"/></ac:link>.</p>");
    fake.add("6", "Upgrade", Some("2"), "<p>From 1.x to 2.x.</p>");
    fake.with(|upstream| {
        upstream.pages.retain(|page| page.id != "5");
        upstream.next_links = false;
    });
    fake.take_requests();

    let report = admin.post("/confluence/imports/DOC/sync", json!({})).await;
    assert_eq!(counts(&report), (1, 1, 3, 0, 0));
    assert_eq!(report["gone"], json!([{ "pageId": before[4]["id"], "title": "Archive" }]));
    assert_eq!(fake.take_requests().iter().map(|request| request.1).collect::<Vec<_>>(), [0, 2, 4]);

    let guide = page(&admin, "DOC", "Guide").await;
    assert_eq!(guide["body"], "Rewritten, see [[Upgrade]].");
    assert_eq!((guide["version"].as_i64(), guide["sourceVersion"].as_i64(), guide["locked"].as_bool()), (Some(2), Some(2), Some(true)));
    assert!(guide["updatedAt"].as_str() > before[1]["updatedAt"].as_str());

    let revisions = admin.get(&format!("/wiki/pages/{}/revisions", guide["id"])).await;
    assert_eq!(revisions.as_array().unwrap().len(), 2);
    assert_eq!((revisions[0]["version"].as_i64(), revisions[0]["authorLabel"].as_str()), (Some(2), Some("Confluence")));
    assert_eq!(admin.get(&format!("/wiki/pages/{}/revisions/1", guide["id"])).await["body"], before[1]["body"]);

    // The new page sits where Confluence has it, and the link to it resolved.
    let upgrade = page(&admin, "DOC", "Upgrade").await;
    assert_eq!((upgrade["parentId"].as_i64(), upgrade["locked"].as_bool()), (guide["id"].as_i64(), Some(true)));
    assert_eq!(upgrade["backlinks"][0]["title"], "Guide");

    // Unchanged pages are exactly as they were; the page gone upstream is still here.
    for old in [&before[0], &before[2], &before[3], &before[4]] {
        let now = page(&admin, "DOC", old["title"].as_str().unwrap()).await;
        assert_eq!(
            (&now["version"], &now["updatedAt"], &now["body"], &now["parentId"]),
            (&old["version"], &old["updatedAt"], &old["body"], &old["parentId"])
        );
    }

    assert_eq!(admin.get("/confluence/imports").await[0]["lastReport"], report);
    assert_eq!(admin.refused(Method::POST, "/confluence/imports/NOPE/sync", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_detached_page_is_never_touched_again() {
    let (_app, admin, fake) = connected().await;
    fill(&fake);
    admin.post("/confluence/import", json!({ "spaceKey": "DOC" })).await;

    let guide = page(&admin, "DOC", "Guide").await;
    let path = format!("/wiki/pages/{}", guide["id"]);
    let detached = admin.post(&format!("{path}/detach"), json!({})).await;
    assert_eq!((detached["locked"].as_bool(), &detached["sourceId"]), (Some(false), &Value::Null));

    let ours = admin.patch(&path, json!({ "title": "Our guide", "body": "Rewritten here.", "baseVersion": 1 })).await;
    assert_eq!(ours["version"], 2);

    // Confluence moves on: the page is edited there, and gets a new child.
    fake.edit("2", "<p>Rewritten there.</p>");
    fake.add("7", "Troubleshooting", Some("2"), "<p>Turn it off and on.</p>");

    let report = admin.post("/confluence/imports/DOC/sync", json!({})).await;
    assert_eq!(counts(&report), (1, 0, 4, 1, 0));
    assert_eq!(report["gone"], json!([]));

    let after = admin.get(&path).await;
    assert_eq!(
        (&after["title"], &after["body"], &after["version"], &after["updatedAt"], &after["locked"]),
        (&ours["title"], &ours["body"], &ours["version"], &ours["updatedAt"], &json!(false))
    );

    // No second copy appeared, and the page keeps its place: old and new children hang under it.
    assert_eq!(
        tree(&admin, "DOC").await,
        [
            ("Home".to_string(), None),
            ("Our guide".to_string(), under("Home")),
            ("Install".to_string(), under("Our guide")),
            ("Troubleshooting".to_string(), under("Our guide")),
            ("Notes".to_string(), under("Home")),
            ("Archive".to_string(), under("Home")),
        ]
    );

    assert_eq!(counts(&admin.post("/confluence/imports/DOC/sync", json!({})).await), (0, 0, 5, 1, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_excluded_page_and_everything_under_it_is_not_stored() {
    let (_app, admin, fake) = connected().await;
    fill(&fake);

    // One page by title (in any case), one by id.
    let report = admin.post("/confluence/import", json!({ "spaceKey": "DOC", "exclude": [" guide ", "5", ""] })).await;
    assert_eq!(counts(&report), (2, 0, 0, 3, 0));
    assert_eq!(tree(&admin, "DOC").await, [("Home".to_string(), None), ("Notes".to_string(), under("Home"))]);
    assert_eq!(admin.get("/wiki/search?q=install").await, json!([]));
    assert_eq!(admin.get("/confluence/imports").await[0]["exclude"], json!(["guide", "5"]));

    // The exclusion is part of the import: a sync keeps to it, also for pages added later.
    fake.add("8", "Uninstall", Some("3"), "<p>Deep under the excluded page.</p>");
    fake.add("9", "Changelog", Some("1"), "<p>New and not excluded.</p>");

    let report = admin.post("/confluence/imports/DOC/sync", json!({})).await;
    assert_eq!(counts(&report), (1, 0, 2, 4, 0));
    assert_eq!(report["gone"], json!([]));
    assert_eq!(
        tree(&admin, "DOC").await,
        [("Home".to_string(), None), ("Notes".to_string(), under("Home")), ("Changelog".to_string(), under("Home"))]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_can_start_at_one_page_and_write_into_a_space_of_another_name() {
    let (_app, admin, fake) = connected().await;
    fill(&fake);
    fake.with(|upstream| upstream.spaces.push(("~jan".to_string(), "Jan's space".to_string())));

    let report = admin.post("/confluence/import", json!({ "spaceKey": "DOC", "rootPageId": "2", "targetSpaceKey": "guide" })).await;
    assert_eq!(report, json!({ "wikiSpaceKey": "GUIDE", "created": 2, "updated": 0, "unchanged": 0, "skipped": 0, "failed": [], "gone": [] }));
    assert_eq!(tree(&admin, "GUIDE").await, [("Guide".to_string(), None), ("Install".to_string(), under("Guide"))]);

    let imports = admin.get("/confluence/imports").await;
    assert_eq!((&imports[0]["wikiSpaceKey"], &imports[0]["spaceKey"], &imports[0]["rootPageId"]), (&json!("GUIDE"), &json!("DOC"), &json!("2")));
    assert_eq!(counts(&admin.post("/confluence/imports/GUIDE/sync", json!({})).await), (0, 0, 2, 0, 0));

    // The same Confluence space can be mirrored a second time, whole, under its own key.
    assert_eq!(counts(&admin.post("/confluence/import", json!({ "spaceKey": "DOC" })).await), (5, 0, 0, 0, 0));
    assert_eq!(admin.get("/confluence/imports").await.as_array().unwrap().len(), 2);

    let (status, message) =
        admin.refused(Method::POST, "/confluence/import", Some(json!({ "spaceKey": "DOC", "rootPageId": "404", "targetSpaceKey": "OTHER" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("no page with the id 404"), "{message}");

    // A Confluence key that cannot be a space key here needs a target; a local space is never overwritten.
    let (status, message) = admin.refused(Method::POST, "/confluence/import", Some(json!({ "spaceKey": "~jan" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("choose a target space key"), "{message}");
    assert_eq!(counts(&admin.post("/confluence/import", json!({ "spaceKey": "~jan", "targetSpaceKey": "JAN" })).await), (5, 0, 0, 0, 0));
    assert_eq!(admin.get("/wiki/spaces/JAN").await["name"], "Jan's space");

    admin.post("/wiki/spaces", json!({ "key": "LOCAL", "name": "Ours" })).await;
    fake.take_requests();
    assert_eq!(
        admin.refused(Method::POST, "/confluence/import", Some(json!({ "spaceKey": "DOC", "targetSpaceKey": "local" }))).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(fake.take_requests(), [], "refused before anything is fetched");

    assert_eq!(admin.refused(Method::POST, "/confluence/import", Some(json!({ "spaceKey": " " }))).await.0, StatusCode::BAD_REQUEST);

    let (status, message) = admin.refused(Method::POST, "/confluence/import", Some(json!({ "spaceKey": "NOPE" }))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(message.contains("check the address and the space key"), "{message}");
    assert_eq!(admin.refused(Method::GET, "/wiki/spaces/NOPE", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_that_cannot_be_stored_is_reported_and_the_rest_is_imported() {
    let (_app, admin, fake) = connected().await;
    fill(&fake);
    fake.add("10", "", Some("1"), "<p>No title upstream.</p>");
    fake.add("11", "Broken", Some("1"), "");
    fake.add("12", "Under the broken one", Some("11"), "<p>Fine.</p>");
    fake.with(|upstream| upstream.without_body.push("11".to_string()));

    let report = admin.post("/confluence/import", json!({ "spaceKey": "DOC" })).await;
    assert_eq!(counts(&report), (7, 0, 0, 0, 1));
    assert_eq!(report["failed"], json!([{ "title": "Broken", "reason": "Confluence sent no body for this page" }]));
    assert_eq!(page(&admin, "DOC", "Untitled").await["body"], "No title upstream.");
    assert_eq!(page(&admin, "DOC", "Under the broken one").await["parentId"], Value::Null);

    // Once Confluence sends the page, the next sync stores it and puts its child under it.
    fake.with(|upstream| upstream.without_body.clear());
    assert_eq!(counts(&admin.post("/confluence/imports/DOC/sync", json!({})).await), (1, 0, 7, 0, 0));
    assert_eq!(page(&admin, "DOC", "Under the broken one").await["parentId"], page(&admin, "DOC", "Broken").await["id"]);
}

#[test]
fn the_module_needs_the_knowledge_base() {
    let meta = ConfluenceModule.meta();

    assert_eq!((meta.id, meta.name, meta.depends_on, meta.default_enabled), ("confluence", "Confluence import", &["wiki"][..], true));
}
