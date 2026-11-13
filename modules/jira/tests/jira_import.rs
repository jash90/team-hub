mod common;

use common::{connected, fake, local, small, Setup};
use hub_app::db::{query_all, query_one};
use hub_app::services::issues::{add_comment, create_issue, issue_detail, update_issue};
use hub_app::services::projects::{create_project, get_project, NewProject};
use hub_app::App;
use hub_core::{IssuePatch, NewIssue, SprintState, StatusCategory, TypeLevel};
use serde_json::{json, Value};

fn count(app: &App, sql: &str) -> i64 {
    app.read(|connection| Ok(connection.query_row(sql, [], |row| row.get(0))?)).unwrap()
}

fn text(app: &App, sql: &str) -> Option<String> {
    app.read(|connection| query_one(connection, sql, [], |row| row.get(0))).unwrap().flatten()
}

fn keys(app: &App) -> Vec<String> {
    app.read(|connection| query_all(connection, "SELECT key FROM issue ORDER BY number", [], |row| row.get(0))).unwrap()
}

type Entry = (String, Option<String>, Option<String>, String, Option<String>);

/// An issue's history as written: field, old, new, when, and who (a name, or a label).
fn history(app: &App, key: &str) -> Vec<Entry> {
    app.read(|connection| {
        query_all(
            connection,
            "SELECT c.field, c.old_value, c.new_value, c.created_at, coalesce(u.name, c.actor_label)
             FROM change c JOIN issue i ON i.id = c.issue_id LEFT JOIN user u ON u.id = c.actor_id WHERE i.key = ?1 ORDER BY c.id",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
    })
    .unwrap()
}

fn step(field: &str, old: Option<&str>, new: Option<&str>, at: &str, who: &str) -> Entry {
    (field.to_string(), old.map(String::from), new.map(String::from), format!("2024-{at}:00.000Z"), Some(who.to_string()))
}

/// Summary, status, type, parent key, sprint name, resolved_at of an issue.
fn state(app: &App, key: &str) -> (String, String, String, Option<String>, Option<String>, Option<String>) {
    app.read(|connection| {
        Ok(connection.query_row(
            "SELECT i.summary, s.name, t.name, p.key, sp.name, i.resolved_at FROM issue i
             JOIN status s ON s.id = i.status_id JOIN issue_type t ON t.id = i.type_id
             LEFT JOIN issue p ON p.id = i.parent_id LEFT JOIN sprint sp ON sp.id = i.sprint_id WHERE i.key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )?)
    })
    .unwrap()
}

fn issue_state(summary: &str, status: &str, kind: &str, parent: Option<&str>, sprint: Option<&str>, resolved: Option<&str>) -> (String, String, String, Option<String>, Option<String>, Option<String>) {
    (summary.to_string(), status.to_string(), kind.to_string(), parent.map(String::from), sprint.map(String::from), resolved.map(|at| format!("2024-{at}:00.000Z")))
}

fn notes(job: &Value, list: &str) -> Vec<(String, String)> {
    job["report"][list].as_array().unwrap().iter().map(|note| (note["key"].as_str().unwrap().to_string(), note["reason"].as_str().unwrap().to_string())).collect()
}

/// The import of the small project, checked from every side. The same for both flavours:
/// what differs is how Jira is asked and what it answers with.
async fn brings_the_project_over(cloud: bool) {
    let Setup { app, admin, member, fake, .. } = connected(small(cloud)).await;

    fake.take_requests();

    let job = admin.import(json!({ "projectKey": "web" })).await;
    let report = &job["report"];

    assert_eq!(job["kind"], "api");
    assert_eq!((job["step"].as_str(), job["processed"].as_i64(), job["total"].as_i64()), (Some("finished"), Some(7), Some(7)), "{job}");
    assert_eq!(report["projectCreated"], true);
    assert_eq!((report["created"].as_i64(), report["updated"].as_i64(), report["unchanged"].as_i64()), (Some(7), Some(0), Some(0)), "{report}");
    assert_eq!((notes(&job, "failed"), notes(&job, "skipped")), (vec![], vec![]));
    assert_eq!(report["comments"], 3);
    assert_eq!(report["worklogs"], 1);
    assert_eq!(report["changes"], 17);
    assert_eq!(report["links"], 2);
    assert_eq!((report["sprints"].as_i64(), report["versions"].as_i64(), report["components"].as_i64()), (Some(3), Some(2), Some(2)));
    assert_eq!(report["usersMatched"], 2);
    assert_eq!(report["usersCreated"], json!(["Zofia Zewnętrzna", "Olaf Odszedł"]));
    assert_eq!(
        report["unmapped"],
        json!([
            "Links of the type “Cloners” were imported as “relates to”",
            "1 links point to issues that are not here (other projects, or issues that were not imported)"
        ])
    );

    // Jira was asked the way its flavour wants: v3 with a page token, or v2 with an offset.
    let requests = fake.take_requests();
    let searches: Vec<&String> = requests.iter().filter(|request| request.contains("/search")).collect();

    if cloud {
        assert!(requests.iter().all(|request| !request.contains("/rest/api/2/") || request.ends_with("/serverInfo") || request.ends_with("/field")), "{requests:?}");
        assert_eq!(searches.len(), 4, "{searches:?}");
        assert!(searches[0].starts_with("POST /rest/api/3/search/approximate-count"));
        assert!(searches[1].starts_with("GET /rest/api/3/search/jql?") && !searches[1].contains("nextPageToken"));
        assert!(searches[2].contains("nextPageToken=page-3") && searches[3].contains("nextPageToken=page-6"));
    } else {
        assert!(requests.iter().all(|request| !request.contains("/rest/api/3/")), "{requests:?}");
        assert_eq!(searches.len(), 4, "{searches:?}");
        assert!(searches[0].contains("maxResults=0"));
        assert!(searches[1].starts_with("GET /rest/api/2/search?") && searches[1].contains("startAt=0"));
        assert!(searches[2].contains("startAt=3") && searches[3].contains("startAt=6"));
    }

    assert!(searches.iter().skip(1).all(|search| search.contains("expand=changelog") && search.contains("ORDER+BY+key")), "{searches:?}");

    // The project: same key, Jira's statuses in workflow order (Jira lists them otherwise),
    // its types, components, versions and sprints.
    let project = app.read(|connection| get_project(connection, &admin.user, "WEB")).unwrap();

    assert_eq!((project.summary.name.as_str(), project.summary.issue_count), ("Web shop", 7));
    assert_eq!(project.summary.lead_id, Some(admin.user.id));
    assert_eq!(
        project.statuses.iter().map(|status| (status.name.as_str(), status.category)).collect::<Vec<_>>(),
        vec![
            ("Open", StatusCategory::Todo),
            ("In Progress", StatusCategory::InProgress),
            ("In Review", StatusCategory::InProgress),
            ("Done", StatusCategory::Done)
        ]
    );
    assert_eq!(
        project.types.iter().map(|kind| (kind.name.as_str(), kind.level)).collect::<Vec<_>>(),
        vec![("Epic", TypeLevel::Epic), ("Story", TypeLevel::Standard), ("Bug", TypeLevel::Standard), ("Subtask", TypeLevel::Subtask)]
    );
    assert_eq!(project.components.iter().map(|component| component.name.as_str()).collect::<Vec<_>>(), vec!["Cart", "Payments"]);

    let released = project.versions.iter().find(|version| version.name == "1.0").unwrap();
    let planned = project.versions.iter().find(|version| version.name == "1.1").unwrap();

    assert_eq!((released.released, released.release_date.as_deref(), released.start_date.as_deref()), (true, Some("2024-03-15"), Some("2024-03-01")));
    assert_eq!(released.released_at.as_deref(), Some("2024-03-15T12:00:00.000Z"));
    assert_eq!((planned.released, planned.release_date.as_deref(), planned.released_at.as_deref()), (false, Some("2024-04-30"), None));
    assert_eq!(
        project.sprints.iter().map(|sprint| (sprint.name.as_str(), sprint.state, sprint.start_date.as_deref(), sprint.end_date.as_deref())).collect::<Vec<_>>(),
        vec![
            ("Sprint 1", SprintState::Closed, Some("2024-03-01"), Some("2024-03-15")),
            ("Sprint 2", SprintState::Active, Some("2024-03-18"), Some("2024-04-01")),
            ("Sprint 3", SprintState::Future, None, None),
        ]
    );
    assert_eq!(project.sprints[0].completed_at.as_deref(), Some("2024-03-15T15:00:00.000Z"));
    assert_eq!(project.sprints[0].goal, "A cart that works");

    // Keys and numbers are Jira's, gaps included, and new issues continue after them.
    assert_eq!(keys(&app), vec!["WEB-1", "WEB-2", "WEB-3", "WEB-5", "WEB-7", "WEB-8", "WEB-9"]);
    assert_eq!(count(&app, "SELECT issue_seq FROM project WHERE key = 'WEB'"), 9);

    // Epic → story → subtask, also when the epic comes after its story (WEB-8 under WEB-9).
    assert_eq!(state(&app, "WEB-1"), issue_state("Checkout", "In Progress", "Epic", None, None, None));
    assert_eq!(state(&app, "WEB-2"), issue_state("Cart", "Done", "Story", Some("WEB-1"), Some("Sprint 1"), Some("03-08T11:00")));
    assert_eq!(state(&app, "WEB-3"), issue_state("Cart API", "In Review", "Subtask", Some("WEB-2"), Some("Sprint 2"), None));
    assert_eq!(state(&app, "WEB-5"), issue_state("Rounding", "In Progress", "Bug", Some("WEB-1"), Some("Sprint 2"), None));
    assert_eq!(state(&app, "WEB-7"), issue_state("Refunds", "Open", "Story", None, None, None));
    assert_eq!(state(&app, "WEB-8"), issue_state("Wishlist", "Done", "Story", Some("WEB-9"), None, Some("03-20T11:00")));
    assert_eq!(state(&app, "WEB-9"), issue_state("Account", "Open", "Epic", None, None, None));

    // People: matched by e-mail (whatever its case), or created switched off and without a password.
    let people: Vec<(String, String, bool, bool)> = app
        .read(|connection| {
            query_all(connection, "SELECT name, email, active, password_hash IS NULL FROM user ORDER BY id", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
        })
        .unwrap();
    let placeholder = if cloud { "jira-acc-zofia@imported.invalid" } else { "jira-zofia@imported.invalid" };

    assert_eq!(people.len(), 4);
    assert_eq!(people[2], ("Zofia Zewnętrzna".to_string(), placeholder.to_string(), false, true));
    assert_eq!(people[3], ("Olaf Odszedł".to_string(), "olaf@old.example.com".to_string(), false, true));

    // One issue in full.
    let cart = issue_detail(&app, &admin.user, "WEB-2").unwrap();

    assert_eq!((cart.row.assignee_id, cart.row.reporter_id), (Some(member.user.id), Some(admin.user.id)));
    assert_eq!((cart.row.priority, cart.row.points, cart.row.due_date.as_deref(), cart.estimate_minutes), (3, Some(5.0), Some("2024-03-10"), Some(480)));
    assert_eq!(cart.row.labels, vec!["checkout", "mvp"]);
    assert_eq!(cart.row.component_ids.len(), 1);
    assert_eq!(cart.row.version_id, Some(released.id));
    assert_eq!((cart.row.created_at.as_str(), cart.row.updated_at.as_str()), ("2024-02-21T08:00:00.000Z", "2024-03-08T11:00:00.000Z"));
    assert_eq!(text(&app, "SELECT status_since FROM issue WHERE key = 'WEB-2'").as_deref(), Some("2024-03-08T11:00:00.000Z"));
    assert_eq!(
        cart.description,
        format!(
            "## Goal\n\nA **better** cart\n\n- fast\n- simple\n\n## Attachments in Jira\n\n- [spec.pdf]({}/secure/attachment/100020/spec.pdf) (200 KB)",
            fake.base_url
        )
    );
    assert_eq!(
        history(&app, "WEB-2"),
        vec![
            step("sprint", None, Some("Sprint 1"), "03-01T08:30", "Ada Admin"),
            step("points", Some("3"), Some("5"), "03-02T08:00", "Ada Admin"),
            step("assignee", None, Some("Marek Member"), "03-03T08:00", "Ada Admin"),
            step("status", Some("Open"), Some("In Progress"), "03-04T09:00", "Marek Member"),
            step("status", Some("In Progress"), Some("In Review"), "03-06T09:00", "Marek Member"),
            step("status", Some("In Review"), Some("Done"), "03-08T11:00", "Ada Admin"),
        ]
    );

    // Status and sprint changes carry the ids the reports read.
    let refs: Vec<(String, Option<String>, Option<String>)> = app
        .read(|connection| {
            query_all(
                connection,
                "SELECT c.field, (SELECT name FROM status WHERE id = c.old_ref), (SELECT name FROM status WHERE id = c.new_ref)
                 FROM change c JOIN issue i ON i.id = c.issue_id WHERE i.key = 'WEB-2' AND c.field = 'status' ORDER BY c.id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .unwrap();

    assert_eq!(refs.iter().map(|(_, old, new)| (old.as_deref(), new.as_deref())).collect::<Vec<_>>(), vec![
        (Some("Open"), Some("In Progress")),
        (Some("In Progress"), Some("In Review")),
        (Some("In Review"), Some("Done"))
    ]);
    assert_eq!(count(&app, "SELECT count(*) FROM change WHERE field = 'sprint' AND new_value IS NOT NULL AND new_ref NOT IN (SELECT id FROM sprint)"), 0);
    assert_eq!(count(&app, "SELECT count(*) FROM change WHERE field = 'assignee' AND new_ref IN (SELECT id FROM user)"), 1);

    // Comments keep their authors and times; work logs too.
    let zofia = people_id(&app, "Zofia Zewnętrzna");

    assert_eq!(
        cart.comments.iter().map(|comment| (comment.author_id, comment.body.as_str(), comment.created_at.as_str())).collect::<Vec<_>>(),
        vec![(Some(zofia), "Looks good on staging.", "2024-03-05T14:30:00.000Z"), (Some(admin.user.id), "Merged.", "2024-03-07T07:00:00.000Z")]
    );
    assert_eq!(
        cart.worklogs.iter().map(|worklog| (worklog.user_id, worklog.minutes, worklog.started_at.as_str(), worklog.note.as_str())).collect::<Vec<_>>(),
        vec![(Some(member.user.id), 90, "2024-03-05T08:00:00.000Z", "Cart endpoints")]
    );
    assert_eq!(cart.spent_minutes, 90);
    assert!(cart.attachments.is_empty());

    // The panel that leads back to the original.
    let panel = &cart.panels["jira"];

    assert_eq!((panel["key"].as_str(), panel["url"].as_str()), (Some("WEB-2"), Some(format!("{}/browse/WEB-2", fake.base_url).as_str())));
    assert_eq!(panel["remoteUpdated"], "2024-03-08T11:00:00.000Z");
    assert_eq!(
        text(&app, "SELECT self_url FROM jira_issue WHERE remote_key = 'WEB-2'"),
        Some(format!("{}/rest/api/{}/issue/10002", fake.base_url, if cloud { 3 } else { 2 }))
    );
    assert_eq!(issue_detail(&app, &admin.user, "WEB-7").unwrap().panels["jira"]["key"], "WEB-7");

    // A sprint that ended with the issue unfinished: Jira just keeps listing it; here the
    // issue goes back to the backlog at that moment, or on to the sprint it was moved to.
    assert_eq!(
        history(&app, "WEB-5"),
        vec![
            step("sprint", None, Some("Sprint 1"), "03-01T08:30", "Ada Admin"),
            step("status", Some("Open"), Some("In Progress"), "03-10T09:00", "Olaf Odszedł"),
            step("sprint", Some("Sprint 1"), None, "03-15T15:00", "Jira"),
            step("sprint", None, Some("Sprint 2"), "03-18T08:30", "Ada Admin"),
        ]
    );
    assert_eq!(
        history(&app, "WEB-8"),
        vec![
            step("sprint", None, Some("Sprint 1"), "03-01T08:30", "Ada Admin"),
            step("status", Some("Open"), Some("In Progress"), "03-12T09:00", "Marek Member"),
            step("sprint", Some("Sprint 1"), None, "03-15T15:00", "Jira"),
            step("status", Some("In Progress"), Some("Done"), "03-20T11:00", "Marek Member"),
        ]
    );
    assert_eq!(text(&app, "SELECT status_since FROM issue WHERE key = 'WEB-9'").as_deref(), Some("2024-03-03T08:00:00.000Z"));
    assert_eq!(count(&app, "SELECT priority FROM issue WHERE key = 'WEB-5'"), 1);
    assert_eq!(count(&app, "SELECT priority FROM issue WHERE key = 'WEB-7'"), 4);

    // Links: listed on both issues in Jira, stored once here.
    let links: Vec<(String, String, String)> = app
        .read(|connection| {
            query_all(
                connection,
                "SELECT s.key, l.type, t.key FROM issue_link l JOIN issue s ON s.id = l.source_id JOIN issue t ON t.id = l.target_id ORDER BY s.number",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .unwrap();

    assert_eq!(links, vec![("WEB-5".to_string(), "blocks".to_string(), "WEB-7".to_string()), ("WEB-7".to_string(), "relates".to_string(), "WEB-2".to_string())]);

    // An import is not something that happens to people: nobody is notified, nobody is
    // made a watcher, and the journal has the import's own entries, not one per issue.
    assert_eq!(count(&app, "SELECT count(*) FROM notification"), 0);
    assert_eq!(count(&app, "SELECT count(*) FROM watcher"), 0);
    assert_eq!(count(&app, "SELECT count(*) FROM event WHERE kind LIKE 'issue.%' OR kind LIKE 'comment.%' OR kind LIKE 'worklog.%'"), 0);
    assert!(count(&app, "SELECT count(*) FROM event WHERE kind = 'jira.job'") >= 5);

    // The imported text is searchable.
    assert_eq!(count(&app, "SELECT count(*) FROM issue_fts WHERE issue_fts MATCH 'staging'"), 1);

    // New work continues after Jira's numbers.
    let new = NewIssue { project_key: "WEB".to_string(), fields: IssuePatch { summary: Some("First issue made here".to_string()), ..Default::default() } };
    let id = app.write(admin.user.actor(), |tx| create_issue(tx, &new)).unwrap();

    assert_eq!(text(&app, &format!("SELECT key FROM issue WHERE id = {id}")).as_deref(), Some("WEB-10"));
}

fn people_id(app: &App, name: &str) -> i64 {
    app.read(|connection| Ok(connection.query_row("SELECT id FROM user WHERE name = ?1", [name], |row| row.get(0))?)).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cloud_project_comes_over_with_keys_history_and_people() {
    brings_the_project_over(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_project_comes_over_with_keys_history_and_people() {
    brings_the_project_over(false).await;
}

fn totals(app: &App) -> [i64; 7] {
    ["issue", "comment", "change", "worklog", "issue_link", "user", "sprint"].map(|table| count(app, &format!("SELECT count(*) FROM {table}")))
}

fn counts(job: &Value) -> (i64, i64, i64, usize, usize) {
    let report = &job["report"];
    let number = |name: &str| report[name].as_i64().unwrap_or(-1);

    (number("created"), number("updated"), number("unchanged"), report["skipped"].as_array().map_or(99, Vec::len), report["failed"].as_array().map_or(99, Vec::len))
}

async fn imports_again_without_duplicates(cloud: bool) {
    let Setup { app, admin, fake, .. } = connected(small(cloud)).await;

    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB" })).await), (7, 0, 0, 0, 0));

    let before = totals(&app);
    let ranks: Vec<String> = app.read(|connection| query_all(connection, "SELECT rank FROM issue ORDER BY number", [], |row| row.get(0))).unwrap();

    // Nothing changed in Jira: only what was touched at or after the last known change is
    // asked for, and nothing is written twice.
    fake.take_requests();

    let again = admin.import(json!({ "projectKey": "WEB" })).await;
    let requests = fake.take_requests();

    assert_eq!(counts(&again), (0, 0, 1, 0, 0));
    assert_eq!(again["report"]["projectCreated"], false);
    assert!(requests.iter().any(|request| request.contains("/search") && request.contains("updated+%3E%3D+%222024-03-20+12%3A00%22")), "{requests:?}");
    assert_eq!(totals(&app), before);

    // A full run reads everything again and still changes nothing.
    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB", "full": true })).await), (0, 0, 7, 0, 0));
    assert_eq!(totals(&app), before);

    // Jira moves on: an issue is edited, moved and commented on, another is new.
    fake.with(|upstream| {
        let author = upstream.users[1].account.clone();
        let refunds = upstream.issue("WEB-7");

        refunds.summary = "Refunds and returns".to_string();
        refunds.status = "In Progress".to_string();
        refunds.updated = local("03-25T10:00");
        refunds.history.push(fake::FakeHistory {
            id: 701,
            author: author.clone(),
            created: local("03-25T10:00"),
            items: vec![
                ("summary".to_string(), None, Some("Refunds".to_string()), None, Some("Refunds and returns".to_string())),
                ("status".to_string(), Some("10002".to_string()), Some("Open".to_string()), Some("10001".to_string()), Some("In Progress".to_string())),
            ],
        });
        refunds.comments.push(fake::FakeComment {
            id: 904,
            author: author.clone(),
            body: fake::Text::plain("Starting with card refunds."),
            created: local("03-25T10:05"),
            updated: local("03-25T10:05"),
        });

        upstream.issues.push(fake::FakeIssue {
            key: "WEB-10".to_string(),
            kind: "Bug".to_string(),
            status: "Open".to_string(),
            summary: "Empty cart shows an error".to_string(),
            reporter: Some(author),
            parent: Some("WEB-1".to_string()),
            created: local("03-26T09:00"),
            updated: local("03-26T09:00"),
            ..Default::default()
        });
    });

    let third = admin.import(json!({ "projectKey": "WEB" })).await;

    assert_eq!(counts(&third), (1, 1, 1, 0, 0), "{third}");
    assert_eq!((third["report"]["comments"].as_i64(), third["report"]["changes"].as_i64()), (Some(1), Some(2)));
    assert_eq!(state(&app, "WEB-7"), issue_state("Refunds and returns", "In Progress", "Story", None, None, None));
    assert_eq!(
        history(&app, "WEB-7"),
        vec![
            step("summary", Some("Refunds"), Some("Refunds and returns"), "03-25T09:00", "Marek Member"),
            step("status", Some("Open"), Some("In Progress"), "03-25T09:00", "Marek Member"),
        ]
    );
    assert_eq!(text(&app, "SELECT status_since FROM issue WHERE key = 'WEB-7'").as_deref(), Some("2024-03-25T09:00:00.000Z"));
    assert_eq!(state(&app, "WEB-10").3.as_deref(), Some("WEB-1"));
    assert_eq!(totals(&app), [before[0] + 1, before[1] + 1, before[2] + 2, before[3], before[4], before[5], before[6]]);

    // The issues that were there keep their place in the order; the new one goes to the end.
    let after: Vec<String> = app.read(|connection| query_all(connection, "SELECT rank FROM issue ORDER BY number", [], |row| row.get(0))).unwrap();

    assert_eq!(after[..7], ranks[..]);
    assert!(after[7] > *ranks.iter().max().unwrap());

    // And once more: the same state, the same rows.
    let settled = totals(&app);

    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB", "full": true })).await), (0, 0, 8, 0, 0));
    assert_eq!(totals(&app), settled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_import_updates_what_changed_and_duplicates_nothing_cloud() {
    imports_again_without_duplicates(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_import_updates_what_changed_and_duplicates_nothing_server() {
    imports_again_without_duplicates(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn issues_edited_here_are_not_overwritten_unless_asked() {
    let Setup { app, admin, fake, .. } = connected(small(true)).await;

    admin.import(json!({ "projectKey": "WEB" })).await;

    // Somebody triages WEB-5 here, and comments on WEB-2 (which changes no field).
    let id_of = |key: &str| count(&app, &format!("SELECT id FROM issue WHERE key = '{key}'"));
    let patch = IssuePatch { summary: Some("Rounding (triaged)".to_string()), priority: Some(2), ..Default::default() };

    app.write(admin.user.actor(), |tx| update_issue(tx, id_of("WEB-5"), &patch)).unwrap();
    app.write(admin.user.actor(), |tx| add_comment(tx, id_of("WEB-2"), "Checked again here.")).unwrap();

    // Jira unchanged: a full run leaves the edit alone and has nothing to report.
    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB", "full": true })).await), (0, 0, 7, 0, 0));
    assert_eq!(state(&app, "WEB-5").0, "Rounding (triaged)");

    // Both change in Jira too.
    fake.with(|upstream| {
        upstream.issue("WEB-5").summary = "Rounding error in totals".to_string();
        upstream.issue("WEB-5").updated = local("03-27T10:00");
        upstream.issue("WEB-2").summary = "Cart, reworked".to_string();
        upstream.issue("WEB-2").updated = local("03-27T11:00");
    });

    let job = admin.import(json!({ "projectKey": "WEB" })).await;

    assert_eq!(counts(&job), (0, 1, 0, 1, 0), "{job}");
    assert_eq!(notes(&job, "skipped"), vec![("WEB-5".to_string(), "Edited here after it was imported; the newer version in Jira was not applied".to_string())]);
    assert_eq!((state(&app, "WEB-5").0.as_str(), count(&app, "SELECT priority FROM issue WHERE key = 'WEB-5'")), ("Rounding (triaged)", 2));
    // A comment made here is not an edit of the issue, and it stays.
    assert_eq!(state(&app, "WEB-2").0, "Cart, reworked");
    assert_eq!(count(&app, "SELECT count(*) FROM comment c JOIN issue i ON i.id = c.issue_id WHERE i.key = 'WEB-2'"), 3);

    // It is skipped every time, until the administrator says Jira wins.
    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB", "full": true })).await), (0, 0, 6, 1, 0));

    let forced = admin.import(json!({ "projectKey": "WEB", "full": true, "overwriteLocal": true })).await;

    assert_eq!(counts(&forced), (0, 1, 6, 0, 0), "{forced}");
    assert_eq!((state(&app, "WEB-5").0.as_str(), count(&app, "SELECT priority FROM issue WHERE key = 'WEB-5'")), ("Rounding error in totals", 1));
    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB", "full": true })).await), (0, 0, 7, 0, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_existing_project_is_only_written_into_when_asked() {
    let Setup { app, admin, .. } = connected(small(false)).await;

    // A project with Jira's key is already here, with an issue of its own under WEB-1.
    let new = NewProject { key: "WEB".to_string(), name: "Our web".to_string(), description: String::new(), board_mode: None, is_private: false, lead_id: None };
    let project = app.write(admin.user.actor(), |tx| create_project(tx, &admin.user, &new)).unwrap();
    let epic = project.types.iter().find(|kind| kind.name == "Epic").unwrap().id;
    let own = NewIssue { project_key: "WEB".to_string(), fields: IssuePatch { summary: Some("Our own epic".to_string()), type_id: Some(epic), ..Default::default() } };

    app.write(admin.user.actor(), |tx| create_issue(tx, &own)).unwrap();

    let refused = admin.finished(&admin.post("/jira/import", json!({ "projectKey": "WEB" })).await).await;

    assert_eq!(refused["state"], "failed");
    assert!(refused["error"].as_str().unwrap().contains("already exists here"), "{refused}");
    assert_eq!(keys(&app), vec!["WEB-1"]);

    let job = admin.import(json!({ "projectKey": "WEB", "intoExisting": true })).await;

    assert_eq!(counts(&job), (6, 0, 0, 1, 0), "{job}");
    assert_eq!(notes(&job, "skipped"), vec![("WEB-1".to_string(), "An issue with this key already exists here and did not come from Jira".to_string())]);
    assert_eq!(job["report"]["projectCreated"], false);
    assert_eq!(state(&app, "WEB-1").0, "Our own epic");
    assert_eq!(state(&app, "WEB-2").3.as_deref(), Some("WEB-1"));

    // The project keeps its own statuses and gains the ones it lacked.
    let project = app.read(|connection| get_project(connection, &admin.user, "WEB")).unwrap();

    assert_eq!(project.summary.name, "Our web");
    assert_eq!(project.statuses.iter().map(|status| status.name.as_str()).collect::<Vec<_>>(), vec!["To Do", "In Progress", "In Review", "Done", "Open"]);
    assert!(project.types.iter().any(|kind| kind.name == "Task"));

    // Once imported, importing again needs no permission, and Jira's WEB-1 still does not
    // replace the local one unless that is asked for.
    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB", "full": true })).await), (0, 0, 6, 1, 0));
    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB", "full": true, "overwriteLocal": true })).await), (0, 1, 6, 0, 0));
    assert_eq!(state(&app, "WEB-1").0, "Checkout");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_mapping_decides_what_things_become() {
    let Setup { app, admin, member, .. } = connected(small(true)).await;
    let mapping = json!({
        "boardMode": "kanban",
        "types": [{ "jiraName": "Bug", "name": "Defect", "level": "standard" }],
        "statuses": [
            { "jiraName": "Open", "name": "To Do", "category": "todo" },
            { "jiraName": "In Review", "name": "In Progress", "category": "in_progress" },
        ],
        "priorities": [{ "jiraName": "Highest", "priority": 2 }],
        "users": [{ "account": "acc-zofia", "userId": member.user.id }],
        "createUsers": false,
    });
    let job = admin.import(json!({ "projectKey": "WEB", "mapping": mapping })).await;

    assert_eq!(counts(&job), (7, 0, 0, 0, 0), "{job}");
    assert_eq!(job["report"]["usersCreated"], json!([]));

    let project = app.read(|connection| get_project(connection, &admin.user, "WEB")).unwrap();

    assert_eq!(project.summary.board_mode, hub_core::BoardMode::Kanban);
    assert_eq!(project.statuses.iter().map(|status| status.name.as_str()).collect::<Vec<_>>(), vec!["To Do", "In Progress", "Done"]);
    assert!(project.types.iter().any(|kind| kind.name == "Defect") && !project.types.iter().any(|kind| kind.name == "Bug"));

    // Two of Jira's statuses share one here: the move between them is no move.
    assert_eq!(state(&app, "WEB-3").1, "In Progress");
    assert_eq!(history(&app, "WEB-3"), vec![step("status", Some("To Do"), Some("In Progress"), "03-19T09:00", "Marek Member")]);
    assert_eq!(text(&app, "SELECT status_since FROM issue WHERE key = 'WEB-3'").as_deref(), Some("2024-03-20T09:00:00.000Z"));
    assert_eq!((state(&app, "WEB-5").2.as_str(), count(&app, "SELECT priority FROM issue WHERE key = 'WEB-5'")), ("Defect", 2));

    // Zofia is the member, by the administrator's choice; Olaf has no account and gets
    // none, but his name stays on what he did.
    assert_eq!(count(&app, "SELECT count(*) FROM user"), 2);
    assert_eq!(count(&app, "SELECT assignee_id FROM issue WHERE key = 'WEB-3'"), member.user.id);
    assert_eq!(count(&app, "SELECT count(*) FROM issue WHERE key = 'WEB-5' AND assignee_id IS NULL"), 1);
    assert_eq!(text(&app, "SELECT c.author_label FROM comment c JOIN issue i ON i.id = c.issue_id WHERE i.key = 'WEB-5'").as_deref(), Some("Jira · Olaf Odszedł"));
    assert_eq!(history(&app, "WEB-5")[1].4.as_deref(), Some("Jira · Olaf Odszedł"));

    // The mapping is remembered: the next preview proposes it again.
    let preview = admin.get("/jira/projects/WEB/preview").await;
    let status_of = |name: &str| preview["mapping"]["statuses"].as_array().unwrap().iter().find(|entry| entry["jiraName"] == name).unwrap()["name"].clone();

    assert_eq!((status_of("Open"), status_of("In Review"), status_of("Done")), (json!("To Do"), json!("In Progress"), json!("Done")));
    assert_eq!(preview["mapping"]["createUsers"], false);
    assert_eq!(preview["mapping"]["boardMode"], "kanban");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_agile_api_sprints_come_from_the_issues() {
    let mut upstream = small(false);
    upstream.agile = false;

    let Setup { app, admin, .. } = connected(upstream).await;
    let preview = admin.get("/jira/projects/WEB/preview").await;

    assert_eq!((preview["agile"].as_bool(), preview["sprints"].as_i64()), (Some(false), Some(0)));

    let job = admin.import(json!({ "projectKey": "WEB" })).await;

    assert_eq!(counts(&job), (7, 0, 0, 0, 0), "{job}");

    // The planned sprint has no issue yet, so only the two that issues name exist.
    let sprints: Vec<(String, String, Option<String>)> = app
        .read(|connection| query_all(connection, "SELECT name, state, completed_at FROM sprint ORDER BY id", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))))
        .unwrap();

    assert_eq!(
        sprints,
        vec![
            ("Sprint 1".to_string(), "closed".to_string(), Some("2024-03-15T15:00:00.000Z".to_string())),
            ("Sprint 2".to_string(), "active".to_string(), None)
        ]
    );
    assert_eq!(state(&app, "WEB-2").4.as_deref(), Some("Sprint 1"));
    assert_eq!(state(&app, "WEB-5").4.as_deref(), Some("Sprint 2"));
    assert_eq!(history(&app, "WEB-5").len(), 4);

    // Without a board to read the columns from, the statuses are still in workflow order.
    let statuses: Vec<String> = app.read(|connection| query_all(connection, "SELECT name FROM status ORDER BY position", [], |row| row.get(0))).unwrap();

    assert_eq!(statuses, vec!["Open", "In Progress", "In Review", "Done"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn comments_beyond_those_in_the_search_result_are_fetched() {
    let mut upstream = small(true);
    upstream.embedded_comments = 1;

    let Setup { app, admin, fake, .. } = connected(upstream).await;

    fake.take_requests();

    let job = admin.import(json!({ "projectKey": "WEB" })).await;
    let requests = fake.take_requests();

    assert_eq!(job["report"]["comments"], 3);
    assert_eq!(count(&app, "SELECT count(*) FROM comment c JOIN issue i ON i.id = c.issue_id WHERE i.key = 'WEB-2'"), 2);
    assert_eq!(requests.iter().filter(|request| request.contains("/issue/WEB-2/comment")).count(), 1, "{requests:?}");
    assert!(!requests.iter().any(|request| request.contains("/issue/WEB-5/comment")), "{requests:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_token_ends_the_import_and_is_not_tried_again() {
    let Setup { app, admin, fake, .. } = connected(small(true)).await;

    fake.with(|upstream| upstream.accepts = "Basic something-else".to_string());
    fake.take_requests();

    let job = admin.finished(&admin.post("/jira/import", json!({ "projectKey": "WEB" })).await).await;
    let requests = fake.take_requests();

    assert_eq!((job["state"].as_str(), job["error"].as_str()), (Some("failed"), Some("Jira rejected the token")), "{job}");
    // Asked what it is, refused once, and that was it.
    assert_eq!(requests, vec!["GET /rest/api/2/serverInfo", "GET /rest/api/3/myself"]);
    assert_eq!(count(&app, "SELECT count(*) FROM project"), 0);
    assert!(!job.to_string().contains(common::TOKEN));

    // The same half-way through: what was imported stays, the job says why it stopped.
    fake.with(|upstream| upstream.accepts = common::BASIC.to_string());
    assert_eq!(counts(&admin.import(json!({ "projectKey": "WEB" })).await), (7, 0, 0, 0, 0));

    let jobs = admin.get("/jira/jobs").await;

    assert_eq!(jobs.as_array().unwrap().iter().map(|job| job["state"].as_str().unwrap()).collect::<Vec<_>>(), vec!["done", "failed"]);
    assert!(!jobs.to_string().contains(common::TOKEN));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_can_be_cancelled_and_picked_up_again() {
    let mut upstream = fake::demo(false, common::BEARER);
    upstream.page_size = 2;
    upstream.delay = 15;

    let Setup { app, admin, member, .. } = connected(upstream).await;
    let job = admin.post("/jira/import", json!({ "projectKey": "WEB" })).await;
    let id = job["id"].as_i64().unwrap();

    // Only one import runs at a time, and only administrators see or stop it.
    let (status, message) = admin.refused(axum::http::Method::POST, "/jira/import", Some(json!({ "projectKey": "WEB" }))).await;

    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert!(message.contains("still running"), "{message}");
    assert_eq!(member.refused(axum::http::Method::POST, &format!("/jira/jobs/{id}/cancel"), None).await.0, axum::http::StatusCode::FORBIDDEN);

    for _ in 0..2000 {
        if admin.get(&format!("/jira/jobs/{id}")).await["processed"].as_i64().unwrap_or(0) >= 4 {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    admin.post(&format!("/jira/jobs/{id}/cancel"), json!({})).await;

    let cancelled = admin.finished(&job).await;
    let imported = count(&app, "SELECT count(*) FROM issue");

    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    assert!((4..60).contains(&imported), "{imported}");
    assert_eq!(cancelled["processed"].as_i64(), Some(imported));
    assert_eq!(admin.refused(axum::http::Method::POST, &format!("/jira/jobs/{id}/cancel"), None).await.0, axum::http::StatusCode::CONFLICT);

    // Running it again finishes the job: nothing twice, nothing missing.
    let done = admin.import(json!({ "projectKey": "WEB" })).await;

    assert_eq!(done["report"]["created"].as_i64(), Some(60 - imported), "{done}");
    assert_eq!(done["report"]["failed"], json!([]));
    assert_eq!(count(&app, "SELECT count(*) FROM issue"), 60);
    assert_eq!(count(&app, "SELECT count(*) FROM issue WHERE parent_id IS NULL AND type_id NOT IN (SELECT id FROM issue_type WHERE level = 'epic')"), 0);
}

async fn a_project_of_sixty_issues(cloud: bool) {
    let Setup { app, admin, .. } = connected(fake::demo(cloud, if cloud { common::BASIC } else { common::BEARER })).await;
    let job = admin.import(json!({ "projectKey": "WEB" })).await;

    assert_eq!(counts(&job), (60, 0, 0, 0, 0), "{job}");
    assert_eq!(job["report"]["unmapped"], json!([]));
    assert_eq!(job["report"]["links"], 4);
    assert_eq!(count(&app, "SELECT issue_seq FROM project"), 63);
    assert_eq!(count(&app, "SELECT count(*) FROM issue WHERE key IN ('WEB-25', 'WEB-26', 'WEB-27')"), 0);

    // 4 epics, 42 stories, bugs and tasks under them, 14 subtasks under those.
    let by_level = |level: &str| count(&app, &format!("SELECT count(*) FROM issue i JOIN issue_type t ON t.id = i.type_id WHERE t.level = '{level}'"));

    assert_eq!((by_level("epic"), by_level("standard"), by_level("subtask")), (4, 42, 14));
    assert_eq!(count(&app, "SELECT count(*) FROM issue i JOIN issue_type t ON t.id = i.type_id WHERE t.level != 'epic' AND i.parent_id IS NULL"), 0);

    // Every issue's history ends where the issue is, and what is done was resolved when
    // its history says it reached "Done".
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM issue i WHERE EXISTS (SELECT 1 FROM change c WHERE c.issue_id = i.id AND c.field = 'status')
             AND i.status_id != (SELECT c.new_ref FROM change c WHERE c.issue_id = i.id AND c.field = 'status' ORDER BY c.created_at DESC, c.id DESC LIMIT 1)"
        ),
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM issue i JOIN status s ON s.id = i.status_id WHERE (s.category = 'done') != (i.resolved_at IS NOT NULL)
             OR (i.resolved_at IS NOT NULL AND i.resolved_at != i.status_since)"
        ),
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM issue i WHERE coalesce(i.sprint_id, 0) != coalesce((SELECT c.new_ref FROM change c WHERE c.issue_id = i.id AND c.field = 'sprint'
             ORDER BY c.created_at DESC, c.id DESC LIMIT 1), i.sprint_id, 0)"
        ),
        0
    );

    // Three sprints are over with all their work done in them, one runs, one is planned.
    let sprints: Vec<(String, String, i64, i64)> = app
        .read(|connection| {
            query_all(
                connection,
                "SELECT sp.name, sp.state, count(i.id), coalesce(sum(s.category = 'done'), 0) FROM sprint sp
                 LEFT JOIN issue i ON i.sprint_id = sp.id AND i.type_id NOT IN (SELECT id FROM issue_type WHERE level = 'subtask')
                 LEFT JOIN status s ON s.id = i.status_id GROUP BY sp.id ORDER BY sp.position",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
        })
        .unwrap();

    assert_eq!(
        sprints.iter().map(|(name, state, issues, done)| (name.as_str(), state.as_str(), *issues, *done)).collect::<Vec<_>>(),
        vec![
            ("WEB Sprint 1", "closed", 8, 8),
            ("WEB Sprint 2", "closed", 6, 6),
            ("WEB Sprint 3", "closed", 10, 10),
            ("WEB Sprint 4", "active", 10, 4),
            ("WEB Sprint 5", "future", 4, 0)
        ]
    );
    // The two issues carried over left the second sprint when it ended and joined the third.
    assert_eq!(count(&app, "SELECT count(*) FROM change WHERE field = 'sprint' AND old_value = 'WEB Sprint 2' AND new_value IS NULL AND actor_label = 'Jira'"), 2);

    // The ranks an import appends stay short enough to sort and to read.
    assert!(count(&app, "SELECT max(length(rank)) FROM issue") <= 16);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_realistic_cloud_project_arrives_whole() {
    a_project_of_sixty_issues(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_realistic_server_project_arrives_whole() {
    a_project_of_sixty_issues(false).await;
}
