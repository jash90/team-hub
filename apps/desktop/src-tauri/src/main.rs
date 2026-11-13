//! The window. Everything it can do is a command of `hub-desktop-core`, forwarded by name;
//! this crate adds only what needs a desktop: the tray, system notifications, and keeping the
//! window's size.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use hub_desktop_core::config::Paths;
use hub_desktop_core::model::AppEvent;
use hub_desktop_core::secrets::OsKeychain;
use hub_desktop_core::{commands, Desktop, OpenOptions};
use serde_json::Value;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let paths = Paths::from_env()?;
            // The core starts tasks when it opens, so it opens inside the runtime.
            let mut options = OpenOptions::new(paths, Arc::new(OsKeychain));

            options.sidecar_script = app.path().resource_dir().ok().map(|resources| resources.join("sidecar/src/main.ts"));

            let desktop = tauri::async_runtime::block_on(async { Desktop::open(options) })?;

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
