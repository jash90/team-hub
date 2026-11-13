//! The functions the import relies on in `hub_app::services::issues`: issues written with
//! their original numbers and times, quietly.

use std::sync::atomic::{AtomicU32, Ordering};

use hub_app::services::history::FieldChange;
use hub_app::services::issues::{
    create_issue, import_change, import_comment, import_issue, import_worklog, issue_detail, reindex_issue, ImportedIssue,
};
use hub_app::services::projects::{create_project, NewProject};
use hub_app::services::sprints::{create_sprint, SprintInput};
use hub_app::services::users::{create_user, NewUser};
use hub_app::{access::project_access, Actor, App, AuthUser, Config};
use hub_core::{Id, IssuePatch, NewIssue, Project, Role};

static NEXT: AtomicU32 = AtomicU32::new(0);

fn setup() -> (App, AuthUser, Project) {
    let directory = std::env::temp_dir().join(format!("team-hub-jira-host-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&directory);
    let app = App::new(Config::for_tests(directory), Vec::new()).expect("the test application starts");
    let new =
        NewUser { name: "Ada Admin".to_string(), email: "ada@example.com".to_string(), password: None, role: Some(Role::Admin), github_login: None };
    let user = app.db.write(|tx| create_user(tx, &new)).unwrap();
    let admin = AuthUser { id: user.id, name: user.name, email: user.email, role: Role::Admin, via_token: true };
    let project = NewProject {
        key: "WEB".to_string(),
        name: "Web".to_string(),
        description: String::new(),
        board_mode: None,
        is_private: false,
        lead_id: None,
    };
    let project = app.write(admin.actor(), |tx| create_project(tx, &admin, &project)).unwrap();

    (app, admin, project)
}

fn type_id(project: &Project, name: &str) -> Id {
    project.types.iter().find(|kind| kind.name == name).unwrap().id
}

fn status_id(project: &Project, name: &str) -> Id {
    project.statuses.iter().find(|status| status.name == name).unwrap().id
}

fn imported(project: &Project, number: i64, kind: &str, status: &str, summary: &str) -> ImportedIssue {
    ImportedIssue {
        project_id: project.summary.id,
        number,
        type_id: type_id(project, kind),
        status_id: status_id(project, status),
        priority: 3,
        summary: summary.to_string(),
        created_at: "2024-03-01T09:00:00.000Z".to_string(),
        updated_at: "2024-03-05T09:00:00.000Z".to_string(),
        status_since: "2024-03-04T09:00:00.000Z".to_string(),
        ..Default::default()
    }
}

fn count(app: &App, sql: &str) -> i64 {
    app.read(|connection| Ok(connection.query_row(sql, [], |row| row.get(0))?)).unwrap()
}

#[test]
fn an_imported_issue_keeps_its_number_and_its_times() {
    let (app, admin, project) = setup();
    let events = count(&app, "SELECT count(*) FROM event");
    let issue = ImportedIssue {
        description: "Zażółć gęślą jaźń".to_string(),
        assignee_id: Some(admin.id),
        reporter_id: Some(admin.id),
        points: Some(5.0),
        due_date: Some("2024-03-10T00:00:00.000Z".to_string()),
        estimate_minutes: Some(90),
        labels: vec!["two words".to_string(), "mvp".to_string(), "MVP".to_string()],
        resolved_at: Some("2024-03-04T10:00:00.000Z".to_string()),
        ..imported(&project, 123, "Story", "Done", "  Pay   by card ")
    };
    let id = app.write(Actor::system("Jira"), |tx| import_issue(tx, &issue)).unwrap();
    let detail = issue_detail(&app, &admin, "WEB-123").unwrap();

    assert_eq!((detail.row.id, detail.row.number, detail.row.summary.as_str()), (id, 123, "Pay by card"));
    assert_eq!((detail.row.created_at.as_str(), detail.row.updated_at.as_str()), ("2024-03-01T09:00:00.000Z", "2024-03-05T09:00:00.000Z"));
    assert_eq!(detail.row.resolved_at.as_deref(), Some("2024-03-04T10:00:00.000Z"));
    assert_eq!((detail.row.points, detail.row.due_date.as_deref(), detail.estimate_minutes), (Some(5.0), Some("2024-03-10"), Some(90)));
    assert_eq!(detail.row.labels, vec!["mvp", "two-words"]);
    assert_eq!(count(&app, "SELECT count(*) FROM issue WHERE status_since = '2024-03-04T09:00:00.000Z'"), 1);

    // Quietly: nobody notified, nobody watching, no history, no event.
    assert_eq!(
        count(&app, "SELECT count(*) FROM notification") + count(&app, "SELECT count(*) FROM watcher") + count(&app, "SELECT count(*) FROM change"),
        0
    );
    assert_eq!(count(&app, "SELECT count(*) FROM event"), events);
    // Findable, with its Polish letters folded.
    assert_eq!(count(&app, "SELECT count(*) FROM issue_fts WHERE issue_fts MATCH 'zazolc'"), 1);

    // New issues continue after the highest number, wherever it was imported.
    app.write(Actor::system("Jira"), |tx| import_issue(tx, &imported(&project, 7, "Story", "To Do", "An older one"))).unwrap();
    assert_eq!(count(&app, "SELECT issue_seq FROM project"), 123);

    let new = NewIssue { project_key: "WEB".to_string(), fields: IssuePatch { summary: Some("Made here".to_string()), ..Default::default() } };
    let made = app.write(admin.actor(), |tx| create_issue(tx, &new)).unwrap();

    assert_eq!(count(&app, &format!("SELECT number FROM issue WHERE id = {made}")), 124);

    // Importing a number again overwrites that issue and leaves its place in the order.
    let rank = |app: &App| {
        app.read(|connection| Ok(connection.query_row("SELECT rank FROM issue WHERE number = 123", [], |row| row.get::<_, String>(0))?)).unwrap()
    };
    let before = rank(&app);
    let again = ImportedIssue { labels: vec!["later".to_string()], ..imported(&project, 123, "Bug", "In Progress", "Pay by card, again") };

    assert_eq!(app.write(Actor::system("Jira"), |tx| import_issue(tx, &again)).unwrap(), id);

    let detail = issue_detail(&app, &admin, "WEB-123").unwrap();

    assert_eq!((detail.row.summary.as_str(), detail.row.type_id, detail.row.assignee_id), ("Pay by card, again", type_id(&project, "Bug"), None));
    // Not done any more, so not resolved, whatever the caller passed.
    assert_eq!((detail.row.resolved_at, detail.row.points, detail.row.labels), (None, None, vec!["later".to_string()]));
    assert_eq!(rank(&app), before);
    assert_eq!(count(&app, "SELECT count(*) FROM issue"), 3);
    assert_eq!(count(&app, "SELECT count(*) FROM issue_fts WHERE issue_fts MATCH 'again'"), 1);
}

#[test]
fn an_imported_issue_must_fit_its_project() {
    let (app, admin, project) = setup();
    let import = |issue: ImportedIssue| app.write(Actor::system("Jira"), |tx| import_issue(tx, &issue));
    let other = NewProject {
        key: "OPS".to_string(),
        name: "Ops".to_string(),
        description: String::new(),
        board_mode: None,
        is_private: false,
        lead_id: None,
    };
    let other = app.write(admin.actor(), |tx| create_project(tx, &admin, &other)).unwrap();
    let story = |number: i64| imported(&project, number, "Story", "To Do", "A story");

    for (refused, why) in [
        (ImportedIssue { number: 0, ..story(1) }, "a number below one"),
        (ImportedIssue { summary: "   ".to_string(), ..story(1) }, "no summary"),
        (ImportedIssue { priority: 9, ..story(1) }, "a priority that does not exist"),
        (ImportedIssue { type_id: type_id(&other, "Story"), ..story(1) }, "a type of another project"),
        (ImportedIssue { status_id: status_id(&other, "To Do"), ..story(1) }, "a status of another project"),
        (ImportedIssue { project_id: 999, ..story(1) }, "a project that does not exist"),
        (ImportedIssue { sprint_id: Some(999), ..story(1) }, "a sprint that does not exist"),
        (ImportedIssue { component_ids: vec![999], ..story(1) }, "a component that does not exist"),
        (ImportedIssue { due_date: Some("soon".to_string()), ..story(1) }, "a date that is none"),
    ] {
        assert!(import(refused).is_err(), "{why}");
    }

    // A refused issue leaves nothing behind.
    assert_eq!(count(&app, "SELECT count(*) FROM issue") + count(&app, "SELECT issue_seq FROM project WHERE key = 'WEB'"), 0);

    // Epics hold stories, stories hold subtasks, and nothing else goes.
    let epic = import(imported(&project, 1, "Epic", "To Do", "An epic")).unwrap();
    let parent = import(ImportedIssue { parent_id: Some(epic), ..story(2) }).unwrap();
    let subtask = import(ImportedIssue { parent_id: Some(parent), ..imported(&project, 3, "Subtask", "To Do", "A part") }).unwrap();

    assert_eq!(count(&app, &format!("SELECT parent_id FROM issue WHERE id = {subtask}")), parent);
    assert!(import(ImportedIssue { parent_id: Some(parent), ..story(4) }).is_err());
    assert!(import(ImportedIssue { parent_id: Some(epic), ..imported(&project, 5, "Subtask", "To Do", "A part") }).is_err());
    assert!(import(ImportedIssue { parent_id: Some(parent), ..imported(&project, 6, "Epic", "To Do", "An epic") }).is_err());
    assert!(import(ImportedIssue { parent_id: Some(parent), ..story(2) }).is_err(), "an issue is not its own parent");
    // A subtask may arrive before its parent; the caller sets the parent once it is there.
    assert!(import(imported(&project, 7, "Subtask", "To Do", "An early part")).is_ok());

    // Unlike an issue made by hand, an imported one may sit in a completed sprint.
    let access = app.read(|connection| project_access(connection, &admin, "WEB")).unwrap();
    let sprint = app.write(admin.actor(), |tx| create_sprint(tx, &access, &SprintInput::default())).unwrap();

    app.db.write(|tx| Ok(tx.execute("UPDATE sprint SET state = 'closed' WHERE id = ?1", [sprint.id])?)).unwrap();
    assert!(import(ImportedIssue { sprint_id: Some(sprint.id), ..imported(&project, 8, "Story", "Done", "Finished long ago") }).is_ok());
    // Done without a resolution time: resolved when it entered its status.
    assert_eq!(count(&app, "SELECT count(*) FROM issue WHERE number = 8 AND resolved_at = status_since"), 1);
}

#[test]
fn comments_changes_and_work_logs_are_written_as_they_happened() {
    let (app, admin, project) = setup();
    let events = count(&app, "SELECT count(*) FROM event");
    let id = app.write(Actor::system("Jira"), |tx| import_issue(tx, &imported(&project, 1, "Story", "In Progress", "A story"))).unwrap();
    let (todo, doing) = (status_id(&project, "To Do"), status_id(&project, "In Progress"));

    app.write(Actor::system("Jira"), |tx| {
        let first =
            import_comment(tx, id, &Actor::user(admin.id), "  Looks good on staging. ", "2024-03-02T10:00:00.000Z", "2024-03-02T11:00:00.000Z")?;
        let second = import_comment(tx, id, &Actor::system("Jira · Olaf"), "Merged.", "2024-03-03T10:00:00.000Z", "2024-03-03T10:00:00.000Z")?;

        assert!(second > first);
        assert!(import_comment(tx, id, &Actor::user(admin.id), "   ", "2024-03-03T10:00:00.000Z", "2024-03-03T10:00:00.000Z").is_err());

        let moved = FieldChange::reference("status", Some((todo, "To Do".to_string())), Some((doing, "In Progress".to_string())));

        import_change(tx, id, &Actor::user(admin.id), &moved, "2024-03-04T09:00:00.000Z")?;
        import_change(
            tx,
            id,
            &Actor::system("Jira"),
            &FieldChange::text("points", Some("3".to_string()), Some("5".to_string())),
            "2024-03-04T09:30:00.000Z",
        )?;
        import_worklog(tx, id, Some(admin.id), 90, "2024-03-04T08:00:00.000Z", " Cart endpoints ")?;
        assert!(import_worklog(tx, id, None, 0, "2024-03-04T08:00:00.000Z", "").is_err());

        reindex_issue(tx, id)
    })
    .unwrap();

    let detail = issue_detail(&app, &admin, "WEB-1").unwrap();

    assert_eq!(
        detail
            .comments
            .iter()
            .map(|comment| (comment.author_id, comment.author_label.as_deref(), comment.body.as_str(), comment.created_at.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (Some(admin.id), None, "Looks good on staging.", "2024-03-02T10:00:00.000Z"),
            (None, Some("Jira · Olaf"), "Merged.", "2024-03-03T10:00:00.000Z")
        ]
    );
    assert_eq!(detail.comments[0].updated_at, "2024-03-02T11:00:00.000Z");
    assert_eq!(
        detail
            .changes
            .iter()
            .map(|change| (change.field.as_str(), change.old_value.as_deref(), change.new_value.as_deref(), change.created_at.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("status", Some("To Do"), Some("In Progress"), "2024-03-04T09:00:00.000Z"),
            ("points", Some("3"), Some("5"), "2024-03-04T09:30:00.000Z")
        ]
    );
    assert_eq!((detail.changes[0].actor_id, detail.changes[1].actor_label.as_deref()), (Some(admin.id), Some("Jira")));
    assert_eq!(count(&app, &format!("SELECT count(*) FROM change WHERE old_ref = '{todo}' AND new_ref = '{doing}'")), 1);
    assert_eq!(
        detail.worklogs.iter().map(|worklog| (worklog.minutes, worklog.note.as_str(), worklog.started_at.as_str())).collect::<Vec<_>>(),
        vec![(90, "Cart endpoints", "2024-03-04T08:00:00.000Z")]
    );

    // The issue itself was not touched, and nobody was told.
    assert_eq!(detail.row.updated_at, "2024-03-05T09:00:00.000Z");
    assert_eq!(count(&app, "SELECT count(*) FROM event"), events);
    assert_eq!(count(&app, "SELECT count(*) FROM notification") + count(&app, "SELECT count(*) FROM watcher"), 0);
    assert_eq!(count(&app, "SELECT count(*) FROM issue_fts WHERE issue_fts MATCH 'staging'"), 1);
}
