//! Reads `git diff` output into files, hunks and numbered lines.

use rayon::prelude::*;

use crate::model::{PatchFile, PatchHunk, PatchLine};

const FILE_HEADER: &str = "diff --git ";

/// `"src/a b.rs"` the way git quotes a path with unusual characters, back to the path.
fn unquote(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) else {
        return path.to_string();
    };
    let mut bytes = Vec::new();
    let mut characters = inner.bytes().peekable();

    while let Some(byte) = characters.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }

        match characters.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(digit @ b'0'..=b'7') => {
                let mut value = u32::from(digit - b'0');

                for _ in 0..2 {
                    if let Some(next @ b'0'..=b'7') = characters.peek().copied() {
                        value = value * 8 + u32::from(next - b'0');
                        characters.next();
                    }
                }

                bytes.push(value as u8);
            }
            Some(other) => bytes.push(other),
            None => {}
        }
    }

    String::from_utf8_lossy(&bytes).into_owned()
}

fn strip_side(path: &str) -> Option<String> {
    let path = unquote(path.trim_end_matches('\t'));

    if path == "/dev/null" {
        return None;
    }

    Some(path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")).unwrap_or(&path).to_string())
}

/// `@@ -12,4 +15,6 @@ fn name()` → (12, 15).
fn hunk_start(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.strip_prefix("@@ ")?.split(' ');
    let old = parts.next()?.strip_prefix('-')?.split(',').next()?.parse().ok()?;
    let new = parts.next()?.strip_prefix('+')?.split(',').next()?.parse().ok()?;

    Some((old, new))
}

fn parse_file(chunk: &str) -> PatchFile {
    let mut lines = chunk.lines();
    let header = lines.next().unwrap_or_default();
    let mut file = PatchFile {
        path: String::new(),
        old_path: None,
        status: "modified".to_string(),
        additions: 0,
        deletions: 0,
        binary: false,
        hunks: Vec::new(),
    };
    let mut old_side: Option<String> = None;
    let mut new_side: Option<String> = None;
    let (mut old_line, mut new_line) = (0u32, 0u32);

    for line in lines {
        if let Some(hunk) = file.hunks.last_mut() {
            let (kind, text) = match line.as_bytes().first() {
                Some(b'+') => ("add", &line[1..]),
                Some(b'-') => ("del", &line[1..]),
                Some(b' ') => ("context", &line[1..]),
                Some(b'@') => ("hunk", line),
                // "\ No newline at end of file" and anything else that is not a line of the file.
                _ => continue,
            };

            match kind {
                "add" => {
                    hunk.lines.push(PatchLine { kind: kind.into(), text: text.into(), old_line: None, new_line: Some(new_line) });
                    new_line += 1;
                    file.additions += 1;
                }
                "del" => {
                    hunk.lines.push(PatchLine { kind: kind.into(), text: text.into(), old_line: Some(old_line), new_line: None });
                    old_line += 1;
                    file.deletions += 1;
                }
                "context" => {
                    hunk.lines.push(PatchLine { kind: kind.into(), text: text.into(), old_line: Some(old_line), new_line: Some(new_line) });
                    old_line += 1;
                    new_line += 1;
                }
                _ => {
                    (old_line, new_line) = hunk_start(line).unwrap_or((old_line, new_line));
                    file.hunks.push(PatchHunk { header: line.to_string(), lines: Vec::new() });
                }
            }

            continue;
        }

        if line.starts_with("@@ ") {
            (old_line, new_line) = hunk_start(line).unwrap_or((1, 1));
            file.hunks.push(PatchHunk { header: line.to_string(), lines: Vec::new() });
        } else if let Some(path) = line.strip_prefix("--- ") {
            old_side = strip_side(path);
        } else if let Some(path) = line.strip_prefix("+++ ") {
            new_side = strip_side(path);
        } else if line.starts_with("new file mode") {
            file.status = "added".to_string();
        } else if line.starts_with("deleted file mode") {
            file.status = "deleted".to_string();
        } else if let Some(path) = line.strip_prefix("rename from ") {
            file.status = "renamed".to_string();
            file.old_path = Some(unquote(path));
        } else if let Some(path) = line.strip_prefix("rename to ") {
            new_side = Some(unquote(path));
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.binary = true;
        }
    }

    // Without ---/+++ lines (a binary file, a pure rename, an empty file) the header names the path.
    let from_header = || {
        let rest = header.strip_prefix(FILE_HEADER).unwrap_or_default();
        let middle = rest.find(" b/").or_else(|| rest.find(" \"b/"));

        middle.and_then(|index| strip_side(rest[index + 1..].trim())).unwrap_or_else(|| rest.to_string())
    };

    file.path = new_side.or(old_side).unwrap_or_else(from_header);
    file
}

/// Every file of a patch. Files are independent, so they are read in parallel.
pub fn parse_patch(patch: &str) -> Vec<PatchFile> {
    let mut starts: Vec<usize> = patch.match_indices(FILE_HEADER).map(|(index, _)| index).filter(|&index| index == 0 || patch.as_bytes()[index - 1] == b'\n').collect();

    starts.push(patch.len());

    let chunks: Vec<&str> = starts.windows(2).map(|window| &patch[window[0]..window[1]]).collect();

    chunks.par_iter().map(|chunk| parse_file(chunk)).collect()
}

/// A file git does not track yet, shown as wholly added.
pub fn untracked_file(path: &str, content: Option<&str>) -> PatchFile {
    let lines: Vec<PatchLine> = content
        .unwrap_or_default()
        .lines()
        .enumerate()
        .map(|(index, text)| PatchLine { kind: "add".into(), text: text.into(), old_line: None, new_line: Some(index as u32 + 1) })
        .collect();

    PatchFile {
        path: path.to_string(),
        old_path: None,
        status: "untracked".to_string(),
        additions: lines.len() as u32,
        deletions: 0,
        binary: content.is_none(),
        hunks: if lines.is_empty() { Vec::new() } else { vec![PatchHunk { header: format!("@@ -0,0 +1,{} @@", lines.len()), lines }] },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/src/login.ts b/src/login.ts
index 1111111..2222222 100644
--- a/src/login.ts
+++ b/src/login.ts
@@ -10,4 +10,5 @@ export function login() {
   const a = 1;
-  const b = 2;
+  const b = 3;
+  const c = 4;
   return a;
@@ -40,2 +41,2 @@
-old
+new
\\ No newline at end of file
diff --git a/README.md b/README.md
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/README.md
@@ -0,0 +1,2 @@
+# Title
+diff --git a/x b/x is only text here
diff --git a/logo.png b/logo.png
index 4444444..5555555 100644
Binary files a/logo.png and b/logo.png differ
diff --git a/old name.txt b/new name.txt
similarity index 100%
rename from old name.txt
rename to new name.txt
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
index 6666666..0000000
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-fn main() {}
";

    #[test]
    fn files_hunks_and_line_numbers_are_read() {
        let files = parse_patch(PATCH);
        let names: Vec<(&str, &str)> = files.iter().map(|file| (file.path.as_str(), file.status.as_str())).collect();

        assert_eq!(
            names,
            vec![("src/login.ts", "modified"), ("README.md", "added"), ("logo.png", "modified"), ("new name.txt", "renamed"), ("gone.rs", "deleted")]
        );

        let login = &files[0];

        assert_eq!((login.additions, login.deletions, login.hunks.len()), (3, 2, 2));
        assert_eq!(login.hunks[0].lines[1], PatchLine { kind: "del".into(), text: "  const b = 2;".into(), old_line: Some(11), new_line: None });
        assert_eq!(login.hunks[0].lines[3].new_line, Some(12));
        assert_eq!(login.hunks[0].lines[4], PatchLine { kind: "context".into(), text: "  return a;".into(), old_line: Some(12), new_line: Some(13) });
        assert_eq!(login.hunks[1].lines.len(), 2);
    }

    #[test]
    fn a_line_that_looks_like_a_file_header_inside_a_hunk_stays_a_line() {
        let files = parse_patch(PATCH);

        assert_eq!(files[1].additions, 2);
        assert_eq!(files[1].hunks[0].lines[1].text, "diff --git a/x b/x is only text here");
    }

    #[test]
    fn binary_files_renames_and_quoted_paths() {
        let files = parse_patch(PATCH);

        assert!(files[2].binary);
        assert_eq!(files[3].old_path.as_deref(), Some("old name.txt"));
        assert_eq!(unquote("\"src/\\305\\274\\303\\263\\305\\202w.txt\""), "src/żółw.txt");
        assert_eq!(unquote("\"a\\tb\""), "a\tb");
    }

    #[test]
    fn an_untracked_file_is_all_additions() {
        let file = untracked_file("notes.md", Some("one\ntwo\n"));

        assert_eq!((file.status.as_str(), file.additions), ("untracked", 2));
        assert!(untracked_file("blob.bin", None).binary);
    }

    #[test]
    fn many_files_keep_their_order() {
        let many: String = (0..200).map(|index| format!("diff --git a/f{index}.txt b/f{index}.txt\n--- a/f{index}.txt\n+++ b/f{index}.txt\n@@ -1 +1 @@\n-a\n+b\n")).collect();
        let files = parse_patch(&many);

        assert_eq!(files.len(), 200);
        assert!(files.iter().enumerate().all(|(index, file)| file.path == format!("f{index}.txt")));
    }
}
