//! Talking to Confluence's REST API (Cloud and Server/Data Center share `/rest/api`).
//! Everything here blocks on the network: call it from `spawn_blocking`, never from the
//! worker pool.

use std::time::Duration;

use base64::Engine;
use hub_app::app::get_setting;
use hub_app::secrets::get_secret;
use hub_app::{ApiError, ApiResult};
use rusqlite::Connection;
use serde_json::Value;

pub const BASE_URL_SETTING: &str = "confluence:base_url";
pub const EMAIL_SETTING: &str = "confluence:email";
pub const TOKEN_SECRET: &str = "confluence.token";

const PAGE_SIZE: usize = 50;
const TRIES: u32 = 3;
const BACKOFF: Duration = Duration::from_millis(300);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);
const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ANSWER_BYTES: u64 = 64 * 1024 * 1024;
/// More result pages than any space has; ends the loop should a server page in circles.
const MAX_REQUESTS: usize = 4000;

pub const REJECTED: &str = "Confluence rejected the token";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub base_url: String,
    pub email: String,
    pub token: Option<String>,
}

pub fn load_settings(connection: &Connection, key: &[u8; 32]) -> ApiResult<Settings> {
    Ok(Settings {
        base_url: get_setting::<String>(connection, BASE_URL_SETTING)?.unwrap_or_default(),
        email: get_setting::<String>(connection, EMAIL_SETTING)?.unwrap_or_default(),
        token: get_secret(connection, key, TOKEN_SECRET)?,
    })
}

fn host_of(authority: &str) -> &str {
    match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    }
}

fn is_loopback(base_url: &str) -> bool {
    let authority = base_url.split_once("://").map_or("", |(_, rest)| rest).split('/').next().unwrap_or("");

    matches!(host_of(authority), "127.0.0.1" | "localhost")
}

/// The address without a trailing slash. Only `https://` is accepted, so the token never
/// crosses the network in the clear; plain `http://` is for this machine only.
pub fn clean_base_url(text: &str) -> ApiResult<String> {
    let url = text.trim().trim_end_matches('/');
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(ApiError::bad_request("The Confluence address must start with https://"));
    };
    let authority = rest.split('/').next().unwrap_or("");
    let malformed = authority.is_empty()
        || authority.contains('@')
        || url.contains(|character: char| character.is_whitespace() || matches!(character, '?' | '#'));

    if malformed {
        return Err(ApiError::bad_request(
            "That does not look like a Confluence address, e.g. https://example.atlassian.net/wiki",
        ));
    }

    match scheme.to_lowercase().as_str() {
        "https" => Ok(url.to_string()),
        "http" if is_loopback(url) => Ok(url.to_string()),
        _ => Err(ApiError::bad_request("The Confluence address must start with https://")),
    }
}

/// `Basic` with the e-mail for Confluence Cloud, `Bearer` for a personal access token.
pub fn authorization(email: &str, token: &str) -> String {
    if email.trim().is_empty() {
        format!("Bearer {token}")
    } else {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{}:{token}", email.trim()))
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteSpace {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteAncestor {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemotePage {
    pub id: String,
    pub title: String,
    pub version: i64,
    /// The body in storage format; none when Confluence did not send one.
    pub storage: Option<String>,
    /// From the top of the space down to the parent.
    pub ancestors: Vec<RemoteAncestor>,
    pub url: String,
}

/// Confluence sends ids as strings, older servers as numbers.
fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    authorization: String,
}

impl Client {
    pub fn new(settings: &Settings) -> ApiResult<Self> {
        let token = settings.token.as_deref().filter(|token| !token.is_empty());
        let (false, Some(token)) = (settings.base_url.is_empty(), token) else {
            return Err(ApiError::conflict("Set the Confluence address and token first"));
        };
        let base = clean_base_url(&settings.base_url)?;
        let mut config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .user_agent("team-hub");

        if is_loopback(&base) {
            config = config.proxy(None);
        }

        Ok(Self {
            agent: config.build().new_agent(),
            authorization: authorization(&settings.email, token),
            base,
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// One GET, answered with JSON. 429 and 5xx are tried again after a pause; a 401 is final.
    fn get(&self, url: &str, query: &[(&str, String)]) -> ApiResult<Value> {
        let mut pause = BACKOFF;
        let mut last_status = 0;

        for attempt in 0..TRIES {
            if attempt > 0 {
                std::thread::sleep(pause);
                pause = BACKOFF * 2u32.pow(attempt);
            }

            let mut request = self
                .agent
                .get(url)
                .header("Authorization", &self.authorization)
                .header("Accept", "application/json");

            for (name, value) in query {
                request = request.query(*name, value);
            }

            let mut response = request
                .call()
                .map_err(|error| ApiError::upstream(format!("Could not reach Confluence: {error}")))?;
            let status = response.status().as_u16();

            match status {
                200..=299 => {
                    return response
                        .body_mut()
                        .with_config()
                        .limit(MAX_ANSWER_BYTES)
                        .read_json::<Value>()
                        .map_err(|error| ApiError::upstream(format!("Confluence sent an answer that is not JSON ({error}); check the address")));
                }
                401 => return Err(ApiError::upstream(REJECTED)),
                403 => {
                    return Err(ApiError::upstream(
                        "Confluence refused access; the token's account may not be allowed to see this",
                    ))
                }
                404 => {
                    return Err(ApiError::upstream(
                        "Confluence has nothing at that address; check the address and the space key",
                    ))
                }
                429 | 500..=599 => {
                    last_status = status;

                    let asked = response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<u64>().ok());

                    if let Some(seconds) = asked {
                        pause = Duration::from_secs(seconds).min(MAX_RETRY_AFTER);
                    }
                }
                other => return Err(ApiError::upstream(format!("Confluence answered with status {other}"))),
            }
        }

        Err(ApiError::upstream(format!(
            "Confluence kept answering with status {last_status}; try again later"
        )))
    }

    /// The address of the next page of results, when the answer names one on this server.
    fn next_url(&self, links: &Value) -> Option<String> {
        let next = links["next"].as_str()?;
        let context = links["context"].as_str().unwrap_or("");
        let relative = if !context.is_empty() && self.base.ends_with(context) && next.starts_with(context) {
            &next[context.len()..]
        } else {
            next
        };

        relative.starts_with('/').then(|| format!("{}{relative}", self.base))
    }

    /// Walks a paged listing: follows `_links.next` where the server gives it, and otherwise
    /// asks for the next `start` until a page comes back short.
    fn paged(&self, path: &str, query: &[(&str, String)], mut each: impl FnMut(&Value)) -> ApiResult<()> {
        let first = format!("{}{path}", self.base);
        let mut start = 0;
        let mut next: Option<String> = None;

        for _ in 0..MAX_REQUESTS {
            let answer = match &next {
                Some(url) => self.get(url, &[])?,
                None => {
                    let mut query = query.to_vec();
                    query.push(("limit", PAGE_SIZE.to_string()));
                    query.push(("start", start.to_string()));

                    self.get(&first, &query)?
                }
            };
            let results = answer["results"].as_array().map(Vec::as_slice).unwrap_or_default();
            let size = answer["size"].as_u64().map_or(results.len(), |size| size as usize);
            let limit = answer["limit"].as_u64().map_or(PAGE_SIZE, |limit| limit as usize);

            results.iter().for_each(&mut each);
            start += size;

            let following = self.next_url(&answer["_links"]);

            if size == 0 || results.is_empty() || (following.is_none() && size < limit) || (following.is_some() && following == next) {
                return Ok(());
            }

            next = following;
        }

        Err(ApiError::upstream("Confluence keeps sending more pages of results than a space can have"))
    }

    /// The cheapest call that proves the address and the token work.
    pub fn check(&self) -> ApiResult<()> {
        self.get(&format!("{}/rest/api/space", self.base), &[("limit", "1".to_string())])
            .map(|_| ())
    }

    pub fn spaces(&self) -> ApiResult<Vec<RemoteSpace>> {
        let mut spaces = Vec::new();

        self.paged("/rest/api/space", &[], |space| {
            if let Some(key) = space["key"].as_str() {
                spaces.push(RemoteSpace {
                    key: key.to_string(),
                    name: space["name"].as_str().unwrap_or(key).to_string(),
                });
            }
        })?;

        Ok(spaces)
    }

    pub fn space_name(&self, key: &str) -> ApiResult<Option<String>> {
        let spaces = self.get(
            &format!("{}/rest/api/space", self.base),
            &[("spaceKey", key.to_string()), ("limit", "1".to_string())],
        )?;

        Ok(spaces["results"][0]["name"].as_str().map(String::from))
    }

    /// Every page of a space, with body, version and ancestors.
    pub fn pages(&self, space_key: &str) -> ApiResult<Vec<RemotePage>> {
        let mut pages = Vec::new();
        let query = [
            ("spaceKey", space_key.to_string()),
            ("type", "page".to_string()),
            ("expand", "body.storage,version,ancestors".to_string()),
        ];

        self.paged("/rest/api/content", &query, |page| {
            let Some(id) = text_of(&page["id"]) else { return };
            let ancestors = page["ancestors"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(|ancestor| {
                    Some(RemoteAncestor {
                        id: text_of(&ancestor["id"])?,
                        title: ancestor["title"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect();
            let url = match page["_links"]["webui"].as_str() {
                Some(path) if path.starts_with('/') => format!("{}{path}", self.base),
                _ => format!("{}/pages/viewpage.action?pageId={id}", self.base),
            };

            pages.push(RemotePage {
                title: page["title"].as_str().unwrap_or("").to_string(),
                version: page["version"]["number"].as_i64().unwrap_or(0),
                storage: page["body"]["storage"]["value"].as_str().map(String::from),
                ancestors,
                url,
                id,
            });
        })?;

        Ok(pages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_addresses_are_accepted_except_on_this_machine() {
        assert_eq!(
            clean_base_url(" https://example.atlassian.net/wiki/ ").unwrap(),
            "https://example.atlassian.net/wiki"
        );
        assert_eq!(
            clean_base_url("https://confluence.example.com:8443").unwrap(),
            "https://confluence.example.com:8443"
        );
        assert_eq!(
            clean_base_url("http://127.0.0.1:8090/confluence").unwrap(),
            "http://127.0.0.1:8090/confluence"
        );
        assert_eq!(clean_base_url("http://localhost").unwrap(), "http://localhost");

        for refused in [
            "http://confluence.example.com",
            "http://127.0.0.1.example.com",
            "http://localhost.example.com/wiki",
            "http://user@127.0.0.1",
            "ftp://example.com",
            "example.atlassian.net/wiki",
            "https://",
            "https://exa mple.com",
            "https://example.com/wiki?x=1",
            "https://user:secret@example.com",
            "",
        ] {
            assert!(clean_base_url(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_authorization_header_depends_on_the_e_mail() {
        assert_eq!(authorization("", "pat-123"), "Bearer pat-123");
        assert_eq!(
            authorization(" anna@example.com ", "api-token"),
            "Basic YW5uYUBleGFtcGxlLmNvbTphcGktdG9rZW4="
        );
    }
}
