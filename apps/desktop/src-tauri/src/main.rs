//! The window. Everything it can do is a command of `hub-desktop-core`, forwarded by name;
//! this crate adds only what needs a desktop: the tray, system notifications, and keeping the
//! window's size.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod environment;

use std::path::PathBuf;
use std::sync::Arc;

use hub_desktop_core::config::{self, Paths};
use hub_desktop_core::model::AppEvent;
use hub_desktop_core::secrets::OsKeychain;
use hub_desktop_core::{commands, Desktop, OpenOptions};
use serde_json::Value;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;

/// The name of the event the interface listens to; its payload is an `AppEvent`.
const EVENT: &str = "hub";

struct Shell {
    desktop: Desktop,
}

/// The one command: a command of the core by name. See `hub_desktop_core::commands`.
#[tauri::command]
async fn hub(state: State<'_, Shell>, command: String, args: Option<Value>) -> Result<Value, String> {
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

fn tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open team-hub", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit (stops working threads)", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main").tooltip("team-hub").menu(&menu).on_menu_event(|app, event| match event.id.as_ref() {
        "open" => show_window(app),
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

/// The helper's script among the files installed with the app (see `bundle.resources` in
/// `tauri.conf.json`), when it is there.
fn installed_script(app: &tauri::App) -> Option<PathBuf> {
    let resources = app.path().resource_dir().ok()?;
    // One folder at a time: on Windows the resource folder comes in the `\\?\` form, in which
    // a `/` would be part of a name.
    let script = environment::plain_path(&resources.join("sidecar").join("src").join("main.ts"));

    script.is_file().then_some(script)
}

/// Whether `TEAM_HUB_SIDECAR` replaces the helper, read the way the core reads it.
fn helper_is_replaced() -> bool {
    std::env::var(config::SIDECAR_VARIABLE).is_ok_and(|line| !line.trim().is_empty())
}

/// Says so in a dialog when the Node of the settings is missing or cannot run the helper,
/// instead of leaving it to the first thread to fail. Asked on a thread of its own, so the
/// window does not wait for Node to answer.
fn warn_about_node(app: AppHandle, node: String) {
    std::thread::spawn(move || {
        if let Some(problem) = environment::node_problem(&node) {
            app.dialog().message(problem).title("team-hub: agent threads cannot start").kind(MessageDialogKind::Warning).show(|_| {});
        }
    });
}

fn main() {
    // Before anything else runs: it changes this process's environment.
    environment::adopt_shell_path();

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let paths = Paths::from_env()?;

            paths.create()?;

            let node = config::load_settings(&paths)?.node_command;
            let script = installed_script(app);
            let replaced = helper_is_replaced();
            // The core starts tasks when it opens, so it opens inside the runtime.
            let mut options = OpenOptions::new(paths, Arc::new(OsKeychain));

            // The installed script is run where it is. Left to itself the core would first
            // copy the scripts into a folder beside them, and an installed app does not write
            // into its own folder: on macOS that is the signed bundle, elsewhere it is often
            // not the person's to write to.
            if let (Some(script), false) = (&script, replaced) {
                options.helper = Some((node.clone(), vec![script.to_string_lossy().into_owned()]));
            }

            options.sidecar_script = script;

            let desktop = tauri::async_runtime::block_on(async { Desktop::open(options) })?;

            if !replaced {
                warn_about_node(app.handle().clone(), node);
            }

            forward_events(app.handle().clone(), &desktop);
            tray(app.handle())?;

            let starting = desktop.clone();

            tauri::async_runtime::spawn(async move {
                let _ = starting.run(|core| Ok(core.reconnect())).await;

                starting.start_background();
            });
            app.manage(Shell { desktop });

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
        .run(tauri::generate_context!())
        .expect("the team-hub desktop app could not start");
}
