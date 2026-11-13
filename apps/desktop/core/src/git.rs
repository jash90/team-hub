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
use crate::secrets::Secret;

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

/// The person's global ignore file, as git itself would find it.
pub fn global_excludes() -> Option<String> {
    let configured =
        Command::new("git").args(["config", "--global", "--get", "core.excludesFile"]).output().ok().filter(|output| output.status.success());
    let named = configured.map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string()).filter(|path| !path.is_empty());
    let home = dirs::home_dir().unwrap_or_default();
    let path = match named {
        Some(path) => path.strip_prefix("~/").map(|rest| home.join(rest)).unwrap_or_else(|| PathBuf::from(&path)),
        None => dirs::config_dir().unwrap_or_else(|| home.join(".config")).join("git/ignore"),
    };

    std::fs::read_to_string(path).ok()
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

    // A tree whose folder somebody deleted is still on git's list; forgetting it is what
    // `git worktree prune` is for, and it touches nothing that exists.
    git(repo, &["worktree", "prune"])?;

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

/// Entries of a folder other than the one named.
fn others(folder: &Path, keep: &std::ffi::OsStr) -> Vec<PathBuf> {
    std::fs::read_dir(folder)
        .map(|entries| entries.flatten().filter(|entry| entry.file_name() != keep).map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

/// Gives every local branch a file of its own under `refs/heads`. Branches that live only in
/// `packed-refs` have none, and a path that does not exist cannot be closed to a thread: it
/// could then create the file and so move the branch. The value written is the one the branch
/// already has, which is what git itself writes on the branch's next update.
fn unpack_branches(repo: &Path, common: &Path) -> CoreResult<()> {
    let listing = git(repo, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads"])?;

    for line in listing.lines() {
        let Some((name, commit)) = line.split_once(' ') else { continue };
        let file = common.join(name);

        if !file.exists() && is_full_sha(commit) {
            if let Some(folder) = file.parent() {
                std::fs::create_dir_all(folder)?;
            }

            std::fs::write(file, format!("{commit}\n"))?;
        }
    }

    Ok(())
}

/// What a thread's shell may write in the clone's `.git`, and what it may not.
///
/// A commit in a working tree needs four places: the object store, the tree's administrative
/// folder, and the folders that hold the ref and the reflog of its branch (git updates a ref
/// by writing `<ref>.lock` beside it and renaming, so the folder has to be writable, not only
/// the file). The agent's sandbox opens the whole `.git` of a working tree's clone by itself,
/// so everything else is closed here by name: every other branch (a thread must not be able
/// to move `main` under the main checkout), remote-tracking refs (a push is judged against
/// them), tags, `HEAD`, the index, `packed-refs`, hooks, config, the packs, the other
/// threads' trees, and the pointers of this tree back to the clone.
pub fn write_access(repo: &Path, worktree: &Path, branch: &str) -> CoreResult<WriteAccess> {
    let common = std::fs::canonicalize(git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?)?;
    let admin = admin_dir(repo, worktree)?;
    let text = |path: PathBuf| path.to_string_lossy().into_owned();
    let name = |text: &'static str| std::ffi::OsStr::new(text);

    unpack_branches(repo, &common)?;

    // Closed even when they do not exist yet, so they cannot be created either.
    let mut protected: Vec<PathBuf> = ["hooks", "config", "info", "modules", "HEAD", "index", "packed-refs", "objects/pack", "objects/info"]
        .map(|entry| common.join(entry))
        .to_vec();
    let open_at_top = ["objects", "refs", "logs", "worktrees"];

    protected.extend(others(&common, name("")).into_iter().filter(|path| !open_at_top.iter().any(|open| path.file_name() == Some(name(open)))));
    protected.extend(others(&common.join("refs"), name("heads")));
    protected.extend(others(&common.join("logs"), name("refs")));
    protected.extend(others(&common.join("logs/refs"), name("heads")));
    protected.extend(others(&common.join("worktrees"), admin.file_name().unwrap_or_default()));
    protected.extend(["commondir", "gitdir", "config.worktree"].map(|entry| admin.join(entry)));

    // Down the branch's path, everything beside it is closed: the other branches.
    let mut folders = Vec::new();

    for root in ["refs/heads", "logs/refs/heads"] {
        let mut folder = common.join(root);

        // The sandbox can only open a folder that exists; git would make these itself.
        std::fs::create_dir_all(common.join(root).join(branch).parent().unwrap_or(&folder))?;

        for part in Path::new(branch).components() {
            protected.extend(others(&folder, part.as_os_str()));
            folders.push(folder.clone());
            folder = folder.join(part);
        }
    }

    protected.sort();
    protected.dedup();

    let ref_folder = common.join("refs/heads").join(branch).parent().map(Path::to_path_buf).unwrap_or_else(|| common.join("refs/heads"));
    let log_folder = common.join("logs/refs/heads").join(branch).parent().map(Path::to_path_buf).unwrap_or_else(|| common.join("logs/refs/heads"));

    Ok(WriteAccess {
        writable: [common.join("objects"), ref_folder, log_folder, admin].into_iter().map(text).collect(),
        protected: protected.into_iter().map(text).collect(),
    })
}

/// Commits what the thread changed, for threads that have no shell to run git in. Run by the
/// app itself, so with the same precautions as every other call here: no hooks, the tree's
/// administrative folder named from the clone's side. With no paths, everything changed in
/// the tree is committed. Returns the new commit as `abc1234 Subject`.
pub fn commit(repo: &Path, worktree: &Path, message: &str, paths: &[String]) -> CoreResult<String> {
    if message.trim().is_empty() {
        return Err(CoreError::git("commit", "the commit needs a message"));
    }

    if let Some(bad) = paths.iter().find(|path| path.starts_with('-') || path.starts_with('/') || path.split('/').any(|part| part == "..")) {
        return Err(CoreError::git("commit", format!("\"{bad}\" is not a path inside the working tree")));
    }

    let mut add = vec!["add", "--"];

    if paths.is_empty() {
        add = vec!["add", "--all"];
    } else {
        add.extend(paths.iter().map(String::as_str));
    }

    git_in_tree(repo, worktree, &add)?;
    git_in_tree(repo, worktree, &["commit", "--quiet", "--message", message.trim()])?;
    git_in_tree(repo, worktree, &["log", "-1", "--format=%h %s"])
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
    let mut read = PushRead {
        commit: commit.clone(),
        base: base.clone(),
        remote_url,
        commits: Vec::new(),
        files: Vec::new(),
        readings: Vec::new(),
        blocked: None,
    };

    if base == commit {
        read.blocked = Some("There is nothing to push: the server already has this commit.".to_string());

        return Ok(read);
    }

    if review_git(repo, &["merge-base", "--is-ancestor", &base, &commit]).is_err() {
        read.blocked =
            Some("The branch no longer grows out of what the server has (its history was rewritten). The app never force-pushes.".to_string());

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
    push_as(repo, branch, commit, None)
}

/// `push`, with a token for the remote where nobody is there to have signed git in.
pub fn push_as(repo: &Path, branch: &str, commit: &str, auth: Option<&RemoteAuth>) -> CoreResult<()> {
    if !is_branch_name(branch) || !is_full_sha(commit) {
        return Err(CoreError::git("push", "the branch or the commit is not spelled in full"));
    }

    let refspec = format!("{commit}:refs/heads/{branch}");
    let mut command = match auth {
        Some(auth) => remote_git(Some(auth)),
        None => hardened_git(),
    };
    let output = command.arg("-C").arg(repo).args(["push", "--porcelain", "origin", &refspec]).env_remove("GIT_OPTIONAL_LOCKS").output()?;

    finished(output, &["push"]).map(|_| ())
}

/// Whether the server's branch, as this clone last saw it, is at `commit`.
pub fn is_pushed(repo: &Path, branch: &str, commit: &str) -> bool {
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]).is_ok_and(|tip| tip == commit)
}

// ---------- clones a runner keeps by itself ----------

/// The variable the token travels in: set for the one git command that needs it, read by the
/// credential helper below, and part of nothing else.
const REMOTE_PASSWORD_VARIABLE: &str = "TEAM_HUB_GIT_PASSWORD";
/// Answers git's question for a password from that variable and keeps nothing: `store` and
/// `erase` are ignored. GitHub takes a token as the password of any user name.
const CREDENTIAL_HELPER: &str = "!f() { test \"$1\" = get && printf 'username=x-access-token\\npassword=%s\\n' \"$TEAM_HUB_GIT_PASSWORD\"; }; f";

/// A token for one git host. It reaches git through the environment of a single command and
/// is offered only to addresses under `base`; it is never written to a config or put in a
/// remote's address.
#[derive(Debug, Clone)]
pub enum RemoteAuth {
    Token {
        base: String,
        token: Secret,
    },
    /// However the person at this computer has signed git in (a credential helper, the
    /// GitHub CLI): the desktop app has no token of its own and leaves that as it is.
    PersonsOwn,
}

impl RemoteAuth {
    pub fn new(base: &str, token: Secret) -> Self {
        Self::Token { base: base.trim_end_matches('/').to_string(), token }
    }
}

/// Git for a command that talks to the remote without a person: no credential helper of the
/// machine sees the token or is asked to keep it, and nothing prompts.
fn remote_git(auth: Option<&RemoteAuth>) -> Command {
    let mut command = hardened_git();

    command.env_remove("GIT_OPTIONAL_LOCKS");

    match auth {
        Some(RemoteAuth::PersonsOwn) => {}
        Some(RemoteAuth::Token { base, token }) => {
            command
                .args(["-c", "credential.helper=", "-c", &format!("credential.{base}.helper={CREDENTIAL_HELPER}")])
                .env(REMOTE_PASSWORD_VARIABLE, token.expose());
        }
        None => {
            command.args(["-c", "credential.helper="]);
        }
    }

    command
}

/// Clones `url` into `path` without checking anything out: the clone only holds history, and
/// every piece of work gets a tree of its own.
pub fn clone(url: &str, path: &Path, auth: Option<&RemoteAuth>) -> CoreResult<()> {
    if url.starts_with('-') {
        return Err(CoreError::git("clone", "the repository's address is not usable"));
    }

    let output = remote_git(auth).args(["clone", "--quiet", "--no-checkout", "--", url]).arg(path).output()?;

    finished(output, &["clone"]).map(|_| ())
}

/// Brings the clone up to date with origin: branches, tags, and branches deleted there.
pub fn fetch(repo: &Path, auth: Option<&RemoteAuth>) -> CoreResult<()> {
    let output = remote_git(auth).arg("-C").arg(repo).args(["fetch", "--quiet", "--prune", "--tags", "origin"]).output()?;

    finished(output, &["fetch"]).map(|_| ())
}

pub fn remote_url(repo: &Path) -> CoreResult<String> {
    git(repo, &["remote", "get-url", "origin"])
}

/// The commit a branch, a tag or a commit id names, as the clone last fetched it. A branch
/// is read from origin, never from a local branch a thread may have moved.
pub fn resolve_commit(repo: &Path, reference: &str) -> CoreResult<String> {
    let reference = reference.trim();
    let unknown = || CoreError::git("rev-parse", format!("\"{reference}\" is not a branch, a tag or a commit of this repository"));

    if is_full_sha(reference) || (reference.len() >= 7 && reference.bytes().all(|byte| byte.is_ascii_hexdigit())) {
        if let Ok(commit) = git(repo, &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")]) {
            return Ok(commit);
        }
    }

    if !is_branch_name(reference) {
        return Err(unknown());
    }

    [format!("refs/remotes/origin/{reference}"), format!("refs/tags/{reference}")]
        .iter()
        .find_map(|name| git(repo, &["rev-parse", "--verify", "--quiet", &format!("{name}^{{commit}}")]).ok())
        .ok_or_else(unknown)
}

/// A checkout of one commit in a folder of its own, with a repository of its own that holds
/// only that commit: whoever works there cannot reach the clone it came from. `branch`
/// names the checkout when the commit was asked for by branch, so tools that look show it.
pub fn checkout_copy(repo: &Path, commit: &str, folder: &Path, branch: Option<&str>) -> CoreResult<()> {
    if !is_full_sha(commit) {
        return Err(CoreError::git("checkout", "the commit to check out is not a full commit id"));
    }

    std::fs::create_dir_all(folder)?;

    let source = std::fs::canonicalize(repo)?.to_string_lossy().into_owned();
    // A commit is asked for by id, which a repository hands out only when told it may.
    let upload = "git -c uploadpack.allowAnySHA1InWant=true upload-pack";

    git(folder, &["init", "--quiet"])?;
    git(folder, &["fetch", "--quiet", "--depth=1", "--no-tags", "--upload-pack", upload, &source, commit])?;

    match branch.filter(|name| is_branch_name(name)) {
        Some(name) => git(folder, &["checkout", "--quiet", "-B", name, "FETCH_HEAD"])?,
        None => git(folder, &["checkout", "--quiet", "--detach", "FETCH_HEAD"])?,
    };

    Ok(())
}

/// What a thread has committed: the commits after `base` up to `head`, and their diff.
#[derive(Debug, Clone, PartialEq)]
pub struct Committed {
    pub commits: Vec<CommitInfo>,
    pub files: Vec<PatchFile>,
    pub patch: String,
}

/// Read the way a push is read: nothing the thread configured changes what is shown.
pub fn committed(repo: &Path, base: &str, head: &str) -> CoreResult<Committed> {
    if !is_full_sha(base) || !is_full_sha(head) {
        return Err(CoreError::git("diff", "the commits to compare are not full commit ids"));
    }

    let patch = review_git(repo, &["diff", "--no-ext-diff", "--no-textconv", "--no-color", "-M", base, head])?;

    Ok(Committed { commits: commits_between(repo, &format!("{base}..{head}"))?, files: parse_patch(&patch), patch })
}

/// Whether origin has the branch at all, as this clone last saw it.
pub fn on_remote(repo: &Path, branch: &str) -> bool {
    is_branch_name(branch) && exists(repo, &format!("refs/remotes/origin/{branch}"))
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
