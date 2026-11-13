//! An agent without a model. It speaks the helper's JSON lines exactly as the real sidecar
//! does, so the whole life of a thread can be tried on a machine with no Claude account: it
//! reads the issue through the core, writes a file in its working tree, commits, proposes,
//! asks, crashes or hangs, depending on a marker in the prompt:
//!
//! `[fake:question]` asks and ends its turn, then works when resumed (`[fake:full]`: then
//! implements and proposes) · `[fake:crash]` dies ·
//! `[fake:hang]` ignores `stop` · `[fake:slow]` works until stopped · `[fake:fail]` reports a
//! failure · `[fake:error]` reports an error · `[fake:write-tool]` tries a tool that changes
//! the tracker · `[fake:subtasks]` proposes subtasks and a status · `[fake:risky]` commits a
//! CI file and something that looks like a token · `[fake:analyse]` changes nothing.
//! Without a marker it implements: a commit, then proposals to comment, push and open a PR.

use std::io::{BufRead, Write};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use hub_desktop_core::protocol::{self, AgentEvent, FromSidecar, StartThread, ToSidecar, PROTOCOL_VERSION};
use serde_json::{json, Value};

struct Agent {
    inbox: Receiver<ToSidecar>,
    requests: u64,
}

fn send(message: &FromSidecar) {
    let mut out = std::io::stdout().lock();

    let _ = writeln!(out, "{}", serde_json::to_string(message).unwrap_or_default());
    let _ = out.flush();
}

fn emit(event: AgentEvent) {
    send(&FromSidecar::Event { event });
}

fn say(text: &str) {
    emit(AgentEvent::Assistant { text: text.to_string() });
}

fn tool(name: &str, detail: &str) {
    emit(AgentEvent::Tool { name: name.to_string(), detail: detail.to_string() });
}

fn result(ok: bool, summary: &str) {
    emit(AgentEvent::Result {
        ok,
        summary: summary.to_string(),
        cost_usd: Some(0.0123),
        turns: Some(3),
        input_tokens: Some(1200),
        output_tokens: Some(340),
    });
}

fn git(cwd: &str, arguments: &[&str]) -> bool {
    Command::new("git")
        .args(["-c", "user.name=Fake Agent", "-c", "user.email=agent@example.invalid", "-c", "commit.gpgsign=false"])
        .args(arguments)
        .current_dir(cwd)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn write(cwd: &str, path: &str, content: &str) {
    let file = Path::new(cwd).join(path);

    if let Some(folder) = file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }

    let _ = std::fs::write(file, content);
    tool("Write", path);
}

fn commit(cwd: &str, message: &str) {
    tool("Bash", &format!("git commit -m \"{message}\""));
    git(cwd, &["add", "--all"]);
    git(cwd, &["commit", "-m", message]);
}

impl Agent {
    /// Asks the core and waits for its reply. `None` when told to stop meanwhile.
    fn ask(&mut self, tool_name: &str, input: Value) -> Option<Result<Value, String>> {
        self.requests += 1;

        let request = self.requests;

        tool(&format!("mcp__team-hub__{}", tool_name.trim_start_matches(protocol::READ_PREFIX)), &input.to_string());
        send(&FromSidecar::Request { request, tool: tool_name.to_string(), input });

        loop {
            match self.inbox.recv().ok()? {
                ToSidecar::Reply { request: answered, ok, result } if answered == request => {
                    return Some(if ok { Ok(result) } else { Err(result.as_str().unwrap_or_default().to_string()) });
                }
                ToSidecar::Stop => return None,
                _ => {}
            }
        }
    }

    fn stopped_within(&self, limit: Duration) -> bool {
        matches!(self.inbox.recv_timeout(limit), Ok(ToSidecar::Stop) | Err(mpsc::RecvTimeoutError::Disconnected))
    }
}

fn issue_key(prompt: &str) -> String {
    prompt.strip_prefix("Issue ").and_then(|rest| rest.split(':').next()).unwrap_or("ISSUE-0").trim().to_string()
}

fn scenario(prompt: &str) -> &str {
    prompt.split("[fake:").nth(1).and_then(|rest| rest.split(']').next()).unwrap_or("implement")
}

/// What a real agent keeps in its session: here, a small file per working tree and session.
fn memory_file(cwd: &str, session: &str) -> std::path::PathBuf {
    let tree = cwd.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3));

    std::env::temp_dir().join("hub-fake-agent").join(format!("{tree:016x}-{session}"))
}

fn work(agent: &mut Agent, start: &StartThread) -> Option<()> {
    let cwd = start.cwd.as_str();
    let resumed = start.resume.is_some();
    let session = start.resume.clone().unwrap_or_else(|| format!("fake-session-{}", start.thread));
    let memory = memory_file(cwd, &session);
    let remembered = std::fs::read_to_string(&memory).unwrap_or_default();
    let (key, kind) = match remembered.split_once('\n') {
        Some((key, kind)) if resumed => (key.to_string(), kind.to_string()),
        _ => (issue_key(&start.prompt), scenario(&start.prompt).to_string()),
    };

    if let Some(folder) = memory.parent() {
        let _ = std::fs::create_dir_all(folder);
        let _ = std::fs::write(&memory, format!("{key}\n{kind}"));
    }

    emit(AgentEvent::Session { session_id: session });

    if resumed && kind == "full" {
        let answer = start.prompt.lines().last().unwrap_or_default();

        say(&format!("Thanks. With \"{answer}\" I can finish."));
        write(cwd, &format!("agent-notes/{key}.md"), &format!("# {key}\n\nDecision: {answer}\n\nImplemented by the fake agent.\n"));
        commit(cwd, &format!("{key}: implement with the decided limit"));
        agent
            .ask(
                protocol::PROPOSE_COMMENT,
                json!({ "body": format!("Implemented on a branch with the decision \"{answer}\"; a pull request follows.") }),
            )?
            .ok();
        agent.ask(protocol::PROPOSE_PUSH, json!({ "note": "One commit." }))?.ok();
        agent.ask(protocol::PROPOSE_PULL_REQUEST, json!({ "title": format!("{key} implement"), "body": format!("Implements {key}.") }))?.ok();
        result(true, &format!("Implemented {key} in one commit; proposed a comment, the push and a pull request."));

        return Some(());
    }

    if resumed {
        say(&format!("Continuing. You said: {}", start.prompt.lines().last().unwrap_or_default()));
        write(cwd, &format!("agent-notes/{key}-follow-up.md"), &format!("# Follow-up for {key}\n\n{}\n", start.prompt));
        commit(cwd, &format!("{key}: follow up on the answer"));
        result(true, "Followed up on what you said and committed the change.");

        return Some(());
    }

    say(&format!("Reading {key} before touching the code."));

    let can_read = start.hub_tools.iter().any(|known| known.name == "get_issue");
    let issue = if can_read { agent.ask("read:get_issue", json!({ "key": key }))? } else { Err("no connection to the tracker".to_string()) };
    let summary = issue
        .as_ref()
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(text.as_str().unwrap_or_default()).ok())
        .and_then(|issue| issue["summary"].as_str().map(String::from))
        .unwrap_or_else(|| "the issue as quoted in the prompt".to_string());

    match kind.as_str() {
        "crash" => {
            say("About to fall over.");
            eprintln!("fake agent: simulated crash");
            std::process::exit(3);
        }
        "hang" => {
            say("Working on something that never ends.");

            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        "slow" => {
            say("This will take a while.");

            // Leaves a child behind on purpose: stopping the thread has to take it down too.
            let _ = Command::new("sleep").arg("600").spawn();

            if agent.stopped_within(Duration::from_secs(120)) {
                return None;
            }

            result(true, "Finished the slow work.");
        }
        "fail" => result(false, "The tests of the project do not pass and I could not find out why."),
        "error" => emit(AgentEvent::Error { message: "The model could not be reached.".to_string() }),
        "question" | "full" => {
            agent.ask(protocol::ASK_QUESTION, json!({ "question": "Should the limit be 10 or 20 attempts?" }))?.ok();
            result(true, "I need one decision before I can continue: the limit.");
        }
        "write-tool" => {
            let refused = agent.ask("read:add_comment", json!({ "key": key, "body": "Posted without asking" }))?;
            let unknown = agent.ask("delete_everything", json!({}))?;

            say(&format!(
                "add_comment: {}; delete_everything: {}",
                refused.err().unwrap_or_else(|| "ALLOWED".into()),
                unknown.err().unwrap_or_else(|| "ALLOWED".into())
            ));
            result(true, "Tried two things I am not allowed to do.");
        }
        "subtasks" => {
            let items = json!([{ "summary": "Validate the form on the server", "description": "Mirror the client rules." }, { "summary": "Add tests for the limit" }]);

            agent.ask(protocol::PROPOSE_SUBTASKS, json!({ "items": items }))?.ok();
            agent.ask(protocol::PROPOSE_STATUS, json!({ "status": "In Progress" }))?.ok();
            result(true, "Split the work into two subtasks and proposed to start the issue.");
        }
        "bad-status" => {
            let refused = agent.ask(protocol::PROPOSE_STATUS, json!({ "status": "Shipped" }))?;

            say(&format!("The first try was refused: {}", refused.err().unwrap_or_else(|| "it was not".into())));
            agent.ask(protocol::PROPOSE_STATUS, json!({ "status": "In Review" }))?.ok();
            result(true, "Proposed to move the issue to review.");
        }
        "no-shell" => {
            // What a thread without a shell does: the file tools write, the app commits.
            let _ = std::fs::write(Path::new(cwd).join("CHANGELOG.md"), format!("- {key}\n"));

            tool("Write", "CHANGELOG.md");

            let made = agent.ask(protocol::COMMIT_CHANGES, json!({ "message": format!("{key}: note the change"), "paths": ["CHANGELOG.md"] }))?;

            say(&format!(
                "commit_changes offered: {}; answer: {}",
                start.commit_tool,
                made.map(|text| text.as_str().unwrap_or_default().to_string()).unwrap_or_else(|error| error)
            ));
            result(true, "Committed through the app.");
        }
        "env" => {
            let seen: Vec<&str> = ["TEAM_HUB_TOKEN", "GH_TOKEN", "PATH"].into_iter().filter(|name| std::env::var_os(name).is_some()).collect();

            say(&format!("In my environment: {}", seen.join(", ")));
            result(true, "Looked at the environment.");
        }
        "analyse" => {
            agent.ask(protocol::PROPOSE_COMMENT, json!({ "body": format!("Analysis of {key}: the change belongs in one module.") }))?.ok();
            result(true, "Read the code; nothing was changed.");
        }
        "risky" => {
            write(cwd, ".github/workflows/ci.yml", "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: make\n");
            write(cwd, "src/config.txt", "api_key = \"abcdef0123456789abcdef\"\n");
            commit(cwd, &format!("{key}: change the pipeline"));
            agent.ask(protocol::PROPOSE_PUSH, json!({ "note": "CI now builds with make." }))?.ok();
            result(true, "Changed the pipeline and proposed the push.");
        }
        _ => {
            write(cwd, &format!("agent-notes/{key}.md"), &format!("# {key}\n\n{summary}\n\nImplemented by the fake agent.\n"));
            commit(cwd, &format!("{key}: implement {}", hub_core::text::truncate(&summary, 50)));
            agent.ask(protocol::PROPOSE_COMMENT, json!({ "body": format!("Implemented on a branch; see the pull request. ({key})") }))?.ok();
            agent.ask(protocol::PROPOSE_PUSH, json!({ "note": "One commit, tests not run (no test suite)." }))?.ok();
            agent.ask(protocol::PROPOSE_PULL_REQUEST, json!({ "title": format!("{key} {summary}"), "body": format!("Implements {key}.") }))?.ok();
            say("The change is committed locally.");
            result(true, &format!("Implemented {key} in one commit and proposed a comment, the push and a pull request."));
        }
    }

    Some(())
}

/// Ends this process and everything it started (the helper leads its own process group).
fn end_with_children() -> ! {
    #[cfg(unix)]
    unsafe {
        libc::kill(0, libc::SIGKILL);
    }

    std::process::exit(0)
}

fn main() {
    let (sender, inbox) = mpsc::channel();

    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if let Ok(message) = serde_json::from_str::<ToSidecar>(&line) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        }

        // The input closed: whoever started this agent is gone. Nothing may work on unseen.
        end_with_children();
    });

    send(&FromSidecar::Ready { protocol: PROTOCOL_VERSION });

    let mut agent = Agent { inbox, requests: 0 };

    while let Ok(message) = agent.inbox.recv() {
        if let ToSidecar::Start(start) = message {
            let _ = work(&mut agent, &start);

            break;
        }
    }
}
