//! Example data for a demo instance: two projects with a few weeks of history, so every
//! view has something real to show. Everything goes through the same services the API uses.

use chrono::{Duration, SecondsFormat, Utc};
use hub_core::{BoardMode, Id, IssuePatch, LinkType, NewIssue, Role};

use crate::access::project_access;
use crate::app::{Actor, App};
use crate::auth::AuthUser;
use crate::db::query_one;
use crate::error::{ApiError, ApiResult};
use crate::services::issues::{add_comment, add_link, add_worklog, create_issue, issue_id_by_key, update_issue};
use crate::services::projects::{add_component, create_project, ComponentInput, NewProject};
use crate::services::schedule::Ticker;
use crate::services::sprints::{complete_sprint, create_sprint, start_sprint, SprintInput};
use crate::services::users::{create_user, get_user, NewUser};
use crate::services::versions::{create_version, set_released, VersionInput};

struct Spec {
    kind: &'static str,
    summary: &'static str,
    description: &'static str,
    parent: Option<&'static str>,
    sprint: Option<usize>,
    /// Statuses the issue went through after creation, with the day (relative to today) of each move.
    path: &'static [(&'static str, i64)],
    /// Index into the demo people; 0 is whoever set the instance up.
    assignee: Option<usize>,
    points: Option<f64>,
    priority: i64,
    labels: &'static [&'static str],
    component: Option<&'static str>,
    version: Option<usize>,
    created: i64,
}

const BASE: Spec = Spec {
    kind: "Story",
    summary: "",
    description: "",
    parent: None,
    sprint: None,
    path: &[],
    assignee: None,
    points: None,
    priority: 3,
    labels: &[],
    component: None,
    version: None,
    created: -24,
};

const WEB: &[Spec] = &[
    Spec { kind: "Epic", summary: "Proces zakupowy", description: "Od koszyka do potwierdzenia zamówienia: koszyk, adres, płatności, e-maile.", created: -30, ..BASE },
    Spec { kind: "Epic", summary: "Konto klienta", description: "Rejestracja, logowanie, profil i historia zamówień.", created: -30, ..BASE },
    Spec { kind: "Epic", summary: "Wydajność i SEO", created: -28, ..BASE },
    Spec { summary: "Koszyk: dodawanie i usuwanie produktów", parent: Some("WEB-1"), sprint: Some(0), path: &[("In Progress", -19), ("In Review", -16), ("Done", -15)], assignee: Some(1), points: Some(5.0), component: Some("Koszyk"), version: Some(0), ..BASE },
    Spec { summary: "Formularz adresu dostawy z walidacją", parent: Some("WEB-1"), sprint: Some(0), path: &[("In Progress", -18), ("In Review", -13), ("Done", -12)], assignee: Some(2), points: Some(3.0), component: Some("Frontend"), version: Some(0), ..BASE },
    Spec { kind: "Bug", summary: "Cena w koszyku nie uwzględnia rabatu", description: "## Kroki\n\n1. Dodaj produkt objęty promocją\n2. Otwórz koszyk\n\n## Aktualnie\n\nSuma liczona z ceny katalogowej.\n\n## Oczekiwane\n\nSuma uwzględnia rabat pozycji.", parent: Some("WEB-1"), sprint: Some(0), path: &[("In Progress", -14), ("Done", -11)], assignee: Some(3), points: Some(2.0), priority: 2, labels: &["regresja"], component: Some("Koszyk"), version: Some(0), created: -17 },
    Spec { kind: "Task", summary: "Konfiguracja CI: testy i lint na każdym PR", sprint: Some(0), path: &[("In Progress", -20), ("Done", -17)], assignee: Some(4), points: Some(2.0), labels: &["devops"], version: Some(0), ..BASE },
    Spec { summary: "Rejestracja i logowanie e-mailem", parent: Some("WEB-2"), sprint: Some(0), path: &[("In Progress", -17), ("In Review", -10), ("Done", -8)], assignee: Some(0), points: Some(5.0), component: Some("Frontend"), version: Some(0), ..BASE },
    Spec { summary: "Płatność kartą przez bramkę", description: "Integracja z bramką płatniczą: formularz karty, 3-D Secure, webhook potwierdzenia.\n\nKryteria akceptacji:\n\n- [ ] płatność testową kartą przechodzi\n- [ ] odrzucona karta pokazuje czytelny komunikat\n- [ ] zamówienie zmienia status po webhooku", parent: Some("WEB-1"), sprint: Some(1), path: &[("In Progress", -5), ("In Review", -1)], assignee: Some(1), points: Some(8.0), priority: 2, labels: &["płatności"], component: Some("Płatności"), version: Some(1), created: -12, ..BASE },
    Spec { summary: "Płatność BLIK", parent: Some("WEB-1"), sprint: Some(1), path: &[("In Progress", -3)], assignee: Some(2), points: Some(5.0), labels: &["płatności"], component: Some("Płatności"), version: Some(1), created: -12, ..BASE },
    Spec { kind: "Bug", summary: "Błąd 500 przy pustym koszyku w podsumowaniu", parent: Some("WEB-1"), sprint: Some(1), path: &[("In Progress", -4), ("In Review", -3), ("Done", -2)], assignee: Some(3), points: Some(2.0), priority: 1, component: Some("Koszyk"), version: Some(1), created: -6, ..BASE },
    Spec { summary: "Historia zamówień w profilu", parent: Some("WEB-2"), sprint: Some(1), path: &[("In Progress", -4)], assignee: Some(0), points: Some(5.0), component: Some("Frontend"), version: Some(1), created: -12, ..BASE },
    Spec { kind: "Task", summary: "Migracja sesji do Redis", sprint: Some(1), assignee: Some(4), points: Some(3.0), labels: &["backend"], created: -10, ..BASE },
    Spec { summary: "Reset hasła", parent: Some("WEB-2"), sprint: Some(1), path: &[("In Progress", -5), ("In Review", -4), ("Done", -3)], assignee: Some(0), points: Some(3.0), version: Some(1), created: -12, ..BASE },
    Spec { kind: "Bug", summary: "Walidacja kodu pocztowego odrzuca poprawne kody", parent: Some("WEB-1"), sprint: Some(1), path: &[("In Progress", -2), ("In Review", -1)], assignee: Some(2), points: Some(1.0), version: Some(1), created: -5, ..BASE },
    Spec { summary: "E-mail z potwierdzeniem zamówienia", parent: Some("WEB-1"), sprint: Some(1), assignee: Some(1), points: Some(3.0), version: Some(1), created: -12, ..BASE },
    Spec { kind: "Task", summary: "Testy e2e ścieżki zakupowej", parent: Some("WEB-1"), sprint: Some(1), path: &[("In Progress", -8)], assignee: Some(3), points: Some(5.0), labels: &["testy"], created: -12, ..BASE },
    Spec { summary: "Kody rabatowe", parent: Some("WEB-1"), sprint: Some(2), points: Some(5.0), version: Some(2), created: -9, ..BASE },
    Spec { summary: "Lista życzeń", parent: Some("WEB-2"), sprint: Some(2), points: Some(3.0), version: Some(2), created: -9, ..BASE },
    Spec { kind: "Task", summary: "Lazy loading obrazów produktów", parent: Some("WEB-3"), sprint: Some(2), assignee: Some(4), points: Some(2.0), labels: &["wydajność"], created: -9, ..BASE },
    Spec { summary: "Porównywarka produktów", points: Some(8.0), priority: 4, created: -8, ..BASE },
    Spec { kind: "Bug", summary: "Menu mobilne nie zamyka się po wyborze kategorii", component: Some("Frontend"), labels: &["mobile"], created: -7, ..BASE },
    Spec { summary: "Opinie i oceny produktów", parent: Some("WEB-2"), points: Some(5.0), created: -7, ..BASE },
    Spec { kind: "Task", summary: "Mapa witryny i dane strukturalne", parent: Some("WEB-3"), points: Some(3.0), labels: &["seo"], created: -6, ..BASE },
    Spec { kind: "Bug", summary: "Literówka w stopce: „regulamn”", priority: 5, created: -4, ..BASE },
    Spec { summary: "Logowanie przez Google", parent: Some("WEB-2"), points: Some(5.0), created: -3, ..BASE },
    Spec { kind: "Subtask", summary: "Formularz karty (frontend)", parent: Some("WEB-9"), sprint: Some(1), path: &[("In Progress", -5), ("Done", -3)], assignee: Some(1), created: -6, ..BASE },
    Spec { kind: "Subtask", summary: "Obsługa 3-D Secure", parent: Some("WEB-9"), sprint: Some(1), path: &[("In Progress", -3)], assignee: Some(1), created: -6, ..BASE },
    Spec { kind: "Subtask", summary: "Webhook potwierdzenia płatności", parent: Some("WEB-9"), sprint: Some(1), assignee: Some(4), created: -6, ..BASE },
];

const API: &[Spec] = &[
    Spec { kind: "Epic", summary: "API zamówień v2", created: -20, ..BASE },
    Spec {
        summary: "Endpoint listy zamówień ze stronicowaniem",
        parent: Some("API-1"),
        path: &[("To Do", -15), ("In Progress", -12), ("In Review", -9), ("Done", -7)],
        assignee: Some(4),
        points: Some(3.0),
        created: -18,
        ..BASE
    },
    Spec {
        summary: "Webhook zmiany statusu zamówienia",
        parent: Some("API-1"),
        path: &[("To Do", -10), ("In Progress", -4)],
        assignee: Some(4),
        points: Some(5.0),
        priority: 2,
        created: -14,
        ..BASE
    },
    Spec {
        kind: "Bug",
        summary: "Limit zapytań liczony per IP zamiast per klucz",
        path: &[("To Do", -6), ("In Progress", -5), ("In Review", -2)],
        assignee: Some(2),
        priority: 2,
        labels: &["bezpieczeństwo"],
        created: -8,
        ..BASE
    },
    Spec {
        kind: "Task",
        summary: "Dokumentacja OpenAPI dla zamówień",
        parent: Some("API-1"),
        path: &[("To Do", -3)],
        assignee: Some(0),
        created: -9,
        ..BASE
    },
    Spec { kind: "Task", summary: "Rotacja kluczy API bez przestoju", priority: 2, created: -5, ..BASE },
    Spec { summary: "Filtrowanie zamówień po dacie i statusie", parent: Some("API-1"), created: -4, ..BASE },
    Spec {
        kind: "Bug",
        summary: "Błędny kod odpowiedzi przy braku uprawnień (500 zamiast 403)",
        path: &[("To Do", -2)],
        assignee: Some(2),
        created: -3,
        ..BASE
    },
];

fn at(day: i64, hour: i64) -> String {
    let instant = Utc::now() + Duration::days(day) - Duration::hours(if day < 0 { hour % 7 } else { 0 }) - Duration::minutes(5);

    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn date(day: i64) -> String {
    (Utc::now() + Duration::days(day)).format("%Y-%m-%d").to_string()
}

fn lookup(app: &App, sql: &str, project_id: Id, name: &str) -> ApiResult<Id> {
    app.read(|connection| query_one(connection, sql, (project_id, name), |row| row.get(0)))?
        .ok_or_else(|| ApiError::internal(format!("demo data refers to \"{name}\", which does not exist")))
}

fn seed_issues(app: &App, project_key: &str, project_id: Id, specs: &[Spec], people: &[Id], sprints: &[Id], versions: &[Id]) -> ApiResult<()> {
    for (index, spec) in specs.iter().enumerate() {
        let type_id = lookup(app, "SELECT id FROM issue_type WHERE project_id = ?1 AND name = ?2", project_id, spec.kind)?;
        let component_ids = match spec.component {
            Some(name) => vec![lookup(app, "SELECT id FROM component WHERE project_id = ?1 AND name = ?2", project_id, name)?],
            None => Vec::new(),
        };
        let reporter = people[(index * 3 + 1) % people.len()];
        let new = NewIssue {
            project_key: project_key.to_string(),
            fields: IssuePatch {
                summary: Some(spec.summary.to_string()),
                description: Some(spec.description.to_string()),
                type_id: Some(type_id),
                priority: Some(spec.priority),
                assignee_id: Some(spec.assignee.map(|person| people[person])),
                parent_key: Some(spec.parent.map(String::from)),
                sprint_id: Some(spec.sprint.map(|sprint| sprints[sprint])),
                version_id: Some(spec.version.map(|version| versions[version])),
                points: Some(spec.points),
                labels: Some(spec.labels.iter().map(|label| label.to_string()).collect()),
                component_ids: Some(component_ids),
                ..Default::default()
            },
        };

        app.write_at(Actor::user(reporter), at(spec.created, index as i64), |tx| create_issue(tx, &new))?;
    }

    Ok(())
}

/// Replays the status moves of every issue in day order, so sprints are started and
/// completed around them at the right moments.
fn seed_moves(app: &App, project_key: &str, project_id: Id, specs: &[Spec], people: &[Id], days: std::ops::Range<i64>) -> ApiResult<()> {
    for day in days {
        for (index, spec) in specs.iter().enumerate() {
            for (status, moved_on) in spec.path {
                if *moved_on != day {
                    continue;
                }

                let key = format!("{project_key}-{}", index + 1);
                let status_id = lookup(app, "SELECT id FROM status WHERE project_id = ?1 AND name = ?2", project_id, status)?;
                let actor = spec.assignee.map(|person| people[person]).unwrap_or(people[0]);

                app.write_at(Actor::user(actor), at(day, index as i64), |tx| {
                    let (issue_id, _) = issue_id_by_key(tx, &key)?.ok_or_else(|| ApiError::not_found("Issue"))?;

                    update_issue(tx, issue_id, &IssuePatch { status_id: Some(status_id), ..Default::default() })
                })?;
            }
        }
    }

    Ok(())
}

fn comment(app: &App, author: Id, day: i64, key: &str, body: &str) -> ApiResult<()> {
    app.write_at(Actor::user(author), at(day, 2), |tx| {
        let (issue_id, _) = issue_id_by_key(tx, key)?.ok_or_else(|| ApiError::not_found("Issue"))?;

        add_comment(tx, issue_id, body).map(|_| ())
    })
}

/// Fills an instance that has exactly one user (whoever just set it up) with example data.
pub fn seed_demo(app: &App, owner_id: Id) -> ApiResult<()> {
    let owner = app.read(|connection| get_user(connection, owner_id))?;
    let admin = AuthUser { id: owner.id, name: owner.name.clone(), email: owner.email.clone(), role: Role::Admin, via_token: false, through: Vec::new() };
    let mut people = vec![owner.id];

    for (name, email, login) in [
        ("Anna Nowak", "anna.nowak@example.com", "anowak"),
        ("Piotr Zieliński", "piotr.zielinski@example.com", "pzielinski"),
        ("Maria Wiśniewska", "maria.wisniewska@example.com", "mwisniewska"),
        ("Tomasz Kowalczyk", "tomasz.kowalczyk@example.com", "tkowalczyk"),
    ] {
        let new = NewUser {
            name: name.to_string(),
            email: email.to_string(),
            password: None,
            role: Some(Role::Member),
            github_login: Some(login.to_string()),
        };
        people.push(app.db.write(|tx| create_user(tx, &new))?.id);
    }

    // --- WEB: a scrum project in its second sprint ---
    let web = app.write_at(admin.actor(), at(-30, 0), |tx| {
        create_project(
            tx,
            &admin,
            &NewProject {
                key: "WEB".to_string(),
                name: "Sklep internetowy".to_string(),
                description: "Sklep B2C: katalog, koszyk, płatności i konto klienta.".to_string(),
                board_mode: Some(BoardMode::Scrum),
                is_private: false,
                lead_id: None,
            },
        )
    })?;
    let web_id = web.summary.id;
    let (sprints, versions) = app.write_at(admin.actor(), at(-29, 0), |tx| {
        let access = project_access(tx, &admin, "WEB")?;

        for name in ["Frontend", "Koszyk", "Płatności"] {
            add_component(tx, &access, &ComponentInput { name: Some(name.to_string()), ..Default::default() })?;
        }

        let mut sprints = Vec::new();

        for (name, goal) in [
            ("WEB Sprint 1", "Klient może złożyć zamówienie z płatnością przy odbiorze"),
            ("WEB Sprint 2", "Płatności online i podstawy konta klienta"),
            ("WEB Sprint 3", "Promocje i lista życzeń"),
        ] {
            let input = SprintInput { name: Some(name.to_string()), goal: Some(goal.to_string()), ..Default::default() };
            sprints.push(create_sprint(tx, &access, &input)?.id);
        }

        let mut versions = Vec::new();

        for (name, description, release) in [
            ("1.0", "Pierwsze wydanie: koszyk, adres, logowanie", -6),
            ("1.1", "Płatności online, historia zamówień", 10),
            ("2.0", "Promocje, lista życzeń", 45),
        ] {
            let input = VersionInput {
                name: Some(name.to_string()),
                description: Some(description.to_string()),
                release_date: Some(date(release)),
                ..Default::default()
            };
            versions.push(create_version(tx, &access, &input)?.id);
        }

        Ok((sprints, versions))
    })?;

    seed_issues(app, "WEB", web_id, WEB, &people, &sprints, &versions)?;

    app.write_at(admin.actor(), at(-20, 6), |tx| {
        start_sprint(tx, &admin, sprints[0], &SprintInput { start_date: Some(date(-20)), end_date: Some(date(-6)), ..Default::default() })
    })?;
    seed_moves(app, "WEB", web_id, WEB, &people, -20..-6)?;
    app.write_at(admin.actor(), at(-6, 6), |tx| complete_sprint(tx, &admin, sprints[0], None))?;
    app.write_at(admin.actor(), at(-6, 5), |tx| set_released(tx, &admin, versions[0], true, None))?;
    app.write_at(admin.actor(), at(-5, 6), |tx| {
        start_sprint(tx, &admin, sprints[1], &SprintInput { start_date: Some(date(-5)), end_date: Some(date(9)), ..Default::default() })
    })?;
    seed_moves(app, "WEB", web_id, WEB, &people, -6..1)?;

    comment(app, people[3], -5, "WEB-9", "Bramka w trybie testowym odrzuca karty bez 3-D Secure. @anna.nowak czy mamy już klucze do sandboxa?")?;
    comment(app, people[1], -4, "WEB-9", "Klucze są w menedżerze haseł zespołu. Formularz karty gotowy, zostaje 3-D Secure i webhook.")?;
    comment(app, people[0], -1, "WEB-9", "Przejrzałem PR: dwie uwagi do obsługi błędów, reszta OK.")?;
    comment(app, people[2], -2, "WEB-10", "BLIK wymaga osobnego kanału powiadomień; rozpisałem to w bazie wiedzy.")?;
    comment(app, people[3], -3, "WEB-11", "Odtworzone na produkcji. Poprawka czeka na review.")?;
    comment(app, people[4], -2, "WEB-13", "Zanim ruszę migrację, potrzebuję okna serwisowego. Proponuję czwartek po 22:00.")?;

    app.write_at(admin.actor(), at(-4, 1), |tx| {
        let id_of = |key: &str| -> ApiResult<Id> { Ok(issue_id_by_key(tx, key)?.ok_or_else(|| ApiError::not_found("Issue"))?.0) };

        add_link(tx, &admin, id_of("WEB-9")?, LinkType::Blocks, "WEB-16")?;
        add_link(tx, &admin, id_of("WEB-11")?, LinkType::Relates, "WEB-6")?;
        add_link(tx, &admin, id_of("WEB-10")?, LinkType::Relates, "WEB-9")?;
        update_issue(
            tx,
            id_of("WEB-9")?,
            &IssuePatch { due_date: Some(Some(date(3))), estimate_minutes: Some(Some(16 * 60)), ..Default::default() },
        )?;
        update_issue(tx, id_of("WEB-12")?, &IssuePatch { due_date: Some(Some(date(6))), ..Default::default() })?;
        update_issue(tx, id_of("WEB-15")?, &IssuePatch { due_date: Some(Some(date(-1))), ..Default::default() })?;

        Ok(())
    })?;
    app.write_at(Actor::user(people[1]), at(-3, 3), |tx| {
        let (issue_id, _) = issue_id_by_key(tx, "WEB-9")?.ok_or_else(|| ApiError::not_found("Issue"))?;
        add_worklog(tx, issue_id, 6 * 60, None, "Formularz karty i walidacja")?;
        add_worklog(tx, issue_id, 3 * 60 + 30, None, "Integracja z sandboxem bramki")?;

        Ok(())
    })?;

    // --- API: a kanban project ---
    let api = app.write_at(admin.actor(), at(-21, 0), |tx| {
        create_project(
            tx,
            &admin,
            &NewProject {
                key: "API".to_string(),
                name: "Platforma API".to_string(),
                description: "Publiczne API dla partnerów i aplikacji mobilnej.".to_string(),
                board_mode: Some(BoardMode::Kanban),
                is_private: false,
                lead_id: Some(people[4]),
            },
        )
    })?;

    seed_issues(app, "API", api.summary.id, API, &people, &[], &[])?;
    seed_moves(app, "API", api.summary.id, API, &people, -20..1)?;
    comment(app, people[2], -2, "API-4", "Poprawka liczy limit per klucz; dodałem test z dwoma kluczami z jednego adresu.")?;

    for module in app.enabled_modules() {
        module.seed_demo(app)?;
    }

    // The demo appears all at once, tasks of the schedule included. Without this, a demo
    // made in the afternoon would run its morning's tasks a minute later.
    Ticker::new(app, Utc::now).skip_due()
}
