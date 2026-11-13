//! Issues: reading them, and every way they change. Each change is recorded in the issue's
//! history and announced as an event.
//!
//! One file per thing that happens to an issue. Everything public is re-exported here, so the
//! rest of the workspace keeps writing `services::issues::create_issue`.

mod attachments;
mod comments;
mod create;
mod current;
mod delete;
mod detail;
mod fields;
mod import;
mod index;
mod links;
mod moving;
mod rows;
mod update;
mod watchers;
mod worklogs;

pub use attachments::{add_attachment, attachment_file, delete_attachment, load_attachments};
pub use comments::{add_comment, delete_comment, load_comments, update_comment};
pub use create::create_issue;
pub use current::status_by_name;
pub use delete::delete_issue;
pub use detail::issue_detail;
pub use import::{import_change, import_comment, import_issue, import_worklog, ImportedIssue};
pub use index::reindex_issue;
pub use links::{add_link, delete_link, load_links};
pub use moving::{move_issue, MoveRequest};
pub use rows::{decorate, issue_access, issue_id_by_key, load_rows, row_by_id, rows_by_ids, ISSUE_SELECT};
pub use update::update_issue;
pub use watchers::set_watching;
pub use worklogs::{add_worklog, delete_worklog, load_worklogs};
