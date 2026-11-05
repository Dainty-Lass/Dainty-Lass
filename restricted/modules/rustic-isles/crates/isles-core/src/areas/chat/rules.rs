//! [`isles_chat`]'s window on the game: what the chat rules read, from the store (`RoomService.ts` and `XMPPAuth.ts`
//! asked `GetRepositories()` and `GetPartyForPlayer` the same).

use isles_chat::{ChatSettings, ChatSource, Player};
use module_api::network::{HomeForward, OperationName};
use module_api::{Actor, ActorContext, BoxFuture, ErrorKind, GlobalId, ModuleError, ModuleId, NodeIdentity};
use serde_json::{Value, json};

use crate::db::IslesStore;
use crate::ids::{AccountId, UserId};
use crate::repo::RepoError;
use crate::repo::auth::AccountRecord;
use crate::repo::chat::FriendshipStatus;
use crate::settings::Settings;

/// `REALTIME_XMPP_DEV_CITY_MUC` and `REALTIME_XMPP_DEV_HUNT_MUC` from the module's env file.
pub fn chat_settings(settings: &Settings) -> ChatSettings {
    let flag = |name: &str| settings.raw(name).map(str::trim).map(str::to_owned);
    ChatSettings {
        // off unless `true` (RU3c); Node's was on unless `false`
        dev_city_muc: flag("REALTIME_XMPP_DEV_CITY_MUC").is_some_and(|value| value == "true"),
        // on unless `false`, as Node's
        dev_hunt_muc: flag("REALTIME_XMPP_DEV_HUNT_MUC").is_none_or(|value| value != "false"),
    }
}

/// A repository that cannot answer is the module being unavailable: the plugin refuses the join or login instead of
/// opening a room the rules would have closed (hooks.rs of the plugin).
fn unavailable(error: RepoError) -> ModuleError {
    tracing::warn!("chat rules: {error}");
    ModuleError::new(ErrorKind::Unavailable, error.to_string())
}

fn player(record: AccountRecord) -> Player {
    Player {
        account: record.id.global().clone(),
        user_id: record.user_id.0,
        name: record.name,
    }
}

/// The chat rules' source over the module's store. Without a database it answers `unsupported`, which the plugin reads
/// as "use the default rules" (the authcid as the JID, every room open).
pub struct StoreChat {
    store: Option<IslesStore>,
    node: NodeIdentity,
    module_id: ModuleId,
    settings: ChatSettings,
    tokens: crate::areas::auth::tokens::GameTokens,
}

pub fn source(node: &NodeIdentity, store: Option<IslesStore>, settings: &Settings) -> StoreChat {
    StoreChat {
        store,
        node: node.clone(),
        module_id: "isles-1-14-7"
            .parse()
            .unwrap_or_else(|_| unreachable!("static module id is valid")),
        settings: chat_settings(settings),
        tokens: crate::areas::auth::tokens::GameTokens(settings.clone()),
    }
}

impl StoreChat {
    #[must_use]
    pub fn with_module_id(mut self, module_id: ModuleId) -> Self {
        self.module_id = module_id;
        self
    }

    fn store(&self) -> Result<&IslesStore, ModuleError> {
        self.store
            .as_ref()
            .ok_or_else(|| ModuleError::unsupported("chat without a database"))
    }
}

fn account_id(account: &GlobalId) -> Result<AccountId, ModuleError> {
    AccountId::try_from(account.clone()).map_err(|e| ModuleError::new(ErrorKind::Invalid, e.to_string()))
}

impl ChatSource for StoreChat {
    fn node(&self) -> &NodeIdentity {
        &self.node
    }

    fn settings(&self) -> ChatSettings {
        self.settings
    }

    fn verifier(&self) -> Option<&dyn isles_chat::login::TokenVerifier> {
        self.store.is_some().then_some(&self.tokens)
    }

    fn player<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            let (store, id) = (self.store()?, account_id(account)?);
            let found = store
                .read(move |conn, repos| repos.accounts.find(conn, &id))
                .await
                .map_err(unavailable)?;
            if let Some(record) = found {
                return Ok(Some(player(record)));
            }
            if account.owner() != &self.node.node
                && let Ok(op) = "chat.player".parse::<OperationName>()
                && let Ok(val) = store
                    .home()
                    .forward(&HomeForward {
                        module: self.module_id.clone(),
                        account: account.clone(),
                        actor: ActorContext::new(Actor::System),
                        operation: op,
                        body: json!({}),
                        key: None,
                    })
                    .await
                && let (Some(user_id), Some(name)) = (
                    val.get("user_id").and_then(Value::as_str),
                    val.get("name").and_then(Value::as_str),
                )
            {
                return Ok(Some(Player {
                    account: account.clone(),
                    user_id: user_id.to_owned(),
                    name: name.to_owned(),
                }));
            }
            Ok(None)
        })
    }

    // A launcher account must be admitted (`IsAccountEligible`: approval, status, a chosen username); an account that
    // is not a launcher account (a dev or service identity) keeps its API-key or token behaviour
    fn player_by_user<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            let (store, user) = (self.store()?, UserId(user_id.to_owned()));
            let found = store
                .read(move |conn, repos| {
                    let Some(account) = repos.accounts.find_by_user_id(conn, &user)? else {
                        return Ok(None);
                    };
                    let eligible = repos
                        .launcher_accounts
                        .find_by_user_id(conn, &user)?
                        .is_none_or(|launcher| crate::areas::auth::policy::is_account_eligible(&launcher));
                    Ok(eligible.then_some(account))
                })
                .await
                .map_err(unavailable)?;
            Ok(found.map(player))
        })
    }

    fn account_of<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Result<Option<GlobalId>, ModuleError>> {
        Box::pin(async move {
            if let Some(p) = self.player_by_user(local).await? {
                return Ok(Some(p.account));
            }
            let store = self.store()?;
            let normalized = local.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return Ok(None);
            }
            let found = store
                .read(move |conn, repos| {
                    let Some(launcher) = repos
                        .launcher_accounts
                        .find_by_display_name_normalized(conn, &normalized)?
                    else {
                        return Ok(None);
                    };
                    if !crate::areas::auth::policy::is_account_eligible(&launcher) {
                        return Ok(None);
                    }
                    Ok(Some(launcher.account.global().clone()))
                })
                .await
                .map_err(unavailable)?;
            Ok(found)
        })
    }

    fn foreign_login<'a>(
        &'a self,
        authcid: &'a str,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            let store = self.store()?;
            if let Ok((ticket, _game)) = module_api::network::SessionTicket::from_token(token) {
                let version = if self.module_id.as_str().contains("1-12") {
                    "1.12.0"
                } else {
                    "1.14.7"
                };
                let expected: module_api::GameVersion = version
                    .parse()
                    .unwrap_or_else(|_| unreachable!("static game version is valid"));
                if let Ok(admitted) = store.home().verify_session_ticket(&ticket, &expected).await
                    && let Some(p) = self.player(&admitted).await?
                    && (admitted.to_string() == authcid || p.user_id == authcid)
                {
                    return Ok(Some(p));
                }
            }
            if let Ok(account) = authcid.parse::<GlobalId>()
                && account.owner() != &self.node.node
                && let Ok(op) = "chat.login".parse::<OperationName>()
                && let Ok(val) = store
                    .home()
                    .forward(&HomeForward {
                        module: self.module_id.clone(),
                        account: account.clone(),
                        actor: ActorContext::new(Actor::Player {
                            account: account.clone(),
                        }),
                        operation: op,
                        body: json!({
                            "authcid": authcid,
                            "token": token,
                        }),
                        key: None,
                    })
                    .await
                && let (Some(user_id), Some(name)) = (
                    val.get("user_id").and_then(Value::as_str),
                    val.get("name").and_then(Value::as_str),
                )
            {
                return Ok(Some(Player {
                    account,
                    user_id: user_id.to_owned(),
                    name: name.to_owned(),
                }));
            }
            Ok(None)
        })
    }

    fn party_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<String>, ModuleError>> {
        Box::pin(async move {
            let (store, id) = (self.store()?, account_id(account)?);
            let party = store
                .read(move |conn, repos| repos.parties.find_by_member(conn, &id))
                .await
                .map_err(unavailable)?;
            if let Some(party) = party {
                return Ok(Some(party.party_id));
            }
            if account.owner() != &self.node.node
                && let Ok(op) = "chat.party".parse::<OperationName>()
                && let Ok(val) = store
                    .home()
                    .forward(&HomeForward {
                        module: self.module_id.clone(),
                        account: account.clone(),
                        actor: ActorContext::new(Actor::System),
                        operation: op,
                        body: json!({}),
                        key: None,
                    })
                    .await
            {
                return Ok(val.get("party_id").and_then(Value::as_str).map(str::to_owned));
            }
            Ok(None)
        })
    }

    fn friends<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Vec<GlobalId>, ModuleError>> {
        Box::pin(async move {
            let (store, id) = (self.store()?, account_id(account)?);
            let edges = store
                .read(move |conn, repos| repos.friendships.list_for_owner(conn, &id))
                .await
                .map_err(unavailable)?;
            Ok(edges
                .into_iter()
                .filter(|edge| edge.status == FriendshipStatus::Accepted)
                .map(|edge| edge.other.global().clone())
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dev_switches_read_like_the_plan_says() {
        let of = |pairs: &[(&str, &str)]| chat_settings(&Settings::from_pairs(pairs.iter().copied()));
        let nothing = of(&[]);
        assert!(!nothing.dev_city_muc, "RU3c: the City switch defaults to false");
        assert!(nothing.dev_hunt_muc, "Node's Hunt switch is on unless `false`");
        assert!(of(&[("REALTIME_XMPP_DEV_CITY_MUC", " true ")]).dev_city_muc);
        assert!(!of(&[("REALTIME_XMPP_DEV_CITY_MUC", "")]).dev_city_muc);
        assert!(!of(&[("REALTIME_XMPP_DEV_CITY_MUC", "false")]).dev_city_muc);
        assert!(!of(&[("REALTIME_XMPP_DEV_HUNT_MUC", "false")]).dev_hunt_muc);
        assert!(of(&[("REALTIME_XMPP_DEV_HUNT_MUC", "")]).dev_hunt_muc);
    }
}
