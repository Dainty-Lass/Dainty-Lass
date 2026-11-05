//! One admin key per person, each with a role (DD23 layer 4, DH2). `REFORGED_ADMIN_API_KEY` in `secrets.env`
//! is the owner's key and stays there, so a server set up before this works unchanged: the single key is
//! the owner. Every other key is `RRA_<id>_<64 hex>`: the id is public (the log names it, `admin key list`
//! shows it) and the rest is 256 random bits. Only the SHA-256 of the whole key is stored, in the admin
//! API's own database; a key is shown once, when it is issued.
//!
//! Every write goes through `Database::write` under the actor that made it, so the change log names who
//! issued or revoked a key (`admin:<key id>`, or `host:...` for the host kit's command), and each
//! moderation action leaves a row in `admin_actions` under the key that made it.

use std::sync::Arc;

use async_graphql::Enum;
use diesel::prelude::*;
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use rand::RngExt;
use store::{Actor, AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema};

use crate::hmac::{same_secret, sha256_hex};
use crate::modules::OWNER_KEY;
use crate::upstream::Operator;

const SQLITE: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("migrations/postgres");

pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    // A revoked key stays as a row with `revoked_at`: the record of who had access
    soft_delete: &[],
};

/// The SQLite file's name beside the node's database.
pub const FILE_NAME: &str = "admin.db";

pub const KEY_PREFIX: &str = "RRA_";
const ID_HEX: usize = 8;
const SECRET_HEX: usize = 64;
/// The most characters of a key's name (who it is for).
pub const MAX_NAME: usize = 64;

diesel::table! {
    admin_keys (id) {
        id -> Text,
        name -> Text,
        role -> Text,
        key_hash -> Text,
        created_at -> BigInt,
        created_by -> Text,
        revoked_at -> Nullable<BigInt>,
        revoked_by -> Nullable<Text>,
    }
}

diesel::table! {
    admin_actions (id) {
        id -> BigInt,
        at -> BigInt,
        key_id -> Text,
        operator -> Text,
        action -> Text,
        target -> Nullable<Text>,
        game -> Nullable<Text>,
        detail -> Nullable<Text>,
        approved_by -> Nullable<Text>,
        via_node -> Nullable<Text>,
        request_key -> Nullable<Text>,
    }
}

/// What a key may do. Each role does what the one before does.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Reads: players, the server, hunts, timeouts, modules.
    Viewer,
    /// Viewer, and kick, timeout and ban.
    Moderator,
    /// Moderator, and unban, lift a timeout, load and unload modules, list keys, issue keys below this role.
    Admin,
    /// Everything: every key, revoking keys, and history reverts.
    Owner,
}

impl Role {
    pub const ALL: [Self; 4] = [Self::Viewer, Self::Moderator, Self::Admin, Self::Owner];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Moderator => "moderator",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == text.trim())
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who is asking: the key that authenticated, its role, and the Discord user the bot names (the caller's
/// word, as before).
#[derive(Debug, Clone)]
pub struct Caller {
    /// The key's id: `owner` for `REFORGED_ADMIN_API_KEY`. The change log's actor is `admin:<key>`.
    pub key: String,
    pub role: Role,
    pub operator: Operator,
}

// ------------------------------------------------------------------ the key's shape

fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A key id: what an issued key has after `RRA_`.
pub fn is_key_id(id: &str) -> bool {
    is_lower_hex(id, ID_HEX)
}

/// The id of a key that has the shape of an issued one; nothing else is looked up.
pub fn key_id(key: &str) -> Option<&str> {
    let (id, secret) = key.strip_prefix(KEY_PREFIX)?.split_once('_')?;
    (is_key_id(id) && is_lower_hex(secret, SECRET_HEX)).then_some(id)
}

pub fn hash_key(key: &str) -> String {
    sha256_hex(key.as_bytes())
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill(&mut buffer[..]);
    buffer
        .iter()
        .flat_map(|&b| [b >> 4, b & 15])
        .filter_map(|n| char::from_digit(u32::from(n), 16))
        .collect()
}

/// A new key and its id.
fn generate() -> (String, String) {
    let id = random_hex(ID_HEX / 2);
    let key = format!("{KEY_PREFIX}{id}_{}", random_hex(SECRET_HEX / 2));
    (id, key)
}

/// A key's name as the log and `admin key list` show it: one line, 1 to [`MAX_NAME`] characters.
pub fn clean_name(given: &str) -> Option<String> {
    let name: String = given.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let name = name.trim();
    (1..=MAX_NAME).contains(&name.chars().count()).then(|| name.to_owned())
}

// ------------------------------------------------------------------ the store

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("a key for {0} is already in use: revoke it first")]
    NameTaken(String),
    #[error("no such key, or it is already revoked")]
    NotFound,
    #[error("the owner's key is REFORGED_ADMIN_API_KEY in secrets.env: change it there")]
    Owner,
    #[error("{0}")]
    Store(#[from] store::Error),
}

impl From<diesel::result::Error> for KeyError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Store(error.into())
    }
}

/// A key as `admin key list` shows it: never the key or its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub created_at: i64,
    /// The actor that issued it, as the change log writes actors (`host:...`, `admin:<key>`).
    pub created_by: String,
    pub revoked_at: Option<i64>,
    pub revoked_by: Option<String>,
}

#[derive(Queryable)]
struct Row {
    id: String,
    name: String,
    role: String,
    key_hash: String,
    created_at: i64,
    created_by: String,
    revoked_at: Option<i64>,
    revoked_by: Option<String>,
}

impl Row {
    fn info(self) -> Option<KeyInfo> {
        Some(KeyInfo {
            role: Role::parse(&self.role)?,
            id: self.id,
            name: self.name,
            created_at: self.created_at,
            created_by: self.created_by,
            revoked_at: self.revoked_at,
            revoked_by: self.revoked_by,
        })
    }
}

/// Why a request's key was not accepted. The caller is told only 401; the log gets the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Not the owner's key and not an issued one
    Unknown,
    /// An issued key that was revoked (its id, which is not secret)
    Revoked(String),
    /// The keys' database did not answer; the owner's key still works
    Unavailable,
}

/// One moderation action, for `admin_actions`.
#[derive(Debug, Clone, Default)]
pub struct Action<'a> {
    pub operator: &'a str,
    pub action: &'a str,
    pub target: Option<&'a str>,
    pub game: Option<&'a str>,
    pub detail: Option<&'a str>,
    /// The second operator's key when a guarded action ran on their approval (DH6).
    pub approved_by: Option<&'a str>,
    /// The originating peer node when this action arrived over `Service::Admin` (DN11).
    pub via_node: Option<&'a str>,
    /// The relay idempotency key when this action arrived over `Service::Admin` (DN11).
    pub request_key: Option<&'a str>,
}

/// The admin API's keys: the owner's from `secrets.env`, the rest in the database.
#[derive(Clone)]
pub struct Keys {
    db: Database,
    owner: Arc<str>,
}

impl Keys {
    /// Opens (or creates) the admin API's database and brings its schema up to date. Blocking: call it at
    /// start. `owner` is `REFORGED_ADMIN_API_KEY`.
    pub fn open(config: &DatabaseConfig, owner: &str) -> Result<Self, String> {
        if let DatabaseConfig::Sqlite { file } = config
            && let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty())
        {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let db = Database::open(config, &DatabaseOptions::default(), &SCHEMA)
            .map_err(|e| format!("the admin API database: {e}"))?;
        Ok(Self {
            db,
            owner: owner.into(),
        })
    }

    /// The admin API database beside the node SQLite file, or the node Postgres database.
    pub fn database_for(node: &DatabaseConfig) -> DatabaseConfig {
        match node {
            DatabaseConfig::Sqlite { file } => DatabaseConfig::Sqlite {
                file: file.with_file_name(FILE_NAME),
            },
            other => other.clone(),
        }
    }

    /// Whose key `given` is. The owner key is checked first and needs no database, so it still works
    /// when the database does not answer. A revoked key is refused here, on its next request.
    pub async fn authenticate(&self, given: &str) -> Result<(String, Role), AuthError> {
        if same_secret(given, &self.owner) {
            return Ok((OWNER_KEY.to_owned(), Role::Owner));
        }
        let Some(id) = key_id(given) else {
            return Err(AuthError::Unknown);
        };
        let id = id.to_owned();
        let row: Option<Row> = self
            .db
            .read(move |conn: &mut AnyConnection| {
                Ok::<_, store::Error>(admin_keys::table.find(id).first(conn).optional()?)
            })
            .await
            .map_err(|_| AuthError::Unavailable)?;
        // The same answer for an id nobody has and a wrong secret, so the id existence is not confirmed
        let Some(row) = row.filter(|r| same_secret(&hash_key(given), &r.key_hash)) else {
            return Err(AuthError::Unknown);
        };
        if row.revoked_at.is_some() {
            return Err(AuthError::Revoked(row.id));
        }
        let id = row.id.clone();
        row.info().map(|k| (id, k.role)).ok_or(AuthError::Unknown)
    }

    /// Makes a key for `name`. Returns it with the key itself, which is not kept and not shown again.
    /// One active key per name.
    pub async fn issue(&self, name: &str, role: Role, by: Actor, at: i64) -> Result<(KeyInfo, String), KeyError> {
        let name = clean_name(name).ok_or(KeyError::Invalid("a key name is 1 to 64 characters"))?;
        let (id, key) = generate();
        let info = KeyInfo {
            id: id.clone(),
            name: name.clone(),
            role,
            created_at: at,
            created_by: by.as_text(),
            revoked_at: None,
            revoked_by: None,
        };
        let row = (
            admin_keys::id.eq(id),
            admin_keys::name.eq(name.clone()),
            admin_keys::role.eq(role.as_str()),
            admin_keys::key_hash.eq(hash_key(&key)),
            admin_keys::created_at.eq(at),
            admin_keys::created_by.eq(info.created_by.clone()),
        );
        self.db
            .write(by, move |tx| {
                let active: Vec<String> = admin_keys::table
                    .filter(admin_keys::revoked_at.is_null())
                    .select(admin_keys::name)
                    .load(tx.conn())?;
                if active.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    return Err(KeyError::NameTaken(name));
                }
                diesel::insert_into(admin_keys::table).values(row).execute(tx.conn())?;
                Ok(())
            })
            .await?;
        Ok((info, key))
    }

    /// Every key, the owner first, then by when it was issued. Never a key or a hash.
    pub async fn list(&self) -> Result<Vec<KeyInfo>, KeyError> {
        let rows: Vec<Row> = self
            .db
            .read(|conn: &mut AnyConnection| {
                Ok::<_, store::Error>(
                    admin_keys::table
                        .order((admin_keys::created_at.asc(), admin_keys::id.asc()))
                        .load(conn)?,
                )
            })
            .await?;
        let mut keys = vec![KeyInfo {
            id: OWNER_KEY.to_owned(),
            name: "REFORGED_ADMIN_API_KEY".to_owned(),
            role: Role::Owner,
            created_at: 0,
            created_by: Actor::Host("setup".into()).as_text(),
            revoked_at: None,
            revoked_by: None,
        }];
        keys.extend(rows.into_iter().filter_map(Row::info));
        Ok(keys)
    }

    /// Revokes a key: its next request is refused. The row stays, with who revoked it and when.
    pub async fn revoke(&self, id: &str, by: Actor, at: i64) -> Result<KeyInfo, KeyError> {
        if id == OWNER_KEY {
            return Err(KeyError::Owner);
        }
        if !is_key_id(id) {
            return Err(KeyError::Invalid(
                "a key id is 8 lower-case hex digits, as admin key list shows",
            ));
        }
        let id = id.to_owned();
        let by_text = by.as_text();
        self.db
            .write(by, move |tx| {
                let live = admin_keys::table
                    .filter(admin_keys::id.eq(&id))
                    .filter(admin_keys::revoked_at.is_null());
                let changed = diesel::update(live)
                    .set((
                        admin_keys::revoked_at.eq(Some(at)),
                        admin_keys::revoked_by.eq(Some(by_text)),
                    ))
                    .execute(tx.conn())?;
                if changed == 0 {
                    return Err(KeyError::NotFound);
                }
                let row: Row = admin_keys::table.find(&id).first(tx.conn())?;
                row.info().ok_or(KeyError::NotFound)
            })
            .await
    }

    /// Records a moderation action under the key that made it: the change log actor is admin:<key id>.
    pub async fn record(&self, key: &str, action: Action<'_>, at: i64) -> Result<(), store::Error> {
        let values = (
            admin_actions::at.eq(at),
            admin_actions::key_id.eq(key.to_owned()),
            admin_actions::operator.eq(action.operator.to_owned()),
            admin_actions::action.eq(action.action.to_owned()),
            admin_actions::target.eq(action.target.map(str::to_owned)),
            admin_actions::game.eq(action.game.map(str::to_owned)),
            admin_actions::detail.eq(action.detail.map(str::to_owned)),
            admin_actions::approved_by.eq(action.approved_by.map(str::to_owned)),
            admin_actions::via_node.eq(action.via_node.map(str::to_owned)),
            admin_actions::request_key.eq(action.request_key.map(str::to_owned)),
        );
        self.db
            .write(Actor::Admin(key.to_owned()), move |tx| {
                diesel::insert_into(admin_actions::table)
                    .values(values)
                    .execute(tx.conn())?;
                Ok::<_, store::Error>(())
            })
            .await
    }

    /// Whether a relayed action from `via_node` with `request_key` was already recorded here (DN11).
    pub async fn has_relayed(&self, via_node: &str, request_key: &str) -> Result<bool, store::Error> {
        let via_node = via_node.to_owned();
        let request_key = request_key.to_owned();
        self.db
            .read(move |conn: &mut AnyConnection| {
                let count: i64 = admin_actions::table
                    .filter(admin_actions::via_node.eq(via_node))
                    .filter(admin_actions::request_key.eq(request_key))
                    .count()
                    .get_result(conn)?;
                Ok::<_, store::Error>(count > 0)
            })
            .await
    }

    /// The role of an active key: `owner` for the owner's, an issued key's own unless revoked. A guarded
    /// action runs only while the key that started it is still one (DH6).
    pub async fn active_role(&self, id: &str) -> Result<Option<Role>, store::Error> {
        if id == OWNER_KEY {
            return Ok(Some(Role::Owner));
        }
        let id = id.to_owned();
        let row: Option<Row> = self
            .db
            .read(move |conn: &mut AnyConnection| {
                Ok::<_, store::Error>(admin_keys::table.find(id).first(conn).optional()?)
            })
            .await?;
        Ok(row
            .filter(|r| r.revoked_at.is_none())
            .and_then(Row::info)
            .map(|k| k.role))
    }

    /// How many of `actions` the key made since `since` (ms): what a bulk threshold counts (DH6).
    pub async fn count_actions(&self, key: &str, actions: &[&str], since: i64) -> Result<i64, store::Error> {
        let key = key.to_owned();
        let actions: Vec<String> = actions.iter().map(|a| (*a).to_owned()).collect();
        self.db
            .read(move |conn: &mut AnyConnection| {
                Ok::<_, store::Error>(
                    admin_actions::table
                        .filter(admin_actions::key_id.eq(key))
                        .filter(admin_actions::action.eq_any(actions))
                        .filter(admin_actions::at.ge(since))
                        .count()
                        .get_result(conn)?,
                )
            })
            .await
    }

    /// The database, for the change log readers (history, shipping).
    pub fn database(&self) -> &Database {
        &self.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_has_a_public_id_and_a_secret_and_nothing_else_is_looked_up() {
        let (id, key) = generate();
        assert!(is_key_id(&id));
        assert_eq!(key_id(&key), Some(id.as_str()));
        assert_eq!(key.len(), KEY_PREFIX.len() + ID_HEX + 1 + SECRET_HEX);
        assert_ne!(generate().1, key);
        let dashed = key.replace("_", "-");
        let letters = format!("{KEY_PREFIX}{id}_{}", "g".repeat(SECRET_HEX));
        let longer = format!("{key}0");
        let upper = key.to_uppercase();
        for bad in [
            "",
            "RRA_",
            "RRK_0123abcd_00",
            "0123456789abcdef0123456789abcdef",
            &key[..key.len() - 1],
            &longer,
            &upper,
            &dashed,
            &letters,
        ] {
            assert_eq!(key_id(bad), None, "{bad}");
        }
        assert_eq!(hash_key(&key).len(), 64);
        assert_ne!(hash_key(&key), hash_key(&generate().1));
    }

    #[test]
    fn roles_are_ordered_and_named() {
        assert!(Role::Viewer < Role::Moderator && Role::Moderator < Role::Admin && Role::Admin < Role::Owner);
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse(" admin "), Some(Role::Admin));
        for bad in ["", "Admin", "root", "mod"] {
            assert_eq!(Role::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_name_is_one_short_line() {
        assert_eq!(clean_name("  Alex\nB "), Some("Alex B".into()));
        assert_eq!(clean_name(" "), None);
        assert_eq!(clean_name(&"x".repeat(MAX_NAME + 1)), None);
        assert!(clean_name(&"x".repeat(MAX_NAME)).is_some());
    }
}
