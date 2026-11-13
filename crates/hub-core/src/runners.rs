//! Runners (`docs/runners.md`): the machines that carry out the work that needs a repository
//! on disk. Modules know them (one for agent runs, one for test jobs); this is the shape they
//! are shown in, and the rule that makes one list of what several modules say.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::types::Id;

/// Something a runner is carrying out now, with the page that shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunnerActive {
    pub title: String,
    pub href: String,
}

/// What one module knows of a runner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunnerWork {
    /// The module's id.
    pub module: String,
    /// What it takes, in the module's words: run kinds, or "tests".
    pub what: Vec<String>,
    /// Project keys it works for; null: any it may edit.
    pub projects: Option<Vec<String>>,
    /// Takes only what is sent to it by name.
    pub pinned_only: bool,
    pub online: bool,
    pub last_seen_at: String,
    pub active: Vec<RunnerActive>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Runner {
    /// Stable per installation; the same for every kind of work it does.
    pub id: String,
    pub name: String,
    pub user_id: Id,
    /// Somebody's computer with the person at it (true), or a service (false).
    pub interactive: bool,
    /// A service that works for everyone, on each person's work with that person's own keys.
    pub shared: bool,
    pub online: bool,
    pub last_seen_at: String,
    pub version: Option<String>,
    pub work: Vec<RunnerWork>,
}

/// A key a person stores for themselves (`docs/runners.md` → Shared runners): a module says
/// that it exists, the core keeps it. Two modules may name the same key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonalKey {
    /// Lower-case letters and underscores, e.g. "github_token".
    pub name: String,
    pub label: String,
    pub hint: String,
}

/// A person's key as a page may see it: never the value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonalKeyState {
    pub name: String,
    pub label: String,
    pub hint: String,
    pub set: bool,
    /// The last four characters of the stored value, behind an ellipsis.
    pub tail: Option<String>,
    pub updated_at: Option<String>,
}

/// One entry per runner from what the modules reported, each of which describes a runner
/// with the one kind of work it knows. The name, the version and whether somebody is at it
/// come from whichever module heard from it last. Connected runners first, then by name.
pub fn merge(reported: Vec<Runner>) -> Vec<Runner> {
    let mut merged: Vec<Runner> = Vec::new();

    for mut runner in reported {
        match merged.iter_mut().find(|known| known.id == runner.id && known.user_id == runner.user_id) {
            Some(known) => {
                if runner.last_seen_at > known.last_seen_at {
                    known.name = std::mem::take(&mut runner.name);
                    known.interactive = runner.interactive;
                    known.shared = runner.shared;
                    known.last_seen_at = std::mem::take(&mut runner.last_seen_at);
                    known.version = runner.version.take().or(known.version.take());
                }

                known.online |= runner.online;
                known.work.append(&mut runner.work);
            }
            None => merged.push(runner),
        }
    }

    merged.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.id.cmp(&b.id)));

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(id: &str, name: &str, module: &str, seen: &str, online: bool) -> Runner {
        Runner {
            id: id.into(),
            name: name.into(),
            user_id: 1,
            interactive: true,
            shared: false,
            online,
            last_seen_at: seen.into(),
            version: None,
            work: vec![RunnerWork { module: module.into(), what: vec![], projects: None, pinned_only: false, online, last_seen_at: seen.into(), active: vec![] }],
        }
    }

    #[test]
    fn a_runner_two_modules_know_is_listed_once_with_both_kinds_of_work() {
        let merged = merge(vec![
            runner("laptop", "Old name", "agents", "2026-01-01T10:00:00Z", false),
            runner("cloud", "Cloud runner", "agents", "2026-01-01T09:00:00Z", false),
            runner("laptop", "MacBook", "e2e", "2026-01-01T10:05:00Z", true),
        ]);

        assert_eq!(merged.len(), 2);
        assert_eq!((merged[0].id.as_str(), merged[0].name.as_str(), merged[0].online), ("laptop", "MacBook", true), "connected first, the later name wins");
        assert_eq!(merged[0].work.iter().map(|work| work.module.as_str()).collect::<Vec<_>>(), ["agents", "e2e"]);
        assert_eq!((merged[0].work[0].online, merged[0].work[1].online), (false, true), "each module keeps what it knows");
        assert_eq!(merged[1].id, "cloud");
    }

    #[test]
    fn the_same_id_under_two_people_stays_two_runners() {
        let mut other = runner("laptop", "Theirs", "e2e", "2026-01-01T10:00:00Z", true);
        other.user_id = 2;

        assert_eq!(merge(vec![runner("laptop", "Mine", "agents", "2026-01-01T10:00:00Z", true), other]).len(), 2);
    }
}
