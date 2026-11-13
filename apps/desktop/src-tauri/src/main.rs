//! The window. Everything it can do is a command of `hub-desktop-core`, forwarded by name;
//! this crate adds only what needs a desktop: the tray, system notifications, and keeping the
//! window's size.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod environment;
mod runtime;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hub_desktop_core::config::{self, Paths};
use hub_desktop_core::model::AppEvent;
use hub_desktop_core::secrets::OsKeychain;
use hub_desktop_core::{commands, Desktop, OpenOptions};
use serde_json::Value;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;

/// The name of the event the interface listens to; its payload is an `AppEvent`.
const EVENT: &str = "hub";

/// Run from a terminal, the app fetches the agent runtime, prints what npm says, and ends.
const FETCH_FLAG: &str = "--fetch-agent-runtime";
const TITLE: &str = "team-hub";
const NO_THREADS: &str = "team-hub: agent threads cannot start";

/// The agent runtime of an installed app (see `runtime.rs`), and how fetching it is going.
struct Fetch {
    plan: runtime::Plan,
    node: String,
    fetching: AtomicBool,
    /// Why the last try did not end with a runtime.
    failure: Mutex<Option<String>>,
}

impl Fetch {
    /// Why a thread cannot start right now, when it cannot. Looks at the disk, so a runtime
    /// that was removed while the app runs is noticed.
    fn in_the_way(&self) -> Option<String> {
        if self.plan.is_ready() {
            return None;
        }

        let last = self.failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();

        Some(match last.filter(|_| !self.fetching.load(Ordering::SeqCst)) {
            Some(reason) => format!(
                "The agent runtime is not on this computer. The last try ended with: {reason} team-hub is trying again now and will say how it went."
            ),
            None => format!("team-hub is downloading {}. Start the thread again when it says the runtime is ready.", runtime::WHAT),
        })
    }
}

struct Shell {
    desktop: Desktop,
    /// None when the helper does not come from a fetched runtime: `TEAM_HUB_SIDECAR`, or a
    /// run from the source tree with `sidecar/node_modules` in it.
    fetch: Option<Arc<Fetch>>,
    /// The runner's rounds start when the server was tried and the runtime is settled, so
    /// that no delegated run is claimed only to fail for want of a runtime being downloaded.
    connected: AtomicBool,
    settled: AtomicBool,
    background: AtomicBool,
}

/// The one command: a command of the core by name. See `hub_desktop_core::commands`.
#[tauri::command]
async fn hub(app: AppHandle, state: State<'_, Shell>, command: String, args: Option<Value>) -> Result<Value, String> {
    // The two commands that start the helper look for the runtime first. Without it they say
    // so, instead of a thread failing, and the download is started if it is not under way.
    if matches!(command.as_str(), "start_thread" | "say_to_thread") {
        if let Some(reason) = state.fetch.as_deref().and_then(Fetch::in_the_way) {
            fetch_runtime(&app, true);

            return Err(reason);
        }
    }

    commands::invoke(&state.desktop, &command, args.unwrap_or(Value::Null)).await
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Passes the core's events to the window, and raises a system notification for the ones
/// that need the person.
fn forward_events(app: AppHandle, desktop: &Desktop) {
    let mut events = desktop.subscribe();

    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let AppEvent::Notify { title, body, .. } = &event {
                        let _ = app.notification().builder().title(title).body(body).show();
                    }

                    let _ = app.emit(EVENT, &event);
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn tray(app: &AppHandle, fetches: bool) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open team-hub", true, None::<&str>)?;
    let runtime = MenuItem::with_id(app, "runtime", "Download the agent runtime", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit (stops working threads)", true, None::<&str>)?;
    let menu = if fetches { Menu::with_items(app, &[&open, &runtime, &quit])? } else { Menu::with_items(app, &[&open, &quit])? };
    let mut builder = TrayIconBuilder::with_id("main").tooltip(TITLE).menu(&menu).on_menu_event(|app, event| match event.id.as_ref() {
        "open" => show_window(app),
        "runtime" => fetch_runtime(app, true),
        "quit" => {
            let app = app.clone();

            // No agent keeps working after the app is gone.
            tauri::async_runtime::spawn(async move {
                app.state::<Shell>().desktop.shutdown().await;
                app.exit(0);
            });
        }
        _ => {}
    });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;

    Ok(())
}

/// Where the agent runtime of this app is or will be: named after the helper the installer
/// carries (`bundle.resources` in `tauri.conf.json`), under the app's data folder. None when
/// the installed files are not there.
fn planned(resources: Option<PathBuf>, paths: &Paths) -> Option<runtime::Plan> {
    // On Windows the resource folder comes in the `\\?\` form, which Node does not take.
    let bundled = environment::plain_path(&resources?.join("sidecar"));
    let data = paths.state.parent()?;

    runtime::Plan::new(&bundled, data).ok().filter(|plan| plan.launcher().is_file())
}

/// Whether `TEAM_HUB_SIDECAR` replaces the helper, read the way the core reads it.
fn helper_is_replaced() -> bool {
    std::env::var(config::SIDECAR_VARIABLE).is_ok_and(|line| !line.trim().is_empty())
}

/// Whether this program was built from a source tree that is here and has the helper's
/// dependencies installed: a development run, which then needs no download. In an installed
/// app the folder named is the build machine's.
fn source_tree_has_the_runtime() -> bool {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../sidecar/node_modules/@anthropic-ai/claude-agent-sdk/package.json").is_file()
}

/// The program a person would start from a terminal.
fn this_program() -> String {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .map_or_else(|| "team-hub-desktop".to_string(), |path| path.display().to_string())
}

/// Shows how the download is going where it can always be seen: the window's title and the
/// tray's tooltip. None puts both back.
fn show_progress(app: &AppHandle, line: Option<&str>) {
    let title = match line {
        Some(line) => {
            format!("{TITLE} · downloading the agent runtime… {}", line.trim_start_matches("npm http ").chars().take(90).collect::<String>())
        }
        None => TITLE.to_string(),
    };

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&title);
    }

    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&title));
    }
}

/// Starts what runs for as long as the app does, once, when both the server was tried and
/// the agent runtime is settled one way or the other.
fn start_background_when_due(app: &AppHandle) {
    let shell = app.state::<Shell>();

    if shell.connected.load(Ordering::SeqCst) && shell.settled.load(Ordering::SeqCst) && !shell.background.swap(true, Ordering::SeqCst) {
        let desktop = shell.desktop.clone();

        tauri::async_runtime::spawn(async move { desktop.start_background() });
    }
}

/// Makes sure the agent runtime is on this computer, on a thread of its own, and tells the
/// person what happens: a dialog when a download starts, its progress in the window's title,
/// a notification when it is there, and the reason with a Retry button when it failed. With
/// the runtime already there it only puts this app's helper beside it, without the network.
fn fetch_runtime(app: &AppHandle, announce: bool) {
    let Some(fetch) = app.state::<Shell>().fetch.clone() else { return };

    if fetch.fetching.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app.clone();

    std::thread::spawn(move || {
        let npm = runtime::Npm::for_node(&fetch.node);
        let was_there = fetch.plan.is_complete();
        let outcome = if let Some(problem) = environment::node_problem(&fetch.node) {
            Err(problem)
        } else if was_there {
            fetch
                .plan
                .place_helper()
                .map_err(|error| format!("The agent's helper could not be copied beside the runtime in {} ({error}).", fetch.plan.folder().display()))
        } else {
            if announce {
                app.dialog()
                    .message(format!(
                        "team-hub is downloading {}.\n\nIt is fetched with your npm, from the registry npm is set up with, and kept in {}. You can use the app meanwhile; agent threads can start when the window's title stops saying \"downloading\" and a notification says the runtime is ready.",
                        runtime::WHAT,
                        fetch.plan.root().display()
                    ))
                    .title("team-hub: downloading the agent runtime")
                    .kind(MessageDialogKind::Info)
                    .show(|_| {});
            }

            show_progress(&app, Some(""));

            let mut shown = Instant::now();
            let outcome = fetch.plan.install(&npm, &mut |line| {
                if shown.elapsed() >= Duration::from_millis(200) {
                    shown = Instant::now();
                    show_progress(&app, Some(line));
                }
            });

            show_progress(&app, None);

            outcome.map_err(|failure| {
                format!(
                    "The agent runtime could not be downloaded: {}\n\n{}\n\nteam-hub ran `{}` in a folder of its own under {}; everything npm printed is in {}.\nTo try from a terminal:\n    \"{}\" {FETCH_FLAG}\n\nUntil then agent threads cannot start; the rest of the app works.",
                    failure.reason,
                    failure.detail,
                    fetch.plan.command_line(&npm),
                    fetch.plan.root().display(),
                    fetch.plan.log().display(),
                    this_program()
                )
            })
        };

        fetch.fetching.store(false, Ordering::SeqCst);

        match outcome {
            Ok(()) => {
                *fetch.failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

                if !was_there {
                    let _ = app.notification().builder().title(TITLE).body("The agent runtime is ready. Agent threads can start.").show();
                } else if announce && app.state::<Shell>().background.load(Ordering::SeqCst) {
                    // Asked for from the tray menu with nothing to do.
                    app.dialog()
                        .message(format!("The agent runtime is on this computer, in {}.", fetch.plan.folder().display()))
                        .title(TITLE)
                        .show(|_| {});
                }
            }
            Err(problem) => {
                *fetch.failure.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(problem.lines().next().unwrap_or_default().to_string());

                let again = app.clone();

                app.dialog()
                    .message(problem)
                    .title(NO_THREADS)
                    .kind(MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::OkCancelCustom("Retry".to_string(), "Close".to_string()))
                    .show(move |retry| {
                        if retry {
                            fetch_runtime(&again, false);
                        }
                    });
            }
        }

        app.state::<Shell>().settled.store(true, Ordering::SeqCst);
        start_background_when_due(&app);
    });
}

/// `team-hub-desktop --fetch-agent-runtime`: the same download without a window, for a
/// terminal and for the release's check of what it packaged. The exit code says how it went.
fn fetch_from_a_terminal(package: &tauri::PackageInfo) -> i32 {
    let plan = Paths::from_env().ok().and_then(|paths| {
        let node = config::load_settings(&paths).map(|settings| settings.node_command).unwrap_or_else(|_| "node".to_string());
        let resources = tauri::utils::platform::resource_dir(package, &tauri::Env::default()).ok();

        planned(resources, &paths).map(|plan| (plan, node))
    });
    let Some((plan, node)) = plan else {
        eprintln!("The helper's files are not among the app's installed files, or the app has no data folder.");

        return 2;
    };

    if let Some(problem) = environment::node_problem(&node) {
        eprintln!("{problem}");

        return 1;
    }

    if plan.is_complete() {
        return match plan.place_helper() {
            Ok(()) => {
                println!("The agent runtime is on this computer already: {}", plan.folder().display());

                0
            }
            Err(error) => {
                eprintln!("The agent's helper could not be copied beside the runtime in {} ({error}).", plan.folder().display());

                1
            }
        };
    }

    let npm = runtime::Npm::for_node(&node);

    println!("Downloading {} into {}", runtime::WHAT, plan.root().display());
    println!("$ {}", plan.command_line(&npm));

    match plan.install(&npm, &mut |line| println!("{line}")) {
        Ok(()) => {
            println!("The agent runtime is ready: {}", plan.folder().display());

            0
        }
        Err(failure) => {
            eprintln!("The agent runtime could not be downloaded: {}\nEverything npm printed is in {}.", failure.reason, plan.log().display());

            1
        }
    }
}

/// Says so in a dialog when the Node of the settings is missing or cannot run the helper,
/// instead of leaving it to the first thread to fail. Asked on a thread of its own, so the
/// window does not wait for Node to answer.
fn warn_about_node(app: AppHandle, node: String) {
    std::thread::spawn(move || {
        if let Some(problem) = environment::node_problem(&node) {
            app.dialog().message(problem).title(NO_THREADS).kind(MessageDialogKind::Warning).show(|_| {});
        }
    });
}

fn main() {
    // Before anything else runs: it changes this process's environment.
    environment::adopt_shell_path();

    let context = tauri::generate_context!();

    if std::env::args().any(|argument| argument == FETCH_FLAG) {
        std::process::exit(fetch_from_a_terminal(context.package_info()));
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let paths = Paths::from_env()?;

            paths.create()?;

            let node = config::load_settings(&paths)?.node_command;
            let replaced = helper_is_replaced();
            let plan = if replaced || source_tree_has_the_runtime() { None } else { planned(app.path().resource_dir().ok(), &paths) };
            // The core starts tasks when it opens, so it opens inside the runtime.
            let mut options = OpenOptions::new(paths, Arc::new(OsKeychain));

            // An installed app runs the launcher it carries, which runs the helper from the
            // runtime fetched into the data folder, or tells the thread why it cannot. Both
            // paths are known now, whether or not the runtime is there yet. Nothing is
            // written into the app's own folder: on macOS that is the signed bundle,
            // elsewhere it is often not the person's to write to. Without a plan the core
            // decides: `TEAM_HUB_SIDECAR`, or the source tree it was built from.
            if let Some(plan) = &plan {
                options.helper =
                    Some((node.clone(), vec![plan.launcher().to_string_lossy().into_owned(), plan.script().to_string_lossy().into_owned()]));
            }

            let desktop = tauri::async_runtime::block_on(async { Desktop::open(options) })?;
            let fetch = plan.map(|plan| Arc::new(Fetch { plan, node: node.clone(), fetching: AtomicBool::new(false), failure: Mutex::new(None) }));
            let fetches = fetch.is_some();

            forward_events(app.handle().clone(), &desktop);
            app.manage(Shell {
                desktop: desktop.clone(),
                fetch,
                connected: AtomicBool::new(false),
                settled: AtomicBool::new(!fetches),
                background: AtomicBool::new(false),
            });
            tray(app.handle(), fetches)?;

            let connecting = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                let _ = desktop.run(|core| Ok(core.reconnect())).await;

                connecting.state::<Shell>().connected.store(true, Ordering::SeqCst);
                start_background_when_due(&connecting);
            });

            if fetches {
                fetch_runtime(app.handle(), true);
            } else if !replaced {
                warn_about_node(app.handle().clone(), node);
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window leaves the app in the tray, where it keeps acting as a runner.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();

                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![hub])
        .run(context)
        .expect("the team-hub desktop app could not start");
}
