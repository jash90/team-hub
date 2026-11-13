//! Working trees, diffs and pushes on real temporary repositories.

mod support;

use std::path::Path;

use hub_desktop_core::git;
use hub_desktop_core::push_guard;
use support::{git as run, repo, write};

/// The main clone's checkout as a thread must leave it: HEAD, branch, index and files.
fn fingerprint(clone: &Path) -> (String, String, String, String) {
    (
        run(clone, &["rev-parse", "HEAD"]),
        run(clone, &["rev-parse", "--abbrev-ref", "HEAD"]),
        run(clone, &["status", "--porcelain"]),
        std::fs::read_to_string(clone.join("src/login.ts")).unwrap(),
    )
}

#[test]
fn a_thread_gets_a_new_branch_named_for_its_issue_and_the_dirty_main_clone_is_untouched() {
    let repo = repo("worktree-new");

    write(&repo.clone, "src/login.ts", "export const limit = 99; // my own unfinished work\n");
    write(&repo.clone, "scratch.txt", "untracked\n");

    let before = fingerprint(&repo.clone);
    let branch = hub_core::keys::suggest_branch_name("WEB-12", "Błąd logowania: 500", "Bug");
    let tree = repo.root.path().join("trees/web-12-t1");
    let made = git::create_worktree(&repo.clone, &tree, &branch).unwrap();

    assert_eq!(branch, "fix/WEB-12-blad-logowania-500");
    assert_eq!(made.branch, branch);
    assert_eq!(made.base, "origin/main");
    assert!(!made.continued);
    assert_eq!(made.base_commit, run(&repo.clone, &["rev-parse", "origin/main"]));
    assert_eq!(run(&tree, &["rev-parse", "--abbrev-ref", "HEAD"]), branch);
    // The tree starts from the server's main, not from what is lying in the main clone.
    assert_eq!(std::fs::read_to_string(tree.join("src/login.ts")).unwrap(), "export const limit = 5;\n");
    assert!(!tree.join("scratch.txt").exists());

    write(&tree, "src/login.ts", "export const limit = 10;\n");
    run(&tree, &["commit", "-am", "WEB-12: raise the limit"]);

    assert_eq!(fingerprint(&repo.clone), before);
}

#[test]
fn an_existing_branch_is_continued_whether_it_is_local_or_only_on_the_server() {
    let repo = repo("worktree-existing");

    run(&repo.clone, &["branch", "feature/WEB-1-login-form-validation"]);
    run(&repo.clone, &["push", "origin", "HEAD:refs/heads/feature/WEB-2-remote-only"]);
    run(&repo.clone, &["fetch", "origin"]);

    let local = git::create_worktree(&repo.clone, &repo.root.path().join("trees/a"), "feature/WEB-1-login-form-validation").unwrap();
    let remote = git::create_worktree(&repo.clone, &repo.root.path().join("trees/b"), "feature/WEB-2-remote-only").unwrap();

    assert!(local.continued && remote.continued);
    assert_eq!(run(&remote.path, &["rev-parse", "--abbrev-ref", "HEAD"]), "feature/WEB-2-remote-only");
    assert_eq!(remote.base_commit, run(&repo.clone, &["rev-parse", "origin/main"]));
}

#[test]
fn a_branch_checked_out_elsewhere_and_a_made_up_name_are_refused() {
    let repo = repo("worktree-refused");

    git::create_worktree(&repo.clone, &repo.root.path().join("trees/a"), "feature/WEB-1").unwrap();

    let twice = git::create_worktree(&repo.clone, &repo.root.path().join("trees/b"), "feature/WEB-1").unwrap_err();
    let main = git::create_worktree(&repo.clone, &repo.root.path().join("trees/c"), "main").unwrap_err();
    let option = git::create_worktree(&repo.clone, &repo.root.path().join("trees/d"), "--detach").unwrap_err();

    assert!(twice.to_string().contains("already"), "{twice}");
    assert!(main.to_string().contains("already"), "{main}");
    assert!(option.to_string().contains("not a usable branch name"));
    assert!(!repo.root.path().join("trees/d").exists());
}

#[test]
fn a_tree_whose_folder_was_deleted_by_hand_does_not_block_the_next_one() {
    let repo = repo("worktree-pruned");
    let tree = repo.root.path().join("trees/a");

    git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();
    std::fs::remove_dir_all(&tree).unwrap();

    let again = git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();

    assert!(again.continued);
    assert_eq!(run(&tree, &["rev-parse", "--abbrev-ref", "HEAD"]), "feature/WEB-1");
}

#[test]
fn a_tree_with_changes_is_not_removed_and_a_clean_one_is_while_the_branch_stays() {
    let repo = repo("worktree-remove");
    let tree = repo.root.path().join("trees/a");

    git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();
    write(&tree, "notes.md", "unsaved work\n");

    assert!(git::remove_worktree(&repo.clone, &tree).is_err());
    assert!(tree.join("notes.md").exists());

    run(&tree, &["add", "--all"]);
    run(&tree, &["commit", "-m", "WEB-1: notes"]);
    git::remove_worktree(&repo.clone, &tree).unwrap();

    assert!(!tree.exists());
    assert_eq!(run(&repo.clone, &["log", "-1", "--format=%s", "feature/WEB-1"]), "WEB-1: notes");
}

#[test]
fn the_threads_changes_are_its_commits_and_its_uncommitted_and_untracked_files() {
    let repo = repo("worktree-diff");
    let tree = repo.root.path().join("trees/a");
    let made = git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();

    write(&tree, "src/login.ts", "export const limit = 10;\nexport const window = 60;\n");
    run(&tree, &["commit", "-am", "WEB-1: raise the limit"]);
    write(&tree, "README.md", "# web\n\nNow with limits.\n");
    write(&tree, "docs/new file.md", "one\ntwo\n");
    std::fs::write(tree.join("logo.bin"), [0u8, 159, 146, 150]).unwrap();

    let index_before = std::fs::metadata(repo.clone.join(".git/worktrees/a/index")).unwrap().modified().unwrap();
    let (files, commits, uncommitted) = git::thread_changes(&repo.clone, &tree, &made.base_commit).unwrap();
    let named: Vec<(&str, &str)> = files.iter().map(|file| (file.path.as_str(), file.status.as_str())).collect();

    assert_eq!(named, vec![("README.md", "modified"), ("src/login.ts", "modified"), ("docs/new file.md", "untracked"), ("logo.bin", "untracked")]);
    assert_eq!((files[1].additions, files[1].deletions), (2, 1));
    assert_eq!(files[2].additions, 2);
    assert!(files[3].binary);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].subject, "WEB-1: raise the limit");
    assert_eq!(uncommitted, 3);
    assert_eq!(std::fs::metadata(repo.clone.join(".git/worktrees/a/index")).unwrap().modified().unwrap(), index_before);
}

#[test]
fn a_tree_whose_git_pointer_was_replaced_is_still_read_through_the_clones_own_record() {
    let repo = repo("worktree-pointer");
    let tree = repo.root.path().join("trees/a");
    let made = git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();
    let evil = repo.root.path().join("evil.git");

    run(repo.root.path(), &["init", "--bare", evil.to_str().unwrap()]);
    write(&evil, "hooks/post-checkout", "#!/bin/sh\ntouch \"$HOME/pwned\"\n");
    write(&tree, ".git", &format!("gitdir: {}\n", evil.display()));
    write(&tree, "src/login.ts", "export const limit = 7;\n");

    let (files, _, _) = git::thread_changes(&repo.clone, &tree, &made.base_commit).unwrap();

    assert_eq!(files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), vec!["src/login.ts"]);
}

#[test]
fn the_sandbox_opens_only_what_a_commit_needs_in_the_clones_git_folder() {
    let repo = repo("worktree-access");
    let tree = repo.root.path().join("trees/a");

    run(&repo.clone, &["branch", "feature/WEB-9-somebody-elses"]);
    git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();

    let access = git::write_access(&repo.clone, &tree, "feature/WEB-1").unwrap();
    let dot_git = repo.clone.join(".git");
    let text = |path: std::path::PathBuf| path.to_string_lossy().into_owned();

    // The folders of the branch's ref and reflog, not the files: git writes `<ref>.lock` beside the ref.
    assert_eq!(
        access.writable,
        vec![
            text(dot_git.join("objects")),
            text(dot_git.join("refs/heads/feature")),
            text(dot_git.join("logs/refs/heads/feature")),
            text(dot_git.join("worktrees/a"))
        ]
    );

    for closed in ["hooks", "config", "info", "modules", "worktrees/a/gitdir", "worktrees/a/commondir", "refs/heads/feature/WEB-9-somebody-elses"] {
        assert!(access.protected.contains(&text(dot_git.join(closed))), "{closed}");
    }

    assert!(!access.protected.contains(&text(dot_git.join("refs/heads/feature/WEB-1"))));

    // A branch without a folder of its own opens refs/heads, with every other branch closed by name.
    let flat = repo.root.path().join("trees/b");

    git::create_worktree(&repo.clone, &flat, "hotfix").unwrap();

    let access = git::write_access(&repo.clone, &flat, "hotfix").unwrap();

    assert!(access.writable.contains(&text(dot_git.join("refs/heads"))));
    assert!(access.protected.contains(&text(dot_git.join("refs/heads/main"))));
    assert!(access.protected.contains(&text(dot_git.join("refs/heads/feature"))));
    assert!(!access.protected.contains(&text(dot_git.join("refs/heads/hotfix"))));
}

#[test]
fn the_app_commits_for_a_thread_without_a_shell_and_runs_no_hook_doing_it() {
    let repo = repo("commit-tool");
    let tree = repo.root.path().join("trees/a");
    let made = git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();

    write(&repo.clone, ".git/hooks/pre-commit", "#!/bin/sh\necho ran > \"$(git rev-parse --show-toplevel)/hook-ran\"\nexit 1\n");
    run(&repo.clone, &["config", "user.name", "Ada"]);
    run(&repo.clone, &["config", "user.email", "ada@example.invalid"]);
    run(&repo.clone, &["config", "commit.gpgsign", "false"]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(repo.clone.join(".git/hooks/pre-commit"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    write(&tree, "src/login.ts", "export const limit = 10;\n");
    write(&tree, "notes.md", "left out of the first commit\n");

    let first = git::commit(&repo.clone, &tree, "WEB-1: raise the limit", &["src/login.ts".to_string()]).unwrap();

    assert!(first.ends_with("WEB-1: raise the limit"), "{first}");
    assert_eq!(run(&tree, &["status", "--porcelain"]), "?? notes.md");
    assert!(!tree.join("hook-ran").exists());

    git::commit(&repo.clone, &tree, "WEB-1: notes", &[]).unwrap();

    let (_, commits, uncommitted) = git::thread_changes(&repo.clone, &tree, &made.base_commit).unwrap();

    assert_eq!((commits.len(), uncommitted), (2, 0));
    assert!(git::commit(&repo.clone, &tree, "WEB-1: nothing", &[]).is_err());
    assert!(git::commit(&repo.clone, &tree, "x", &["../web/README.md".to_string()]).is_err());
    assert!(git::commit(&repo.clone, &tree, "x", &["--amend".to_string()]).is_err());
    assert!(git::commit(&repo.clone, &tree, "  ", &[]).is_err());
    assert_eq!(run(&repo.clone, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
}

#[test]
fn a_clone_is_inspected_without_being_changed() {
    let repo = repo("inspect");

    write(&repo.clone, "src/login.ts", "changed\n");
    git::create_worktree(&repo.clone, &repo.root.path().join("trees/a"), "feature/WEB-1").unwrap();

    let before = fingerprint(&repo.clone);
    let found = git::inspect(&repo.clone.join("src")).unwrap();

    assert_eq!(found.root, repo.clone);
    assert_eq!(found.default_branch.as_deref(), Some("origin/main"));
    assert_eq!(found.current_branch.as_deref(), Some("main"));
    assert_eq!(found.changed_files, 1);
    assert_eq!(found.worktrees.len(), 1);
    assert_eq!(found.worktrees[0].branch.as_deref(), Some("feature/WEB-1"));
    assert!(found.remote_url.as_deref().is_some_and(|url| url.ends_with("origin.git")));
    assert_eq!(fingerprint(&repo.clone), before);
    assert!(git::inspect(repo.root.path()).is_err());
}

#[test]
fn a_push_sends_exactly_the_commit_that_was_read_and_nothing_added_since() {
    let repo = repo("push-exact");
    let tree = repo.root.path().join("trees/a");
    let made = git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();

    write(&tree, "src/login.ts", "export const limit = 10;\n");
    run(&tree, &["commit", "-am", "WEB-1: raise the limit"]);

    let read = git::read_push(&repo.clone, "feature/WEB-1", None, &made.base_commit).unwrap();

    assert_eq!(read.blocked, None);
    assert_eq!(read.base, made.base_commit);
    assert_eq!(read.commits.len(), 1);
    assert_eq!(read.files.len(), 1);
    assert!(push_guard::review(&read.readings).is_empty());

    // The thread commits again after the person looked.
    write(&tree, "src/extra.ts", "export const sneaky = true;\n");
    run(&tree, &["add", "--all"]);
    run(&tree, &["commit", "-m", "WEB-1: one more thing"]);

    git::push(&repo.clone, "feature/WEB-1", &read.commit).unwrap();

    assert_eq!(run(&repo.origin, &["rev-parse", "refs/heads/feature/WEB-1"]), read.commit);
    assert_eq!(run(&repo.origin, &["rev-parse", "refs/heads/main"]), made.base_commit);
    assert!(git::is_pushed(&repo.clone, "feature/WEB-1", &read.commit));

    let second = git::read_push(&repo.clone, "feature/WEB-1", None, &made.base_commit).unwrap();

    assert_eq!(second.base, read.commit);
    assert_eq!(second.commits.iter().map(|commit| commit.subject.as_str()).collect::<Vec<_>>(), vec!["WEB-1: one more thing"]);
}

#[test]
fn a_push_is_blocked_when_there_is_nothing_to_send_or_history_was_rewritten_or_merged() {
    let repo = repo("push-blocked");
    let tree = repo.root.path().join("trees/a");
    let made = git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();
    let nothing = git::read_push(&repo.clone, "feature/WEB-1", None, &made.base_commit).unwrap();

    assert!(nothing.blocked.unwrap().contains("nothing to push"));

    write(&tree, "a.txt", "a\n");
    run(&tree, &["add", "--all"]);
    run(&tree, &["commit", "-m", "WEB-1: a"]);

    let first = git::read_push(&repo.clone, "feature/WEB-1", None, &made.base_commit).unwrap();

    git::push(&repo.clone, "feature/WEB-1", &first.commit).unwrap();
    run(&tree, &["commit", "--amend", "-m", "WEB-1: a, reworded"]);

    let rewritten = git::read_push(&repo.clone, "feature/WEB-1", None, &made.base_commit).unwrap();

    assert!(rewritten.blocked.unwrap().contains("never force-pushes"));
    // And git itself refuses the rewritten commit, because the push is never forced.
    assert!(git::push(&repo.clone, "feature/WEB-1", &rewritten.commit).is_err());
    assert_eq!(run(&repo.origin, &["rev-parse", "refs/heads/feature/WEB-1"]), first.commit);

    let merged = git::create_worktree(&repo.clone, &repo.root.path().join("trees/b"), "feature/WEB-2").unwrap();

    run(&merged.path, &["merge", "--no-ff", "-m", "Merge the other branch", &first.commit]);

    let with_merge = git::read_push(&repo.clone, "feature/WEB-2", None, &merged.base_commit).unwrap();

    assert!(with_merge.blocked.unwrap().contains("merge commit"));
    assert!(git::read_push(&repo.clone, "feature/WEB-2", Some("abc123"), &merged.base_commit).is_err());
    assert!(git::push(&repo.clone, "feature/WEB-2", "HEAD").is_err());
}

#[test]
fn ci_files_and_a_secret_removed_in_a_later_commit_are_found_in_what_would_be_pushed() {
    let repo = repo("push-findings");
    let tree = repo.root.path().join("trees/a");
    let made = git::create_worktree(&repo.clone, &tree, "feature/WEB-1").unwrap();

    write(&tree, ".github/workflows/ci.yml", "on: push\n");
    write(&tree, "src/config.ts", "export const apiKey = \"abcdef0123456789\";\n");
    run(&tree, &["add", "--all"]);
    run(&tree, &["commit", "-m", "WEB-1: pipeline and config"]);
    write(&tree, "src/config.ts", "export const apiKey = process.env.KEY;\n");
    run(&tree, &["commit", "-am", "WEB-1: read the key from the environment"]);

    let read = git::read_push(&repo.clone, "feature/WEB-1", None, &made.base_commit).unwrap();
    let findings = push_guard::review(&read.readings);
    let found: Vec<(&str, Option<&str>)> = findings.iter().map(|found| (found.kind.as_str(), found.path.as_deref())).collect();

    assert_eq!(read.readings.len(), 2);
    assert_eq!(found, vec![("ci", Some(".github/workflows/ci.yml")), ("secret", Some("src/config.ts"))]);
}
