mod common;

use axum::http::{Method, StatusCode};
use common::{test_app, Client};
use hub_server::{build_router, modules};
use serde_json::{json, Value};

async fn demo() -> Client {
    let mut client = Client::new(build_router(test_app(modules())));
    client.setup(true).await;

    client
}

/// Keys of the issues of a project that carry a badge of `module`.
async fn badged(client: &mut Client, project: &str, module: &str) -> Vec<String> {
    let found = client.get(&format!("/api/search?q=project%20%3D%20{project}&limit=200")).await;

    found["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .filter(|issue| !issue["ext"][module].is_null())
        .map(|issue| issue["key"].as_str().unwrap_or("").to_string())
        .collect()
}

fn disabled(projects: &Value, key: &str) -> Value {
    projects.as_array().expect("projects").iter().find(|project| project["key"] == key).unwrap_or_else(|| panic!("no project {key}"))
        ["disabledModules"]
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_switched_off_in_a_project_leaves_that_project_only() {
    let mut client = demo().await;

    assert!(!badged(&mut client, "WEB", "github").await.is_empty(), "the demo has pull requests on WEB issues");
    assert!(!badged(&mut client, "API", "github").await.is_empty(), "the demo has pull requests on API issues");
    assert!(client.get("/api/issues/WEB-10").await["panels"]["github"].is_object());

    let project = client.put("/api/projects/WEB/modules/github", json!({ "enabled": false })).await;
    assert_eq!(project["disabledModules"], json!(["github"]));

    let issue = client.get("/api/issues/WEB-10").await;
    assert!(issue["panels"]["github"].is_null(), "the panel is gone: {}", issue["panels"]);
    assert!(issue["panels"]["wiki"].is_object(), "other modules keep their panel");
    assert!(issue["ext"]["github"].is_null());
    assert_eq!(badged(&mut client, "WEB", "github").await, Vec::<String>::new());
    assert!(client.get("/api/projects/WEB/board").await.to_string().matches("\"github\"").count() == 0, "no badge on the board");

    assert!(!badged(&mut client, "API", "github").await.is_empty(), "the other project keeps its badges");
    assert!(client.get("/api/issues/API-2").await["panels"]["github"].is_object(), "and its panel");

    let projects = client.get("/api/bootstrap").await["projects"].clone();
    assert_eq!(disabled(&projects, "WEB"), json!(["github"]));
    assert_eq!(disabled(&projects, "API"), json!([]));
    assert!(client.get("/api/bootstrap").await["modules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|module| module["id"] == "github" && module["enabled"] == true));

    let project = client.put("/api/projects/WEB/modules/github", json!({ "enabled": true })).await;
    assert_eq!(project["disabledModules"], json!([]));
    assert!(client.get("/api/issues/WEB-10").await["panels"]["github"].is_object(), "switched back on, the panel returns");
    assert!(!badged(&mut client, "WEB", "github").await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_switched_off_in_a_project_leaves_that_project_out_of_my_work() {
    let mut client = demo().await;
    let repos = |section: &Value| -> Vec<String> {
        section["pulls"]
            .as_array()
            .map(|pulls| pulls.iter().map(|pull| pull["repo"].as_str().unwrap_or("").to_string()).collect())
            .unwrap_or_default()
    };

    let before = client.get("/api/dashboard").await["ext"]["github"].clone();
    assert!(repos(&before).iter().any(|repo| repo == "acme/sklep-web"), "the demo has my pull requests in WEB: {before}");

    client.put("/api/projects/WEB/modules/github", json!({ "enabled": false })).await;
    let after = client.get("/api/dashboard").await["ext"]["github"].clone();
    assert!(!repos(&after).iter().any(|repo| repo == "acme/sklep-web"), "WEB is out of the section: {after}");
    assert!(!after["drift"].to_string().contains("\"WEB-"), "and so are its issues: {after}");

    client.put("/api/projects/WEB/modules/github", json!({ "enabled": true })).await;
    assert_eq!(client.get("/api/dashboard").await["ext"]["github"], before);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_project_administrator_switches_modules() {
    let mut client = demo().await;
    let (mut member, _) = client.member("Ola Nowak", "ola@example.com").await;

    assert_eq!(member.refused(Method::PUT, "/api/projects/WEB/modules/github", Some(json!({ "enabled": false }))).await, StatusCode::FORBIDDEN);
    assert_eq!(client.get("/api/projects/WEB").await["disabledModules"], json!([]));

    assert_eq!(client.refused(Method::PUT, "/api/projects/WEB/modules/nothing", Some(json!({ "enabled": false }))).await, StatusCode::NOT_FOUND);
    assert_eq!(client.refused(Method::PUT, "/api/projects/NOPE/modules/github", Some(json!({ "enabled": false }))).await, StatusCode::NOT_FOUND);

    let mut stranger = Client::new(client.router.clone());
    assert_eq!(stranger.refused(Method::PUT, "/api/projects/WEB/modules/github", Some(json!({ "enabled": false }))).await, StatusCode::UNAUTHORIZED);
}
