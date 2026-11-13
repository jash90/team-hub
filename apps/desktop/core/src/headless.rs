//! The core without a person at it (docs/cloud-runner.md): configured from the environment,
//! with clones it makes itself, and with everything that leaves the machine decided on the
//! web. This file reads the configuration and says how the core is opened; what a headless
//! runner then does differently lives where the desktop app does the same thing by hand
//! (`app.rs`), so there is one runner and not two.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::app::OpenOptions;
use crate::config::{self, Paths};
use crate::jobs::JobUser;
use crate::model::Settings;
use crate::repos;
use crate::runs::{RunKind, RUNNER_KINDS};
use crate::secrets::{MemoryKeychain, Secret, TOKEN_VARIABLE};

pub const URL_VARIABLE: &str = "TEAM_HUB_URL";
pub const GITHUB_TOKEN_VARIABLE: &str = "GITHUB_TOKEN";
pub const MODEL_KEY_VARIABLE: &str = crate::policy::MODEL_KEY_VARIABLE;
pub const DEFAULT_NAME: &str = "Cloud runner";
pub const DEFAULT_DATA_DIR: &str = "/data";
pub const DEFAULT_JOB_USER: &str = "job";

/// The variables that hold a secret. The runner reads them once and then removes them from
/// its own environment, so no process it starts can find them there.
pub const SECRET_VARIABLES: [&str; 3] = [TOKEN_VARIABLE, GITHUB_TOKEN_VARIABLE, MODEL_KEY_VARIABLE];

/// Where a headless runner keeps things under its data folder, beside the core's own state
/// and working trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    /// One clone per repository: `repos/<owner>/<name>`.
    pub repos: PathBuf,
    /// One folder per test job, removed when the job ends.
    pub jobs: PathBuf,
    /// The browsers test suites install, kept between jobs.
    pub browsers: PathBuf,
    /// The home of the agent helper: its sessions, so a thread can be resumed.
    pub helper_home: PathBuf,
}

impl Folders {
    pub fn under(root: &Path) -> Self {
        Self { repos: root.join("repos"), jobs: root.join("jobs"), browsers: root.join("browsers"), helper_home: root.join("agent-home") }
    }

    /// Makes the folders. The clones and the helper's home are the runner's alone; the jobs
    /// folder can be passed through, so a job's own user reaches the folder it owns.
    pub fn create(&self) -> std::io::Result<()> {
        for folder in [&self.repos, &self.jobs, &self.browsers, &self.helper_home] {
            std::fs::create_dir_all(folder)?;
        }

        config::private_to_owner(&self.repos);
        config::private_to_owner(&self.helper_home);
        config::passable(&self.jobs);

        Ok(())
    }
}

/// How the core is opened for a runner nobody sits at.
#[derive(Debug, Clone)]
pub struct Headless {
    pub server_url: String,
    pub name: String,
    /// The run kinds it takes; empty when it has no model key and only runs test jobs.
    pub kinds: Vec<RunKind>,
    /// None: every project the person can edit.
    pub projects: Option<Vec<String>>,
    pub max_running: u32,
    pub allow_shell: bool,
    pub model_key: Option<Secret>,
    pub github_token: Option<Secret>,
    /// The team's runner: it holds no keys of its own and is given each person's with their
    /// work (docs/runners.md § Shared runners).
    pub shared: bool,
    /// Where `owner/name` is cloned from.
    pub git_base: String,
    pub folders: Folders,
}

impl Headless {
    /// The saved settings with what the environment says laid over them. The runner's id is
    /// the one thing that is kept: the server knows the runner by it across restarts.
    pub fn settings(&self, saved: Settings) -> Settings {
        Settings {
            server_url: Some(self.server_url.clone()),
            runner_id: saved.runner_id.strip_prefix("desktop-").map_or(saved.runner_id.clone(), |id| format!("runner-{id}")),
            runner_name: self.name.clone(),
            runner_enabled: (self.shared || self.model_key.is_some()) && !self.kinds.is_empty(),
            max_running: self.max_running,
            allow_unsandboxed_shell: self.allow_shell,
            ..saved
        }
    }
}

/// The runner's configuration as the environment gives it.
#[derive(Debug, Clone)]
pub struct RunnerEnv {
    pub url: Option<String>,
    pub token: Option<Secret>,
    pub github_token: Option<Secret>,
    pub model_key: Option<Secret>,
    pub name: String,
    pub kinds: Vec<RunKind>,
    pub projects: Option<Vec<String>>,
    pub max_running: u32,
    pub test_jobs: bool,
    pub allow_shell: bool,
    /// The team's runner, working with each person's own keys; else its owner's, with its own.
    pub shared: bool,
    pub data_dir: PathBuf,
    pub git_base: String,
    pub job_user: String,
    /// Values that could not be used as written, each with what is used instead.
    pub remarks: Vec<String>,
}

fn list(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|part| !part.is_empty()).map(String::from).collect()
}

fn switched_on(text: &str) -> bool {
    matches!(text.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

impl RunnerEnv {
    pub fn from_process() -> Self {
        Self::read(|name| std::env::var(name).ok())
    }

    /// Reads the variables of docs/cloud-runner.md. Nothing here fails: a value that cannot
    /// be used is replaced by its default and remarked on, and what is missing is told by
    /// `missing`, so a half-configured runner can say so instead of dying.
    pub fn read(variable: impl Fn(&str) -> Option<String>) -> Self {
        let text = |name: &str| variable(name).map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
        let secret = |name: &str| text(name).map(Secret::new);
        let mut remarks = Vec::new();

        let url = text(URL_VARIABLE).and_then(|url| match config::normalise_server_url(&url) {
            Ok(url) => Some(url),
            Err(error) => {
                remarks.push(format!("{URL_VARIABLE} is not usable ({error})"));
                None
            }
        });
        let kinds = match text("RUNNER_KINDS") {
            Some(wanted) => {
                let named = list(&wanted);
                let (known, unknown): (Vec<_>, Vec<_>) = named.iter().partition(|name| RUNNER_KINDS.iter().any(|kind| kind.as_str() == name.as_str()));

                if !unknown.is_empty() {
                    remarks.push(format!(
                        "RUNNER_KINDS names kinds a runner does not carry out ({}); it takes implement, review and custom",
                        unknown.iter().map(|name| name.as_str()).collect::<Vec<_>>().join(", ")
                    ));
                }

                RUNNER_KINDS.iter().copied().filter(|kind| known.iter().any(|name| name.as_str() == kind.as_str())).collect()
            }
            None => RUNNER_KINDS.to_vec(),
        };
        let projects = text("RUNNER_PROJECTS").map(|keys| list(&keys.to_uppercase())).map(|keys| {
            let (valid, invalid): (Vec<String>, Vec<String>) = keys.into_iter().partition(|key| hub_core::keys::is_project_key(key));

            if !invalid.is_empty() {
                remarks.push(format!("RUNNER_PROJECTS names what is not a project key ({}); those are left out", invalid.join(", ")));
            }

            valid
        });
        let max_running = match text("RUNNER_MAX_RUNNING").map(|value| value.parse::<u32>()) {
            Some(Ok(count)) if (1..=16).contains(&count) => count,
            Some(_) => {
                remarks.push("RUNNER_MAX_RUNNING is a number between 1 and 16; 2 is used".to_string());
                2
            }
            None => 2,
        };

        let (github_token, model_key) = (secret(GITHUB_TOKEN_VARIABLE), secret(MODEL_KEY_VARIABLE));
        // Shared unless it was given keys of its own, or told otherwise.
        let shared = text("RUNNER_SHARED").map_or(github_token.is_none() && model_key.is_none(), |value| switched_on(&value));

        if shared && (github_token.is_some() || model_key.is_some()) {
            remarks.push(format!(
                "RUNNER_SHARED=1: {GITHUB_TOKEN_VARIABLE} and {MODEL_KEY_VARIABLE} are not used; a shared runner works with each person's own keys"
            ));
        }

        Self {
            url,
            token: secret(TOKEN_VARIABLE),
            github_token: github_token.filter(|_| !shared),
            model_key: model_key.filter(|_| !shared),
            shared,
            name: text("RUNNER_NAME").unwrap_or_else(|| DEFAULT_NAME.to_string()),
            kinds,
            projects,
            max_running,
            test_jobs: text("RUNNER_TEST_JOBS").is_none_or(|value| value != "0"),
            allow_shell: text("RUNNER_ALLOW_SHELL").is_some_and(|value| switched_on(&value)),
            data_dir: PathBuf::from(text("RUNNER_DATA_DIR").unwrap_or_else(|| DEFAULT_DATA_DIR.to_string())),
            git_base: text("RUNNER_GIT_BASE").unwrap_or_else(|| repos::GITHUB.to_string()),
            job_user: text("RUNNER_JOB_USER").unwrap_or_else(|| DEFAULT_JOB_USER.to_string()),
            remarks,
        }
    }

    /// The variables without which the runner can do nothing at all.
    pub fn missing(&self) -> Vec<&'static str> {
        [(URL_VARIABLE, self.url.is_none()), (TOKEN_VARIABLE, self.token.is_none())].into_iter().filter(|(_, missing)| *missing).map(|(name, _)| name).collect()
    }

    /// What is switched off by what is not set, in words for the log.
    pub fn limits(&self) -> Vec<String> {
        let mut limits = Vec::new();

        if self.shared {
            limits.push(format!(
                "A shared runner: it works with the keys each person stores in team-hub (Settings → My keys), and {TOKEN_VARIABLE} has to be an administrator's"
            ));
        } else if self.model_key.is_none() {
            limits.push(format!("{MODEL_KEY_VARIABLE} is not set: this runner takes test jobs only, no agent runs"));
        }

        if !self.shared && self.github_token.is_none() {
            limits.push(format!("{GITHUB_TOKEN_VARIABLE} is not set: only public repositories can be cloned, and nothing can be pushed"));
        }

        if !self.test_jobs {
            limits.push("RUNNER_TEST_JOBS=0: this runner takes no test jobs".to_string());
        }

        limits
    }

    pub fn paths(&self) -> Paths {
        Paths::under(&self.data_dir)
    }

    pub fn folders(&self) -> Folders {
        Folders::under(&self.data_dir)
    }

    /// How to open the core, once the address and the token are there.
    pub fn open_options(&self) -> Option<OpenOptions> {
        let (url, token) = (self.url.clone()?, self.token.clone()?);
        let mut options = OpenOptions::new(self.paths(), Arc::new(MemoryKeychain::with_token(token.expose())));

        // A redeploy gives the process ten seconds: a helper that does not end when asked is
        // killed well inside them.
        options.stop_grace = Duration::from_secs(3);
        options.headless = Some(Headless {
            server_url: url,
            name: self.name.clone(),
            kinds: if self.shared || self.model_key.is_some() { self.kinds.clone() } else { Vec::new() },
            projects: self.projects.clone(),
            max_running: self.max_running,
            allow_shell: self.allow_shell,
            model_key: self.model_key.clone(),
            github_token: self.github_token.clone(),
            shared: self.shared,
            git_base: self.git_base.clone(),
            folders: self.folders(),
        });

        Some(options)
    }

    /// Who a test job's processes run as: see `JobUser::for_runner`.
    pub fn job_user(&self) -> JobUser {
        JobUser::for_runner(&self.job_user)
    }
}

/// Takes the secrets out of this process's environment once they have been read: out of
/// what later code and child processes inherit, and, on Linux, out of the block the kernel
/// shows as `/proc/<pid>/environ`, which a process of the same user could otherwise read.
/// The process is also closed to being inspected by its children. Call it at the very start,
/// before any other thread runs: changing the environment is not safe beside one.
pub fn forget_secrets_in_environment() {
    #[cfg(target_os = "linux")]
    // SAFETY: `environ` is the process's own null-terminated list of NUL-terminated strings,
    // nothing else runs yet, and each write stays inside the string it was measured from.
    unsafe {
        extern "C" {
            static environ: *const *mut libc::c_char;
        }

        let mut entry = environ;

        while !entry.is_null() && !(*entry).is_null() {
            let bytes = std::ffi::CStr::from_ptr(*entry).to_bytes();
            let name_length = bytes.iter().position(|byte| *byte == b'=').unwrap_or(bytes.len());
            let is_secret = SECRET_VARIABLES.iter().any(|name| name.as_bytes() == &bytes[..name_length]);

            if is_secret && name_length < bytes.len() {
                std::ptr::write_bytes((*entry).add(name_length + 1), 0, bytes.len() - name_length - 1);
            }

            entry = entry.add(1);
        }

        libc::prctl(libc::PR_SET_DUMPABLE, 0);
    }

    for name in SECRET_VARIABLES {
        // SAFETY: as above, no other thread exists yet.
        unsafe { std::env::remove_var(name) };
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> RunnerEnv {
        let map: HashMap<String, String> = pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();

        RunnerEnv::read(|name| map.get(name).cloned())
    }

    #[test]
    fn with_nothing_set_the_defaults_hold_and_the_two_needed_variables_are_named() {
        let empty = env(&[]);

        assert_eq!(empty.missing(), vec!["TEAM_HUB_URL", "TEAM_HUB_TOKEN"]);
        assert!(empty.open_options().is_none());
        assert_eq!((empty.name.as_str(), empty.max_running, empty.test_jobs, empty.allow_shell), ("Cloud runner", 2, true, false));
        assert_eq!(empty.kinds, RUNNER_KINDS.to_vec());
        assert_eq!((empty.projects.clone(), empty.data_dir.clone()), (None, PathBuf::from("/data")));
        assert_eq!((empty.git_base.as_str(), empty.job_user.as_str(), empty.shared), ("https://github.com", "job", true));
        assert_eq!(env(&[("TEAM_HUB_URL", "https://hub.example.com/")]).missing(), vec!["TEAM_HUB_TOKEN"]);
    }

    #[test]
    fn every_variable_of_the_table_is_read() {
        let full = env(&[
            ("TEAM_HUB_URL", "http://team-hub.railway.internal:8080/"),
            ("TEAM_HUB_TOKEN", " th_abc "),
            ("GITHUB_TOKEN", "ghp_x"),
            ("ANTHROPIC_API_KEY", "sk-ant-x"),
            ("RUNNER_NAME", "Railway"),
            ("RUNNER_KINDS", "review, implement"),
            ("RUNNER_PROJECTS", "web, API"),
            ("RUNNER_MAX_RUNNING", "4"),
            ("RUNNER_TEST_JOBS", "0"),
            ("RUNNER_ALLOW_SHELL", "1"),
            ("RUNNER_DATA_DIR", "/srv/runner"),
        ]);

        assert!(full.missing().is_empty() && full.remarks.is_empty());
        assert_eq!(full.url.as_deref(), Some("http://team-hub.railway.internal:8080"));
        assert_eq!(full.token.as_ref().map(Secret::expose), Some("th_abc"));
        assert_eq!(full.kinds, vec![RunKind::Implement, RunKind::Review]);
        assert_eq!(full.projects, Some(vec!["WEB".to_string(), "API".to_string()]));
        assert_eq!((full.max_running, full.test_jobs, full.allow_shell), (4, false, true));
        assert_eq!(full.folders().repos, PathBuf::from("/srv/runner/repos"));

        let headless = full.open_options().unwrap().headless.unwrap();

        assert_eq!((headless.name.as_str(), headless.max_running, headless.allow_shell), ("Railway", 4, true));
        assert!(!format!("{full:?} {headless:?}").contains("th_abc") && !format!("{full:?} {headless:?}").contains("sk-ant-x"));
    }

    #[test]
    fn a_value_that_cannot_be_used_is_remarked_on_and_replaced() {
        let odd = env(&[
            ("TEAM_HUB_URL", "hub.example.com"),
            ("RUNNER_KINDS", "implement,refine"),
            ("RUNNER_PROJECTS", "WEB,not a key"),
            ("RUNNER_MAX_RUNNING", "many"),
            ("RUNNER_ALLOW_SHELL", "0"),
        ]);

        assert_eq!(odd.remarks.len(), 4, "{:?}", odd.remarks);
        assert_eq!((odd.url.clone(), odd.kinds.clone(), odd.max_running, odd.allow_shell), (None, vec![RunKind::Implement], 2, false));
        assert_eq!(odd.projects, Some(vec!["WEB".to_string()]));
    }

    #[test]
    fn a_runner_is_shared_unless_it_is_given_keys_of_its_own_or_told_otherwise() {
        let base = [("TEAM_HUB_URL", "https://hub.example.com"), ("TEAM_HUB_TOKEN", "th_abc")];
        let with = |more: &[(&'static str, &'static str)]| env(&[&base[..], more].concat());

        // Nothing of its own: shared, and it takes agent runs without a key in its environment.
        let shared = with(&[]);
        let headless = shared.open_options().unwrap().headless.unwrap();

        assert!(shared.shared && headless.shared && headless.model_key.is_none());
        assert_eq!(headless.kinds, RUNNER_KINDS.to_vec());
        assert!(headless.settings(config::default_settings()).runner_enabled);

        // Either key makes it its owner's; RUNNER_SHARED says it outright, both ways.
        assert!(!with(&[("GITHUB_TOKEN", "ghp_x")]).shared && !with(&[("ANTHROPIC_API_KEY", "sk-ant-x")]).shared);
        assert!(!with(&[("RUNNER_SHARED", "0")]).shared);

        let told = with(&[("RUNNER_SHARED", "1"), ("GITHUB_TOKEN", "ghp_x"), ("ANTHROPIC_API_KEY", "sk-ant-x")]);

        assert!(told.shared && told.github_token.is_none() && told.model_key.is_none());
        assert_eq!(told.remarks.len(), 1);
    }

    #[test]
    fn without_a_model_key_the_runner_takes_no_agent_runs() {
        let jobs_only = env(&[("TEAM_HUB_URL", "https://hub.example.com"), ("TEAM_HUB_TOKEN", "th_abc"), ("RUNNER_SHARED", "0")]);
        let headless = jobs_only.open_options().unwrap().headless.unwrap();

        assert!(headless.kinds.is_empty());
        assert!(!headless.settings(config::default_settings()).runner_enabled);
        assert!(headless.settings(config::default_settings()).runner_id.starts_with("runner-"));
        assert_eq!(jobs_only.limits().len(), 2);
    }
}
