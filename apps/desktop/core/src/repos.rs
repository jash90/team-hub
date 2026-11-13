//! The clones a runner keeps by itself (docs/cloud-runner.md § Repositories): one per
//! repository under its data folder, cloned the first time a run or a job names it and
//! fetched before each use. The token goes to git for one command at a time (`git::RemoteAuth`)
//! and is in no config, no remote address and no child process's environment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::{CoreError, CoreResult};
use crate::git::{self, RemoteAuth};
use crate::secrets::Secret;

pub const GITHUB: &str = "https://github.com";

pub struct Repos {
    root: PathBuf,
    /// Where `owner/name` is cloned from: GitHub, or what stands in for it in a test.
    base: String,
    /// The runner's own token; none for a shared runner, which is given one with each piece of work.
    auth: Option<RemoteAuth>,
    /// A shared runner: a failure to read is the requester's token's, and is said so.
    shared: bool,
    /// One at a time per clone: git does not like two fetches into the same repository.
    busy: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

fn is_name_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 100
        && !part.starts_with(['.', '-'])
        && !part.ends_with(".git")
        && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// `Acme/Web` → (`acme`, `web`). Anything that is not exactly an owner and a name is refused,
/// because the two become folders and a part of an address.
pub fn parse_name(name: &str) -> CoreResult<(String, String)> {
    let lower = name.trim().to_lowercase();

    match lower.split('/').collect::<Vec<_>>()[..] {
        [owner, repo] if is_name_part(owner) && is_name_part(repo) => Ok((owner.to_string(), repo.to_string())),
        _ => Err(CoreError::not_possible(format!("\"{}\" is not a repository name: it looks like owner/name", name.trim()))),
    }
}

impl Repos {
    pub fn new(root: PathBuf, base: &str, token: Option<Secret>, shared: bool) -> Self {
        let base = base.trim().trim_end_matches('/').to_string();

        Self { auth: token.map(|token| RemoteAuth::new(&base, token)), shared, root, base, busy: Mutex::new(HashMap::new()) }
    }

    /// The token for one command: the one that came with the work, else the runner's own.
    fn auth_of(&self, token: Option<&Secret>) -> Option<RemoteAuth> {
        token.map(|token| RemoteAuth::new(&self.base, token.clone())).or_else(|| self.auth.clone())
    }

    /// On a shared runner a repository that cannot be read is the requester's to fix: the
    /// reason names where, and the clone another person's token made is not used instead.
    fn unreadable(&self, name: &str, error: CoreError) -> CoreError {
        match (self.shared, error) {
            (true, CoreError::Git { command, reason }) => CoreError::not_possible(format!(
                "Your GitHub token cannot read {name} (git {command}: {reason}). Set one that can in team-hub under Settings → My keys, and start this again"
            )),
            (_, error) => error,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn url_of(&self, owner: &str, repo: &str) -> String {
        format!("{}/{owner}/{repo}.git", self.base)
    }

    fn alone<T>(&self, clone: &Path, work: impl FnOnce() -> CoreResult<T>) -> CoreResult<T> {
        let turn = Arc::clone(self.busy.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).entry(clone.to_path_buf()).or_default());
        let _held = turn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        work()
    }

    /// The clone of `owner/name`, up to date with its origin: cloned when it is not there yet,
    /// fetched when it is.
    pub fn ensure(&self, name: &str, token: Option<&Secret>) -> CoreResult<PathBuf> {
        self.ensure_with(name, self.auth_of(token).as_ref()).map_err(|error| self.unreadable(name, error))
    }

    fn ensure_with(&self, name: &str, auth: Option<&RemoteAuth>) -> CoreResult<PathBuf> {
        let (owner, repo) = parse_name(name)?;
        let url = self.url_of(&owner, &repo);
        let folder = self.root.join(&owner);
        let path = folder.join(&repo);

        self.alone(&path, || {
            if path.join(".git").exists() {
                let origin = git::remote_url(&path)?;

                if origin != url {
                    return Err(CoreError::not_possible(format!(
                        "{} is a clone of {origin}, not of {url}. Remove that folder so the runner can clone it again",
                        path.display()
                    )));
                }

                git::fetch(&path, auth)?;

                return Ok(path.clone());
            }

            std::fs::create_dir_all(&folder)?;

            // Cloned beside its place and moved there when whole, so a clone cut short is
            // never taken for a repository.
            let unfinished = folder.join(format!(".{repo}.cloning-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&unfinished);
            let _ = std::fs::remove_dir_all(&path);
            let cloned = git::clone(&url, &unfinished, auth).and_then(|()| Ok(std::fs::rename(&unfinished, &path)?));

            if cloned.is_err() {
                let _ = std::fs::remove_dir_all(&unfinished);
            }

            cloned.map(|()| path.clone())
        })
    }

    /// Fetches a clone this runner made. A clone it did not make gets no token.
    pub fn fetch(&self, clone: &Path, token: Option<&Secret>) -> CoreResult<()> {
        let auth = self.auth_for(clone, token);

        self.alone(clone, || git::fetch(clone, auth.as_ref()))
    }

    /// Pushes exactly `commit` to `branch` on origin, never forced.
    pub fn push(&self, clone: &Path, branch: &str, commit: &str, token: Option<&Secret>) -> CoreResult<()> {
        let auth = self.auth_for(clone, token);

        self.alone(clone, || git::push_as(clone, branch, commit, auth.as_ref()))
    }

    fn auth_for(&self, clone: &Path, token: Option<&Secret>) -> Option<RemoteAuth> {
        self.auth_of(token).filter(|_| clone.starts_with(&self.root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_name_is_an_owner_and_a_name_and_nothing_else() {
        assert_eq!(parse_name(" Acme/Web ").unwrap(), ("acme".to_string(), "web".to_string()));
        assert_eq!(parse_name("e2e-inc/e2e.js").unwrap().1, "e2e.js");

        for bad in ["", "web", "a/b/c", "../web", "acme/..", "acme/.git", "acme/web.git", "-x/web", "acme/we b", "acme/", "/web", "acme/web;rm"] {
            assert!(parse_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_address_holds_no_token() {
        let repos = Repos::new(PathBuf::from("/data/repos"), "https://github.com/", Some(Secret::new("ghp_secret")), false);

        assert_eq!(repos.url_of("acme", "web"), "https://github.com/acme/web.git");
        assert!(!format!("{:?}", repos.auth).contains("ghp_secret"));
    }
}
