//! Connecting a desktop app to a person's account without a token passing through anybody's
//! hands (`docs/desktop-pairing.md`).
//!
//! The app asks first, signed in as nobody: it gets a secret, which it keeps, and a short
//! code, which it shows. The person, signed in in a browser, is shown the same code and says
//! yes. Then the app, and only the app, collects an API token of that person with its secret,
//! once. Nothing here is a token until that last step: a pairing nobody collects leaves
//! nothing behind.

use axum::http::StatusCode;
use chrono::{Duration, SecondsFormat, Utc};
use hub_core::{DesktopPairing, Id};
use rusqlite::Connection;

use crate::app::App;
use crate::auth::{self, AuthUser};
use crate::db::{now_iso, query_one};
use crate::error::{ApiError, ApiResult};
use crate::services::users;

/// How long the person has to say yes, and the app to collect.
const MINUTES: i64 = 10;
/// Anybody can ask, so only this many can be waiting at once.
pub const MAX_OPEN: i64 = 20;
const CODE_LENGTH: usize = 8;
/// Letters and digits that are not mistaken for one another when read off a screen.
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const MAX_NAME: usize = 80;

/// What the app gets when it asks. The secret is in the clear here and nowhere else.
pub struct Started {
    pub id: String,
    pub code: String,
    pub secret: String,
    pub expires_at: String,
}

/// What the app gets when it comes to collect.
pub enum Claim {
    /// Nobody has said yes yet.
    Pending,
    Granted { token: String, token_id: Id },
}

fn gone() -> ApiError {
    ApiError::new(StatusCode::GONE, "gone", "This request to connect has expired or was declined; start again in the app")
}

/// A code as people type it (`abcd-efgh`) in the form it is stored in.
fn clean_code(code: &str) -> String {
    code.chars().filter(|character| character.is_ascii_alphanumeric()).map(|character| character.to_ascii_uppercase()).collect()
}

fn clean_name(name: &str) -> ApiResult<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");

    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request("The app needs a name of 1 to 80 characters"));
    }

    Ok(name)
}

fn random_code() -> ApiResult<String> {
    let mut bytes = [0u8; CODE_LENGTH];
    getrandom::fill(&mut bytes).map_err(|error| ApiError::internal(format!("random: {error}")))?;

    // 256 is a multiple of the alphabet's 32 letters, so every letter is as likely as another.
    Ok(bytes.iter().map(|byte| ALPHABET[usize::from(*byte) % ALPHABET.len()] as char).collect())
}

/// The app asks to be connected. It is nobody yet, so this changes nothing but the list of
/// those waiting.
pub fn start(connection: &Connection, name: &str, runner_id: &str) -> ApiResult<Started> {
    let name = clean_name(name)?;
    let runner_id: String = runner_id.trim().chars().filter(|character| !character.is_control()).take(MAX_NAME).collect();
    let now = now_iso();

    connection.execute("DELETE FROM desktop_pairing WHERE expires_at < ?1", [&now])?;

    let open: i64 = connection.query_row("SELECT count(*) FROM desktop_pairing", [], |row| row.get(0))?;

    if open >= MAX_OPEN {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_pairings",
            "Too many apps are waiting to be connected; try again in a few minutes",
        ));
    }

    let started = Started {
        id: auth::random_token()?[..32].to_string(),
        code: random_code()?,
        secret: auth::random_token()?,
        expires_at: (Utc::now() + Duration::minutes(MINUTES)).to_rfc3339_opts(SecondsFormat::Millis, true),
    };

    connection.execute(
        "INSERT INTO desktop_pairing (id, code, secret_hash, name, runner_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (&started.id, &started.code, auth::digest(&started.secret), name, runner_id, now, &started.expires_at),
    )?;

    Ok(started)
}

/// Saying yes or no to an app is done by the person, where they signed in themselves: a
/// token that leaked must not be able to make itself more tokens.
fn in_person(user: &AuthUser) -> ApiResult<()> {
    if user.via_token {
        return Err(ApiError::forbidden("An app is connected in the browser, signed in, not with an API token"));
    }

    Ok(())
}

struct Waiting {
    id: String,
    pairing: DesktopPairing,
    user_id: Option<Id>,
}

fn by_code(connection: &Connection, code: &str) -> ApiResult<Waiting> {
    query_one(
        connection,
        "SELECT id, code, name, created_at, expires_at, user_id FROM desktop_pairing WHERE code = ?1 AND expires_at > ?2",
        (clean_code(code), now_iso()),
        |row| {
            let user_id: Option<Id> = row.get(5)?;

            Ok(Waiting {
                id: row.get(0)?,
                pairing: DesktopPairing { code: row.get(1)?, name: row.get(2)?, created_at: row.get(3)?, expires_at: row.get(4)?, approved: user_id.is_some() },
                user_id,
            })
        },
    )?
    .ok_or_else(gone)
}

/// The app that waits under this code, for the person who is asked about it.
pub fn show(connection: &Connection, user: &AuthUser, code: &str) -> ApiResult<DesktopPairing> {
    in_person(user)?;

    Ok(by_code(connection, code)?.pairing)
}

/// The person says yes: the app that holds the secret may collect a token of theirs.
pub fn approve(connection: &Connection, user: &AuthUser, code: &str) -> ApiResult<DesktopPairing> {
    in_person(user)?;

    let mut waiting = by_code(connection, code)?;

    if waiting.user_id.is_some_and(|approver| approver != user.id) {
        return Err(ApiError::conflict("Somebody else has connected this app already"));
    }

    connection.execute("UPDATE desktop_pairing SET user_id = ?1 WHERE id = ?2", (user.id, &waiting.id))?;
    waiting.pairing.approved = true;

    Ok(waiting.pairing)
}

/// The person says no. The app is told the request is gone, as when it runs out of time.
pub fn decline(connection: &Connection, user: &AuthUser, code: &str) -> ApiResult<()> {
    in_person(user)?;

    let waiting = by_code(connection, code)?;

    if waiting.user_id.is_some_and(|approver| approver != user.id) {
        return Err(ApiError::conflict("Somebody else has connected this app already"));
    }

    connection.execute("DELETE FROM desktop_pairing WHERE id = ?1", [&waiting.id])?;

    Ok(())
}

/// The app comes to collect. A token is made only now, handed over once, and the pairing is
/// gone with it.
pub fn claim(app: &App, id: &str, secret: &str) -> ApiResult<Claim> {
    let found: Option<(String, String, String, Option<Id>)> = app.read(|connection| {
        query_one(connection, "SELECT secret_hash, name, expires_at, user_id FROM desktop_pairing WHERE id = ?1", [id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
    })?;
    let Some((secret_hash, name, expires_at, user_id)) = found else { return Err(gone()) };

    // Compared by digest, so that the time it takes says nothing about the secret.
    if auth::digest(secret) != secret_hash {
        return Err(ApiError::forbidden("This is not the app that asked to be connected"));
    }

    if expires_at <= now_iso() {
        app.db.write(|tx| Ok(tx.execute("DELETE FROM desktop_pairing WHERE id = ?1", [id])?))?;

        return Err(gone());
    }

    let Some(user_id) = user_id else { return Ok(Claim::Pending) };

    app.db.write(|tx| {
        // Whoever removes the row has the token: two requests at once get one between them.
        if tx.execute("DELETE FROM desktop_pairing WHERE id = ?1 AND user_id = ?2", (id, user_id))? == 0 {
            return Err(gone());
        }

        let active: Option<bool> = query_one(tx, "SELECT active FROM user WHERE id = ?1", [user_id], |row| row.get(0))?;

        if active != Some(true) {
            return Err(gone());
        }

        let token = users::create_token(tx, user_id, &name)?;

        Ok(Claim::Granted { token, token_id: tx.last_insert_rowid() })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_read_as_people_type_it() {
        assert_eq!(clean_code(" abcd-2345 "), "ABCD2345");
        assert_eq!(clean_code("ABCD2345"), "ABCD2345");
    }

    #[test]
    fn a_code_has_eight_characters_nobody_confuses() {
        let code = random_code().unwrap();

        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|byte| ALPHABET.contains(&byte)), "{code}");
        assert!(!ALPHABET.iter().any(|letter| b"IO01".contains(letter)));
    }

    #[test]
    fn an_app_needs_a_name_that_fits_on_a_line() {
        assert_eq!(clean_name("  Desktop ·   laptop ").unwrap(), "Desktop · laptop");
        assert!(clean_name("  ").is_err());
        assert!(clean_name(&"x".repeat(81)).is_err());
    }
}
