//! `controllers/character.ts` and `controllers/loadout.ts` over the repositories: one closure is one transaction on
//! `store`, so Node's revision-retry loops (they raced other requests) run inside it. A client names a player by its
//! `userId`; the rows belong to the account's global id (DD18), found through `accounts`.
//!
//! Other areas call [`characters_for_user`] (`GetCharactersForUid`, which creates the first character),
//! [`ensure_bootstrap`] (`EnsureStarterBootstrapRecords`) and [`is_fresh_onboarding`].

use module_api::ActorContext;
use serde_json::{Map, Value, json};
use service_kit::{js, jsjson};

use super::data::{self, DataError};
use super::starter::{self, EnsureError};
use crate::db::IslesStore;
use crate::repo::auth::AccountRecord;
use crate::repo::characters::slots::{self, DEFAULT_ACCOUNT_LOADOUT_SLOTS, MAX_TOTAL_LOADOUT_SLOTS, SlotRangeError};
use crate::repo::characters::{CharacterRecord, LoadoutRecord};
use crate::repo::inventory::InventoryRecord;
use crate::repo::{AccountId, RepoError, Repositories, Tx, UserId};
use crate::tables::VersionTables;

/// Node's `MAX_ATTEMPTS` for a revision-guarded write.
const MAX_ATTEMPTS: u32 = 10;

#[derive(Debug, thiserror::Error)]
pub enum CtlError {
    /// `CharacterOwnershipError`: the character is not this account's
    #[error("the character does not belong to the user")]
    NotOwned,
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Data(#[from] DataError),
    /// Node threw an `Error`
    #[error("{0}")]
    Failed(String),
}

impl From<EnsureError> for CtlError {
    fn from(error: EnsureError) -> Self {
        match error {
            EnsureError::NotOwned { .. } => Self::NotOwned,
            EnsureError::Repo(error) => Self::Repo(error),
        }
    }
}

impl From<SlotRangeError> for CtlError {
    fn from(error: SlotRangeError) -> Self {
        Self::Failed(error.0)
    }
}

type Outcome<T> = Result<T, CtlError>;

/// What a closure on `store` returns when it can also fail in Node's terms.
type Inner<T> = Result<T, CtlError>;

/// The nested result of a closure: the outer error is the database's, the inner one Node's.
fn flatten<T>(result: Result<Inner<T>, RepoError>) -> Outcome<T> {
    result?
}

/// The account of a `userId`, if there is one.
pub async fn find_account(store: &IslesStore, user_id: &str) -> Result<Option<AccountId>, RepoError> {
    let user = UserId(user_id.to_owned());
    let found = store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await?;
    Ok(found.map(|account| account.id))
}

/// The account of a `userId`, created when there is none: Node kept characters under any `userId` a trusted caller
/// named (a gameserver's configured account has no login), so the first read makes the account.
pub async fn find_or_create_account(
    store: &IslesStore,
    actor: &ActorContext,
    user_id: &str,
) -> Result<AccountId, RepoError> {
    if let Some(found) = find_account(store, user_id).await? {
        return Ok(found);
    }
    let node = store.node().clone();
    let user = UserId(user_id.to_owned());
    store
        .write(actor, move |tx, repos| {
            // Another request may have made it between the read and this write
            if let Some(found) = repos.accounts.find_by_user_id(tx.conn(), &user)? {
                return Ok(found.id);
            }
            let id = AccountId::new(&node);
            repos.accounts.create(
                tx,
                &AccountRecord {
                    id: id.clone(),
                    user_id: user.clone(),
                    name: user.0.clone(),
                    notes: 0,
                },
            )?;
            Ok(id)
        })
        .await
}

fn changelist(tables: &VersionTables) -> Option<u32> {
    Some(tables.target_changelist)
}

/// `TransformDbCharacterToWireCharacter`: `accountId` is the client's `userId`, and `data` is normalised on the way out.
pub fn wire_character(
    record: &CharacterRecord,
    user_id: &str,
    dojo_unlocked: bool,
    tables: &VersionTables,
    now: i64,
) -> Result<Value, DataError> {
    let (data, _) = data::ensure_legacy_dojo_series(&record.data, dojo_unlocked, changelist(tables), now)?;
    Ok(json!({
        "accountId": user_id,
        "catalogDaoId": null,
        "createdDate": record.created_date,
        "data": data,
        "id": record.character_id,
        "lastModifiedDate": record.last_modified_date,
        "name": record.name,
        "updateVersion": record.update_version
    }))
}

/// Whether the Slayer's Path has Dojo unlocked. A journey that can't be read counts as locked, as in Node.
fn read_dojo_unlocked(conn: &mut crate::repo::AnyConnection, repos: &Repositories, account: &AccountId) -> bool {
    match repos.player_journey.find(conn, account) {
        Ok(Some(journey)) => data::dojo_unlocked(&journey.nodes),
        Ok(None) => false,
        Err(error) => {
            tracing::warn!("[dojo-reconcile] Unable to read Player Journey for account {account}: {error}");
            false
        }
    }
}

/// `GetUsernameForUserId`: the launcher account's display name, else the account's name, else the `userId`.
fn username(conn: &mut crate::repo::AnyConnection, repos: &Repositories, account: &AccountId, user_id: &str) -> String {
    if let Ok(Some(launcher)) = repos.launcher_accounts.find_by_account(conn, account)
        && !launcher.display_name.is_empty()
    {
        return launcher.display_name;
    }
    match repos.accounts.find(conn, account) {
        Ok(Some(record)) => record.name,
        _ => user_id.to_owned(),
    }
}

/// `CreateCharacterForUid`: the character row and the whole starter manifest (inventory, loadout, wallet) together.
pub fn create_character(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    tables: &VersionTables,
    account: &AccountId,
    name: &str,
    now: i64,
) -> Outcome<CharacterRecord> {
    let today = data::display_date(now);
    let record = CharacterRecord {
        character_id: uuid::Uuid::new_v4().to_string(),
        account: account.clone(),
        created_date: today.clone(),
        last_modified_date: today,
        name: name.to_owned(),
        update_version: 0,
        data: data::normalize("{}", changelist(tables), now)?,
    };
    let created = repos.characters.create(tx, &record)?;
    starter::seed_new_account(tx, repos, tables, account, &created.character_id)?;
    Ok(created)
}

/// `PUT /character`: a new character for the player, wired.
pub async fn create_character_for_user(
    store: &IslesStore,
    tables: &VersionTables,
    actor: &ActorContext,
    user_id: &str,
    name: &str,
) -> Outcome<Value> {
    let account = find_or_create_account(store, actor, user_id).await?;
    let (tables, name, now, owner) = (*tables, name.to_owned(), data::now_secs(), user_id.to_owned());
    let created = store
        .account_write(&account.clone(), actor, move |tx, repos| {
            Ok(create_character(tx, repos, &tables, &account, &name, now)
                .and_then(|record| wire_character(&record, &owner, false, &tables, now).map_err(CtlError::from)))
        })
        .await;
    flatten(created)
}

/// `GetCharactersForUid`: the player's characters, wired, creating the first one (named after the player) when
/// there is none.
pub async fn characters_for_user(
    store: &IslesStore,
    tables: &VersionTables,
    actor: &ActorContext,
    user_id: &str,
) -> Outcome<Vec<Value>> {
    let account = find_or_create_account(store, actor, user_id).await?;
    let now = data::now_secs();
    let (tables_copy, owner) = (*tables, user_id.to_owned());
    let read = |account: AccountId, owner: String| {
        store.read(move |conn, repos| {
            let found = repos.characters.find_many_by_account(conn, &account)?;
            let dojo = if found.is_empty() {
                false
            } else {
                read_dojo_unlocked(conn, repos, &account)
            };
            Ok((found, dojo, owner))
        })
    };
    let (mut found, mut dojo, _) = read(account.clone(), owner.clone()).await?;
    if found.is_empty() {
        let (create_for, create_owner) = (account.clone(), owner.clone());
        flatten(
            store
                .account_write(&account, actor, move |tx, repos| {
                    // A concurrent first read may have made it
                    if !repos
                        .characters
                        .find_many_by_account(tx.conn(), &create_for)?
                        .is_empty()
                    {
                        return Ok(Ok(()));
                    }
                    let name = username(tx.conn(), repos, &create_for, &create_owner);
                    Ok(create_character(tx, repos, &tables_copy, &create_for, &name, now).map(|_| ()))
                })
                .await,
        )?;
        (found, dojo, _) = read(account.clone(), owner.clone()).await?;
    }
    found
        .iter()
        .map(|record| wire_character(record, user_id, dojo, tables, now).map_err(CtlError::from))
        .collect()
}

/// `GetCharacterWithUid`: `None` when the player has no such character.
pub async fn character_for_user(
    store: &IslesStore,
    tables: &VersionTables,
    user_id: &str,
    character_id: &str,
) -> Outcome<Option<Value>> {
    let Some(account) = find_account(store, user_id).await? else {
        return Ok(None);
    };
    let (id, now) = (character_id.to_owned(), data::now_secs());
    let found = store
        .read(move |conn, repos| {
            let record = repos.characters.find(conn, &id, &account)?;
            let dojo = record.is_some() && read_dojo_unlocked(conn, repos, &account);
            Ok((record, dojo))
        })
        .await?;
    match found {
        (Some(record), dojo) => Ok(Some(wire_character(&record, user_id, dojo, tables, now)?)),
        (None, _) => Ok(None),
    }
}

/// `body.data` as `CharacterDataToUpdateWith || "{}"` reads it.
fn data_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        other if js::truthy(other) => js::string(other),
        _ => "{}".to_owned(),
    }
}

/// `UpdateCharacterForUid`. `false` is Node's refusal: no such character, or a client write on a stale version
/// (a gameserver is authoritative and is written through at the next version). `update_version` is the body's.
#[allow(clippy::too_many_arguments)] // Node's UpdateCharacterForUid takes these
pub async fn update_character_for_user(
    store: &IslesStore,
    tables: &VersionTables,
    actor: &ActorContext,
    user_id: &str,
    character_id: &str,
    new_data: Option<&Value>,
    update_version: Option<&Value>,
    is_gameserver: bool,
) -> Outcome<bool> {
    let Some(current) = character_for_user(store, tables, user_id, character_id).await? else {
        tracing::warn!(
            "UpdateCharacterForUid: no character record for characterId {character_id} userId {user_id} (isGameserver={is_gameserver}) - treating as a rejected write instead of throwing"
        );
        return Ok(false);
    };
    let current_version = current["updateVersion"].as_i64().unwrap_or(0);
    let requested = js::number(update_version);
    // `current >= requested` is false for NaN; a version that isn't a whole number can't be stored, so it writes nothing
    let mut effective =
        (requested.fract() == 0.0 && requested.abs() <= 9_007_199_254_740_991.0).then_some(requested as i64);
    if current_version as f64 >= requested {
        if !is_gameserver {
            return Ok(false);
        }
        effective = Some(current_version + 1);
        tracing::info!(
            "[gameserver-force] Stale gameserver write for characterId {character_id} - was updateVersion {requested}, forcing to {} (server is authoritative for progress/PJM)",
            current_version + 1
        );
    }

    let now = data::now_secs();
    let current_data = current["data"].as_str().unwrap_or("{}");
    let cl = changelist(tables);
    let guarded = data::guard_progression(&data_text(new_data), current_data);
    let processed = data::normalize(&data::process_triggers(&guarded, cl, now)?, cl, now)?;
    // The read overlays SERIE_dojo whenever the journey has Dojo unlocked; keep that on every write so a stale
    // client blob can't remove the legacy gate again
    let dojo = jsjson::parse(current_data)
        .ok()
        .is_some_and(|d| !matches!(d.get("SERIE_dojo"), None | Some(Value::Null)));
    let (final_data, added) = data::ensure_legacy_dojo_series(&processed, dojo, cl, now)?;
    if added {
        tracing::info!("[dojo-reconcile] Persisting SERIE_dojo for userId {user_id} characterId {character_id}");
    }

    let Some(effective) = effective else {
        return Ok(true);
    };
    let Some(account) = find_account(store, user_id).await? else {
        return Ok(false);
    };
    let id = character_id.to_owned();
    store
        .account_write(&account.clone(), actor, move |tx, repos| {
            repos
                .characters
                .update_data_conditional(tx, &id, &account, &final_data, effective)
        })
        .await?;
    Ok(true)
}

/// `IsFreshOnboarding`.
pub fn is_fresh_onboarding(
    conn: &mut crate::repo::AnyConnection,
    repos: &Repositories,
    account: &AccountId,
) -> Result<bool, RepoError> {
    let characters = repos.characters.find_many_by_account(conn, account)?;
    let texts: Vec<&str> = characters.iter().map(|c| c.data.as_str()).collect();
    Ok(texts.is_empty() || data::is_fresh(&texts))
}

/// A loadout as the routes answer it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadoutState {
    pub loadouts: Vec<Value>,
    pub persistent: Value,
    pub active_index: u32,
}

fn parse_loadout_array(raw: &str, character_id: &str) -> Outcome<Vec<Value>> {
    let parsed = jsjson::parse(raw).map_err(|_| {
        CtlError::Failed(format!(
            "Persisted loadout JSON is invalid for character {character_id}"
        ))
    })?;
    match parsed {
        Value::Array(items)
            if (DEFAULT_ACCOUNT_LOADOUT_SLOTS as usize..=MAX_TOTAL_LOADOUT_SLOTS as usize).contains(&items.len()) =>
        {
            Ok(items)
        }
        Value::Array(items) => Err(CtlError::Failed(format!(
            "Persisted loadout count {} is invalid for character {character_id}",
            items.len()
        ))),
        _ => Err(CtlError::Failed(format!(
            "Persisted loadout count non-array is invalid for character {character_id}"
        ))),
    }
}

/// `GetLoadoutStateForUserIdAndCharacterId` from the loadout row.
pub fn state_of(row: &LoadoutRecord, tables: &VersionTables) -> Outcome<LoadoutState> {
    let stored = parse_loadout_array(&row.loadouts, &row.character_id)?;
    let visible = slots::resolve_visible_total(stored.len() as i64, row.unlocked_total_slots.map(i64::from))?;
    let persistent = jsjson::parse(&row.persistent)
        .map_err(|e| CtlError::Failed(format!("Persisted loadout persistent is invalid: {e}")))?;
    Ok(LoadoutState {
        loadouts: stored.into_iter().take(visible as usize).collect(),
        persistent: starter::with_default_chat_wheels(persistent, tables),
        active_index: slots::resolve_active_index(row.active_index.map(i64::from), visible)?,
    })
}

/// `EnsureStarterBootstrapRecords`: repairs a missing inventory or loadout, after the ownership gate. A read decides
/// whether anything needs writing, so an ordinary request opens no write.
pub async fn ensure_bootstrap(
    store: &IslesStore,
    tables: &VersionTables,
    actor: &ActorContext,
    user_id: &str,
    character_id: &str,
) -> Outcome<(InventoryRecord, LoadoutRecord)> {
    let Some(account) = find_account(store, user_id).await? else {
        return Err(CtlError::NotOwned);
    };
    let (tables, id, owner) = (*tables, character_id.to_owned(), user_id.to_owned());
    let account_for_read = account.clone();
    let read_id = id.clone();
    let present = store
        .read(move |conn, repos| {
            if repos.characters.find(conn, &read_id, &account_for_read)?.is_none() {
                return Ok(None);
            }
            let inventory = repos.inventories.find_by_character_id(conn, &read_id)?;
            let loadout = repos.loadouts.find(conn, &read_id, &account_for_read)?;
            Ok(Some((inventory, loadout)))
        })
        .await?;
    match present {
        None => return Err(CtlError::NotOwned),
        Some((Some(inventory), Some(loadout))) if !starter::needs_repair(Some(&inventory), Some(&loadout), &tables) => {
            return Ok((inventory, loadout));
        }
        Some(_) => {}
    }
    let repaired = store
        .account_write(&account.clone(), actor, move |tx, repos| {
            Ok(starter::ensure_in_transaction(tx, repos, &tables, &account, &owner, &id).map_err(CtlError::from))
        })
        .await;
    flatten(repaired)
}

/// `GetLoadoutStateForUserIdAndCharacterId`.
pub async fn loadout_state(
    store: &IslesStore,
    tables: &VersionTables,
    actor: &ActorContext,
    user_id: &str,
    character_id: &str,
) -> Outcome<LoadoutState> {
    let (_, loadout) = ensure_bootstrap(store, tables, actor, user_id, character_id).await?;
    state_of(&loadout, tables)
}

fn clone_for_slot(source: &Value, slot_index: usize) -> Value {
    let mut clone = source.clone();
    if let Value::Object(map) = &mut clone {
        map.insert("slot_index".into(), json!(slot_index));
        map.insert("update_version".into(), json!(0));
        map.insert("custom_name".into(), json!(""));
    }
    clone
}

/// `EnsureTotalLoadoutSlots`: grows the stored slots to the entitlement and records it. Never relocks a slot, and
/// stored excess contents stay dormant.
pub async fn ensure_total_slots(
    store: &IslesStore,
    tables: &VersionTables,
    actor: &ActorContext,
    user_id: &str,
    character_id: &str,
    desired: u32,
) -> Outcome<()> {
    if !(DEFAULT_ACCOUNT_LOADOUT_SLOTS..=MAX_TOTAL_LOADOUT_SLOTS).contains(&desired) {
        return Err(CtlError::Failed(format!(
            "Total loadout slot count must be between 1 and {MAX_TOTAL_LOADOUT_SLOTS}"
        )));
    }
    ensure_bootstrap(store, tables, actor, user_id, character_id).await?;
    let Some(account) = find_account(store, user_id).await? else {
        return Err(CtlError::NotOwned);
    };
    let id = character_id.to_owned();
    let done = store
        .account_write(&account.clone(), actor, move |tx, repos| {
            Ok((|| {
                for attempt in 0..MAX_ATTEMPTS {
                    let Some(row) = repos.loadouts.find(tx.conn(), &id, &account)? else {
                        return Err(CtlError::Failed(format!(
                            "Bootstrap recovery did not create loadout for {id}"
                        )));
                    };
                    let stored = parse_loadout_array(&row.loadouts, &id)?;
                    let persisted = row.unlocked_total_slots.map(i64::from);
                    if persisted.is_some() {
                        slots::resolve_visible_total(stored.len() as i64, persisted)?;
                    }
                    let target = slots::resolve_requested_total(persisted, i64::from(desired))? as usize;
                    let mut extended = stored.clone();
                    while extended.len() < target {
                        let next = extended.len();
                        extended.push(clone_for_slot(&stored[0], next));
                    }
                    if extended.len() == stored.len() && persisted == Some(target as i64) {
                        return Ok(());
                    }
                    let applied = repos.loadouts.replace_all_and_entitlement_if_revision_matches(
                        tx,
                        &id,
                        &account,
                        &jsjson::stringify(&Value::Array(extended)),
                        target as u32,
                        row.revision.unwrap_or(0),
                    )?;
                    if applied {
                        return Ok(());
                    }
                    tracing::warn!(
                        "Loadout unlock revision conflict for characterId {id}, attempt {}/{MAX_ATTEMPTS} - retrying",
                        attempt + 1
                    );
                }
                Err(CtlError::Failed(format!(
                    "Loadout unlock for characterId {id} failed after {MAX_ATTEMPTS} revision-conflict retries"
                )))
            })())
        })
        .await;
    flatten(done)
}

const MAX_LOADOUT_DATA_LENGTH: usize = 1_048_576;

/// `ValidateLoadoutWriteData`: the reason a write is refused, or `None`.
pub fn validate_loadout_write(index: &str, data: Option<&Value>) -> Option<String> {
    let numeric = index.parse::<i64>().ok();
    if index != "persistent" && !numeric.is_some_and(|n| n.to_string() == index && (0..=5).contains(&n)) {
        return Some(format!("unsupported loadout index {index}"));
    }
    let Some(Value::String(text)) = data else {
        return Some("data must be a non-empty JSON string".into());
    };
    if text.is_empty() {
        return Some("data must be a non-empty JSON string".into());
    }
    if text.encode_utf16().count() > MAX_LOADOUT_DATA_LENGTH {
        return Some("data is too large".into());
    }
    match jsjson::parse(text) {
        Err(_) => Some("data is not valid JSON".into()),
        Ok(Value::Object(_)) => None,
        Ok(_) => Some("data must be a JSON object".into()),
    }
}

/// `SetLoadoutDataForUserIdAndCharacterId`: whether the write landed.
pub async fn set_loadout_data(
    store: &IslesStore,
    actor: &ActorContext,
    user_id: &str,
    character_id: &str,
    index: &str,
    data: Option<&Value>,
) -> Outcome<bool> {
    tracing::info!("Attempting to update loadout data Index {index}");
    if let Some(reason) = validate_loadout_write(index, data) {
        tracing::error!("Loadout write rejected for characterId {character_id}: {reason}");
        return Ok(false);
    }
    let Some(account) = find_account(store, user_id).await? else {
        return Ok(false);
    };
    let (id, index, text) = (
        character_id.to_owned(),
        index.to_owned(),
        data.and_then(Value::as_str).unwrap_or_default().to_owned(),
    );
    let applied = store
        .account_write(&account.clone(), actor, move |tx, repos| {
            for attempt in 0..MAX_ATTEMPTS {
                // No row: the guarded write could only refuse (revision 0 of nothing)
                let Some(row) = repos.loadouts.find(tx.conn(), &id, &account)? else {
                    return Ok(false);
                };
                let expected = row.revision.unwrap_or(0);
                let applied = if index == "persistent" {
                    repos.loadouts.update_persistent_if_revision_matches(tx, &id, &account, &text, expected)?
                } else {
                    // `validate_loadout_write` allowed only 0..=5
                    let slot = index.parse::<u32>().unwrap_or(0);
                    repos.loadouts.replace_slot_if_revision_matches(tx, &id, &account, slot, &text, expected)?
                };
                if applied {
                    return Ok(true);
                }
                tracing::warn!(
                    "Loadout revision conflict for characterId {id} index {index}, attempt {}/{MAX_ATTEMPTS} - retrying",
                    attempt + 1
                );
            }
            tracing::error!("Loadout write for characterId {id} index {index} failed after {MAX_ATTEMPTS} revision-conflict retries");
            Ok(false)
        })
        .await?;
    Ok(applied)
}

/// `SetActiveLoadoutIndexForUserIdAndCharacterId`: whether the slot is now the active one. A slot that isn't
/// unlocked is refused.
pub async fn set_active_index(
    store: &IslesStore,
    tables: &VersionTables,
    actor: &ActorContext,
    user_id: &str,
    character_id: &str,
    active_index: i64,
) -> Outcome<bool> {
    if !(0..i64::from(MAX_TOTAL_LOADOUT_SLOTS)).contains(&active_index) {
        return Ok(false);
    }
    ensure_bootstrap(store, tables, actor, user_id, character_id).await?;
    let Some(account) = find_account(store, user_id).await? else {
        return Ok(false);
    };
    let id = character_id.to_owned();
    let done = store
        .account_write(&account.clone(), actor, move |tx, repos| {
            Ok((|| {
                for attempt in 0..MAX_ATTEMPTS {
                    let Some(row) = repos.loadouts.find(tx.conn(), &id, &account)? else {
                        return Ok(false);
                    };
                    let stored = parse_loadout_array(&row.loadouts, &id)?;
                    let visible = slots::resolve_visible_total(stored.len() as i64, row.unlocked_total_slots.map(i64::from))?;
                    if active_index >= i64::from(visible) {
                        return Ok(false);
                    }
                    if repos.loadouts.update_active_index_if_revision_matches(
                        tx,
                        &id,
                        &account,
                        active_index as u32,
                        row.revision.unwrap_or(0),
                    )? {
                        return Ok(true);
                    }
                    tracing::warn!(
                        "Active loadout revision conflict for characterId {id} index {active_index}, attempt {}/{MAX_ATTEMPTS} - retrying",
                        attempt + 1
                    );
                }
                tracing::error!("Active loadout write for characterId {id} index {active_index} failed after {MAX_ATTEMPTS} attempts");
                Ok(false)
            })())
        })
        .await;
    flatten(done)
}

/// The `{code: null, message: "OK", payload}` body of every loadout answer (`LoadoutPayload`).
pub fn loadout_payload(state: &LoadoutState) -> Value {
    let (num_account, max_account, num_character, max_character) = slots::wire_counts(state.loadouts.len() as f64);
    let mut payload = Map::new();
    payload.insert("loadouts".into(), Value::Array(state.loadouts.clone()));
    payload.insert("persistent".into(), state.persistent.clone());
    payload.insert("num_account_slots".into(), json!(num_account));
    payload.insert("max_account_slots".into(), json!(max_account));
    payload.insert("num_character_slots".into(), json!(num_character));
    payload.insert("max_character_slots".into(), json!(max_character));
    payload.insert("active_index".into(), json!(state.active_index));
    payload.insert("needs_migration".into(), json!(false));
    json!({ "code": null, "message": "OK", "payload": Value::Object(payload) })
}

/// The no-player sentinel's (the hub phantom's) empty state: 0/0 account, 1/6 character, active slot 0. Never a 2xx
/// for its `/unlock` (a 2xx would loop the native success branch).
pub fn empty_loadout_response() -> Value {
    let (num_account, max_account, num_character, max_character) = slots::wire_counts(1.0);
    json!({
        "code": null,
        "message": "OK",
        "payload": {
            "loadouts": [],
            "persistent": null,
            "num_account_slots": num_account,
            "max_account_slots": max_account,
            "num_character_slots": num_character,
            "max_character_slots": max_character,
            "active_index": 0,
            "needs_migration": false
        }
    })
}
