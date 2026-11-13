//! Keys people store for themselves (`docs/runners.md` → Shared runners): their own model
//! key, their own GitHub token. Modules say which exist; the values are kept here, encrypted
//! with the instance key like every other secret, and no route gives one back.

use hub_core::runners::{PersonalKey, PersonalKeyState};
use hub_core::Id;
use rusqlite::Connection;

use crate::app::App;
use crate::auth::AuthUser;
use crate::db::query_one;
use crate::error::{ApiError, ApiResult};
use crate::secrets::{clear_secret, get_secret, set_secret};

const MAX_LENGTH: usize = 2000;

/// The keys the switched-on modules name, each once, in the modules' order.
pub fn known(app: &App) -> Vec<PersonalKey> {
    let mut keys: Vec<PersonalKey> = Vec::new();

    for key in app.enabled_modules().flat_map(|module| module.personal_keys()) {
        let valid = !key.name.is_empty() && key.name.chars().all(|character| character.is_ascii_lowercase() || character == '_');

        if valid && !keys.iter().any(|known| known.name == key.name) {
            keys.push(key);
        }
    }

    keys
}

/// Where a person's key is kept among the instance's secrets.
fn stored_as(user_id: Id, name: &str) -> String {
    format!("user:{user_id}:{name}")
}

fn spec(app: &App, name: &str) -> ApiResult<PersonalKey> {
    known(app).into_iter().find(|key| key.name == name).ok_or_else(|| ApiError::not_found("Key"))
}

/// The caller's keys: which are set, never what they hold.
pub fn list(app: &App, connection: &Connection, user: &AuthUser) -> ApiResult<Vec<PersonalKeyState>> {
    known(app)
        .into_iter()
        .map(|key| {
            let stored: Option<(String, String)> =
                query_one(connection, "SELECT hint, updated_at FROM secret WHERE name = ?1", [stored_as(user.id, &key.name)], |row| Ok((row.get(0)?, row.get(1)?)))?;
            let (tail, updated_at) = stored.unzip();

            Ok(PersonalKeyState { name: key.name, label: key.label, hint: key.hint, set: tail.is_some(), tail, updated_at })
        })
        .collect()
}

/// A person changes their keys where they signed in themselves: a token that leaked should
/// not be enough to swap the key their work is billed to.
fn in_person(user: &AuthUser) -> ApiResult<()> {
    if user.via_token {
        return Err(ApiError::forbidden("Your keys are changed in the browser, signed in, not with an API token"));
    }

    Ok(())
}

pub fn set(app: &App, connection: &Connection, user: &AuthUser, name: &str, value: &str) -> ApiResult<()> {
    in_person(user)?;
    let key = spec(app, name)?;
    let value = value.trim();

    if value.is_empty() || value.len() > MAX_LENGTH || value.chars().any(|character| character.is_control() || character.is_whitespace()) {
        return Err(ApiError::bad_request("A key is one line without spaces"));
    }

    set_secret(connection, &app.config.secret_key, &stored_as(user.id, &key.name), value)
}

pub fn clear(app: &App, connection: &Connection, user: &AuthUser, name: &str) -> ApiResult<()> {
    in_person(user)?;
    let key = spec(app, name)?;

    clear_secret(connection, &stored_as(user.id, &key.name))
}

/// Whether that person has stored the key. For modules: to say early that work cannot start.
pub fn has(connection: &Connection, user_id: Id, name: &str) -> ApiResult<bool> {
    Ok(query_one(connection, "SELECT 1 FROM secret WHERE name = ?1", [stored_as(user_id, name)], |_| Ok(()))?.is_some())
}

/// A condition that holds when the person in `user_column` has stored the key, for a
/// module's own statement. `name` is a constant of the module's, not anything a caller sent.
pub fn stored_condition(user_column: &str, name: &str) -> String {
    format!("EXISTS (SELECT 1 FROM secret WHERE secret.name = 'user:' || {user_column} || ':{name}')")
}

/// The value itself. For modules, and only to hand to a shared runner that has claimed that
/// person's work; it must not be logged or stored anywhere else.
pub fn reveal(app: &App, connection: &Connection, user_id: Id, name: &str) -> ApiResult<Option<String>> {
    get_secret(connection, &app.config.secret_key, &stored_as(user_id, name))
}
