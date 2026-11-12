//! Line diff between two versions of a text, for page history.

use serde::Serialize;
use ts_rs::TS;

use crate::types::string_enum;
use serde::Deserialize;

string_enum!(DiffKind { Same => "same", Added => "added", Removed => "removed" });

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

/// Above this many line pairs the table would be too large; the diff degrades to "all removed, all added".
const MAX_CELLS: usize = 4_000_000;

pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let line = |kind, text: &str| DiffLine { kind, text: text.to_string() };

    let prefix = old_lines.iter().zip(&new_lines).take_while(|(left, right)| left == right).count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];
    let mut result: Vec<DiffLine> = old_lines[..prefix].iter().map(|text| line(DiffKind::Same, text)).collect();

    if old_mid.len() * new_mid.len() > MAX_CELLS {
        result.extend(old_mid.iter().map(|text| line(DiffKind::Removed, text)));
        result.extend(new_mid.iter().map(|text| line(DiffKind::Added, text)));
    } else {
        let width = new_mid.len() + 1;
        let mut table = vec![0u32; (old_mid.len() + 1) * width];

        for row in (0..old_mid.len()).rev() {
            for column in (0..new_mid.len()).rev() {
                table[row * width + column] = if old_mid[row] == new_mid[column] {
                    table[(row + 1) * width + column + 1] + 1
                } else {
                    table[(row + 1) * width + column].max(table[row * width + column + 1])
                };
            }
        }

        let (mut row, mut column) = (0, 0);

        while row < old_mid.len() && column < new_mid.len() {
            if old_mid[row] == new_mid[column] {
                result.push(line(DiffKind::Same, old_mid[row]));
                row += 1;
                column += 1;
            } else if table[(row + 1) * width + column] >= table[row * width + column + 1] {
                result.push(line(DiffKind::Removed, old_mid[row]));
                row += 1;
            } else {
                result.push(line(DiffKind::Added, new_mid[column]));
                column += 1;
            }
        }

        result.extend(old_mid[row..].iter().map(|text| line(DiffKind::Removed, text)));
        result.extend(new_mid[column..].iter().map(|text| line(DiffKind::Added, text)));
    }

    result.extend(old_lines[old_lines.len() - suffix..].iter().map(|text| line(DiffKind::Same, text)));

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(lines: &[DiffLine]) -> String {
        lines
            .iter()
            .map(|line| match line.kind {
                DiffKind::Same => format!(" {}", line.text),
                DiffKind::Added => format!("+{}", line.text),
                DiffKind::Removed => format!("-{}", line.text),
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn marks_changed_lines_between_unchanged_ones() {
        assert_eq!(summary(&diff_lines("a\nb\nc\nd", "a\nx\nc\nd\ne")), " a|-b|+x| c| d|+e");
    }

    #[test]
    fn handles_empty_sides_and_identical_texts() {
        assert_eq!(summary(&diff_lines("", "a\nb")), "+a|+b");
        assert_eq!(summary(&diff_lines("a", "")), "-a");
        assert_eq!(summary(&diff_lines("a\nb", "a\nb")), " a| b");
    }
}
