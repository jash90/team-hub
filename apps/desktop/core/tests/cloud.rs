//! The headless runner of docs/cloud-runner.md against the fake team-hub, a stand-in for
//! GitHub that wants a token, real git and the fake agent: configuration, clones on demand,
//! the changes a person looks at, and pushes and pull requests decided on the web.

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use hub_desktop_core::model::{EventKind, ProposalState, Thread, ThreadState};
use serde_json::{json, Value};
use support::cloud::{github, holds, open_headless, open_headless_in, GitHost, GITHUB_TOKEN, MODEL_KEY};
use support::{git, write, Bench, FakeHub, TempDir, TOKEN};

const BRANCH: &str = "feature/WEB-1-login-form-validation";

async fn ready(label: &str) -> (Bench, FakeHub, GitHost) {
    let hub = FakeHub::start().await;
    let host = github(label).await;
    let bench = open_headless(label, &hub, &host, &[], |_| {}).await;

    (bench, hub, host)
}

/// The thread a delegated run became, once its repository is cloned.
async fn thread_of(bench: &Bench, run_id: i64) -> Thread {
    for _ in 0..200 {
        bench.desktop.tick(true).await.unwrap();

        let found = bench.desktop.run(move |core| Ok(core.threads()?.into_iter().find(|thread| thread.run_id == Some(run_id)))).await.unwrap();

        if let Some(thread) = found {
            return thread;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("run {run_id} never became a thread");
}

async fn carried_out(bench: &Bench, hub: &FakeHub, instructions: &str) -> (i64, Thread) {
    let run_id = hub.delegate_in("WEB-1", "implement", instructions, Some("Acme/Web"));
    let id = thread_of(bench, run_id).await.id;

    (run_id, bench.until_state(id, ThreadState::Done).await)
}

/// Rounds of the runner until the fake server shows what is waited for.
async fn until(bench: &Bench, hub: &FakeHub, what: &str, shown: impl Fn(&FakeHub) -> bool) {
    for _ in 0..200 {
        bench.desktop.tick(true).await.unwrap();

        if shown(hub) {
            return;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("the server never showed: {what}");
}

fn said(bench_events: &[hub_desktop_core::model::ThreadEvent], start: &str) -> String {
    bench_events
        .iter()
        .find(|event| event.kind == EventKind::Message && event.text.starts_with(start))
        .map(|event| event.text.clone())
        .unwrap_or_default()
}

// ---------- configuration and presence ----------

#[tokio::test(flavor = "multi_thread")]
async fn the_runner_announces_itself_headless_with_what_the_environment_says() {
    let hub = FakeHub::start().await;
    let host = github("announce").await;
    let more = [("RUNNER_NAME", "Railway runner"), ("RUNNER_KINDS", "implement,review"), ("RUNNER_PROJECTS", "web"), ("RUNNER_MAX_RUNNING", "3")];
    let bench = open_headless("announce", &hub, &host, &more, |_| {}).await;

    bench.desktop.tick(true).await.unwrap();

    let runner = hub.state().runners[0].clone();

    assert_eq!(runner["interactive"], false);
    assert_eq!(
        (&runner["name"], &runner["kinds"], &runner["projects"]),
        (&json!("Railway runner"), &json!(["implement", "review"]), &json!(["WEB"]))
    );
    assert!(runner["runnerId"].as_str().unwrap().starts_with("runner-"));
    assert!(runner.get("pinnedOnly").is_none());

    let settings = bench.desktop.core().settings();

    assert_eq!((settings.max_running, settings.allow_unsandboxed_shell, settings.runner_enabled), (3, false, true));

    // The id is the installation's: the same after a restart on the same data.
    let id = settings.runner_id.clone();
    let Bench { desktop, data, .. } = bench;

    drop(desktop);

    let again = open_headless_in(data, &hub, &host, &[], |_| {}).await;

    assert_eq!(again.desktop.core().settings().runner_id, id);

    // Without a list of projects it works for every project the person can edit: none is sent.
    again.desktop.tick(true).await.unwrap();
    assert!(hub.state().runners.iter().any(|runner| runner["name"] == "Cloud runner" && runner.get("projects").is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_model_key_no_agent_run_is_claimed() {
    let hub = FakeHub::start().await;
    let host = github("no-key").await;
    let bench = open_headless("no-key", &hub, &host, &[("ANTHROPIC_API_KEY", "")], |_| {}).await;
    let run_id = hub.delegate_in("WEB-1", "implement", "", Some("acme/web"));

    for _ in 0..5 {
        bench.desktop.tick(true).await.unwrap();
    }

    assert_eq!(hub.run(run_id)["run"]["state"], "queued");
    assert_eq!(hub.state().runners[0]["kinds"], json!([]));
}

// ---------- repositories ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_repository_is_cloned_when_a_run_first_names_it_and_the_token_is_left_nowhere() {
    let (bench, hub, host) = ready("clone").await;
    let (run_id, thread) = carried_out(&bench, &hub, "").await;
    let clone = bench.data.path().join("repos/acme/web");

    // The clone is where the document says, its origin is the plain address, and the run's
    // project is mapped to it without anybody doing so.
    assert_eq!(PathBuf::from(&thread.repo_path), clone);
    assert_eq!(git(&clone, &["remote", "get-url", "origin"]), format!("{}/acme/web.git", host.url));
    assert_eq!(
        std::fs::read_to_string(clone.join(".git/config")).unwrap().matches("credential").count()
            + std::fs::read_to_string(clone.join(".git/config")).unwrap().matches("extraheader").count(),
        0
    );

    let mapped = bench.desktop.run(|core| core.repos()).await.unwrap();

    assert_eq!((mapped[0].mapping.project_key.as_str(), mapped[0].mapping.github_repo.as_deref()), ("WEB", Some("acme/web")));
    assert!(thread.worktree.as_deref().unwrap().starts_with(bench.data.path().join("worktrees").to_str().unwrap()));
    assert_eq!(hub.run(run_id)["run"]["branch"], BRANCH);

    // GitHub was asked with the token (and refused the one request without it)…
    assert!(host.authorised_requests() >= 2, "{:?}", host.requests.lock().unwrap());
    assert!(std::process::Command::new("git")
        .args(["-c", "credential.helper=", "ls-remote", &format!("{}/acme/web.git", host.url)])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .is_ok_and(|output| !output.status.success()));

    // …and a second run fetches: it starts from what main has become meanwhile.
    let newer = host.commit_on_main("src/later.ts", "export const later = true;\n", "Later on main");
    let second = hub.delegate_in("WEB-2", "implement", "", Some("acme/web"));
    let other = thread_of(&bench, second).await;
    let other = bench.until_state(other.id, ThreadState::Done).await;

    assert_eq!(other.base_commit.as_deref(), Some(newer.as_str()));
    assert_eq!(std::fs::read_dir(bench.data.path().join("repos/acme")).unwrap().count(), 1);

    // After a clone, two fetches and the threads' work, none of the secrets is in any file
    // under the data folder: no config, no remote address, no log, no database.
    for secret in [GITHUB_TOKEN, TOKEN, MODEL_KEY] {
        assert_eq!(holds(bench.data.path(), secret), Vec::<PathBuf>::new(), "{secret}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_names_no_repository_or_one_that_cannot_be_cloned_fails_with_the_reason() {
    let (bench, hub, _host) = ready("no-repo").await;
    let nameless = hub.delegate("WEB-1", "implement", "");
    let unknown = hub.delegate_in("WEB-2", "implement", "", Some("acme/missing"));
    let odd = hub.delegate_in("WEB-2", "implement", "", Some("../../etc"));

    until(&bench, &hub, "three failed runs", |hub| [nameless, unknown, odd].iter().all(|id| hub.run(*id)["run"]["state"] == "failed")).await;

    assert!(hub.run(nameless)["run"]["summary"].as_str().unwrap().contains("names no repository"));
    assert!(hub.run(unknown)["run"]["summary"].as_str().unwrap().starts_with("The runner could not start this run: git clone"));
    assert!(hub.run(odd)["run"]["summary"].as_str().unwrap().contains("is not a repository name"));
    assert!(!bench.data.path().join("repos/acme/missing").exists());
}

// ---------- what a person looks at, and decides, on the web ----------

#[tokio::test(flavor = "multi_thread")]
async fn what_the_thread_committed_is_uploaded_and_again_when_it_commits_more() {
    let (bench, hub, _host) = ready("changes").await;
    let (run_id, thread) = carried_out(&bench, &hub, "").await;
    let tree = PathBuf::from(thread.worktree.clone().unwrap());
    let head = git(&tree, &["rev-parse", "HEAD"]);
    let changes = hub.state().changes[&run_id].clone();

    assert_eq!((&changes["base"], &changes["head"]), (&json!(thread.base_commit), &json!(head)));
    assert_eq!(changes["commits"], json!([{ "sha": head, "subject": "WEB-1: implement Login form validation", "author": "Fake Agent" }]));
    assert_eq!(changes["files"], json!([{ "path": "agent-notes/WEB-1.md", "status": "added", "additions": 5, "deletions": 0 }]));
    assert!(changes["patch"].as_str().unwrap().starts_with("diff --git a/agent-notes/WEB-1.md b/agent-notes/WEB-1.md"));
    assert_eq!(changes["truncated"], false);
    assert_eq!(hub.run(run_id)["run"]["hasChanges"], true);

    // The changes were there before the proposals that ask for a decision on them.
    assert_eq!(hub.proposal_on_run(run_id, "push")["changesWereThere"], true);
    assert_eq!(hub.proposal_on_run(run_id, "pull_request")["changesWereThere"], true);
    // The push proposal names the commit it would push.
    assert_eq!(
        hub.proposal_on_run(run_id, "push")["payload"],
        json!({ "branch": BRANCH, "note": "One commit, tests not run (no test suite).", "commit": head })
    );

    // The thread commits more: the run gets the new head, with both commits.
    write(&tree, "later.txt", "added later\n");
    git(&tree, &["add", "--all"]);
    git(&tree, &["commit", "-m", "WEB-1: later"]);

    let later = git(&tree, &["rev-parse", "HEAD"]);

    until(&bench, &hub, "the newer head", |hub| hub.state().changes[&run_id]["head"] == later.as_str()).await;
    assert_eq!(hub.state().changes[&run_id]["commits"].as_array().unwrap().len(), 2);
    assert_eq!(hub.state().changes[&run_id]["files"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_approved_on_the_web_sends_exactly_the_commit_that_was_shown() {
    let (bench, hub, host) = ready("web-push").await;
    let (run_id, thread) = carried_out(&bench, &hub, "").await;
    let tree = PathBuf::from(thread.worktree.clone().unwrap());
    let shown = git(&tree, &["rev-parse", "HEAD"]);
    let push = hub.proposal_on_run(run_id, "push");

    // Nothing has left: the run waits for a person, GitHub has only main.
    assert_eq!((hub.run(run_id)["run"]["state"].as_str(), host.refs().as_str()), (Some("waiting"), "refs/heads/main"));
    assert_eq!(push["state"], "pending");

    // The thread adds a commit after the proposal was made; then the person approves.
    write(&tree, "later.txt", "added after the proposal\n");
    git(&tree, &["add", "--all"]);
    git(&tree, &["commit", "-m", "WEB-1: later"]);
    hub.decide_like_the_web(push["id"].as_i64().unwrap(), true).await.unwrap();

    assert_eq!((&hub.proposal_on_run(run_id, "push")["state"], &hub.proposal_on_run(run_id, "push")["result"]), (&json!("approved"), &Value::Null));

    until(&bench, &hub, "the push reported", |hub| !hub.proposal_on_run(run_id, "push")["result"].is_null()).await;

    // The commit that was shown is on GitHub; the later one is not; main is untouched.
    assert_eq!(hub.proposal_on_run(run_id, "push")["result"], json!({ "commit": shown }));
    assert_eq!(host.tip(BRANCH), shown);
    assert_ne!(git(&tree, &["rev-parse", "HEAD"]), shown);
    assert_eq!(host.tip("main"), thread.base_commit.clone().unwrap());

    // Reported once, through the headless route; the desktop's own way was not tried.
    let written = hub.written();

    assert_eq!(written.iter().filter(|line| line.ends_with("/result")).count(), 1);
    assert!(!written.iter().any(|line| line.ends_with("/settle")));

    let detail = bench.desktop.run(move |core| core.thread(thread.id)).await.unwrap();

    assert_eq!(detail.proposals[1].state, ProposalState::Approved);
    assert!(detail.events.iter().any(|event| event.text.starts_with(&format!("Pushed {BRANCH} at "))));
    // The pull request is still undecided, so the run goes on waiting.
    bench.desktop.tick(true).await.unwrap();
    assert_eq!(hub.run(run_id)["run"]["state"], "waiting");

    for secret in [GITHUB_TOKEN, TOKEN] {
        assert_eq!(holds(bench.data.path(), secret), Vec::<PathBuf>::new(), "{secret}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn what_would_need_a_second_confirmation_is_not_pushed_and_the_run_says_why() {
    let (bench, hub, host) = ready("web-risky").await;
    let (run_id, thread) = carried_out(&bench, &hub, "[fake:risky]").await;
    let push = hub.proposal_on_run(run_id, "push");

    hub.decide_like_the_web(push["id"].as_i64().unwrap(), true).await.unwrap();
    until(&bench, &hub, "the push refused", |hub| hub.proposal_on_run(run_id, "push")["state"] == "failed").await;

    let note = hub.proposal_on_run(run_id, "push")["note"].as_str().unwrap().to_string();

    assert!(note.starts_with("Not pushed."), "{note}");
    assert!(note.contains(".github/workflows/ci.yml: it changes what CI or the build runs") && note.contains("src/config.txt"), "{note}");
    assert_eq!(host.refs(), "refs/heads/main");
    assert_eq!(hub.state().writes.iter().filter(|(_, path, body)| path.ends_with("/result") && body["error"].is_string()).count(), 1);

    let proposals = bench.desktop.run(move |core| core.thread(thread.id)).await.unwrap().proposals;

    assert_eq!((proposals[0].state, proposals[0].note.as_deref()), (ProposalState::Failed, Some(note.as_str())));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pull_request_approved_on_the_web_is_opened_through_the_server_once_the_branch_is_pushed() {
    let (bench, hub, host) = ready("web-pr").await;

    hub.state().issues.get_mut("WEB-1").unwrap().panels.insert("github".into(), json!({ "prs": [], "repos": [{ "id": 3, "fullName": "Acme/Web" }] }));

    let (run_id, thread) = carried_out(&bench, &hub, "").await;
    let (push, pull_request) = (hub.proposal_on_run(run_id, "push"), hub.proposal_on_run(run_id, "pull_request"));

    // Approved first, while the push is undecided: the pull request waits, it does not fail.
    hub.decide_like_the_web(pull_request["id"].as_i64().unwrap(), true).await.unwrap();

    for _ in 0..4 {
        bench.desktop.tick(true).await.unwrap();
    }

    assert_eq!(
        (&hub.proposal_on_run(run_id, "pull_request")["state"], &hub.proposal_on_run(run_id, "pull_request")["result"]),
        (&json!("approved"), &Value::Null)
    );
    assert!(!hub.written().iter().any(|line| line.ends_with("/pull-request")));

    hub.decide_like_the_web(push["id"].as_i64().unwrap(), true).await.unwrap();
    until(&bench, &hub, "the pull request reported", |hub| !hub.proposal_on_run(run_id, "pull_request")["result"].is_null()).await;

    assert_eq!(hub.proposal_on_run(run_id, "pull_request")["result"], json!({ "url": "https://github.com/acme/web/pull/12" }));
    assert!(!host.tip(BRANCH).is_empty());

    let asked = hub.state().writes.iter().find(|(_, path, _)| path.ends_with("/pull-request")).map(|(_, _, body)| body.clone()).unwrap();

    assert_eq!(asked, json!({ "repoId": 3, "head": BRANCH, "base": "main", "title": "WEB-1 Login form validation", "body": "Implements WEB-1." }));

    // Everything is decided: the run ends with the branch and the pull request.
    until(&bench, &hub, "the run ended", |hub| hub.run(run_id)["run"]["state"] == "succeeded").await;
    assert_eq!(hub.run(run_id)["run"]["pullRequestUrl"], "https://github.com/acme/web/pull/12");
    assert_eq!(bench.thread(thread.id).await.pull_request_url.as_deref(), Some("https://github.com/acme/web/pull/12"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pull_request_the_server_cannot_open_and_a_push_that_is_rejected() {
    let (bench, hub, host) = ready("web-reject").await;
    let (run_id, thread) = carried_out(&bench, &hub, "").await;
    let (push, pull_request) = (hub.proposal_on_run(run_id, "push"), hub.proposal_on_run(run_id, "pull_request"));

    // Rejected: marked here, and nothing is pushed or reported.
    hub.decide_like_the_web(push["id"].as_i64().unwrap(), false).await.unwrap();
    until(&bench, &hub, "the rejection taken over", |_| {
        let id = thread.id;

        futures_free::block(bench.desktop.run(move |core| core.thread(id))).is_ok_and(|detail| detail.proposals[1].state == ProposalState::Rejected)
    })
    .await;

    assert_eq!(host.refs(), "refs/heads/main");
    assert!(!hub.written().iter().any(|line| line.ends_with("/result") || line.ends_with("/settle")));

    // The pull request is approved although nothing was pushed: it fails with the way out.
    hub.decide_like_the_web(pull_request["id"].as_i64().unwrap(), true).await.unwrap();
    until(&bench, &hub, "the pull request refused", |hub| hub.proposal_on_run(run_id, "pull_request")["state"] == "failed").await;

    assert_eq!(hub.proposal_on_run(run_id, "pull_request")["note"], format!("Branch {BRANCH} is not on GitHub: its push has to be approved first."));
}

mod futures_free {
    /// Waits for a future from inside a synchronous closure of an async test.
    pub fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
    }
}

// ---------- safety ----------

#[tokio::test(flavor = "multi_thread")]
async fn the_helper_gets_the_model_key_a_home_of_its_own_and_none_of_the_runners_tokens() {
    let (bench, hub, _host) = ready("helper-env").await;
    let (_, thread) = carried_out(&bench, &hub, "[fake:environment]").await;
    let events = bench.desktop.run(move |core| core.thread(thread.id)).await.unwrap().events;
    let names: Vec<String> = said(&events, "Variables: ").trim_start_matches("Variables: ").split(' ').map(String::from).collect();

    for withheld in [
        "TEAM_HUB_TOKEN",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "RUNNER_DATA_DIR",
        "TEAM_HUB_URL",
        "TEAM_HUB_GIT_PASSWORD",
        "AWS_SECRET_ACCESS_KEY",
        "CLAUDE_CODE_MESSAGING_TOKEN",
    ] {
        assert!(!names.contains(&withheld.to_string()), "{withheld} reached the helper: {names:?}");
    }

    assert!(names.contains(&"PATH".to_string()) && names.contains(&"ANTHROPIC_API_KEY".to_string()));
    assert!(
        names.iter().all(|name| hub_desktop_core::policy::PASSED_TO_A_HEADLESS_HELPER.contains(&name.as_str())
            || ["HOME", "ANTHROPIC_API_KEY"].contains(&name.as_str())),
        "{names:?}"
    );
    assert_eq!(said(&events, "HOME="), format!("HOME={}", bench.data.path().join("agent-home").display()));
    assert_eq!(said(&events, "Model key: "), format!("Model key: {} characters, ending in {}", MODEL_KEY.len(), &MODEL_KEY[MODEL_KEY.len() - 8..]));
}

async fn permissions(label: &str, more: &[(&str, &str)], sandbox: hub_desktop_core::model::SandboxState) -> (Bench, String, String) {
    let hub = FakeHub::start().await;
    let host = github(label).await;
    let bench = open_headless(label, &hub, &host, more, |options| options.sandbox = Some(sandbox)).await;

    // Another repository's clone and another thread's tree, to be closed to this thread.
    std::fs::create_dir_all(bench.data.path().join("repos/other/site/.git")).unwrap();
    std::fs::create_dir_all(bench.data.path().join("worktrees/site/web-9-t9")).unwrap();

    let (_, thread) = carried_out(&bench, &hub, "[fake:permissions]").await;
    let events = bench.desktop.run(move |core| core.thread(thread.id)).await.unwrap().events;

    (bench, said(&events, "Allowed: "), said(&events, "Closed to the shell: "))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_threads_file_tools_are_tied_to_its_tree_and_the_shell_is_there_only_when_allowed_or_confined() {
    use hub_desktop_core::model::SandboxState::{Available, Unavailable};

    // No sandbox, the default: reading, editing and writing in the tree, and no shell.
    let (bench, allowed, closed) = permissions("perm-default", &[], Unavailable).await;
    let tree = std::fs::read_dir(bench.data.path().join("worktrees/web")).unwrap().next().unwrap().unwrap().path();
    let rule = format!("/{}/**", tree.display());

    assert_eq!(allowed, format!("Allowed: Read({rule}) | Edit({rule}) | Write({rule})"));
    assert_eq!(closed, "Closed to the shell: no sandbox");

    // RUNNER_ALLOW_SHELL=1 is what adds the shell where there is no sandbox.
    let (_bench, allowed, closed) = permissions("perm-shell", &[("RUNNER_ALLOW_SHELL", "1")], Unavailable).await;

    assert!(allowed.ends_with(" | Bash") && allowed.starts_with("Allowed: Read(//"), "{allowed}");
    assert_eq!(closed, "Closed to the shell: no sandbox");

    // With a working sandbox the shell is there without the switch, and it is shown neither
    // the runner's state, nor the jobs, nor another clone, nor another thread's tree.
    let (bench, allowed, closed) = permissions("perm-sandbox", &[], Available).await;
    let data = bench.data.path();

    assert!(allowed.ends_with(" | Bash"), "{allowed}");

    for folder in ["state", "jobs", "agent-home", "repos/other", "worktrees/site"] {
        assert!(closed.split(" | ").any(|path| Path::new(path) == data.join(folder)), "{folder} is open: {closed}");
    }

    for own in ["repos/acme/web", "repos/acme", "worktrees/web"] {
        assert!(!closed.split(" | ").any(|path| Path::new(path) == data.join(own)), "{own} is closed: {closed}");
    }
}

// ---------- the desktop app is not changed by any of this ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_runner_with_a_person_at_it_announces_and_decides_as_before() {
    let hub = FakeHub::start().await;
    let bench = support::open("still-desktop");
    let repo = support::repo("still-desktop");

    bench.connect(&hub).await;
    bench.map(&repo).await;

    let run_id = hub.delegate("WEB-1", "implement", "");
    let thread = thread_of(&bench, run_id).await;
    let thread = bench.until_state(thread.id, ThreadState::Done).await;
    let runner = hub.state().runners[0].clone();
    let mut fields: Vec<&str> = runner.as_object().unwrap().keys().map(String::as_str).collect();

    fields.sort_unstable();
    assert_eq!(fields, vec!["kinds", "name", "projects", "runnerId", "version"]);
    assert_eq!(runner["projects"], json!(["WEB"]));

    // Nothing is uploaded, the web cannot decide its push, and the app settles it itself.
    assert!(hub.state().changes.is_empty());

    let push = hub.proposal_on_run(run_id, "push");

    assert!(hub.decide_like_the_web(push["id"].as_i64().unwrap(), true).await.unwrap_err().contains("decided in the runner's app"));

    let id = thread.id;
    let commit = bench.desktop.run(move |core| core.push_preview(id)).await.unwrap().commit;

    assert_eq!(push["payload"]["commit"], commit.as_str());
    bench.desktop.run(move |core| core.push(id, &commit, false)).await.unwrap();
    assert!(hub.written().iter().any(|line| line.ends_with("/settle")));
    assert!(!hub.written().iter().any(|line| line.ends_with("/result") || line.contains("/changes")));
    drop(TempDir::new("unused"));
}

// ---------- a shared runner: the team's, working with each person's own keys ----------

mod shared {
    use std::path::PathBuf;

    use hub_desktop_core::model::{EventKind, ThreadState};
    use serde_json::json;

    use super::support::cloud::{github, holds, open_headless, open_headless_in, PERSON_GITHUB_TOKEN, PERSON_MODEL_KEY};
    use super::support::{Bench, FakeHub, TempDir};
    use super::{carried_out, said, thread_of, until, BRANCH};

    /// No key of its own in the environment: that is what makes a runner shared.
    const NO_KEYS: [(&str, &str); 2] = [("GITHUB_TOKEN", ""), ("ANTHROPIC_API_KEY", "")];

    fn person_keys() -> serde_json::Value {
        json!({ "anthropicApiKey": PERSON_MODEL_KEY, "githubToken": PERSON_GITHUB_TOKEN })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn it_announces_itself_shared_and_works_with_the_keys_that_come_with_each_run() {
        let hub = FakeHub::start().await;
        let host = github("shared").await;
        let bench = open_headless("shared", &hub, &host, &NO_KEYS, |_| {}).await;

        hub.state().person_keys = person_keys();

        let (run_id, thread) = carried_out(&bench, &hub, "[fake:environment]").await;
        let runner = hub.state().runners[0].clone();

        assert_eq!(
            (&runner["shared"], &runner["interactive"], &runner["kinds"]),
            (&json!(true), &json!(false), &json!(["implement", "review", "custom"]))
        );

        // The helper of that run got that person's model key; the clone was made with their token.
        let id = thread.id;
        let events = bench.desktop.run(move |core| core.thread(id)).await.unwrap().events;

        assert_eq!(
            said(&events, "Model key: "),
            format!("Model key: {} characters, ending in {}", PERSON_MODEL_KEY.len(), &PERSON_MODEL_KEY[PERSON_MODEL_KEY.len() - 8..])
        );
        assert!(!said(&events, "Variables: ").contains("GITHUB_TOKEN"));
        assert!(host.authorised_requests() >= 2);
        assert!(bench.data.path().join("repos/acme/web/.git").exists());

        // Neither key is in any file under the data folder: not the state, not a git config.
        for secret in [PERSON_MODEL_KEY, PERSON_GITHUB_TOKEN] {
            assert_eq!(holds(bench.data.path(), secret), Vec::<PathBuf>::new(), "{secret}");
        }

        assert_eq!(hub.run(run_id)["run"]["state"], "succeeded");

        // A push approved on the web goes out with the same person's token.
        let (run_id, _) = carried_out(&bench, &hub, "").await;
        let push = hub.proposal_on_run(run_id, "push");

        hub.decide_like_the_web(push["id"].as_i64().unwrap(), true).await.unwrap();
        until(&bench, &hub, "the push reported", |hub| !hub.proposal_on_run(run_id, "push")["result"].is_null()).await;
        assert_eq!(host.tip(BRANCH), push["payload"]["commit"].as_str().unwrap());
        assert_eq!(holds(bench.data.path(), PERSON_GITHUB_TOKEN), Vec::<PathBuf>::new());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_token_that_cannot_read_the_repository_fails_the_run_even_when_the_clone_is_already_there() {
        let hub = FakeHub::start().await;
        let host = github("shared-denied").await;
        let bench = open_headless("shared-denied", &hub, &host, &NO_KEYS, |_| {}).await;

        // One person's run clones the repository…
        hub.state().person_keys = person_keys();
        carried_out(&bench, &hub, "").await;
        assert!(bench.data.path().join("repos/acme/web/.git").exists());

        // …and another person's, whose token GitHub does not take, gets nothing from that clone.
        hub.state().person_keys = json!({ "anthropicApiKey": PERSON_MODEL_KEY, "githubToken": "ghp_cannot_read_this_repository_0000000" });

        let denied = hub.delegate_in("WEB-2", "implement", "", Some("acme/web"));

        until(&bench, &hub, "the run failed", |hub| hub.run(denied)["run"]["state"] == "failed").await;

        let summary = hub.run(denied)["run"]["summary"].as_str().unwrap().to_string();

        assert!(summary.contains("Your GitHub token cannot read acme/web") && summary.contains("Settings → My keys"), "{summary}");
        assert!(!summary.contains("ghp_cannot_read"));
        assert_eq!(bench.desktop.run(|core| core.threads()).await.unwrap().len(), 1, "no thread was made for it");

        // Without a model key stored the run does not start either, and says where to set one.
        hub.state().person_keys = json!({ "githubToken": PERSON_GITHUB_TOKEN });

        let keyless = hub.delegate_in("WEB-2", "implement", "", Some("acme/web"));
        let thread = thread_of(&bench, keyless).await;
        let thread = bench.until_state(thread.id, ThreadState::Failed).await;
        let events = bench.desktop.run(move |core| core.thread(thread.id)).await.unwrap().events;

        assert!(events.iter().any(|event| event.kind == EventKind::Error && event.text.contains("came without a model key")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_token_that_is_not_an_administrators_is_refused_and_the_runner_waits_instead_of_ending() {
        let hub = FakeHub::start().await;
        let host = github("shared-refused").await;

        hub.state().refuses_shared = true;
        hub.state().person_keys = person_keys();

        let bench = open_headless("shared-refused", &hub, &host, &NO_KEYS, |_| {}).await;
        let run_id = hub.delegate_in("WEB-1", "implement", "", Some("acme/web"));

        for _ in 0..4 {
            bench.desktop.tick(true).await.unwrap();
        }

        let error = bench.desktop.core().status().runner.error.unwrap();

        assert!(error.contains("administrator's API token") && error.contains("RUNNER_SHARED=0"), "{error}");
        assert!(hub.state().runners.is_empty());
        assert_eq!(hub.run(run_id)["run"]["state"], "queued", "nothing is claimed by a runner the server did not take");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn after_a_restart_the_keys_are_gone_and_the_run_fails_saying_so() {
        let hub = FakeHub::start().await;
        let host = github("shared-restart").await;
        let bench = open_headless("shared-restart", &hub, &host, &NO_KEYS, |_| {}).await;

        hub.state().person_keys = person_keys();

        let run_id = hub.delegate_in("WEB-1", "implement", "[fake:slow]", Some("acme/web"));
        let thread = thread_of(&bench, run_id).await;

        bench.until_state(thread.id, ThreadState::Running).await;

        // The container is replaced: helpers end, the server is told nothing, the keys are gone.
        bench.desktop.suspend().await;
        assert_eq!(hub.run(run_id)["run"]["state"], "running");

        let Bench { desktop, data, .. } = bench;

        drop(desktop);

        let back = open_headless_in(data, &hub, &host, &NO_KEYS, |_| {}).await;

        until(&back, &hub, "the run failed", |hub| hub.run(run_id)["run"]["state"] == "failed").await;
        assert_eq!(hub.run(run_id)["run"]["summary"], "The runner restarted and no longer holds your keys; start the run again.");
        assert_eq!(back.thread(thread.id).await.state, ThreadState::Failed);
        drop(TempDir::new("unused"));
    }
}
