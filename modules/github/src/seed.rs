//! Example repositories for a demo instance, fed through the same code that handles
//! webhooks. The workflow rules are not run, so the demo shows statuses and code disagreeing.
//!
//! Titles, branch names and commit messages come in both languages of the demo; numbers,
//! dates and commit SHAs are the same in both.

use chrono::{Duration, SecondsFormat, Utc};
use hub_app::db::{query_all, query_one};
use hub_app::{Actor, ApiResult, App};
use hub_core::demo::{DemoLanguage, DemoText as Text};
use hub_core::Id;
use serde_json::{json, Value};

use crate::drift::recompute_all;
use crate::ingest::ingest;
use crate::routes::connect_repo;

fn at(day: i64, hour: i64) -> String {
    (Utc::now() + Duration::days(day) - Duration::hours(hour % 6) - Duration::minutes(10)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The shop's repository is named in the language of the demo, the platform's reads the same in both.
const WEB_REPO: Text = Text::new("acme/store-web", "acme/sklep-web");
const API_REPO: Text = Text::same("acme/platform-api");

struct Pull {
    repo: Text,
    number: i64,
    title: Text,
    branch: Text,
    /// Index into the demo people; 0 is whoever set the instance up.
    author: usize,
    /// Commit messages on the branch.
    commits: &'static [Text],
    opened: i64,
    /// The day it was merged, if it was.
    merged: Option<i64>,
    draft: bool,
    review: Option<&'static str>,
    checks: Option<&'static str>,
}

const BASE: Pull = Pull {
    repo: WEB_REPO,
    number: 0,
    title: Text::EMPTY,
    branch: Text::EMPTY,
    author: 0,
    commits: &[],
    opened: 0,
    merged: None,
    draft: false,
    review: None,
    checks: None,
};

const PULLS: &[Pull] = &[
    Pull {
        number: 41,
        title: Text::new("WEB-4 Cart: add and remove products", "WEB-4 Koszyk: dodawanie i usuwanie produktów"),
        branch: Text::new("feature/WEB-4-cart", "feature/WEB-4-koszyk"),
        author: 1,
        commits: &[
            Text::new("WEB-4 Cart: model and endpoints", "WEB-4 Koszyk: model i endpointy"),
            Text::new("WEB-4 Cart: view and tests", "WEB-4 Koszyk: widok i testy"),
        ],
        opened: -17,
        merged: Some(-15),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 44,
        title: Text::new("WEB-6 Cart total includes the item's discount", "WEB-6 Suma koszyka uwzględnia rabat pozycji"),
        branch: Text::new("fix/WEB-6-cart-discount", "fix/WEB-6-rabat-w-koszyku"),
        author: 3,
        commits: &[Text::new("WEB-6 Item discount in the cart total", "WEB-6 Rabat pozycji w sumie koszyka")],
        opened: -12,
        merged: Some(-11),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 47,
        title: Text::new("WEB-8 Sign-up and sign-in by e-mail", "WEB-8 Rejestracja i logowanie e-mailem"),
        branch: Text::new("feature/WEB-8-sign-in", "feature/WEB-8-logowanie"),
        author: 0,
        commits: &[
            Text::new("WEB-8 Sign-up form", "WEB-8 Formularz rejestracji"),
            Text::new("WEB-8 Session and sign-out", "WEB-8 Sesja i wylogowanie"),
        ],
        opened: -10,
        merged: Some(-8),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 52,
        title: Text::new("WEB-9 Card payment through the gateway", "WEB-9 Płatność kartą przez bramkę"),
        branch: Text::new("feature/WEB-9-card-payment", "feature/WEB-9-platnosc-karta"),
        author: 1,
        commits: &[
            Text::new("WEB-9 Card form", "WEB-9 Formularz karty"),
            Text::new("WEB-9 Payment gateway client", "WEB-9 Klient bramki płatniczej"),
            Text::new("WEB-9 Handle a declined card", "WEB-9 Obsługa odrzuconej karty"),
        ],
        opened: -2,
        review: Some("changes_requested"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 53,
        title: Text::new("WEB-11 Order summary: handle an empty cart", "WEB-11 Podsumowanie: obsługa pustego koszyka"),
        branch: Text::new("fix/WEB-11-empty-cart", "fix/WEB-11-pusty-koszyk"),
        author: 3,
        commits: &[Text::new("WEB-11 An empty cart no longer causes error 500", "WEB-11 Pusty koszyk nie wywołuje błędu 500")],
        opened: -3,
        merged: Some(-2),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 55,
        title: Text::new("WEB-15 Postcode validation", "WEB-15 Walidacja kodu pocztowego"),
        branch: Text::new("fix/WEB-15-postcode", "fix/WEB-15-kod-pocztowy"),
        author: 2,
        commits: &[Text::new("WEB-15 Pattern for postcodes without a space", "WEB-15 Wyrażenie dla kodów z myślnikiem")],
        opened: -1,
        checks: Some("failure"),
        ..BASE
    },
    Pull {
        number: 56,
        title: Text::new("WEB-14 Password reset", "WEB-14 Reset hasła"),
        branch: Text::new("feature/WEB-14-password-reset", "feature/WEB-14-reset-hasla"),
        author: 0,
        commits: &[Text::new("WEB-14 Reset link and new password form", "WEB-14 Link resetujący i formularz nowego hasła")],
        opened: -4,
        merged: Some(-3),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 57,
        title: Text::new("WEB-12 Order history in the profile", "WEB-12 Historia zamówień w profilu"),
        branch: Text::new("feature/WEB-12-order-history", "feature/WEB-12-historia-zamowien"),
        author: 0,
        commits: &[Text::new("WEB-12 Order list", "WEB-12 Lista zamówień"), Text::new("WEB-12 Order details", "WEB-12 Szczegóły zamówienia")],
        opened: -1,
        checks: Some("pending"),
        ..BASE
    },
    Pull {
        number: 58,
        title: Text::new("WEB-13 Move sessions to Redis", "WEB-13 Migracja sesji do Redis"),
        branch: Text::new("feature/WEB-13-redis-sessions", "feature/WEB-13-sesje-redis"),
        author: 4,
        commits: &[Text::new("WEB-13 Session store in Redis (draft)", "WEB-13 Magazyn sesji w Redis (szkic)")],
        opened: -1,
        draft: true,
        ..BASE
    },
    Pull {
        number: 59,
        title: Text::new("Update frontend dependencies", "Aktualizacja zależności frontendu"),
        branch: Text::new("chore/dependencies", "chore/zaleznosci"),
        author: 4,
        commits: &[],
        opened: -2,
        checks: Some("success"),
        ..BASE
    },
    Pull {
        repo: API_REPO,
        number: 12,
        title: Text::new("API-2 Order list with pagination", "API-2 Lista zamówień ze stronicowaniem"),
        branch: Text::new("feature/API-2-order-list", "feature/API-2-lista-zamowien"),
        author: 4,
        commits: &[Text::new("API-2 Cursor pagination", "API-2 Stronicowanie kursorem")],
        opened: -9,
        merged: Some(-7),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        repo: API_REPO,
        number: 15,
        title: Text::new("API-4 Rate limit per API key", "API-4 Limit zapytań per klucz API"),
        branch: Text::new("fix/API-4-limit-per-key", "fix/API-4-limit-per-klucz"),
        author: 2,
        commits: &[
            Text::new("API-4 Counter per key", "API-4 Licznik per klucz"),
            Text::new("API-4 Test with two keys from one address", "API-4 Test dwóch kluczy z jednego adresu"),
        ],
        opened: -2,
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
];

/// A branch somebody pushed to without opening a pull request yet.
struct Branch {
    repo: Text,
    name: Text,
    author: usize,
    commits: &'static [Text],
    day: i64,
}

const LOOSE_BRANCHES: &[Branch] = &[
    Branch {
        repo: WEB_REPO,
        name: Text::new("feature/WEB-10-pay-by-bank", "feature/WEB-10-platnosc-blik"),
        author: 2,
        commits: &[
            Text::new("WEB-10 Pay by bank client", "WEB-10 Klient BLIK"),
            Text::new("WEB-10 Payment notification channel", "WEB-10 Kanał powiadomień o płatności"),
        ],
        day: -2,
    },
    Branch {
        repo: API_REPO,
        name: Text::new("feature/API-3-status-webhook", "feature/API-3-webhook-statusu"),
        author: 4,
        commits: &[Text::new("API-3 Order status event queue", "API-3 Kolejka zdarzeń statusu zamówienia")],
        day: -3,
    },
];

fn sha(seed: &str) -> String {
    hub_app::auth::digest(seed)[..40].to_string()
}

/// The SHA of a demo commit. It is made from the Polish texts, so a commit has the same
/// SHA whichever language the demo is written in.
fn commit_sha(repo: Text, branch: Text, message: &str) -> String {
    sha(&format!("{}{}{message}", repo.pl, branch.pl))
}

fn push(language: DemoLanguage, repo: Text, branch: Text, author: &str, messages: &[Text], day: i64) -> Value {
    let repo_name = repo.pick(language);
    let commits: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let id = commit_sha(repo, branch, message.pl);

            json!({
                "id": id,
                "message": message.pick(language),
                "timestamp": at(day, index as i64),
                "url": format!("https://github.com/{repo_name}/commit/{id}"),
                "author": { "name": author, "username": author, "email": format!("{author}@users.noreply.github.com") },
            })
        })
        .collect();

    json!({
        "ref": format!("refs/heads/{}", branch.pick(language)),
        "created": true,
        "deleted": false,
        "after": commits.last().map(|commit| commit["id"].clone()).unwrap_or_else(|| json!(sha(branch.pl))),
        "repository": { "full_name": repo_name },
        "commits": commits,
    })
}

fn pull_request(language: DemoLanguage, pull: &Pull, author: &str, action: &str) -> Value {
    let merged = action == "closed";
    let repo = pull.repo.pick(language);
    let head = commit_sha(pull.repo, pull.branch, pull.commits.last().unwrap_or(&pull.branch).pl);

    json!({
        "action": action,
        "repository": { "full_name": repo },
        "pull_request": {
            "number": pull.number,
            "title": pull.title.pick(language),
            "body": "",
            "state": if merged { "closed" } else { "open" },
            "draft": pull.draft,
            "merged": merged,
            "merged_at": pull.merged.filter(|_| merged).map(|day| at(day, 1)),
            "closed_at": pull.merged.filter(|_| merged).map(|day| at(day, 1)),
            "user": { "login": author },
            "head": { "ref": pull.branch.pick(language), "sha": head },
            "base": { "ref": "main" },
            "html_url": format!("https://github.com/{repo}/pull/{}", pull.number),
            "additions": 40 + pull.number * 7 % 300,
            "deletions": 5 + pull.number * 3 % 80,
            "created_at": at(pull.opened, 3),
            "updated_at": at(pull.merged.unwrap_or(pull.opened.max(-1)), 1),
        },
    })
}

/// The GitHub logins of the demo people, in the order the demo made them. Whoever set the
/// instance up gets a login first, so "my pull requests" has something to show.
fn demo_logins(app: &App) -> ApiResult<Vec<String>> {
    app.db.write(|tx| {
        tx.execute("UPDATE user SET github_login = lower(replace(substr(email, 1, instr(email, '@') - 1), '.', '-')) WHERE id = (SELECT min(id) FROM user) AND github_login IS NULL", [])?;

        query_all(tx, "SELECT github_login FROM user WHERE github_login IS NOT NULL ORDER BY id LIMIT 5", [], |row| row.get(0))
    })
}

pub fn seed_demo(app: &App, language: DemoLanguage) -> ApiResult<()> {
    let project = |key: &str| -> ApiResult<Option<Id>> {
        app.read(|connection| query_one(connection, "SELECT id FROM project WHERE key = ?1", [key], |row| row.get(0)))
    };
    let (Some(web), Some(api)) = (project("WEB")?, project("API")?) else { return Ok(()) };
    let logins = demo_logins(app)?;
    let login = |person: usize| logins[person % logins.len()].as_str();

    app.db.write(|tx| {
        connect_repo(tx, web, WEB_REPO.pick(language))?;
        connect_repo(tx, api, API_REPO.pick(language))?;

        Ok(())
    })?;

    let deliver =
        |day: i64, event: &str, payload: &Value| app.write_at(Actor::system("GitHub"), at(day, 2), |tx| ingest(tx, event, payload).map(|_| ()));

    for pull in PULLS {
        let author = login(pull.author);

        if !pull.commits.is_empty() {
            deliver(pull.opened, "push", &push(language, pull.repo, pull.branch, author, pull.commits, pull.opened))?;
        }

        deliver(pull.opened, "pull_request", &pull_request(language, pull, author, "opened"))?;

        if let Some(review) = pull.review {
            let mut payload = pull_request(language, pull, author, "submitted");
            payload["review"] = json!({ "state": review });
            deliver(pull.merged.unwrap_or(pull.opened), "pull_request_review", &payload)?;
        }

        if let Some(checks) = pull.checks {
            let head = pull_request(language, pull, author, "opened")["pull_request"]["head"]["sha"].clone();
            let suite = if checks == "pending" {
                json!({ "head_sha": head, "status": "in_progress" })
            } else {
                json!({ "head_sha": head, "status": "completed", "conclusion": checks })
            };

            deliver(pull.opened, "check_suite", &json!({ "repository": { "full_name": pull.repo.pick(language) }, "check_suite": suite }))?;
        }

        if let Some(day) = pull.merged {
            deliver(day, "pull_request", &pull_request(language, pull, author, "closed"))?;
        }
    }

    for branch in LOOSE_BRANCHES {
        deliver(branch.day, "push", &push(language, branch.repo, branch.name, login(branch.author), branch.commits, branch.day))?;
    }

    app.db.write(|tx| recompute_all(tx).map(|_| ()))
}
