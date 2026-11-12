//! Issue keys (`WEB-12`): parsing them, finding them in commits, branches and pull requests.

use std::collections::HashSet;
use std::sync::LazyLock;

use rayon::prelude::*;
use regex::Regex;

use crate::text::slugify;

static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Za-z][A-Za-z0-9]{1,9})-(\d{1,7})").expect("valid regex"));
static PROJECT_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z][A-Z0-9]{1,9}$").expect("valid regex"));

pub fn is_project_key(text: &str) -> bool {
    PROJECT_KEY.is_match(text)
}

/// `web-12` → (`WEB`, 12). The whole text must be a key.
pub fn parse_issue_key(text: &str) -> Option<(String, i64)> {
    let text = text.trim();
    let captures = KEY.captures(text)?;

    if captures.get(0)?.as_str().len() != text.len() {
        return None;
    }

    Some((captures[1].to_uppercase(), captures[2].parse().ok()?))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeyCase {
    /// Only `WEB-12`: prose, commit messages, pull request titles.
    #[default]
    Exact,
    /// `web-12` too: branch names.
    Any,
}

/// Issue keys mentioned in a text, upper-cased, in order of first appearance. Only keys of
/// `projects` count, and a key glued to a longer word (`WEB-12a`) does not.
pub fn extract_issue_keys(text: &str, projects: &HashSet<String>, case: KeyCase) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found: Vec<String> = Vec::new();

    for captures in KEY.captures_iter(text) {
        let whole = captures.get(0).expect("group 0 always exists");
        let glued_before = whole.start() > 0 && bytes[whole.start() - 1].is_ascii_alphanumeric();
        let glued_after = bytes.get(whole.end()).is_some_and(u8::is_ascii_alphanumeric);

        if glued_before || glued_after {
            continue;
        }

        let project = &captures[1];
        let upper = project.to_uppercase();

        if case == KeyCase::Exact && project != upper {
            continue;
        }

        if !projects.contains(&upper) {
            continue;
        }

        let Ok(number) = captures[2].parse::<i64>() else { continue };
        let key = format!("{upper}-{number}");

        if !found.contains(&key) {
            found.push(key);
        }
    }

    found
}

/// Keys for many texts at once (a push with hundreds of commits, a backfill of pull requests).
pub fn extract_issue_keys_many(texts: &[&str], projects: &HashSet<String>, case: KeyCase) -> Vec<Vec<String>> {
    texts.par_iter().map(|text| extract_issue_keys(text, projects, case)).collect()
}

/// `feature/WEB-12-login-form-validation`, the branch name suggested for an issue.
pub fn suggest_branch_name(key: &str, summary: &str, type_name: &str) -> String {
    let prefix = match type_name.to_lowercase().as_str() {
        "bug" => "fix",
        "epic" => "epic",
        _ => "feature",
    };
    let slug = slugify(summary, 40);

    if slug.is_empty() {
        format!("{prefix}/{key}")
    } else {
        format!("{prefix}/{key}-{slug}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projects() -> HashSet<String> {
        ["WEB", "API"].into_iter().map(String::from).collect()
    }

    #[test]
    fn parses_a_whole_key() {
        assert_eq!(parse_issue_key(" web-12 "), Some(("WEB".to_string(), 12)));
        assert_eq!(parse_issue_key("WEB-12 fix"), None);
        assert_eq!(parse_issue_key("12"), None);
    }

    #[test]
    fn finds_keys_of_known_projects_only() {
        let text = "WEB-12: fix login (see API-7, UTF-8, WEB-12 again, XWEB-3, WEB-4a)";

        assert_eq!(extract_issue_keys(text, &projects(), KeyCase::Exact), vec!["WEB-12", "API-7"]);
    }

    #[test]
    fn branch_names_match_in_any_case() {
        assert_eq!(extract_issue_keys("feature/web-12-login", &projects(), KeyCase::Any), vec!["WEB-12"]);
        assert!(extract_issue_keys("feature/web-12-login", &projects(), KeyCase::Exact).is_empty());
    }

    #[test]
    fn suggests_a_branch() {
        assert_eq!(suggest_branch_name("WEB-12", "Błąd logowania: 500", "Bug"), "fix/WEB-12-blad-logowania-500");
        assert_eq!(suggest_branch_name("WEB-1", "???", "Task"), "feature/WEB-1");
    }
}
