//! The window and the two webviews in it.
//!
//! `main` is the app's own interface, loaded from the app's own files: threads, decisions,
//! repositories, settings. It is the only one that can call the core.
//!
//! `web` is the server's web interface, loaded from the server the app is connected to: the
//! tracker itself, the same pages a browser shows. It lies over the part of the window the
//! local interface leaves for it (`web_show` says where), and it cannot call anything here:
//! Tauri refuses commands from a page that is not the app's own, no capability names a
//! remote address, and every command below also asks which webview is calling. What comes
//! from the server can change with the server; what can run an agent or push from this
//! computer stays in files that came with the app.

use serde_json::Value;
use tauri::webview::{Cookie, NewWindowResponse};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, Url, Webview, WebviewBuilder, WebviewUrl, Window};
use tauri_plugin_opener::OpenerExt;

use hub_desktop_core::api::SESSION_COOKIE;
use hub_desktop_core::commands;

use crate::Shell;

/// The label of the window and of the local interface in it.
pub const LOCAL: &str = "main";
/// The label of the server's web interface.
const WEB: &str = "web";

/// Where the web interface starts: the sign-in form, which a signed-in person is sent on
/// from to their start page. `/` would be the page about team-hub for somebody signed out.
const START: &str = "/login";

/// The server whose web interface is in the window, when one is.
#[derive(Default)]
pub struct Stage {
    server: tokio::sync::Mutex<Option<Url>>,
}

/// Refuses a command that does not come from the app's own interface.
pub fn local_only(webview: &Webview) -> Result<(), String> {
    if webview.label() == LOCAL {
        Ok(())
    } else {
        Err("Only the app's own interface can do this.".to_string())
    }
}

/// The window, with the local interface filling it.
pub fn open_window(app: &AppHandle, title: &str) -> tauri::Result<Window> {
    let window = tauri::window::WindowBuilder::new(app, LOCAL).title(title).inner_size(1440.0, 900.0).min_inner_size(900.0, 560.0).build()?;
    let local = window.add_child(
        WebviewBuilder::new(LOCAL, WebviewUrl::App("index.html".into())).auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        window.inner_size()?,
    )?;

    layout::wrap(&local)?;

    Ok(window)
}

fn same_origin(one: &Url, other: &Url) -> bool {
    one.scheme() == other.scheme() && one.host_str() == other.host_str() && one.port_or_known_default() == other.port_or_known_default()
}

/// The name of the events the window sends to the local interface.
const STAGE_EVENT: &str = "stage";

/// The run whose thread a link of the server's asks the app to show: `/desktop/thread/12`
/// on the server itself, and nothing else. Such a link is never followed.
fn thread_link(url: &Url, server: &Url) -> Option<i64> {
    if !same_origin(url, server) || url.query().is_some() || url.fragment().is_some() {
        return None;
    }

    let run = url.path().strip_prefix("/desktop/thread/")?;

    if run.is_empty() || run.len() > 15 || !run.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    run.parse().ok()
}

/// A link that leads away from the server opens in the person's browser.
fn open_outside(app: &AppHandle, url: &Url) {
    if matches!(url.scheme(), "http" | "https") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}

/// What the web interface is told about where it runs: nothing it could not be told by
/// anyone, and enough to offer "this computer" where work is sent to a runner.
fn introduction(server: &Url, runner: &str, version: &str) -> String {
    let about = serde_json::json!({ "runnerId": runner, "version": version });
    let origin = Value::String(server.origin().ascii_serialization());

    format!("if (window.location.origin === {origin}) {{ Object.defineProperty(window, '__TEAM_HUB_DESKTOP__', {{ value: Object.freeze({about}) }}); }}")
}

/// The web interface of the server the app is connected to, made when it is not there yet
/// and made anew when the server is another one.
///
/// The second value says that it was made just now: it has no page yet, only an address
/// it is on its way to.
async fn web(app: &AppHandle, shell: &Shell, stage: &Stage) -> Result<(Webview, bool), String> {
    let status = commands::invoke(&shell.desktop, "status", Value::Null).await?;
    let server = status["serverUrl"].as_str().and_then(|address| Url::parse(address).ok()).ok_or("The app is not connected to a team-hub server.")?;
    let runner = status["runner"]["id"].as_str().unwrap_or_default().to_string();
    let mut current = stage.server.lock().await;

    if let Some(existing) = app.get_webview(WEB) {
        if current.as_ref().is_some_and(|shown| same_origin(shown, &server)) {
            return Ok((existing, false));
        }

        layout::release(app);
        existing.close().map_err(|error| error.to_string())?;
    }

    let start = server.join(START).map_err(|error| error.to_string())?;
    let stay = server.clone();
    let leave = app.clone();
    let opened = server.clone();
    let opener = app.clone();
    let builder = WebviewBuilder::new(WEB, WebviewUrl::External(start))
        .initialization_script(introduction(&server, &runner, &app.package_info().version.to_string()))
        // The view shows the server and nothing else.
        .on_navigation(move |url| {
            // The one thing a page of the server can ask of the app: to show the thread of a
            // run. It changes what the window shows and nothing else; the page stays.
            if let Some(run) = thread_link(url, &stay) {
                let _ = leave.emit_to(LOCAL, STAGE_EVENT, serde_json::json!({ "openRun": run }));

                return false;
            }

            if same_origin(url, &stay) || url.scheme() == "about" {
                return true;
            }

            open_outside(&leave, url);

            false
        })
        // There is one window. A link that asks for a new one opens here when it is the
        // server's (an attachment), and in the browser when it is not.
        .on_new_window(move |url, _| {
            if same_origin(&url, &opened) {
                if let Some(view) = opener.get_webview(WEB) {
                    let _ = view.navigate(url);
                }
            } else {
                open_outside(&opener, &url);
            }

            NewWindowResponse::Deny
        })
        .on_download(|_, _| true);
    let window = app.get_window(LOCAL).ok_or("The window is gone.")?;
    let view = window.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(1.0, 1.0)).map_err(|error| error.to_string())?;

    layout::adopt(&view).map_err(|error| error.to_string())?;
    *current = Some(server);

    Ok((view, true))
}

/// Shows the server's web interface over this part of the local interface, in the local
/// interface's own units.
// The four numbers are the command's arguments, as the interface sends them.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn web_show(
    app: AppHandle,
    webview: Webview,
    shell: State<'_, Shell>,
    stage: State<'_, Stage>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    local_only(&webview)?;

    let (view, _) = web(&app, &shell, &stage).await?;

    layout::place(&app, &view, Some((x.max(0.0), y.max(0.0), width.max(1.0), height.max(1.0)))).map_err(|error| error.to_string())
}

/// Takes the web interface off the window; it keeps its page.
#[tauri::command]
pub async fn web_hide(app: AppHandle, webview: Webview) -> Result<(), String> {
    local_only(&webview)?;

    match app.get_webview(WEB) {
        Some(view) => layout::place(&app, &view, None).map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// Opens a page of the server in the web interface: `/issue/WEB-12`.
#[tauri::command]
pub async fn web_open(app: AppHandle, webview: Webview, shell: State<'_, Shell>, stage: State<'_, Stage>, path: String) -> Result<(), String> {
    local_only(&webview)?;

    if !path.starts_with('/') || path.starts_with("//") {
        return Err("A page of the server starts with one slash.".to_string());
    }

    let (view, fresh) = web(&app, &shell, &stage).await?;
    let server = stage.server.lock().await.clone().ok_or("The app is not connected to a team-hub server.")?;
    let target = server.join(&path).map_err(|error| error.to_string())?;

    if !same_origin(&target, &server) {
        return Err("That is not a page of the server.".to_string());
    }

    // The interface is one page that follows the address: on the server already, changing
    // the address is enough and keeps what the page has loaded. A view made just now has
    // no page to tell yet, so it is sent to the address itself.
    if !fresh && view.url().is_ok_and(|now| same_origin(&now, &server)) {
        let to = Value::String(format!("{}{}", target.path(), target.query().map(|query| format!("?{query}")).unwrap_or_default()));

        view.eval(format!("window.history.pushState(null, '', {to}); window.dispatchEvent(new PopStateEvent('popstate'));"))
    } else {
        view.navigate(target)
    }
    .map_err(|error| error.to_string())
}

/// The page of the server the web interface shows now, as a path, when it shows one.
#[tauri::command]
pub async fn web_location(app: AppHandle, webview: Webview, stage: State<'_, Stage>) -> Result<Option<String>, String> {
    local_only(&webview)?;

    let server = stage.server.lock().await.clone();
    let now = app.get_webview(WEB).and_then(|view| view.url().ok());

    Ok(match (server, now) {
        (Some(server), Some(now)) if same_origin(&now, &server) => {
            Some(format!("{}{}", now.path(), now.query().map(|query| format!("?{query}")).unwrap_or_default()))
        }
        _ => None,
    })
}

/// Back, forward and reload, for a page without a browser around it.
#[tauri::command]
pub async fn web_go(app: AppHandle, webview: Webview, step: String) -> Result<(), String> {
    local_only(&webview)?;

    let script = match step.as_str() {
        "back" => "window.history.back();",
        "forward" => "window.history.forward();",
        "reload" => "window.location.reload();",
        _ => return Err("Back, forward or reload.".to_string()),
    };

    match app.get_webview(WEB) {
        Some(view) => view.eval(script).map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// The language and the theme are chosen once, in the app's own bar, and the web interface
/// follows: it keeps both under these names in its page storage, and reads them when a page
/// loads.
#[tauri::command]
pub async fn web_set(app: AppHandle, webview: Webview, stage: State<'_, Stage>, what: String, value: String) -> Result<(), String> {
    local_only(&webview)?;

    let key = match (what.as_str(), value.as_str()) {
        ("language", "en" | "pl") => "team-hub:language",
        ("theme", "light" | "dark") => "team-hub:theme",
        _ => return Err("A language (en, pl) or a theme (light, dark).".to_string()),
    };
    let server = stage.server.lock().await.clone();
    let view = app.get_webview(WEB);

    match (server, view) {
        // Only on a page of the server: the storage is that of the page it runs in.
        (Some(server), Some(view)) if view.url().is_ok_and(|now| same_origin(&now, &server)) => view
            .eval(format!(
                "try {{ if (window.localStorage.getItem('{key}') !== '{value}') {{ window.localStorage.setItem('{key}', '{value}'); window.location.reload(); }} }} catch (error) {{}}"
            ))
            .map_err(|error| error.to_string()),
        _ => Ok(()),
    }
}

/// How long the server keeps a session, which is how long the tracker stays signed in.
const SESSION_SECONDS: u32 = 30 * 24 * 3600;

/// The cookie that makes the tracker's pages those of a signed-in person, as the server
/// itself would set it.
fn session_cookie(server: &Url, value: &str) -> Option<Cookie<'static>> {
    // What the server makes is hex; anything else is not a session of its making.
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }

    let secure = if server.scheme() == "https" { "; Secure" } else { "" };
    let text = format!("{SESSION_COOKIE}={value}; Domain={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_SECONDS}{secure}", server.host_str()?);

    Cookie::parse(text).ok()
}

/// Signs in with the e-mail address and password of an account, once for both halves of
/// the window: the core makes itself a token in the session the password opens, and the
/// same session goes to the tracker, so that its pages are signed in too. The local
/// interface gets the connection's status back and nothing else: not the session.
#[tauri::command]
pub async fn sign_in(
    app: AppHandle,
    webview: Webview,
    shell: State<'_, Shell>,
    stage: State<'_, Stage>,
    url: String,
    email: String,
    password: String,
) -> Result<Value, String> {
    local_only(&webview)?;

    let (status, session) = shell.desktop.run(move |core| core.sign_in(&url, &email, &password)).await.map_err(|error| error.to_string())?;
    // Whatever goes wrong from here on, the app is connected: the tracker then asks for
    // the password itself, as it did before there was this.
    let handed = async {
        let (view, _) = web(&app, &shell, &stage).await?;
        let server = stage.server.lock().await.clone().ok_or("no server")?;
        let cookie = session_cookie(&server, session.cookie.expose()).ok_or("no cookie")?;

        view.set_cookie(cookie).map_err(|error| error.to_string())?;
        view.navigate(server.join("/").map_err(|error| error.to_string())?).map_err(|error| error.to_string())
    };

    if handed.await.is_err() {
        let _ = shell
            .desktop
            .run(move |_| {
                session.end();

                Ok(())
            })
            .await;
    }

    serde_json::to_value(status).map_err(|error| error.to_string())
}

/// Closes the web interface and forgets what its pages stored: the sign-in above all. For
/// when the app is disconnected from its server.
#[tauri::command]
pub async fn web_forget(app: AppHandle, webview: Webview, stage: State<'_, Stage>) -> Result<(), String> {
    local_only(&webview)?;

    let mut current = stage.server.lock().await;

    if let Some(view) = app.get_webview(WEB) {
        let _ = view.clear_all_browsing_data();

        layout::release(&app);
        view.close().map_err(|error| error.to_string())?;
    }

    *current = None;

    Ok(())
}

/// Where the two webviews are in the window. Everywhere but Linux Tauri places a webview
/// where it is told.
#[cfg(not(target_os = "linux"))]
mod layout {
    use tauri::{AppHandle, LogicalPosition, LogicalSize, Rect, Webview};

    pub fn wrap(_local: &Webview) -> tauri::Result<()> {
        Ok(())
    }

    pub fn adopt(view: &Webview) -> tauri::Result<()> {
        view.hide()
    }

    pub fn release(_app: &AppHandle) {}

    pub fn place(_app: &AppHandle, view: &Webview, at: Option<(f64, f64, f64, f64)>) -> tauri::Result<()> {
        match at {
            Some((x, y, width, height)) => {
                view.set_bounds(Rect { position: LogicalPosition::new(x, y).into(), size: LogicalSize::new(width, height).into() })?;
                view.show()
            }
            None => view.hide(),
        }
    }
}

/// On Linux Tauri puts a window's webviews into one vertical box, one under another, and a
/// position given to a webview does nothing. So the local interface is put into a GTK
/// overlay, and the web interface is laid over it at the place asked for.
#[cfg(target_os = "linux")]
mod layout {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gtk::prelude::*;
    use tauri::{AppHandle, Webview};

    struct Laid {
        overlay: gtk::Overlay,
        web: Option<gtk::Widget>,
        at: Rc<Cell<(i32, i32, i32, i32)>>,
    }

    thread_local! {
        // GTK's widgets belong to the main thread, and so does this.
        static LAID: RefCell<Option<Laid>> = const { RefCell::new(None) };
    }

    pub fn wrap(local: &Webview) -> tauri::Result<()> {
        local.with_webview(|platform| {
            let widget: gtk::Widget = platform.inner().upcast();
            let Some(parent) = widget.parent().and_then(|parent| parent.downcast::<gtk::Box>().ok()) else { return };
            let overlay = gtk::Overlay::new();
            let at = Rc::new(Cell::new((0, 0, 1, 1)));
            let asked = at.clone();

            parent.remove(&widget);
            overlay.add(&widget);
            parent.pack_start(&overlay, true, true, 0);
            overlay.connect_get_child_position(move |_, _| {
                let (x, y, width, height) = asked.get();

                Some(gtk::gdk::Rectangle::new(x, y, width, height))
            });
            overlay.show_all();

            LAID.with(|laid| *laid.borrow_mut() = Some(Laid { overlay, web: None, at }));
        })
    }

    pub fn adopt(view: &Webview) -> tauri::Result<()> {
        view.with_webview(|platform| {
            let widget: gtk::Widget = platform.inner().upcast();

            LAID.with(|laid| {
                let mut laid = laid.borrow_mut();
                let Some(laid) = laid.as_mut() else { return };

                if let Some(parent) = widget.parent().and_then(|parent| parent.downcast::<gtk::Container>().ok()) {
                    parent.remove(&widget);
                }

                laid.overlay.add_overlay(&widget);
                widget.hide();
                laid.web = Some(widget);
            });
        })
    }

    /// Before the web interface is closed: the overlay lets go of it.
    pub fn release(app: &AppHandle) {
        let _ = app.run_on_main_thread(|| {
            LAID.with(|laid| {
                if let Some(laid) = laid.borrow_mut().as_mut() {
                    if let Some(widget) = laid.web.take() {
                        laid.overlay.remove(&widget);
                    }
                }
            });
        });
    }

    pub fn place(app: &AppHandle, _view: &Webview, at: Option<(f64, f64, f64, f64)>) -> tauri::Result<()> {
        app.run_on_main_thread(move || {
            LAID.with(|laid| {
                let laid = laid.borrow();
                let Some(laid) = laid.as_ref() else { return };
                let Some(widget) = &laid.web else { return };

                match at {
                    Some((x, y, width, height)) => {
                        laid.at.set((x.round() as i32, y.round() as i32, width.round() as i32, height.round() as i32));
                        widget.show();
                        laid.overlay.queue_resize();
                    }
                    None => widget.hide(),
                }
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn a_page_is_the_servers_by_scheme_host_and_port() {
        let server = url("https://hub.example.com");

        assert!(same_origin(&url("https://hub.example.com/issue/WEB-1?x=1"), &server));
        assert!(same_origin(&url("https://hub.example.com:443/"), &server));
        assert!(!same_origin(&url("http://hub.example.com/"), &server));
        assert!(!same_origin(&url("https://hub.example.com.evil.test/"), &server));
        assert!(!same_origin(&url("https://hub.example.com:8443/"), &server));
        assert!(!same_origin(&url("https://evil.test/https://hub.example.com/"), &server));
    }

    #[test]
    fn the_session_goes_to_the_tracker_as_the_servers_own_cookie() {
        let cookie = session_cookie(&url("https://hub.example.com/"), "0a1b2c").unwrap();

        assert_eq!((cookie.name(), cookie.value(), cookie.domain(), cookie.path()), ("th_session", "0a1b2c", Some("hub.example.com"), Some("/")));
        assert_eq!((cookie.http_only(), cookie.secure()), (Some(true), Some(true)));
        assert!(cookie.max_age().is_some(), "it outlives the app being closed");
        assert_eq!(session_cookie(&url("http://127.0.0.1:8080/"), "0a1b2c").unwrap().secure(), None);
        // A value that could add attributes of its own is not a session.
        assert!(session_cookie(&url("https://hub.example.com/"), "x; Domain=evil.test").is_none());
        assert!(session_cookie(&url("https://hub.example.com/"), "").is_none());
    }

    #[test]
    fn only_the_servers_own_link_to_a_run_asks_for_its_thread() {
        let server = url("https://hub.example.com");

        assert_eq!(thread_link(&url("https://hub.example.com/desktop/thread/12"), &server), Some(12));
        // Another site, another scheme or another port is not the server.
        assert_eq!(thread_link(&url("https://evil.test/desktop/thread/12"), &server), None);
        assert_eq!(thread_link(&url("http://hub.example.com/desktop/thread/12"), &server), None);
        assert_eq!(thread_link(&url("https://hub.example.com:8443/desktop/thread/12"), &server), None);
        // Digits and nothing but digits.
        for path in ["/desktop/thread/", "/desktop/thread/12/", "/desktop/thread/12a", "/desktop/thread/-1", "/desktop/thread/1.5", "/desktop/thread/12/x"] {
            assert_eq!(thread_link(&url(&format!("https://hub.example.com{path}")), &server), None, "{path}");
        }
        assert_eq!(thread_link(&url("https://hub.example.com/desktop/thread/%31%32"), &server), None);
        assert_eq!(thread_link(&url("https://hub.example.com/desktop/thread/12?x=1"), &server), None);
        assert_eq!(thread_link(&url("https://hub.example.com/desktop/thread/12#x"), &server), None);
        assert_eq!(thread_link(&url("https://hub.example.com/desktop/thread/99999999999999999999"), &server), None);
        assert_eq!(thread_link(&url("https://hub.example.com/x/desktop/thread/12"), &server), None);
    }

    #[test]
    fn the_introduction_is_for_the_servers_pages_and_says_which_runner_the_app_is() {
        let script = introduction(&url("http://127.0.0.1:8080/"), "desktop-1\"; alert(1); \"", "0.1.1");

        assert!(script.starts_with("if (window.location.origin === \"http://127.0.0.1:8080\")"));
        // The runner's id is data: it cannot end the text it is written in.
        assert!(script.contains(r#""runnerId":"desktop-1\"; alert(1); \"""#));
        assert!(script.contains(r#""version":"0.1.1""#));
    }
}
