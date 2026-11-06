// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The rules' source over the module (spec §5.1, §7): its store, auth, node, module id and home.

use std::fmt;
use std::sync::Arc;

use module_api::network::SessionTicket;
use module_api::{
    Actor, ActorContext, BoxFuture, ErrorKind, GlobalId, HomeApi, HomeForward, IdKind, ModuleError, ModuleId,
    NodeIdentity, OperationName,
};
use rustic_isles::{AccountId, IslesAuth, Shared, UserId, replay, season};
use rustic_isles_auth::{AuthStore, LauncherAccount, is_account_eligible};
use rustic_isles_store::repo::chat::FriendshipStatus;
use rustic_isles_store::{IslesAuthStore, IslesStore, RepoError};
use serde_json::{Map, Value, json};

use crate::{ChatSettings, ChatSource, InvalidToken, Player, TokenCheck};

struct GameTokens(Arc<IslesAuth>);

impl TokenCheck for GameTokens {
    fn validate(&self, token: &str) -> Result<Map<String, Value>, InvalidToken> {
        self.0.validate_game_token(token).map_err(|_| InvalidToken)
    }
}

pub struct StoreChat {
    node: NodeIdentity,
    module_id: ModuleId,
    store: Option<IslesStore>,
    settings: ChatSettings,
    tokens: GameTokens,
    home: Arc<dyn HomeApi>,
}

impl fmt::Debug for StoreChat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreChat")
            .field("node", &self.node)
            .field("module_id", &self.module_id)
            .field("database", &self.store.is_some())
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

pub(crate) fn no_database() -> ModuleError {
    ModuleError::new(ErrorKind::Unsupported, "chat without a database")
}

fn unavailable(error: impl fmt::Display) -> ModuleError {
    tracing::warn!("chat rules: {error}");
    ModuleError::new(ErrorKind::Unavailable, error.to_string())
}

fn account_id(account: &GlobalId) -> Result<AccountId, ModuleError> {
    AccountId::try_from(account.clone()).map_err(|e| ModuleError::new(ErrorKind::Invalid, e.to_string()))
}

fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

/// A home's answer with string `user_id` and `name`.
fn player_from(answer: &Value, account: &GlobalId) -> Option<Player> {
    Some(Player {
        account: account.clone(),
        user_id: answer.get("user_id")?.as_str()?.to_owned(),
        name: answer.get("name")?.as_str()?.to_owned(),
    })
}

fn now_iso() -> String {
    season::to_iso_string(replay::now_ms())
}

impl StoreChat {
    pub fn new(
        node: NodeIdentity,
        module_id: ModuleId,
        store: Option<IslesStore>,
        settings: ChatSettings,
        auth: Arc<IslesAuth>,
        home: Arc<dyn HomeApi>,
    ) -> Self {
        Self {
            node,
            module_id,
            store,
            settings,
            tokens: GameTokens(auth),
            home,
        }
    }

    /// The module's node, settings, auth and home (the composer's one call at start).
    pub fn for_module(shared: &Shared, module_id: ModuleId, store: Option<IslesStore>) -> Self {
        Self::new(
            shared.node().clone(),
            module_id,
            store,
            ChatSettings::from_settings(shared.settings()),
            Arc::clone(shared.auth()),
            shared.home(),
        )
    }

    pub fn store(&self) -> Option<&IslesStore> {
        self.store.as_ref()
    }

    pub fn module_id(&self) -> &ModuleId {
        &self.module_id
    }

    fn database(&self) -> Result<&IslesStore, ModuleError> {
        self.store.as_ref().ok_or_else(no_database)
    }

    fn is_foreign(&self, account: &GlobalId) -> bool {
        account.owner() != &self.node.node
    }

    /// The home's answer to `operation`; any failure is none.
    async fn forward(&self, account: &GlobalId, actor: ActorContext, operation: &str, body: Value) -> Option<Value> {
        let call = HomeForward {
            module: self.module_id.clone(),
            account: account.clone(),
            actor,
            operation: OperationName::new(operation).ok()?,
            body,
            key: None,
        };
        self.home.forward(&call).await.ok()
    }

    /// The live `accounts` row here, never forwarded (`chat.player` at the home).
    pub(crate) async fn local_player(&self, account: &AccountId) -> Result<Option<Player>, RepoError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| RepoError::Data("no database".into()))?;
        let id = account.clone();
        let record = store.read(move |conn, repos| repos.accounts.find(conn, &id)).await?;
        Ok(record.map(|r| Player {
            account: r.id.global().clone(),
            user_id: r.user_id.as_str().to_owned(),
            name: r.name,
        }))
    }

    /// The client-facing id of the account's party here, never forwarded (`chat.party` at the home).
    pub(crate) async fn local_party(&self, account: &AccountId) -> Result<Option<String>, RepoError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| RepoError::Data("no database".into()))?;
        let id = account.clone();
        let party = store
            .read(move |conn, repos| repos.parties.find_by_member(conn, &id))
            .await?;
        Ok(party.map(|p| p.party_id))
    }

    fn eligible(launcher: Option<LauncherAccount>) -> bool {
        launcher.is_none_or(|account| is_account_eligible(&account, &now_iso()))
    }

    /// Steps 1-5 of §5.5's `player by userId` with the store error kept (the login's `account lookup error`).
    async fn player_by_user_id_inner(&self, store: &IslesStore, user_id: &str) -> Result<Option<Player>, String> {
        let key = UserId::new(user_id);
        let record = store
            .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &key))
            .await
            .map_err(|e| e.to_string())?;
        let Some(record) = record else {
            return Ok(None);
        };
        let launcher = IslesAuthStore::new(store.clone())
            .launcher_account_by_user_id(user_id)
            .await
            .map_err(|e| e.to_string())?;
        if !Self::eligible(launcher) {
            return Ok(None);
        }
        Ok(Some(Player {
            account: record.id.global().clone(),
            user_id: record.user_id.as_str().to_owned(),
            name: record.name,
        }))
    }

    async fn ticket_login(&self, authcid: &str, token: &str) -> Option<Player> {
        let (ticket, _) = SessionTicket::from_token(token).ok()?;
        let game = &self.tokens.0.config().game_version;
        let admitted = self.home.verify_session_ticket(&ticket, game).await.ok()?;
        let player = self.player_by_account(&admitted).await.ok()??;
        (admitted.to_string() == authcid || player.user_id == authcid).then_some(player)
    }
}

impl ChatSource for StoreChat {
    fn node(&self) -> &NodeIdentity {
        &self.node
    }

    fn settings(&self) -> ChatSettings {
        self.settings
    }

    fn token_check(&self) -> Option<&dyn TokenCheck> {
        self.store.as_ref().map(|_| &self.tokens as &dyn TokenCheck)
    }

    fn player_by_account<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            self.database()?;
            let id = account_id(account)?;
            if let Some(player) = self.local_player(&id).await.map_err(unavailable)? {
                return Ok(Some(player));
            }
            if !self.is_foreign(account) {
                return Ok(None);
            }
            let answer = self.forward(account, system(), "chat.player", json!({})).await;
            Ok(answer.and_then(|a| player_from(&a, account)))
        })
    }

    fn player_by_user_id<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            let store = self.database()?;
            self.player_by_user_id_inner(store, user_id).await.map_err(unavailable)
        })
    }

    fn account_of<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Result<Option<GlobalId>, ModuleError>> {
        Box::pin(async move {
            if let Some(player) = self.player_by_user_id(local).await? {
                return Ok(Some(player.account));
            }
            let normalized = service_kit::js::trim(local).to_ascii_lowercase();
            if normalized.is_empty() {
                return Ok(None);
            }
            let store = self.database()?;
            let launcher = IslesAuthStore::new(store.clone())
                .launcher_account_by_name(&normalized)
                .await
                .map_err(unavailable)?;
            let Some(launcher) = launcher else {
                return Ok(None);
            };
            if !is_account_eligible(&launcher, &now_iso()) {
                return Ok(None);
            }
            Ok(launcher.account_id.parse::<GlobalId>().ok())
        })
    }

    fn cross_node_login<'a>(&'a self, authcid: &'a str, token: &'a str) -> BoxFuture<'a, Option<Player>> {
        Box::pin(async move {
            if SessionTicket::from_token(token).is_ok() {
                return self.ticket_login(authcid, token).await;
            }
            let account = authcid.parse::<GlobalId>().ok()?;
            if account.kind() != IdKind::Account || !self.is_foreign(&account) {
                return None;
            }
            let actor = ActorContext::new(Actor::Player {
                account: account.clone(),
            });
            let body = json!({"authcid": authcid, "token": token});
            let answer = self.forward(&account, actor, "chat.login", body).await?;
            player_from(&answer, &account)
        })
    }

    fn party_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<String>, ModuleError>> {
        Box::pin(async move {
            self.database()?;
            let id = account_id(account)?;
            if let Some(party) = self.local_party(&id).await.map_err(unavailable)? {
                return Ok(Some(party));
            }
            if !self.is_foreign(account) {
                return Ok(None);
            }
            let answer = self.forward(account, system(), "chat.party", json!({})).await;
            Ok(answer.and_then(|a| a.get("party_id").and_then(Value::as_str).map(str::to_owned)))
        })
    }

    fn friends<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Vec<GlobalId>, ModuleError>> {
        Box::pin(async move {
            let store = self.database()?;
            let id = account_id(account)?;
            let edges = store
                .read(move |conn, repos| repos.friendships.list_for_owner(conn, &id))
                .await
                .map_err(unavailable)?;
            Ok(edges
                .into_iter()
                .filter(|e| e.status == FriendshipStatus::Accepted)
                .map(|e| e.other.global().clone())
                .collect())
        })
    }
}
