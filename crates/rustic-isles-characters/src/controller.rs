// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The area's state and its business rules over the store (spec §5, §7): accounts, characters, the loadout state
//! and its writes, and the starter records' check and repair.

use std::sync::Arc;

use dainty_module_api::ActorContext;
use dainty_service_kit::{js, jsjson};
use rustic_isles::{VersionTables, replay};
use rustic_isles_store::repo::auth::AccountRecord;
use rustic_isles_store::repo::bounties::WalletRecord;
use rustic_isles_store::repo::characters::{CharacterRecord, LoadoutRecord, slots};
use rustic_isles_store::repo::inventory::InventoryRecord;
use rustic_isles_store::{AccountId, AnyConnection, IslesStore, RepoError, RepoResult, Repositories, Tx, UserId};
use serde_json::{Value, json};

use crate::{CharacterData, CharactersError, EnsureError, StarterKit, data, starter};

/// UE4's "no logged-in player" userId.
pub(crate) const INVALID: &str = "INVALID";
const ATTEMPTS: u32 = 10;

/// What a loadout route answers (spec §5.9): the visible slots, `persistent` and the active index.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadoutState {
    /// The visible loadout slots, each a stored slot object.
    pub loadouts: Vec<Value>,
    /// The account-wide `persistent` object (banner, glider, quick chats).
    pub persistent: Value,
    /// The slot the client has selected, an index into `loadouts`.
    pub active_index: u32,
}

/// The loadout answer (spec §4.1).
pub fn loadout_payload(state: &LoadoutState) -> Value {
    let visible = u32::try_from(state.loadouts.len()).unwrap_or(u32::MAX);
    let (account, max_account, character, max_character) = slots::wire_counts(f64::from(visible));
    json!({
        "code": null,
        "message": "OK",
        "payload": {
            "loadouts": state.loadouts,
            "persistent": state.persistent,
            "num_account_slots": account,
            "max_account_slots": max_account,
            "num_character_slots": character,
            "max_character_slots": max_character,
            "active_index": state.active_index,
            "needs_migration": false,
        },
    })
}

/// The hub phantom's answer (spec §4.2).
pub fn empty_loadout_payload() -> Value {
    loadout_payload(&LoadoutState {
        loadouts: Vec::new(),
        persistent: Value::Null,
        active_index: 0,
    })
}

/// `IsFreshOnboarding` for matchmaking's FTUE choice (spec §5.8).
pub fn is_fresh_onboarding(conn: &mut AnyConnection, repos: &Repositories, account: &AccountId) -> RepoResult<bool> {
    let records = repos.characters.find_many_by_account(conn, account)?;
    Ok(data::is_fresh(records.iter().map(|r| r.data.as_str())))
}

pub(crate) fn now_secs() -> i64 {
    replay::now_ms().div_euclid(1000)
}

fn failed(text: impl Into<String>) -> CharactersError {
    CharactersError::Failed(text.into())
}

/// The stored array (1 to 6 entries) and the visible total.
fn stored_slots(row: &LoadoutRecord) -> Result<(Vec<Value>, u32), CharactersError> {
    let id = &row.character_id;
    let items = match jsjson::parse(&row.loadouts) {
        Err(_) => return Err(failed(format!("Persisted loadout JSON is invalid for character {id}"))),
        Ok(Value::Array(items)) if (1..=6).contains(&items.len()) => items,
        Ok(Value::Array(items)) => {
            let n = items.len();
            return Err(failed(format!(
                "Persisted loadout count {n} is invalid for character {id}"
            )));
        }
        Ok(_) => {
            return Err(failed(format!(
                "Persisted loadout count non-array is invalid for character {id}"
            )));
        }
    };
    let stored = i64::try_from(items.len()).unwrap_or(i64::MAX);
    let visible = slots::resolve_visible_total(stored, row.unlocked_total_slots.map(i64::from))
        .map_err(|e| failed(e.to_string()))?;
    Ok((items, visible))
}

/// A copy of slot 0 as slot `index`.
fn copy_slot(first: &Value, index: usize) -> Value {
    let mut copy = first.clone();
    if let Value::Object(map) = &mut copy {
        map.insert("slot_index".into(), Value::from(index));
        map.insert("update_version".into(), Value::from(0));
        map.insert("custom_name".into(), Value::from(""));
    }
    copy
}

pub(crate) async fn lookup(store: &IslesStore, user_id: &str) -> RepoResult<Option<AccountRecord>> {
    let user = UserId::new(user_id);
    store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await
}

/// The account of a userId, created on this node when there is none (no home check: it is new here).
pub(crate) async fn find_or_create(
    store: &IslesStore,
    actor: &ActorContext,
    user_id: &str,
) -> RepoResult<AccountRecord> {
    if let Some(account) = lookup(store, user_id).await? {
        return Ok(account);
    }
    let node = store.node().clone();
    let text = user_id.to_owned();
    store
        .write(actor, move |tx, repos| {
            let user = UserId::new(text.as_str());
            if let Some(account) = repos.accounts.find_by_user_id(tx.conn(), &user)? {
                return Ok(account);
            }
            let record = AccountRecord {
                id: AccountId::new(&node),
                user_id: user,
                name: text,
                notes: 0,
            };
            repos.accounts.create(tx, &record)?;
            Ok(record)
        })
        .await
}

/// What a set-active or data write ended as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Write {
    Accepted,
    Refused,
}

/// The area's state (spec §8.2), in the composer's typed slot.
#[derive(Debug, Clone)]
pub struct Characters {
    data: Arc<CharacterData>,
    kit: Arc<StarterKit>,
}

impl Characters {
    /// The kit's slot appearance is taken from `data`, so the two never disagree.
    pub fn new(data: CharacterData, mut kit: StarterKit) -> Self {
        kit.default_appearance = data.default_appearance.clone();
        Self {
            data: Arc::new(data),
            kit: Arc::new(kit),
        }
    }

    /// The character-data rules this unit applies.
    pub fn data(&self) -> &CharacterData {
        &self.data
    }

    /// The starter kit a new character is given.
    pub fn kit(&self) -> &StarterKit {
        &self.kit
    }

    /// The kit's `instanced` (`rustic-isles-db`'s transforms).
    pub fn instanced_catalog_ids(&self) -> &[String] {
        &self.kit.instanced
    }

    /// The one starter wallet, which the economy unit reads too.
    pub fn starter_wallet(&self) -> &[(String, f64)] {
        &self.kit.wallet
    }

    async fn username(&self, store: &IslesStore, account: &AccountRecord) -> String {
        let id = account.id.clone();
        let launcher = store
            .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &id))
            .await;
        if let Ok(Some(launcher)) = launcher
            && !launcher.display_name.is_empty()
        {
            return launcher.display_name;
        }
        if !account.name.is_empty() {
            return account.name.clone();
        }
        account.user_id.as_str().to_owned()
    }

    /// `IsDojoUnlockedForUser`; a journey read error reads as locked.
    async fn dojo_flag(&self, store: &IslesStore, account: &AccountId) -> bool {
        let id = account.clone();
        match store
            .read(move |conn, repos| repos.player_journey.find(conn, &id))
            .await
        {
            Ok(Some(row)) => data::dojo_unlocked(&row.nodes, &self.data),
            Ok(None) => false,
            Err(e) => {
                tracing::warn!("[dojo-reconcile] Unable to read Player Journey for account {account}: {e}");
                false
            }
        }
    }

    fn read_form(&self, text: &str, tables: &VersionTables, dojo: bool, now: i64) -> Result<String, CharactersError> {
        Ok(data::read_form(text, &self.data, tables.target_changelist, now, dojo)?.0)
    }

    /// The wire character (spec §4.3); `accountId` is the userId the request named.
    fn wire(
        &self,
        record: &CharacterRecord,
        user_id: &str,
        tables: &VersionTables,
        dojo: bool,
        now: i64,
    ) -> Result<Value, CharactersError> {
        Ok(json!({
            "accountId": user_id,
            "catalogDaoId": null,
            "createdDate": record.created_date,
            "data": self.read_form(&record.data, tables, dojo, now)?,
            "id": record.character_id,
            "lastModifiedDate": record.last_modified_date,
            "name": record.name,
            "updateVersion": record.update_version,
        }))
    }

    /// Inventory, loadout and (on a create) the wallet of a new character, in the caller's transaction.
    fn seed(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        tables: &VersionTables,
        account: &AccountId,
        character_id: &str,
        wallet: bool,
    ) -> RepoResult<()> {
        let manifest = starter::build_manifest(&self.kit, tables)?;
        let bootstrap = Some(tables.starter_manifest.bootstrap_version.to_owned());
        let inventory = InventoryRecord {
            character_id: character_id.to_owned(),
            account: Some(account.clone()),
            instanced_items: jsjson::stringify(&Value::Array(manifest.instanced_items)),
            stacked_items: jsjson::stringify(&Value::Array(manifest.stacked_items)),
            revision: Some(0),
            bootstrap_version: bootstrap.clone(),
        };
        repos.inventories.create(tx, &inventory)?;
        let loadout = LoadoutRecord {
            character_id: character_id.to_owned(),
            account: account.clone(),
            loadouts: jsjson::stringify(&Value::Array(vec![manifest.loadout_slot])),
            persistent: jsjson::stringify(&manifest.persistent),
            active_index: None,
            unlocked_total_slots: None,
            revision: Some(0),
            bootstrap_version: bootstrap.clone(),
        };
        repos.loadouts.create(tx, &loadout)?;
        if wallet {
            let record = WalletRecord {
                account: account.clone(),
                balances: self.kit.wallet.clone(),
                bootstrap_version: bootstrap,
            };
            repos.wallets.create_if_missing(tx, &record)?;
        }
        Ok(())
    }

    /// A character and its starter records; the character id is the first id draw.
    fn create_in(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        tables: &VersionTables,
        account: &AccountId,
        name: &str,
        now: i64,
    ) -> RepoResult<CharacterRecord> {
        let character_id = replay::next_uuid();
        let date = data::display_date(now);
        let blob =
            data::normalize("{}", &self.data, tables.target_changelist, now).map_err(|e| RepoError::Data(e.0))?;
        let record = CharacterRecord {
            character_id: character_id.clone(),
            account: account.clone(),
            created_date: date.clone(),
            last_modified_date: date,
            name: name.to_owned(),
            update_version: 0,
            data: blob,
        };
        let record = repos.characters.create(tx, &record)?;
        self.seed(tx, repos, tables, account, &character_id, true)?;
        Ok(record)
    }

    async fn list(&self, store: &IslesStore, account: &AccountId) -> RepoResult<Vec<CharacterRecord>> {
        let id = account.clone();
        store
            .read(move |conn, repos| repos.characters.find_many_by_account(conn, &id))
            .await
    }

    /// `GetCharactersForUid`: the wire characters in creation order, the first one created when there are none.
    pub async fn characters_for_user(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        actor: &ActorContext,
        user_id: &str,
    ) -> Result<Vec<Value>, CharactersError> {
        let account = find_or_create(store, actor, user_id).await?;
        let mut records = self.list(store, &account.id).await?;
        let now = now_secs();
        if records.is_empty() {
            let name = self.username(store, &account).await;
            let (me, tables_copy, id) = (self.clone(), *tables, account.id.clone());
            store
                .account_write(&account.id, actor, move |tx, repos| {
                    // A concurrent request may have made one
                    if !repos.characters.find_many_by_account(tx.conn(), &id)?.is_empty() {
                        return Ok(());
                    }
                    me.create_in(tx, repos, &tables_copy, &id, &name, now).map(|_| ())
                })
                .await?;
            records = self.list(store, &account.id).await?;
        }
        let dojo = !records.is_empty() && self.dojo_flag(store, &account.id).await;
        records
            .iter()
            .map(|record| self.wire(record, user_id, tables, dojo, now))
            .collect()
    }

    /// PUT's create: the account found or made, one more character; wired with the Dojo flag off.
    pub(crate) async fn create_character(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        actor: &ActorContext,
        user_id: &str,
        name: &str,
    ) -> Result<Value, CharactersError> {
        let account = find_or_create(store, actor, user_id).await?;
        let now = now_secs();
        let (me, tables_copy, id, name) = (self.clone(), *tables, account.id.clone(), name.to_owned());
        let record = store
            .account_write(&account.id, actor, move |tx, repos| {
                me.create_in(tx, repos, &tables_copy, &id, &name, now)
            })
            .await?;
        self.wire(&record, user_id, tables, false, now)
    }

    async fn one(
        &self,
        store: &IslesStore,
        user_id: &str,
        character_id: &str,
    ) -> Result<Option<(AccountRecord, CharacterRecord)>, CharactersError> {
        let Some(account) = lookup(store, user_id).await? else {
            return Ok(None);
        };
        let (id, character) = (account.id.clone(), character_id.to_owned());
        let record = store
            .read(move |conn, repos| repos.characters.find(conn, &character, &id))
            .await?;
        Ok(record.map(|record| (account, record)))
    }

    /// `GetCharacterWithUid`: the user's character, wired; another account's is none.
    pub async fn character_for_user(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        user_id: &str,
        character_id: &str,
    ) -> Result<Option<Value>, CharactersError> {
        let Some((account, record)) = self.one(store, user_id, character_id).await? else {
            return Ok(None);
        };
        let dojo = self.dojo_flag(store, &account.id).await;
        self.wire(&record, user_id, tables, dojo, now_secs()).map(Some)
    }

    /// `UpdateCharacterForUid` (spec §5.6): whether the save is accepted.
    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is one part of the save, as spec §5.6 lists them"
    )]
    pub(crate) async fn update_character(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        actor: &ActorContext,
        user_id: &str,
        character_id: &str,
        body_data: Option<&Value>,
        body_version: Option<&Value>,
        gameserver: bool,
    ) -> Result<bool, CharactersError> {
        let Some((account, record)) = self.one(store, user_id, character_id).await? else {
            tracing::warn!(
                "UpdateCharacterForUid: no character record for characterId {character_id:?} userId {user_id:?} \
                 (isGameserver={gameserver}) - treating as a rejected write instead of throwing"
            );
            return Ok(false);
        };
        let now = now_secs();
        let dojo = self.dojo_flag(store, &account.id).await;
        let current_data = self.read_form(&record.data, tables, dojo, now)?;
        let current = record.update_version;
        let requested = js::number(body_version);
        let mut effective = js::safe_integer(Some(&Value::from(requested))).filter(|_| requested.is_finite());
        // NaN compares false, so it is never stale
        #[allow(clippy::cast_precision_loss, reason = "a stored counter, far below 2^53")]
        let stale = (current as f64) >= requested;
        if stale {
            if !gameserver {
                return Ok(false);
            }
            let forced = current.saturating_add(1);
            tracing::info!(
                "[gameserver-force] Stale gameserver write for characterId {character_id:?} - was updateVersion \
                 {}, forcing to {forced} (server is authoritative for progress/PJM)",
                jsjson::number(requested)
            );
            effective = Some(forced);
        }
        let incoming = match body_data {
            Some(Value::String(text)) if !text.is_empty() => text.clone(),
            other if js::truthy(other) => js::string(other),
            _ => "{}".to_owned(),
        };
        let guarded = data::guard_progression(&incoming, &current_data);
        let mut next = data::process_triggers(&guarded, &self.data, tables.target_changelist, now)?;
        let stored_series = data_member(&current_data, &self.data.dojo_series_key);
        if stored_series {
            let (kept, added) = data::read_form(&next, &self.data, tables.target_changelist, now, true)?;
            if added {
                tracing::info!(
                    "[dojo-reconcile] Persisting SERIE_dojo for userId {user_id:?} characterId {character_id:?}"
                );
            }
            next = kept;
        }
        let Some(effective) = effective else {
            return Ok(true);
        };
        let (id, character) = (account.id.clone(), character_id.to_owned());
        store
            .account_write(&account.id, actor, move |tx, repos| {
                repos
                    .characters
                    .update_data_conditional(tx, &character, &id, &next, effective)
            })
            .await?;
        Ok(true)
    }

    /// Whether the in-transaction check would write anything (spec §7.2 "nothing to repair", negated).
    pub fn needs_repair(
        &self,
        inventory: Option<&InventoryRecord>,
        loadout: Option<&LoadoutRecord>,
        tables: &VersionTables,
    ) -> bool {
        let (Some(inventory), Some(_)) = (inventory, loadout) else {
            return true;
        };
        let items = starter::parse_items(&inventory.instanced_items);
        let stacked = starter::parse_items(&inventory.stacked_items);
        !starter::missing_parts(&items, &self.kit).is_empty() || !starter::missing_stacked(&stacked, tables).is_empty()
    }

    /// `EnsureStarterBootstrapRecordsInTransaction` (spec §7.2): the ownership gate, then the records made, repaired
    /// or reconciled; both as they now are.
    pub fn ensure_starter_in(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        tables: &VersionTables,
        account: &AccountId,
        user_id: &str,
        character_id: &str,
    ) -> Result<(InventoryRecord, LoadoutRecord), EnsureError> {
        if repos.characters.find(tx.conn(), character_id, account)?.is_none() {
            return Err(EnsureError::NotOwned {
                character_id: character_id.to_owned(),
                user: user_id.to_owned(),
            });
        }
        let inventory = repos.inventories.find_by_character_id(tx.conn(), character_id)?;
        let loadout = repos.loadouts.find(tx.conn(), character_id, account)?;
        let bootstrap = Some(tables.starter_manifest.bootstrap_version.to_owned());
        match (inventory, loadout) {
            (Some(inventory), Some(loadout)) => {
                let fixed = starter::reconcile(&inventory.instanced_items, &inventory.stacked_items, &self.kit, tables);
                if fixed.added_parts.is_empty() && fixed.added_stacked.is_empty() {
                    return Ok((inventory, loadout));
                }
                let updated = repos.inventories.update_both_if_revision_matches(
                    tx,
                    character_id,
                    &jsjson::stringify(&Value::Array(fixed.instanced)),
                    &jsjson::stringify(&Value::Array(fixed.stacked)),
                    inventory.revision.unwrap_or(0),
                )?;
                let Some(updated) = updated else {
                    let text = format!("Default weapon-part reconciliation conflict for inventory {character_id}");
                    return Err(RepoError::Data(text).into());
                };
                if !fixed.added_parts.is_empty() {
                    tracing::info!(
                        "[inventory] reconciled missing default weapon parts [{}] for characterId {character_id} \
                         (userId {user_id:?})",
                        fixed.added_parts.join(", ")
                    );
                }
                if !fixed.added_stacked.is_empty() {
                    tracing::info!(
                        "[inventory] reconciled missing starter stacked items [{}] for characterId {character_id} \
                         (userId {user_id:?})",
                        fixed.added_stacked.join(", ")
                    );
                }
                Ok((updated, loadout))
            }
            (None, None) => {
                self.seed(tx, repos, tables, account, character_id, false)?;
                self.reread(tx, repos, account, character_id)
            }
            (None, Some(loadout)) => {
                let source = match jsjson::parse(&loadout.loadouts) {
                    Ok(Value::Array(items)) => items.into_iter().next(),
                    _ => None,
                };
                let repaired = starter::repair(Vec::new(), source.as_ref(), &self.kit);
                let record = InventoryRecord {
                    character_id: character_id.to_owned(),
                    account: Some(account.clone()),
                    instanced_items: jsjson::stringify(&Value::Array(repaired.items)),
                    stacked_items: jsjson::stringify(&Value::Array(starter::stacked_items(tables))),
                    revision: Some(0),
                    bootstrap_version: bootstrap,
                };
                repos.inventories.create(tx, &record)?;
                self.reread(tx, repos, account, character_id)
            }
            (Some(inventory), None) => {
                let items = starter::parse_items(&inventory.instanced_items);
                let repaired = starter::repair(items, None, &self.kit);
                if repaired.changed {
                    let updated = repos.inventories.update_both_if_revision_matches(
                        tx,
                        character_id,
                        &jsjson::stringify(&Value::Array(repaired.items.clone())),
                        &inventory.stacked_items,
                        inventory.revision.unwrap_or(0),
                    )?;
                    if updated.is_none() {
                        let text = format!("Starter recovery conflict for inventory {character_id}");
                        return Err(RepoError::Data(text).into());
                    }
                }
                let persistent = starter::persistent_for(&self.kit, tables, &repaired.ids)?;
                let slot = starter::slot_for(&self.kit, &repaired.ids, persistent.clone())?;
                let record = LoadoutRecord {
                    character_id: character_id.to_owned(),
                    account: account.clone(),
                    loadouts: jsjson::stringify(&Value::Array(vec![slot])),
                    persistent: jsjson::stringify(&persistent),
                    active_index: None,
                    unlocked_total_slots: None,
                    revision: Some(0),
                    bootstrap_version: bootstrap,
                };
                repos.loadouts.create(tx, &record)?;
                self.reread(tx, repos, account, character_id)
            }
        }
    }

    fn reread(
        &self,
        tx: &mut Tx<'_>,
        repos: &Repositories,
        account: &AccountId,
        character_id: &str,
    ) -> Result<(InventoryRecord, LoadoutRecord), EnsureError> {
        let inventory = repos.inventories.find_by_character_id(tx.conn(), character_id)?;
        let loadout = repos.loadouts.find(tx.conn(), character_id, account)?;
        match (inventory, loadout) {
            (Some(inventory), Some(loadout)) => Ok((inventory, loadout)),
            _ => Err(RepoError::Data(format!("starter records for {character_id} vanished")).into()),
        }
    }

    /// The outside-transaction form, with the account it found.
    async fn ensure(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        actor: &ActorContext,
        user_id: &str,
        character_id: &str,
    ) -> Result<(AccountRecord, InventoryRecord, LoadoutRecord), CharactersError> {
        let Some(account) = lookup(store, user_id).await? else {
            return Err(CharactersError::NotOwned);
        };
        let (id, character) = (account.id.clone(), character_id.to_owned());
        let found = store
            .read(move |conn, repos| {
                if repos.characters.find(conn, &character, &id)?.is_none() {
                    return Ok(None);
                }
                let inventory = repos.inventories.find_by_character_id(conn, &character)?;
                Ok(Some((inventory, repos.loadouts.find(conn, &character, &id)?)))
            })
            .await?;
        let Some((inventory, loadout)) = found else {
            return Err(CharactersError::NotOwned);
        };
        if let (Some(inventory), Some(loadout)) = (&inventory, &loadout)
            && !self.needs_repair(Some(inventory), Some(loadout), tables)
        {
            return Ok((account, inventory.clone(), loadout.clone()));
        }
        let (me, tables_copy, id) = (self.clone(), *tables, account.id.clone());
        let (user, character) = (user_id.to_owned(), character_id.to_owned());
        let records = store
            .account_write(&account.id, actor, move |tx, repos| {
                match me.ensure_starter_in(tx, repos, &tables_copy, &id, &user, &character) {
                    Ok(records) => Ok(Some(records)),
                    Err(EnsureError::NotOwned { .. }) => Ok(None),
                    Err(EnsureError::Repo(e)) => Err(e),
                }
            })
            .await?;
        let (inventory, loadout) = records.ok_or(CharactersError::NotOwned)?;
        Ok((account, inventory, loadout))
    }

    /// `EnsureStarterBootstrapRecords` outside a transaction (what the loadout routes call).
    pub async fn ensure_starter(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        actor: &ActorContext,
        user_id: &str,
        character_id: &str,
    ) -> Result<(InventoryRecord, LoadoutRecord), CharactersError> {
        let (_, inventory, loadout) = self.ensure(store, tables, actor, user_id, character_id).await?;
        Ok((inventory, loadout))
    }

    fn state_of(&self, row: &LoadoutRecord, tables: &VersionTables) -> Result<LoadoutState, CharactersError> {
        let (mut items, visible) = stored_slots(row)?;
        items.truncate(usize::try_from(visible).unwrap_or(usize::MAX));
        let persistent = jsjson::parse(&row.persistent)
            .map_err(|e| failed(format!("Persisted loadout persistent is invalid: {e}")))?;
        let active =
            slots::resolve_active_index(row.active_index.map(i64::from), visible).map_err(|e| failed(e.to_string()))?;
        Ok(LoadoutState {
            loadouts: items,
            persistent: starter::with_default_chat_wheels(persistent, &self.kit, tables),
            active_index: active,
        })
    }

    /// `GetLoadoutStateForUserIdAndCharacterId` (spec §5.9).
    pub async fn loadout_state(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        actor: &ActorContext,
        user_id: &str,
        character_id: &str,
    ) -> Result<LoadoutState, CharactersError> {
        let (_, _, row) = self.ensure(store, tables, actor, user_id, character_id).await?;
        self.state_of(&row, tables)
    }

    /// `EnsureTotalLoadoutSlots` (spec §5.10): grows the stored slots to the entitlement; never relocks one.
    pub(crate) async fn ensure_total_slots(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        actor: &ActorContext,
        user_id: &str,
        character_id: &str,
        desired: u32,
    ) -> Result<(), CharactersError> {
        if !(1..=slots::MAX_TOTAL_LOADOUT_SLOTS).contains(&desired) {
            return Err(failed("Total loadout slot count must be between 1 and 6"));
        }
        let (account, _, _) = self.ensure(store, tables, actor, user_id, character_id).await?;
        let (id, character) = (account.id.clone(), character_id.to_owned());
        store
            .account_write(&account.id, actor, move |tx, repos| {
                for attempt in 1..=ATTEMPTS {
                    let Some(row) = repos.loadouts.find(tx.conn(), &character, &id)? else {
                        return Ok(Err(failed(format!(
                            "Bootstrap recovery did not create loadout for {character}"
                        ))));
                    };
                    let mut items = match stored_slots(&row) {
                        Ok((items, _)) => items,
                        Err(e) => return Ok(Err(e)),
                    };
                    let stored = row.unlocked_total_slots.map(i64::from);
                    let target = match slots::resolve_requested_total(stored, i64::from(desired)) {
                        Ok(target) => target,
                        Err(e) => return Ok(Err(failed(e.to_string()))),
                    };
                    let before = items.len();
                    while items.len() < usize::try_from(target).unwrap_or(0) {
                        let copy = copy_slot(&items[0], items.len());
                        items.push(copy);
                    }
                    if items.len() == before && stored == Some(i64::from(target)) {
                        return Ok(Ok(()));
                    }
                    let text = jsjson::stringify(&Value::Array(items));
                    let revision = row.revision.unwrap_or(0);
                    if repos
                        .loadouts
                        .replace_all_and_entitlement_if_revision_matches(tx, &character, &id, &text, target, revision)?
                    {
                        return Ok(Ok(()));
                    }
                    tracing::warn!(
                        "Loadout unlock revision conflict for characterId {character}, attempt {attempt}/{ATTEMPTS} \
                         - retrying"
                    );
                }
                Ok(Err(failed(format!(
                    "Loadout unlock for characterId {character} failed after {ATTEMPTS} revision-conflict retries"
                ))))
            })
            .await?
    }

    /// `SetActiveLoadoutIndexForUserIdAndCharacterId` (spec §5.11).
    pub(crate) async fn set_active_index(
        &self,
        store: &IslesStore,
        tables: &VersionTables,
        actor: &ActorContext,
        user_id: &str,
        character_id: &str,
        index: u32,
    ) -> Result<Write, CharactersError> {
        if index >= slots::MAX_TOTAL_LOADOUT_SLOTS {
            return Ok(Write::Refused);
        }
        let (account, _, _) = self.ensure(store, tables, actor, user_id, character_id).await?;
        let (id, character) = (account.id.clone(), character_id.to_owned());
        store
            .account_write(&account.id, actor, move |tx, repos| {
                for attempt in 1..=ATTEMPTS {
                    let Some(row) = repos.loadouts.find(tx.conn(), &character, &id)? else {
                        return Ok(Ok(Write::Refused));
                    };
                    let visible = match stored_slots(&row) {
                        Ok((_, visible)) => visible,
                        Err(e) => return Ok(Err(e)),
                    };
                    if index >= visible {
                        return Ok(Ok(Write::Refused));
                    }
                    let revision = row.revision.unwrap_or(0);
                    if repos
                        .loadouts
                        .update_active_index_if_revision_matches(tx, &character, &id, index, revision)?
                    {
                        return Ok(Ok(Write::Accepted));
                    }
                    tracing::warn!(
                        "Active loadout revision conflict for characterId {character} index {index}, attempt \
                         {attempt}/{ATTEMPTS} - retrying"
                    );
                }
                tracing::error!(
                    "Active loadout write for characterId {character} index {index} failed after {ATTEMPTS} attempts"
                );
                Ok(Ok(Write::Refused))
            })
            .await?
    }

    /// `SetLoadoutDataForUserIdAndCharacterId` (spec §5.12); the starter records are not ensured first.
    pub(crate) async fn set_loadout_data(
        &self,
        store: &IslesStore,
        actor: &ActorContext,
        user_id: &str,
        character_id: &str,
        index: &str,
        body_data: Option<&Value>,
    ) -> Result<Write, CharactersError> {
        if let Some(reason) = data::validate_loadout_write(index, body_data) {
            tracing::error!("Loadout write rejected for characterId {character_id:?}: {reason}");
            return Ok(Write::Refused);
        }
        let Some(text) = body_data.and_then(Value::as_str).map(str::to_owned) else {
            return Ok(Write::Refused);
        };
        let Some(account) = lookup(store, user_id).await? else {
            return Ok(Write::Refused);
        };
        let slot = index.parse::<u32>().ok();
        let (id, character, shown) = (account.id.clone(), character_id.to_owned(), index.to_owned());
        let outcome = store
            .account_write(&account.id, actor, move |tx, repos| {
                for attempt in 1..=ATTEMPTS {
                    let Some(row) = repos.loadouts.find(tx.conn(), &character, &id)? else {
                        return Ok(Write::Refused);
                    };
                    let revision = row.revision.unwrap_or(0);
                    let applied = match slot {
                        None => repos
                            .loadouts
                            .update_persistent_if_revision_matches(tx, &character, &id, &text, revision)?,
                        Some(slot) => repos
                            .loadouts
                            .replace_slot_if_revision_matches(tx, &character, &id, slot, &text, revision)?,
                    };
                    if applied {
                        return Ok(Write::Accepted);
                    }
                    tracing::warn!(
                        "Loadout revision conflict for characterId {character:?} index {shown}, attempt \
                         {attempt}/{ATTEMPTS} - retrying"
                    );
                }
                tracing::error!(
                    "Loadout write for characterId {character:?} index {shown} failed after {ATTEMPTS} \
                     revision-conflict retries"
                );
                Ok(Write::Refused)
            })
            .await?;
        Ok(outcome)
    }
}

/// Whether the object text has a non-null `key`.
fn data_member(text: &str, key: &str) -> bool {
    match jsjson::parse(text) {
        Ok(Value::Object(map)) => map.get(key).is_some_and(|v| !v.is_null()),
        _ => false,
    }
}

/// The unlock entitlement from the target's Slayer's Path: 1 plus the loadout nodes it unlocked.
pub(crate) async fn journey_total(store: &IslesStore, target: &str) -> RepoResult<u32> {
    if target.is_empty() || target == INVALID {
        return Ok(slots::resolve_player_journey_total(None));
    }
    let Some(account) = lookup(store, target).await? else {
        return Ok(slots::resolve_player_journey_total(None));
    };
    let id = account.id.clone();
    let row = store
        .read(move |conn, repos| repos.player_journey.find(conn, &id))
        .await?;
    let nodes = row.map(|row| jsjson::parse(&row.nodes).unwrap_or_else(|_| json!({})));
    Ok(slots::resolve_player_journey_total(nodes.as_ref()))
}
