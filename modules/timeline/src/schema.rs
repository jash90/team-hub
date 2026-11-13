pub const MIGRATIONS: &[&str] = &[r#"
-- The day work on an issue starts. Where it ends is the tracker's own due date
-- (issue.due_date), so an issue without a row here has no start and nothing else is lost.
CREATE TABLE timeline_span (
    issue_id INTEGER PRIMARY KEY REFERENCES issue (id) ON DELETE CASCADE,
    start_date TEXT NOT NULL
);
"#];
