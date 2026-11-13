mod common;

use axum::http::{Method, StatusCode};
use common::{id_of, issue, member, person, project, test_app, titles, Client};
use hub_app::db::{query_all, query_one};
use hub_app::services::issues;
use hub_app::{Actor, App, Module};
use hub_core::{Id, ProjectRole, Role};
use hub_mod_wiki::store::{self, ImportOutcome, ImportedPage, ImportedSpace};
use hub_mod_wiki::WikiModule;
use serde_json::{json, Value};

async fn space(client: &Client, key: &str, project_key: Option<&str>) -> Value {
    client.post("/spaces", json!({ "key": key, "name": format!("Space {key}"), "projectKey": project_key })).await
}

async fn page(client: &Client, space_key: &str, title: &str, body: &str, parent_id: Option<Id>) -> Value {
    client.post(&format!("/spaces/{space_key}/pages"), json!({ "title": title, "body": body, "parentId": parent_id })).await
}

fn link_count(app: &App, page_id: Id) -> i64 {
    app.read(|connection| {
        Ok(connection.query_row(
            "SELECT count(*) FROM link WHERE (from_kind = 'page' AND from_id = ?1) OR (to_kind = 'page' AND to_id = ?1)",
            [page_id.to_string()],
            |row| row.get(0),
        )?)
    })
    .expect("a count")
}

fn event_kinds(app: &App) -> Vec<(String, Option<Id>)> {
    app.read(|connection| {
        query_all(connection, "SELECT kind, project_id FROM event WHERE kind LIKE 'page.%' ORDER BY id", [], |row| Ok((row.get(0)?, row.get(1)?)))
    })
    .expect("events")
}

/// Puts a page into `space_key` the way an importing module does.
fn import_page(app: &App, space_key: &str, source_id: &str, version: i64, title: &str, body: &str, parent_id: Option<Id>) -> (Id, ImportOutcome) {
    app.write(Actor::system("Confluence"), |tx| {
        let space = store::upsert_space(
            tx,
            &ImportedSpace { key: space_key, name: "Imported", description: "", source: "confluence", source_ref: &json!({ "spaceKey": "DOC" }) },
        )?;
        let imported = store::upsert_imported_page(
            tx,
            &space,
            &ImportedPage { source_id, source_version: version, source_url: Some("https://confluence.example/x"), title, body, parent_id },
        )?;
        store::refresh_links(tx, &[imported.0])?;

        Ok(imported)
    })
    .expect("an imported page")
}

#[tokio::test(flavor = "multi_thread")]
async fn routes_need_a_signed_in_user() {
    let app = test_app();
    let mut stranger = person(&app, "Ada Admin", Role::Admin);
    stranger.bearer = None;

    assert_eq!(stranger.refused(Method::GET, "/spaces", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(stranger.refused(Method::GET, "/templates", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(stranger.refused(Method::POST, "/spaces", Some(json!({ "key": "DOCS", "name": "Docs" }))).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn spaces_are_created_listed_changed_and_deleted() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let member = person(&app, "Marek Member", Role::Member);

    let created = member.post("/spaces", json!({ "key": " docs ", "name": "  Team   docs ", "description": "How we work" })).await;

    assert_eq!(created["key"], "DOCS");
    assert_eq!(created["name"], "Team docs");
    assert_eq!(created["source"], "local");
    assert_eq!(created["projectKey"], Value::Null);
    assert_eq!(created["canEdit"], true);
    assert_eq!(created["canAdmin"], false);

    assert_eq!(member.refused(Method::POST, "/spaces", Some(json!({ "key": "docs", "name": "Again" }))).await.0, StatusCode::CONFLICT);
    assert_eq!(member.refused(Method::POST, "/spaces", Some(json!({ "key": "1x", "name": "Bad key" }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(member.refused(Method::POST, "/spaces", Some(json!({ "key": "OK", "name": "  " }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        member.refused(Method::POST, "/spaces", Some(json!({ "key": "OK", "name": "x", "projectKey": "NOPE" }))).await.0,
        StatusCode::NOT_FOUND
    );

    page(&member, "docs", "First", "", None).await;
    page(&admin, "DOCS", "Second", "", None).await;

    let listed = admin.get("/spaces").await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["pageCount"], 2);
    assert_eq!(listed[0]["canAdmin"], true);
    assert_eq!(member.get("/spaces/docs").await["pageCount"], 2);

    let renamed = member.patch("/spaces/DOCS", json!({ "name": "Handbook", "description": "" })).await;
    assert_eq!(renamed["name"], "Handbook");
    assert_eq!(renamed["description"], "");

    assert_eq!(member.refused(Method::DELETE, "/spaces/DOCS", None).await.0, StatusCode::FORBIDDEN);
    admin.delete("/spaces/DOCS").await;
    assert_eq!(admin.get("/spaces").await, json!([]));
    assert_eq!(admin.refused(Method::GET, "/spaces/DOCS/tree", None).await.0, StatusCode::NOT_FOUND);

    let leftovers: i64 = app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM wiki_fts", [], |row| row.get(0))?)).unwrap();
    assert_eq!(leftovers, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bound_space_follows_the_access_of_its_project() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let lead = person(&app, "Lena Lead", Role::Member);
    let viewer = person(&app, "Wit Viewer", Role::Member);
    let outsider = person(&app, "Olga Outsider", Role::Member);
    project(&app, &lead, "SEC", true);
    member(&app, &lead, "SEC", &viewer, ProjectRole::Viewer);

    assert_eq!(
        outsider.refused(Method::POST, "/spaces", Some(json!({ "key": "SEC", "name": "x", "projectKey": "SEC" }))).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        viewer.refused(Method::POST, "/spaces", Some(json!({ "key": "SEC", "name": "x", "projectKey": "SEC" }))).await.0,
        StatusCode::FORBIDDEN
    );

    let created = space(&lead, "SEC", Some("sec")).await;
    assert_eq!(created["projectKey"], "SEC");
    assert_eq!(created["canAdmin"], true);

    let secret = id_of(&page(&lead, "SEC", "Klucze produkcyjne", "Hasło do sejfu: tajne", None).await);

    // A viewer of the project reads but does not write.
    assert_eq!(viewer.get("/spaces").await[0]["canEdit"], false);
    assert_eq!(viewer.get(&format!("/pages/{secret}")).await["title"], "Klucze produkcyjne");
    assert_eq!(viewer.refused(Method::POST, "/spaces/SEC/pages", Some(json!({ "title": "Nope" }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        viewer.refused(Method::PATCH, &format!("/pages/{secret}"), Some(json!({ "body": "x", "baseVersion": 1 }))).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(viewer.refused(Method::DELETE, &format!("/pages/{secret}"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        viewer.refused(Method::POST, &format!("/pages/{secret}/move"), Some(json!({ "parentId": null, "position": 0 }))).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(viewer.refused(Method::PATCH, "/spaces/SEC", Some(json!({ "name": "Mine" }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(viewer.refused(Method::DELETE, "/spaces/SEC", None).await.0, StatusCode::FORBIDDEN);

    // For somebody outside a private project the space does not exist.
    assert_eq!(outsider.get("/spaces").await, json!([]));

    for path in [
        "/spaces/SEC".to_string(),
        "/spaces/SEC/tree".to_string(),
        format!("/pages/{secret}"),
        format!("/pages/{secret}/revisions"),
        format!("/pages/{secret}/diff"),
    ] {
        assert_eq!(outsider.refused(Method::GET, &path, None).await.0, StatusCode::NOT_FOUND, "{path}");
    }

    assert_eq!(
        outsider.refused(Method::PATCH, &format!("/pages/{secret}"), Some(json!({ "body": "x", "baseVersion": 1 }))).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(outsider.get("/search?q=sejfu").await, json!([]));
    assert_eq!(viewer.get("/search?q=sejfu").await.as_array().unwrap().len(), 1);
    assert_eq!(app.read(|connection| WikiModule.quick_search(connection, &outsider.user, "klucze")).unwrap(), Vec::new());

    // Unbinding makes the space visible to everyone, so it takes an instance administrator.
    assert_eq!(lead.refused(Method::PATCH, "/spaces/SEC", Some(json!({ "projectKey": null }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(admin.patch("/spaces/SEC", json!({ "projectKey": null })).await["projectKey"], Value::Null);
    assert_eq!(outsider.get("/spaces").await[0]["key"], "SEC");

    admin.patch("/spaces/SEC", json!({ "projectKey": "SEC" })).await;
    lead.delete("/spaces/SEC").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_form_a_tree_that_can_be_rearranged() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    space(&admin, "DOCS", None).await;
    space(&admin, "OTHER", None).await;

    let guide = id_of(&page(&admin, "DOCS", "Guide", "", None).await);
    let install = id_of(&page(&admin, "DOCS", "  Install   notes ", "", Some(guide)).await);
    let usage = id_of(&page(&admin, "DOCS", "Usage", "", Some(guide)).await);
    let faq = id_of(&page(&admin, "DOCS", "FAQ", "", None).await);
    let deep = id_of(&page(&admin, "DOCS", "Linux", "", Some(install)).await);
    let elsewhere = id_of(&page(&admin, "OTHER", "Elsewhere", "", None).await);

    let tree = admin.get("/spaces/DOCS/tree").await;
    assert_eq!(titles(&tree), ["Guide", "Install notes", "Linux", "Usage", "FAQ"]);
    assert_eq!(
        tree[1],
        json!({ "id": install, "parentId": guide, "title": "Install notes", "position": 0, "updatedAt": tree[1]["updatedAt"], "locked": false })
    );

    let detail = admin.get(&format!("/pages/{deep}")).await;
    assert_eq!(titles(&detail["breadcrumbs"]), ["Guide", "Install notes"]);
    assert_eq!(detail["space"]["key"], "DOCS");
    assert_eq!(detail["version"], 1);
    assert_eq!(titles(&admin.get(&format!("/pages/{guide}")).await["children"]), ["Install notes", "Usage"]);

    assert_eq!(admin.refused(Method::POST, "/spaces/DOCS/pages", Some(json!({ "title": " " }))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        admin.refused(Method::POST, "/spaces/DOCS/pages", Some(json!({ "title": "x", "parentId": elsewhere }))).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(admin.refused(Method::POST, "/spaces/DOCS/pages", Some(json!({ "title": "x", "parentId": 9999 }))).await.0, StatusCode::BAD_REQUEST);

    // Usage moves in front of Install notes; FAQ moves under Guide, in the middle.
    admin.post(&format!("/pages/{usage}/move"), json!({ "parentId": guide, "position": 0 })).await;
    let moved = admin.post(&format!("/pages/{faq}/move"), json!({ "parentId": guide, "position": 1 })).await;
    assert_eq!(moved["parentId"], guide);
    assert_eq!(titles(&admin.get("/spaces/DOCS/tree").await), ["Guide", "Usage", "FAQ", "Install notes", "Linux"]);

    // A position past the end means "last"; null means the top level.
    admin.post(&format!("/pages/{install}/move"), json!({ "parentId": null, "position": 99 })).await;
    assert_eq!(titles(&admin.get("/spaces/DOCS/tree").await), ["Guide", "Usage", "FAQ", "Install notes", "Linux"]);
    admin.post(&format!("/pages/{install}/move"), json!({ "parentId": null, "position": 0 })).await;
    assert_eq!(titles(&admin.get("/spaces/DOCS/tree").await), ["Install notes", "Linux", "Guide", "Usage", "FAQ"]);

    let (status, message) = admin.refused(Method::POST, &format!("/pages/{install}/move"), Some(json!({ "parentId": deep, "position": 0 }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("descendants"), "{message}");
    assert_eq!(
        admin.refused(Method::POST, &format!("/pages/{install}/move"), Some(json!({ "parentId": install, "position": 0 }))).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        admin.refused(Method::POST, &format!("/pages/{install}/move"), Some(json!({ "parentId": elsewhere, "position": 0 }))).await.0,
        StatusCode::BAD_REQUEST
    );

    // Deleting a page with children needs to be asked for.
    assert_eq!(admin.refused(Method::DELETE, &format!("/pages/{install}"), None).await.0, StatusCode::CONFLICT);
    assert_eq!(admin.refused(Method::DELETE, &format!("/pages/{install}?recursive=false"), None).await.0, StatusCode::CONFLICT);
    assert_eq!(admin.delete(&format!("/pages/{install}?recursive=true")).await, json!({ "deleted": 2 }));
    assert_eq!(admin.delete(&format!("/pages/{faq}")).await, json!({ "deleted": 1 }));
    assert_eq!(titles(&admin.get("/spaces/DOCS/tree").await), ["Guide", "Usage"]);
    assert_eq!(admin.refused(Method::GET, &format!("/pages/{deep}"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(admin.get("/search?q=linux").await, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn every_save_is_a_version_and_stale_saves_are_refused() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let other = person(&app, "Marek Member", Role::Member);
    space(&admin, "DOCS", None).await;
    let id = id_of(&page(&admin, "DOCS", "Deploy", "step one\nstep two\nstep three", None).await);
    let path = format!("/pages/{id}");

    let second = other.patch(&path, json!({ "body": "step one\nstep 2\nstep three\nstep four", "baseVersion": 1, "message": "Clearer steps" })).await;
    assert_eq!(second["version"], 2);
    assert_eq!(second["authorId"], admin.user.id);
    assert_eq!(second["updatedBy"], other.user.id);

    // The first editor still has version 1 open.
    let (status, message) = admin.refused(Method::PATCH, &path, Some(json!({ "body": "mine", "baseVersion": 1 }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("version 2"), "{message}");
    assert_eq!(admin.refused(Method::PATCH, &path, Some(json!({ "body": "mine" }))).await.0, StatusCode::UNPROCESSABLE_ENTITY);

    // Saving what is already there is not a new version.
    assert_eq!(admin.patch(&path, json!({ "title": "Deploy", "baseVersion": 2 })).await["version"], 2);

    let third = admin.patch(&path, json!({ "title": "Deployment", "baseVersion": 2 })).await;
    assert_eq!((third["version"].as_i64(), third["title"].as_str()), (Some(3), Some("Deployment")));
    assert_eq!(admin.refused(Method::PATCH, &path, Some(json!({ "title": "  ", "baseVersion": 3 }))).await.0, StatusCode::BAD_REQUEST);

    let revisions = admin.get(&format!("{path}/revisions")).await;
    let summary: Vec<(i64, &str, &str)> = revisions
        .as_array()
        .unwrap()
        .iter()
        .map(|revision| (revision["version"].as_i64().unwrap(), revision["title"].as_str().unwrap(), revision["message"].as_str().unwrap()))
        .collect();
    assert_eq!(summary, [(3, "Deployment", ""), (2, "Deploy", "Clearer steps"), (1, "Deploy", "")]);
    assert_eq!(revisions[1]["authorId"], other.user.id);
    assert!(revisions[0].get("body").is_none());

    let first = admin.get(&format!("{path}/revisions/1")).await;
    assert_eq!(first["body"], "step one\nstep two\nstep three");
    assert_eq!(admin.refused(Method::GET, &format!("{path}/revisions/9"), None).await.0, StatusCode::NOT_FOUND);

    let diff = admin.get(&format!("{path}/diff?from=1&to=2")).await;
    let lines: Vec<String> = diff["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| format!("{}:{}", line["kind"].as_str().unwrap(), line["text"].as_str().unwrap()))
        .collect();
    assert_eq!(lines, ["same:step one", "removed:step two", "added:step 2", "same:step three", "added:step four"]);

    // Without bounds the diff is the latest version against the one before.
    let latest = admin.get(&format!("{path}/diff")).await;
    assert_eq!((latest["from"].as_i64(), latest["to"].as_i64()), (Some(2), Some(3)));
    assert_eq!((latest["fromTitle"].as_str(), latest["toTitle"].as_str()), (Some("Deploy"), Some("Deployment")));
    assert_eq!(admin.refused(Method::GET, &format!("{path}/diff?from=1&to=7"), None).await.0, StatusCode::NOT_FOUND);

    let restored = other.post(&format!("{path}/restore"), json!({ "version": 1 })).await;
    assert_eq!(restored["version"], 4);
    assert_eq!(restored["title"], "Deploy");
    assert_eq!(restored["body"], "step one\nstep two\nstep three");
    assert_eq!(admin.get(&format!("{path}/revisions")).await[0]["message"], "Restored version 1");
    assert_eq!(other.post(&format!("{path}/restore"), json!({ "version": 4 })).await["version"], 4);
    assert_eq!(other.refused(Method::POST, &format!("{path}/restore"), Some(json!({ "version": 12 }))).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirrored_page_is_read_only_until_it_is_detached() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    project(&app, &admin, "WEB", false);
    let (id, outcome) = import_page(&app, "MIR", "c-100", 3, "  Imported   page ", "- [ ] From Confluence", None);
    let path = format!("/pages/{id}");

    assert_eq!(outcome, ImportOutcome::Created);

    let detail = admin.get(&path).await;
    assert_eq!(detail["locked"], true);
    assert_eq!(detail["title"], "Imported page");
    assert_eq!(detail["sourceId"], "c-100");
    assert_eq!(detail["sourceVersion"], 3);
    assert_eq!(detail["authorId"], Value::Null);
    assert_eq!(detail["space"]["source"], "confluence");
    assert_eq!(admin.get("/spaces/MIR/tree").await[0]["locked"], true);
    assert_eq!(admin.get(&format!("{path}/revisions")).await[0]["authorLabel"], "Confluence");

    let (status, message) = admin.refused(Method::PATCH, &path, Some(json!({ "body": "mine", "baseVersion": 1 }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(message.contains("detach it to edit"), "{message}");
    assert_eq!(admin.refused(Method::POST, &format!("{path}/restore"), Some(json!({ "version": 1 }))).await.0, StatusCode::CONFLICT);
    assert_eq!(admin.refused(Method::POST, &format!("{path}/move"), Some(json!({ "parentId": null, "position": 0 }))).await.0, StatusCode::CONFLICT);
    assert_eq!(
        admin.refused(Method::POST, &format!("{path}/issues"), Some(json!({ "projectKey": "WEB", "lines": [0] }))).await.0,
        StatusCode::CONFLICT
    );

    // The same version again changes nothing; a newer one becomes a new revision.
    assert_eq!(import_page(&app, "MIR", "c-100", 3, "Imported page", "ignored", None), (id, ImportOutcome::Unchanged));
    assert_eq!(admin.get(&path).await["version"], 1);
    assert_eq!(import_page(&app, "MIR", "c-100", 4, "Imported page", "From Confluence, edited", None), (id, ImportOutcome::Updated));

    let updated = admin.get(&path).await;
    assert_eq!((updated["version"].as_i64(), updated["sourceVersion"].as_i64(), updated["locked"].as_bool()), (Some(2), Some(4), Some(true)));
    assert_eq!(admin.get(&format!("{path}/revisions")).await[0]["message"], "Version 4 from confluence");

    let detached = admin.post(&format!("{path}/detach"), json!({})).await;
    assert_eq!(detached["locked"], false);
    assert_eq!((&detached["sourceId"], &detached["sourceVersion"], &detached["sourceUrl"]), (&Value::Null, &Value::Null, &Value::Null));
    assert_eq!(admin.patch(&path, json!({ "body": "Ours now", "baseVersion": 2 })).await["version"], 3);

    // The importer is told the page was detached and leaves it alone, whatever the version.
    assert_eq!(import_page(&app, "MIR", "c-100", 9, "Imported page", "Upstream moved on", None), (id, ImportOutcome::Detached));
    assert_eq!(admin.get(&path).await["body"], "Ours now");
    let space_id = id_of(&admin.get("/spaces/MIR").await);
    assert_eq!(app.read(|connection| store::detached_sources(connection, space_id)).unwrap().get("c-100"), Some(&id));
    assert!(app.read(|connection| store::imported_versions(connection, space_id)).unwrap().is_empty());

    // A local space is never turned into a mirror.
    space(&admin, "LOCAL", None).await;
    let refused = app.write(Actor::system("Confluence"), |tx| {
        store::upsert_space(tx, &ImportedSpace { key: "LOCAL", name: "x", description: "", source: "confluence", source_ref: &json!({}) })
    });
    assert_eq!(refused.unwrap_err().status, StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirrored_page_follows_its_source_in_the_tree() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let (home, _) = import_page(&app, "MIR", "1", 1, "Home", "", None);
    let (guide, _) = import_page(&app, "MIR", "2", 1, "Guide", "", Some(home));
    let (notes, _) = import_page(&app, "MIR", "3", 1, "Notes", "", Some(guide));

    assert_eq!(titles(&admin.get("/spaces/MIR/tree").await), ["Home", "Guide", "Notes"]);

    // Upstream moved Notes directly under Home without a new version.
    assert_eq!(import_page(&app, "MIR", "3", 1, "Notes", "", Some(home)).1, ImportOutcome::Unchanged);

    let moved = admin.get(&format!("/pages/{notes}")).await;
    assert_eq!((moved["parentId"].as_i64(), moved["version"].as_i64()), (Some(home), Some(1)));

    // A parent under the page itself would close a loop; the page goes to the top instead.
    import_page(&app, "MIR", "1", 2, "Home", "", Some(notes));
    assert_eq!(admin.get(&format!("/pages/{home}")).await["parentId"], Value::Null);
    assert_eq!(titles(&admin.get("/spaces/MIR/tree").await), ["Home", "Guide", "Notes"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn issue_keys_in_the_body_become_links_and_show_on_the_issue() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let lead = person(&app, "Lena Lead", Role::Member);
    let outsider = person(&app, "Olga Outsider", Role::Member);
    project(&app, &admin, "WEB", false);
    project(&app, &lead, "SEC", true);
    let card = issue(&app, &admin, "WEB", "Płatność kartą");
    let blik = issue(&app, &admin, "WEB", "Płatność BLIK");
    let secret = issue(&app, &lead, "SEC", "Rotacja kluczy");
    space(&admin, "DOCS", None).await;

    let body = format!("Karty: {card}. BLIK: {blik}, jeszcze raz {card}. Tajne: {secret}. Nie ma: WEB-99, web-1, UTF-8.");
    let created = page(&lead, "DOCS", "Płatności", &body, None).await;
    let id = id_of(&created);
    let path = format!("/pages/{id}");
    let linked = |detail: &Value| -> Vec<(String, String)> {
        detail["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| (entry["issue"]["key"].as_str().unwrap().to_string(), entry["relation"].as_str().unwrap().to_string()))
            .collect()
    };
    let mentions = |key: &str| (key.to_string(), "mentions".to_string());
    let documents = |key: &str| (key.to_string(), "documents".to_string());

    assert_eq!(linked(&created), [mentions(&secret), mentions(&card), mentions(&blik)]);
    assert_eq!(created["issues"][1]["issue"]["summary"], "Płatność kartą");

    // Issues of a project the reader cannot see are left out.
    assert_eq!(linked(&outsider.get(&path).await), [mentions(&card), mentions(&blik)]);

    // An explicit link is stronger than a mention and survives edits of the body.
    assert_eq!(linked(&lead.put(&format!("{path}/issues/{}", card.to_lowercase())).await), [mentions(&secret), documents(&card), mentions(&blik)]);
    lead.put(&format!("{path}/issues/{card}")).await;
    assert_eq!(outsider.refused(Method::PUT, &format!("{path}/issues/{secret}"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(lead.refused(Method::PUT, &format!("{path}/issues/WEB-99"), None).await.0, StatusCode::NOT_FOUND);

    let edited = lead.patch(&path, json!({ "body": format!("Już tylko {blik}."), "baseVersion": 1 })).await;
    assert_eq!(linked(&edited), [documents(&card), mentions(&blik)]);

    // The issue's page lists the pages, for those who may see them.
    let panel = |viewer: &Client, key: &str| -> Value {
        app.read(|connection| {
            let (issue_id, _) = issues::issue_id_by_key(connection, key)?.expect("the issue exists");
            let row = issues::row_by_id(&app, connection, issue_id)?;

            Ok(WikiModule.issue_panel(&app, connection, &row, &viewer.user)?.expect("a panel"))
        })
        .unwrap()
    };
    let on_card = panel(&admin, &card);
    assert_eq!(
        on_card,
        json!({ "pages": [{ "pageId": id, "spaceKey": "DOCS", "title": "Płatności", "relation": "documents", "updatedAt": edited["updatedAt"] }] })
    );
    assert_eq!(panel(&admin, &blik)["pages"][0]["relation"], "mentions");
    assert_eq!(panel(&admin, &secret), json!({ "pages": [] }));

    space(&lead, "SEC", Some("SEC")).await;
    page(&lead, "SEC", "Procedura", &format!("Opisuje {card}."), None).await;
    assert_eq!(panel(&lead, &card)["pages"].as_array().unwrap().len(), 2);
    assert_eq!(panel(&outsider, &card)["pages"].as_array().unwrap().len(), 1);

    assert_eq!(linked(&lead.delete(&format!("{path}/issues/{card}")).await), [mentions(&blik)]);

    // Deleting an issue takes its links along, so a later issue with the same id inherits nothing.
    app.write(admin.user.actor(), |tx| {
        let (issue_id, _) = issues::issue_id_by_key(tx, &blik)?.expect("the issue exists");

        issues::delete_issue(tx, issue_id).map(|_| ())
    })
    .unwrap();
    assert_eq!(link_count(&app, id), 0);
    assert_eq!(linked(&lead.get(&path).await), []);
}

#[tokio::test(flavor = "multi_thread")]
async fn wiki_links_become_backlinks_whatever_the_order_of_writing() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    space(&admin, "DOCS", None).await;
    space(&admin, "TEAM", None).await;
    let backlinks = |detail: &Value| -> Vec<String> {
        detail["backlinks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|link| format!("{}:{}", link["spaceKey"].as_str().unwrap(), link["title"].as_str().unwrap()))
            .collect()
    };

    // The overview links to pages that do not exist yet.
    let overview = id_of(
        &page(&admin, "DOCS", "Przegląd", "Zobacz [[Płatności]], [[płatności|drugi raz]], [[TEAM:Zasady pracy]] i [[Brak takiej]].", None).await,
    );
    let payments = page(&admin, "DOCS", "Płatności", "Wróć do [[Przegląd]]. Sam do siebie: [[Płatności]].", None).await;
    let rules = page(&admin, "TEAM", "Zasady pracy", "Bez linków.", None).await;
    let namesake = page(&admin, "TEAM", "Płatności", "Inna strona o tym samym tytule, w innej przestrzeni.", None).await;

    assert_eq!(backlinks(&payments), ["DOCS:Przegląd"]);
    assert_eq!(backlinks(&rules), ["DOCS:Przegląd"]);
    assert_eq!(backlinks(&namesake), Vec::<String>::new());
    assert_eq!(backlinks(&admin.get(&format!("/pages/{overview}")).await), ["DOCS:Płatności"]);
    assert_eq!(link_count(&app, overview), 3);

    // A rename breaks the links by the old title and picks up those by the new one.
    let renamed = admin.patch(&format!("/pages/{}", id_of(&payments)), json!({ "title": "Brak takiej", "baseVersion": 1 })).await;
    assert_eq!(backlinks(&renamed), ["DOCS:Przegląd"]);
    assert_eq!(link_count(&app, overview), 3);
    admin.patch(&format!("/pages/{}", id_of(&payments)), json!({ "title": "Rozliczenia", "baseVersion": 2 })).await;
    assert_eq!(backlinks(&admin.get(&format!("/pages/{}", id_of(&payments))).await), Vec::<String>::new());

    // Removing the link from the body removes the backlink; deleting a page removes its links.
    admin.patch(&format!("/pages/{overview}"), json!({ "body": "Zobacz [[TEAM:Zasady pracy]].", "baseVersion": 1 })).await;
    assert_eq!(backlinks(&admin.get(&format!("/pages/{}", id_of(&rules))).await), ["DOCS:Przegląd"]);
    admin.delete(&format!("/pages/{overview}")).await;
    assert_eq!(backlinks(&admin.get(&format!("/pages/{}", id_of(&rules))).await), Vec::<String>::new());
    assert_eq!(link_count(&app, overview), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn search_ignores_diacritics_and_shows_where_the_words_are() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    space(&admin, "DOCS", None).await;
    space(&admin, "TEAM", None).await;
    let blik = id_of(&page(&admin, "DOCS", "BLIK — kanał powiadomień", "Wstęp.\n\nPłatność BLIK wymaga osobnego kanału powiadomień.", None).await);
    page(&admin, "DOCS", "Żółta kartka", "Zupełnie o czym innym, choć raz pada słowo płatność.", None).await;
    page(&admin, "TEAM", "Zasady", "Płatności za szkolenia zatwierdza lider.", None).await;

    let hits = admin.get("/search?q=platnosc%20blik").await;
    assert_eq!(
        hits,
        json!([{ "pageId": blik, "spaceKey": "DOCS", "title": "BLIK — kanał powiadomień", "snippet": "Wstęp. <mark>Płatność</mark> <mark>BLIK</mark> wymaga osobnego kanału powiadomień." }])
    );

    // Words match as prefixes; a hit in the title counts for more than one in the body.
    assert_eq!(titles(&admin.get("/search?q=PŁATN").await).len(), 3);
    assert_eq!(titles(&admin.get("/search?q=kartka%20platnosc").await), ["Żółta kartka"]);
    assert_eq!(titles(&admin.get("/search?q=zolta").await), ["Żółta kartka"]);
    assert_eq!(titles(&admin.get("/search?q=p%C5%82atno&space=team").await), ["Zasady"]);
    assert_eq!(admin.get("/search?q=nieistniejace").await, json!([]));
    assert_eq!(admin.get("/search?q=%22%20*%20").await, json!([]));
    assert_eq!(admin.get("/search").await, json!([]));

    // An edit is searchable at once, and the old words are gone.
    admin.patch(&format!("/pages/{blik}"), json!({ "body": "Teraz o przelewach.", "baseVersion": 1 })).await;
    assert_eq!(titles(&admin.get("/search?q=przelewach").await), ["BLIK — kanał powiadomień"]);
    assert_eq!(titles(&admin.get("/search?q=osobnego").await), Vec::<String>::new());

    let quick = app.read(|connection| WikiModule.quick_search(connection, &admin.user, "kanal")).unwrap();
    assert_eq!(quick.len(), 1);
    assert_eq!((quick[0].kind.as_str(), quick[0].url.clone(), quick[0].subtitle.as_str()), ("page", format!("/wiki/DOCS/{blik}"), "Space DOCS"));
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_start_a_page() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    space(&admin, "DOCS", None).await;

    let templates = admin.get("/templates").await;
    let ids: Vec<&str> = templates.as_array().unwrap().iter().map(|template| template["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["decision-record", "meeting-notes", "runbook", "technical-spec", "retrospective"]);
    assert!(templates
        .as_array()
        .unwrap()
        .iter()
        .all(|template| !template["name"].as_str().unwrap().is_empty() && template["body"].as_str().unwrap().contains("## ")));

    let notes = admin.post("/spaces/DOCS/pages", json!({ "title": "Weekly", "templateId": "meeting-notes" })).await;
    assert_eq!(notes["body"], templates[1]["body"]);
    assert_eq!(admin.post("/spaces/DOCS/pages", json!({ "title": "Own words", "templateId": "runbook", "body": "Mine" })).await["body"], "Mine");
    assert_eq!(admin.post("/spaces/DOCS/pages", json!({ "title": "Empty" })).await["body"], "");
    assert_eq!(
        admin.refused(Method::POST, "/spaces/DOCS/pages", Some(json!({ "title": "x", "templateId": "nope" }))).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn task_lines_become_issues() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let viewer = person(&app, "Wit Viewer", Role::Member);
    project(&app, &admin, "WEB", false);
    project(&app, &admin, "OPS", false);
    member(&app, &admin, "OPS", &viewer, ProjectRole::Viewer);
    let existing = issue(&app, &admin, "WEB", "Already there");
    space(&admin, "DOCS", None).await;

    let body = format!("# Planowanie\r\n\r\n- [ ] Zamówić klucze   API  \r\n  * [ ] Przygotować dane testowe\r\n- [x] Ustalić termin\r\n- [ ] {existing} już zgłoszone\r\nZwykła linia");
    let id = id_of(&page(&viewer, "DOCS", "Planowanie [sprint 2]", &body, None).await);
    let path = format!("/pages/{id}/issues");

    for (lines, expected) in [
        (json!([6]), "Line 7 is not an open task"),
        (json!([4]), "Line 5 is not an open task"),
        (json!([40]), "The page has no line 41"),
        (json!([]), "Choose at least one task"),
    ] {
        let (status, message) = viewer.refused(Method::POST, &path, Some(json!({ "projectKey": "WEB", "lines": lines }))).await;
        assert_eq!((status, message.as_str()), (StatusCode::BAD_REQUEST, expected));
    }

    let (status, message) = viewer.refused(Method::POST, &path, Some(json!({ "projectKey": "WEB", "lines": [2, 5] }))).await;
    assert_eq!((status, message), (StatusCode::BAD_REQUEST, format!("Line 6 already belongs to {existing}")));
    assert_eq!(viewer.refused(Method::POST, &path, Some(json!({ "projectKey": "OPS", "lines": [2] }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(viewer.refused(Method::POST, &path, Some(json!({ "projectKey": "NOPE", "lines": [2] }))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        viewer.refused(Method::POST, &path, Some(json!({ "projectKey": "WEB", "lines": [2], "baseVersion": 7 }))).await.0,
        StatusCode::CONFLICT
    );

    // Nothing was created by the refused requests, not even by the one that failed on its second line.
    let count = |app: &App| -> i64 { app.read(|connection| Ok(connection.query_row("SELECT count(*) FROM issue", [], |row| row.get(0))?)).unwrap() };
    assert_eq!(count(&app), 1);

    let result = viewer.post(&path, json!({ "projectKey": "web", "lines": [3, 2, 3], "baseVersion": 1 })).await;
    assert_eq!(result["keys"], json!(["WEB-2", "WEB-3"]));
    assert_eq!(
        result["page"]["body"],
        format!("# Planowanie\r\n\r\n- [ ] WEB-2 Zamówić klucze   API\r\n  * [ ] WEB-3 Przygotować dane testowe\r\n- [x] Ustalić termin\r\n- [ ] {existing} już zgłoszone\r\nZwykła linia")
    );
    assert_eq!(result["page"]["version"], 2);

    let keys: Vec<&str> = result["page"]["issues"].as_array().unwrap().iter().map(|entry| entry["issue"]["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["WEB-1", "WEB-2", "WEB-3"]);
    assert_eq!(result["page"]["issues"][1]["issue"]["summary"], "Zamówić klucze API");
    assert_eq!(result["page"]["issues"][1]["issue"]["reporterId"], viewer.user.id);
    assert_eq!(viewer.get(&format!("/pages/{id}/revisions")).await[0]["message"], "Created WEB-2, WEB-3 from tasks");

    let description: String =
        app.read(|connection| Ok(connection.query_row("SELECT description FROM issue WHERE key = 'WEB-3'", [], |row| row.get(0))?)).unwrap();
    assert_eq!(description, format!("Created from a task on the knowledge base page [Planowanie \\[sprint 2\\]](/wiki/DOCS/{id})."));

    // A line that was converted cannot be converted again.
    assert_eq!(viewer.refused(Method::POST, &path, Some(json!({ "projectKey": "WEB", "lines": [2] }))).await.0, StatusCode::BAD_REQUEST);

    // An issue type of the project can be asked for.
    let bug: Id = app
        .read(|connection| {
            Ok(query_one(
                connection,
                "SELECT id FROM issue_type WHERE name = 'Bug' AND project_id = (SELECT id FROM project WHERE key = 'WEB')",
                [],
                |row| row.get(0),
            )?
            .unwrap())
        })
        .unwrap();
    let other = id_of(&page(&admin, "DOCS", "Bugs", "- [ ] Koszyk gubi rabat", None).await);
    let result = admin.post(&format!("/pages/{other}/issues"), json!({ "projectKey": "WEB", "typeId": bug, "lines": [0] })).await;
    assert_eq!(result["page"]["issues"][0]["issue"]["typeId"], bug);
    assert_eq!(result["page"]["body"], "- [ ] WEB-4 Koszyk gubi rabat");
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_are_announced_with_the_project_of_the_space() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let project_id = project(&app, &admin, "WEB", false);
    space(&admin, "WEB", Some("WEB")).await;
    space(&admin, "FREE", None).await;
    let mut stream = app.subscribe();

    let bound = id_of(&page(&admin, "WEB", "Bound", "", None).await);
    let free = id_of(&page(&admin, "FREE", "Free", "", None).await);
    admin.patch(&format!("/pages/{bound}"), json!({ "body": "edited", "baseVersion": 1 })).await;
    admin.patch(&format!("/pages/{bound}"), json!({ "body": "edited", "baseVersion": 2 })).await;
    admin.post(&format!("/pages/{bound}/move"), json!({ "parentId": null, "position": 0 })).await;
    admin.delete(&format!("/pages/{free}")).await;
    admin.delete(&format!("/pages/{bound}")).await;

    assert_eq!(
        event_kinds(&app),
        [
            ("page.created".to_string(), Some(project_id)),
            ("page.created".to_string(), None),
            ("page.updated".to_string(), Some(project_id)),
            ("page.updated".to_string(), Some(project_id)),
            ("page.deleted".to_string(), None),
            ("page.deleted".to_string(), Some(project_id)),
        ]
    );

    let first = stream.try_recv().expect("the event reached the live stream");
    assert_eq!((first.kind.as_str(), first.actor_id), ("page.created", Some(admin.user.id)));
    assert_eq!(first.payload, json!({ "pageId": bound, "spaceKey": "WEB", "title": "Bound" }));

    let edited = stream.next_of_kind("page.updated");
    assert_eq!(edited, json!({ "pageId": bound, "spaceKey": "WEB", "title": "Bound", "change": "edited" }));
}

trait NextOfKind {
    fn next_of_kind(&mut self, kind: &str) -> Value;
}

impl NextOfKind for tokio::sync::broadcast::Receiver<hub_app::Event> {
    fn next_of_kind(&mut self, kind: &str) -> Value {
        loop {
            let event = self.try_recv().expect("an event of that kind");

            if event.kind == kind {
                return event.payload;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_work_through_the_tools_with_the_access_of_their_user() {
    let app = test_app();
    let admin = person(&app, "Ada Admin", Role::Admin);
    let lead = person(&app, "Lena Lead", Role::Member);
    let outsider = person(&app, "Olga Outsider", Role::Member);
    project(&app, &lead, "SEC", true);
    space(&lead, "SEC", Some("SEC")).await;
    let call = |who: &Client, name: &str, arguments: Value| WikiModule.call_tool(&app, &who.user, name, &arguments).expect("a tool of the wiki");

    let names: Vec<&str> = WikiModule.tools().iter().map(|tool| tool.name).collect();
    assert_eq!(names, ["search_pages", "get_page", "create_page", "update_page"]);
    assert_eq!(WikiModule.tools().iter().map(|tool| tool.read_only).collect::<Vec<_>>(), [true, true, false, false]);
    assert!(WikiModule.call_tool(&app, &admin.user, "create_issue", &json!({})).is_none());

    let created = call(&lead, "create_page", json!({ "spaceKey": "sec", "title": "Rotacja kluczy", "body": "Co kwartał." })).unwrap();
    let id = id_of(&created);
    let child = call(&lead, "create_page", json!({ "spaceKey": "SEC", "title": "Checklist", "body": "", "parentId": id })).unwrap();
    assert_eq!(child["parentId"], id);

    assert_eq!(call(&lead, "get_page", json!({ "id": id })).unwrap()["children"][0]["title"], "Checklist");
    assert_eq!(call(&lead, "search_pages", json!({ "query": "kwartal" })).unwrap()[0]["pageId"], id);
    assert_eq!(call(&lead, "search_pages", json!({ "query": "kwartal", "space": "NOPE" })).unwrap(), json!([]));

    let updated = call(&lead, "update_page", json!({ "id": id, "body": "Co miesiąc.", "baseVersion": 1 })).unwrap();
    assert_eq!((updated["version"].as_i64(), updated["body"].as_str()), (Some(2), Some("Co miesiąc.")));
    assert_eq!(call(&lead, "update_page", json!({ "id": id, "body": "Stale.", "baseVersion": 1 })).unwrap_err().status, StatusCode::CONFLICT);
    assert_eq!(call(&lead, "update_page", json!({ "id": id })).unwrap_err().status, StatusCode::BAD_REQUEST);

    assert_eq!(call(&outsider, "get_page", json!({ "id": id })).unwrap_err().status, StatusCode::NOT_FOUND);
    assert_eq!(call(&outsider, "search_pages", json!({ "query": "kwartal miesiac" })).unwrap(), json!([]));
    assert_eq!(call(&outsider, "create_page", json!({ "spaceKey": "SEC", "title": "x", "body": "" })).unwrap_err().status, StatusCode::NOT_FOUND);
    assert_eq!(call(&outsider, "update_page", json!({ "id": id, "body": "x", "baseVersion": 2 })).unwrap_err().status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demo_has_a_knowledge_base_tied_to_its_issues() {
    let app = test_app();
    let admin = person(&app, "Bartek Admin", Role::Admin);
    hub_app::seed::seed_demo(&app, admin.user.id, hub_core::DemoLanguage::Pl).expect("the demo data");

    let spaces = admin.get("/spaces").await;
    let summary: Vec<(&str, &str, Option<&str>)> = spaces
        .as_array()
        .unwrap()
        .iter()
        .map(|space| (space["key"].as_str().unwrap(), space["name"].as_str().unwrap(), space["projectKey"].as_str()))
        .collect();
    assert_eq!(summary, [("WEB", "Sklep — baza wiedzy", Some("WEB")), ("TEAM", "Zespół", None)]);
    assert_eq!((spaces[0]["pageCount"].as_i64(), spaces[1]["pageCount"].as_i64()), (Some(11), Some(2)));

    let tree = admin.get("/spaces/WEB/tree").await;
    assert_eq!(
        titles(&tree),
        [
            "Architektura sklepu",
            "Płatności",
            "BLIK — kanał powiadomień",
            "Bramka kartowa: sandbox",
            "Decyzje",
            "ADR-001: Sesje w Redis",
            "ADR-002: Jedna bramka dla kart i BLIK",
            "Spotkania",
            "Retrospektywa sprintu 1",
            "Planowanie sprintu 2 — notatki",
            "Wdrożenie produkcyjne",
        ]
    );

    let id_named = |title: &str| id_of(tree.as_array().unwrap().iter().find(|node| node["title"] == title).expect("the page"));
    let blik = admin.get(&format!("/pages/{}", id_named("BLIK — kanał powiadomień"))).await;
    assert_eq!(blik["version"], 3);
    assert_eq!(titles(&blik["breadcrumbs"]), ["Płatności"]);
    assert_eq!(blik["issues"][0]["issue"]["key"], "WEB-10");
    assert_eq!(blik["issues"][0]["issue"]["summary"], "Płatność BLIK");
    assert_eq!(admin.get(&format!("/pages/{}/revisions", id_of(&blik))).await.as_array().unwrap().len(), 3);
    assert!(blik["updatedAt"].as_str().unwrap() < hub_app::db::now_iso().as_str());
    assert!(titles(&blik["backlinks"]).contains(&"Płatności".to_string()));

    // The pages mention the issues the demo talks about, and link among themselves.
    let mentioned: Vec<String> = app
        .read(|connection| {
            query_all(
                connection,
                "SELECT DISTINCT i.key FROM link l JOIN issue i ON i.id = CAST(l.to_id AS INTEGER)
                 WHERE l.from_kind = 'page' AND l.to_kind = 'issue' AND l.relation = 'mentions' ORDER BY i.number",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(mentioned, ["WEB-6", "WEB-7", "WEB-9", "WEB-10", "WEB-11", "WEB-13", "WEB-17"]);

    let architecture = admin.get(&format!("/pages/{}", id_named("Architektura sklepu"))).await;
    let mut backlinks: Vec<String> = architecture["backlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|link| format!("{}:{}", link["spaceKey"].as_str().unwrap(), link["title"].as_str().unwrap()))
        .collect();
    backlinks.sort();
    assert_eq!(backlinks, ["TEAM:Onboarding nowej osoby", "WEB:Wdrożenie produkcyjne"]);

    let panel = app
        .read(|connection| {
            let (issue_id, _) = issues::issue_id_by_key(connection, "WEB-9")?.expect("the issue exists");

            WikiModule.issue_panel(&app, connection, &issues::row_by_id(&app, connection, issue_id)?, &admin.user)
        })
        .unwrap()
        .expect("a panel");
    let mut on_card: Vec<&str> = panel["pages"].as_array().unwrap().iter().map(|page| page["title"].as_str().unwrap()).collect();
    on_card.sort_unstable();
    assert_eq!(on_card, ["ADR-002: Jedna bramka dla kart i BLIK", "Bramka kartowa: sandbox", "Planowanie sprintu 2 — notatki", "Płatności"]);

    // The meeting notes have open tasks ready to become issues.
    let notes = admin.get(&format!("/pages/{}", id_named("Planowanie sprintu 2 — notatki"))).await;
    let task = notes["body"].as_str().unwrap().split('\n').position(|line| line.starts_with("- [ ] ")).expect("an open task");
    let converted = admin.post(&format!("/pages/{}/issues", id_of(&notes)), json!({ "projectKey": "WEB", "lines": [task] })).await;
    assert_eq!(converted["keys"], json!(["WEB-30"]));

    assert_eq!(titles(&admin.get("/search?q=sandbox%20karty").await)[0], "Bramka kartowa: sandbox");

    // Seeding twice adds nothing.
    WikiModule.seed_demo(&app, hub_core::DemoLanguage::Pl).unwrap();
    assert_eq!(admin.get("/spaces").await.as_array().unwrap().len(), 2);
}
