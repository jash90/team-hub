//! A last look at what a push would send. A change to what runs in CI or during a build, and
//! anything that reads like a secret, keeps the push waiting until the person confirms it by
//! name. Every commit is read on its own: a secret added and removed again still travels.

use std::sync::LazyLock;

use rayon::prelude::*;
use regex::Regex;

use crate::model::PushFinding;

static SECRET_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
        r"|\bth_[A-Za-z0-9]{24,}",
        r"|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{20,}",
        r"|glpat-[A-Za-z0-9_-]{20}",
        r"|xox[abpr]-[A-Za-z0-9-]{10,}",
        r"|AKIA[0-9A-Z]{16}",
        r"|sk-ant-[A-Za-z0-9_-]{20,}",
        r"|sk-[A-Za-z0-9]{32,}",
        r"|AIza[0-9A-Za-z_-]{35}",
        r#"|(?i)(password|passwd|secret|token|api[_-]?key)\s*[:=]\s*["'][^"'\s]{8,}["']"#,
    ))
    .expect("valid regex")
});
static SECRET_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|/)(\.env(\.[^/]*)?|id_rsa|id_ed25519|[^/]*\.(pem|p12|pfx|jks|keystore)|\.npmrc|\.netrc|\.pypirc|credentials(\.json)?)$")
        .expect("valid regex")
});
/// Files whose change runs code on CI runners, in every build or on every commit.
static RUNS_IN_CI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(^|/)(\.github/.*|\.gitlab-ci\.ya?ml|\.gitlab/.*|\.circleci/.*|\.buildkite/.*|Jenkinsfile|azure-pipelines\.ya?ml|\.travis\.ya?ml",
        r"|bitbucket-pipelines\.ya?ml|Makefile|Dockerfile[^/]*|docker-compose[^/]*\.ya?ml|package\.json|build\.rs|Cargo\.toml|\.cargo/.*",
        r"|[^/]*\.gradle(\.kts)?|gradle/wrapper/.*|pom\.xml|Gemfile|Podfile|fastlane/.*|setup\.py|pyproject\.toml|\.husky/.*",
        r"|\.pre-commit-config\.ya?ml|\.gitmodules|\.gitattributes)$",
    ))
    .expect("valid regex")
});

const SYMLINK: &str = "120000";
const SUBMODULE: &str = "160000";
const EXECUTABLE: &str = "100755";

/// What git says about one commit, in forms that cannot be confused: `diff-tree --raw -z` for
/// paths and modes, `--numstat -z` for binary files, the text patch for content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitReading {
    pub commit: String,
    pub raw: String,
    pub numstat: String,
    pub patch: String,
    pub message: String,
}

fn finding(kind: &str, commit: &str, path: Option<&str>, detail: String) -> PushFinding {
    PushFinding { kind: kind.to_string(), path: path.map(String::from), commit: Some(commit.chars().take(8).collect()), detail }
}

fn path_findings(reading: &CommitReading) -> Vec<PushFinding> {
    let commit = reading.commit.as_str();
    let unreadable = |problem: &str| vec![finding("unreadable", commit, None, format!("the list of changed files could not be read ({problem})"))];
    let mut fields = reading.raw.split('\0').filter(|field| !field.is_empty());
    let mut findings = Vec::new();

    while let Some(meta) = fields.next() {
        let Some(path) = fields.next() else { return unreadable("a change without a path") };
        let parts: Vec<&str> = meta.strip_prefix(':').unwrap_or_default().split(' ').collect();
        let [old_mode, new_mode, _, _, status] = parts.as_slice() else { return unreadable("an entry in an unknown form") };
        let modes = [*old_mode, *new_mode];
        let found = match () {
            _ if path.contains('\u{FFFD}') => Some(("unreadable", "its path is not valid text")),
            _ if modes.contains(&SYMLINK) => Some(("link", "it is a symbolic link")),
            _ if modes.contains(&SUBMODULE) => Some(("link", "it is a submodule")),
            _ if *status == "T" || (*status == "M" && old_mode != new_mode) => Some(("mode", "its type or permissions change")),
            _ if *status == "A" && *new_mode == EXECUTABLE => Some(("mode", "it is a new executable file")),
            _ if SECRET_FILE.is_match(path) => Some(("secret", "files with this name usually hold secrets")),
            _ if RUNS_IN_CI.is_match(path) => Some(("ci", "it changes what CI or the build runs")),
            _ => None,
        };

        if let Some((kind, why)) = found {
            findings.push(finding(kind, commit, Some(path), format!("{path}: {why}")));
        }
    }

    findings
}

fn binary_findings(reading: &CommitReading) -> Vec<PushFinding> {
    reading
        .numstat
        .split('\0')
        .filter_map(|entry| entry.strip_prefix("-\t-\t"))
        .map(|path| finding("binary", &reading.commit, Some(path), format!("{path}: a binary file, which nobody can read in the diff")))
        .collect()
}

/// The file each added line belongs to, from the patch's own `+++` lines.
fn content_findings(reading: &CommitReading) -> Vec<PushFinding> {
    let mut findings: Vec<PushFinding> = Vec::new();
    let mut file: Option<&str> = None;
    let mut in_hunk = false;

    for line in reading.patch.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
        } else if line.starts_with("@@ ") {
            in_hunk = true;
        } else if !in_hunk {
            if let Some(path) = line.strip_prefix("+++ ") {
                file = Some(path.strip_prefix("b/").unwrap_or(path));
            }
        } else if line.starts_with('+') && SECRET_TEXT.is_match(line) {
            let already = findings.iter().any(|found| found.path.as_deref() == file);

            if !already {
                let place = file.unwrap_or("a file");

                findings.push(finding("secret", &reading.commit, file, format!("{place}: an added line looks like a key, a token or a password")));
            }
        }
    }

    if SECRET_TEXT.is_match(&reading.message) {
        findings.push(finding("secret", &reading.commit, None, "the commit message looks like it holds a key, a token or a password".to_string()));
    }

    findings
}

fn commit_findings(reading: &CommitReading) -> Vec<PushFinding> {
    [path_findings(reading), binary_findings(reading), content_findings(reading)].concat()
}

/// Everything in the pushed commits that a person has to confirm. Empty: the push is ordinary code.
pub fn review(readings: &[CommitReading]) -> Vec<PushFinding> {
    let mut findings: Vec<PushFinding> = readings.par_iter().flat_map_iter(commit_findings).collect();

    // The same file flagged the same way in several commits is said once.
    let mut seen = std::collections::HashSet::new();

    findings.retain(|found| seen.insert((found.kind.clone(), found.path.clone(), found.path.is_none().then(|| found.commit.clone()))));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(entries: &[(&str, &str, &str, &str)]) -> String {
        entries.iter().map(|(old, new, status, path)| format!(":{old} {new} {} {} {status}\0{path}\0", "a".repeat(40), "b".repeat(40))).collect()
    }

    fn reading(raw: String, patch: &str) -> CommitReading {
        CommitReading { commit: "c".repeat(40), raw, numstat: String::new(), patch: patch.to_string(), message: "WEB-1 fix the form".to_string() }
    }

    fn kinds(findings: &[PushFinding]) -> Vec<&str> {
        findings.iter().map(|found| found.kind.as_str()).collect()
    }

    #[test]
    fn ordinary_code_goes_without_remarks() {
        let patch =
            "diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1 +1 @@\n-const token = read();\n+const token = readToken();\n";

        assert!(review(&[reading(raw(&[("100644", "100644", "M", "src/a.ts")]), patch)]).is_empty());
    }

    #[test]
    fn ci_and_build_files_wait_for_a_confirmation() {
        for path in [
            ".github/workflows/ci.yml",
            "package.json",
            "web/package.json",
            "Dockerfile",
            "build.rs",
            "Makefile",
            "android/app/build.gradle",
            ".gitlab-ci.yml",
        ] {
            let findings = review(&[reading(raw(&[("100644", "100644", "M", path)]), "")]);

            assert_eq!(kinds(&findings), vec!["ci"], "{path}");
        }

        assert!(review(&[reading(raw(&[("100644", "100644", "M", "docs/package.json.md")]), "")]).is_empty());
    }

    #[test]
    fn secrets_are_found_in_added_lines_file_names_and_commit_messages() {
        let patch = "diff --git a/src/config.ts b/src/config.ts\n--- a/src/config.ts\n+++ b/src/config.ts\n@@ -1,2 +1,3 @@\n const a = 1;\n-const old = \"ghp_0123456789012345678901234567890123\";\n+const apiKey = \"abcdef0123456789\";\n+const other = \"ghp_0123456789012345678901234567890123\";\n";
        let findings = review(&[reading(raw(&[("100644", "100644", "M", "src/config.ts")]), patch)]);

        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].kind.as_str(), findings[0].path.as_deref()), ("secret", Some("src/config.ts")));
        assert_eq!(kinds(&review(&[reading(raw(&[("000000", "100644", "A", "deploy/.env.production")]), "")])), vec!["secret"]);

        let mut with_message = reading(String::new(), "");
        with_message.message = "use password = \"hunter2hunter2\" for now".to_string();

        assert_eq!(kinds(&review(&[with_message])), vec!["secret"]);
    }

    #[test]
    fn a_removed_secret_is_not_flagged_but_one_added_in_an_earlier_commit_is() {
        let removes = "diff --git a/k b/k\n--- a/k\n+++ b/k\n@@ -1 +0,0 @@\n-AKIAABCDEFGHIJKLMNOP\n";
        let adds = "diff --git a/k b/k\n--- /dev/null\n+++ b/k\n@@ -0,0 +1 @@\n+AKIAABCDEFGHIJKLMNOP\n";

        assert!(review(&[reading(raw(&[("100644", "000000", "D", "k")]), removes)]).is_empty());
        assert_eq!(
            kinds(&review(&[reading(raw(&[("000000", "100644", "A", "k")]), adds), reading(raw(&[("100644", "000000", "D", "k")]), removes)])),
            vec!["secret"]
        );
    }

    #[test]
    fn links_modes_binaries_and_unreadable_listings_wait() {
        let mut binary = reading(raw(&[("000000", "100644", "A", "logo.png")]), "");
        binary.numstat = "-\t-\tlogo.png\0".to_string();

        assert_eq!(kinds(&review(&[binary])), vec!["binary"]);
        assert_eq!(kinds(&review(&[reading(raw(&[("000000", "120000", "A", "link")]), "")])), vec!["link"]);
        assert_eq!(kinds(&review(&[reading(raw(&[("000000", "160000", "A", "vendor/lib")]), "")])), vec!["link"]);
        assert_eq!(kinds(&review(&[reading(raw(&[("100644", "100755", "M", "run.sh")]), "")])), vec!["mode"]);
        assert_eq!(kinds(&review(&[reading(raw(&[("000000", "100755", "A", "run.sh")]), "")])), vec!["mode"]);
        assert_eq!(kinds(&review(&[reading("garbage\0path\0".to_string(), "")])), vec!["unreadable"]);
    }

    #[test]
    fn a_line_inside_a_hunk_cannot_pose_as_a_file_header() {
        let patch = "diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1 +1,2 @@\n+++ b/innocent.txt\n+const password = \"correcthorsebattery\";\n";
        let findings = review(&[reading(raw(&[("100644", "100644", "M", "src/a.ts")]), patch)]);

        assert_eq!(findings[0].path.as_deref(), Some("src/a.ts"));
    }
}
