//! `hub`: the desktop app's core from a terminal. The same connection, repositories, threads
//! and runner as the window, which also makes the core testable end to end without one.

use std::io::BufRead;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use hub_core::priority_name;
use hub_desktop_core::config::Paths;
use hub_desktop_core::model::{AppEvent, NewThread, Thread, ThreadKind, ThreadState};
use hub_desktop_core::runner::{self, line_of};
use hub_desktop_core::secrets::{OsKeychain, TOKEN_VARIABLE};
use hub_desktop_core::{CoreError, CoreResult, Desktop, OpenOptions};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "hub", version, about = "team-hub from the terminal: my work, repositories, agent threads, the runner")]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The connection, my work and the threads at a glance.
    Status,
    /// Connect to a server. The API token is read from standard input (or TEAM_HUB_TOKEN) and kept in the keychain.
    Connect { url: String },
    /// Forget the token and the stored copy of the server's data.
    Disconnect,
    /// One issue: fields, description, comments, code.
    Issue { key: String },
    /// Local clones mapped to projects.
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    /// Agent threads.
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    /// Agent runs on the server.
    Runs {
        /// Only the runs I requested.
        #[arg(long)]
        mine: bool,
    },
    /// Act as a runner: claim runs delegated on the web and carry them out, until interrupted.
    Runner {
        /// Leave once nothing is queued, working or left to claim.
        #[arg(long)]
        once: bool,
    },
    /// Every runner I may see: whose it is, what it takes, what it is carrying out.
    Runners,
    /// A project's test jobs.
    Tests {
        #[command(subcommand)]
        command: TestsCommand,
    },
    /// Proposals waiting for a decision.
    Proposals,
    /// Decide a proposal of a thread.
    Decide {
        id: i64,
        #[arg(long, conflicts_with = "reject")]
        approve: bool,
        #[arg(long)]
        reject: bool,
        #[arg(long)]
        note: Option<String>,
    },
    /// Show what pushing a thread's branch would send; push it with --yes.
    Push {
        thread: i64,
        /// Push exactly the commit shown.
        #[arg(long)]
        yes: bool,
        /// Also confirm what the preview flags (CI or build files, something like a secret).
        #[arg(long)]
        confirm_findings: bool,
    },
}

#[derive(Subcommand)]
enum RepoCommand {
    List,
    /// Map a project to a local clone; with --github the clone's origin must be that repository.
    Add {
        project: String,
        path: String,
        #[arg(long)]
        github: Option<String>,
    },
    Remove {
        id: i64,
    },
}

#[derive(Subcommand)]
enum TestsCommand {
    /// The project's jobs, newest first.
    List { project: String },
    /// Start the project's suite and follow it. Without --here or --on, any runner that fits takes it.
    Run {
        project: String,
        /// Run it on this computer, in this process.
        #[arg(long, conflicts_with = "on")]
        here: bool,
        /// Send it to the runner with this id (see `hub runners`).
        #[arg(long)]
        on: Option<String>,
        /// The branch, tag or commit; the project's default when left out.
        #[arg(long = "ref")]
        reference: Option<String>,
        /// Test files or a --grep pattern, as for `e2e run`.
        #[arg(long)]
        filter: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Implement,
    Review,
    Analyse,
    Custom,
}

#[derive(Subcommand)]
enum ThreadCommand {
    /// Start a thread for an issue and follow it until it ends or needs a person.
    Run {
        key: String,
        #[arg(long, value_enum, default_value = "implement")]
        kind: Kind,
        #[arg(long, default_value = "")]
        instructions: String,
        #[arg(long)]
        model: Option<String>,
        /// The mapped repository to work in, by its id.
        #[arg(long)]
        repo: Option<i64>,
        /// Continue this branch instead of the one suggested for the issue.
        #[arg(long)]
        branch: Option<String>,
    },
    List,
    /// What a thread said and did, and its proposals.
    Show {
        id: i64,
    },
    /// The thread's changes against the point it started from.
    Diff {
        id: i64,
    },
    /// Answer a thread's question or tell it more, and follow it.
    Say {
        id: i64,
        text: String,
    },
    /// Note the pull request opened for the thread's branch, so the thread and its run show it.
    Pr {
        id: i64,
        #[arg(long)]
        url: String,
    },
    /// Remove the thread's working tree (the branch stays).
    Remove {
        id: i64,
    },
}

fn print_json<T: Serialize>(value: &T) {
    println!("{}", serde_json::to_string_pretty(value).unwrap_or_default());
}

fn line_of_thread(thread: &Thread) -> String {
    let needs = match (thread.pending_question, thread.pending_proposals) {
        (true, _) => "  · waits for an answer".to_string(),
        (false, 0) => String::new(),
        (false, count) => format!("  · {count} to decide"),
    };
    let run = thread.run_id.map_or("local".to_string(), |id| format!("run {id}"));

    format!(
        "#{:<3} {:<8} {:<10} {:<9} {:<8} {}{needs}",
        thread.id,
        thread.state.as_str(),
        thread.issue_key,
        thread.kind.as_str(),
        run,
        thread.branch.as_deref().unwrap_or("-")
    )
}

/// Follows one thread: prints what it says, keeps the runner's rounds going, and returns when
/// the thread is no longer working.
async fn follow(desktop: &Desktop, id: i64) -> CoreResult<Thread> {
    let mut events = desktop.subscribe();

    loop {
        tokio::select! {
            event = events.recv() => {
                if let Ok(AppEvent::ThreadEvent { thread_id, event }) = event {
                    if thread_id == id {
                        println!("{}", line_of(&event));
                    }
                }
            }
            () = tokio::time::sleep(Duration::from_millis(400)) => {
                desktop.tick(true).await?;

                let thread = desktop.run(move |core| Ok(core.thread(id)?.thread)).await?;

                if !thread.state.is_active() {
                    // Whatever was said after the last look.
                    while let Ok(event) = events.try_recv() {
                        if let AppEvent::ThreadEvent { thread_id, event } = event {
                            if thread_id == id {
                                println!("{}", line_of(&event));
                            }
                        }
                    }

                    desktop.tick(true).await?;

                    return Ok(thread);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("Stopping thread {id}…");

                let thread = desktop.stop_thread(id).await?;

                desktop.tick(false).await?;

                return Ok(thread);
            }
        }
    }
}

fn outcome(thread: &Thread) -> ExitCode {
    println!("\n{}", line_of_thread(thread));

    if let Some(summary) = &thread.summary {
        println!("{summary}");
    }

    match thread.state {
        ThreadState::Done => ExitCode::SUCCESS,
        ThreadState::Waiting => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}

async fn status(desktop: &Desktop, json: bool) -> CoreResult<()> {
    let status = desktop.run(|core| Ok(core.status())).await?;
    let dashboard = desktop.run(|core| core.dashboard()).await;
    let threads = desktop.run(|core| core.threads()).await?;

    if json {
        print_json(&serde_json::json!({ "status": status, "dashboard": dashboard.as_ref().ok(), "threads": threads }));

        return Ok(());
    }

    match (&status.server_url, &status.user) {
        (Some(url), Some(user)) => println!("{url} as {} ({})", user.name, if status.online { "online" } else { "offline: showing the stored copy" }),
        (Some(url), None) => println!("{url}: {}", status.error.as_deref().unwrap_or("no token; run `hub connect`")),
        (None, _) => println!("Not connected. Run `hub connect <server address>`."),
    }

    println!(
        "Runner {} ({}){}",
        status.runner.name,
        status.runner.id,
        match status.runner.runs_api {
            Some(false) => " · this server has no agent runs API",
            _ => "",
        }
    );
    println!("Shell sandbox: {} · stored copy: {}", status.sandbox, status.cache);

    match dashboard {
        Ok(view) => {
            let status_of = |row: &hub_core::IssueRow| {
                view.projects
                    .iter()
                    .flat_map(|project| &project.statuses)
                    .find(|status| status.id == row.status_id)
                    .map_or("?", |status| status.name.as_str())
            };

            println!("\nMy work ({} as of {})", view.dashboard.assigned.len(), view.fetched_at);

            for row in &view.dashboard.assigned {
                println!("  {:<10} {:<12} {:<8} {}", row.key, status_of(row), priority_name(row.priority), row.summary);
            }
        }
        Err(CoreError::NotConnected) => {}
        Err(error) => println!("\nMy work: {error}"),
    }

    if !threads.is_empty() {
        println!("\nThreads");

        for thread in threads.iter().take(15) {
            println!("  {}", line_of_thread(thread));
        }
    }

    Ok(())
}

async fn run(cli: Cli, desktop: &Desktop) -> CoreResult<ExitCode> {
    let json = cli.json;

    match cli.command {
        Command::Status => status(desktop, json).await?,
        Command::Connect { url } => {
            let token = match std::env::var(TOKEN_VARIABLE) {
                Ok(token) if !token.trim().is_empty() => token,
                _ => {
                    eprintln!("Paste the API token (team-hub → Settings → API tokens) and press Enter:");

                    std::io::stdin().lock().lines().next().transpose()?.unwrap_or_default()
                }
            };
            let status = desktop.run(move |core| core.connect(&url, &token)).await?;

            println!("Connected to {} as {}.", status.server_url.unwrap_or_default(), status.user.map(|user| user.name).unwrap_or_default());
        }
        Command::Disconnect => {
            desktop.run(|core| core.disconnect()).await?;
            println!("The token and the stored copy are gone.");
        }
        Command::Issue { key } => {
            let view = desktop.run(move |core| core.issue(&key)).await?;

            if json {
                print_json(&view);

                return Ok(ExitCode::SUCCESS);
            }

            let issue = &view.issue;
            let project = view.project.as_ref();
            let status = project
                .and_then(|project| project.statuses.iter().find(|status| status.id == issue.row.status_id))
                .map_or("?", |status| status.name.as_str());

            println!("{} {}{}", issue.row.key, issue.row.summary, if view.offline { "  (stored copy)" } else { "" });
            println!(
                "{status} · {} · {} comments · spent {}",
                priority_name(issue.row.priority),
                issue.comments.len(),
                hub_core::text::format_duration(issue.spent_minutes)
            );

            if !issue.description.trim().is_empty() {
                println!("\n{}", issue.description.trim());
            }

            for comment in &issue.comments {
                println!(
                    "\n— {} ({})\n{}",
                    comment.author_label.clone().unwrap_or_else(|| format!("user {}", comment.author_id.unwrap_or_default())),
                    comment.created_at,
                    comment.body
                );
            }

            for (module, panel) in &issue.panels {
                if let Some(prs) = panel.get("prs").and_then(|prs| prs.as_array()) {
                    for pr in prs {
                        println!(
                            "\n[{module}] {} #{} {} ({})",
                            pr["repo"].as_str().unwrap_or_default(),
                            pr["number"],
                            pr["title"].as_str().unwrap_or_default(),
                            pr["state"].as_str().unwrap_or_default()
                        );
                    }
                }
            }
        }
        Command::Repo { command } => match command {
            RepoCommand::List => {
                let repos = desktop.run(|core| core.repos()).await?;

                if json {
                    print_json(&repos);

                    return Ok(ExitCode::SUCCESS);
                }

                for repo in repos {
                    let state = match (&repo.error, repo.changed_files) {
                        (Some(error), _) => error.clone(),
                        (None, 0) => "clean".to_string(),
                        (None, changed) => format!("{changed} changed"),
                    };

                    println!(
                        "#{} {} {} → {}",
                        repo.mapping.id,
                        repo.mapping.project_key,
                        repo.mapping.github_repo.as_deref().unwrap_or("-"),
                        repo.mapping.path
                    );
                    println!("    default {} · {state} · {} working trees", repo.default_branch.as_deref().unwrap_or("?"), repo.worktrees.len());

                    for tree in repo.worktrees {
                        println!(
                            "    {} {}{}",
                            tree.head,
                            tree.branch.as_deref().unwrap_or("(detached)"),
                            tree.thread_id.map(|id| format!("  thread #{id}")).unwrap_or_default()
                        );
                    }
                }
            }
            RepoCommand::Add { project, path, github } => {
                let mapping = desktop.run(move |core| core.map_repo(&project, github.as_deref(), &path)).await?;

                println!("Mapped {} to {} (#{}).", mapping.project_key, mapping.path, mapping.id);
            }
            RepoCommand::Remove { id } => {
                desktop.run(move |core| core.unmap_repo(id)).await?;
                println!("Removed.");
            }
        },
        Command::Thread { command } => match command {
            ThreadCommand::Run { key, kind, instructions, model, repo, branch } => {
                let kind = match kind {
                    Kind::Implement => ThreadKind::Implement,
                    Kind::Review => ThreadKind::Review,
                    Kind::Analyse => ThreadKind::Analyse,
                    Kind::Custom => ThreadKind::Custom,
                };
                let new = NewThread { issue_key: key, kind, instructions, model, repo_id: repo, branch };
                let thread = desktop.run(move |core| core.start_thread(&new)).await?;

                println!("{}", line_of_thread(&thread));

                return Ok(outcome(&follow(desktop, thread.id).await?));
            }
            ThreadCommand::List => {
                let threads = desktop.run(|core| core.threads()).await?;

                if json {
                    print_json(&threads);
                } else {
                    threads.iter().for_each(|thread| println!("{}", line_of_thread(thread)));
                }
            }
            ThreadCommand::Show { id } => {
                let detail = desktop.run(move |core| core.thread(id)).await?;

                if json {
                    print_json(&detail);

                    return Ok(ExitCode::SUCCESS);
                }

                println!(
                    "{}\n{} · {}",
                    line_of_thread(&detail.thread),
                    detail.thread.issue_summary,
                    detail.thread.worktree.as_deref().unwrap_or("no working tree")
                );
                detail.events.iter().for_each(|event| println!("{}", line_of(event)));

                for proposal in detail.proposals {
                    println!("proposal #{} {} [{}] {}", proposal.id, proposal.kind, proposal.state, proposal.payload);
                }
            }
            ThreadCommand::Diff { id } => {
                let diff = desktop.run(move |core| core.thread_diff(id)).await?;

                if json {
                    print_json(&diff);

                    return Ok(ExitCode::SUCCESS);
                }

                println!("Against {} · {} commits · {} uncommitted", diff.base, diff.commits.len(), diff.uncommitted);
                diff.commits.iter().for_each(|commit| println!("  {} {}", commit.short, commit.subject));

                for file in diff.files {
                    println!("\n{} {} (+{} −{})", file.status, file.path, file.additions, file.deletions);

                    for hunk in file.hunks {
                        println!("{}", hunk.header);

                        for line in hunk.lines {
                            let sign = match line.kind.as_str() {
                                "add" => '+',
                                "del" => '-',
                                _ => ' ',
                            };

                            println!("{sign}{}", line.text);
                        }
                    }
                }
            }
            ThreadCommand::Say { id, text } => {
                desktop.run(move |core| core.say_to_thread(id, Some(&text))).await?;

                return Ok(outcome(&follow(desktop, id).await?));
            }
            ThreadCommand::Pr { id, url } => {
                let thread = desktop.run(move |core| core.record_pull_request(id, &url)).await?;

                desktop.tick(true).await?;
                println!("{}", line_of_thread(&thread));
            }
            ThreadCommand::Remove { id } => {
                desktop.remove_worktree(id).await?;
                println!("The working tree of thread {id} is removed; its branch is kept.");
            }
        },
        Command::Runs { mine } => {
            let runs = desktop.run(move |core| core.runs(mine)).await?;

            if json {
                print_json(&runs);
            } else {
                for run in runs {
                    let runner = run.runner_name.as_deref().unwrap_or("-");

                    println!(
                        "{:<5} {:<10} {:<9} {:<10} {:<24} {}",
                        run.id,
                        run.state.as_str(),
                        run.kind.as_str(),
                        run.issue_key,
                        runner,
                        run.branch.as_deref().unwrap_or("")
                    );
                }
            }
        }
        Command::Runner { once } => runner::attended(desktop, once).await?,
        Command::Runners => {
            let runners = desktop.run(|core| core.runners()).await?;
            let here = desktop.core().settings().runner_id;

            if json {
                print_json(&runners);

                return Ok(ExitCode::SUCCESS);
            }

            for runner in runners.as_array().cloned().unwrap_or_default() {
                let words = |value: &serde_json::Value| value.as_array().map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>().join(", "));
                let kind = if runner["interactive"] == true { "computer" } else if runner["shared"] == true { "shared service" } else { "service" };
                let mark = if runner["id"] == here.as_str() { "  (this computer)" } else { "" };

                println!("{} · {} · {kind} · {}{mark}", runner["id"].as_str().unwrap_or_default(), runner["name"].as_str().unwrap_or_default(), if runner["online"] == true { "online" } else { "offline" });

                for work in runner["work"].as_array().cloned().unwrap_or_default() {
                    let active: Vec<String> = work["active"].as_array().map(|items| items.iter().filter_map(|item| item["title"].as_str().map(String::from)).collect()).unwrap_or_default();

                    println!(
                        "    {:<7} {}{}{}",
                        work["module"].as_str().unwrap_or_default(),
                        words(&work["what"]).unwrap_or_default(),
                        if work["pinnedOnly"] == true { " · only what is sent to it by name" } else { "" },
                        if active.is_empty() { String::new() } else { format!(" · now: {}", active.join("; ")) }
                    );
                }
            }
        }
        Command::Tests { command } => match command {
            TestsCommand::List { project } => {
                let jobs = desktop.run(move |core| core.test_jobs(&project)).await?;

                if json {
                    print_json(&jobs);
                } else {
                    for job in jobs.as_array().cloned().unwrap_or_default() {
                        println!(
                            "{:<5} {:<10} {:<24} {:<20} {}",
                            job["id"],
                            job["state"].as_str().unwrap_or_default(),
                            job["ref"].as_str().unwrap_or_default(),
                            job["runnerName"].as_str().or(job["onRunnerName"].as_str()).unwrap_or("-"),
                            job["error"].as_str().unwrap_or_default()
                        );
                    }
                }
            }
            TestsCommand::Run { project, here, on, reference, filter } => {
                let runner = desktop.job_runner();
                let (stop, mut stopped) = tokio::sync::watch::channel(false);
                let on_runner = if here { Some(desktop.core().settings().runner_id) } else { on };

                // A job can be sent here by name only once the server has heard of this runner.
                if here {
                    runner.run_next(&mut stopped).await?;
                }

                let new = hub_desktop_core::model::NewTestJob { project_key: project, reference, filter, on_runner };
                let job = desktop.run(move |core| core.start_test_job(&new)).await?;
                let id = job["id"].as_i64().unwrap_or_default();
                let working = here.then(|| tokio::spawn(async move { runner.run(stopped).await }));
                let mut seen = 0;

                println!("Test job {id} for {} at {}", job["repo"].as_str().unwrap_or_default(), job["ref"].as_str().unwrap_or_default());

                let ended = loop {
                    let detail = desktop.run(move |core| core.test_job(id, seen)).await?;

                    for line in detail["log"].as_array().cloned().unwrap_or_default() {
                        seen = seen.max(line["seq"].as_i64().unwrap_or(seen));
                        println!("{}", line["text"].as_str().unwrap_or_default());
                    }

                    if matches!(detail["job"]["state"].as_str(), Some("succeeded" | "failed" | "cancelled")) {
                        break detail["job"].clone();
                    }

                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_secs(1)) => {}
                        () = runner::asked_to_stop() => {
                            desktop.run(move |core| core.cancel_test_job(id)).await?;
                        }
                    }
                };

                let _ = stop.send(true);

                if let Some(working) = working {
                    let _ = tokio::time::timeout(Duration::from_secs(5), working).await;
                }

                println!("Job {id} {}{}", ended["state"].as_str().unwrap_or_default(), ended["error"].as_str().map(|error| format!(": {error}")).unwrap_or_default());

                return Ok(if ended["state"] == "succeeded" { ExitCode::SUCCESS } else { ExitCode::FAILURE });
            }
        },
        Command::Proposals => {
            let pending = desktop.run(|core| core.pending_proposals()).await?;

            if json {
                print_json(&pending);
            } else {
                for proposal in pending {
                    println!(
                        "#{:<3} thread #{:<3} {:<10} {:<16} {}",
                        proposal.id,
                        proposal.thread_id,
                        proposal.issue_key,
                        proposal.kind.as_str(),
                        proposal.payload
                    );
                }
            }
        }
        Command::Decide { id, approve, reject, note } => {
            if approve == reject {
                return Err(CoreError::not_possible("Say --approve or --reject"));
            }

            let proposal = desktop.run(move |core| core.decide_proposal(id, approve, note.as_deref())).await?;

            println!("Proposal #{} is {}{}", proposal.id, proposal.state, proposal.note.map(|note| format!(": {note}")).unwrap_or_default());
        }
        Command::Push { thread, yes, confirm_findings } => {
            let preview = desktop.run(move |core| core.push_preview(thread)).await?;

            if json {
                print_json(&preview);
            } else {
                println!("Push {} → {}", preview.branch, preview.remote_url);
                println!("{} commits up to {}:", preview.commits.len(), preview.commit);
                preview.commits.iter().for_each(|commit| println!("  {} {}", commit.short, commit.subject));
                preview.files.iter().for_each(|file| println!("  {:<9} {} (+{} −{})", file.status, file.path, file.additions, file.deletions));
                preview.findings.iter().for_each(|found| println!("  ! {}", found.detail));

                if preview.uncommitted > 0 {
                    println!("  ({} uncommitted files stay here)", preview.uncommitted);
                }
            }

            if let Some(blocked) = preview.blocked {
                return Err(CoreError::not_possible(blocked));
            }

            if !yes {
                println!("Nothing was pushed. Repeat with --yes to push exactly this commit.");

                return Ok(ExitCode::SUCCESS);
            }

            let commit = preview.commit.clone();
            let pushed = desktop.run(move |core| core.push(thread, &commit, confirm_findings)).await?;

            println!("Pushed {} at {}.", pushed.branch, pushed.commit);
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let opened = Paths::from_env().and_then(|paths| Desktop::open(OpenOptions::new(paths, Arc::new(OsKeychain))));
    let desktop = match opened {
        Ok(desktop) => desktop,
        Err(error) => {
            eprintln!("hub: {error}");

            return ExitCode::FAILURE;
        }
    };

    let _ = desktop.run(|core| Ok(core.reconnect())).await;

    match run(cli, &desktop).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("hub: {error}");

            ExitCode::FAILURE
        }
    }
}
