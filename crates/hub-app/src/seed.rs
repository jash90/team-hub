//! Example data for a demo instance: two projects with a few weeks of history, so every
//! view has something real to show. Everything goes through the same services the API uses.
//!
//! The data comes in English and in Polish. The structure is one (the same keys, statuses,
//! people's roles and dates); every text a person reads is a pair, and the language the
//! instance is set up in picks one of the two.

use chrono::{Duration, SecondsFormat, Utc};
use hub_core::demo::{DemoLanguage, DemoText as Text};
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
    summary: Text,
    description: Text,
    parent: Option<&'static str>,
    sprint: Option<usize>,
    /// Statuses the issue went through after creation, with the day (relative to today) of each move.
    path: &'static [(&'static str, i64)],
    /// Index into the demo people; 0 is whoever set the instance up.
    assignee: Option<usize>,
    points: Option<f64>,
    priority: i64,
    labels: &'static [Text],
    component: Option<Text>,
    version: Option<usize>,
    created: i64,
}

const BASE: Spec = Spec {
    kind: "Story",
    summary: Text::EMPTY,
    description: Text::EMPTY,
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
    Spec {
        kind: "Epic",
        summary: Text::new("Checkout", "Proces zakupowy"),
        description: Text::new(
            "From the cart to the order confirmation: cart, address, payments, e-mails.",
            "Od koszyka do potwierdzenia zamówienia: koszyk, adres, płatności, e-maile.",
        ),
        created: -30,
        ..BASE
    },
    Spec {
        kind: "Epic",
        summary: Text::new("Customer account", "Konto klienta"),
        description: Text::new("Sign-up, sign-in, profile and order history.", "Rejestracja, logowanie, profil i historia zamówień."),
        created: -30,
        ..BASE
    },
    Spec {
        kind: "Epic",
        summary: Text::new("Performance and SEO", "Wydajność i SEO"),
        created: -28,
        ..BASE
    },
    Spec {
        summary: Text::new("Cart: add and remove products", "Koszyk: dodawanie i usuwanie produktów"),
        parent: Some("WEB-1"),
        sprint: Some(0),
        path: &[("In Progress", -19), ("In Review", -16), ("Done", -15)],
        assignee: Some(1),
        points: Some(5.0),
        component: Some(Text::new("Cart", "Koszyk")),
        version: Some(0),
        ..BASE
    },
    Spec {
        summary: Text::new("Delivery address form with validation", "Formularz adresu dostawy z walidacją"),
        parent: Some("WEB-1"),
        sprint: Some(0),
        path: &[("In Progress", -18), ("In Review", -13), ("Done", -12)],
        assignee: Some(2),
        points: Some(3.0),
        component: Some(Text::same("Frontend")),
        version: Some(0),
        ..BASE
    },
    Spec {
        kind: "Bug",
        summary: Text::new("Cart price ignores the discount", "Cena w koszyku nie uwzględnia rabatu"),
        description: Text::new(
            "## Steps\n\n1. Add a product that is on promotion\n2. Open the cart\n\n## Actual\n\nThe total is calculated from the list price.\n\n## Expected\n\nThe total includes the item's discount.",
            "## Kroki\n\n1. Dodaj produkt objęty promocją\n2. Otwórz koszyk\n\n## Aktualnie\n\nSuma liczona z ceny katalogowej.\n\n## Oczekiwane\n\nSuma uwzględnia rabat pozycji.",
        ),
        parent: Some("WEB-1"),
        sprint: Some(0),
        path: &[("In Progress", -14), ("Done", -11)],
        assignee: Some(3),
        points: Some(2.0),
        priority: 2,
        labels: &[Text::new("regression", "regresja")],
        component: Some(Text::new("Cart", "Koszyk")),
        version: Some(0),
        created: -17,
    },
    Spec {
        kind: "Task",
        summary: Text::new("CI setup: tests and lint on every PR", "Konfiguracja CI: testy i lint na każdym PR"),
        sprint: Some(0),
        path: &[("In Progress", -20), ("Done", -17)],
        assignee: Some(4),
        points: Some(2.0),
        labels: &[Text::same("devops")],
        version: Some(0),
        ..BASE
    },
    Spec {
        summary: Text::new("Sign-up and sign-in by e-mail", "Rejestracja i logowanie e-mailem"),
        parent: Some("WEB-2"),
        sprint: Some(0),
        path: &[("In Progress", -17), ("In Review", -10), ("Done", -8)],
        assignee: Some(0),
        points: Some(5.0),
        component: Some(Text::same("Frontend")),
        version: Some(0),
        ..BASE
    },
    Spec {
        summary: Text::new("Card payment through the gateway", "Płatność kartą przez bramkę"),
        description: Text::new(
            "Integration with the payment gateway: card form, 3-D Secure, confirmation webhook.\n\nAcceptance criteria:\n\n- [ ] a payment with a test card goes through\n- [ ] a declined card shows a clear message\n- [ ] the order changes status after the webhook",
            "Integracja z bramką płatniczą: formularz karty, 3-D Secure, webhook potwierdzenia.\n\nKryteria akceptacji:\n\n- [ ] płatność testową kartą przechodzi\n- [ ] odrzucona karta pokazuje czytelny komunikat\n- [ ] zamówienie zmienia status po webhooku",
        ),
        parent: Some("WEB-1"),
        sprint: Some(1),
        path: &[("In Progress", -5), ("In Review", -1)],
        assignee: Some(1),
        points: Some(8.0),
        priority: 2,
        labels: &[Text::new("payments", "płatności")],
        component: Some(Text::new("Payments", "Płatności")),
        version: Some(1),
        created: -12,
        ..BASE
    },
    Spec {
        summary: Text::new("Pay by bank", "Płatność BLIK"),
        parent: Some("WEB-1"),
        sprint: Some(1),
        path: &[("In Progress", -3)],
        assignee: Some(2),
        points: Some(5.0),
        labels: &[Text::new("payments", "płatności")],
        component: Some(Text::new("Payments", "Płatności")),
        version: Some(1),
        created: -12,
        ..BASE
    },
    Spec {
        kind: "Bug",
        summary: Text::new("Error 500 on the order summary with an empty cart", "Błąd 500 przy pustym koszyku w podsumowaniu"),
        parent: Some("WEB-1"),
        sprint: Some(1),
        path: &[("In Progress", -4), ("In Review", -3), ("Done", -2)],
        assignee: Some(3),
        points: Some(2.0),
        priority: 1,
        component: Some(Text::new("Cart", "Koszyk")),
        version: Some(1),
        created: -6,
        ..BASE
    },
    Spec {
        summary: Text::new("Order history in the profile", "Historia zamówień w profilu"),
        parent: Some("WEB-2"),
        sprint: Some(1),
        path: &[("In Progress", -4)],
        assignee: Some(0),
        points: Some(5.0),
        component: Some(Text::same("Frontend")),
        version: Some(1),
        created: -12,
        ..BASE
    },
    Spec {
        kind: "Task",
        summary: Text::new("Move sessions to Redis", "Migracja sesji do Redis"),
        sprint: Some(1),
        assignee: Some(4),
        points: Some(3.0),
        labels: &[Text::same("backend")],
        created: -10,
        ..BASE
    },
    Spec {
        summary: Text::new("Password reset", "Reset hasła"),
        parent: Some("WEB-2"),
        sprint: Some(1),
        path: &[("In Progress", -5), ("In Review", -4), ("Done", -3)],
        assignee: Some(0),
        points: Some(3.0),
        version: Some(1),
        created: -12,
        ..BASE
    },
    Spec {
        kind: "Bug",
        summary: Text::new("Postcode validation rejects valid postcodes", "Walidacja kodu pocztowego odrzuca poprawne kody"),
        parent: Some("WEB-1"),
        sprint: Some(1),
        path: &[("In Progress", -2), ("In Review", -1)],
        assignee: Some(2),
        points: Some(1.0),
        version: Some(1),
        created: -5,
        ..BASE
    },
    Spec {
        summary: Text::new("Order confirmation e-mail", "E-mail z potwierdzeniem zamówienia"),
        parent: Some("WEB-1"),
        sprint: Some(1),
        assignee: Some(1),
        points: Some(3.0),
        version: Some(1),
        created: -12,
        ..BASE
    },
    Spec {
        kind: "Task",
        summary: Text::new("End-to-end tests of the checkout flow", "Testy e2e ścieżki zakupowej"),
        parent: Some("WEB-1"),
        sprint: Some(1),
        path: &[("In Progress", -8)],
        assignee: Some(3),
        points: Some(5.0),
        labels: &[Text::new("tests", "testy")],
        created: -12,
        ..BASE
    },
    Spec {
        summary: Text::new("Discount codes", "Kody rabatowe"),
        parent: Some("WEB-1"),
        sprint: Some(2),
        points: Some(5.0),
        version: Some(2),
        created: -9,
        ..BASE
    },
    Spec {
        summary: Text::new("Wishlist", "Lista życzeń"),
        parent: Some("WEB-2"),
        sprint: Some(2),
        points: Some(3.0),
        version: Some(2),
        created: -9,
        ..BASE
    },
    Spec {
        kind: "Task",
        summary: Text::new("Lazy loading of product images", "Lazy loading obrazów produktów"),
        parent: Some("WEB-3"),
        sprint: Some(2),
        assignee: Some(4),
        points: Some(2.0),
        labels: &[Text::new("performance", "wydajność")],
        created: -9,
        ..BASE
    },
    Spec {
        summary: Text::new("Product comparison", "Porównywarka produktów"),
        points: Some(8.0),
        priority: 4,
        created: -8,
        ..BASE
    },
    Spec {
        kind: "Bug",
        summary: Text::new("Mobile menu stays open after a category is chosen", "Menu mobilne nie zamyka się po wyborze kategorii"),
        component: Some(Text::same("Frontend")),
        labels: &[Text::same("mobile")],
        created: -7,
        ..BASE
    },
    Spec {
        summary: Text::new("Product reviews and ratings", "Opinie i oceny produktów"),
        parent: Some("WEB-2"),
        points: Some(5.0),
        created: -7,
        ..BASE
    },
    Spec {
        kind: "Task",
        summary: Text::new("Sitemap and structured data", "Mapa witryny i dane strukturalne"),
        parent: Some("WEB-3"),
        points: Some(3.0),
        labels: &[Text::same("seo")],
        created: -6,
        ..BASE
    },
    Spec {
        kind: "Bug",
        summary: Text::new("Typo in the footer: “Terms and Conditons”", "Literówka w stopce: „regulamn”"),
        priority: 5,
        created: -4,
        ..BASE
    },
    Spec {
        summary: Text::new("Sign in with Google", "Logowanie przez Google"),
        parent: Some("WEB-2"),
        points: Some(5.0),
        created: -3,
        ..BASE
    },
    Spec {
        kind: "Subtask",
        summary: Text::new("Card form (frontend)", "Formularz karty (frontend)"),
        parent: Some("WEB-9"),
        sprint: Some(1),
        path: &[("In Progress", -5), ("Done", -3)],
        assignee: Some(1),
        created: -6,
        ..BASE
    },
    Spec {
        kind: "Subtask",
        summary: Text::new("3-D Secure handling", "Obsługa 3-D Secure"),
        parent: Some("WEB-9"),
        sprint: Some(1),
        path: &[("In Progress", -3)],
        assignee: Some(1),
        created: -6,
        ..BASE
    },
    Spec {
        kind: "Subtask",
        summary: Text::new("Payment confirmation webhook", "Webhook potwierdzenia płatności"),
        parent: Some("WEB-9"),
        sprint: Some(1),
        assignee: Some(4),
        created: -6,
        ..BASE
    },
];

const API: &[Spec] = &[
    Spec { kind: "Epic", summary: Text::new("Orders API v2", "API zamówień v2"), created: -20, ..BASE },
    Spec {
        summary: Text::new("Order list endpoint with pagination", "Endpoint listy zamówień ze stronicowaniem"),
        parent: Some("API-1"),
        path: &[("To Do", -15), ("In Progress", -12), ("In Review", -9), ("Done", -7)],
        assignee: Some(4),
        points: Some(3.0),
        created: -18,
        ..BASE
    },
    Spec {
        summary: Text::new("Order status change webhook", "Webhook zmiany statusu zamówienia"),
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
        summary: Text::new("Rate limit counted per IP instead of per key", "Limit zapytań liczony per IP zamiast per klucz"),
        path: &[("To Do", -6), ("In Progress", -5), ("In Review", -2)],
        assignee: Some(2),
        priority: 2,
        labels: &[Text::new("security", "bezpieczeństwo")],
        created: -8,
        ..BASE
    },
    Spec {
        kind: "Task",
        summary: Text::new("OpenAPI documentation for orders", "Dokumentacja OpenAPI dla zamówień"),
        parent: Some("API-1"),
        path: &[("To Do", -3)],
        assignee: Some(0),
        created: -9,
        ..BASE
    },
    Spec {
        kind: "Task",
        summary: Text::new("API key rotation without downtime", "Rotacja kluczy API bez przestoju"),
        priority: 2,
        created: -5,
        ..BASE
    },
    Spec {
        summary: Text::new("Filter orders by date and status", "Filtrowanie zamówień po dacie i statusie"),
        parent: Some("API-1"),
        created: -4,
        ..BASE
    },
    Spec {
        kind: "Bug",
        summary: Text::new(
            "Wrong response code when access is denied (500 instead of 403)",
            "Błędny kod odpowiedzi przy braku uprawnień (500 zamiast 403)",
        ),
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

/// Where the issues of one project go: the project, and what its specs point at by index.
struct Target<'a> {
    project_key: &'a str,
    project_id: Id,
    people: &'a [Id],
    sprints: &'a [Id],
    versions: &'a [Id],
}

fn seed_issues(app: &App, language: DemoLanguage, target: &Target, specs: &[Spec]) -> ApiResult<()> {
    let Target { project_key, project_id, people, sprints, versions } = *target;

    for (index, spec) in specs.iter().enumerate() {
        let type_id = lookup(app, "SELECT id FROM issue_type WHERE project_id = ?1 AND name = ?2", project_id, spec.kind)?;
        let component_ids = match spec.component {
            Some(name) => vec![lookup(app, "SELECT id FROM component WHERE project_id = ?1 AND name = ?2", project_id, name.pick(language))?],
            None => Vec::new(),
        };
        let reporter = people[(index * 3 + 1) % people.len()];
        let new = NewIssue {
            project_key: project_key.to_string(),
            fields: IssuePatch {
                summary: Some(spec.summary.pick(language).to_string()),
                description: Some(spec.description.pick(language).to_string()),
                type_id: Some(type_id),
                priority: Some(spec.priority),
                assignee_id: Some(spec.assignee.map(|person| people[person])),
                parent_key: Some(spec.parent.map(String::from)),
                sprint_id: Some(spec.sprint.map(|sprint| sprints[sprint])),
                version_id: Some(spec.version.map(|version| versions[version])),
                points: Some(spec.points),
                labels: Some(spec.labels.iter().map(|label| label.pick(language).to_string()).collect()),
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

struct Person {
    name: Text,
    email: Text,
    github_login: Text,
}

/// The team next to whoever set the instance up. Their positions, counted from 1, are what
/// `Spec::assignee` and the comments name.
const TEAM: &[Person] = &[
    Person {
        name: Text::new("Emma Clarke", "Anna Nowak"),
        email: Text::new("emma.clarke@example.com", "anna.nowak@example.com"),
        github_login: Text::new("eclarke", "anowak"),
    },
    Person {
        name: Text::new("James Bennett", "Piotr Zieliński"),
        email: Text::new("james.bennett@example.com", "piotr.zielinski@example.com"),
        github_login: Text::new("jbennett", "pzielinski"),
    },
    Person {
        name: Text::new("Olivia Hughes", "Maria Wiśniewska"),
        email: Text::new("olivia.hughes@example.com", "maria.wisniewska@example.com"),
        github_login: Text::new("ohughes", "mwisniewska"),
    },
    Person {
        name: Text::new("Daniel Foster", "Tomasz Kowalczyk"),
        email: Text::new("daniel.foster@example.com", "tomasz.kowalczyk@example.com"),
        github_login: Text::new("dfoster", "tkowalczyk"),
    },
];

const WEB_NAME: Text = Text::new("Online store", "Sklep internetowy");
const WEB_DESCRIPTION: Text =
    Text::new("B2C store: catalogue, cart, payments and customer account.", "Sklep B2C: katalog, koszyk, płatności i konto klienta.");
const API_NAME: Text = Text::new("API platform", "Platforma API");
const API_DESCRIPTION: Text = Text::new("Public API for partners and the mobile app.", "Publiczne API dla partnerów i aplikacji mobilnej.");

const WEB_COMPONENTS: &[Text] = &[Text::same("Frontend"), Text::new("Cart", "Koszyk"), Text::new("Payments", "Płatności")];

/// The sprints of the shop: the name and the goal.
const WEB_SPRINTS: &[(&str, Text)] = &[
    ("WEB Sprint 1", Text::new("A customer can place an order and pay on delivery", "Klient może złożyć zamówienie z płatnością przy odbiorze")),
    ("WEB Sprint 2", Text::new("Online payments and the basics of the customer account", "Płatności online i podstawy konta klienta")),
    ("WEB Sprint 3", Text::new("Promotions and the wishlist", "Promocje i lista życzeń")),
];

/// The versions of the shop: the name, what is in it, and the day of the release.
const WEB_VERSIONS: &[(&str, Text, i64)] = &[
    ("1.0", Text::new("First release: cart, address, sign-in", "Pierwsze wydanie: koszyk, adres, logowanie"), -6),
    ("1.1", Text::new("Online payments, order history", "Płatności online, historia zamówień"), 10),
    ("2.0", Text::new("Promotions, wishlist", "Promocje, lista życzeń"), 45),
];

struct Comment {
    /// Index into the demo people; 0 is whoever set the instance up.
    author: usize,
    day: i64,
    key: &'static str,
    body: Text,
}

const WEB_COMMENTS: &[Comment] = &[
    Comment {
        author: 3,
        day: -5,
        key: "WEB-9",
        body: Text::new(
            "The gateway in test mode declines cards without 3-D Secure. @emma.clarke do we have the sandbox keys yet?",
            "Bramka w trybie testowym odrzuca karty bez 3-D Secure. @anna.nowak czy mamy już klucze do sandboxa?",
        ),
    },
    Comment {
        author: 1,
        day: -4,
        key: "WEB-9",
        body: Text::new(
            "The keys are in the team's password manager. The card form is done; 3-D Secure and the webhook are left.",
            "Klucze są w menedżerze haseł zespołu. Formularz karty gotowy, zostaje 3-D Secure i webhook.",
        ),
    },
    Comment {
        author: 0,
        day: -1,
        key: "WEB-9",
        body: Text::new(
            "I reviewed the PR: two remarks on error handling, the rest is fine.",
            "Przejrzałem PR: dwie uwagi do obsługi błędów, reszta OK.",
        ),
    },
    Comment {
        author: 2,
        day: -2,
        key: "WEB-10",
        body: Text::new(
            "Pay by bank needs a notification channel of its own; I wrote it up in the knowledge base.",
            "BLIK wymaga osobnego kanału powiadomień; rozpisałem to w bazie wiedzy.",
        ),
    },
    Comment {
        author: 3,
        day: -3,
        key: "WEB-11",
        body: Text::new("Reproduced in production. The fix is waiting for review.", "Odtworzone na produkcji. Poprawka czeka na review."),
    },
    Comment {
        author: 4,
        day: -2,
        key: "WEB-13",
        body: Text::new(
            "Before I start the migration I need a maintenance window. I suggest Thursday after 10 pm.",
            "Zanim ruszę migrację, potrzebuję okna serwisowego. Proponuję czwartek po 22:00.",
        ),
    },
];

const API_COMMENTS: &[Comment] = &[Comment {
    author: 2,
    day: -2,
    key: "API-4",
    body: Text::new(
        "The fix counts the limit per key; I added a test with two keys from one address.",
        "Poprawka liczy limit per klucz; dodałem test z dwoma kluczami z jednego adresu.",
    ),
}];

/// Time logged on the card payment: minutes and what they went on.
const WEB_WORKLOGS: &[(i64, Text)] = &[
    (6 * 60, Text::new("Card form and validation", "Formularz karty i walidacja")),
    (3 * 60 + 30, Text::new("Integration with the gateway's sandbox", "Integracja z sandboxem bramki")),
];

fn seed_comments(app: &App, language: DemoLanguage, people: &[Id], comments: &[Comment]) -> ApiResult<()> {
    for comment in comments {
        app.write_at(Actor::user(people[comment.author]), at(comment.day, 2), |tx| {
            let (issue_id, _) = issue_id_by_key(tx, comment.key)?.ok_or_else(|| ApiError::not_found("Issue"))?;

            add_comment(tx, issue_id, comment.body.pick(language)).map(|_| ())
        })?;
    }

    Ok(())
}

/// The demo people: whoever set the instance up, then the team.
fn seed_people(app: &App, language: DemoLanguage, owner_id: Id) -> ApiResult<Vec<Id>> {
    let mut people = vec![owner_id];

    for person in TEAM {
        let new = NewUser {
            name: person.name.pick(language).to_string(),
            email: person.email.pick(language).to_string(),
            password: None,
            role: Some(Role::Member),
            github_login: Some(person.github_login.pick(language).to_string()),
        };
        people.push(app.db.write(|tx| create_user(tx, &new))?.id);
    }

    Ok(people)
}

/// The components, sprints and versions of the shop; answers the ids of the last two.
fn seed_web_plan(app: &App, language: DemoLanguage, admin: &AuthUser) -> ApiResult<(Vec<Id>, Vec<Id>)> {
    app.write_at(admin.actor(), at(-29, 0), |tx| {
        let access = project_access(tx, admin, "WEB")?;

        for name in WEB_COMPONENTS {
            add_component(tx, &access, &ComponentInput { name: Some(name.pick(language).to_string()), ..Default::default() })?;
        }

        let mut sprints = Vec::new();

        for (name, goal) in WEB_SPRINTS {
            let input = SprintInput { name: Some(name.to_string()), goal: Some(goal.pick(language).to_string()), ..Default::default() };
            sprints.push(create_sprint(tx, &access, &input)?.id);
        }

        let mut versions = Vec::new();

        for (name, description, release) in WEB_VERSIONS {
            let input = VersionInput {
                name: Some(name.to_string()),
                description: Some(description.pick(language).to_string()),
                release_date: Some(date(*release)),
                ..Default::default()
            };
            versions.push(create_version(tx, &access, &input)?.id);
        }

        Ok((sprints, versions))
    })
}

/// Links, due dates and logged time around the card payment.
fn seed_web_details(app: &App, language: DemoLanguage, admin: &AuthUser, people: &[Id]) -> ApiResult<()> {
    app.write_at(admin.actor(), at(-4, 1), |tx| {
        let id_of = |key: &str| -> ApiResult<Id> { Ok(issue_id_by_key(tx, key)?.ok_or_else(|| ApiError::not_found("Issue"))?.0) };

        add_link(tx, admin, id_of("WEB-9")?, LinkType::Blocks, "WEB-16")?;
        add_link(tx, admin, id_of("WEB-11")?, LinkType::Relates, "WEB-6")?;
        add_link(tx, admin, id_of("WEB-10")?, LinkType::Relates, "WEB-9")?;
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

        for (minutes, note) in WEB_WORKLOGS {
            add_worklog(tx, issue_id, *minutes, None, note.pick(language))?;
        }

        Ok(())
    })
}

/// WEB: a scrum project in its second sprint.
fn seed_web(app: &App, language: DemoLanguage, admin: &AuthUser, people: &[Id]) -> ApiResult<()> {
    let web = app.write_at(admin.actor(), at(-30, 0), |tx| {
        create_project(
            tx,
            admin,
            &NewProject {
                key: "WEB".to_string(),
                name: WEB_NAME.pick(language).to_string(),
                description: WEB_DESCRIPTION.pick(language).to_string(),
                board_mode: Some(BoardMode::Scrum),
                is_private: false,
                lead_id: None,
            },
        )
    })?;
    let web_id = web.summary.id;
    let (sprints, versions) = seed_web_plan(app, language, admin)?;

    seed_issues(app, language, &Target { project_key: "WEB", project_id: web_id, people, sprints: &sprints, versions: &versions }, WEB)?;

    app.write_at(admin.actor(), at(-20, 6), |tx| {
        start_sprint(tx, admin, sprints[0], &SprintInput { start_date: Some(date(-20)), end_date: Some(date(-6)), ..Default::default() })
    })?;
    seed_moves(app, "WEB", web_id, WEB, people, -20..-6)?;
    app.write_at(admin.actor(), at(-6, 6), |tx| complete_sprint(tx, admin, sprints[0], None))?;
    app.write_at(admin.actor(), at(-6, 5), |tx| set_released(tx, admin, versions[0], true, None))?;
    app.write_at(admin.actor(), at(-5, 6), |tx| {
        start_sprint(tx, admin, sprints[1], &SprintInput { start_date: Some(date(-5)), end_date: Some(date(9)), ..Default::default() })
    })?;
    seed_moves(app, "WEB", web_id, WEB, people, -6..1)?;

    seed_comments(app, language, people, WEB_COMMENTS)?;
    seed_web_details(app, language, admin, people)
}

/// API: a kanban project.
fn seed_api(app: &App, language: DemoLanguage, admin: &AuthUser, people: &[Id]) -> ApiResult<()> {
    let api = app.write_at(admin.actor(), at(-21, 0), |tx| {
        create_project(
            tx,
            admin,
            &NewProject {
                key: "API".to_string(),
                name: API_NAME.pick(language).to_string(),
                description: API_DESCRIPTION.pick(language).to_string(),
                board_mode: Some(BoardMode::Kanban),
                is_private: false,
                lead_id: Some(people[4]),
            },
        )
    })?;
    let api_id = api.summary.id;

    seed_issues(app, language, &Target { project_key: "API", project_id: api_id, people, sprints: &[], versions: &[] }, API)?;
    seed_moves(app, "API", api_id, API, people, -20..1)?;
    seed_comments(app, language, people, API_COMMENTS)
}

/// Fills an instance that has exactly one user (whoever just set it up) with example data,
/// written in `language`.
pub fn seed_demo(app: &App, owner_id: Id, language: DemoLanguage) -> ApiResult<()> {
    let owner = app.read(|connection| get_user(connection, owner_id))?;
    let admin =
        AuthUser { id: owner.id, name: owner.name.clone(), email: owner.email.clone(), role: Role::Admin, via_token: false, through: Vec::new() };
    let people = seed_people(app, language, owner.id)?;

    seed_web(app, language, &admin, &people)?;
    seed_api(app, language, &admin, &people)?;

    for module in app.enabled_modules() {
        module.seed_demo(app, language)?;
    }

    // The demo appears all at once, tasks of the schedule included. Without this, a demo
    // made in the afternoon would run its morning's tasks a minute later.
    Ticker::new(app, Utc::now).skip_due()
}
