//! Example repositories for a demo instance, fed through the same code that handles
//! webhooks. The workflow rules are not run, so the demo shows statuses and code disagreeing.

use chrono::{Duration, SecondsFormat, Utc};
use hub_app::db::query_one;
use hub_app::{Actor, ApiResult, App};
use hub_core::Id;
use serde_json::{json, Value};

use crate::drift::recompute_all;
use crate::ingest::ingest;
use crate::routes::connect_repo;

fn at(day: i64, hour: i64) -> String {
    (Utc::now() + Duration::days(day) - Duration::hours(hour % 6) - Duration::minutes(10)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

struct Pull {
    repo: &'static str,
    number: i64,
    title: &'static str,
    branch: &'static str,
    author: &'static str,
    /// Commit messages on the branch.
    commits: &'static [&'static str],
    opened: i64,
    /// The day it was merged, if it was.
    merged: Option<i64>,
    draft: bool,
    review: Option<&'static str>,
    checks: Option<&'static str>,
}

const BASE: Pull = Pull {
    repo: "acme/sklep-web",
    number: 0,
    title: "",
    branch: "",
    author: "",
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
        title: "WEB-4 Koszyk: dodawanie i usuwanie produktów",
        branch: "feature/WEB-4-koszyk",
        author: "anowak",
        commits: &["WEB-4 Koszyk: model i endpointy", "WEB-4 Koszyk: widok i testy"],
        opened: -17,
        merged: Some(-15),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 44,
        title: "WEB-6 Suma koszyka uwzględnia rabat pozycji",
        branch: "fix/WEB-6-rabat-w-koszyku",
        author: "mwisniewska",
        commits: &["WEB-6 Rabat pozycji w sumie koszyka"],
        opened: -12,
        merged: Some(-11),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 47,
        title: "WEB-8 Rejestracja i logowanie e-mailem",
        branch: "feature/WEB-8-logowanie",
        author: "@owner",
        commits: &["WEB-8 Formularz rejestracji", "WEB-8 Sesja i wylogowanie"],
        opened: -10,
        merged: Some(-8),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 52,
        title: "WEB-9 Płatność kartą przez bramkę",
        branch: "feature/WEB-9-platnosc-karta",
        author: "anowak",
        commits: &["WEB-9 Formularz karty", "WEB-9 Klient bramki płatniczej", "WEB-9 Obsługa odrzuconej karty"],
        opened: -2,
        review: Some("changes_requested"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 53,
        title: "WEB-11 Podsumowanie: obsługa pustego koszyka",
        branch: "fix/WEB-11-pusty-koszyk",
        author: "mwisniewska",
        commits: &["WEB-11 Pusty koszyk nie wywołuje błędu 500"],
        opened: -3,
        merged: Some(-2),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 55,
        title: "WEB-15 Walidacja kodu pocztowego",
        branch: "fix/WEB-15-kod-pocztowy",
        author: "pzielinski",
        commits: &["WEB-15 Wyrażenie dla kodów z myślnikiem"],
        opened: -1,
        checks: Some("failure"),
        ..BASE
    },
    Pull {
        number: 56,
        title: "WEB-14 Reset hasła",
        branch: "feature/WEB-14-reset-hasla",
        author: "@owner",
        commits: &["WEB-14 Link resetujący i formularz nowego hasła"],
        opened: -4,
        merged: Some(-3),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        number: 57,
        title: "WEB-12 Historia zamówień w profilu",
        branch: "feature/WEB-12-historia-zamowien",
        author: "@owner",
        commits: &["WEB-12 Lista zamówień", "WEB-12 Szczegóły zamówienia"],
        opened: -1,
        checks: Some("pending"),
        ..BASE
    },
    Pull {
        number: 58,
        title: "WEB-13 Migracja sesji do Redis",
        branch: "feature/WEB-13-sesje-redis",
        author: "tkowalczyk",
        commits: &["WEB-13 Magazyn sesji w Redis (szkic)"],
        opened: -1,
        draft: true,
        ..BASE
    },
    Pull {
        number: 59,
        title: "Aktualizacja zależności frontendu",
        branch: "chore/zaleznosci",
        author: "tkowalczyk",
        commits: &[],
        opened: -2,
        checks: Some("success"),
        ..BASE
    },
    Pull {
        repo: "acme/platform-api",
        number: 12,
        title: "API-2 Lista zamówień ze stronicowaniem",
        branch: "feature/API-2-lista-zamowien",
        author: "tkowalczyk",
        commits: &["API-2 Stronicowanie kursorem"],
        opened: -9,
        merged: Some(-7),
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
    Pull {
        repo: "acme/platform-api",
        number: 15,
        title: "API-4 Limit zapytań per klucz API",
        branch: "fix/API-4-limit-per-klucz",
        author: "pzielinski",
        commits: &["API-4 Licznik per klucz", "API-4 Test dwóch kluczy z jednego adresu"],
        opened: -2,
        review: Some("approved"),
        checks: Some("success"),
        ..BASE
    },
];

const LOOSE_BRANCHES: &[(&str, &str, &str, &[&str], i64)] = &[
    ("acme/sklep-web", "feature/WEB-10-platnosc-blik", "pzielinski", &["WEB-10 Klient BLIK", "WEB-10 Kanał powiadomień o płatności"], -2),
    ("acme/platform-api", "feature/API-3-webhook-statusu", "tkowalczyk", &["API-3 Kolejka zdarzeń statusu zamówienia"], -3),
];

fn sha(seed: &str) -> String {
    hub_app::auth::digest(seed)[..40].to_string()
}

fn push(repo: &str, branch: &str, author: &str, messages: &[&str], day: i64) -> Value {
    let commits: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let id = sha(&format!("{repo}{branch}{message}"));

            json!({
                "id": id,
                "message": message,
                "timestamp": at(day, index as i64),
                "url": format!("https://github.com/{repo}/commit/{id}"),
                "author": { "name": author, "username": author, "email": format!("{author}@users.noreply.github.com") },
            })
        })
        .collect();

    json!({
        "ref": format!("refs/heads/{branch}"),
        "created": true,
        "deleted": false,
        "after": commits.last().map(|commit| commit["id"].clone()).unwrap_or_else(|| json!(sha(branch))),
        "repository": { "full_name": repo },
        "commits": commits,
    })
}

fn pull_request(pull: &Pull, author: &str, action: &str) -> Value {
    let merged = action == "closed";
    let head = sha(&format!("{}{}{}", pull.repo, pull.branch, pull.commits.last().unwrap_or(&pull.branch)));

    json!({
        "action": action,
        "repository": { "full_name": pull.repo },
        "pull_request": {
            "number": pull.number,
            "title": pull.title,
            "body": "",
            "state": if merged { "closed" } else { "open" },
            "draft": pull.draft,
            "merged": merged,
            "merged_at": pull.merged.filter(|_| merged).map(|day| at(day, 1)),
            "closed_at": pull.merged.filter(|_| merged).map(|day| at(day, 1)),
            "user": { "login": author },
            "head": { "ref": pull.branch, "sha": head },
            "base": { "ref": "main" },
            "html_url": format!("https://github.com/{}/pull/{}", pull.repo, pull.number),
            "additions": 40 + pull.number * 7 % 300,
            "deletions": 5 + pull.number * 3 % 80,
            "created_at": at(pull.opened, 3),
            "updated_at": at(pull.merged.unwrap_or(pull.opened.max(-1)), 1),
        },
    })
}

pub fn seed_demo(app: &App) -> ApiResult<()> {
    let project = |key: &str| -> ApiResult<Option<Id>> {
        app.read(|connection| query_one(connection, "SELECT id FROM project WHERE key = ?1", [key], |row| row.get(0)))
    };
    let (Some(web), Some(api)) = (project("WEB")?, project("API")?) else { return Ok(()) };

    // Whoever set the instance up gets a GitHub login, so "my pull requests" has something to show.
    let owner_login = app.db.write(|tx| {
        tx.execute("UPDATE user SET github_login = lower(replace(substr(email, 1, instr(email, '@') - 1), '.', '-')) WHERE id = (SELECT min(id) FROM user) AND github_login IS NULL", [])?;

        Ok(tx.query_row("SELECT github_login FROM user WHERE id = (SELECT min(id) FROM user)", [], |row| row.get::<_, String>(0))?)
    })?;

    app.db.write(|tx| {
        connect_repo(tx, web, "acme/sklep-web")?;
        connect_repo(tx, api, "acme/platform-api")?;

        Ok(())
    })?;

    let deliver =
        |day: i64, event: &str, payload: &Value| app.write_at(Actor::system("GitHub"), at(day, 2), |tx| ingest(tx, event, payload).map(|_| ()));

    for pull in PULLS {
        let author = if pull.author == "@owner" { owner_login.as_str() } else { pull.author };

        if !pull.commits.is_empty() {
            deliver(pull.opened, "push", &push(pull.repo, pull.branch, author, pull.commits, pull.opened))?;
        }

        deliver(pull.opened, "pull_request", &pull_request(pull, author, "opened"))?;

        if let Some(review) = pull.review {
            let mut payload = pull_request(pull, author, "submitted");
            payload["review"] = json!({ "state": review });
            deliver(pull.merged.unwrap_or(pull.opened), "pull_request_review", &payload)?;
        }

        if let Some(checks) = pull.checks {
            let head = pull_request(pull, author, "opened")["pull_request"]["head"]["sha"].clone();
            let suite = if checks == "pending" {
                json!({ "head_sha": head, "status": "in_progress" })
            } else {
                json!({ "head_sha": head, "status": "completed", "conclusion": checks })
            };

            deliver(pull.opened, "check_suite", &json!({ "repository": { "full_name": pull.repo }, "check_suite": suite }))?;
        }

        if let Some(day) = pull.merged {
            deliver(day, "pull_request", &pull_request(pull, author, "closed"))?;
        }
    }

    for (repo, branch, author, messages, day) in LOOSE_BRANCHES {
        deliver(*day, "push", &push(repo, branch, author, messages, *day))?;
    }

    app.db.write(|tx| recompute_all(tx).map(|_| ()))
}
