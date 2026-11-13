//! Git, as the app uses it: looking at clones, one working tree and branch per thread, the
//! thread's diff, and the push that a person approved. Every function blocks.
//!
//! The main clone's checkout is never touched: no checkout, no reset, no commit happens in it.
//! Working trees live outside it, and a thread can leave anything in its own tree, git
//! settings included, so the app's calls run no hook, no file-system monitor, no pager and no
//! external diff, and name the tree's administrative folder themselves.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::diff::{parse_patch, untracked_file};
use crate::error::{CoreError, CoreResult};
use crate::model::{CommitInfo, PatchFile};
use crate::push_guard::CommitReading;

const FALLBACK_BASES: [&str; 3] = ["origin/main", "origin/master", "origin/develop"];
/// More commits than one thread makes; past it the push is not read, so it does not go.
pub const MAX_PUSHED_COMMITS: usize = 100;
const MAX_UNTRACKED_BYTES: u64 = 200_000;
const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

fn hardened_git() -> Command {
    let mut command = Command::new("git");

    command
        .args(["-c", "core.fsmonitor=", "-c", &format!("core.hooksPath={NULL_DEVICE}"), "-c", "core.pager=cat", "-c", "diff.external="])
        .args(["-c", "core.quotePath=false", "-c", "color.ui=never"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    command
}

fn finished(output: Output, arguments: &[&str]) -> CoreResult<String> {
    if !output.status.success() {
        return Err(CoreError::git(arguments.first().copied().unwrap_or_default(), String::from_utf8_lossy(&output.stderr).trim().to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim_end_matches(['\n', '\r']).to_string())
}

fn git(folder: &Path, arguments: &[&str]) -> CoreResult<String> {
    finished(hardened_git().arg("-C").arg(folder).args(arguments).output()?, arguments)
}

/// Git for judging what would be pushed: additionally no global config, no attribute files
/// and no text conversion, so nothing a thread wrote changes what is read.
fn review_git(repo: &Path, arguments: &[&str]) -> CoreResult<String> {
    let output = hardened_git()
        .args(["-c", &format!("core.attributesFile={NULL_DEVICE}"), "-c", "diff.noprefix=false"])
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_ATTR_NOSYSTEM", "1")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .output()?;

    finished(output, arguments)
}

fn exists(repo: &Path, reference: &str) -> bool {
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")]).is_ok()
}

fn is_full_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A branch name that cannot be read as an option or as another kind of ref.
pub fn is_branch_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("@{")
        && !name.contains("//")
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
}

// ---------- looking at a clone ----------

/// `git@github.com:Acme/web.git`, `https://github.com/acme/web`, `ssh://git@github.com/acme/web.git` → `acme/web`.
pub fn repo_of_remote(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let path = match trimmed.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => trimmed.split_once(':').map_or(trimmed, |(_, path)| path),
    };
    let mut parts = path.rsplit('/');
    let name = parts.next().filter(|name| !name.is_empty())?;
    let owner = parts.next().filter(|owner| !owner.is_empty())?;

    Some(format!("{owner}/{name}").to_lowercase())
}

pub fn remote_matches(url: &str, github_repo: &str) -> bool {
    repo_of_remote(url).is_some_and(|found| found == github_repo.trim().to_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: String,
}

/// The output of `git worktree list --porcelain` without the clone's own checkout (the first entry).
pub fn parse_worktrees(porcelain: &str) -> Vec<Listed> {
    porcelain
        .split("\n\n")
        .skip(1)
        .filter_map(|entry| {
            let field = |name: &str| entry.lines().find_map(|line| line.strip_prefix(name));

            Some(Listed {
                path: PathBuf::from(field("worktree ")?),
                branch: field("branch refs/heads/").map(str::to_owned),
                head: field("HEAD ")?.chars().take(8).collect(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub root: PathBuf,
    pub remote_url: Option<String>,
    /// `origin/main`, as far as the clone knows it. Nothing is fetched.
    pub default_branch: Option<String>,
    pub current_branch: Option<String>,
    pub changed_files: u32,
    pub worktrees: Vec<Listed>,
}

/// The ref new work starts from: the remote's default branch, else the clone's own.
pub fn base_ref(repo: &Path) -> CoreResult<String> {
    if let Ok(head) = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        return Ok(head);
    }

    if let Some(found) = FALLBACK_BASES.iter().find(|candidate| exists(repo, candidate)) {
        return Ok((*found).to_string());
    }

    for local in ["main", "master"] {
        if exists(repo, &format!("refs/heads/{local}")) {
            return Ok(local.to_string());
        }
    }

    git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// What a folder is, read without changing anything in it.
pub fn inspect(path: &Path) -> CoreResult<Inspection> {
    let root = PathBuf::from(git(path, &["rev-parse", "--show-toplevel"])?);
    let status = git(&root, &["status", "--porcelain"])?;
    let current = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).ok().filter(|name| name != "HEAD");

    Ok(Inspection {
        remote_url: git(&root, &["remote", "get-url", "origin"]).ok(),
        default_branch: base_ref(&root).ok(),
        current_branch: current,
        changed_files: status.lines().filter(|line| !line.is_empty()).count() as u32,
        worktrees: parse_worktrees(&git(&root, &["worktree", "list", "--porcelain"])?),
        root,
    })
}

// ---------- one working tree per thread ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    /// The ref the branch is compared with, e.g. `origin/main`.
    pub base: String,
    /// The commit the thread's own work starts after.
    pub base_commit: String,
    /// True when the branch was already there and the thread continues it.
    pub continued: bool,
}

/// A working tree at `path` with `branch` checked out. An existing branch (local, or the
/// remote's as last fetched) is continued; otherwise the branch is created from the base.
/// Git refuses a branch that is checked out elsewhere, and that refusal is passed on.
pub fn create_worktree(repo: &Path, path: &Path, branch: &str) -> CoreResult<Worktree> {
    if !is_branch_name(branch) {
        return Err(CoreError::git("worktree", format!("\"{branch}\" is not a usable branch name")));
    }

    if path.exists() {
        return Err(CoreError::git("worktree", format!("{} already exists", path.display())));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let base = base_ref(repo)?;
    let place = path.to_string_lossy().into_owned();
    let local = format!("refs/heads/{branch}");
    let remote = format!("refs/remotes/origin/{branch}");
    let continued = exists(repo, &local) || exists(repo, &remote);

    if exists(repo, &local) {
        git(repo, &["worktree", "add", &place, branch])?;
    } else if exists(repo, &remote) {
        git(repo, &["worktree", "add", "-b", branch, &place, &remote])?;
    } else {
        git(repo, &["worktree", "add", "-b", branch, &place, &base])?;
    }

    let base_commit = git(repo, &["merge-base", &local, &base]).or_else(|_| git(repo, &["rev-parse", &base]))?;

    Ok(Worktree { path: path.to_path_buf(), branch: branch.to_string(), base, base_commit, continued })
}

/// Never forced: git refuses while the tree holds changes, and the branch is left alone.
pub fn remove_worktree(repo: &Path, worktree: &Path) -> CoreResult<()> {
    git(repo, &["worktree", "remove", &worktree.to_string_lossy()]).map(|_| ())
}

/// The administrative folder of a working tree, found from the clone's side: the entry under
/// `.git/worktrees` whose `gitdir` file names this tree. The `.git` file inside the tree is
/// not trusted, because the thread could have pointed it somewhere else.
fn admin_dir(repo: &Path, worktree: &Path) -> CoreResult<PathBuf> {
    let not_registered = || CoreError::git("worktree", "this working tree is not registered in the repository");
    let wanted = std::fs::canonicalize(worktree)?;
    let common = PathBuf::from(git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?);
    let registry = std::fs::canonicalize(common.join("worktrees")).map_err(|_| not_registered())?;

    for entry in std::fs::read_dir(&registry)?.flatten() {
        let pointer = std::fs::read_to_string(entry.path().join("gitdir")).unwrap_or_default();
        let named = Path::new(pointer.trim()).parent().map(Path::to_path_buf);

        if named.and_then(|named| std::fs::canonicalize(named).ok()).is_some_and(|named| named == wanted) {
            return Ok(entry.path());
        }
    }

    Err(not_registered())
}

fn git_in_tree(repo: &Path, worktree: &Path, arguments: &[&str]) -> CoreResult<String> {
    let admin = admin_dir(repo, worktree)?;
    let output = hardened_git().arg("--git-dir").arg(admin).arg("--work-tree").arg(worktree).args(arguments).current_dir(worktree).output()?;

    finished(output, arguments)
}

/// What shell commands of a thread may write to outside its working tree, and what they may
/// not touch even there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteAccess {
    pub writable: Vec<String>,
    pub protected: Vec<String>,
}

/// A commit in a working tree needs four places in the clone's `.git`: the object store, the
/// tree's administrative folder, and the ref and reflog of its branch. Everything in `.git`
/// that runs code or redirects git (hooks, config, info, modules, the tree's pointers back to
/// the clone) stays closed.
pub fn write_access(repo: &Path, worktree: &Path, branch: &str) -> CoreResult<WriteAccess> {
    let common = std::fs::canonicalize(git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?)?;
    let admin = admin_dir(repo, worktree)?;
    let text = |path: PathBuf| path.to_string_lossy().into_owned();

    Ok(WriteAccess {
        writable: vec![
            text(common.join("objects")),
            text(common.join("refs/heads").join(branch)),
            text(common.join("logs/refs/heads").join(branch)),
            text(admin.clone()),
        ],
        protected: ["commondir", "gitdir", "config.worktree"]
            .map(|name| admin.join(name))
            .into_iter()
            .chain(["hooks", "config", "info", "modules"].map(|name| common.join(name)))
            .map(text)
            .collect(),
    })
}

fn commits_between(repo: &Path, range: &str) -> CoreResult<Vec<CommitInfo>> {
    let log = review_git(repo, &["log", "--reverse", "--format=%H%x1f%h%x1f%s%x1f%an%x1f%aI%x1e", range])?;

    Ok(log
        .split('\u{1e}')
        .filter_map(|entry| {
            let mut fields = entry.trim().split('\u{1f}');

            Some(CommitInfo {
                sha: fields.next().filter(|sha| !sha.is_empty())?.to_string(),
                short: fields.next()?.to_string(),
                subject: fields.next()?.to_string(),
                author: fields.next()?.to_string(),
                at: fields.next()?.to_string(),
            })
        })
        .collect())
}

/// Files in the tree that git does not track and does not ignore, with their text when they
/// are small text files. Links are named, never followed.
fn untracked(repo: &Path, worktree: &Path) -> CoreResult<Vec<PatchFile>> {
    let listing = git_in_tree(repo, worktree, &["ls-files", "--others", "--exclude-standard", "-z"])?;

    Ok(listing
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| {
            let file = worktree.join(path);
            let readable = std::fs::symlink_metadata(&file).is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_UNTRACKED_BYTES);
            let content = readable.then(|| std::fs::read(&file).ok()).flatten().and_then(|bytes| String::from_utf8(bytes).ok());

            untracked_file(path, content.as_deref())
        })
        .collect())
}

/// Everything the thread changed since `base_commit`: its commits, and the files as they are
/// in the tree now, committed or not. The index is not touched.
pub fn thread_changes(repo: &Path, worktree: &Path, base_commit: &str) -> CoreResult<(Vec<PatchFile>, Vec<CommitInfo>, u32)> {
    if !is_full_sha(base_commit) {
        return Err(CoreError::git("diff", "the thread's starting commit is not a commit id"));
    }

    let patch = git_in_tree(repo, worktree, &["diff", "--no-ext-diff", "--no-textconv", "--no-color", "-M", base_commit])?;
    let status = git_in_tree(repo, worktree, &["status", "--porcelain"])?;
    let head = git_in_tree(repo, worktree, &["rev-parse", "HEAD"])?;
    let mut files = parse_patch(&patch);

    files.extend(untracked(repo, worktree)?);

    Ok((files, commits_between(repo, &format!("{base_commit}..{head}"))?, status.lines().filter(|line| !line.is_empty()).count() as u32))
}

pub fn head(repo: &Path, worktree: &Path) -> CoreResult<String> {
    git_in_tree(repo, worktree, &["rev-parse", "HEAD"])
}

// ---------- the push a person approved ----------

/// What a push of `branch` would send, pinned to the commit the branch points at now.
#[derive(Debug, Clone, PartialEq)]
pub struct PushRead {
    pub commit: String,
    /// What the server is taken to have already.
    pub base: String,
    pub remote_url: String,
    pub commits: Vec<CommitInfo>,
    pub files: Vec<PatchFile>,
    pub readings: Vec<CommitReading>,
    /// Why the push cannot go at all.
    pub blocked: Option<String>,
}

/// The server's branch as last seen, else the point where the branch left the base.
fn pushed_base(repo: &Path, branch: &str, fallback: &str) -> CoreResult<String> {
    let tracked = format!("refs/remotes/origin/{branch}");

    if exists(repo, &tracked) {
        return review_git(repo, &["rev-parse", &tracked]);
    }

    let base = base_ref(repo)?;

    review_git(repo, &["merge-base", &format!("refs/heads/{branch}"), &base]).or_else(|_| Ok(fallback.to_string()))
}

/// Reads what pushing `commit` (or the branch's tip) onto `branch` would send. Each commit is
/// read on its own against its parent, because a push sends history, not only the final state.
pub fn read_push(repo: &Path, branch: &str, commit: Option<&str>, fork_point: &str) -> CoreResult<PushRead> {
    if !is_branch_name(branch) {
        return Err(CoreError::git("push", format!("\"{branch}\" is not a usable branch name")));
    }

    let commit = match commit {
        Some(sha) if is_full_sha(sha) && exists(repo, sha) => sha.to_string(),
        Some(_) => return Err(CoreError::git("push", "the commit to push is not a full commit id of this repository")),
        None => review_git(repo, &["rev-parse", "--verify", &format!("refs/heads/{branch}^{{commit}}")])?,
    };
    let remote_url = git(repo, &["remote", "get-url", "origin"]).map_err(|_| CoreError::git("push", "the repository has no remote called origin"))?;
    let base = pushed_base(repo, branch, fork_point)?;
    let mut read = PushRead { commit: commit.clone(), base: base.clone(), remote_url, commits: Vec::new(), files: Vec::new(), readings: Vec::new(), blocked: None };

    if base == commit {
        read.blocked = Some("There is nothing to push: the server already has this commit.".to_string());

        return Ok(read);
    }

    if review_git(repo, &["merge-base", "--is-ancestor", &base, &commit]).is_err() {
        read.blocked = Some("The branch no longer grows out of what the server has (its history was rewritten). The app never force-pushes.".to_string());

        return Ok(read);
    }

    let range = format!("{base}..{commit}");
    let listing = review_git(repo, &["rev-list", "--reverse", "--parents", &format!("--max-count={}", MAX_PUSHED_COMMITS + 1), &range])?;
    let lines: Vec<&str> = listing.lines().filter(|line| !line.is_empty()).collect();

    if lines.len() > MAX_PUSHED_COMMITS {
        read.blocked = Some(format!("More than {MAX_PUSHED_COMMITS} commits: too many to read before pushing."));

        return Ok(read);
    }

    for line in &lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [sha, _parent] = parts[..] else {
            read.blocked = Some("The push contains a merge commit or a commit without a parent, which cannot be read as one change.".to_string());

            return Ok(read);
        };
        let tree = ["diff-tree", "-r", "--no-commit-id", "--no-renames", "--no-ext-diff", "--no-textconv"];
        let patch = ["-p", "--text", "--no-color", "--unified=0", "--src-prefix=a/", "--dst-prefix=b/"];

        read.readings.push(CommitReading {
            commit: sha.to_string(),
            raw: review_git(repo, &[&tree[..], &["--raw", "-z", "--no-abbrev", sha]].concat())?,
            numstat: review_git(repo, &[&tree[..], &["--numstat", "-z", sha]].concat())?,
            patch: review_git(repo, &[&tree[..], &patch[..], &[sha]].concat())?,
            message: review_git(repo, &["log", "-1", "--format=%B", sha])?,
        });
    }

    read.commits = commits_between(repo, &range)?;
    read.files = parse_patch(&review_git(repo, &["diff", "--no-ext-diff", "--no-textconv", "--no-color", "-M", &base, &commit])?);

    Ok(read)
}

/// Sends exactly `commit` to `branch` on origin. Never forced: when the server's branch has
/// moved on, git refuses and so does this.
pub fn push(repo: &Path, branch: &str, commit: &str) -> CoreResult<()> {
    if !is_branch_name(branch) || !is_full_sha(commit) {
        return Err(CoreError::git("push", "the branch or the commit is not spelled in full"));
    }

    let refspec = format!("{commit}:refs/heads/{branch}");
    let output = hardened_git().arg("-C").arg(repo).args(["push", "--porcelain", "origin", &refspec]).env_remove("GIT_OPTIONAL_LOCKS").output()?;

    finished(output, &["push"]).map(|_| ())
}

/// Whether the server's branch, as this clone last saw it, is at `commit`.
pub fn is_pushed(repo: &Path, branch: &str, commit: &str) -> bool {
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]).is_ok_and(|tip| tip == commit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_are_recognised_in_every_spelling() {
        for url in [
            "git@github.com:Acme/Web.git",
            "https://github.com/acme/web",
            "https://github.com/acme/web.git/",
            "ssh://git@github.com/acme/web.git",
            "https://token@github.com/acme/web.git",
        ] {
            assert_eq!(repo_of_remote(url).as_deref(), Some("acme/web"), "{url}");
            assert!(remote_matches(url, "Acme/web"));
        }

        assert!(!remote_matches("git@github.com:acme/api.git", "acme/web"));
        assert_eq!(repo_of_remote("/srv/git/web.git").as_deref(), Some("git/web"));
        assert_eq!(repo_of_remote("web"), None);
    }

    #[test]
    fn a_branch_name_cannot_smuggle_an_option_or_another_ref() {
        assert!(is_branch_name("feature/WEB-12-login-form"));
        assert!(is_branch_name("fix/WEB-1"));

        for bad in ["", "-f", "--upload-pack=x", "a..b", "a b", "refs/heads/x:refs/heads/y", "x@{1}", "a//b", "/a", "a/", "a.lock", "a;b", "$(x)"] {
            assert!(!is_branch_name(bad), "{bad}");
        }
    }

    #[test]
    fn the_worktree_listing_skips_the_main_checkout() {
        let porcelain = "worktree /repos/web\nHEAD 1111111111111111\nbranch refs/heads/main\n\nworktree /data/worktrees/web/web-12\nHEAD 2222222222222222\nbranch refs/heads/feature/WEB-12-login\n\nworktree /data/worktrees/web/detached\nHEAD 3333333333333333\ndetached\n";
        let listed = parse_worktrees(porcelain);

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].branch.as_deref(), Some("feature/WEB-12-login"));
        assert_eq!((listed[1].branch.as_deref(), listed[1].head.as_str()), (None, "33333333"));
    }
}
