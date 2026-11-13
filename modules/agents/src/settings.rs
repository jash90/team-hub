//! How the server carries out runs itself: the model provider's key, which model, which
//! kinds of run, and how much it may spend in a month.

use chrono::{DateTime, Datelike, Utc};
use hub_app::app::{get_setting, set_setting};
use hub_app::secrets::{clear_secret, get_secret, secret_hint, set_secret};
use hub_app::{ApiError, ApiResult, App};
use hub_core::double_option;
use rusqlite::Connection;
use serde::Deserialize;

use crate::runs::SERVER_RUNNER;
use crate::types::{AgentModel, AgentRunKind, AgentSettings};

const KEY_SECRET: &str = "agents.api_key";
const URL_SETTING: &str = "agents:api_url";
const MODEL_SETTING: &str = "agents:default_model";
const KINDS_SETTING: &str = "agents:server_kinds";
const CAP_SETTING: &str = "agents:monthly_cap_usd";
pub const DEFAULT_API_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";

/// A model the server may be told to use, with its price in US dollars per million tokens.
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub input_price: f64,
    pub output_price: f64,
    /// A prompt longer than this many tokens is billed at the second pair of prices.
    pub long_prompt: Option<(i64, f64, f64)>,
    /// Whether a request the model declines may be retried by the provider on another model.
    pub fallbacks: bool,
}

pub const MODELS: [ModelInfo; 4] = [
    ModelInfo { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", input_price: 2.0, output_price: 10.0, long_prompt: None, fallbacks: true },
    ModelInfo { id: "claude-opus-5-5", name: "Claude Opus 5.5", input_price: 4.0, output_price: 20.0, long_prompt: None, fallbacks: true },
    ModelInfo {
        id: "claude-haiku-5-5",
        name: "Claude Haiku 5.5",
        input_price: 0.1,
        output_price: 0.5,
        long_prompt: Some((100_000, 0.5, 2.5)),
        fallbacks: false,
    },
    ModelInfo { id: "claude-fable-5-2", name: "Claude Fable 5.2", input_price: 8.0, output_price: 40.0, long_prompt: None, fallbacks: true },
];

pub fn model(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|model| model.id == id)
}

/// What one request cost, from the tokens the provider says it used.
pub fn cost(model: &ModelInfo, input_tokens: i64, output_tokens: i64) -> f64 {
    let (input_price, output_price) = match model.long_prompt {
        Some((limit, input_price, output_price)) if input_tokens > limit => (input_price, output_price),
        _ => (model.input_price, model.output_price),
    };

    (input_tokens as f64 * input_price + output_tokens as f64 * output_price) / 1_000_000.0
}

/// The key is sent to this address, so it must be HTTPS (or this machine, for tests).
fn clean_api_url(url: &str) -> ApiResult<String> {
    let url = url.trim().trim_end_matches('/');
    let local = ["http://127.0.0.1", "http://localhost"].iter().any(|prefix| url == *prefix || url.starts_with(&format!("{prefix}:")));

    if url.starts_with("https://") || local {
        Ok(url.to_string())
    } else {
        Err(ApiError::bad_request("The API address must start with https://"))
    }
}

fn server_kinds(connection: &Connection) -> ApiResult<Vec<AgentRunKind>> {
    let stored: Option<Vec<String>> = get_setting(connection, KINDS_SETTING)?;

    Ok(match stored {
        Some(kinds) => kinds.iter().filter_map(|kind| AgentRunKind::parse(kind)).filter(|kind| kind.server_can_run()).collect(),
        None => AgentRunKind::ALL.iter().copied().filter(|kind| kind.server_can_run()).collect(),
    })
}

/// What the server's own runs have cost since the first of this month.
fn spent_this_month(connection: &Connection, now: DateTime<Utc>) -> ApiResult<f64> {
    let month = format!("{:04}-{:02}-01", now.year(), now.month());

    Ok(connection.query_row(
        "SELECT coalesce(sum(cost_usd), 0) FROM agent_run WHERE runner_id = ?1 AND coalesce(started_at, created_at) >= ?2",
        (SERVER_RUNNER, month),
        |row| row.get(0),
    )?)
}

pub fn load(connection: &Connection, admin: bool, now: DateTime<Utc>) -> ApiResult<AgentSettings> {
    let key_hint = secret_hint(connection, KEY_SECRET)?;
    let monthly_cap_usd: Option<f64> = get_setting(connection, CAP_SETTING)?;
    let spent_this_month_usd = spent_this_month(connection, now)?;
    let server_status = match (&key_hint, monthly_cap_usd) {
        (None, _) => "no_key",
        (Some(_), Some(cap)) if spent_this_month_usd >= cap => "cap_reached",
        _ => "ready",
    };

    Ok(AgentSettings {
        key_configured: key_hint.is_some(),
        key_hint: key_hint.filter(|_| admin),
        api_url: get_setting(connection, URL_SETTING)?.unwrap_or_else(|| DEFAULT_API_URL.to_string()),
        default_model: get_setting::<String>(connection, MODEL_SETTING)?
            .filter(|id| model(id).is_some())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        models: MODELS
            .iter()
            .map(|model| AgentModel {
                id: model.id.to_string(),
                name: model.name.to_string(),
                input_price: model.input_price,
                output_price: model.output_price,
            })
            .collect(),
        server_kinds: server_kinds(connection)?,
        monthly_cap_usd,
        spent_this_month_usd: (spent_this_month_usd * 10_000.0).round() / 10_000.0,
        server_status: server_status.to_string(),
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsInput {
    /// Blank or absent keeps the stored key.
    pub api_key: Option<String>,
    pub clear_api_key: bool,
    pub api_url: Option<String>,
    pub default_model: Option<String>,
    pub server_kinds: Option<Vec<AgentRunKind>>,
    /// Null removes the cap.
    #[serde(deserialize_with = "double_option")]
    pub monthly_cap_usd: Option<Option<f64>>,
}

pub fn save(app: &App, connection: &Connection, input: &SettingsInput) -> ApiResult<()> {
    if input.clear_api_key {
        clear_secret(connection, KEY_SECRET)?;
    }

    if let Some(key) = input.api_key.as_deref().map(str::trim).filter(|key| !key.is_empty()) {
        set_secret(connection, &app.config.secret_key, KEY_SECRET, key)?;
    }

    if let Some(url) = input.api_url.as_deref().map(str::trim) {
        let url = if url.is_empty() { DEFAULT_API_URL.to_string() } else { clean_api_url(url)? };
        set_setting(connection, URL_SETTING, &url)?;
    }

    if let Some(id) = &input.default_model {
        let known = model(id.trim()).ok_or_else(|| ApiError::bad_request("That is not one of the models the server can use"))?;
        set_setting(connection, MODEL_SETTING, &known.id)?;
    }

    if let Some(kinds) = &input.server_kinds {
        let kinds: Vec<&str> = kinds.iter().filter(|kind| kind.server_can_run()).map(|kind| kind.as_str()).collect();
        set_setting(connection, KINDS_SETTING, &kinds)?;
    }

    match input.monthly_cap_usd {
        Some(Some(cap)) if !(0.0..=1_000_000.0).contains(&cap) => return Err(ApiError::bad_request("The monthly cap is an amount in US dollars")),
        Some(Some(cap)) => set_setting(connection, CAP_SETTING, &cap)?,
        Some(None) => {
            connection.execute("DELETE FROM setting WHERE key = ?1", [CAP_SETTING])?;
        }
        None => {}
    }

    Ok(())
}

/// What the server needs to carry out a run. None while it should not take any: no key, or
/// this month's cap is spent.
pub struct ServerSetup {
    pub api_key: String,
    pub api_url: String,
    pub default_model: &'static ModelInfo,
    pub kinds: Vec<AgentRunKind>,
}

pub fn server_setup(app: &App, now: DateTime<Utc>) -> ApiResult<Option<ServerSetup>> {
    app.read(|connection| {
        let settings = load(connection, true, now)?;
        let Some(api_key) = get_secret(connection, &app.config.secret_key, KEY_SECRET)? else { return Ok(None) };

        if settings.server_status != "ready" {
            return Ok(None);
        }

        Ok(Some(ServerSetup {
            api_key,
            api_url: settings.api_url,
            default_model: model(&settings.default_model).unwrap_or(&MODELS[0]),
            kinds: settings.server_kinds,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_follows_the_price_list_and_a_long_prompt_costs_more_on_the_small_model() {
        let sonnet = model("claude-sonnet-5-5").expect("the default model");
        let haiku = model("claude-haiku-5-5").expect("the small model");

        assert_eq!(cost(sonnet, 1_000_000, 100_000), 3.0);
        assert_eq!(cost(haiku, 100_000, 10_000), 0.015);
        assert_eq!(cost(haiku, 200_000, 10_000), 0.125);
        assert!(model("gpt-4").is_none());
    }

    #[test]
    fn the_key_only_ever_goes_to_https_or_to_this_machine() {
        assert_eq!(clean_api_url(" https://api.anthropic.com/ ").ok().as_deref(), Some("https://api.anthropic.com"));
        assert_eq!(clean_api_url("http://127.0.0.1:4010").ok().as_deref(), Some("http://127.0.0.1:4010"));
        assert!(clean_api_url("http://example.com").is_err());
        assert!(clean_api_url("http://localhost.evil.example").is_err());
    }
}
