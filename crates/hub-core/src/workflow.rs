//! Workflows: which status may follow which, and the templates a new project starts from.

use crate::types::{Id, StatusCategory, Transition, TypeLevel};

pub struct StatusTemplate {
    pub name: &'static str,
    pub category: StatusCategory,
    pub on_board: bool,
}

pub struct TypeTemplate {
    pub name: &'static str,
    pub level: TypeLevel,
    pub icon: &'static str,
}

pub const DEFAULT_TYPES: [TypeTemplate; 5] = [
    TypeTemplate { name: "Epic", level: TypeLevel::Epic, icon: "epic" },
    TypeTemplate { name: "Story", level: TypeLevel::Standard, icon: "story" },
    TypeTemplate { name: "Task", level: TypeLevel::Standard, icon: "task" },
    TypeTemplate { name: "Bug", level: TypeLevel::Standard, icon: "bug" },
    TypeTemplate { name: "Subtask", level: TypeLevel::Subtask, icon: "subtask" },
];

pub const SCRUM_STATUSES: [StatusTemplate; 4] = [
    StatusTemplate { name: "To Do", category: StatusCategory::Todo, on_board: true },
    StatusTemplate { name: "In Progress", category: StatusCategory::InProgress, on_board: true },
    StatusTemplate { name: "In Review", category: StatusCategory::InProgress, on_board: true },
    StatusTemplate { name: "Done", category: StatusCategory::Done, on_board: true },
];

pub const KANBAN_STATUSES: [StatusTemplate; 5] = [
    StatusTemplate { name: "Backlog", category: StatusCategory::Todo, on_board: false },
    StatusTemplate { name: "To Do", category: StatusCategory::Todo, on_board: true },
    StatusTemplate { name: "In Progress", category: StatusCategory::InProgress, on_board: true },
    StatusTemplate { name: "In Review", category: StatusCategory::InProgress, on_board: true },
    StatusTemplate { name: "Done", category: StatusCategory::Done, on_board: true },
];

/// Whether an issue may move from `from` to `to`.
///
/// A project without a strict workflow lets anything go anywhere. A strict one allows only its
/// listed transitions; a transition with no source status applies from every status.
pub fn can_transition(strict: bool, transitions: &[Transition], from: Id, to: Id) -> bool {
    if from == to || !strict {
        return true;
    }

    transitions.iter().any(|transition| transition.to_status_id == to && transition.from_status_id.is_none_or(|source| source == from))
}

/// The statuses reachable from `from`, in the order of `all_statuses`.
pub fn allowed_targets(strict: bool, transitions: &[Transition], all_statuses: &[Id], from: Id) -> Vec<Id> {
    all_statuses.iter().copied().filter(|status| *status != from && can_transition(strict, transitions, from, *status)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition(from: Option<Id>, to: Id) -> Transition {
        Transition { id: 0, from_status_id: from, to_status_id: to, name: String::new() }
    }

    #[test]
    fn open_workflow_allows_everything() {
        assert!(can_transition(false, &[], 1, 3));
    }

    #[test]
    fn strict_workflow_follows_the_list() {
        let transitions = [transition(Some(1), 2), transition(Some(2), 3), transition(None, 1)];

        assert!(can_transition(true, &transitions, 1, 2));
        assert!(!can_transition(true, &transitions, 1, 3));
        assert!(can_transition(true, &transitions, 3, 1));
        assert!(can_transition(true, &transitions, 2, 2));
        assert_eq!(allowed_targets(true, &transitions, &[1, 2, 3], 2), vec![1, 3]);
    }
}
