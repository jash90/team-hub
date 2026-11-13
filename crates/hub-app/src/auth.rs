//! Who is calling: a browser session (cookie plus CSRF token) or an API token (Bearer).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use hub_core::{Id, Role};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::app::{Actor, App};
use crate::db::{now_iso, query_one};
use crate::error::{ApiError, ApiResult};

pub const SESSION_COOKIE: &str = "th_session";
pub const CSRF_HEADER: &str = "x-csrf-token";
pub const TOKEN_PREFIX: &str = "th_";
const SESSION_DAYS: i64 = 30;
const LOGIN_ATTEMPTS: u32 = 10;
const LOGIN_WINDOW: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq)]
pub struct AuthUser {
    pub id: Id,
    pub name: String,
    pub email: String,
    pub role: Role,
    /// Signed in with an API token rather than a browser session.
    pub via_token: bool,
    /// The modules this request passes through. A project that has switched one of them off
    /// does not exist for the request: see `access`.
    pub through: Vec<&'static str>,
}

/// Marks a request as being inside a module's routes; the host sets it, and the user taken
/// from the request carries it.
#[derive(Debug, Clone, Copy)]
pub struct ModuleScope(pub &'static str);

impl AuthUser {
    /// The same person acting through a module: for the host when it hands the user to a
    /// module's hook, and for a module that authenticates its callers itself.
    pub fn through(&self, module_id: &'static str) -> AuthUser {
        let mut user = self.clone();

        if !user.through.contains(&module_id) {
            user.through.push(module_id);
        }

        user
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    pub fn actor(&self) -> Actor {
        Actor::user(self.id)
    }

    pub fn require_admin(&self) -> ApiResult<()> {
        if self.is_admin() {
            Ok(())
        } else {
            Err(ApiError::forbidden("Only an administrator can do that"))
        }
    }
}

/// The caller if there is one, with the CSRF token of their session.
pub struct MaybeUser {
    pub user: Option<AuthUser>,
    pub csrf: Option<String>,
}

pub fn random_token() -> ApiResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| ApiError::internal(format!("random: {error}")))?;

    Ok(hex::encode(bytes))
}

pub fn digest(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

pub fn hash_password(password: &str) -> ApiResult<String> {
    if password.chars().count() < 8 {
        return Err(ApiError::bad_request("The password needs at least 8 characters"));
    }

    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|error| ApiError::internal(format!("random: {error}")))?;
    let salt = SaltString::encode_b64(&salt).map_err(|error| ApiError::internal(format!("salt: {error}")))?;

    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| ApiError::internal(format!("password hash: {error}")))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

static ATTEMPTS: LazyLock<Mutex<HashMap<String, (u32, Instant)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Counts a sign-in attempt for `email`; an error once there have been too many in a short time.
pub fn count_login_attempt(email: &str) -> ApiResult<()> {
    let mut attempts = ATTEMPTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = attempts.entry(email.to_lowercase()).or_insert((0, Instant::now()));

    if entry.1.elapsed() > LOGIN_WINDOW {
        *entry = (0, Instant::now());
    }

    entry.0 += 1;

    if entry.0 > LOGIN_ATTEMPTS {
        return Err(ApiError::new(axum::http::StatusCode::TOO_MANY_REQUESTS, "too_many_attempts", "Too many attempts; try again in a few minutes"));
    }

    Ok(())
}

pub fn forget_login_attempts(email: &str) {
    if let Ok(mut attempts) = ATTEMPTS.lock() {
        attempts.remove(&email.to_lowercase());
    }
}

pub struct Session {
    pub token: String,
    pub csrf: String,
}

pub fn create_session(connection: &Connection, user_id: Id) -> ApiResult<Session> {
    let session = Session { token: random_token()?, csrf: random_token()? };
    let expires = (Utc::now() + ChronoDuration::days(SESSION_DAYS)).to_rfc3339_opts(SecondsFormat::Millis, true);

    connection.execute("DELETE FROM session WHERE expires_at < ?1", [now_iso()])?;
    connection.execute(
        "INSERT INTO session (id, user_id, csrf, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        (digest(&session.token), user_id, &session.csrf, now_iso(), expires),
    )?;

    Ok(session)
}

pub fn destroy_session(connection: &Connection, token: &str) -> ApiResult<()> {
    connection.execute("DELETE FROM session WHERE id = ?1", [digest(token)])?;

    Ok(())
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };

    format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{secure}", SESSION_DAYS * 24 * 3600)
}

pub fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;

    value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")).map(|token| token.trim().to_string())
}

fn user_from_row(row: &rusqlite::Row, via_token: bool) -> rusqlite::Result<AuthUser> {
    let role: String = row.get(3)?;

    Ok(AuthUser { id: row.get(0)?, name: row.get(1)?, email: row.get(2)?, role: Role::parse(&role).unwrap_or(Role::Member), via_token, through: Vec::new() })
}

/// The user an API token belongs to. Also used by modules that accept tokens on their own routes.
pub fn user_for_token(app: &App, token: &str) -> ApiResult<Option<AuthUser>> {
    let hash = digest(token);
    let found = app.read(|connection| {
        query_one(
            connection,
            "SELECT u.id, u.name, u.email, u.role, t.id, t.last_used_at FROM api_token t JOIN user u ON u.id = t.user_id
             WHERE t.token_hash = ?1 AND u.active = 1",
            [&hash],
            |row| Ok((user_from_row(row, true)?, row.get::<_, Id>(4)?, row.get::<_, Option<String>>(5)?)),
        )
    })?;
    let Some((user, token_id, last_used)) = found else { return Ok(None) };
    let today = now_iso()[..10].to_string();

    if last_used.is_none_or(|at| !at.starts_with(&today)) {
        app.db.write(|tx| Ok(tx.execute("UPDATE api_token SET last_used_at = ?1 WHERE id = ?2", (now_iso(), token_id))?))?;
    }

    Ok(Some(user))
}

fn authenticate(app: &App, parts: &Parts) -> ApiResult<MaybeUser> {
    if let Some(token) = bearer_token(parts) {
        let user = user_for_token(app, &token)?.ok_or_else(ApiError::unauthorized)?;

        return Ok(MaybeUser { user: Some(user), csrf: None });
    }

    let Some(token) = cookie_value(&parts.headers, SESSION_COOKIE) else { return Ok(MaybeUser { user: None, csrf: None }) };
    let found = app.read(|connection| {
        query_one(
            connection,
            "SELECT u.id, u.name, u.email, u.role, s.csrf FROM session s JOIN user u ON u.id = s.user_id
             WHERE s.id = ?1 AND s.expires_at > ?2 AND u.active = 1",
            (digest(&token), now_iso()),
            |row| Ok((user_from_row(row, false)?, row.get::<_, String>(4)?)),
        )
    })?;
    let Some((user, csrf)) = found else { return Ok(MaybeUser { user: None, csrf: None }) };
    let safe = matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS);

    if !safe {
        let sent = parts.headers.get(CSRF_HEADER).and_then(|value| value.to_str().ok());

        if sent != Some(csrf.as_str()) {
            return Err(ApiError::forbidden("Missing or stale CSRF token; reload the page"));
        }
    }

    Ok(MaybeUser { user: Some(user), csrf: Some(csrf) })
}

impl FromRequestParts<App> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, Self::Rejection> {
        authenticate(app, parts)
    }
}

impl FromRequestParts<App> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, Self::Rejection> {
        let user = authenticate(app, parts)?.user.ok_or_else(ApiError::unauthorized)?;

        Ok(match parts.extensions.get::<ModuleScope>() {
            Some(ModuleScope(module_id)) => user.through(module_id),
            None => user,
        })
    }
}
