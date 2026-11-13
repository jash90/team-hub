//! Where the app keeps its files, and the settings a person can change.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, CoreResult};
use crate::model::Settings;

pub const DATA_DIR_VARIABLE: &str = "TEAM_HUB_DESKTOP_DIR";
/// Replaces the helper process, e.g. with the fake agent: the program, then its arguments.
pub const SIDECAR_VARIABLE: &str = "TEAM_HUB_SIDECAR";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const MODELS: [&str; 4] = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-fable-5-2"];

const SETTINGS_FILE: &str = "settings.json";
const DATABASE_FILE: &str = "desktop.db";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The database and the settings: closed to agent threads.
    pub state: PathBuf,
    /// One folder per repository, one working tree per thread.
    pub worktrees: PathBuf,
}

impl Paths {
    pub fn under(root: &Path) -> Self {
        Self { state: root.join("state"), worktrees: root.join("worktrees") }
    }

    /// `TEAM_HUB_DESKTOP_DIR`, or the per-user data folder of the operating system.
    pub fn from_env() -> CoreResult<Self> {
        if let Some(root) = std::env::var_os(DATA_DIR_VARIABLE).filter(|value| !value.is_empty()) {
            return Ok(Self::under(Path::new(&root)));
        }

        let base = dirs::data_dir().ok_or_else(|| CoreError::not_possible("this system has no per-user data folder; set TEAM_HUB_DESKTOP_DIR"))?;

        Ok(Self::under(&base.join("team-hub-desktop")))
    }

    pub fn database(&self) -> PathBuf {
        self.state.join(DATABASE_FILE)
    }

    fn settings(&self) -> PathBuf {
        self.state.join(SETTINGS_FILE)
    }

    pub fn create(&self) -> CoreResult<()> {
        std::fs::create_dir_all(&self.state)?;
        std::fs::create_dir_all(&self.worktrees)?;
        private_to_owner(&self.state);

        Ok(())
    }
}

#[cfg(unix)]
fn private_to_owner(folder: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let _ = std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn private_to_owner(_folder: &Path) {}

fn random_id() -> String {
    let mut bytes = [0u8; 8];

    if getrandom::fill(&mut bytes).is_err() {
        bytes = std::process::id().to_le_bytes().repeat(2).try_into().unwrap_or([7; 8]);
    }

    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn machine_name() -> String {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this computer".to_string())
}

pub fn default_settings() -> Settings {
    Settings {
        server_url: None,
        runner_id: format!("desktop-{}", random_id()),
        runner_name: format!("Desktop · {}", machine_name()),
        runner_enabled: true,
        max_running: 2,
        max_turns: 80,
        default_model: DEFAULT_MODEL.to_string(),
        node_command: "node".to_string(),
        allow_unsandboxed_shell: false,
    }
}

/// The saved settings; on first use the defaults are written, so the runner id stays the same.
pub fn load_settings(paths: &Paths) -> CoreResult<Settings> {
    let file = paths.settings();

    if let Ok(text) = std::fs::read_to_string(&file) {
        if let Ok(settings) = serde_json::from_str::<Settings>(&text) {
            return Ok(settings);
        }
    }

    let settings = default_settings();

    save_settings(paths, &settings)?;

    Ok(settings)
}

pub fn save_settings(paths: &Paths, settings: &Settings) -> CoreResult<()> {
    std::fs::create_dir_all(&paths.state)?;
    std::fs::write(paths.settings(), serde_json::to_vec_pretty(settings)?)?;

    Ok(())
}

/// `https://hub.example.com/` → `https://hub.example.com`. Only http(s), and no credentials in it.
pub fn normalise_server_url(text: &str) -> CoreResult<String> {
    let url = text.trim().trim_end_matches('/');
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    let host = rest.map(|rest| rest.split('/').next().unwrap_or_default());

    match host {
        Some(host) if !host.is_empty() && !host.contains('@') && !host.contains(char::is_whitespace) => Ok(url.to_string()),
        _ => Err(CoreError::not_possible("The server address looks like https://hub.example.com")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runner_id_is_made_once_and_kept() {
        let root = std::env::temp_dir().join(format!("hub-desktop-config-{}", random_id()));
        let paths = Paths::under(&root);
        let first = load_settings(&paths).unwrap();
        let second = load_settings(&paths).unwrap();

        assert!(first.runner_id.starts_with("desktop-"));
        assert_eq!(first.runner_id, second.runner_id);
        assert_eq!(first.max_running, 2);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_server_address_is_http_and_carries_no_password() {
        assert_eq!(normalise_server_url(" https://hub.example.com/ ").unwrap(), "https://hub.example.com");
        assert_eq!(normalise_server_url("http://127.0.0.1:8195").unwrap(), "http://127.0.0.1:8195");
        assert!(normalise_server_url("hub.example.com").is_err());
        assert!(normalise_server_url("https://me:secret@hub.example.com").is_err());
        assert!(normalise_server_url("file:///etc/passwd").is_err());
    }
}
