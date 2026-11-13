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

/// Calls a tool over MCP and says whether it failed.
async fn tool_fails(client: &mut Client, name: &str, key: &str) -> bool {
    let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": { "key": key } } });
    let message = client.post("/api/m/agents/mcp", call).await;

    message["result"]["isError"].as_bool().unwrap_or_else(|| panic!("{name} for {key}: {message}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_switched_off_in_a_project_does_not_answer_for_that_project() {
    let mut client = demo().await;
    let of = |keys: &[String], project: &str| keys.iter().filter(|key| key.starts_with(project)).count();
    let status = |code: StatusCode| code;

    let with_pulls = client.search_keys("pr is not empty").await;
    assert!(of(&with_pulls, "WEB-") > 0 && of(&with_pulls, "API-") > 0, "the demo has pull requests in both projects: {with_pulls:?}");
    assert!(!tool_fails(&mut client, "get_issue_development", "WEB-10").await);

    client.put("/api/projects/WEB/modules/github", json!({ "enabled": false })).await;

    // Its query fields match nothing there, its routes and its tools do not know the project.
    let with_pulls = client.search_keys("pr is not empty").await;
    assert_eq!((of(&with_pulls, "WEB-"), of(&with_pulls, "API-") > 0), (0, true), "{with_pulls:?}");
    assert_eq!(status(client.refused(Method::GET, "/api/m/github/projects/WEB", None).await), StatusCode::NOT_FOUND);
    assert_eq!(status(client.refused(Method::POST, "/api/m/github/issues/WEB-10/branch", Some(json!({ "repoId": 1 }))).await), StatusCode::NOT_FOUND);
    assert_eq!(status(client.refused(Method::GET, "/api/m/github/projects/API", None).await), StatusCode::OK);
    assert!(tool_fails(&mut client, "get_issue_development", "WEB-10").await);
    assert!(!tool_fails(&mut client, "get_issue_development", "API-2").await);

    // Everything else goes on as before: the tracker, and the other modules.
    assert_eq!(client.get("/api/issues/WEB-10").await["key"], "WEB-10");
    assert_eq!(status(client.refused(Method::GET, "/api/m/reports/projects/WEB/velocity", None).await), StatusCode::OK);
    assert_eq!(status(client.refused(Method::GET, "/api/m/agents/issues/WEB-10/runs", None).await), StatusCode::OK);
    assert!(!tool_fails(&mut client, "get_issue", "WEB-10").await, "agents still work in the project");

    // With the agents switched off in the project, no agent reaches it, by any tool.
    client.put("/api/projects/WEB/modules/agents", json!({ "enabled": false })).await;
    assert!(tool_fails(&mut client, "get_issue", "WEB-10").await);
    assert!(!tool_fails(&mut client, "get_issue", "API-2").await);
    assert_eq!(status(client.refused(Method::GET, "/api/m/agents/issues/WEB-10/runs", None).await), StatusCode::NOT_FOUND);
    assert_eq!(
        status(client.refused(Method::POST, "/api/m/agents/issues/WEB-10/runs", Some(json!({ "kind": "refine" }))).await),
        StatusCode::NOT_FOUND
    );
    assert_eq!(client.get("/api/issues/WEB-10").await["key"], "WEB-10");

    client.put("/api/projects/WEB/modules/agents", json!({ "enabled": true })).await;
    client.put("/api/projects/WEB/modules/github", json!({ "enabled": true })).await;
    assert!(!tool_fails(&mut client, "get_issue_development", "WEB-10").await);
    assert!(of(&client.search_keys("pr is not empty").await, "WEB-") > 0);
}
