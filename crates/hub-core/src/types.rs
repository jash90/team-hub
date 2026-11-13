//! The shapes that cross the API. TypeScript bindings are generated from these by `cargo test`.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

pub type Id = i64;

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        #[ts(export)]
        pub enum $name {
            $(#[serde(rename = $text)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            pub fn parse(text: &str) -> Option<Self> {
                match text { $($text => Some(Self::$variant),)+ _ => None }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

pub(crate) use string_enum;

string_enum!(Role { Admin => "admin", Member => "member" });
string_enum!(ProjectRole { Admin => "admin", Member => "member", Viewer => "viewer" });
string_enum!(StatusCategory { Todo => "todo", InProgress => "in_progress", Done => "done" });
string_enum!(TypeLevel { Epic => "epic", Standard => "standard", Subtask => "subtask" });
string_enum!(SprintState { Future => "future", Active => "active", Closed => "closed" });
string_enum!(LinkType { Blocks => "blocks", Relates => "relates", Duplicates => "duplicates" });
string_enum!(BoardMode { Scrum => "scrum", Kanban => "kanban" });
string_enum!(LinkDirection { Outward => "outward", Inward => "inward" });

/// Distinguishes "field absent" (outer `None`) from "field set to null" (`Some(None)`) in a patch.
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct User {
    pub id: Id,
    pub name: String,
    pub email: String,
    pub role: Role,
    pub active: bool,
    pub color: String,
    pub github_login: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Status {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    pub category: StatusCategory,
    pub position: i64,
    pub on_board: bool,
    pub wip_limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IssueType {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    pub level: TypeLevel,
    pub icon: String,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Transition {
    pub id: Id,
    pub from_status_id: Option<Id>,
    pub to_status_id: Id,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Component {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    pub description: String,
    pub lead_id: Option<Id>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkCounts {
    pub total: i64,
    pub todo: i64,
    pub in_progress: i64,
    pub done: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Version {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    pub description: String,
    pub start_date: Option<String>,
    pub release_date: Option<String>,
    pub released: bool,
    pub released_at: Option<String>,
    pub archived: bool,
    pub counts: WorkCounts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sprint {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    pub goal: String,
    pub state: SprintState,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub completed_at: Option<String>,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectSummary {
    pub id: Id,
    pub key: String,
    pub name: String,
    pub description: String,
    pub lead_id: Option<Id>,
    pub board_mode: BoardMode,
    pub is_private: bool,
    pub archived: bool,
    /// The caller's role in this project; none when they can only see it.
    pub role: Option<ProjectRole>,
    pub issue_count: i64,
    pub open_count: i64,
    /// Ids of the modules switched off in this project.
    #[serde(default)]
    pub disabled_modules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectMember {
    pub user_id: Id,
    pub role: ProjectRole,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Project {
    #[serde(flatten)]
    pub summary: ProjectSummary,
    pub strict_workflow: bool,
    pub statuses: Vec<Status>,
    pub types: Vec<IssueType>,
    pub transitions: Vec<Transition>,
    pub components: Vec<Component>,
    pub versions: Vec<Version>,
    pub sprints: Vec<Sprint>,
    pub members: Vec<ProjectMember>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubtaskTally {
    pub total: i64,
    pub done: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IssueRow {
    pub id: Id,
    pub key: String,
    pub project_id: Id,
    pub project_key: String,
    pub number: i64,
    pub summary: String,
    pub type_id: Id,
    pub status_id: Id,
    /// 1 is the highest priority, 5 the lowest.
    pub priority: i64,
    pub assignee_id: Option<Id>,
    pub reporter_id: Option<Id>,
    pub parent_id: Option<Id>,
    pub parent_key: Option<String>,
    pub parent_summary: Option<String>,
    pub sprint_id: Option<Id>,
    pub version_id: Option<Id>,
    pub points: Option<f64>,
    pub due_date: Option<String>,
    pub labels: Vec<String>,
    pub component_ids: Vec<Id>,
    pub rank: String,
    pub created_at: String,
    pub updated_at: String,
    pub resolved_at: Option<String>,
    pub subtasks: SubtaskTally,
    pub comment_count: i64,
    /// Per-module decorations, keyed by module id.
    pub ext: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Comment {
    pub id: Id,
    pub issue_id: Id,
    pub author_id: Option<Id>,
    /// Who wrote it when there is no user behind it, e.g. "GitHub · octocat".
    pub author_label: Option<String>,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Change {
    pub id: Id,
    pub issue_id: Id,
    pub actor_id: Option<Id>,
    pub actor_label: Option<String>,
    pub field: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IssueLink {
    pub id: Id,
    #[serde(rename = "type")]
    pub link_type: LinkType,
    /// Outward: this issue is the source ("blocks"). Inward: it is the target ("is blocked by").
    pub direction: LinkDirection,
    pub issue: IssueRow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Attachment {
    pub id: Id,
    pub issue_id: Id,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub uploader_id: Option<Id>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Worklog {
    pub id: Id,
    pub issue_id: Id,
    pub user_id: Option<Id>,
    pub minutes: i64,
    pub started_at: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Issue {
    #[serde(flatten)]
    pub row: IssueRow,
    pub description: String,
    pub estimate_minutes: Option<i64>,
    pub spent_minutes: i64,
    pub comments: Vec<Comment>,
    pub changes: Vec<Change>,
    pub links: Vec<IssueLink>,
    pub children: Vec<IssueRow>,
    pub attachments: Vec<Attachment>,
    pub worklogs: Vec<Worklog>,
    pub watchers: Vec<Id>,
    /// Data for per-module panels on the issue page, keyed by module id.
    pub panels: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, optional_fields)]
pub struct IssuePatch {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub type_id: Option<Id>,
    pub status_id: Option<Id>,
    pub priority: Option<i64>,
    #[serde(deserialize_with = "double_option")]
    #[ts(type = "number | null")]
    pub assignee_id: Option<Option<Id>>,
    #[serde(deserialize_with = "double_option")]
    #[ts(type = "number | null")]
    pub reporter_id: Option<Option<Id>>,
    #[serde(deserialize_with = "double_option")]
    #[ts(type = "string | null")]
    pub parent_key: Option<Option<String>>,
    #[serde(deserialize_with = "double_option")]
    #[ts(type = "number | null")]
    pub sprint_id: Option<Option<Id>>,
    #[serde(deserialize_with = "double_option")]
    #[ts(type = "number | null")]
    pub version_id: Option<Option<Id>>,
    #[serde(deserialize_with = "double_option")]
    #[ts(type = "number | null")]
    pub points: Option<Option<f64>>,
    #[serde(deserialize_with = "double_option")]
    #[ts(type = "string | null")]
    pub due_date: Option<Option<String>>,
    #[serde(deserialize_with = "double_option")]
    #[ts(type = "number | null")]
    pub estimate_minutes: Option<Option<i64>>,
    pub labels: Option<Vec<String>>,
    pub component_ids: Option<Vec<Id>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewIssue {
    pub project_key: String,
    #[serde(flatten)]
    pub fields: IssuePatch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BoardColumn {
    pub status: Status,
    pub issues: Vec<IssueRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Board {
    pub mode: BoardMode,
    pub sprint: Option<Sprint>,
    pub columns: Vec<BoardColumn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SprintIssues {
    pub sprint: Sprint,
    pub issues: Vec<IssueRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Backlog {
    pub sprints: Vec<SprintIssues>,
    pub backlog: Vec<IssueRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchError {
    pub message: String,
    pub position: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResult {
    pub issues: Vec<IssueRow>,
    pub total: i64,
    /// Set when the query could not be understood; `issues` is then empty.
    pub error: Option<SearchError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedFilter {
    pub id: Id,
    pub owner_id: Id,
    pub name: String,
    pub query: String,
    pub shared: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Notification {
    pub id: Id,
    pub kind: String,
    pub issue_key: Option<String>,
    pub actor_id: Option<Id>,
    pub text: String,
    pub created_at: String,
    pub read_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActivityItem {
    pub id: Id,
    pub at: String,
    pub actor_id: Option<Id>,
    pub actor_label: Option<String>,
    pub kind: String,
    pub project_key: Option<String>,
    pub issue_key: Option<String>,
    pub issue_summary: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModuleInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub enabled: bool,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApiToken {
    pub id: Id,
    pub name: String,
    pub hint: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Bootstrap {
    pub setup_required: bool,
    pub user: Option<User>,
    pub csrf: Option<String>,
    pub users: Vec<User>,
    pub projects: Vec<ProjectSummary>,
    pub modules: Vec<ModuleInfo>,
    pub version: String,
    pub unread_notifications: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QuickHit {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SprintProgress {
    pub project_key: String,
    pub sprint: Sprint,
    pub total: i64,
    pub done: i64,
    pub points: f64,
    pub points_done: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Dashboard {
    pub assigned: Vec<IssueRow>,
    pub reported: Vec<IssueRow>,
    pub watching: Vec<IssueRow>,
    pub recently_done: Vec<IssueRow>,
    pub active_sprints: Vec<SprintProgress>,
    /// Module sections, keyed by module id.
    pub ext: BTreeMap<String, serde_json::Value>,
}

pub const PRIORITY_NAMES: [&str; 5] = ["Highest", "High", "Medium", "Low", "Lowest"];
pub const DEFAULT_PRIORITY: i64 = 3;

pub fn priority_name(value: i64) -> &'static str {
    usize::try_from(value - 1).ok().and_then(|index| PRIORITY_NAMES.get(index)).copied().unwrap_or("Medium")
}

pub fn priority_from_name(name: &str) -> Option<i64> {
    PRIORITY_NAMES.iter().position(|candidate| candidate.eq_ignore_ascii_case(name)).map(|index| index as i64 + 1)
}
