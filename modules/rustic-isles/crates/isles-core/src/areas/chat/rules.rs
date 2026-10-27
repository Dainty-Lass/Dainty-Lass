//! [`isles_chat`]'s window on the game: what the chat rules read, from the store (`RoomService.ts` and `XMPPAuth.ts`
//! asked `GetRepositories()` and `GetPartyForPlayer` the same).

use isles_chat::{ChatSettings, ChatSource, Player};
use module_api::{BoxFuture, ErrorKind, GlobalId, ModuleError, NodeIdentity};

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
    settings: ChatSettings,
}

pub fn source(node: &NodeIdentity, store: Option<IslesStore>, settings: &Settings) -> StoreChat {
    StoreChat {
        store,
        node: node.clone(),
        settings: chat_settings(settings),
    }
}

impl StoreChat {
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

    fn player<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            let (store, id) = (self.store()?, account_id(account)?);
            let found = store
                .read(move |conn, repos| repos.accounts.find(conn, &id))
                .await
                .map_err(unavailable)?;
            Ok(found.map(player))
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

    fn party_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<String>, ModuleError>> {
        Box::pin(async move {
            let (store, id) = (self.store()?, account_id(account)?);
            let party = store
                .read(move |conn, repos| repos.parties.find_by_member(conn, &id))
                .await
                .map_err(unavailable)?;
            Ok(party.map(|party| party.party_id))
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
