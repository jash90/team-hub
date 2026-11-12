use hub_core::text::{fold, handle_of};
use hub_core::{ApiToken, Id, Role, User};
use rusqlite::Connection;
use serde::Deserialize;

use crate::app::{App, Tx};
use crate::auth::{self, AuthUser, Session};
use crate::db::{now_iso, query_all, query_one};
use crate::error::{ApiError, ApiResult};

const COLUMNS: &str = "id, name, email, role, active, color, github_login";
const COLORS: [&str; 10] = ["#2563eb", "#db2777", "#059669", "#d97706", "#7c3aed", "#0891b2", "#dc2626", "#4d7c0f", "#c026d3", "#475569"];

fn map_user(row: &rusqlite::Row) -> rusqlite::Result<User> {
    let role: String = row.get(3)?;

    Ok(User {
        id: row.get(0)?,
        name: row.get(1)?,
        email: row.get(2)?,
        role: Role::parse(&role).unwrap_or(Role::Member),
        active: row.get(4)?,
        color: row.get(5)?,
        github_login: row.get(6)?,
    })
}

pub fn list_users(connection: &Connection) -> ApiResult<Vec<User>> {
    query_all(connection, &format!("SELECT {COLUMNS} FROM user ORDER BY name COLLATE NOCASE"), [], map_user)
}

pub fn get_user(connection: &Connection, id: Id) -> ApiResult<User> {
    query_one(connection, &format!("SELECT {COLUMNS} FROM user WHERE id = ?1"), [id], map_user)?.ok_or_else(|| ApiError::not_found("User"))
}

pub fn user_name(connection: &Connection, id: Option<Id>) -> ApiResult<Option<String>> {
    match id {
        Some(id) => query_one(connection, "SELECT name FROM user WHERE id = ?1", [id], |row| row.get(0)),
        None => Ok(None),
    }
}

pub fn setup_required(connection: &Connection) -> ApiResult<bool> {
    let count: i64 = connection.query_row("SELECT count(*) FROM user", [], |row| row.get(0))?;

    Ok(count == 0)
}

fn clean_email(email: &str) -> ApiResult<String> {
    let email = email.trim().to_lowercase();
    let valid = email.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.') && !email.contains(char::is_whitespace));

    if valid {
        Ok(email)
    } else {
        Err(ApiError::bad_request("That does not look like an e-mail address"))
    }
}

fn clean_name(name: &str) -> ApiResult<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");

    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::bad_request("A name of 1 to 80 characters is required"));
    }

    Ok(name)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewUser {
    pub name: String,
    pub email: String,
    pub password: Option<String>,
    pub role: Option<Role>,
    pub github_login: Option<String>,
}

pub fn create_user(connection: &Connection, new: &NewUser) -> ApiResult<User> {
    let name = clean_name(&new.name)?;
    let email = clean_email(&new.email)?;
    let hash = new.password.as_deref().map(auth::hash_password).transpose()?;
    let taken: Option<Id> = query_one(connection, "SELECT id FROM user WHERE email = ?1", [&email], |row| row.get(0))?;

    if taken.is_some() {
        return Err(ApiError::conflict("Somebody already uses that e-mail address"));
    }

    let count: i64 = connection.query_row("SELECT count(*) FROM user", [], |row| row.get(0))?;
    let color = COLORS[count as usize % COLORS.len()];
    let github_login = new.github_login.as_deref().map(str::trim).filter(|login| !login.is_empty());

    connection.execute(
        "INSERT INTO user (email, name, password_hash, role, color, github_login, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (&email, &name, hash, new.role.unwrap_or(Role::Member).as_str(), color, github_login, now_iso()),
    )?;

    get_user(connection, connection.last_insert_rowid())
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserPatch {
    pub name: Option<String>,
    pub email: Option<String>,
    pub role: Option<Role>,
    pub active: Option<bool>,
    pub password: Option<String>,
    pub github_login: Option<String>,
}

/// Changes a user. People may change their own name, e-mail, password and GitHub login; role
/// and activity are for administrators, who cannot remove the last active administrator.
pub fn update_user(tx: &Tx, caller: &AuthUser, id: Id, patch: &UserPatch) -> ApiResult<User> {
    let current = get_user(tx, id)?;
    let own = caller.id == id;

    if !own && !caller.is_admin() {
        return Err(ApiError::forbidden("You can only change your own account"));
    }

    if (patch.role.is_some() || patch.active.is_some()) && !caller.is_admin() {
        return Err(ApiError::forbidden("Only an administrator can change roles"));
    }

    let demotes = patch.role.is_some_and(|role| role != Role::Admin) || patch.active == Some(false);

    if demotes && current.role == Role::Admin && current.active {
        let admins: i64 = tx.query_row("SELECT count(*) FROM user WHERE role = 'admin' AND active = 1", [], |row| row.get(0))?;

        if admins <= 1 {
            return Err(ApiError::conflict("This is the last administrator"));
        }
    }

    if let Some(name) = &patch.name {
        tx.execute("UPDATE user SET name = ?1 WHERE id = ?2", (clean_name(name)?, id))?;
    }

    if let Some(email) = &patch.email {
        tx.execute("UPDATE user SET email = ?1 WHERE id = ?2", (clean_email(email)?, id))?;
    }

    if let Some(role) = patch.role {
        tx.execute("UPDATE user SET role = ?1 WHERE id = ?2", (role.as_str(), id))?;
    }

    if let Some(active) = patch.active {
        tx.execute("UPDATE user SET active = ?1 WHERE id = ?2", (active, id))?;

        if !active {
            tx.execute("DELETE FROM session WHERE user_id = ?1", [id])?;
        }
    }

    if let Some(password) = &patch.password {
        tx.execute("UPDATE user SET password_hash = ?1 WHERE id = ?2", (auth::hash_password(password)?, id))?;
    }

    if let Some(login) = &patch.github_login {
        let login = Some(login.trim()).filter(|login| !login.is_empty());
        tx.execute("UPDATE user SET github_login = ?1 WHERE id = ?2", (login, id))?;
    }

    get_user(tx, id)
}

pub fn login(app: &App, email: &str, password: &str) -> ApiResult<(User, Session)> {
    auth::count_login_attempt(email)?;

    let found: Option<(Id, Option<String>)> = app.read(|connection| {
        query_one(connection, "SELECT id, password_hash FROM user WHERE email = ?1 AND active = 1", [email.trim()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
    })?;
    let verified = found.and_then(|(id, hash)| hash.filter(|hash| auth::verify_password(password, hash)).map(|_| id));
    let Some(id) = verified else { return Err(ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "bad_credentials", "Wrong e-mail or password")) };

    auth::forget_login_attempts(email);

    app.db.write(|tx| Ok((get_user(tx, id)?, auth::create_session(tx, id)?)))
}

/// The user a piece of text names: an e-mail, a full name, or a mention handle (`anna.nowak`).
pub fn resolve_user(connection: &Connection, text: &str) -> ApiResult<Option<Id>> {
    let wanted = fold(text.trim().trim_start_matches('@'));
    let users = query_all(connection, "SELECT id, name, email FROM user", [], |row| {
        Ok((row.get::<_, Id>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;

    Ok(users
        .into_iter()
        .find(|(_, name, email)| fold(email) == wanted || fold(name) == wanted || handle_of(name) == wanted || fold(email.split('@').next().unwrap_or("")) == wanted)
        .map(|(id, _, _)| id))
}

pub fn list_tokens(connection: &Connection, user_id: Id) -> ApiResult<Vec<ApiToken>> {
    query_all(connection, "SELECT id, name, hint, created_at, last_used_at FROM api_token WHERE user_id = ?1 ORDER BY id DESC", [user_id], |row| {
        Ok(ApiToken { id: row.get(0)?, name: row.get(1)?, hint: row.get(2)?, created_at: row.get(3)?, last_used_at: row.get(4)? })
    })
}

/// Creates an API token and returns it in the clear, once; only its hash is stored.
pub fn create_token(connection: &Connection, user_id: Id, name: &str) -> ApiResult<String> {
    let name = name.trim();

    if name.is_empty() {
        return Err(ApiError::bad_request("Give the token a name"));
    }

    let token = format!("{}{}", auth::TOKEN_PREFIX, auth::random_token()?);
    let hint = format!("…{}", &token[token.len() - 4..]);

    connection.execute(
        "INSERT INTO api_token (user_id, name, token_hash, hint, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        (user_id, name, auth::digest(&token), hint, now_iso()),
    )?;

    Ok(token)
}

pub fn delete_token(connection: &Connection, user_id: Id, token_id: Id) -> ApiResult<()> {
    let deleted = connection.execute("DELETE FROM api_token WHERE id = ?1 AND user_id = ?2", (token_id, user_id))?;

    if deleted == 0 {
        return Err(ApiError::not_found("Token"));
    }

    Ok(())
}
