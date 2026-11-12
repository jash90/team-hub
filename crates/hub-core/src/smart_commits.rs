//! Commands in commit messages: `WEB-12 #comment fixed the redirect #time 1h 30m #in-review`,
//! and closing words in pull requests: `Fixes WEB-12`.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::keys::{extract_issue_keys, KeyCase};
use crate::text::parse_duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartCommand {
    Comment(String),
    /// Minutes of work to log.
    Time(i64),
    /// The name of a status (or transition) to move the issue to, as written, with dashes for spaces.
    Transition(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartCommit {
    pub key: String,
    pub commands: Vec<SmartCommand>,
}

static PIECE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?P<key>\b[A-Z][A-Z0-9]{1,9}-\d{1,7}\b)|(?:^|\s)#(?P<command>[A-Za-z][A-Za-z-]*)").expect("valid regex"));

static CLOSING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:fix(?:es|ed)?|close[sd]?|resolve[sd]?)\b[:\s]+((?:[A-Za-z][A-Za-z0-9]{1,9}-\d{1,7}(?:\s*,\s*|\s+and\s+)?)+)")
        .expect("valid regex")
});

/// The commands a commit message gives, per issue. Commands apply to the keys written before
/// them on the same line; a key after a command starts a new group.
pub fn parse_smart_commits(message: &str, projects: &HashSet<String>) -> Vec<SmartCommit> {
    let mut commits: Vec<SmartCommit> = Vec::new();

    for line in message.lines() {
        let pieces: Vec<_> = PIECE.captures_iter(line).collect();
        let mut keys: Vec<String> = Vec::new();
        let mut last_was_command = false;

        for (index, piece) in pieces.iter().enumerate() {
            if let Some(key) = piece.name("key") {
                if last_was_command {
                    keys.clear();
                }

                last_was_command = false;

                if let Some(known) = extract_issue_keys(key.as_str(), projects, KeyCase::Exact).pop() {
                    keys.push(known);
                }

                continue;
            }

            let Some(command) = piece.name("command") else { continue };
            last_was_command = true;

            let end = pieces.get(index + 1).map_or(line.len(), |next| next.get(0).map_or(line.len(), |whole| whole.start()));
            let argument = line[command.end()..end].trim();
            let name = command.as_str().to_lowercase();
            let mut commands = Vec::new();

            match name.as_str() {
                "comment" if !argument.is_empty() => commands.push(SmartCommand::Comment(argument.to_string())),
                "comment" => {}
                "time" => {
                    if let Some(minutes) = parse_duration(argument).filter(|minutes| *minutes > 0) {
                        commands.push(SmartCommand::Time(minutes));
                    }
                }
                _ => {
                    commands.push(SmartCommand::Transition(name.replace('-', " ")));

                    if !argument.is_empty() {
                        commands.push(SmartCommand::Comment(argument.to_string()));
                    }
                }
            }

            for key in &keys {
                match commits.iter_mut().find(|commit| &commit.key == key) {
                    Some(commit) => commit.commands.extend(commands.iter().cloned()),
                    None => commits.push(SmartCommit { key: key.clone(), commands: commands.clone() }),
                }
            }
        }
    }

    commits.retain(|commit| !commit.commands.is_empty());

    commits
}

/// Issues a text promises to close: `Fixes WEB-12`, `closes web-3, WEB-4`.
pub fn closing_keys(text: &str, projects: &HashSet<String>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();

    for captures in CLOSING.captures_iter(text) {
        for key in extract_issue_keys(&captures[1], projects, KeyCase::Any) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }

    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projects() -> HashSet<String> {
        ["WEB", "API"].into_iter().map(String::from).collect()
    }

    #[test]
    fn reads_comment_time_and_transition() {
        let commits = parse_smart_commits("WEB-12 #comment fixed the redirect #time 1h 30m #in-review", &projects());

        assert_eq!(
            commits,
            vec![SmartCommit {
                key: "WEB-12".into(),
                commands: vec![
                    SmartCommand::Comment("fixed the redirect".into()),
                    SmartCommand::Time(90),
                    SmartCommand::Transition("in review".into()),
                ],
            }]
        );
    }

    #[test]
    fn commands_apply_to_the_keys_before_them() {
        let commits = parse_smart_commits("WEB-1 API-2 #done\nWEB-3 #time 2h WEB-4 #comment later", &projects());

        assert_eq!(commits.len(), 4);
        assert_eq!(commits[0].commands, vec![SmartCommand::Transition("done".into())]);
        assert_eq!(commits[1].key, "API-2");
        assert_eq!(commits[2].commands, vec![SmartCommand::Time(120)]);
        assert_eq!(commits[3].commands, vec![SmartCommand::Comment("later".into())]);
    }

    #[test]
    fn ignores_plain_mentions_unknown_projects_and_hash_in_words() {
        assert!(parse_smart_commits("WEB-12 fix login, see issue#42", &projects()).is_empty());
        assert!(parse_smart_commits("OPS-1 #done", &projects()).is_empty());
        assert!(parse_smart_commits("#done without a key", &projects()).is_empty());
    }

    #[test]
    fn finds_closing_words() {
        assert_eq!(closing_keys("Fixes WEB-12 and API-3.\nRelated to WEB-9, closes: web-4", &projects()), vec!["WEB-12", "API-3", "WEB-4"]);
        assert!(closing_keys("prefixes WEB-12", &projects()).is_empty());
    }
}
