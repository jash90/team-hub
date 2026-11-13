//! The schedule through the API, with a module made for the test standing in for the ones
//! that own task kinds: it records how it was asked to start, and answers as told.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use common::{test_app, Client};
use hub_app::services::schedule::{self, Outcome, Ticker};
use hub_app::{Actor, ApiError, ApiResult, App, AuthUser, Event, Module, ModuleMeta};
use hub_core::schedule::{ScheduledTask, TaskField, TaskFieldType, TaskKind, TaskOption, TaskSlot, TaskStarted};
use hub_server::build_router;
use serde_json::{json, Value};
use tokio::sync::broadcast::Receiver;

// ---------- a module with task kinds ----------

/// One call of `start_task`: which task, as whom, through which modules.
#[derive(Debug, Clone, PartialEq)]
struct Started {
    task: String,
    actor: i64,
    through: Vec<&'static str>,
}

#[derive(Default)]
struct Probe {
    started: Arc<Mutex<Vec<Started>>>,
}

impl Probe {
    fn names(&self) -> Vec<String> {
        self.started.lock().unwrap().iter().map(|call| call.task.clone()).collect()
    }

    fn forget(&self) {
        self.started.lock().unwrap().clear();
    }
}

fn field(name: &str, label: &str, kind: TaskFieldType) -> TaskField {
    TaskField { name: name.to_string(), label: label.to_string(), kind, ..TaskField::default() }
}

/// The module a test hands to the application; the test keeps the other end of the record.
struct ProbeModule(Arc<Probe>);

impl Module for ProbeModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: "probe",
            name: "Probe",
            description: "Task kinds for the schedule's tests.",
            version: "0",
            depends_on: &[],
            default_enabled: true,
        }
    }

    fn task_kinds(&self) -> Vec<TaskKind> {
        let options = vec![
            TaskOption { value: "fast".to_string(), label: "Fast".to_string() },
            TaskOption { value: "slow".to_string(), label: "Slow".to_string() },
        ];

        vec![
            // Answers at once: done with `say`, or failed with `fail` when that is set.
            TaskKind {
                id: "probe.now".to_string(),
                module: "probe".to_string(),
                name: "Answer at once".to_string(),
                description: "Done as soon as it is started.".to_string(),
                fields: vec![
                    TaskField { required: true, default: Some(json!("all good")), ..field("say", "Say", TaskFieldType::Text) },
                    field("fail", "Fail with", TaskFieldType::Text),
                    field("query", "Query", TaskFieldType::Query),
                    TaskField { options, ..field("speed", "Speed", TaskFieldType::Select) },
                    field("loud", "Loud", TaskFieldType::Boolean),
                    TaskField { default: Some(json!(3)), ..field("times", "Times", TaskFieldType::Number) },
                    field("notes", "Notes", TaskFieldType::Textarea),
                ],
            },
            // Hands the work on: the test finishes it, as the module's own worker would.
            TaskKind { id: "probe.later".to_string(), module: "probe".to_string(), name: "Answer later".to_string(), ..TaskKind::default() },
            TaskKind { id: "probe.panic".to_string(), module: "probe".to_string(), name: "Break".to_string(), ..TaskKind::default() },
            // Not this module's to offer: the host leaves both out.
            TaskKind { id: "other.thing".to_string(), module: "probe".to_string(), ..TaskKind::default() },
            TaskKind { id: "probe.stolen".to_string(), module: "other".to_string(), ..TaskKind::default() },
        ]
    }

    fn start_task(&self, _app: &App, task: &ScheduledTask, actor: &AuthUser) -> Option<ApiResult<TaskStarted>> {
        self.0.started.lock().unwrap().push(Started { task: task.name.clone(), actor: actor.id, through: actor.through.clone() });

        match task.kind.as_str() {
            "probe.now" => Some(match task.config.get("fail").and_then(Value::as_str).filter(|reason| !reason.is_empty()) {
                Some(reason) => Err(ApiError::conflict(reason)),
                None => {
                    Ok(TaskStarted::Done { summary: task.config["say"].as_str().unwrap_or("").to_string(), link: Some("/somewhere".to_string()) })
                }
            }),
            "probe.later" => Some(Ok(TaskStarted::Running { summary: "Handed on".to_string() })),
            "probe.panic" => panic!("the probe was told to"),
            _ => None,
        }
    }
}

// ---------- the scene ----------

struct Scene {
    app: App,
    probe: Arc<Probe>,
    admin: Client,
    events: Receiver<Event>,
}

/// An instance with one administrator, the probe module and the project WEB.
async fn scene() -> Scene {
    let probe = Arc::new(Probe::default());
    let app = test_app(vec![Arc::new(ProbeModule(probe.clone()))]);
    let mut admin = Client::new(build_router(app.clone()));
    admin.setup(false).await;
    admin.post("/api/projects", json!({ "key": "WEB", "name": "Web" })).await;

    Scene { events: app.subscribe(), app, probe, admin }
}

fn now_task(name: &str, slot: &str) -> Value {
    json!({ "kind": "probe.now", "name": name, "slot": slot, "config": { "say": format!("{name} is done") } })
}

async fn make(client: &mut Client, project: &str, task: Value) -> i64 {
    client.post(&format!("/api/projects/{project}/tasks"), task).await["id"].as_i64().expect("a task id")
}

async fn task(client: &mut Client, project: &str, id: i64) -> Value {
    let tasks = client.get(&format!("/api/projects/{project}/tasks")).await;

    tasks.as_array().expect("tasks").iter().find(|task| task["id"] == id).unwrap_or_else(|| panic!("no task {id} in {tasks}")).clone()
}

/// Waits until the task's attempt has ended, as announced on the live stream.
async fn ended(events: &mut Receiver<Event>, id: i64) -> String {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(30), events.recv()).await.expect("the task's attempt ends").expect("an event");
        let state = event.payload["state"].as_str().unwrap_or("");

        if event.kind == "task.updated" && event.payload["taskId"] == id && event.payload["change"].is_null() && ["ok", "failed"].contains(&state) {
            return state.to_string();
        }
    }
}

/// Runs the task from its button and waits for the attempt to end.
async fn run(scene: &mut Scene, id: i64) -> String {
    // What earlier attempts announced is not this one's end.
    scene.events = scene.app.subscribe();

    let started = scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await;
    assert_eq!(started["state"], "running", "the answer is the task, now running");

    ended(&mut scene.events, id).await
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text).expect("an instant").with_timezone(&Utc)
}

/// A ticker on a clock the test sets.
fn clockwork(app: &App) -> (Ticker, Arc<Mutex<DateTime<Utc>>>) {
    let clock = Arc::new(Mutex::new(at("2026-11-13T06:00:00Z")));
    let hand = clock.clone();

    (Ticker::new(app, move || *hand.lock().unwrap()), clock)
}

/// Moves the clock and checks once. Times are UTC; the schedule's zone is set to UTC too.
fn tick_at(ticker: &Ticker, clock: &Mutex<DateTime<Utc>>, text: &str) -> Vec<(String, TaskSlot)> {
    *clock.lock().unwrap() = at(text);

    ticker.tick().expect("a check")
}

async fn utc_schedule(client: &mut Client) {
    client.put("/api/admin/schedule", json!({ "morning": "10:00", "afternoon": "16:00", "timezone": "UTC", "weekdaysOnly": true })).await;
}

// ---------- tasks ----------

#[tokio::test(flavor = "multi_thread")]
async fn the_kinds_are_those_of_the_modules_that_are_on() {
    let mut scene = scene().await;
    let kinds = scene.admin.get("/api/task-kinds").await;
    let ids: Vec<&str> = kinds.as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap()).collect();

    assert_eq!(ids, ["probe.now", "probe.later", "probe.panic"], "a kind that is not named after its module is left out");
    assert_eq!(kinds[0]["module"], "probe");
    assert_eq!(
        kinds[0]["fields"][0],
        json!({ "name": "say", "label": "Say", "required": true, "type": "text", "options": [], "default": "all good" })
    );
    assert_eq!(kinds[0]["fields"][3]["options"][1], json!({ "value": "slow", "label": "Slow" }));

    scene.admin.put("/api/admin/modules/probe", json!({ "enabled": false })).await;
    assert_eq!(scene.admin.get("/api/task-kinds").await, json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_is_made_changed_and_deleted() {
    let mut scene = scene().await;
    let me = scene.admin.get("/api/bootstrap").await["user"]["id"].clone();
    let made = scene
        .admin
        .post(
            "/api/projects/web/tasks",
            json!({ "kind": "probe.now", "name": "  Morning check  ", "slot": "morning", "config": { "say": " hello ", "loud": true, "unknown": "dropped" } }),
        )
        .await;
    let id = made["id"].as_i64().unwrap();

    assert_eq!(made["projectKey"], "WEB");
    assert_eq!(made["kind"], "probe.now");
    assert_eq!(made["name"], "Morning check");
    assert_eq!(made["slot"], "morning");
    assert_eq!(made["enabled"], true);
    assert_eq!(made["runAs"], me);
    assert_eq!(made["state"], "idle");
    assert_eq!(made["kindAvailable"], true);
    assert!(made["result"].is_null() && made["lastError"].is_null() && made["lastStartedAt"].is_null() && made["lastFinishedAt"].is_null());
    assert_eq!(
        made["config"],
        json!({ "say": "hello", "fail": "", "query": "", "speed": "", "loud": true, "times": 3, "notes": "" }),
        "trimmed, defaults filled in, what the form does not have left out"
    );

    let changed = scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "name": "Evening check", "slot": "afternoon", "enabled": false })).await;
    assert_eq!(
        (changed["name"].as_str(), changed["slot"].as_str(), changed["enabled"].as_bool()),
        (Some("Evening check"), Some("afternoon"), Some(false))
    );
    assert_eq!(changed["config"]["say"], "hello", "what was not sent stays");

    let changed = scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "config": { "say": "bye", "speed": "slow", "times": 7 } })).await;
    assert_eq!(changed["config"], json!({ "say": "bye", "fail": "", "query": "", "speed": "slow", "loud": false, "times": 7, "notes": "" }));

    assert_eq!(scene.admin.get("/api/projects/WEB/tasks").await.as_array().unwrap().len(), 1);
    assert_eq!(scene.admin.delete(&format!("/api/tasks/{id}")).await, json!({ "ok": true }));
    assert_eq!(scene.admin.get("/api/projects/WEB/tasks").await, json!([]));
    assert_eq!(scene.admin.refused(Method::DELETE, &format!("/api/tasks/{id}"), None).await, StatusCode::NOT_FOUND);

    // Each of the three is in the project's activity, told apart by `change`.
    let activity = scene.admin.get("/api/activity?project=WEB").await;
    let changes: Vec<(&str, &str)> = activity
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "task.updated")
        .map(|item| (item["payload"]["change"].as_str().unwrap(), item["payload"]["name"].as_str().unwrap()))
        .collect();
    assert_eq!(changes, [("deleted", "Evening check"), ("changed", "Evening check"), ("changed", "Evening check"), ("created", "Morning check")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_task_holds_is_checked_when_it_is_saved() {
    let mut scene = scene().await;
    let with = |config: Value| json!({ "kind": "probe.now", "name": "Check", "slot": "on_demand", "config": config });

    for (body, message) in [
        (
            json!({ "kind": "nobody.knows", "name": "Check", "slot": "on_demand" }),
            "That kind of task is not available in this project; its module may be switched off",
        ),
        (json!({ "kind": "probe.now", "name": "  ", "slot": "on_demand" }), "The task needs a name"),
        (with(json!({ "say": "" })), "\"Say\" is required"),
        (with(json!({ "say": 5 })), "\"Say\" has to be text"),
        (with(json!({ "times": "many" })), "\"Times\" has to be a number"),
        (with(json!({ "loud": "yes" })), "\"Loud\" has to be yes or no"),
        (with(json!({ "speed": "warp" })), "\"Speed\" has to be one of its options"),
        (with(json!({ "query": "status = " })), "\"Query\": Expected a value"),
    ] {
        let (status, answer) = scene.admin.send(Method::POST, "/api/projects/WEB/tasks", Some(body.clone())).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(answer["error"]["message"], message, "{body}");
    }

    let unknown_slot = json!({ "kind": "probe.now", "name": "Check", "slot": "at_night" });
    assert!(scene.admin.refused(Method::POST, "/api/projects/WEB/tasks", Some(unknown_slot)).await.is_client_error());

    let fine = with(json!({ "query": "status = \"To Do\" ORDER BY created", "speed": "fast" }));
    scene.admin.post("/api/projects/WEB/tasks", fine).await;
    assert_eq!(scene.admin.get("/api/projects/WEB/tasks").await.as_array().unwrap().len(), 1, "nothing that was refused was kept");
}

#[tokio::test(flavor = "multi_thread")]
async fn who_may_do_what_with_tasks() {
    let mut scene = scene().await;
    let (mut viewer, viewer_id) = scene.admin.member("Wiktor", "wiktor@example.com").await;
    let (mut member, _) = scene.admin.member("Marta", "marta@example.com").await;
    let (mut lead, lead_id) = scene.admin.member("Lena", "lena@example.com").await;
    let (mut outsider, _) = scene.admin.member("Olek", "olek@example.com").await;

    scene.admin.post("/api/projects", json!({ "key": "SEC", "name": "Secret", "isPrivate": true })).await;
    scene.admin.put(&format!("/api/projects/WEB/members/{viewer_id}"), json!({ "role": "viewer" })).await;
    scene.admin.put(&format!("/api/projects/WEB/members/{lead_id}"), json!({ "role": "admin" })).await;
    scene.admin.put(&format!("/api/projects/SEC/members/{viewer_id}"), json!({ "role": "viewer" })).await;

    let id = make(&mut scene.admin, "WEB", now_task("Check", "morning")).await;
    let secret = make(&mut scene.admin, "SEC", now_task("Secret check", "morning")).await;
    let new = Some(now_task("Another", "on_demand"));
    let rename = Some(json!({ "name": "Renamed" }));
    let slot = Some(json!({ "slot": "morning" }));

    // Everyone signed in reads the kinds and the settings; only people of the project its tasks.
    for client in [&mut viewer, &mut member, &mut lead, &mut outsider] {
        assert!(client.get("/api/task-kinds").await.is_array());
        assert_eq!(client.get("/api/schedule").await["timezone"], "Europe/Warsaw");
        assert_eq!(client.get("/api/projects/WEB/tasks").await.as_array().unwrap().len(), 1);
    }

    // A viewer reads.
    assert_eq!(viewer.refused(Method::POST, "/api/projects/WEB/tasks", new.clone()).await, StatusCode::FORBIDDEN);
    assert_eq!(viewer.refused(Method::PATCH, &format!("/api/tasks/{id}"), rename.clone()).await, StatusCode::FORBIDDEN);
    assert_eq!(viewer.refused(Method::DELETE, &format!("/api/tasks/{id}"), None).await, StatusCode::FORBIDDEN);
    assert_eq!(viewer.refused(Method::POST, &format!("/api/tasks/{id}/run"), Some(json!({}))).await, StatusCode::FORBIDDEN);
    assert_eq!(viewer.refused(Method::POST, "/api/projects/WEB/tasks/run", slot.clone()).await, StatusCode::FORBIDDEN);
    assert_eq!(viewer.get("/api/projects/SEC/tasks").await.as_array().unwrap().len(), 1, "a viewer of the private project sees its tasks");

    // A member runs, and changes nothing.
    assert_eq!(member.refused(Method::POST, "/api/projects/WEB/tasks", new.clone()).await, StatusCode::FORBIDDEN);
    assert_eq!(member.refused(Method::PATCH, &format!("/api/tasks/{id}"), rename.clone()).await, StatusCode::FORBIDDEN);
    assert_eq!(member.refused(Method::DELETE, &format!("/api/tasks/{id}"), None).await, StatusCode::FORBIDDEN);
    assert_eq!(member.post(&format!("/api/tasks/{id}/run"), json!({})).await["state"], "running");
    assert_eq!(ended(&mut scene.events, id).await, "ok");
    assert_eq!(member.post("/api/projects/WEB/tasks/run", json!({ "slot": "morning" })).await.as_array().unwrap().len(), 1);
    assert_eq!(ended(&mut scene.events, id).await, "ok");

    // A project administrator does everything, and so does an administrator of the instance.
    let made = lead.post("/api/projects/WEB/tasks", new.clone().unwrap()).await;
    assert_eq!(made["runAs"], lead_id);
    assert_eq!(lead.patch(&format!("/api/tasks/{id}"), rename.clone().unwrap()).await["name"], "Renamed");
    assert_eq!(lead.delete(&format!("/api/tasks/{}", made["id"])).await, json!({ "ok": true }));

    // A private project and its tasks do not exist for people outside it.
    assert_eq!(outsider.refused(Method::GET, "/api/projects/SEC/tasks", None).await, StatusCode::NOT_FOUND);
    assert_eq!(outsider.refused(Method::POST, "/api/projects/SEC/tasks", new.clone()).await, StatusCode::NOT_FOUND);
    assert_eq!(outsider.refused(Method::POST, "/api/projects/SEC/tasks/run", slot.clone()).await, StatusCode::NOT_FOUND);
    assert_eq!(outsider.refused(Method::PATCH, &format!("/api/tasks/{secret}"), rename.clone()).await, StatusCode::NOT_FOUND);
    assert_eq!(outsider.refused(Method::DELETE, &format!("/api/tasks/{secret}"), None).await, StatusCode::NOT_FOUND);
    assert_eq!(outsider.refused(Method::POST, &format!("/api/tasks/{secret}/run"), Some(json!({}))).await, StatusCode::NOT_FOUND);
    assert_eq!(
        lead.refused(Method::POST, &format!("/api/tasks/{secret}/run"), Some(json!({}))).await,
        StatusCode::NOT_FOUND,
        "an administrator of another project is outside too"
    );

    // And nobody without a session gets anywhere.
    let mut nobody = Client::new(scene.admin.router.clone());
    assert_eq!(nobody.refused(Method::GET, "/api/task-kinds", None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(nobody.refused(Method::GET, "/api/schedule", None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(nobody.refused(Method::GET, "/api/projects/WEB/tasks", None).await, StatusCode::UNAUTHORIZED);
}

// ---------- running ----------

#[tokio::test(flavor = "multi_thread")]
async fn run_now_marks_the_task_running_and_records_what_the_module_answered() {
    let mut scene = scene().await;
    let me = scene.admin.get("/api/bootstrap").await["user"]["id"].as_i64().unwrap();
    let id = make(&mut scene.admin, "WEB", now_task("Check", "on_demand")).await;

    let started = scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await;
    assert_eq!(started["state"], "running");
    assert!(started["lastStartedAt"].is_string());

    assert_eq!(ended(&mut scene.events, id).await, "ok");

    let task = task(&mut scene.admin, "WEB", id).await;
    assert_eq!(task["state"], "ok");
    assert_eq!(task["result"]["summary"], "Check is done");
    assert_eq!(task["result"]["link"], "/somewhere");
    assert_eq!(task["result"]["at"], task["lastFinishedAt"]);
    assert!(task["lastError"].is_null() && task["progress"].is_null());

    // The module was asked once, as the person the task runs as, acting through the module.
    assert_eq!(*scene.probe.started.lock().unwrap(), [Started { task: "Check".to_string(), actor: me, through: vec!["probe"] }]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_attempt_keeps_the_last_good_result() {
    let mut scene = scene().await;
    let id = make(&mut scene.admin, "WEB", now_task("Check", "on_demand")).await;

    assert_eq!(run(&mut scene, id).await, "ok");
    let good = task(&mut scene.admin, "WEB", id).await["result"].clone();
    assert_eq!(good["summary"], "Check is done");

    scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "config": { "say": "never said", "fail": "The lights went out" } })).await;
    assert_eq!(run(&mut scene, id).await, "failed");

    let failed = task(&mut scene.admin, "WEB", id).await;
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["lastError"], "The lights went out");
    assert_eq!(failed["result"], good, "the older result stays, with the time it is from");
    assert!(failed["lastFinishedAt"].as_str() > good["at"].as_str(), "and the attempt that failed has its own time");

    // A second failure changes nothing about that; a success replaces both.
    assert_eq!(run(&mut scene, id).await, "failed");
    assert_eq!(task(&mut scene.admin, "WEB", id).await["result"], good);

    scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "config": { "say": "back again" } })).await;
    assert_eq!(run(&mut scene, id).await, "ok");

    let recovered = task(&mut scene.admin, "WEB", id).await;
    assert_eq!(recovered["result"]["summary"], "back again");
    assert!(recovered["lastError"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_that_never_worked_fails_with_no_result() {
    let mut scene = scene().await;
    let id =
        make(&mut scene.admin, "WEB", json!({ "kind": "probe.now", "name": "Check", "slot": "on_demand", "config": { "fail": "No luck" } })).await;

    assert_eq!(run(&mut scene, id).await, "failed");

    let task = task(&mut scene.admin, "WEB", id).await;
    assert_eq!(task["lastError"], "No luck");
    assert!(task["result"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_finishes_what_it_reported_as_running() {
    let mut scene = scene().await;
    let id = make(&mut scene.admin, "WEB", json!({ "kind": "probe.later", "name": "Long job", "slot": "on_demand" })).await;

    scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await;

    // The module answered `Running`: the task stays so, with what the module said.
    loop {
        let event = tokio::time::timeout(Duration::from_secs(30), scene.events.recv()).await.expect("the module answers").expect("an event");

        if event.kind == "task.updated" && scene.probe.names() == ["Long job"] && task(&mut scene.admin, "WEB", id).await["progress"] == "Handed on" {
            break;
        }
    }

    let running = task(&mut scene.admin, "WEB", id).await;
    assert_eq!(running["state"], "running");
    assert!(running["result"].is_null());

    // It cannot be started again meanwhile, by its button or with its group.
    let (status, answer) = scene.admin.send(Method::POST, &format!("/api/tasks/{id}/run"), Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(answer["error"]["message"], "The task is already running");
    assert_eq!(scene.admin.post("/api/projects/WEB/tasks/run", json!({ "slot": "on_demand" })).await, json!([]));
    assert_eq!(scene.probe.names(), ["Long job"]);

    // The module's work ends: it says so in its own transaction.
    let outcome = Outcome { summary: "12 passed".to_string(), link: Some("/p/WEB/tests".to_string()) };
    scene.app.write(Actor::system("Probe"), |tx| schedule::finish(tx, id, Ok(outcome))).unwrap();
    assert_eq!(ended(&mut scene.events, id).await, "ok");

    let done = task(&mut scene.admin, "WEB", id).await;
    assert_eq!(done["state"], "ok");
    assert_eq!(done["result"]["summary"], "12 passed");
    assert_eq!(done["result"]["link"], "/p/WEB/tests");
    assert!(done["progress"].is_null());

    // The next time it ends badly: the reason is kept beside the older result.
    scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await;
    scene.app.write(Actor::system("Probe"), |tx| schedule::finish(tx, id, Err("3 of 12 failed".to_string()))).unwrap();
    assert_eq!(ended(&mut scene.events, id).await, "failed");

    let failed = task(&mut scene.admin, "WEB", id).await;
    assert_eq!(failed["lastError"], "3 of 12 failed");
    assert_eq!(failed["result"]["summary"], "12 passed");

    // A task deleted while its work went on has nobody to tell, and that is not an error.
    scene.admin.delete(&format!("/api/tasks/{id}")).await;
    scene.app.write(Actor::system("Probe"), |tx| schedule::finish(tx, id, Err("too late".to_string()))).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_that_breaks_fails_the_attempt_and_nothing_else() {
    let mut scene = scene().await;
    let id = make(&mut scene.admin, "WEB", json!({ "kind": "probe.panic", "name": "Breaks", "slot": "on_demand" })).await;

    assert_eq!(run(&mut scene, id).await, "failed");
    assert_eq!(task(&mut scene.admin, "WEB", id).await["lastError"], "The task's module broke while starting it; the server's log has the details");

    let fine = make(&mut scene.admin, "WEB", now_task("Fine", "on_demand")).await;
    assert_eq!(run(&mut scene, fine).await, "ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_starts_its_enabled_tasks_in_the_order_they_were_made() {
    let mut scene = scene().await;
    let first = make(&mut scene.admin, "WEB", now_task("First", "afternoon")).await;
    let off = make(&mut scene.admin, "WEB", now_task("Switched off", "afternoon")).await;
    make(&mut scene.admin, "WEB", now_task("Morning's", "morning")).await;
    let second =
        make(&mut scene.admin, "WEB", json!({ "kind": "probe.now", "name": "Second", "slot": "afternoon", "config": { "fail": "Not today" } })).await;
    let third = make(&mut scene.admin, "WEB", now_task("Third", "afternoon")).await;
    scene.admin.patch(&format!("/api/tasks/{off}"), json!({ "enabled": false })).await;

    let started = scene.admin.post("/api/projects/WEB/tasks/run", json!({ "slot": "afternoon" })).await;
    let names: Vec<&str> = started.as_array().unwrap().iter().map(|task| task["name"].as_str().unwrap()).collect();

    assert_eq!(names, ["First", "Second", "Third"]);
    assert!(started.as_array().unwrap().iter().all(|task| task["state"] == "running"));

    // One fails; the ones after it still run.
    assert_eq!(ended(&mut scene.events, first).await, "ok");
    assert_eq!(ended(&mut scene.events, second).await, "failed");
    assert_eq!(ended(&mut scene.events, third).await, "ok");
    assert_eq!(scene.probe.names(), ["First", "Second", "Third"]);

    // A task that is switched off still runs from its own button.
    assert_eq!(run(&mut scene, off).await, "ok");

    let (status, _) = scene.admin.send(Method::POST, "/api/projects/WEB/tasks/run", Some(json!({ "slot": "at_night" }))).await;
    assert!(status.is_client_error());
}

// ---------- the minute-by-minute check ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_slot_runs_when_its_time_has_passed_and_once_a_day() {
    let mut scene = scene().await;
    utc_schedule(&mut scene.admin).await;
    let morning = make(&mut scene.admin, "WEB", now_task("Morning", "morning")).await;
    let afternoon = make(&mut scene.admin, "WEB", now_task("Afternoon", "afternoon")).await;
    let on_demand = make(&mut scene.admin, "WEB", now_task("On demand", "on_demand")).await;
    let (ticker, clock) = clockwork(&scene.app);

    // Friday 13 November 2026.
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T09:59:00Z"), [], "not due yet");
    assert_eq!(scene.probe.names(), Vec::<String>::new());

    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T10:00:00Z"), [("WEB".to_string(), TaskSlot::Morning)]);
    assert_eq!(scene.probe.names(), ["Morning"]);

    // The check is over when `tick` returns: the result is there, dated by the ticker's clock.
    let ran = task(&mut scene.admin, "WEB", morning).await;
    assert_eq!(ran["state"], "ok");
    assert_eq!(ran["lastStartedAt"], "2026-11-13T10:00:00.000Z");
    assert_eq!(ran["result"], json!({ "at": "2026-11-13T10:00:00.000Z", "summary": "Morning is done", "link": "/somewhere" }));

    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T10:01:00Z"), [], "once a day");
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T15:59:00Z"), []);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T16:00:00Z"), [("WEB".to_string(), TaskSlot::Afternoon)]);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T23:59:00Z"), []);
    assert_eq!(scene.probe.names(), ["Morning", "Afternoon"]);
    assert_eq!(task(&mut scene.admin, "WEB", afternoon).await["result"]["at"], "2026-11-13T16:00:00.000Z");
    assert_eq!(task(&mut scene.admin, "WEB", on_demand).await["state"], "idle", "on demand runs from its button only");

    // The weekend is quiet; Monday starts again.
    assert_eq!(tick_at(&ticker, &clock, "2026-11-14T12:00:00Z"), []);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-15T18:00:00Z"), []);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-16T10:00:00Z"), [("WEB".to_string(), TaskSlot::Morning)]);
    assert_eq!(scene.probe.names(), ["Morning", "Afternoon", "Morning"]);

    // The journal names the schedule, not a person, for what it started.
    let by_schedule = scene.app.read(|connection| {
        Ok(connection
            .query_row("SELECT count(*) FROM event WHERE kind = 'task.updated' AND actor_label = 'Schedule'", [], |row| row.get::<_, i64>(0))?)
    });
    assert_eq!(by_schedule.unwrap(), 6, "three runs, each marked running and then done");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_was_down_catches_up_once() {
    let mut scene = scene().await;
    utc_schedule(&mut scene.admin).await;
    make(&mut scene.admin, "WEB", now_task("Morning", "morning")).await;
    make(&mut scene.admin, "WEB", now_task("Afternoon", "afternoon")).await;
    let (ticker, clock) = clockwork(&scene.app);

    // Down from nine to two on Monday: the morning slot runs once when it is back.
    assert_eq!(tick_at(&ticker, &clock, "2026-11-09T14:00:00Z"), [("WEB".to_string(), TaskSlot::Morning)]);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-09T14:01:00Z"), []);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-09T16:30:00Z"), [("WEB".to_string(), TaskSlot::Afternoon)]);
    scene.probe.forget();

    // Down all of Tuesday and Wednesday, back on Thursday at eight: nothing is made up.
    assert_eq!(tick_at(&ticker, &clock, "2026-11-12T08:00:00Z"), []);

    // Down again until Friday evening: one run of each slot for today, the morning's first.
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T17:00:00Z"), [("WEB".to_string(), TaskSlot::Morning), ("WEB".to_string(), TaskSlot::Afternoon)]);
    assert_eq!(scene.probe.names(), ["Morning", "Afternoon"]);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T17:01:00Z"), []);

    // What ran is remembered in the database, not in the ticker: a new one knows it too.
    let (again, clock) = clockwork(&scene.app);
    assert_eq!(tick_at(&again, &clock, "2026-11-13T17:02:00Z"), []);
}

#[tokio::test(flavor = "multi_thread")]
async fn example_data_starts_as_if_the_day_had_already_run() {
    let mut scene = scene().await;
    utc_schedule(&mut scene.admin).await;
    make(&mut scene.admin, "WEB", now_task("Morning", "morning")).await;
    make(&mut scene.admin, "WEB", now_task("Afternoon", "afternoon")).await;
    let (ticker, clock) = clockwork(&scene.app);

    // The demo is made at two in the afternoon: its morning's tasks do not run a minute later.
    *clock.lock().unwrap() = at("2026-11-13T14:00:00Z");
    ticker.skip_due().unwrap();
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T14:01:00Z"), []);

    // From then on the schedule is the usual one.
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T16:00:00Z"), [("WEB".to_string(), TaskSlot::Afternoon)]);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-16T10:00:00Z"), [("WEB".to_string(), TaskSlot::Morning)]);
    assert_eq!(scene.probe.names(), ["Afternoon", "Morning"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_settings_decide_when_and_whether_a_slot_is_due() {
    let mut scene = scene().await;
    make(&mut scene.admin, "WEB", now_task("Morning", "morning")).await;
    let (ticker, clock) = clockwork(&scene.app);

    // The default is ten o'clock in Warsaw: 09:00 UTC in winter.
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T08:59:00Z"), []);
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T09:00:00Z"), [("WEB".to_string(), TaskSlot::Morning)]);

    // Saturday, with the weekend switched on and the morning moved to half past seven in Tokyo.
    scene.admin.put("/api/admin/schedule", json!({ "morning": "7:30", "afternoon": "16:00", "timezone": "Asia/Tokyo", "weekdaysOnly": false })).await;
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T22:29:00Z"), [], "07:29 on Saturday in Tokyo");
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T22:30:00Z"), [("WEB".to_string(), TaskSlot::Morning)], "Saturday's run, though UTC says Friday");
    assert_eq!(scene.probe.names(), ["Morning", "Morning"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_project_has_its_own_slots() {
    let mut scene = scene().await;
    utc_schedule(&mut scene.admin).await;
    scene.admin.post("/api/projects", json!({ "key": "API", "name": "Api" })).await;
    scene.admin.post("/api/projects", json!({ "key": "OLD", "name": "Old" })).await;
    make(&mut scene.admin, "WEB", now_task("Web's", "morning")).await;
    make(&mut scene.admin, "API", now_task("Api's first", "morning")).await;
    make(&mut scene.admin, "API", now_task("Api's second", "morning")).await;
    make(&mut scene.admin, "OLD", now_task("Old's", "morning")).await;
    scene.admin.patch("/api/projects/OLD", json!({ "archived": true })).await;
    let (ticker, clock) = clockwork(&scene.app);

    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T10:00:00Z"), [("WEB".to_string(), TaskSlot::Morning), ("API".to_string(), TaskSlot::Morning)]);
    assert_eq!(scene.probe.names(), ["Web's", "Api's first", "Api's second"], "an archived project runs nothing");

    // A project made after the slot's time waits for tomorrow, like a task added after it.
    scene.admin.post("/api/projects", json!({ "key": "NEW", "name": "New" })).await;
    assert_eq!(
        tick_at(&ticker, &clock, "2026-11-13T11:00:00Z"),
        [("NEW".to_string(), TaskSlot::Morning)],
        "its slot is remembered as run, with nothing in it"
    );
    make(&mut scene.admin, "NEW", now_task("New's", "morning")).await;
    make(&mut scene.admin, "WEB", now_task("Web's late one", "morning")).await;
    assert_eq!(tick_at(&ticker, &clock, "2026-11-13T11:01:00Z"), []);
    assert_eq!(scene.probe.names().len(), 3);

    assert_eq!(tick_at(&ticker, &clock, "2026-11-16T10:00:00Z").len(), 3);
    assert_eq!(scene.probe.names()[3..], ["Web's", "Web's late one", "Api's first", "Api's second", "New's"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slot_leaves_alone_what_is_switched_off_or_still_running() {
    let mut scene = scene().await;
    utc_schedule(&mut scene.admin).await;
    let long = make(&mut scene.admin, "WEB", json!({ "kind": "probe.later", "name": "Long job", "slot": "morning" })).await;
    let off = make(&mut scene.admin, "WEB", now_task("Switched off", "morning")).await;
    make(&mut scene.admin, "WEB", now_task("Quick", "morning")).await;
    scene.admin.patch(&format!("/api/tasks/{off}"), json!({ "enabled": false })).await;
    let (ticker, clock) = clockwork(&scene.app);

    tick_at(&ticker, &clock, "2026-11-12T10:00:00Z");
    assert_eq!(scene.probe.names(), ["Long job", "Quick"]);
    assert_eq!(task(&mut scene.admin, "WEB", long).await["state"], "running");
    assert_eq!(task(&mut scene.admin, "WEB", long).await["progress"], "Handed on");
    assert_eq!(task(&mut scene.admin, "WEB", off).await["state"], "idle");

    // Its work reports back within six hours, so the task was not given up on...
    *clock.lock().unwrap() = at("2026-11-12T15:59:00Z");
    ticker.tick().unwrap();
    assert_eq!(task(&mut scene.admin, "WEB", long).await["state"], "running");
    scene
        .app
        .write(Actor::system("Probe"), |tx| schedule::finish(tx, long, Ok(Outcome { summary: "Done at last".to_string(), link: None })))
        .unwrap();

    // ...and the next morning it starts again. This time nothing reports back.
    scene.probe.forget();
    tick_at(&ticker, &clock, "2026-11-13T10:00:00Z");
    assert_eq!(scene.probe.names(), ["Long job", "Quick"]);

    // Still running when the slot comes round: it is left alone, the others run.
    scene.admin.put("/api/admin/schedule", json!({ "morning": "10:00", "afternoon": "12:00", "timezone": "UTC", "weekdaysOnly": true })).await;
    let other = make(&mut scene.admin, "WEB", json!({ "kind": "probe.later", "name": "Long job", "slot": "afternoon" })).await;
    scene.admin.patch(&format!("/api/tasks/{long}"), json!({ "slot": "afternoon" })).await;
    scene.probe.forget();
    tick_at(&ticker, &clock, "2026-11-13T12:00:00Z");
    assert_eq!(scene.probe.started.lock().unwrap().len(), 1, "only the one that was not running");
    assert_eq!(task(&mut scene.admin, "WEB", long).await["lastStartedAt"], "2026-11-13T10:00:00.000Z");
    assert_eq!(task(&mut scene.admin, "WEB", other).await["lastStartedAt"], "2026-11-13T12:00:00.000Z");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_left_running_for_six_hours_is_marked_failed() {
    let mut scene = scene().await;
    utc_schedule(&mut scene.admin).await;
    let id = make(&mut scene.admin, "WEB", json!({ "kind": "probe.later", "name": "Long job", "slot": "morning" })).await;
    let (ticker, clock) = clockwork(&scene.app);

    // A first attempt works, so that there is a result to keep.
    tick_at(&ticker, &clock, "2026-11-12T10:00:00Z");
    scene.app.write(Actor::system("Probe"), |tx| schedule::finish(tx, id, Ok(Outcome { summary: "All passed".to_string(), link: None }))).unwrap();
    let good = task(&mut scene.admin, "WEB", id).await["result"].clone();

    tick_at(&ticker, &clock, "2026-11-13T10:00:00Z");
    tick_at(&ticker, &clock, "2026-11-13T15:59:00Z");
    assert_eq!(task(&mut scene.admin, "WEB", id).await["state"], "running", "a minute short of six hours");

    tick_at(&ticker, &clock, "2026-11-13T16:00:00Z");
    let given_up = task(&mut scene.admin, "WEB", id).await;
    assert_eq!(given_up["state"], "failed");
    assert_eq!(given_up["lastError"], "It was still running after six hours and was given up on");
    assert_eq!(given_up["lastFinishedAt"], "2026-11-13T16:00:00.000Z");
    assert_eq!(given_up["result"], good, "the last good result stays");
    assert!(given_up["progress"].is_null());

    // It can be run again, and work that reports back late is still believed.
    assert_eq!(scene.admin.post(&format!("/api/tasks/{id}/run"), json!({})).await["state"], "running");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_cut_off_by_the_server_stopping_is_failed_when_it_comes_back() {
    let mut scene = scene().await;
    let cut_off = make(&mut scene.admin, "WEB", now_task("Cut off", "on_demand")).await;
    let handed_on = make(&mut scene.admin, "WEB", json!({ "kind": "probe.later", "name": "Handed on", "slot": "on_demand" })).await;
    let (ticker, clock) = clockwork(&scene.app);

    // As the database looks when the server stops between marking a task and starting it,
    // next to one whose module had answered and is working on it.
    scene
        .app
        .db
        .write(|tx| {
            tx.execute("UPDATE scheduled_task SET state = 'running', last_started_at = '2026-11-13T09:00:00.000Z' WHERE id = ?1", [cut_off])?;
            tx.execute(
                "UPDATE scheduled_task SET state = 'running', last_started_at = '2026-11-13T09:00:00.000Z', progress = 'Handed on' WHERE id = ?1",
                [handed_on],
            )?;

            Ok(())
        })
        .unwrap();

    *clock.lock().unwrap() = at("2026-11-13T09:05:00Z");
    ticker.recover().unwrap();

    let failed = task(&mut scene.admin, "WEB", cut_off).await;
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["lastError"], "The server stopped while the task was starting");
    assert_eq!(task(&mut scene.admin, "WEB", handed_on).await["state"], "running");
}

// ---------- modules that are off, people who are gone ----------

#[tokio::test(flavor = "multi_thread")]
async fn a_task_of_a_module_that_is_off_is_shown_and_never_run() {
    let mut scene = scene().await;
    utc_schedule(&mut scene.admin).await;
    scene.admin.post("/api/projects", json!({ "key": "API", "name": "Api" })).await;
    let web = make(&mut scene.admin, "WEB", now_task("Web's", "morning")).await;
    let api = make(&mut scene.admin, "API", now_task("Api's", "morning")).await;
    let (ticker, clock) = clockwork(&scene.app);

    // Off in one project: that project's task is listed as unavailable, the other runs.
    scene.admin.put("/api/projects/WEB/modules/probe", json!({ "enabled": false })).await;
    assert_eq!(task(&mut scene.admin, "WEB", web).await["kindAvailable"], false);
    assert_eq!(task(&mut scene.admin, "API", api).await["kindAvailable"], true);
    assert_eq!(scene.admin.get("/api/task-kinds").await.as_array().unwrap().len(), 3, "the instance still has the kinds");

    let (status, answer) = scene.admin.send(Method::POST, &format!("/api/tasks/{web}/run"), Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(answer["error"]["message"], "This task cannot run: the module it belongs to is switched off");
    assert_eq!(scene.admin.post("/api/projects/WEB/tasks/run", json!({ "slot": "morning" })).await, json!([]));
    assert_eq!(scene.admin.refused(Method::POST, "/api/projects/WEB/tasks", Some(now_task("Another", "morning"))).await, StatusCode::BAD_REQUEST);
    assert_eq!(
        scene.admin.refused(Method::PATCH, &format!("/api/tasks/{web}"), Some(json!({ "config": { "say": "x" } }))).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        scene.admin.patch(&format!("/api/tasks/{web}"), json!({ "enabled": false })).await["enabled"],
        false,
        "it can still be switched off, renamed, deleted"
    );
    scene.admin.patch(&format!("/api/tasks/{web}"), json!({ "enabled": true })).await;

    tick_at(&ticker, &clock, "2026-11-12T10:00:00Z");
    assert_eq!(scene.probe.names(), ["Api's"], "the slot skipped it");
    assert_eq!(task(&mut scene.admin, "WEB", web).await["state"], "idle");

    // Off for the instance: nothing of the module runs anywhere.
    scene.admin.put("/api/projects/WEB/modules/probe", json!({ "enabled": true })).await;
    scene.admin.put("/api/admin/modules/probe", json!({ "enabled": false })).await;
    assert_eq!(task(&mut scene.admin, "WEB", web).await["kindAvailable"], false);
    assert_eq!(task(&mut scene.admin, "API", api).await["kindAvailable"], false);
    assert_eq!(scene.admin.refused(Method::POST, &format!("/api/tasks/{api}/run"), Some(json!({}))).await, StatusCode::CONFLICT);
    tick_at(&ticker, &clock, "2026-11-13T10:00:00Z");
    assert_eq!(scene.probe.names(), ["Api's"]);

    // Back on: both run with the next slot.
    scene.admin.put("/api/admin/modules/probe", json!({ "enabled": true })).await;
    assert_eq!(task(&mut scene.admin, "WEB", web).await["kindAvailable"], true);
    tick_at(&ticker, &clock, "2026-11-16T10:00:00Z");
    assert_eq!(scene.probe.names(), ["Api's", "Web's", "Api's"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_never_runs_as_somebody_who_is_gone() {
    let mut scene = scene().await;
    utc_schedule(&mut scene.admin).await;
    let (mut ola, ola_id) = scene.admin.member("Ola Lis", "ola@example.com").await;
    scene.admin.post("/api/projects", json!({ "key": "SEC", "name": "Secret", "isPrivate": true })).await;
    scene.admin.put(&format!("/api/projects/SEC/members/{ola_id}"), json!({ "role": "admin" })).await;

    let id = make(&mut ola, "SEC", now_task("Ola's", "morning")).await;
    assert_eq!(task(&mut scene.admin, "SEC", id).await["runAs"], ola_id);
    assert_eq!(run(&mut scene, id).await, "ok");
    assert_eq!(scene.probe.started.lock().unwrap()[0].actor, ola_id, "whoever presses the button, the task runs as the person it belongs to");
    let good = task(&mut scene.admin, "SEC", id).await["result"].clone();
    scene.probe.forget();

    // She loses her place in the private project.
    scene.admin.put(&format!("/api/projects/SEC/members/{ola_id}"), json!({ "role": null })).await;
    assert_eq!(run(&mut scene, id).await, "failed");
    let failed = task(&mut scene.admin, "SEC", id).await;
    assert_eq!(
        failed["lastError"],
        "Ola Lis can no longer see this project, and the task runs with their permissions; save the task again to run it as yourself"
    );
    assert_eq!(failed["result"], good);

    // She is deactivated: the same from the slot.
    scene.admin.put(&format!("/api/projects/SEC/members/{ola_id}"), json!({ "role": "member" })).await;
    scene.admin.patch(&format!("/api/users/{ola_id}"), json!({ "active": false })).await;
    let (ticker, clock) = clockwork(&scene.app);
    tick_at(&ticker, &clock, "2026-11-13T10:00:00Z");
    let failed = task(&mut scene.admin, "SEC", id).await;
    assert_eq!(failed["state"], "failed");
    assert_eq!(
        failed["lastError"],
        "Ola Lis has been deactivated, and the task runs with their permissions; save the task again to run it as yourself"
    );
    assert_eq!(scene.probe.names(), Vec::<String>::new(), "the module was not asked either time");

    // Changing its name does not change whose it is; saving its settings does.
    let me = scene.admin.get("/api/bootstrap").await["user"]["id"].clone();
    assert_eq!(scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "name": "Was Ola's" })).await["runAs"], ola_id);
    assert_eq!(scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "config": { "say": "mine now" } })).await["runAs"], me);
    assert_eq!(run(&mut scene, id).await, "ok");
    assert_eq!(task(&mut scene.admin, "SEC", id).await["result"]["summary"], "mine now");
}

// ---------- settings, and who hears what ----------

#[tokio::test(flavor = "multi_thread")]
async fn administrators_set_the_times_and_the_zone() {
    let mut scene = scene().await;
    let (mut member, _) = scene.admin.member("Marta", "marta@example.com").await;
    let defaults = json!({ "morning": "10:00", "afternoon": "16:00", "timezone": "Europe/Warsaw", "weekdaysOnly": true });

    assert_eq!(scene.admin.get("/api/schedule").await, defaults);
    assert_eq!(member.refused(Method::PUT, "/api/admin/schedule", Some(defaults.clone())).await, StatusCode::FORBIDDEN);

    for (change, message) in [
        (json!({ "timezone": "Mars/Olympus" }), "\"Mars/Olympus\" is not a time zone; write its IANA name, like Europe/Warsaw"),
        (json!({ "timezone": "" }), "\"\" is not a time zone; write its IANA name, like Europe/Warsaw"),
        (json!({ "morning": "25:00" }), "The morning time has to be written as hours and minutes, like 10:00"),
        (json!({ "afternoon": "four" }), "The afternoon time has to be written as hours and minutes, like 10:00"),
        (json!({ "morning": "17:00" }), "The morning run has to come before the afternoon one"),
    ] {
        let mut wanted = defaults.clone();
        wanted.as_object_mut().unwrap().extend(change.as_object().unwrap().clone());
        let (status, answer) = scene.admin.send(Method::PUT, "/api/admin/schedule", Some(wanted)).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{change}");
        assert_eq!(answer["error"]["message"], message);
    }

    assert_eq!(scene.admin.get("/api/schedule").await, defaults, "nothing that was refused was kept");

    let saved = scene
        .admin
        .put("/api/admin/schedule", json!({ "morning": "8:30", "afternoon": "17:15", "timezone": " America/New_York ", "weekdaysOnly": false }))
        .await;
    assert_eq!(saved, json!({ "morning": "08:30", "afternoon": "17:15", "timezone": "America/New_York", "weekdaysOnly": false }));
    assert_eq!(member.get("/api/schedule").await, saved);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_change_to_a_task_is_announced_with_its_project() {
    let mut scene = scene().await;
    let project = scene.admin.get("/api/projects/WEB").await["id"].as_i64().unwrap();
    let id = make(&mut scene.admin, "WEB", now_task("Check", "on_demand")).await;
    scene.admin.patch(&format!("/api/tasks/{id}"), json!({ "enabled": false })).await;
    assert_eq!(run(&mut scene, id).await, "ok");
    scene.admin.delete(&format!("/api/tasks/{id}")).await;

    let announced: Vec<(Option<i64>, i64, String, Value)> = scene
        .app
        .read(|connection| {
            hub_app::db::query_all(connection, "SELECT project_id, payload FROM event WHERE kind = 'task.updated' ORDER BY id", [], |row| {
                let payload: Value = serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or_default();

                Ok((
                    row.get(0)?,
                    payload["taskId"].as_i64().unwrap_or(0),
                    payload["state"].as_str().unwrap_or("").to_string(),
                    payload["change"].clone(),
                ))
            })
        })
        .unwrap();

    assert_eq!(
        announced,
        [
            (Some(project), id, "idle".to_string(), json!("created")),
            (Some(project), id, "idle".to_string(), json!("changed")),
            (Some(project), id, "running".to_string(), Value::Null),
            (Some(project), id, "ok".to_string(), Value::Null),
            (Some(project), id, "ok".to_string(), json!("deleted")),
        ]
    );
}
