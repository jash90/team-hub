//! What a Jira project consists of (its "structure"), and the mapping an import proposes:
//! how Jira's types, statuses, priorities and people become this project's.

use std::collections::HashSet;

use hub_app::ApiResult;
use hub_core::{BoardMode, Id, StatusCategory, TypeLevel};

use crate::client::Client;
use crate::remote::{RemoteSprint, RemoteStatus, RemoteType, RemoteUser, RemoteVersion};
use crate::types::{JiraMapping, JiraPriorityMapping, JiraStatusMapping, JiraTypeMapping, JiraUserMapping};

const MAX_NAME: usize = 60;

/// Everything about a project that is not an issue.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Structure {
    pub key: String,
    pub name: String,
    pub description: String,
    pub lead: Option<RemoteUser>,
    pub types: Vec<RemoteType>,
    /// In workflow order: to do, in progress, done, and within those as the board has them.
    pub statuses: Vec<RemoteStatus>,
    /// Statuses that are in no column of the project's board (folded names); none when the
    /// board could not be read.
    pub off_board: Option<HashSet<String>>,
    /// Highest first.
    pub priorities: Vec<String>,
    pub components: Vec<(String, String)>,
    pub versions: Vec<RemoteVersion>,
    pub sprints: Vec<RemoteSprint>,
    pub people: Vec<RemoteUser>,
    pub scrum: bool,
    /// Whether boards and sprints could be read from the agile API.
    pub agile: bool,
}

pub fn fold(name: &str) -> String {
    hub_core::text::fold(name.trim())
}

/// A name that fits where names of statuses, types and components go.
pub fn clean_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME).collect()
}

fn category_rank(category: &str) -> usize {
    match category {
        "new" => 0,
        "done" => 2,
        _ => 1,
    }
}

/// Reads the project's configuration. Boards and sprints come from the agile API; a Jira
/// without it (or a token that may not use it) still imports, with sprints taken from the issues.
pub fn fetch(client: &Client, key: &str) -> ApiResult<Structure> {
    let project = client.project(key)?;
    let mut statuses = client.statuses(key)?;
    let boards = client.boards(key);
    let agile = boards.is_ok();
    let boards = boards.unwrap_or_default();
    let mut sprints: Vec<RemoteSprint> = Vec::new();

    for board in &boards {
        // A kanban board answers that it has no sprints; that is not a failure of the import.
        for sprint in client.sprints(&board.id).unwrap_or_default() {
            if !sprints.iter().any(|known| known.id == sprint.id) {
                sprints.push(sprint);
            }
        }
    }

    let columns = boards
        .iter()
        .find(|board| board.scrum)
        .or(boards.first())
        .and_then(|board| client.columns(&board.id).ok())
        .filter(|columns| !columns.is_empty());
    let column_of = |status: &RemoteStatus| columns.as_ref().and_then(|columns| columns.iter().position(|ids| ids.contains(&status.id)));
    // Within a category the board's columns give the order. What is in no column comes
    // before the first column while it is still to do (a backlog), and after the last otherwise.
    let place = |status: &RemoteStatus| match (column_of(status), status.category.as_str()) {
        (Some(column), _) => column as i64,
        (None, "new") => -1,
        (None, _) => i64::MAX,
    };
    let off_board =
        columns.is_some().then(|| statuses.iter().filter(|status| column_of(status).is_none()).map(|status| fold(&status.name)).collect());
    let mut ordered: Vec<(usize, i64, usize, RemoteStatus)> =
        statuses.drain(..).enumerate().map(|(index, status)| (category_rank(&status.category), place(&status), index, status)).collect();

    ordered.sort_by_key(|(category, place, index, _)| (*category, *place, *index));

    Ok(Structure {
        key: project.key,
        name: project.name,
        description: project.description,
        lead: project.lead,
        types: project.types,
        statuses: ordered.into_iter().map(|(_, _, _, status)| status).collect(),
        off_board,
        priorities: client.priorities().unwrap_or_default(),
        components: project.components,
        versions: project.versions,
        scrum: boards.iter().any(|board| board.scrum) || !sprints.is_empty(),
        sprints,
        // Not every Jira lets a token list people; they are then matched as issues arrive.
        people: client.people(key).unwrap_or_default(),
        agile,
    })
}

/// What a status means when Jira does not say: guessed from its name.
pub fn guess_category(name: &str) -> StatusCategory {
    let name = fold(name);
    let has = |words: &[&str]| words.iter().any(|word| name.contains(word));

    if has(&["done", "closed", "resolved", "complete", "released", "cancel", "rejected", "fixed", "won't", "wont", "zamkn", "zrobione", "gotowe"]) {
        StatusCategory::Done
    } else if has(&["to do", "todo", "open", "backlog", "new", "selected", "ready for dev", "do zrobienia", "otwarte"]) {
        StatusCategory::Todo
    } else {
        StatusCategory::InProgress
    }
}

pub fn category_of(jira_category: &str, name: &str) -> StatusCategory {
    match jira_category {
        "new" => StatusCategory::Todo,
        "indeterminate" => StatusCategory::InProgress,
        "done" => StatusCategory::Done,
        _ => guess_category(name),
    }
}

pub fn default_type(issue_type: &RemoteType) -> JiraTypeMapping {
    let folded = fold(&issue_type.name);
    let level = if issue_type.subtask || folded.contains("sub-task") || folded.contains("subtask") {
        TypeLevel::Subtask
    } else if issue_type.epic {
        TypeLevel::Epic
    } else {
        TypeLevel::Standard
    };
    // Jira's "Sub-task" is this tracker's "Subtask".
    let name = if matches!(folded.as_str(), "sub-task" | "subtask") { "Subtask".to_string() } else { clean_name(&issue_type.name) };

    JiraTypeMapping { jira_name: issue_type.name.clone(), name, level }
}

pub fn default_status(status: &RemoteStatus) -> JiraStatusMapping {
    JiraStatusMapping {
        jira_name: status.name.clone(),
        jira_category: status.category.clone(),
        name: clean_name(&status.name),
        category: category_of(&status.category, &status.name),
    }
}

/// 1 (highest) to 5 for a priority: by its usual name, otherwise by its place in Jira's list.
pub fn default_priority(name: &str, position: usize, count: usize) -> i64 {
    match fold(name).as_str() {
        "highest" | "blocker" | "urgent" | "p0" | "p1" => 1,
        "high" | "critical" | "p2" => 2,
        "medium" | "major" | "normal" | "p3" => 3,
        "low" | "minor" | "p4" => 4,
        "lowest" | "trivial" | "p5" => 5,
        _ if count > 1 => 1 + (position * 4 / (count - 1)) as i64,
        _ => hub_core::DEFAULT_PRIORITY,
    }
}

/// The icon of a type this tracker draws: its own for the kinds it knows, a task otherwise.
pub fn type_icon(name: &str, level: TypeLevel) -> &'static str {
    let folded = fold(name);

    match level {
        TypeLevel::Epic => "epic",
        TypeLevel::Subtask => "subtask",
        TypeLevel::Standard if folded.contains("bug") || folded.contains("defect") || folded.contains("blad") => "bug",
        TypeLevel::Standard if folded.contains("story") => "story",
        TypeLevel::Standard => "task",
    }
}

/// The mapping an import uses: what the administrator chose, else what the last import of the
/// project used, else the default for each thing Jira has.
pub fn resolve(structure: &Structure, local_users: &[(Id, String)], stored: Option<&JiraMapping>, chosen: Option<&JiraMapping>) -> JiraMapping {
    let pick = |find: &dyn Fn(&JiraMapping) -> bool| chosen.filter(|mapping| find(mapping)).or(stored.filter(|mapping| find(mapping)));
    let types = structure
        .types
        .iter()
        .map(|issue_type| {
            let same = |entry: &&JiraTypeMapping| fold(&entry.jira_name) == fold(&issue_type.name) && !entry.name.trim().is_empty();

            pick(&|mapping| mapping.types.iter().any(|entry| same(&entry)))
                .and_then(|mapping| mapping.types.iter().find(same).cloned())
                .map(|entry| JiraTypeMapping { name: clean_name(&entry.name), jira_name: issue_type.name.clone(), ..entry })
                .unwrap_or_else(|| default_type(issue_type))
        })
        .collect();
    let statuses = structure
        .statuses
        .iter()
        .map(|status| {
            let same = |entry: &&JiraStatusMapping| fold(&entry.jira_name) == fold(&status.name) && !entry.name.trim().is_empty();

            pick(&|mapping| mapping.statuses.iter().any(|entry| same(&entry)))
                .and_then(|mapping| mapping.statuses.iter().find(same).cloned())
                .map(|entry| JiraStatusMapping {
                    name: clean_name(&entry.name),
                    jira_name: status.name.clone(),
                    jira_category: status.category.clone(),
                    ..entry
                })
                .unwrap_or_else(|| default_status(status))
        })
        .collect();
    let priorities = structure
        .priorities
        .iter()
        .enumerate()
        .map(|(position, name)| {
            let same = |entry: &&JiraPriorityMapping| fold(&entry.jira_name) == fold(name);
            let priority = pick(&|mapping| mapping.priorities.iter().any(|entry| same(&entry)))
                .and_then(|mapping| mapping.priorities.iter().find(same).map(|entry| entry.priority))
                .unwrap_or_else(|| default_priority(name, position, structure.priorities.len()));

            JiraPriorityMapping { jira_name: name.clone(), priority: priority.clamp(1, 5) }
        })
        .collect();
    let users = structure
        .people
        .iter()
        .map(|person| {
            let same = |entry: &&JiraUserMapping| entry.account == person.account;
            let by_choice = pick(&|mapping| mapping.users.iter().any(|entry| same(&entry) && entry.user_id.is_some()))
                .and_then(|mapping| mapping.users.iter().find(same).and_then(|entry| entry.user_id))
                .filter(|id| local_users.iter().any(|(known, _)| known == id));
            let by_email = person
                .email
                .as_deref()
                .and_then(|email| local_users.iter().find(|(_, known)| known.eq_ignore_ascii_case(email.trim())).map(|(id, _)| *id));

            JiraUserMapping {
                account: person.account.clone(),
                name: person.name.clone(),
                email: person.email.clone(),
                user_id: by_choice.or(by_email),
            }
        })
        .collect();

    JiraMapping {
        board_mode: chosen.and_then(|mapping| mapping.board_mode).or(stored.and_then(|mapping| mapping.board_mode)).or(Some(if structure.scrum {
            BoardMode::Scrum
        } else {
            BoardMode::Kanban
        })),
        types,
        statuses,
        priorities,
        users,
        create_users: chosen.or(stored).is_none_or(|mapping| mapping.create_users),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(name: &str, category: &str) -> RemoteStatus {
        RemoteStatus { id: name.to_string(), name: name.to_string(), category: category.to_string() }
    }

    #[test]
    fn defaults_follow_jira() {
        assert_eq!(default_type(&RemoteType { name: "Sub-task".into(), subtask: true, epic: false }).name, "Subtask");
        assert_eq!(default_type(&RemoteType { name: "Sub-task".into(), subtask: true, epic: false }).level, TypeLevel::Subtask);
        assert_eq!(default_type(&RemoteType { name: "Epic".into(), subtask: false, epic: true }).level, TypeLevel::Epic);
        assert_eq!(default_type(&RemoteType { name: "Improvement".into(), subtask: false, epic: false }).level, TypeLevel::Standard);
        assert_eq!(default_status(&status("Selected for Development", "new")).category, StatusCategory::Todo);
        assert_eq!(default_status(&status("Code review", "indeterminate")).category, StatusCategory::InProgress);
        assert_eq!(default_status(&status("Won't do", "done")).category, StatusCategory::Done);
        assert_eq!(category_of("", "Closed"), StatusCategory::Done);
        assert_eq!(category_of("", "Backlog"), StatusCategory::Todo);
        assert_eq!(category_of("", "Testing"), StatusCategory::InProgress);
        assert_eq!(["Highest", "High", "Medium", "Low", "Lowest"].map(|name| default_priority(name, 0, 5)), [1, 2, 3, 4, 5]);
        assert_eq!(["Blocker", "Critical", "Major", "Minor", "Trivial"].map(|name| default_priority(name, 0, 5)), [1, 2, 3, 4, 5]);
        assert_eq!((0..3).map(|position| default_priority("Custom", position, 3)).collect::<Vec<_>>(), vec![1, 3, 5]);
        assert_eq!(type_icon("Defect", TypeLevel::Standard), "bug");
        assert_eq!(type_icon("Improvement", TypeLevel::Standard), "task");
    }

    #[test]
    fn a_choice_beats_the_last_import_which_beats_the_default() {
        let structure = Structure {
            types: vec![
                RemoteType { name: "Bug".into(), subtask: false, epic: false },
                RemoteType { name: "Story".into(), subtask: false, epic: false },
            ],
            statuses: vec![status("Open", "new"), status("Closed", "done")],
            priorities: vec!["High".into(), "Low".into()],
            people: vec![
                RemoteUser { account: "a1".into(), name: "Anna".into(), email: Some("ANNA@example.com".into()) },
                RemoteUser { account: "b2".into(), name: "Bob".into(), email: None },
            ],
            ..Default::default()
        };
        let stored = JiraMapping {
            board_mode: Some(BoardMode::Kanban),
            types: vec![JiraTypeMapping { jira_name: "bug".into(), name: "Defect".into(), level: TypeLevel::Standard }],
            statuses: vec![JiraStatusMapping {
                jira_name: "Open".into(),
                jira_category: String::new(),
                name: "To Do".into(),
                category: StatusCategory::Todo,
            }],
            users: vec![JiraUserMapping { account: "b2".into(), name: String::new(), email: None, user_id: Some(9) }],
            create_users: false,
            ..Default::default()
        };
        let chosen = JiraMapping {
            statuses: vec![JiraStatusMapping {
                jira_name: "Closed".into(),
                jira_category: String::new(),
                name: " Done ".into(),
                category: StatusCategory::Done,
            }],
            priorities: vec![JiraPriorityMapping { jira_name: "Low".into(), priority: 9 }],
            create_users: true,
            ..Default::default()
        };
        let users = [(7, "anna@example.com".to_string()), (9, "bob@example.com".to_string())];
        let mapping = resolve(&structure, &users, Some(&stored), Some(&chosen));

        assert_eq!(mapping.board_mode, Some(BoardMode::Kanban));
        assert_eq!(mapping.types.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(), vec!["Defect", "Story"]);
        assert_eq!(mapping.statuses.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(), vec!["To Do", "Done"]);
        assert_eq!(mapping.statuses[1].jira_category, "done");
        assert_eq!(mapping.priorities.iter().map(|entry| entry.priority).collect::<Vec<_>>(), vec![2, 5]);
        assert_eq!(mapping.users.iter().map(|entry| entry.user_id).collect::<Vec<_>>(), vec![Some(7), Some(9)]);
        assert!(mapping.create_users);
        assert!(resolve(&structure, &users, None, None).create_users);
        assert!(!resolve(&structure, &users, Some(&stored), None).create_users);
    }
}
