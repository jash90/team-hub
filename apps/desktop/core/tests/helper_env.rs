//! Alone in its own test binary, because it changes the environment of the process.

mod support;

use hub_desktop_core::model::{EventKind, NewThread, ThreadKind, ThreadState};
use support::{open, repo, FakeHub};

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_never_inherits_the_apps_token_or_the_persons_other_tokens() {
    std::env::set_var("TEAM_HUB_TOKEN", support::TOKEN);
    std::env::set_var("GH_TOKEN", "ghp_not_for_agents");

    let hub = FakeHub::start().await;
    let bench = open("env");
    let repo = repo("env");

    bench.connect(&hub).await;
    bench.map(&repo).await;

    let new = NewThread {
        issue_key: "WEB-1".into(),
        kind: ThreadKind::Custom,
        instructions: "[fake:env]".into(),
        model: None,
        repo_id: None,
        branch: None,
    };
    let id = bench.desktop.run(move |core| core.start_thread(&new)).await.unwrap().id;

    bench.until_state(id, ThreadState::Done).await;

    let detail = bench.desktop.run(move |core| core.thread(id)).await.unwrap();
    let said = detail.events.iter().rev().find(|event| event.kind == EventKind::Message).unwrap();

    assert_eq!(said.text, "In my environment: PATH");
}
