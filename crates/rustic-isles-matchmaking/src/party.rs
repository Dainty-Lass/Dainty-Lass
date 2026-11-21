// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Parties (spec §7, store §11.2): membership, revisions and the outbox live in the database under global ids
//! (DD18); the client keeps seeing Node's `partyId` and userIds.

use dainty_module_api::{Actor, ActorContext, GlobalId, IdKind, NodeId};
use rustic_isles_store::repo::parties::{AcceptInvite, NewInvite, NewParty, PartyRecord, REVISION_CONFLICT};
use rustic_isles_store::{AccountId, IslesStore, RepoError, Repositories, Tx, UserId};

use crate::rules::make_party_id;

/// Invites last ten minutes.
pub const INVITE_TTL_MS: i64 = 10 * 60 * 1000;
/// Most members a party may hold.
pub const MAX_PARTY_SIZE: u32 = 4;
/// Every player is on Windows.
pub const PLATFORM: &str = "win";
const WRITE_ATTEMPTS: usize = 3;

/// Why a party operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartyError {
    /// The routes' 501
    NoDatabase,
    /// The store refused, or the player has no account; the text says which.
    Failed(String),
}

impl std::fmt::Display for PartyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDatabase => f.write_str("no database is configured"),
            Self::Failed(text) => f.write_str(text),
        }
    }
}

impl From<RepoError> for PartyError {
    fn from(error: RepoError) -> Self {
        Self::Failed(error.to_string())
    }
}

/// An invitation as `GET /party/invites` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    /// The party's id.
    pub party_id: String,
    /// The inviter's userId.
    pub sender_user_id: String,
    /// The inviter's display name when the invite was sent.
    pub display_name: String,
    /// The inviter's platform.
    pub platform: String,
}

/// The party operations over the module's store, as one acting request.
pub struct Parties<'a> {
    /// The module's store; none answers [`PartyError::NoDatabase`].
    pub store: Option<&'a IslesStore>,
    /// The node new party ids belong to.
    pub node: &'a NodeId,
    /// The request's id, carried into store writes.
    pub request_id: Option<&'a str>,
    /// The request's time, Unix ms.
    pub now: i64,
}

fn iso(ms: i64) -> String {
    rustic_isles::season::to_iso_string(ms)
}

impl Parties<'_> {
    fn store(&self) -> Result<&IslesStore, PartyError> {
        self.store.ok_or(PartyError::NoDatabase)
    }

    fn new_party(&self, build_id: &str) -> NewParty {
        let id = GlobalId::new(IdKind::Party, self.node);
        let party_id = make_party_id(&id, build_id);
        NewParty { id, party_id }
    }

    fn actor(&self, account: &AccountId) -> ActorContext {
        let context = ActorContext::new(Actor::Player {
            account: account.global().clone(),
        });
        match self.request_id {
            Some(id) => context.with_request(id),
            None => context,
        }
    }

    /// The account for a userId, `None` when it has none.
    pub async fn account_of(&self, user: &str) -> Result<Option<AccountId>, PartyError> {
        let store = self.store()?;
        let user = UserId::new(user);
        let found = store
            .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
            .await?;
        Ok(found.map(|record| record.id))
    }

    async fn require_account(&self, user: &str) -> Result<AccountId, PartyError> {
        self.account_of(user)
            .await?
            .ok_or_else(|| PartyError::Failed(format!("{user} has no account")))
    }

    /// An account write retried on a lost revision race, three attempts in all.
    async fn write<T, F>(&self, account: &AccountId, f: F) -> Result<T, PartyError>
    where
        F: Fn(&mut Tx<'_>, &Repositories) -> Result<T, RepoError> + Clone + Send + 'static,
        T: Send + 'static,
    {
        let store = self.store()?;
        let actor = self.actor(account);
        let mut attempt = 0;
        loop {
            attempt += 1;
            match store.account_write(account, &actor, f.clone()).await {
                Err(e) if attempt < WRITE_ATTEMPTS && e.to_string().contains(REVISION_CONFLICT) => continue,
                answer => return answer.map_err(PartyError::from),
            }
        }
    }

    /// The player's party, `None` without an account or a party.
    pub async fn for_player(&self, user: &str) -> Result<Option<PartyRecord>, PartyError> {
        let Some(account) = self.account_of(user).await? else {
            return Ok(None);
        };
        let store = self.store()?;
        Ok(store
            .read(move |conn, repos| repos.parties.find_by_member(conn, &account))
            .await?)
    }

    /// The player's party, or a new solo party when the player has none.
    pub async fn get_or_create(&self, user: &str, build_id: &str) -> Result<PartyRecord, PartyError> {
        let account = self.require_account(user).await?;
        let new_party = self.new_party(build_id);
        let (build, now, who) = (build_id.to_owned(), iso(self.now), account.clone());
        self.write(&account, move |tx, repos| {
            repos.parties.get_or_create_solo(tx, &who, &build, &new_party, &now)
        })
        .await
    }

    /// Invites `recipient` to `sender`'s party for ten minutes; a missing recipient, or the sender as recipient, is a no-op.
    pub async fn invite(
        &self,
        sender: &str,
        recipient: &str,
        build_id: &str,
        name: &str,
        platform: &str,
    ) -> Result<(), PartyError> {
        if recipient.is_empty() || recipient == sender {
            return Ok(());
        }
        let sender_account = self.require_account(sender).await?;
        // No one could ever read an invite to nobody
        let Some(recipient_account) = self.account_of(recipient).await? else {
            return Ok(());
        };
        let invite = NewInvite {
            sender: sender_account.clone(),
            recipient: recipient_account,
            build_id: build_id.to_owned(),
            sender_display_name: name.to_owned(),
            sender_platform: platform.to_owned(),
            new_party: self.new_party(build_id),
            now: iso(self.now),
            expires_at: iso(self.now.saturating_add(INVITE_TTL_MS)),
        };
        self.write(&sender_account, move |tx, repos| {
            repos.parties.create_invite(tx, &invite)
        })
        .await
    }

    /// The player's pending invitations; none without an account.
    pub async fn invites_for(&self, user: &str) -> Result<Vec<Invitation>, PartyError> {
        let Some(account) = self.account_of(user).await? else {
            return Ok(Vec::new());
        };
        let now = iso(self.now);
        let store = self.store()?;
        let listed = store
            .read(move |conn, repos| {
                let invites = repos.parties.list_pending_invites(conn, &account, &now)?;
                let mut out = Vec::with_capacity(invites.len());
                for invite in invites {
                    let sender = repos.accounts.find(conn, &invite.sender)?;
                    let sender_user_id =
                        sender.map_or_else(|| invite.sender.to_string(), |s| s.user_id.as_str().to_owned());
                    out.push(Invitation {
                        party_id: invite.party_id,
                        sender_user_id,
                        display_name: invite.sending_display_name,
                        platform: invite.sending_platform,
                    });
                }
                Ok(out)
            })
            .await?;
        Ok(listed)
    }

    /// Joins the inviter's party; a missing inviter leaves the recipient's party as it is. Returns the recipient's
    /// party.
    pub async fn accept(
        &self,
        recipient: &str,
        inviter: &str,
        build_id: &str,
    ) -> Result<Option<PartyRecord>, PartyError> {
        let recipient_account = self.require_account(recipient).await?;
        let Some(inviter_account) = self.account_of(inviter).await? else {
            return self.for_player(recipient).await;
        };
        let accept = AcceptInvite {
            recipient: recipient_account.clone(),
            inviter: inviter_account,
            build_id: build_id.to_owned(),
            new_party: self.new_party(build_id),
            now: iso(self.now),
            max_party_size: MAX_PARTY_SIZE,
        };
        self.write(&recipient_account, move |tx, repos| {
            repos.parties.accept_invite(tx, &accept)
        })
        .await
    }

    /// The caller runs the member-left hook after it.
    pub async fn leave(&self, user: &str) -> Result<(), PartyError> {
        let account = self.require_account(user).await?;
        let (now, who) = (iso(self.now), account.clone());
        self.write(&account, move |tx, repos| repos.parties.leave(tx, &who, &now))
            .await
    }

    /// Whether the target was removed: only then does the member-left hook run (a departure, spec §15 item 8).
    pub async fn kick(&self, actor: &str, target: &str) -> Result<bool, PartyError> {
        let actor_account = self.require_account(actor).await?;
        let Some(target_account) = self.account_of(target).await? else {
            return Ok(false);
        };
        let (now, who) = (iso(self.now), actor_account.clone());
        self.write(&actor_account, move |tx, repos| {
            let before = repos.parties.find_by_member(tx.conn(), &target_account)?;
            repos.parties.kick(tx, &who, &target_account, &now)?;
            let after = repos.parties.find_by_member(tx.conn(), &target_account)?;
            let left = |party: &Option<PartyRecord>| party.as_ref().map(|p| p.id.clone());
            Ok(before.is_some() && left(&before) != left(&after))
        })
        .await
    }

    /// Makes `target` the leader of `actor`'s party; a missing target is a no-op.
    pub async fn promote(&self, actor: &str, target: &str) -> Result<(), PartyError> {
        let actor_account = self.require_account(actor).await?;
        let Some(target_account) = self.account_of(target).await? else {
            return Ok(());
        };
        let (now, who) = (iso(self.now), actor_account.clone());
        self.write(&actor_account, move |tx, repos| {
            repos.parties.promote(tx, &who, &target_account, &now)
        })
        .await
    }

    /// The launcher account's display name, else the game account's name, else the userId itself.
    pub async fn username(&self, user: &str) -> Result<String, PartyError> {
        let store = self.store()?;
        let id = UserId::new(user);
        let account = store
            .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &id))
            .await?;
        let Some(account) = account else {
            return Ok(user.to_owned());
        };
        let key = account.id.clone();
        let launcher = store
            .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &key))
            .await;
        match launcher {
            Ok(Some(record)) if !record.display_name.is_empty() => Ok(record.display_name),
            _ if !account.name.is_empty() => Ok(account.name),
            _ => Ok(user.to_owned()),
        }
    }
}
