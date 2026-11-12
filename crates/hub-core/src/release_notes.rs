//! Release notes for a version, as Markdown.

pub struct NotesIssue<'a> {
    pub key: &'a str,
    pub summary: &'a str,
    pub type_name: &'a str,
    pub done: bool,
}

/// Finished issues grouped by type, in the order the types are given; unfinished ones listed
/// last so a release is never described as more complete than it is.
pub fn release_notes(version: &str, release_date: Option<&str>, type_order: &[&str], issues: &[NotesIssue]) -> String {
    let mut notes = format!("# {version}\n");

    if let Some(date) = release_date {
        notes.push_str(&format!("\n_{date}_\n"));
    }

    for type_name in type_order {
        let lines: Vec<String> = issues
            .iter()
            .filter(|issue| issue.done && issue.type_name == *type_name)
            .map(|issue| format!("- {} {}", issue.key, issue.summary))
            .collect();

        if !lines.is_empty() {
            notes.push_str(&format!("\n## {type_name}\n\n{}\n", lines.join("\n")));
        }
    }

    let open: Vec<String> =
        issues.iter().filter(|issue| !issue.done).map(|issue| format!("- {} {}", issue.key, issue.summary)).collect();

    if !open.is_empty() {
        notes.push_str(&format!("\n## Not finished\n\n{}\n", open.join("\n")));
    }

    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_done_issues_and_lists_open_ones_last() {
        let issues = [
            NotesIssue { key: "WEB-1", summary: "Login", type_name: "Story", done: true },
            NotesIssue { key: "WEB-2", summary: "Crash", type_name: "Bug", done: true },
            NotesIssue { key: "WEB-3", summary: "Later", type_name: "Story", done: false },
        ];
        let notes = release_notes("1.0", Some("2026-11-20"), &["Bug", "Story", "Task"], &issues);

        assert_eq!(
            notes,
            "# 1.0\n\n_2026-11-20_\n\n## Bug\n\n- WEB-2 Crash\n\n## Story\n\n- WEB-1 Login\n\n## Not finished\n\n- WEB-3 Later\n"
        );
    }
}
