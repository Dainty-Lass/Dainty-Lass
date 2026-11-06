// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The answer to each `ChatQuery` for 1.14.7 (spec §5.2-§5.8). Each query is one method with its default, so
//! another version can override one and keep the rest.

use module_api::chat::{ChatAnswer, ChatQuery, Jid, RoomKind, RoomRules};
use module_api::{BoxFuture, ErrorKind, GlobalId, IdKind, ModuleError, NodeIdentity, Region};
use rustic_isles::replay;

use crate::hubs::HubSessions;
use crate::ids::instance_id;
use crate::{ChatSource, Player, Refusal, XMPP_DOMAIN, authenticate};

/// History is never kept: Paradox's rooms were in memory, so a joiner sees only what is said after joining.
const OPEN: RoomRules = RoomRules {
    may_join: true,
    may_speak: true,
    history: false,
};
const REFUSED: RoomRules = RoomRules {
    may_join: false,
    may_speak: false,
    history: false,
};

fn open_if(open: bool) -> RoomRules {
    if open { OPEN } else { REFUSED }
}

pub trait ChatRules: Send + Sync {
    fn answer<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        hubs: &'a HubSessions,
        query: &'a ChatQuery,
    ) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>> {
        Box::pin(async move {
            match query {
                ChatQuery::Jid { account } => self.jid(source, account).await.map(ChatAnswer::Jid),
                ChatQuery::Room { room, account } => self.room(source, hubs, room, account).await.map(ChatAnswer::Room),
                ChatQuery::Login { authcid, token } => {
                    self.login(source, authcid, token).await.map(|p| ChatAnswer::Login {
                        account: p.account,
                        name: p.name,
                    })
                }
                ChatQuery::Friends { account } => self.friends(source, account).await.map(ChatAnswer::Friends),
                ChatQuery::MayWhisper { from, to } => Ok(ChatAnswer::MayWhisper(self.may_whisper(from, to))),
                ChatQuery::RoomOf { name } => Ok(ChatAnswer::RoomOf(self.room_of(source, hubs, name))),
                ChatQuery::AccountOf { local } => self.account_of(source, local).await.map(ChatAnswer::AccountOf),
            }
        })
    }

    /// `<userId>@prod.ol.epicgames.com`, no resource (§5.2).
    fn jid<'a>(&'a self, source: &'a dyn ChatSource, account: &'a GlobalId) -> BoxFuture<'a, Result<Jid, ModuleError>> {
        Box::pin(async move {
            let player = source
                .player_by_account(account)
                .await?
                .ok_or_else(|| ModuleError::new(ErrorKind::NotFound, format!("no account {account}")))?;
            Jid::new(&player.user_id, XMPP_DOMAIN, None)
                .map_err(|e| ModuleError::new(ErrorKind::Invalid, format!("the userId is not a JID local part ({e})")))
        })
    }

    /// §5.4; the account is not checked for existence.
    fn room<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        hubs: &'a HubSessions,
        room: &'a RoomKind,
        account: &'a GlobalId,
    ) -> BoxFuture<'a, Result<RoomRules, ModuleError>> {
        Box::pin(async move {
            Ok(match room {
                // Paradox has no network room
                RoomKind::Network => REFUSED,
                RoomKind::Region(region) => {
                    open_if(source.settings().dev_city_muc || hubs.may_join_region(account, region, replay::now_ms()))
                }
                RoomKind::Hunt(_) => open_if(source.settings().dev_hunt_muc),
                RoomKind::Party(party) => {
                    let ours = source
                        .party_of(account)
                        .await?
                        .and_then(|id| instance_id(&id, IdKind::Party, &source.node().node));
                    open_if(ours.as_ref() == Some(party))
                }
            })
        })
    }

    /// §5.5: the token's player, else a player homed elsewhere, else the refusal.
    fn login<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        authcid: &'a str,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Player, ModuleError>> {
        Box::pin(async move {
            if source.token_check().is_none() {
                return Err(ModuleError::new(
                    ErrorKind::Unsupported,
                    "chat login without a token verifier",
                ));
            }
            match authenticate(authcid, token, source).await {
                Ok(player) => Ok(player),
                Err(Refusal::LookupError) => {
                    Err(ModuleError::new(ErrorKind::Unavailable, Refusal::LookupError.as_str()))
                }
                Err(refusal) => match source.cross_node_login(authcid, token).await {
                    Some(player) => Ok(player),
                    None => Err(ModuleError::new(ErrorKind::Invalid, refusal.as_str())),
                },
            }
        })
    }

    /// Answered, though the plugin does no friend presence fan-out yet.
    fn friends<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        account: &'a GlobalId,
    ) -> BoxFuture<'a, Result<Vec<GlobalId>, ModuleError>> {
        source.friends(account)
    }

    /// 1.14.7 has no block list.
    fn may_whisper(&self, _from: &GlobalId, _to: &GlobalId) -> bool {
        true
    }

    /// §5.3; never reads the store. Only names the plugin's own naming refused reach it.
    fn room_of(&self, source: &dyn ChatSource, hubs: &HubSessions, name: &str) -> Option<RoomKind> {
        room_kind_of(name, source.node(), hubs, replay::now_ms())
    }

    fn account_of<'a>(
        &'a self,
        source: &'a dyn ChatSource,
        local: &'a str,
    ) -> BoxFuture<'a, Result<Option<GlobalId>, ModuleError>> {
        source.account_of(local)
    }
}

/// What a room local part names on `node` at `now` (§5.3).
pub fn room_kind_of(name: &str, node: &NodeIdentity, hubs: &HubSessions, now: i64) -> Option<RoomKind> {
    if let Some(rest) = name.strip_prefix("Party-") {
        return instance_id(rest, IdKind::Party, &node.node).map(RoomKind::Party);
    }
    if let Some(rest) = name.strip_prefix("Hunt-") {
        return instance_id(rest, IdKind::Hunt, &node.node).map(RoomKind::Hunt);
    }
    let rest = name.strip_prefix("City-")?;
    if let Some(region) = hubs.region_of_hub(rest, now) {
        return Some(RoomKind::Region(region));
    }
    if let Ok(region) = Region::new(rest) {
        return Some(RoomKind::Region(region));
    }
    // DD21: one room per region, so any other hub names this node's
    (!rest.is_empty()).then(|| RoomKind::Region(node.region.clone()))
}

/// Rustic Isles 1.14.7's rules: every default.
#[derive(Debug, Clone, Copy, Default)]
pub struct IslesChat;

impl ChatRules for IslesChat {}
