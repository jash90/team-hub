//! Integration tokens at rest: encrypted with the instance key, never returned by any route.
//! What the interface may show is a hint, the last four characters.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rusqlite::Connection;

use crate::db::{now_iso, query_one};
use crate::error::{ApiError, ApiResult};

fn cipher(key: &[u8; 32]) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(Key::from_slice(key))
}

fn hint_of(value: &str) -> String {
    let tail: String = value.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();

    if value.chars().count() > 8 {
        format!("…{tail}")
    } else {
        "…".to_string()
    }
}

pub fn set_secret(connection: &Connection, key: &[u8; 32], name: &str, value: &str) -> ApiResult<()> {
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|error| ApiError::internal(format!("random: {error}")))?;
    let sealed = cipher(key)
        .encrypt(Nonce::from_slice(&nonce), value.as_bytes())
        .map_err(|_| ApiError::internal("could not encrypt the secret"))?;

    connection.execute(
        "INSERT INTO secret (name, nonce, sealed, hint, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (name) DO UPDATE SET nonce = ?2, sealed = ?3, hint = ?4, updated_at = ?5",
        (name, nonce.as_slice(), sealed, hint_of(value), now_iso()),
    )?;

    Ok(())
}

pub fn clear_secret(connection: &Connection, name: &str) -> ApiResult<()> {
    connection.execute("DELETE FROM secret WHERE name = ?1", [name])?;

    Ok(())
}

pub fn get_secret(connection: &Connection, key: &[u8; 32], name: &str) -> ApiResult<Option<String>> {
    let stored: Option<(Vec<u8>, Vec<u8>)> =
        query_one(connection, "SELECT nonce, sealed FROM secret WHERE name = ?1", [name], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let Some((nonce, sealed)) = stored else { return Ok(None) };
    let opened = cipher(key)
        .decrypt(Nonce::from_slice(&nonce), sealed.as_slice())
        .map_err(|_| ApiError::internal(format!("the stored secret \"{name}\" cannot be read with this instance key")))?;

    Ok(Some(String::from_utf8_lossy(&opened).into_owned()))
}

/// The hint of a stored secret, or none when nothing is stored.
pub fn secret_hint(connection: &Connection, name: &str) -> ApiResult<Option<String>> {
    query_one(connection, "SELECT hint FROM secret WHERE name = ?1", [name], |row| row.get(0))
}
