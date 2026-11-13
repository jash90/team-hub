//! The example data comes in English and in Polish: one data set, written in two languages.
//! The structure must be the same in both, and the English one must not have a Polish word
//! anywhere a person reads.

mod common;

use std::collections::BTreeMap;

use axum::http::{Method, StatusCode};
use common::{keys, test_app, Client};
use hub_server::{build_router, modules};
use serde_json::{json, Value};

const POLISH_LETTERS: &str = "ąćęłńóśźżĄĆĘŁŃÓŚŹŻ";
/// Pieces of words the Polish demo is full of, in lower case, with and without Polish
/// letters (branch names and file names are written without them).
const POLISH_WORDS: &[&str] = &[
    "zamówie",
    "zamowie",
    "koszyk",
    "hasł",
    "hasl",
    "płatno",
    "platno",
    "sklep",
    "zgłosze",
    "klient",
    "bramk",
    "rabat",
    "logowani",
    "wdroż",
    "wdroz",
    "spotka",
    "pusty",
    "pocztow",
    "zalezn",
    "blik",
    "nowak",
    "zieli",
    "kowalczyk",
    "wisniewsk",
];

/// A fresh instance with every module, set up with example data. Without a language the
/// request is the one every earlier client sends.
async fn demo(admin: &str, language: Option<&str>) -> Client {
    let mut client = Client::new(build_router(test_app(modules())));
    let mut request = json!({ "name": admin, "email": "admin@example.com", "password": "correct horse", "demo": true });

    if let Some(language) = language {
        request["demoLanguage"] = json!(language);
    }

    client.post("/api/setup", request).await;

    client
}

fn array(value: &Value) -> &Vec<Value> {
    value.as_array().unwrap_or_else(|| panic!("an array, not {value}"))
}

/// The names of a project's things (statuses, types, sprints) by their ids.
fn names(project: &Value, of: &str) -> BTreeMap<i64, String> {
    array(&project[of]).iter().map(|item| (item["id"].as_i64().unwrap(), item["name"].as_str().unwrap().to_string())).collect()
}

/// The people in the order they were made: whoever set the instance up, then the team.
async fn people(client: &mut Client) -> Vec<(i64, String)> {
    let mut people: Vec<(i64, String)> = array(&client.get("/api/users").await)
        .iter()
        .map(|user| (user["id"].as_i64().unwrap(), user["name"].as_str().unwrap().to_string()))
        .collect();
    people.sort();

    people
}

fn names_of(people: Vec<(i64, String)>) -> Vec<String> {
    people.into_iter().map(|(_, name)| name).collect()
}

/// What an issue is, leaving out every text of the example data: its key, status, type,
/// where its assignee stands among the people, its sprint, parent, points and priority.
async fn issue_shapes(client: &mut Client) -> Vec<Value> {
    let people: Vec<i64> = people(client).await.into_iter().map(|(id, _)| id).collect();
    let mut shapes = Vec::new();

    for key in keys(&client.get("/api/projects").await) {
        let project = client.get(&format!("/api/projects/{key}")).await;
        let (statuses, types, sprints) = (names(&project, "statuses"), names(&project, "types"), names(&project, "sprints"));
        let found = client.get(&format!("/api/search?q=project%20%3D%20{key}%20ORDER%20BY%20created%2C%20key&limit=200")).await;

        for issue in array(&found["issues"]) {
            shapes.push(json!({
                "key": issue["key"],
                "status": statuses[&issue["statusId"].as_i64().unwrap()],
                "type": types[&issue["typeId"].as_i64().unwrap()],
                "assignee": issue["assigneeId"].as_i64().map(|id| people.iter().position(|person| *person == id)),
                "sprint": issue["sprintId"].as_i64().map(|id| sprints[&id].clone()),
                "parent": issue["parentKey"],
                "points": issue["points"],
                "priority": issue["priority"],
                "labels": array(&issue["labels"]).len(),
                "components": array(&issue["componentIds"]).len(),
                "comments": issue["commentCount"],
            }));
        }
    }

    shapes.sort_by_key(|shape| shape["key"].as_str().unwrap().to_string());

    shapes
}

/// Everything the main pages of the demo show, as the API answers it.
async fn everything(client: &mut Client) -> Vec<(String, Value)> {
    let anywhere =
        ["/api/bootstrap", "/api/users", "/api/projects", "/api/dashboard", "/api/activity?limit=200", "/api/notifications", "/api/m/wiki/spaces"];
    let mut paths: Vec<String> = anywhere.into_iter().map(String::from).collect();

    for key in keys(&client.get("/api/projects").await) {
        for page in ["", "/board", "/backlog", "/tasks"] {
            paths.push(format!("/api/projects/{key}{page}"));
        }

        for module in ["github", "e2e", "timeline"] {
            paths.push(format!("/api/m/{module}/projects/{key}"));
        }

        for report in ["velocity", "cumulative-flow", "created-vs-resolved", "cycle-time", "workload"] {
            paths.push(format!("/api/m/reports/projects/{key}/{report}"));
        }

        let found = client.get(&format!("/api/search?q=project%20%3D%20{key}&limit=200")).await;
        paths.extend(keys(&found["issues"]).iter().map(|issue| format!("/api/issues/{issue}")));
        paths.push(format!("/api/search?q=project%20%3D%20{key}&limit=200"));
    }

    for space in keys(&client.get("/api/m/wiki/spaces").await) {
        paths.push(format!("/api/m/wiki/spaces/{space}/tree"));

        for page in array(&client.get(&format!("/api/m/wiki/spaces/{space}/tree")).await) {
            let revisions = format!("/api/m/wiki/pages/{}/revisions", page["id"]);

            paths.push(format!("/api/m/wiki/pages/{}", page["id"]));
            paths.extend(array(&client.get(&revisions).await).iter().map(|revision| format!("{revisions}/{}", revision["version"])));
            paths.push(revisions);
        }
    }

    for run in array(&client.get("/api/m/e2e/projects/WEB").await["runs"]) {
        paths.push(format!("/api/m/e2e/runs/{}", run["id"]));
    }

    let mut answers = Vec::new();

    for path in paths {
        let answer = client.get(&path).await;
        answers.push((path, answer));
    }

    answers
}

/// Every text in a JSON value, keys of objects included, with where it was found.
fn texts(value: &Value, at: &str, found: &mut Vec<(String, String)>) {
    match value {
        Value::String(text) => found.push((at.to_string(), text.clone())),
        Value::Array(items) => items.iter().enumerate().for_each(|(index, item)| texts(item, &format!("{at}[{index}]"), found)),
        Value::Object(fields) => fields.iter().for_each(|(name, field)| {
            found.push((at.to_string(), name.clone()));
            texts(field, &format!("{at}.{name}"), found);
        }),
        _ => {}
    }
}

fn polish_in(text: &str) -> Option<String> {
    let lower = text.to_lowercase();

    text.chars()
        .find(|letter| POLISH_LETTERS.contains(*letter))
        .map(|letter| letter.to_string())
        .or_else(|| POLISH_WORDS.iter().find(|word| lower.contains(**word)).map(|word| word.to_string()))
}

/// How many of each thing the modules made.
async fn counts(client: &mut Client) -> Value {
    let mut pages = 0;
    let mut revisions = 0;

    for space in keys(&client.get("/api/m/wiki/spaces").await) {
        for page in array(&client.get(&format!("/api/m/wiki/spaces/{space}/tree")).await) {
            pages += 1;
            revisions += array(&client.get(&format!("/api/m/wiki/pages/{}/revisions", page["id"])).await).len();
        }
    }

    let (mut pulls, mut commits, mut branches) = (Vec::new(), Vec::new(), 0);

    for key in keys(&client.get("/api/projects").await) {
        let code = client.get(&format!("/api/m/github/projects/{key}")).await;
        pulls.extend(array(&code["pulls"]).iter().map(|pull| json!([key, pull["number"], pull["state"], pull["draft"]])));

        for issue in keys(&client.get(&format!("/api/search?q=project%20%3D%20{key}&limit=200")).await["issues"]) {
            let panel = client.get(&format!("/api/issues/{issue}")).await["panels"]["github"].clone();
            commits.extend(array(&panel["commits"]).iter().map(|commit| commit["sha"].clone()));
            branches += array(&panel["branches"]).len();
        }
    }

    let tests = client.get("/api/m/e2e/projects/WEB").await;
    let tasks = client.get("/api/projects/WEB/tasks").await;
    let timeline = client.get("/api/m/timeline/projects/WEB").await;

    commits.sort_by_key(|sha| sha.as_str().unwrap_or("").to_string());

    json!({
        "pages": pages,
        "revisions": revisions,
        "pulls": pulls,
        "commits": commits,
        "branches": branches,
        "runs": array(&tests["runs"]).iter().map(|run| json!([run["name"], run["status"], run["prNumber"]])).collect::<Vec<_>>(),
        "tests": tests["counts"],
        "tasks": array(&tasks).iter().map(|task| json!([task["slot"], task["kind"], task["state"]])).collect::<Vec<_>>(),
        "epics": array(&timeline["epics"]).iter().map(|epic| json!([epic["key"], epic["start"], epic["due"]])).collect::<Vec<_>>(),
    })
}

/// The seeds agree on the names they share: a test run of a pull request is on that pull
/// request's branch, in that repository. Answers how many such runs there are.
async fn runs_on_pull_requests(client: &mut Client) -> usize {
    let code = client.get("/api/m/github/projects/WEB").await;
    let repo = code["repos"][0]["fullName"].as_str().expect("a repository").to_string();
    let tests = client.get("/api/m/e2e/projects/WEB").await;
    let mut seen = 0;

    for run in array(&tests["runs"]) {
        assert!(run["ciUrl"].as_str().unwrap().starts_with(&format!("https://github.com/{repo}/")), "{run}");

        if let Some(number) = run["prNumber"].as_i64() {
            let pull = array(&code["pulls"]).iter().find(|pull| pull["number"] == number).unwrap_or_else(|| panic!("pull request {number}"));

            assert_eq!(run["branch"], pull["headRef"], "run {} of pull request {number}", run["name"]);
            seen += 1;
        }
    }

    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn the_demo_is_one_data_set_in_two_languages() {
    let mut english = demo("Alex Morgan", Some("en")).await;
    let mut polish = demo("Alex Morgan", None).await;

    assert_eq!(keys(&english.get("/api/projects").await), ["API", "WEB"]);
    assert_eq!(keys(&polish.get("/api/projects").await), ["API", "WEB"]);

    let shapes = issue_shapes(&mut english).await;
    assert_eq!(shapes.len(), 37, "29 issues of the shop and 8 of the platform");
    assert_eq!(shapes, issue_shapes(&mut polish).await);

    let made = counts(&mut english).await;
    assert_eq!((made["pages"].clone(), made["revisions"].clone()), (json!(13), json!(15)), "{made}");
    assert_eq!((array(&made["pulls"]).len(), array(&made["runs"]).len(), array(&made["tasks"]).len()), (12, 8, 3), "{made}");
    assert!(array(&made["commits"]).len() >= 19, "every commit of the demo is on an issue: {made}");
    assert_eq!(made, counts(&mut polish).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_english_demo_has_no_polish_in_it() {
    let mut english = demo("Alex Morgan", Some("en")).await;
    let mut found = Vec::new();

    for (path, answer) in everything(&mut english).await {
        texts(&answer, &path, &mut found);
    }

    assert!(found.len() > 5_000, "the walk reads the whole demo, not {} texts", found.len());

    let polish: Vec<String> =
        found.iter().filter_map(|(at, text)| polish_in(text).map(|what| format!("{at}: \"{what}\" in \"{text}\""))).take(20).collect();
    assert!(polish.is_empty(), "Polish in the English demo:\n{}", polish.join("\n"));

    // The same walk does find Polish in the Polish demo, so it looks in the right places.
    let mut polish_demo = demo("Alex Morgan", Some("pl")).await;
    let mut found = Vec::new();

    for (path, answer) in everything(&mut polish_demo).await {
        texts(&answer, &path, &mut found);
    }

    assert!(found.iter().filter(|(_, text)| polish_in(text).is_some()).count() > 500);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_english_demo_reads_like_an_english_team() {
    let mut client = demo("Alex Morgan", Some("en")).await;
    let people = names_of(people(&mut client).await);
    let projects = client.get("/api/projects").await;

    assert_eq!(people, ["Alex Morgan", "Emma Clarke", "James Bennett", "Olivia Hughes", "Daniel Foster"]);
    assert_eq!((projects[0]["name"].as_str(), projects[1]["name"].as_str()), (Some("API platform"), Some("Online store")));
    assert_eq!(client.get("/api/issues/WEB-14").await["summary"], "Password reset");

    // A comment mentions a person by the handle they have in this language.
    let card = client.get("/api/issues/WEB-9").await;
    assert!(array(&card["comments"]).iter().any(|comment| comment["body"].as_str().unwrap().contains("@emma.clarke")), "{}", card["comments"]);

    assert_eq!(client.get("/api/m/github/projects/WEB").await["repos"][0]["fullName"], "acme/store-web");
    assert_eq!(runs_on_pull_requests(&mut client).await, 3);
    assert_eq!(client.get("/api/issues/WEB-9").await["panels"]["github"]["prs"][0]["headRef"], "feature/WEB-9-card-payment");
    assert_eq!(client.get("/api/issues/WEB-10").await["panels"]["github"]["branches"][0]["name"], "feature/WEB-10-pay-by-bank");
    assert!(client.get("/api/issues/WEB-10").await["panels"]["wiki"]["pages"].as_array().is_some_and(|pages| !pages.is_empty()));
    assert_eq!(client.get("/api/quick?q=pay%20by%20bank").await[0]["id"], "WEB-10");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_demo_without_a_language_is_the_polish_one() {
    let mut client = demo("Bartek Admin", None).await;
    let people = names_of(people(&mut client).await);
    let projects = client.get("/api/projects").await;

    assert_eq!(people, ["Bartek Admin", "Anna Nowak", "Piotr Zieliński", "Maria Wiśniewska", "Tomasz Kowalczyk"]);
    assert_eq!((projects[0]["name"].as_str(), projects[1]["name"].as_str()), (Some("Platforma API"), Some("Sklep internetowy")));
    assert_eq!(client.get("/api/issues/WEB-14").await["summary"], "Reset hasła");
    assert_eq!(client.get("/api/issues/WEB-9").await["summary"], "Płatność kartą przez bramkę");
    assert_eq!(client.get("/api/m/github/projects/WEB").await["repos"][0]["fullName"], "acme/sklep-web");
    assert_eq!(client.get("/api/m/github/projects/WEB").await["pulls"].as_array().map(Vec::len), Some(10));
    assert_eq!(client.get("/api/projects/WEB/tasks").await[0]["name"], "Triage nowych zgłoszeń");
    assert_eq!(client.get("/api/m/wiki/spaces/WEB").await["name"], "Sklep — baza wiedzy");
    assert_eq!(runs_on_pull_requests(&mut client).await, 3);

    // Asking for Polish by name gives the same.
    let mut asked = demo("Bartek Admin", Some("pl")).await;
    assert_eq!(asked.get("/api/projects").await[1]["name"], "Sklep internetowy");
    assert_eq!(issue_shapes(&mut asked).await, issue_shapes(&mut client).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_language_the_demo_is_not_written_in_is_refused() {
    let mut client = Client::new(build_router(test_app(Vec::new())));
    let request = |language: &str, demo: bool| {
        Some(json!({ "name": "Alex Morgan", "email": "admin@example.com", "password": "correct horse", "demo": demo, "demoLanguage": language }))
    };

    assert_eq!(client.refused(Method::POST, "/api/setup", request("de", true)).await, StatusCode::BAD_REQUEST);
    assert_eq!(client.refused(Method::POST, "/api/setup", request("EN", false)).await, StatusCode::BAD_REQUEST);
    assert_eq!(client.get("/api/bootstrap").await["setupRequired"], true, "a refused setup leaves the instance as it was");

    // The language says nothing when no example data is asked for.
    assert_eq!(client.refused(Method::POST, "/api/setup", request("en", false)).await, StatusCode::OK);
    assert_eq!(client.get("/api/projects").await, json!([]));
}
