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

async fn call_tool(machine: &mut Client, name: &str, arguments: Value) -> Value {
    let answer = machine
        .post("/api/m/agents/mcp", json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": arguments } }))
        .await;
    assert_eq!(answer["result"]["isError"], false, "{name}: {answer}");

    serde_json::from_str(answer["result"]["content"][0]["text"].as_str().expect("text content")).expect("the tool answers with JSON")
}

#[tokio::test(flavor = "multi_thread")]
async fn every_module_has_its_part_of_the_demo() {
    let mut client = demo().await;
    let enabled: Vec<String> =
        client.get("/api/bootstrap").await["modules"].as_array().unwrap().iter().map(|module| module["id"].as_str().unwrap().to_string()).collect();

    for id in ["github", "wiki", "confluence", "reports", "agents"] {
        assert!(enabled.iter().any(|module| module == id), "{id} is not loaded: {enabled:?}");
    }

    let spaces = client.get("/api/m/wiki/spaces").await;
    assert!(spaces.as_array().unwrap().iter().any(|space| space["key"] == "WEB"));

    let issue = client.get("/api/issues/WEB-10").await;
    assert!(issue["panels"]["wiki"]["pages"].as_array().is_some_and(|pages| !pages.is_empty()), "a page mentions WEB-10");
    assert_eq!(issue["panels"]["github"]["branches"][0]["name"], "feature/WEB-10-platnosc-blik");

    let burndown = client.get("/api/m/reports/projects/WEB/burndown").await;
    assert_eq!(burndown["sprint"]["name"], "WEB Sprint 2");
    assert!(burndown["days"].as_array().is_some_and(|days| days.len() >= 5));

    let hits = client.get("/api/quick?q=blik").await;
    let kinds: Vec<&str> = hits.as_array().unwrap().iter().map(|hit| hit["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"issue") && kinds.contains(&"page"), "quick search spans issues and pages: {kinds:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reaches_issues_code_and_pages_through_one_endpoint() {
    let mut client = demo().await;
    let token = client.post("/api/me/tokens", json!({ "name": "agent" })).await["token"].as_str().unwrap().to_string();
    let mut machine = Client::new(client.router.clone());
    machine.bearer = Some(token);

    let listed = machine.post("/api/m/agents/mcp", json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).await;
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();

    for tool in ["search_issues", "get_issue", "transition_issue", "get_issue_development", "search_pages", "get_page"] {
        assert!(names.contains(&tool), "{tool} is missing from {names:?}");
    }

    let development = call_tool(&mut machine, "get_issue_development", json!({ "key": "WEB-9" })).await;
    assert_eq!(development["prs"][0]["number"], 52);
    assert_eq!(development["drift"], json!(["changes_requested"]));

    let found = call_tool(&mut machine, "search_issues", json!({ "query": "project = WEB AND drift is not empty ORDER BY key" })).await;
    assert_eq!(found.to_string().matches("\"key\"").count(), 4);

    call_tool(&mut machine, "transition_issue", json!({ "key": "WEB-16", "status": "In Progress" })).await;
    call_tool(&mut machine, "add_comment", json!({ "key": "WEB-16", "body": "Picked up by the agent." })).await;

    let issue = client.get("/api/issues/WEB-16").await;
    assert_eq!(issue["comments"].as_array().unwrap().last().unwrap()["body"], "Picked up by the agent.");
    assert_eq!(issue["changes"].as_array().unwrap().last().unwrap()["newValue"], "In Progress");

    let mut stranger = Client::new(client.router.clone());
    assert_eq!(
        stranger.refused(Method::POST, "/api/m/agents/mcp", Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))).await,
        StatusCode::UNAUTHORIZED
    );
}
