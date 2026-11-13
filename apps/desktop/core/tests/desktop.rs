//! The whole core against a fake team-hub, real git repositories and the fake agent:
//! connection and offline copy, the life of a thread, the runner's side of the runs contract,
//! and the rule that nothing leaves without a click.

mod support;

use std::time::{Duration, Instant};

use hub_desktop_core::commands;
use hub_desktop_core::model::{AppEvent, EventKind, NewThread, ProposalKind, ProposalState, ThreadKind, ThreadOrigin, ThreadState};
use hub_desktop_core::{CoreError, Desktop};
use serde_json::{json, Value};
use support::{git, open, open_in, repo, write, Bench, FakeHub, TOKEN};

fn new_thread(key: &str, kind: ThreadKind, instructions: &str) -> NewThread {
    NewThread { issue_key: key.to_string(), kind, instructions: instructions.to_string(), model: None, repo_id: None, branch: None }
}

async fn start(bench: &Bench, key: &str, kind: ThreadKind, instructions: &str) -> i64 {
    let new = new_thread(key, kind, instructions);

    bench.desktop.run(move |core| core.start_thread(&new)).await.unwrap().id
}

async fn ready(label: &str) -> (Bench, FakeHub, support::Repo) {
    let hub = FakeHub::start().await;
    let bench = open(label);
    let repo = repo(label);

    bench.connect(&hub).await;
    bench.map(&repo).await;

    (bench, hub, repo)
}

fn texts(events: &[Value], kind: &str) -> Vec<String> {
    events.iter().filter(|event| event["kind"] == kind).map(|event| event["text"].as_str().unwrap_or_default().to_string()).collect()
}

fn files_under(folder: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();

    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            found.extend(files_under(&entry.path()));
        } else {
            found.push(entry.path());
        }
    }

    found
}

// ---------- connection ----------

#[tokio::test(flavor = "multi_thread")]
async fn the_token_lives_in_the_keychain_and_in_no_file() {
    let hub = FakeHub::start().await;
    let bench = open("token");
    let url = hub.url.clone();
    let refused = bench.desktop.run(move |core| core.connect(&url, "th_wrong")).await.unwrap_err();

    assert!(refused.to_string().contains("did not accept"), "{refused}");

    bench.connect(&hub).await;

    let status = bench.desktop.run(|core| Ok(core.status())).await.unwrap();

    assert!(status.online && status.has_token);
    assert_eq!(status.user.unwrap().name, "Ada Nowak");
    assert_eq!(status.server_url.as_deref(), Some(hub.url.as_str()));

    use hub_desktop_core::secrets::Keychain;

    assert_eq!(bench.keychain.get("api-token").unwrap().unwrap().expose(), TOKEN);

    bench.desktop.run(|core| core.dashboard()).await.unwrap();

    for file in files_under(bench.data.path()) {
        let bytes = std::fs::read(&file).unwrap();

        assert!(!bytes.windows(TOKEN.len()).any(|window| window == TOKEN.as_bytes()), "the token is in {}", file.display());
    }

    let json = serde_json::to_string(&bench.desktop.run(|core| Ok((core.status(), core.settings()))).await.unwrap()).unwrap();

    assert!(!json.contains(TOKEN));
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_in_with_a_password_makes_the_app_a_token_and_keeps_the_password_nowhere() {
    use hub_desktop_core::secrets::Keychain;

    let hub = FakeHub::start().await;
    let bench = open("password");

    // A wrong password is the server's refusal, and connects nothing.
    let url = hub.url.clone();
    let refused = bench.desktop.run(move |core| core.sign_in(&url, support::EMAIL, "not it")).await.unwrap_err();
    assert!(refused.to_string().contains("Wrong e-mail or password"), "{refused}");
    assert!(bench.desktop.run(|core| core.sign_in("http://127.0.0.1:1", " ", "")).await.is_err());
    assert_eq!((bench.keychain.get("api-token").unwrap(), hub.state().made_tokens.len()), (None, 0));

    // The right one: a token named after this app, connected like a pasted one.
    let url = hub.url.clone();
    let (status, session) = bench.desktop.run(move |core| core.sign_in(&url, &format!(" {} ", support::EMAIL), support::PASSWORD)).await.unwrap();

    assert!(status.online && status.has_token);
    assert_eq!(status.user.unwrap().name, "Ada Nowak");
    assert!(hub.state().made_tokens[0].starts_with("Desktop"));
    assert_eq!(bench.keychain.get("api-token").unwrap().unwrap().expose(), TOKEN);
    assert_eq!(bench.keychain.get("api-token-id").unwrap().unwrap().expose(), support::PAIRED_TOKEN_ID.to_string(), "the token made now, not the older one");

    // The session is the caller's: the window gives it to the tracker, anyone else ends it.
    assert_eq!((session.cookie.expose(), hub.state().sessions), ("session-of-ada", 1));
    assert!(!format!("{session:?}").contains("session-of-ada"));
    session.end();
    assert_eq!(hub.state().sessions, 0);

    // Neither the password nor the session is in a file.
    for file in files_under(bench.data.path()) {
        let bytes = std::fs::read(&file).unwrap();

        for secret in [support::PASSWORD, "session-of-ada", TOKEN] {
            assert!(!bytes.windows(secret.len()).any(|window| window == secret.as_bytes()), "{secret} is in {}", file.display());
        }
    }

    // Disconnecting takes the token back on the server.
    bench.desktop.run(|core| core.disconnect()).await.unwrap();
    assert_eq!(hub.state().revoked_tokens, [support::PAIRED_TOKEN_ID]);

    // Through the commands (a browser, the command line) nobody shows the tracker: the
    // session is ended there, and the answer is the status alone.
    let args = json!({ "url": hub.url, "email": support::EMAIL, "password": support::PASSWORD });
    let answer = commands::invoke(&bench.desktop, "sign_in", args).await.unwrap();
    assert_eq!((&answer["hasToken"], hub.state().sessions), (&json!(true), 0));
    assert!(!answer.to_string().contains("session-of-ada") && !answer.to_string().contains(support::PASSWORD));
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_in_from_the_window_gets_the_token_once_a_person_has_said_yes() {
    use hub_desktop_core::secrets::Keychain;

    let hub = FakeHub::start().await;
    let bench = open("pairing");
    let url = format!("{}/", hub.url);

    // Nothing to wait for before anything was asked.
    assert!(bench.desktop.run(|core| core.pair_wait()).await.unwrap_err().to_string().contains("Nothing is waiting"));

    let pairing = bench.desktop.run(move |core| core.pair_start(&url)).await.unwrap();
    let asked = hub.state().pairings.values().next().cloned().expect("the server was asked");

    // The server heard which app this is; the window heard the code and where to say yes.
    assert!(asked.name.starts_with("Desktop") && asked.runner_id.starts_with("desktop-"), "{asked:?}");
    assert_eq!((pairing.code.as_str(), pairing.path.as_str()), (asked.code.as_str(), format!("/desktop/pair?code={}", asked.code).as_str()));
    assert_eq!(pairing.server_url, hub.url);
    assert!(!serde_json::to_string(&pairing).unwrap().contains(&asked.secret), "the secret stays in the core");

    // The address is kept, so the window can show the server's sign-in; connected it is not.
    let status = bench.desktop.run(|core| Ok(core.status())).await.unwrap();
    assert_eq!((status.server_url.as_deref(), status.has_token, status.user.is_none()), (Some(hub.url.as_str()), false, true));
    assert_eq!(bench.keychain.get("api-token").unwrap(), None);

    // The app asks, and asks again, until the person has said yes.
    let waiting = bench.desktop.clone();
    let waiter = tokio::spawn(async move { waiting.run(|core| core.pair_wait()).await });

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!waiter.is_finished());
    hub.answer_pairings(true);

    let status = waiter.await.unwrap().unwrap();

    assert!(status.online && status.has_token);
    assert_eq!(status.user.unwrap().name, "Ada Nowak");
    assert_eq!(bench.keychain.get("api-token").unwrap().unwrap().expose(), TOKEN);
    assert_eq!(bench.keychain.get("api-token-id").unwrap().unwrap().expose(), support::PAIRED_TOKEN_ID.to_string());
    bench.desktop.run(|core| core.dashboard()).await.unwrap();

    for file in files_under(bench.data.path()) {
        let bytes = std::fs::read(&file).unwrap();

        for secret in [TOKEN, asked.secret.as_str()] {
            assert!(!bytes.windows(secret.len()).any(|window| window == secret.as_bytes()), "{secret} is in {}", file.display());
        }
    }

    // Asking again while connected is refused: one server, one token.
    let again = hub.url.clone();
    assert!(bench.desktop.run(move |core| core.pair_start(&again)).await.unwrap_err().to_string().contains("connected already"));

    // Disconnecting takes the token back on the server, since the app was given it there.
    let status = bench.desktop.run(|core| core.disconnect()).await.unwrap();

    assert!(!status.has_token);
    assert_eq!(hub.state().revoked_tokens, [support::PAIRED_TOKEN_ID]);
    assert_eq!((bench.keychain.get("api-token").unwrap(), bench.keychain.get("api-token-id").unwrap()), (None, None));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_to_connect_ends_when_declined_or_given_up_and_a_pasted_token_stays_on_the_server() {
    let hub = FakeHub::start().await;
    let bench = open("pairing-no");

    // Declined in the tracker.
    let url = hub.url.clone();
    bench.desktop.run(move |core| core.pair_start(&url)).await.unwrap();
    hub.answer_pairings(false);
    let declined = bench.desktop.run(|core| core.pair_wait()).await.unwrap_err();
    assert!(declined.to_string().contains("declined or ran out of time"), "{declined}");
    assert!(!bench.desktop.run(|core| Ok(core.status())).await.unwrap().has_token);

    // Given up in the app: whoever waits hears it at once, and the server is left alone.
    let url = hub.url.clone();
    bench.desktop.run(move |core| core.pair_start(&url)).await.unwrap();
    let waiting = bench.desktop.clone();
    let waiter = tokio::spawn(async move { waiting.run(|core| core.pair_wait()).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = Instant::now();
    bench.desktop.run(|core| {
        core.pair_cancel();
        Ok(())
    })
    .await
    .unwrap();
    assert!(waiter.await.unwrap().unwrap_err().to_string().contains("cancelled"));
    assert!(before.elapsed() < Duration::from_secs(1), "noticed without waiting for the next question");
    hub.answer_pairings(true);
    assert!(bench.desktop.run(|core| core.pair_wait()).await.is_err(), "a yes that comes after giving up connects nothing");
    assert!(!bench.desktop.run(|core| Ok(core.status())).await.unwrap().has_token);

    // A server from before there was such a thing says what to do instead.
    hub.state().no_pairing = true;
    let url = hub.url.clone();
    let old = bench.desktop.run(move |core| core.pair_start(&url)).await.unwrap_err();
    assert!(old.to_string().contains("API token"), "{old}");
    assert!(bench.desktop.run(|core| core.pair_start("ftp://hub.example.com")).await.is_err());

    // A token somebody pasted is theirs: disconnecting removes nothing on the server.
    bench.connect(&hub).await;
    bench.desktop.run(|core| core.disconnect()).await.unwrap();
    assert!(hub.state().revoked_tokens.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_server_the_stored_copy_is_shown_and_it_is_sealed_on_disk() {
    let hub = FakeHub::start().await;
    let data = support::TempDir::new("offline");
    let bench = open_in(data);

    bench.connect(&hub).await;

    let fresh = bench.desktop.run(|core| core.dashboard()).await.unwrap();
    let issue = bench.desktop.run(|core| core.issue("web-2")).await.unwrap();

    assert!(!fresh.offline);
    assert_eq!(fresh.dashboard.assigned.len(), 2);
    assert_eq!(fresh.projects[0].summary.key, "WEB");
    assert_eq!(issue.issue.row.summary, "Błąd logowania: 500");

    // The server goes away: same address, nobody listening.
    hub.state().token = "th_rotated".to_string();

    let unauthorised = bench.desktop.run(|core| core.dashboard()).await.unwrap_err();

    assert!(unauthorised.is_unauthorized(), "a refused token is an error, not a reason to show old data: {unauthorised}");

    let dead = "http://127.0.0.1:9";
    let keychain = bench.keychain.clone();
    let mut settings: Value = serde_json::from_slice(&std::fs::read(bench.data.path().join("state/settings.json")).unwrap()).unwrap();

    settings["serverUrl"] = json!(dead);
    std::fs::write(bench.data.path().join("state/settings.json"), settings.to_string()).unwrap();

    let mut options = hub_desktop_core::OpenOptions::new(hub_desktop_core::config::Paths::under(bench.data.path()), keychain);

    options.helper = Some(support::fake_agent());

    let offline = Desktop::open(options).unwrap();
    let status = offline.run(|core| Ok(core.reconnect())).await.unwrap();

    assert!(!status.online);
    assert_eq!(status.user.unwrap().name, "Ada Nowak");

    let stored = offline.run(|core| core.dashboard()).await.unwrap();
    let stored_issue = offline.run(|core| core.issue("WEB-2")).await.unwrap();

    assert!(stored.offline && stored_issue.offline);
    assert_eq!(stored.dashboard.assigned.len(), 2);
    assert_eq!(stored_issue.issue.description, "Fails for long passwords.");
    assert!(offline.run(|core| core.issue("WEB-1")).await.unwrap_err().is_offline());
    assert!(offline.run(|core| core.comment("WEB-2", "hello")).await.unwrap_err().is_offline());

    for file in files_under(&bench.data.path().join("state")) {
        let bytes = std::fs::read(&file).unwrap();

        assert!(!bytes.windows(12).any(|window| window == b"long passwor"), "the copy is readable in {}", file.display());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_basics_on_an_issue_go_through_the_api_as_the_person() {
    let (bench, hub, _repo) = ready("basics").await;
    let commented = bench.desktop.run(|core| core.comment("WEB-1", "Looking at it.")).await.unwrap();
    let moved = bench.desktop.run(|core| core.transition("WEB-1", 2)).await.unwrap();
    let assigned = bench.desktop.run(|core| core.assign_to_me("WEB-1")).await.unwrap();
    let logged = bench.desktop.run(|core| core.log_work("WEB-1", "1h 30m", "reading")).await.unwrap();
    let nonsense = bench.desktop.run(|core| core.log_work("WEB-1", "soon", "")).await.unwrap_err();
    let found = bench.desktop.run(|core| core.search("login")).await.unwrap();

    assert_eq!(commented.issue.comments[0].body, "Looking at it.");
    assert_eq!(moved.issue.row.status_id, 2);
    assert_eq!(assigned.issue.row.assignee_id, Some(1));
    assert_eq!(logged.issue.spent_minutes, 90);
    assert!(nonsense.to_string().contains("1h 30m"));
    assert_eq!(found.result.total, 1);

    let writes: Vec<(String, String)> = hub.state().writes.iter().map(|(method, path, _)| (method.clone(), path.clone())).collect();

    assert_eq!(
        writes,
        vec![
            ("POST".to_string(), "/api/issues/WEB-1/comments".to_string()),
            ("PATCH".to_string(), "/api/issues/WEB-1".to_string()),
            ("PATCH".to_string(), "/api/issues/WEB-1".to_string()),
            ("POST".to_string(), "/api/issues/WEB-1/worklogs".to_string()),
        ]
    );
    assert_eq!(hub.state().writes[1].2, json!({ "statusId": 2 }));
}

// ---------- repositories ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_repository_is_mapped_only_when_its_origin_is_the_named_one() {
    let hub = FakeHub::start().await;
    let bench = open("repos");
    let repo = repo("repos");
    let path = repo.clone.to_string_lossy().into_owned();

    bench.connect(&hub).await;
    git(&repo.clone, &["remote", "set-url", "origin", "git@github.com:acme/web.git"]);

    let (wrong, not_a_repo, right) = {
        let path = path.clone();
        let outside = repo.root.path().to_string_lossy().into_owned();

        bench
            .desktop
            .run(move |core| {
                Ok((
                    core.map_repo("WEB", Some("acme/api"), &path).unwrap_err(),
                    core.map_repo("WEB", None, &outside).unwrap_err(),
                    core.map_repo("web", Some("Acme/Web"), &format!("{path}/src"))?,
                ))
            })
            .await
            .unwrap()
    };

    assert!(wrong.to_string().contains("not acme/api"), "{wrong}");
    assert!(not_a_repo.to_string().contains("Not a git repository"));
    assert_eq!((right.project_key.as_str(), right.path.as_str()), ("WEB", path.as_str()));

    write(&repo.clone, "src/login.ts", "dirty\n");

    let listed = bench.desktop.run(|core| core.repos()).await.unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].default_branch.as_deref(), Some("origin/main"));
    assert_eq!(listed[0].changed_files, 1);
    assert_eq!(listed[0].remote_matches, Some(true));
    assert!(listed[0].worktrees.is_empty());
}

// ---------- the life of a thread ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_works_in_its_own_tree_and_shows_up_on_the_server_as_a_run() {
    let (bench, hub, repo) = ready("lifecycle").await;

    write(&repo.clone, "src/login.ts", "my own unfinished work\n");

    let main_before = (git(&repo.clone, &["rev-parse", "HEAD"]), git(&repo.clone, &["status", "--porcelain"]));
    let id = start(&bench, "WEB-1", ThreadKind::Implement, "Keep it small.").await;
    let thread = bench.until_state(id, ThreadState::Done).await;
    let branch = "feature/WEB-1-login-form-validation";
    let tree = std::path::PathBuf::from(thread.worktree.clone().unwrap());

    assert_eq!(thread.branch.as_deref(), Some(branch));
    assert_eq!(thread.origin, ThreadOrigin::App);
    assert_eq!(thread.session_id.as_deref(), Some(format!("fake-session-{id}").as_str()));
    assert!(tree.starts_with(bench.data.path().join("worktrees")), "{}", tree.display());
    assert!(!tree.starts_with(&repo.clone));
    assert_eq!((thread.input_tokens, thread.output_tokens, thread.turns), (1200, 340, 3));
    assert_eq!(thread.pending_proposals, 3);

    // The agent's commit is on the thread's branch; the main clone is exactly as it was.
    assert_eq!(git(&repo.clone, &["log", "-1", "--format=%s", branch]), "WEB-1: implement Login form validation");
    assert_eq!((git(&repo.clone, &["rev-parse", "HEAD"]), git(&repo.clone, &["status", "--porcelain"])), main_before);
    assert_eq!(git(&repo.clone, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(git(&repo.origin, &["for-each-ref", "--format=%(refname)"]), "refs/heads/main");

    let detail = bench.desktop.run(move |core| core.thread(id)).await.unwrap();
    let said: Vec<&str> = detail.events.iter().filter(|event| event.kind == EventKind::Message).map(|event| event.text.as_str()).collect();
    let tools: Vec<&str> =
        detail.events.iter().filter(|event| event.kind == EventKind::Tool).map(|event| event.data["name"].as_str().unwrap()).collect();
    let kinds: Vec<ProposalKind> = detail.proposals.iter().map(|proposal| proposal.kind).collect();

    assert_eq!(said, vec!["Reading WEB-1 before touching the code.", "The change is committed locally."]);
    assert_eq!(tools[..3], ["mcp__team-hub__get_issue", "Write", "Bash"]);
    assert_eq!(kinds, vec![ProposalKind::Comment, ProposalKind::Push, ProposalKind::PullRequest]);
    assert!(detail.proposals.iter().all(|proposal| proposal.remote_id.is_some()), "the run on the server shows all three");

    let diff = bench.desktop.run(move |core| core.thread_diff(id)).await.unwrap();

    assert_eq!(diff.base, "origin/main");
    assert_eq!(diff.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), vec!["agent-notes/WEB-1.md"]);
    assert_eq!((diff.commits.len(), diff.uncommitted), (1, 0));

    // The same thread, as the web interface sees it.
    let run_id = thread.run_id.unwrap();
    let run = hub.run(run_id);
    let events = run["events"].as_array().unwrap();

    // Done here, but the push and the pull request still wait for the person: so does the run.
    assert_eq!(run["run"]["state"], "waiting");
    assert_eq!(run["run"]["kind"], "implement");
    assert_eq!(run["run"]["instructions"], "Keep it small.");
    assert_eq!(run["run"]["branch"], branch);
    assert_eq!(run["run"]["inputTokens"], 1200);
    assert_eq!(run["run"]["outputTokens"], 340);
    assert_eq!(run["run"]["costUsd"], 0.0123);
    assert_eq!(run["run"]["runnerName"], bench.desktop.core().settings().runner_name);
    assert!(run["run"]["summary"].as_str().unwrap().starts_with("Implemented WEB-1"));
    assert_eq!(texts(events, "message"), said);
    // The web can decide the comment; the push and the pull request it only shows.
    let shown: Vec<(&str, &str)> =
        run["proposals"].as_array().unwrap().iter().map(|proposal| (proposal["kind"].as_str().unwrap(), proposal["state"].as_str().unwrap())).collect();

    assert_eq!(shown, vec![("comment", "pending"), ("push", "pending"), ("pull_request", "pending")]);
    assert_eq!(run["proposals"][1]["payload"]["branch"], branch);
    assert_eq!(run["proposals"][2]["payload"]["head"], branch);
    assert!(texts(events, "status").iter().any(|text| text.contains("pushing the branch")));
    // Mirrored once: another round sends nothing again.
    let sent = hub.state().events.len();

    bench.desktop.tick(true).await.unwrap();

    assert_eq!(hub.state().events.len(), sent);
    // The description's injected "instruction" reached the agent only inside a data fence.
    assert_eq!(hub.state().tool_calls, vec!["get_issue"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_delegated_on_the_web_is_claimed_and_carried_out() {
    let (bench, hub, _repo) = ready("delegated").await;
    let run_id = hub.delegate("WEB-2", "implement", "From the web. [fake:analyse]");

    bench.desktop.tick(true).await.unwrap();

    let claimed = hub.run(run_id);

    assert_eq!(claimed["run"]["runnerId"], bench.desktop.core().settings().runner_id);
    assert_eq!(hub.state().runners[0]["kinds"], json!(["implement", "review", "custom"]));
    assert_eq!(hub.state().runners[0]["projects"], json!(["WEB"]));

    let thread = bench.desktop.run(|core| core.threads()).await.unwrap().remove(0);
    let done = bench.until_state(thread.id, ThreadState::Done).await;

    assert_eq!(done.origin, ThreadOrigin::Delegated);
    assert_eq!(done.run_id, Some(run_id));
    assert_eq!(done.branch.as_deref(), Some("fix/WEB-2-blad-logowania-500"));
    assert_eq!(done.instructions, "From the web. [fake:analyse]");
    assert_eq!(hub.run(run_id)["run"]["state"], "succeeded");
    // A run of a kind this runner does not carry out stays queued.
    let refine = hub.delegate("WEB-2", "refine", "");
    // A run for a repository this machine has no clone of is given back as failed, with the reason.
    let elsewhere = hub.delegate_in("WEB-2", "review", "", Some("acme/other"));

    bench.desktop.tick(true).await.unwrap();

    assert_eq!(hub.run(refine)["run"]["state"], "queued");
    assert_eq!(hub.run(elsewhere)["run"]["state"], "failed");
    assert!(hub.run(elsewhere)["run"]["summary"].as_str().unwrap().contains("acme/other has no local clone mapped"));
    assert_eq!(bench.desktop.run(|core| core.threads()).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_waits_for_a_person_and_the_answer_from_the_web_resumes_the_same_session() {
    let (bench, hub, _repo) = ready("question-web").await;
    let mut events = bench.desktop.subscribe();
    let id = start(&bench, "WEB-1", ThreadKind::Implement, "[fake:question]").await;
    let waiting = bench.until_state(id, ThreadState::Waiting).await;
    let run_id = waiting.run_id.unwrap();

    assert!(waiting.pending_question);
    assert_eq!(hub.run(run_id)["run"]["state"], "waiting");
    assert_eq!(texts(hub.run(run_id)["events"].as_array().unwrap(), "question"), vec!["Should the limit be 10 or 20 attempts?"]);

    let mut notified = false;

    while let Ok(event) = events.try_recv() {
        notified |= matches!(event, AppEvent::Notify { title, .. } if title == "WEB-1 is waiting for your answer");
    }

    assert!(notified, "a waiting thread raises a notification");

    hub.answer_on_web(run_id, "Twenty.");

    let done = bench.until_state(id, ThreadState::Done).await;
    let detail = bench.desktop.run(move |core| core.thread(id)).await.unwrap();

    assert!(!done.pending_question);
    assert_eq!(done.session_id, waiting.session_id);
    assert_eq!(done.worktree, waiting.worktree);
    assert!(detail.events.iter().any(|event| event.kind == EventKind::Answer && event.text == "Twenty."));
    assert!(detail.events.iter().any(|event| event.kind == EventKind::Message && event.text == "Continuing. You said: Twenty."));
    assert_eq!(hub.run(run_id)["run"]["state"], "succeeded");
    // The answer came from the server, so it is not sent back to it.
    assert_eq!(texts(hub.run(run_id)["events"].as_array().unwrap(), "answer"), vec!["Twenty."]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_given_in_the_app_goes_to_the_run_first() {
    let (bench, hub, _repo) = ready("question-app").await;
    let id = start(&bench, "WEB-1", ThreadKind::Implement, "[fake:question]").await;
    let run_id = bench.until_state(id, ThreadState::Waiting).await.run_id.unwrap();

    bench.desktop.run(move |core| core.say_to_thread(id, Some("Ten is enough."))).await.unwrap();
    bench.until_state(id, ThreadState::Done).await;

    assert_eq!(texts(hub.run(run_id)["events"].as_array().unwrap(), "answer"), vec!["Ten is enough."]);
    assert_eq!(hub.run(run_id)["run"]["state"], "succeeded");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_request_from_the_web_stops_the_thread() {
    let (bench, hub, _repo) = ready("cancel").await;
    let id = start(&bench, "WEB-1", ThreadKind::Custom, "[fake:slow]").await;
    let running = bench.until(id, "saying it will take a while", |thread| thread.last_text.is_some()).await;

    hub.cancel_on_web(running.run_id.unwrap());

    let stopped = bench.until_state(id, ThreadState::Stopped).await;

    bench.desktop.tick(true).await.unwrap();

    assert_eq!(hub.run(stopped.run_id.unwrap())["run"]["state"], "cancelled");
    assert!(bench.desktop.helper_pid(id).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_two_threads_work_at_once_and_the_third_takes_the_freed_slot() {
    let (bench, _hub, _repo) = ready("limit").await;
    let first = start(&bench, "WEB-1", ThreadKind::Custom, "[fake:slow]").await;
    let second = start(&bench, "WEB-2", ThreadKind::Custom, "[fake:slow]").await;

    bench.until(first, "working", |thread| thread.last_text.is_some()).await;
    bench.until(second, "working", |thread| thread.last_text.is_some()).await;

    let third = {
        let mut new = new_thread("WEB-1", ThreadKind::Analyse, "[fake:analyse]");

        new.branch = Some("feature/WEB-1-analysis".to_string());
        bench.desktop.run(move |core| core.start_thread(&new)).await.unwrap().id
    };

    tokio::time::sleep(Duration::from_millis(300)).await;

    let queued = bench.thread(third).await;

    assert_eq!(queued.state, ThreadState::Queued);
    assert_eq!(queued.worktree, None);

    bench.desktop.stop_thread(first).await.unwrap();

    let done = bench.until_state(third, ThreadState::Done).await;

    assert_eq!(done.branch.as_deref(), Some("feature/WEB-1-analysis"));
    assert_eq!(bench.thread(second).await.state, ThreadState::Running);

    bench.desktop.stop_thread(second).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_thread_on_the_same_branch_takes_over_the_tree_of_one_at_rest_and_waits_for_one_at_work() {
    let (bench, _hub, repo) = ready("same-branch").await;
    let first = start(&bench, "WEB-1", ThreadKind::Implement, "").await;
    let done = bench.until_state(first, ThreadState::Done).await;
    let second = start(&bench, "WEB-1", ThreadKind::Custom, "[fake:slow]").await;
    let working = bench.until(second, "working", |thread| thread.last_text.is_some()).await;

    // The earlier thread was at rest: the new one continues in its tree, on its branch.
    assert_eq!(working.worktree, done.worktree);
    assert_eq!(working.branch, done.branch);
    assert_eq!(working.base_commit, done.base_commit);
    assert_eq!(bench.thread(first).await.worktree, None);
    assert_eq!(git(&repo.clone, &["worktree", "list", "--porcelain"]).matches("worktree ").count(), 2);

    // A third one while the second is working cannot have the tree.
    let third = start(&bench, "WEB-1", ThreadKind::Analyse, "[fake:analyse]").await;
    let refused = bench.until_state(third, ThreadState::Failed).await;
    let reason = bench.desktop.run(move |core| core.thread(third)).await.unwrap().events.pop().unwrap().text;

    assert_eq!(reason, format!("Thread #{second} is already working on branch feature/WEB-1-login-form-validation; stop it or wait for it"));
    assert_eq!(refused.worktree, None);

    bench.desktop.stop_thread(second).await.unwrap();

    let diff = bench.desktop.run(move |core| core.thread_diff(second)).await.unwrap();

    assert_eq!(diff.commits.len(), 1, "the second thread sees the first one's commit on the branch");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_thread_leaves_no_process_behind_even_when_the_agent_hangs() {
    let (bench, _hub, _repo) = ready("stop").await;

    for marker in ["[fake:slow]", "[fake:hang]"] {
        let id = start(&bench, "WEB-1", ThreadKind::Custom, marker).await;

        bench.until(id, "working", |thread| thread.last_text.is_some()).await;

        let pid = bench.desktop.helper_pid(id).expect("a working thread has a process");

        assert!(support::group_alive(pid));

        let stopped = bench.desktop.stop_thread(id).await.unwrap();

        assert_eq!(stopped.state, ThreadState::Stopped, "{marker}");
        assert!(!support::group_alive(pid), "{marker}: the helper or something it started is still alive");
        assert!(bench.desktop.helper_pid(id).is_none());

        let last = bench.desktop.run(move |core| core.thread(id)).await.unwrap().events.pop().unwrap();

        assert_eq!((last.kind, last.text.as_str()), (EventKind::Status, "Stopped."));

        // The tree is clean, so it can go; the thread on the same issue then starts afresh.
        bench.desktop.remove_worktree(id).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_a_failure_and_an_error_each_end_the_thread_as_failed_with_the_reason() {
    let (bench, hub, _repo) = ready("failures").await;
    let expectations = [
        ("[fake:crash]", "The agent process ended without a result (exit code 3).\nfake agent: simulated crash"),
        ("[fake:fail]", "The tests of the project do not pass and I could not find out why."),
        ("[fake:error]", "The model could not be reached."),
    ];

    for (marker, reason) in expectations {
        let id = start(&bench, "WEB-1", ThreadKind::Custom, marker).await;
        let failed = bench.until_state(id, ThreadState::Failed).await;
        let detail = bench.desktop.run(move |core| core.thread(id)).await.unwrap();
        let errors: Vec<&str> = detail.events.iter().filter(|event| event.kind == EventKind::Error).map(|event| event.text.as_str()).collect();

        assert_eq!(errors, vec![reason], "{marker}");
        assert_eq!(hub.run(failed.run_id.unwrap())["run"]["state"], "failed", "{marker}");
        assert!(bench.desktop.helper_pid(id).is_none());

        bench.desktop.remove_worktree(id).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_reads_through_the_servers_read_only_tools_and_cannot_call_the_rest() {
    let (bench, hub, _repo) = ready("read-only").await;
    let id = start(&bench, "WEB-1", ThreadKind::Custom, "[fake:write-tool]").await;

    bench.until_state(id, ThreadState::Done).await;

    let detail = bench.desktop.run(move |core| core.thread(id)).await.unwrap();
    let report = detail.events.iter().rev().find(|event| event.kind == EventKind::Message).unwrap();

    assert_eq!(
        report.text,
        "add_comment: add_comment is not a read-only tool of the server. To change something, propose it.; delete_everything: There is no tool called delete_everything"
    );
    // Only the read reached the server; the comment was never written.
    assert_eq!(hub.state().tool_calls, vec!["get_issue"]);
    assert!(hub.state().issues["WEB-1"].comments.is_empty());
    assert!(!hub.state().writes.iter().any(|(_, path, _)| path.contains("comments")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_without_a_shell_commits_through_the_app_and_a_reading_thread_cannot() {
    let (bench, _hub, repo) = ready("no-shell").await;

    git(&repo.clone, &["config", "commit.gpgsign", "false"]);
    git(&repo.clone, &["config", "user.name", "Ada"]);
    git(&repo.clone, &["config", "user.email", "ada@example.invalid"]);

    let id = start(&bench, "WEB-1", ThreadKind::Implement, "[fake:no-shell]").await;
    let done = bench.until_state(id, ThreadState::Done).await;
    let detail = bench.desktop.run(move |core| core.thread(id)).await.unwrap();
    let said = detail.events.iter().rev().find(|event| event.kind == EventKind::Message).unwrap();
    let short = git(&repo.clone, &["rev-parse", "--short", done.branch.as_deref().unwrap()]);

    assert_eq!(said.text, format!("commit_changes offered: true; answer: Committed {short} WEB-1: note the change"));
    assert_eq!(git(&repo.clone, &["log", "-1", "--format=%s", done.branch.as_deref().unwrap()]), "WEB-1: note the change");
    assert!(detail.events.iter().any(|event| event.text.contains("works without a shell")));

    let reading = {
        let mut new = new_thread("WEB-2", ThreadKind::Review, "[fake:no-shell]");

        new.branch = Some("feature/WEB-2-review".to_string());
        bench.desktop.run(move |core| core.start_thread(&new)).await.unwrap().id
    };

    bench.until_state(reading, ThreadState::Done).await;

    let refused = bench.desktop.run(move |core| core.thread(reading)).await.unwrap();
    let said = refused.events.iter().rev().find(|event| event.kind == EventKind::Message).unwrap();

    assert_eq!(said.text, "commit_changes offered: false; answer: This thread only reads; it has nothing to commit");
    assert_eq!(git(&repo.clone, &["rev-list", "--count", "origin/main..feature/WEB-2-review"]), "0");
}

// ---------- nothing leaves without a click ----------

#[tokio::test(flavor = "multi_thread")]
async fn proposals_change_nothing_until_decided_in_the_app_or_on_the_web() {
    let (bench, hub, _repo) = ready("proposals").await;
    let id = start(&bench, "WEB-1", ThreadKind::Analyse, "[fake:subtasks]").await;
    let done = bench.until_state(id, ThreadState::Done).await;
    let run_id = done.run_id.unwrap();
    let proposals = bench.desktop.run(move |core| core.thread(id)).await.unwrap().proposals;

    assert_eq!(proposals.iter().map(|proposal| proposal.kind).collect::<Vec<_>>(), vec![ProposalKind::CreateSubtasks, ProposalKind::Transition]);
    assert_eq!(hub.state().issues.len(), 2);
    assert_eq!(hub.state().issues["WEB-1"].row.status_id, 1);
    assert_eq!(hub.run(run_id)["run"]["pendingProposals"], 2);

    // Subtasks: approved in the app, applied by the server.
    let subtasks = proposals[0].id;
    let decided = bench.desktop.run(move |core| core.decide_proposal(subtasks, true, None)).await.unwrap();

    assert_eq!(decided.state, ProposalState::Approved);
    assert_eq!(decided.result, json!({ "keys": ["WEB-3", "WEB-4"] }));
    assert_eq!(hub.state().issues["WEB-3"].row.parent_key.as_deref(), Some("WEB-1"));
    assert!(bench.desktop.run(move |core| core.decide_proposal(subtasks, true, None)).await.is_err());

    // The status change: rejected on the web, and the app learns of it.
    hub.decide_on_web(proposals[1].remote_id.unwrap(), false);
    bench.until(id, "no pending proposals", |thread| thread.pending_proposals == 0).await;

    let transition = proposals[1].id;
    let rejected = bench.desktop.run(move |core| core.pending_proposals().map(|pending| pending.len())).await.unwrap();

    assert_eq!(rejected, 0);
    assert_eq!(bench.desktop.run(move |core| Ok(core.thread(id)?.proposals[1].state)).await.unwrap(), ProposalState::Rejected);
    assert_eq!(hub.state().issues["WEB-1"].row.status_id, 1);
    assert!(bench.desktop.run(move |core| core.decide_proposal(transition, true, None)).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_goes_out_only_after_the_click_and_only_the_commit_that_was_shown() {
    let (bench, hub, repo) = ready("push").await;
    let id = start(&bench, "WEB-1", ThreadKind::Implement, "").await;
    let thread = bench.until_state(id, ThreadState::Done).await;
    let branch = thread.branch.clone().unwrap();

    assert_eq!(git(&repo.origin, &["for-each-ref", "--format=%(refname)"]), "refs/heads/main");

    let push_proposal = bench.desktop.run(move |core| core.thread(id)).await.unwrap().proposals[1].id;
    let approve_blindly = bench.desktop.run(move |core| core.decide_proposal(push_proposal, true, None)).await.unwrap_err();

    assert!(approve_blindly.to_string().contains("from its preview"));

    let preview = bench.desktop.run(move |core| core.push_preview(id)).await.unwrap();

    assert_eq!(preview.branch, branch);
    assert_eq!(preview.commits.len(), 1);
    assert_eq!(preview.files[0].path, "agent-notes/WEB-1.md");
    assert!(preview.findings.is_empty() && preview.blocked.is_none());

    // The thread adds a commit after the preview was shown.
    let tree = std::path::PathBuf::from(thread.worktree.unwrap());

    write(&tree, "later.txt", "added after the person looked\n");
    git(&tree, &["add", "--all"]);
    git(&tree, &["commit", "-m", "WEB-1: later"]);

    let commit = preview.commit.clone();
    let pushed = bench.desktop.run(move |core| core.push(id, &commit, false)).await.unwrap();

    assert_eq!(git(&repo.origin, &["rev-parse", &format!("refs/heads/{branch}")]), preview.commit);
    assert_eq!(pushed.commit, preview.commit);
    assert_eq!(git(&repo.origin, &["rev-parse", "refs/heads/main"]), git(&repo.clone, &["rev-parse", "origin/main"]));

    let proposals = bench.desktop.run(move |core| core.thread(id)).await.unwrap().proposals;

    assert_eq!(proposals[1].state, ProposalState::Approved);
    assert_eq!(proposals[2].state, ProposalState::Pending);

    // The pull request waits for the newer commit to be pushed too.
    let pull_request = bench.desktop.run(move |core| core.pull_request_preview(id)).await.unwrap();

    assert_eq!((pull_request.base.as_str(), pull_request.head.as_str()), ("main", branch.as_str()));
    assert_eq!(pull_request.title, "WEB-1 Login form validation");
    assert!(!pull_request.pushed);
    assert!(bench
        .desktop
        .run(move |core| core.open_pull_request(id, "WEB-1 Login", ""))
        .await
        .unwrap_err()
        .to_string()
        .contains("Push the branch first"));

    // While the pull request is undecided the run waits; once it is there, the run ends with it.
    let run_id = thread.run_id.unwrap();

    bench.desktop.tick(true).await.unwrap();

    assert_eq!(hub.run(run_id)["run"]["state"], "waiting");
    assert!(bench.desktop.run(move |core| core.record_pull_request(id, "javascript:alert(1)")).await.is_err());

    // The run was told how the push went as soon as it was done.
    let on_run = |kind: &str| hub.run(run_id)["proposals"].as_array().unwrap().iter().find(|proposal| proposal["kind"] == kind).cloned().unwrap();

    assert_eq!((&on_run("push")["state"], &on_run("push")["result"]["commit"]), (&json!("approved"), &json!(preview.commit)));
    assert_eq!(on_run("pull_request")["state"], "pending");

    bench.desktop.run(move |core| core.record_pull_request(id, "https://github.com/acme/web/pull/7")).await.unwrap();
    assert_eq!(
        (&on_run("pull_request")["state"], &on_run("pull_request")["result"]["url"]),
        (&json!("approved"), &json!("https://github.com/acme/web/pull/7"))
    );
    bench.desktop.tick(true).await.unwrap();

    let run = hub.run(run_id);

    assert_eq!(run["run"]["state"], "succeeded");
    assert_eq!(run["run"]["branch"], branch);
    assert_eq!(run["run"]["pullRequestUrl"], "https://github.com/acme/web/pull/7");
    assert!(texts(run["events"].as_array().unwrap(), "status").iter().any(|text| text.starts_with("Pushed feature/WEB-1-login-form-validation at ")));
}

#[tokio::test(flavor = "multi_thread")]
async fn without_gh_the_pull_request_is_opened_by_the_server_or_finished_in_the_browser() {
    let (bench, hub, repo) = ready("server-pr").await;
    let path = repo.clone.to_string_lossy().into_owned();

    // The clone's origin reads as a GitHub address; pushes still go to the folder.
    git(&repo.clone, &["config", &format!("url.{}.pushInsteadOf", repo.origin.to_string_lossy()), "https://github.com/acme/web.git"]);
    git(&repo.clone, &["config", "remote.origin.url", "https://github.com/acme/web.git"]);
    bench.desktop.run(move |core| core.map_repo("WEB", Some("acme/web"), &path)).await.unwrap();

    let id = start(&bench, "WEB-1", ThreadKind::Implement, "").await;
    let thread = bench.until_state(id, ThreadState::Done).await;
    let branch = thread.branch.clone().unwrap();
    let commit = bench.desktop.run(move |core| core.push_preview(id)).await.unwrap().commit;

    bench.desktop.run(move |core| core.push(id, &commit, false)).await.unwrap();

    // The server does not know the repository: only the browser is left.
    let preview = bench.desktop.run(move |core| core.pull_request_preview(id)).await.unwrap();
    assert_eq!((preview.pushed, preview.can_create, preview.server_repo), (true, false, None));

    // It knows it but has no token: the person still gets the page to finish on, and nothing is recorded.
    hub.state().issues.get_mut("WEB-1").unwrap().panels.insert("github".into(), json!({ "prs": [], "repos": [{ "id": 3, "fullName": "Acme/Web" }] }));
    hub.state().github_token_missing = true;

    let preview = bench.desktop.run(move |core| core.pull_request_preview(id)).await.unwrap();
    assert_eq!((preview.can_create, preview.server_repo), (true, Some(3)));

    let by_hand = bench.desktop.run(move |core| core.open_pull_request(id, "WEB-1 Login", "Adds validation.")).await.unwrap();
    assert_eq!(by_hand.url, None);
    assert!(by_hand.browser_url.as_deref().is_some_and(|url| url.starts_with("https://github.com/acme/web/compare/main...")), "{by_hand:?}");
    assert_eq!(bench.thread(id).await.pull_request_url, None);

    // With a token the server opens it, for exactly the branch that was pushed.
    hub.state().github_token_missing = false;

    let opened = bench.desktop.run(move |core| core.open_pull_request(id, "WEB-1 Login", "Adds validation.")).await.unwrap();
    assert_eq!((opened.url.as_deref(), opened.browser_url), (Some("https://github.com/acme/web/pull/12"), None));
    assert_eq!(bench.thread(id).await.pull_request_url.as_deref(), Some("https://github.com/acme/web/pull/12"));

    let asked = hub.state().writes.iter().rev().find(|(_, path, _)| path.ends_with("/pull-request")).map(|(_, _, body)| body.clone()).unwrap();
    assert_eq!(asked, json!({ "repoId": 3, "head": branch, "base": "main", "title": "WEB-1 Login", "body": "Adds validation." }));

    bench.desktop.tick(true).await.unwrap();
    assert_eq!(hub.run(thread.run_id.unwrap())["run"]["pullRequestUrl"], "https://github.com/acme/web/pull/12");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_declined_here_is_declined_on_the_run_and_an_older_server_is_simply_not_told() {
    let (bench, hub, _repo) = ready("declined").await;
    let id = start(&bench, "WEB-1", ThreadKind::Implement, "").await;
    let thread = bench.until_state(id, ThreadState::Done).await;
    let run_id = thread.run_id.unwrap();
    let push = bench.desktop.run(move |core| core.thread(id)).await.unwrap().proposals[1].clone();
    let push_id = push.id;

    // The settlement does not get through; the next round catches up.
    hub.state().runs_api = false;
    bench.desktop.run(move |core| core.decide_proposal(push_id, false, Some("Not this week."))).await.unwrap();
    hub.state().runs_api = true;

    let on_run = |hub: &FakeHub| hub.run(run_id)["proposals"].as_array().unwrap().iter().find(|proposal| proposal["kind"] == "push").cloned().unwrap();

    assert_eq!(on_run(&hub)["state"], "pending");
    bench.desktop.tick(true).await.unwrap();
    assert_eq!((&on_run(&hub)["state"], &on_run(&hub)["note"]), (&json!("rejected"), &json!("Not this week.")));

    // A server that cannot show such proposals: they are decided here all the same.
    let (bench, hub, _repo) = ready("declined-old-server").await;

    hub.state().shows_no_pushes = true;

    let id = start(&bench, "WEB-1", ThreadKind::Implement, "").await;
    let thread = bench.until_state(id, ThreadState::Done).await;
    let proposals = bench.desktop.run(move |core| core.thread(id)).await.unwrap().proposals;

    assert_eq!(proposals.iter().map(|proposal| proposal.remote_id.is_some()).collect::<Vec<_>>(), vec![true, false, false]);
    assert_eq!(hub.run(thread.run_id.unwrap())["proposals"].as_array().unwrap().len(), 1);

    let push_id = proposals[1].id;
    let declined = bench.desktop.run(move |core| core.decide_proposal(push_id, false, None)).await.unwrap();

    assert_eq!(declined.state, ProposalState::Rejected);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_that_touches_ci_or_looks_like_a_secret_needs_a_second_confirmation() {
    let (bench, _hub, repo) = ready("risky").await;
    let id = start(&bench, "WEB-1", ThreadKind::Implement, "[fake:risky]").await;

    bench.until_state(id, ThreadState::Done).await;

    let preview = bench.desktop.run(move |core| core.push_preview(id)).await.unwrap();
    let found: Vec<(&str, Option<&str>)> = preview.findings.iter().map(|found| (found.kind.as_str(), found.path.as_deref())).collect();

    assert_eq!(found, vec![("ci", Some(".github/workflows/ci.yml")), ("secret", Some("src/config.txt"))]);

    let commit = preview.commit.clone();
    let refused = bench.desktop.run(move |core| core.push(id, &commit, false)).await.unwrap_err();

    assert!(matches!(refused, CoreError::NeedsConfirmation(_)), "{refused}");
    assert_eq!(git(&repo.origin, &["for-each-ref", "--format=%(refname)"]), "refs/heads/main");

    let commit = preview.commit.clone();

    bench.desktop.run(move |core| core.push(id, &commit, true)).await.unwrap();

    assert_eq!(git(&repo.origin, &["rev-parse", "refs/heads/feature/WEB-1-login-form-validation"]), preview.commit);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_the_server_refuses_goes_back_to_the_agent_and_a_trusted_kind_is_applied_at_once() {
    let (bench, hub, _repo) = ready("refused").await;

    hub.state().trusted = vec!["comment".to_string()];

    let id = start(&bench, "WEB-1", ThreadKind::Analyse, "[fake:bad-status]").await;

    bench.until_state(id, ThreadState::Done).await;

    let detail = bench.desktop.run(move |core| core.thread(id)).await.unwrap();
    let said = detail.events.iter().rev().find(|event| event.kind == EventKind::Message).unwrap();

    assert_eq!(said.text, "The first try was refused: Unknown status \"Shipped\" (valid: To Do, In Progress, In Review, Done)");
    assert_eq!(detail.proposals.len(), 1);
    assert_eq!(detail.proposals[0].payload, json!({ "status": "In Review" }));

    let other = start(&bench, "WEB-2", ThreadKind::Analyse, "[fake:analyse]").await;
    let done = bench.until_state(other, ThreadState::Done).await;
    let trusted = bench.desktop.run(move |core| core.thread(other)).await.unwrap().proposals.remove(0);

    assert_eq!((trusted.kind, trusted.state), (ProposalKind::Comment, ProposalState::Approved));
    assert_eq!(done.pending_proposals, 0);
    assert_eq!(hub.state().issues["WEB-2"].comments.len(), 1);
}

// ---------- without the runs API, after a restart, through the window's commands ----------

#[tokio::test(flavor = "multi_thread")]
async fn on_a_server_without_the_runs_api_threads_work_locally_and_approved_proposals_go_through_the_ordinary_api() {
    let (bench, hub, _repo) = ready("no-runs-api").await;

    hub.state().runs_api = false;

    let id = start(&bench, "WEB-1", ThreadKind::Implement, "").await;
    let done = bench.until_state(id, ThreadState::Done).await;
    let status = bench.desktop.run(|core| Ok(core.status())).await.unwrap();

    assert_eq!(done.run_id, None);
    assert_eq!(status.runner.runs_api, Some(false));
    assert!(hub.state().issues["WEB-1"].comments.is_empty());

    let comment = bench.desktop.run(move |core| core.thread(id)).await.unwrap().proposals[0].clone();

    assert_eq!((comment.kind, comment.remote_id), (ProposalKind::Comment, None));

    let comment_id = comment.id;
    let decided = bench.desktop.run(move |core| core.decide_proposal(comment_id, true, Some("fine"))).await.unwrap();

    assert_eq!(decided.state, ProposalState::Approved);
    assert_eq!(hub.state().issues["WEB-1"].comments[0].body, "Implemented on a branch; see the pull request. (WEB-1)");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_thread_can_be_told_more_in_the_same_session_and_gets_a_new_run() {
    let (bench, hub, _repo) = ready("follow-up").await;
    let id = start(&bench, "WEB-1", ThreadKind::Analyse, "[fake:analyse]").await;
    let first = bench.until_state(id, ThreadState::Done).await;

    bench.desktop.tick(true).await.unwrap();
    bench.desktop.run(move |core| core.say_to_thread(id, Some("Also cover the empty password."))).await.unwrap();

    let second = bench.until(id, "a second turn", |thread| thread.state == ThreadState::Done && thread.turns == 6).await;
    let (first_run, second_run) = (first.run_id.unwrap(), second.run_id.unwrap());

    assert_ne!(first_run, second_run);
    assert_eq!(second.session_id, first.session_id);
    assert_eq!(second.worktree, first.worktree);
    assert_eq!(hub.run(first_run)["run"]["state"], "succeeded");
    assert_eq!(hub.run(second_run)["run"]["state"], "succeeded");
    assert_eq!(hub.run(second_run)["run"]["instructions"], "Also cover the empty password.");
    assert_eq!(texts(hub.run(second_run)["events"].as_array().unwrap(), "message"), vec!["Continuing. You said: Also cover the empty password."]);
    // What was said here is on the run as said by the person, and is not heard a second time.
    assert_eq!(texts(hub.run(second_run)["events"].as_array().unwrap(), "answer"), vec!["Also cover the empty password."]);
    assert!(!texts(hub.run(second_run)["events"].as_array().unwrap(), "status").iter().any(|line| line.starts_with("Said in the desktop app")));
    bench.desktop.tick(true).await.unwrap();
    assert_eq!(bench.desktop.run(move |core| core.thread(id)).await.unwrap().thread.turns, 6, "the thread does not answer its own echo");
    assert_eq!(git(std::path::Path::new(&second.repo_path), &["rev-list", "--count", "origin/main..feature/WEB-1-login-form-validation"]), "1");
}

#[tokio::test(flavor = "multi_thread")]
async fn on_a_server_that_takes_no_answers_from_runners_what_was_said_is_a_line_of_the_runner() {
    let (bench, hub, _repo) = ready("follow-up-old-server").await;
    hub.state().refuses_runner_answers = true;
    let id = start(&bench, "WEB-1", ThreadKind::Analyse, "[fake:analyse]").await;
    bench.until_state(id, ThreadState::Done).await;

    bench.desktop.tick(true).await.unwrap();
    bench.desktop.run(move |core| core.say_to_thread(id, Some("Also cover the empty password."))).await.unwrap();

    let second = bench.until(id, "a second turn", |thread| thread.state == ThreadState::Done && thread.turns == 6).await;
    let events = hub.run(second.run_id.unwrap())["events"].clone();

    assert_eq!(texts(events.as_array().unwrap(), "status")[0], "Said in the desktop app: Also cover the empty password.");
    assert!(texts(events.as_array().unwrap(), "answer").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_left_working_when_the_app_died_is_marked_interrupted_and_can_be_resumed() {
    let (bench, _hub, _repo) = ready("restart").await;
    let id = start(&bench, "WEB-1", ThreadKind::Custom, "[fake:slow]").await;

    bench.until(id, "working", |thread| thread.last_text.is_some()).await;

    // What the disk holds at the moment the app dies: a thread that says it is working.
    let died = support::TempDir::new("restart-died");

    for entry in std::fs::read_dir(bench.data.path().join("state")).unwrap().flatten() {
        std::fs::create_dir_all(died.path().join("state")).unwrap();
        std::fs::copy(entry.path(), died.path().join("state").join(entry.file_name())).unwrap();
    }

    bench.desktop.shutdown().await;

    let mut options = hub_desktop_core::OpenOptions::new(hub_desktop_core::config::Paths::under(died.path()), bench.keychain.clone());

    options.helper = Some(support::fake_agent());
    options.stop_grace = Duration::from_millis(400);

    let reopened = Desktop::open(options).unwrap();
    let detail = reopened.run(move |core| core.thread(id)).await.unwrap();

    assert_eq!(detail.thread.state, ThreadState::Stopped);
    assert!(detail.events.last().unwrap().text.starts_with("Interrupted"));
    assert_eq!(detail.thread.session_id.as_deref(), Some("fake-session-1"));

    // Resumed, it continues the same session in the same tree.
    reopened.run(|core| Ok(core.reconnect())).await.unwrap();
    reopened.run(move |core| core.say_to_thread(id, Some("Go on."))).await.unwrap();

    assert!(reopened.wait_idle(Duration::from_secs(10)).await);

    let resumed = reopened.run(move |core| core.thread(id)).await.unwrap();

    assert_eq!(resumed.thread.state, ThreadState::Done);
    assert_eq!(resumed.thread.worktree, detail.thread.worktree);
    assert!(resumed.events.iter().any(|event| event.text == "Continuing. You said: Go on."));
}

#[tokio::test(flavor = "multi_thread")]
async fn quitting_the_app_stops_every_working_thread() {
    let (bench, hub, _repo) = ready("shutdown").await;
    let id = start(&bench, "WEB-1", ThreadKind::Custom, "[fake:slow]").await;
    let working = bench.until(id, "working", |thread| thread.last_text.is_some()).await;
    let pid = bench.desktop.helper_pid(id).unwrap();

    bench.desktop.shutdown().await;

    assert_eq!(bench.thread(id).await.state, ThreadState::Stopped);
    assert_eq!(hub.run(working.run_id.unwrap())["run"]["state"], "cancelled");

    #[cfg(unix)]
    assert!(!support::group_alive(pid));
}

/// The helper's side of the contract: when its input closes, its parent is gone, and it ends
/// together with whatever it started.
#[cfg(unix)]
#[test]
fn a_helper_whose_parent_is_gone_ends_and_takes_its_children_along() {
    use std::io::{BufRead, Write};
    use std::os::unix::process::CommandExt;

    let folder = support::TempDir::new("orphan");
    let (program, _) = support::fake_agent();
    let mut child = std::process::Command::new(program)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = child.id();
    // Built from the core's own types, so the test cannot drift from the protocol.
    let start = serde_json::to_string(&hub_desktop_core::protocol::ToSidecar::Start(Box::new(hub_desktop_core::protocol::StartThread {
        thread: 1,
        prompt: "Issue WEB-1: x\n\n[fake:slow]".to_string(),
        cwd: folder.path().to_string_lossy().into_owned(),
        model: "m".to_string(),
        resume: None,
        max_turns: 5,
        rules: String::new(),
        allowed_tools: Vec::new(),
        denied_tools: Vec::new(),
        sandbox: None,
        hub_tools: Vec::new(),
        commit_tool: false,
        env: Default::default(),
    })))
    .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();

    writeln!(stdin, "{start}").unwrap();

    // Wait until it says it is working, which is after it started its child process.
    while let Some(Ok(line)) = lines.next() {
        if line.contains("This will take a while") {
            break;
        }
    }

    std::thread::sleep(Duration::from_millis(200));
    assert!(support::group_alive(pid));

    drop(stdin);
    child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(200));

    assert!(!support::group_alive(pid), "the helper's child outlived it");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_windows_commands_reach_the_same_core() {
    let (bench, _hub, repo) = ready("commands").await;
    let desktop = &bench.desktop;
    let status = commands::invoke(desktop, "status", Value::Null).await.unwrap();
    let dashboard = commands::invoke(desktop, "dashboard", Value::Null).await.unwrap();
    let issue = commands::invoke(desktop, "issue", json!({ "key": "WEB-1" })).await.unwrap();
    let check = commands::invoke(desktop, "check_repo", json!({ "path": repo.clone, "githubRepo": "acme/web" })).await.unwrap();
    let thread =
        commands::invoke(desktop, "start_thread", json!({ "issueKey": "WEB-1", "kind": "analyse", "instructions": "[fake:analyse]" })).await.unwrap();
    let id = thread["id"].as_i64().unwrap();

    bench.until_state(id, ThreadState::Done).await;

    let detail = commands::invoke(desktop, "thread", json!({ "id": id })).await.unwrap();
    let diff = commands::invoke(desktop, "thread_diff", json!({ "id": id })).await.unwrap();
    let unknown = commands::invoke(desktop, "format_disk", Value::Null).await.unwrap_err();
    let bad = commands::invoke(desktop, "issue", json!({})).await.unwrap_err();
    let missing = commands::invoke(desktop, "issue", json!({ "key": "WEB-99" })).await.unwrap_err();

    assert_eq!(status["online"], true);
    assert_eq!(status["runner"]["id"], desktop.core().settings().runner_id);
    assert_eq!(dashboard["dashboard"]["assigned"].as_array().unwrap().len(), 2);
    assert_eq!(issue["issue"]["key"], "WEB-1");
    assert_eq!(issue["project"]["statuses"].as_array().unwrap().len(), 4);
    assert_eq!(check["matches"], false);
    assert_eq!(thread["kind"], "analyse");
    assert_eq!(detail["thread"]["state"], "done");
    assert_eq!(detail["proposals"][0]["kind"], "comment");
    assert_eq!(diff["files"], json!([]));
    assert!(unknown.contains("no command called format_disk"));
    assert!(bad.contains("missing field"));
    assert_eq!(missing, "No such issue");

    for command in commands::COMMANDS {
        let answer = commands::invoke(desktop, command, json!({ "nonsense": true })).await;

        assert!(!answer.is_err_and(|error| error.contains("no command called")), "{command} is listed but not handled");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_on_the_server_arrive_through_the_live_stream_and_delegated_runs_start_by_themselves() {
    let (bench, hub, _repo) = ready("live").await;
    let mut events = bench.desktop.subscribe();

    bench.desktop.start_background();
    tokio::time::sleep(Duration::from_millis(400)).await;
    bench.desktop.run(|core| core.comment("WEB-1", "From somewhere.")).await.unwrap();

    let run_id = hub.delegate("WEB-2", "custom", "[fake:analyse]");
    let mut saw_issue = false;
    let mut delegated = None;

    for _ in 0..200 {
        while let Ok(event) = events.try_recv() {
            match event {
                AppEvent::DataChanged { scope, issue_key } if scope == "issue" && issue_key.as_deref() == Some("WEB-1") => saw_issue = true,
                AppEvent::Notify { title, thread_id, .. } if title.contains("delegated") => delegated = thread_id,
                _ => {}
            }
        }

        let finished = hub.run(run_id)["run"]["state"] == "succeeded";

        if saw_issue && delegated.is_some() && finished {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(saw_issue, "the comment was announced on the live stream");
    assert_eq!(hub.run(run_id)["run"]["state"], "succeeded");
    assert_eq!(bench.thread(delegated.unwrap()).await.state, ThreadState::Done);

    bench.desktop.close();
}
