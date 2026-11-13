use std::path::PathBuf;

use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone)]
pub struct Config {
    /// Database, attachments and the generated secret key live here.
    pub data_dir: PathBuf,
    pub bind: String,
    pub port: u16,
    /// The address people reach the app at; used in links sent to other systems.
    pub base_url: String,
    /// The built web interface.
    pub web_dir: PathBuf,
    /// The page about team-hub that a visitor who is not signed in sees at `/`, with the
    /// way to the sign-in and to the desktop app. `None`: such a visitor gets the sign-in
    /// right away.
    pub landing_dir: Option<PathBuf>,
    /// Encrypts stored integration tokens.
    pub secret_key: [u8; 32],
    /// When set, the first account can only be created by someone who knows it. For an
    /// instance that is reachable by others before its owner has set it up.
    pub setup_token: Option<String>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn random_key() -> ApiResult<[u8; 32]> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|error| ApiError::internal(format!("random: {error}")))?;

    Ok(key)
}

fn parse_key(text: &str) -> ApiResult<[u8; 32]> {
    hex::decode(text.trim())
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| ApiError::internal("the secret key must be 64 hex characters"))
}

/// The key from `TEAM_HUB_SECRET_KEY`, or one generated on first start and kept in the data
/// directory, readable only by its owner.
fn load_secret_key(data_dir: &std::path::Path) -> ApiResult<[u8; 32]> {
    if let Some(text) = env("TEAM_HUB_SECRET_KEY") {
        return parse_key(&text);
    }

    let path = data_dir.join("secret.key");

    if let Ok(text) = std::fs::read_to_string(&path) {
        return parse_key(&text);
    }

    let key = random_key()?;
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(&path, hex::encode(key))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(key)
}

impl Config {
    pub fn from_env() -> ApiResult<Self> {
        let data_dir = PathBuf::from(env("TEAM_HUB_DATA_DIR").unwrap_or_else(|| "data".to_string()));
        // `PORT` is what hosting platforms hand a container.
        let port = env("TEAM_HUB_PORT").or_else(|| env("PORT")).and_then(|value| value.parse().ok()).unwrap_or(8080);
        let secret_key = load_secret_key(&data_dir)?;

        Ok(Self {
            bind: env("TEAM_HUB_BIND").unwrap_or_else(|| "127.0.0.1".to_string()),
            base_url: env("TEAM_HUB_BASE_URL").unwrap_or_else(|| format!("http://localhost:{port}")),
            web_dir: PathBuf::from(env("TEAM_HUB_WEB_DIR").unwrap_or_else(|| "web/dist".to_string())),
            landing_dir: match env("TEAM_HUB_LANDING").as_deref().map(str::trim) {
                Some("off") => None,
                _ => Some(PathBuf::from(env("TEAM_HUB_LANDING_DIR").unwrap_or_else(|| "landing".to_string()))),
            },
            data_dir,
            port,
            secret_key,
            setup_token: env("TEAM_HUB_SETUP_TOKEN").map(|token| token.trim().to_string()),
        })
    }

    /// A configuration for tests: its own directory, a fixed key.
    pub fn for_tests(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            bind: "127.0.0.1".to_string(),
            port: 0,
            base_url: "http://localhost".to_string(),
            web_dir: PathBuf::from("web/dist"),
            landing_dir: None,
            secret_key: [7; 32],
            setup_token: None,
        }
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("team-hub.db")
    }

    pub fn attachments_dir(&self) -> PathBuf {
        self.data_dir.join("attachments")
    }
}
