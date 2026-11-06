// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The module's admin actions (admin spec §10, core §7.6) and home operations (§11, core §7.8, DN5).

use std::sync::Arc;

use module_api::network::HomeInvocation;
use module_api::{
    Actor, ActorContext, AdminCall, AdminRequest, AdminResponse, ModuleError, PlayerInfo, PlayerRestriction,
    RestrictionKind,
};
use rustic_isles::{Shared, UserId};
use rustic_isles_store::repo::auth::{AccessChange, AccountTimeout, LauncherAccountRecord, OperationalStatus};
use rustic_isles_store::repo::bounties::{BountyGroup, BountyStateRecord, DraftData, StoredBounty};
use rustic_isles_store::repo::characters::CharacterRecord;
use rustic_isles_store::repo::inventory::InventoryRecord;
use rustic_isles_store::{AccountId, AnyConnection, IslesStore, RepoError, Repositories};
use serde_json::{Map, Value};

use super::ReforgedAdmin;
use crate::state::ReforgedState;
use crate::util::{self, MINUTE_MS, iso, now_ms, object};

fn failed(error: RepoError) -> ModuleError {
    ModuleError::failed(error.to_string())
}

fn no_store() -> ModuleError {
    ModuleError::new(module_api::ErrorKind::Unavailable, "the module has no database")
}

/// The player an admin text names.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The game account.
    pub account: AccountId,
    /// The game account's userId, else the account id text.
    pub user_id: String,
    /// The display name, else the game name, else the userId.
    pub name: String,
    /// The launcher account, if the player has one.
    pub launcher: Option<LauncherAccountRecord>,
}

/// A global id (game account, else launcher account), else a userId, else a launcher name in any case.
pub fn resolve_player(
    conn: &mut AnyConnection,
    repos: &Repositories,
    text: &str,
) -> Result<Option<Resolved>, RepoError> {
    let text = util::trim(text);
    if text.is_empty() {
        return Ok(None);
    }
    if let Ok(account) = text.parse::<AccountId>() {
        if let Some(game) = repos.accounts.find(conn, &account)? {
            let launcher = repos.launcher_accounts.find_by_account(conn, &account)?;
            return Ok(Some(Resolved {
                account,
                user_id: game.user_id.as_str().to_owned(),
                name: game.name,
                launcher,
            }));
        }
        if let Some(launcher) = repos.launcher_accounts.find_by_account(conn, &account)? {
            return Ok(Some(Resolved {
                user_id: account.to_string(),
                account,
                name: launcher.display_name.clone(),
                launcher: Some(launcher),
            }));
        }
    }
    if let Some(game) = repos.accounts.find_by_user_id(conn, &UserId::new(text))? {
        let launcher = repos.launcher_accounts.find_by_account(conn, &game.id)?;
        return Ok(Some(Resolved {
            account: game.id,
            user_id: game.user_id.as_str().to_owned(),
            name: game.name,
            launcher,
        }));
    }
    if let Some(launcher) = repos
        .launcher_accounts
        .find_by_display_name_normalized(conn, &text.to_lowercase())?
    {
        let user_id = super::user_id_of(conn, repos, &launcher.account);
        return Ok(Some(Resolved {
            account: launcher.account.clone(),
            user_id,
            name: launcher.display_name.clone(),
            launcher: Some(launcher),
        }));
    }
    Ok(None)
}

fn reason_of(reason: Option<&String>) -> Option<String> {
    reason.map(|r| util::take_chars(r, 500)).filter(|r| !r.is_empty())
}

fn restriction(launcher: Option<&LauncherAccountRecord>, now: &str) -> Option<PlayerRestriction> {
    let launcher = launcher?;
    let stored = launcher.approval_reason.clone().filter(|r| !r.is_empty());
    match launcher.status {
        OperationalStatus::Banned | OperationalStatus::Disabled => Some(PlayerRestriction {
            kind: if launcher.status == OperationalStatus::Banned {
                RestrictionKind::Ban
            } else {
                RestrictionKind::Disabled
            },
            reason: stored,
            since: launcher.approval_updated_at.clone(),
            until: None,
        }),
        OperationalStatus::Active => launcher.timeout_at(now).map(|t| PlayerRestriction {
            kind: RestrictionKind::Timeout,
            reason: t.reason.clone(),
            since: Some(t.since.clone()),
            until: Some(t.until.clone()),
        }),
    }
}

impl ReforgedAdmin {
    /// An admin action whose `Ban` `until` core already checked; each effect is its own write.
    pub async fn admin_action(&self, shared: &Arc<Shared>, call: AdminCall) -> Result<AdminResponse, ModuleError> {
        let store = util::store_of(shared).ok_or_else(no_store)?;
        let player = match &call.request {
            AdminRequest::Kick { player, .. }
            | AdminRequest::Ban { player, .. }
            | AdminRequest::Unban { player, .. }
            | AdminRequest::LiftTimeout { player, .. }
            | AdminRequest::Lookup { player } => player.clone(),
        };
        let found = store
            .read(move |conn, repos| resolve_player(conn, repos, &player))
            .await
            .map_err(failed)?;
        let Some(target) = found else {
            return Ok(AdminResponse::NotFound);
        };
        let state = ReforgedState::of(shared);
        let actor = call.actor;
        match call.request {
            AdminRequest::Kick { .. } => {
                kick(shared, &store, &actor, &target, "kicked by the host").await?;
            }
            AdminRequest::Ban {
                until: Some(until),
                reason,
                ..
            } => {
                let timeout = AccountTimeout {
                    until,
                    since: iso(now_ms()),
                    reason: reason_of(reason.as_ref()),
                };
                set_timeout(&store, &actor, &target.account, Some(timeout)).await?;
                state.forget_status();
                kick(shared, &store, &actor, &target, "timed out by the host").await?;
            }
            AdminRequest::Ban {
                until: None, reason, ..
            } => {
                set_status(
                    &store,
                    &actor,
                    &target.account,
                    OperationalStatus::Banned,
                    reason_of(reason.as_ref()),
                )
                .await?;
                state.forget_status();
                set_timeout(&store, &actor, &target.account, None).await?;
                state.forget_status();
                let account = target.account.clone();
                store
                    .account_write(&target.account, &actor, move |tx, repos| {
                        let now = iso(now_ms());
                        repos.refresh_sessions.revoke_all_for_account(tx, &account, &now)?;
                        repos.game_exchange_codes.revoke_unused_for_account(tx, &account, &now)
                    })
                    .await
                    .map_err(failed)?;
                shared
                    .presence()
                    .close_connections(&target.user_id, 1008, "banned by the host");
                state.forget_status();
            }
            AdminRequest::Unban { reason, .. } => {
                set_status(
                    &store,
                    &actor,
                    &target.account,
                    OperationalStatus::Active,
                    reason_of(reason.as_ref()),
                )
                .await?;
                state.forget_status();
            }
            AdminRequest::LiftTimeout { .. } => {
                set_timeout(&store, &actor, &target.account, None).await?;
                state.forget_status();
            }
            AdminRequest::Lookup { .. } => {
                let online = state.active_players(shared, 3 * MINUTE_MS).contains(&target.user_id);
                return Ok(AdminResponse::Player(PlayerInfo {
                    account: Some(target.account.global().clone()),
                    name: target.name.clone(),
                    online,
                    restriction: restriction(target.launcher.as_ref(), &iso(now_ms())),
                }));
            }
        }
        Ok(AdminResponse::Done)
    }

    /// A home operation for an account homed here; `None` is one this module does not have.
    pub async fn home_operation(
        &self,
        shared: &Arc<Shared>,
        name: &str,
        account: &AccountId,
        actor: ActorContext,
        body: Value,
    ) -> Result<Option<Value>, ModuleError> {
        if name.starts_with("chat.") {
            let Some(chat) = self.chat() else {
                return Ok(None);
            };
            let Ok(operation) = module_api::network::OperationName::new(name) else {
                return Ok(None);
            };
            let invocation = HomeInvocation {
                from: shared.node().node.clone(),
                account: account.global().clone(),
                actor,
                operation,
                body,
            };
            return rustic_isles_chat::home_operation(name, chat, &invocation)
                .await
                .transpose();
        }
        let store = util::store_of(shared).ok_or_else(no_store)?;
        // Home operations write as the account itself, whatever the invocation's actor (§19 item 15)
        let player = ActorContext::new(Actor::Player {
            account: account.global().clone(),
        });
        let text = account.to_string();
        let a = account.clone();
        let answer = match name {
            "progression.grant" => {
                let xp = body.get("xp").and_then(Value::as_i64).unwrap_or(0).max(0);
                let track = body
                    .get("track")
                    .and_then(Value::as_str)
                    .unwrap_or("ExperienceTrack_PlayerLevel")
                    .to_owned();
                let t = track.clone();
                let after = store
                    .account_write(account, &player, move |tx, repos| {
                        repos.progression_tracks.increment(tx, &a, &t, xp)
                    })
                    .await
                    .map_err(failed)?;
                object(vec![
                    ("ok", Value::Bool(true)),
                    ("account", Value::from(text)),
                    ("progression_id", Value::from(track)),
                    ("progress", Value::from(after.progress)),
                ])
            }
            "progression.read" => {
                let tracks = store
                    .read(move |conn, repos| repos.progression_tracks.get_all_for_account(conn, &a))
                    .await
                    .map_err(failed)?;
                let list: Vec<Value> = tracks
                    .into_iter()
                    .map(|t| {
                        object(vec![
                            ("progression_id", Value::from(t.progression_id)),
                            ("progress", Value::from(t.progress)),
                            ("update_version", Value::from(t.update_version)),
                        ])
                    })
                    .collect();
                object(vec![("tracks", Value::Array(list))])
            }
            "inventory.write" => {
                let loot: Vec<String> = body
                    .get("loot")
                    .and_then(Value::as_array)
                    .map(|l| l.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
                let applied = loot.len();
                store
                    .account_write(account, &player, move |tx, repos| write_loot(tx, repos, &a, &loot))
                    .await
                    .map_err(failed)?;
                object(vec![
                    ("ok", Value::Bool(true)),
                    ("account", Value::from(text)),
                    ("loot_applied", Value::from(applied)),
                ])
            }
            "inventory.read" => store
                .read(move |conn, repos| {
                    let characters = repos.characters.find_many_by_account(conn, &a)?;
                    let Some(first) = characters.into_iter().next() else {
                        return Ok(None);
                    };
                    let inventory = repos.inventories.find_by_character_id(conn, &first.character_id)?;
                    Ok(Some((first.character_id, inventory)))
                })
                .await
                .map_err(failed)?
                .map_or_else(
                    || {
                        object(vec![
                            ("stacked_items", Value::Array(Vec::new())),
                            ("instanced_items", Value::Array(Vec::new())),
                        ])
                    },
                    |(character, inventory)| {
                        let parse = |text: Option<&str>| {
                            text.and_then(|t| serde_json::from_str::<Value>(t).ok())
                                .filter(Value::is_array)
                                .unwrap_or(Value::Array(Vec::new()))
                        };
                        object(vec![
                            ("character_id", Value::from(character)),
                            (
                                "stacked_items",
                                parse(inventory.as_ref().map(|i| i.stacked_items.as_str())),
                            ),
                            (
                                "instanced_items",
                                parse(inventory.as_ref().map(|i| i.instanced_items.as_str())),
                            ),
                        ])
                    },
                ),
            "bounty.write" => {
                let progress = body
                    .get("bounty_progress")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                let version = store
                    .account_write(account, &player, move |tx, repos| {
                        let now = iso(now_ms());
                        let stored = repos.bounty_states.get(tx.conn(), &a)?;
                        let expected = stored.as_ref().map(|s| s.update_version);
                        let mut record = stored.unwrap_or_else(|| BountyStateRecord {
                            account: a.clone(),
                            bounties: Vec::new(),
                            draft_data: DraftData::default(),
                            daily_reward_credited_on: None,
                            daily_premium_draft_granted_on: None,
                            update_version: 0,
                            created_at: now.clone(),
                            updated_at: now.clone(),
                        });
                        record.bounties.push(StoredBounty {
                            group: BountyGroup::Base,
                            bounty: progress,
                            claimed_at: None,
                            reward_credit: None,
                        });
                        record.update_version = expected.map_or(1, |v| v + 1);
                        record.daily_reward_credited_on = None;
                        record.daily_premium_draft_granted_on = None;
                        record.updated_at = now;
                        let version = record.update_version;
                        // A lost race is ignored (§19 item 15)
                        let _ = repos.bounty_states.save_if_version(tx, &record, expected)?;
                        Ok(version)
                    })
                    .await
                    .map_err(failed)?;
                object(vec![
                    ("ok", Value::Bool(true)),
                    ("account", Value::from(text)),
                    ("update_version", Value::from(version)),
                ])
            }
            "bounty.read" => {
                let stored = store
                    .read(move |conn, repos| repos.bounty_states.get(conn, &a))
                    .await
                    .map_err(failed)?;
                let list: Vec<Value> = stored
                    .map(|s| s.bounties.into_iter().map(|b| b.bounty).collect())
                    .unwrap_or_default();
                object(vec![
                    ("count", Value::from(list.len())),
                    ("bounties", Value::Array(list)),
                ])
            }
            "character.load" | "account.write" => object(vec![
                ("ok", Value::Bool(true)),
                ("account", Value::from(text)),
                ("operation", Value::from(name)),
            ]),
            _ => return Ok(None),
        };
        Ok(Some(answer))
    }
}

fn write_loot(
    tx: &mut rustic_isles_store::Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    loot: &[String],
) -> Result<(), RepoError> {
    let now = iso(now_ms());
    let character = match repos
        .characters
        .find_many_by_account(tx.conn(), account)?
        .into_iter()
        .next()
    {
        Some(c) => c,
        None => repos.characters.create(
            tx,
            &CharacterRecord {
                character_id: format!("char-{account}"),
                account: account.clone(),
                created_date: now.clone(),
                last_modified_date: now.clone(),
                name: "Slayer".to_owned(),
                update_version: 1,
                data: "{}".to_owned(),
            },
        )?,
    };
    let existing = repos
        .inventories
        .find_by_character_id(tx.conn(), &character.character_id)?;
    let mut stacked: Vec<Value> = existing
        .as_ref()
        .and_then(|i| serde_json::from_str::<Value>(&i.stacked_items).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    for entry in loot {
        let (catalog, quantity) = match entry.split_once(':') {
            Some((catalog, qty)) => (catalog, qty.trim().parse::<i64>().unwrap_or(1).max(1)),
            None => (entry.as_str(), 1),
        };
        let found = stacked
            .iter_mut()
            .find(|s| s.get("catalogId").and_then(Value::as_str) == Some(catalog));
        match found {
            Some(stack) => {
                let before = stack.get("quantity").and_then(Value::as_i64).unwrap_or(0);
                stack["quantity"] = Value::from(before + quantity);
            }
            None => stacked.push(object(vec![
                ("catalogId", Value::from(catalog)),
                ("quantity", Value::from(quantity)),
            ])),
        }
    }
    let stacked_text = service_kit::jsjson::stringify(&Value::Array(stacked));
    match existing {
        None => repos.inventories.create(
            tx,
            &InventoryRecord {
                character_id: character.character_id,
                account: Some(account.clone()),
                instanced_items: "[]".to_owned(),
                stacked_items: stacked_text,
                revision: Some(1),
                bootstrap_version: None,
            },
        ),
        Some(inventory) => {
            // A lost revision race is ignored (§19 item 15)
            let _ = repos.inventories.update_both_if_revision_matches(
                tx,
                &inventory.character_id,
                &inventory.instanced_items,
                &stacked_text,
                inventory.revision.unwrap_or(0),
            )?;
            Ok(())
        }
    }
}

async fn kick(
    shared: &Shared,
    store: &IslesStore,
    actor: &ActorContext,
    target: &Resolved,
    reason: &str,
) -> Result<(), ModuleError> {
    let account = target.account.clone();
    store
        .account_write(&target.account, actor, move |tx, repos| {
            repos
                .game_exchange_codes
                .revoke_unused_for_account(tx, &account, &iso(now_ms()))
        })
        .await
        .map_err(failed)?;
    shared.presence().close_connections(&target.user_id, 1008, reason);
    ReforgedState::of(shared).forget_status();
    Ok(())
}

async fn set_timeout(
    store: &IslesStore,
    actor: &ActorContext,
    account: &AccountId,
    timeout: Option<AccountTimeout>,
) -> Result<(), ModuleError> {
    let a = account.clone();
    store
        .account_write(account, actor, move |tx, repos| {
            repos
                .launcher_accounts
                .set_timeout(tx, &a, timeout.as_ref())
                .map(|_| ())
        })
        .await
        .map_err(failed)
}

async fn set_status(
    store: &IslesStore,
    actor: &ActorContext,
    account: &AccountId,
    status: OperationalStatus,
    reason: Option<String>,
) -> Result<(), ModuleError> {
    let a = account.clone();
    store
        .account_write(account, actor, move |tx, repos| {
            let change = AccessChange {
                approval_status: None,
                status: Some(status),
                approval_updated_at: iso(now_ms()),
                approval_updated_by: "host".to_owned(),
                approval_reason: reason,
            };
            repos.launcher_accounts.set_access_state(tx, &a, &change).map(|_| ())
        })
        .await
        .map_err(failed)
}

/// The module hooks over this half: admin actions and home operations (the rest keep core's defaults).
pub struct ReforgedHooks(pub Arc<ReforgedAdmin>);

impl rustic_isles::ModuleHooks for ReforgedHooks {
    fn admin<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        call: AdminCall,
    ) -> module_api::BoxFuture<'a, Result<AdminResponse, ModuleError>> {
        Box::pin(self.0.admin_action(shared, call))
    }

    fn home_operation<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        operation: &'a str,
        account: &'a AccountId,
        actor: ActorContext,
        body: Value,
    ) -> module_api::BoxFuture<'a, Result<Option<Value>, ModuleError>> {
        Box::pin(self.0.home_operation(shared, operation, account, actor, body))
    }
}
