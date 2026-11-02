//! `controllers/party.ts`: the party service (mm2-prod's party, captured in
//! `DauntlessEndpointDocumentation/Matchmaking/{GetParty,Party/*}.md`). Membership, revisions and the outbox live in
//! the database under global ids (DD18); the client keeps seeing Node's `partyId` and `userId`s.

use module_api::{Actor, ActorContext, GlobalId, IdKind, NodeId};

use super::util::{Clock, iso};
use crate::db::IslesStore;
use crate::http::{Rifts, Shared};
use crate::repo::parties::{AcceptInvite, NewInvite, NewParty, PartyActivity, PartyRecord, REVISION_CONFLICT};
use crate::repo::{AccountId, RepoError, RepoResult, Repositories, Tx, UserId};

pub const INVITE_TTL_MS: i64 = 10 * 60 * 1000;
pub const MAX_PARTY_SIZE: u32 = 4;
pub const PLATFORM: &str = "win";

/// `Party`: `userId`s and Node's `partyId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    pub party_id: String,
    /// The party's global id, which the realms side names it by
    pub global_id: GlobalId,
    pub revision: i64,
    pub leader: String,
    pub members: Vec<String>,
    pub build_id: String,
    pub activity: PartyActivity,
    pub active_hunt_session_id: Option<String>,
}

impl From<PartyRecord> for Party {
    fn from(record: PartyRecord) -> Self {
        Self {
            party_id: record.party_id,
            global_id: record.id,
            revision: record.revision,
            leader: record.leader.user_id.to_string(),
            members: record.members.iter().map(|m| m.user_id.to_string()).collect(),
            build_id: record.build_id,
            activity: record.activity,
            active_hunt_session_id: record.active_hunt_session_id,
        }
    }
}

/// `PartyInvite`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyInvite {
    pub party_id: String,
    pub sending_player_id: String,
    pub sending_display_name: String,
    pub sending_platform: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PartyError {
    #[error("no database is configured")]
    NoDatabase,
    #[error("{0} has no account")]
    UnknownAccount(String),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// `MakePartyId`, spelt the way the chat plugin reads a party in a room name (`Party-<id>`): the party's global id as
/// `<ULID>.<node>`, then the build id in base64 after an underscore, which the plugin ignores.
pub fn make_party_id(id: &GlobalId, build_id: &str) -> String {
    format!("{}_{}", isles_chat::ids::room_id_text(id), base64(build_id.as_bytes()))
}

/// Standard base64 with padding, as `Buffer.toString("base64")`.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What the party service needs of its module, borrowed: the module's request handlers have it in `Shared`, and the
/// realms side's `IslesCore` holds the same parts without one.
pub struct Parties<'a> {
    pub store: Option<&'a IslesStore>,
    pub node: &'a NodeId,
    pub clock: &'a Clock,
    /// For the member-left hook; the realms' own calls into the core have no use for it
    pub rifts: Option<&'a Rifts>,
    pub request_id: Option<String>,
}

fn conflict(error: &RepoError) -> bool {
    matches!(error, RepoError::Data(message) if message.contains(REVISION_CONFLICT))
}

impl<'a> Parties<'a> {
    pub fn of(shared: &'a Shared, request_id: Option<String>) -> Self {
        Self {
            store: shared.store.as_ref(),
            node: &shared.node.node,
            clock: &shared.matchmaking.clock,
            rifts: Some(&shared.rifts),
            request_id,
        }
    }

    fn store(&self) -> Result<&IslesStore, PartyError> {
        self.store.ok_or(PartyError::NoDatabase)
    }

    fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    fn new_party(&self, build_id: &str) -> NewParty {
        let id = GlobalId::new(IdKind::Party, self.node);
        NewParty {
            party_id: make_party_id(&id, build_id),
            id,
        }
    }

    async fn account(&self, user: &str) -> Result<Option<AccountId>, PartyError> {
        let user = UserId(user.to_owned());
        let found = self
            .store()?
            .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
            .await?;
        Ok(found.map(|account| account.id))
    }

    async fn require_account(&self, user: &str) -> Result<AccountId, PartyError> {
        self.account(user)
            .await?
            .ok_or_else(|| PartyError::UnknownAccount(user.to_owned()))
    }

    /// `WithPartyTransaction`: three attempts at a write that lost a revision race.
    async fn write<T, F>(&self, account: &AccountId, work: F) -> Result<T, PartyError>
    where
        T: Send + 'static,
        F: Fn(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Clone + Send + 'static,
    {
        let mut actor = ActorContext::new(Actor::Player {
            account: account.global().clone(),
        });
        if let Some(id) = &self.request_id {
            actor = actor.with_request(id.clone());
        }
        let store = self.store()?;
        for attempt in 1..=3 {
            match store.account_write(account, &actor, work.clone()).await {
                Err(error) if attempt < 3 && conflict(&error) => {}
                result => return Ok(result?),
            }
        }
        Err(RepoError::Data(REVISION_CONFLICT.to_owned()).into())
    }

    /// `GetPartyForPlayer`.
    pub async fn for_player(&self, user: &str) -> Result<Option<Party>, PartyError> {
        let Some(account) = self.account(user).await? else {
            return Ok(None);
        };
        let record = self
            .store()?
            .read(move |conn, repos| repos.parties.find_by_member(conn, &account))
            .await?;
        Ok(record.map(Party::from))
    }

    /// `GetOrCreateParty`.
    pub async fn get_or_create(&self, user: &str, build_id: &str) -> Result<Party, PartyError> {
        let account = self.require_account(user).await?;
        let now = iso(self.now());
        let (build, new_party) = (build_id.to_owned(), self.new_party(build_id));
        let who = account.clone();
        let record = self
            .write(&account, move |tx, repos| {
                repos.parties.get_or_create_solo(tx, &who, &build, &new_party, &now)
            })
            .await?;
        Ok(record.into())
    }

    /// `InviteToParty`. An invite to someone who has no account can never be read by anyone, so it is dropped.
    pub async fn invite(
        &self,
        sender: &str,
        recipient: &str,
        build_id: &str,
        sender_display_name: &str,
        sender_platform: &str,
    ) -> Result<(), PartyError> {
        if recipient.is_empty() || recipient == sender {
            return Ok(());
        }
        let sender_account = self.require_account(sender).await?;
        let Some(recipient_account) = self.account(recipient).await? else {
            return Ok(());
        };
        let now = self.now();
        let invite = NewInvite {
            sender: sender_account.clone(),
            recipient: recipient_account,
            build_id: build_id.to_owned(),
            sender_display_name: sender_display_name.to_owned(),
            sender_platform: sender_platform.to_owned(),
            new_party: self.new_party(build_id),
            now: iso(now),
            expires_at: iso(now + INVITE_TTL_MS),
        };
        self.write(&sender_account, move |tx, repos| {
            repos.parties.create_invite(tx, &invite)
        })
        .await
    }

    /// `GetInvitesForPlayer`.
    pub async fn invites_for(&self, user: &str) -> Result<Vec<PartyInvite>, PartyError> {
        let Some(account) = self.account(user).await? else {
            return Ok(Vec::new());
        };
        let now = iso(self.now());
        let records = self
            .store()?
            .read(move |conn, repos| repos.parties.list_pending_invites(conn, &account, &now))
            .await?;
        Ok(records
            .into_iter()
            .map(|record| PartyInvite {
                party_id: record.party_id,
                sending_player_id: record.sender.user_id.to_string(),
                sending_display_name: record.sending_display_name,
                sending_platform: record.sending_platform,
            })
            .collect())
    }

    /// After a member leaves or is kicked, his forming player-hosted or realm hunt stops waiting for him (the realms
    /// hook). A failure is logged and never fails the party change, which has already been written.
    async fn after_member_left(&self, user: &str) {
        let Some(rifts) = self.rifts else { return };
        let account = rifts_seam::AccountId(user.to_owned());
        if let Err(error) = rifts.realms().party_member_left(account).await {
            tracing::warn!("[PartyLeave] could not update the forming hunt of {user}: {error}");
        }
    }

    /// `LeaveParty`.
    pub async fn leave(&self, user: &str) -> Result<(), PartyError> {
        let account = self.require_account(user).await?;
        let now = iso(self.now());
        let who = account.clone();
        self.write(&account, move |tx, repos| repos.parties.leave(tx, &who, &now))
            .await?;
        self.after_member_left(user).await;
        Ok(())
    }

    /// `AcceptInvite`.
    pub async fn accept(&self, recipient: &str, inviter: &str, build_id: &str) -> Result<Option<Party>, PartyError> {
        let recipient_account = self.require_account(recipient).await?;
        let Some(inviter_account) = self.account(inviter).await? else {
            // no account, so no invite from it: Node answers with the recipient's own party
            return self.for_player(recipient).await;
        };
        let accept = AcceptInvite {
            recipient: recipient_account.clone(),
            inviter: inviter_account,
            build_id: build_id.to_owned(),
            new_party: self.new_party(build_id),
            now: iso(self.now()),
            max_party_size: MAX_PARTY_SIZE,
        };
        let record = self
            .write(&recipient_account, move |tx, repos| {
                repos.parties.accept_invite(tx, &accept)
            })
            .await?;
        Ok(record.map(Party::from))
    }

    /// `KickMember`. A target that has no account is no member.
    pub async fn kick(&self, actor: &str, target: &str) -> Result<(), PartyError> {
        let actor_account = self.require_account(actor).await?;
        if let Some(target_account) = self.account(target).await? {
            let now = iso(self.now());
            let who = actor_account.clone();
            self.write(&actor_account, move |tx, repos| {
                repos.parties.kick(tx, &who, &target_account, &now)
            })
            .await?;
        }
        self.after_member_left(target).await;
        Ok(())
    }

    /// `PromoteMember`.
    pub async fn promote(&self, actor: &str, target: &str) -> Result<(), PartyError> {
        let actor_account = self.require_account(actor).await?;
        let Some(target_account) = self.account(target).await? else {
            return Ok(());
        };
        let now = iso(self.now());
        let who = actor_account.clone();
        self.write(&actor_account, move |tx, repos| {
            repos.parties.promote(tx, &who, &target_account, &now)
        })
        .await
    }

    /// `GetUsernameForUserId`: the launcher account's display name, else the game account's name, else the id.
    pub async fn username(&self, user: &str) -> Result<String, PartyError> {
        let id = UserId(user.to_owned());
        let name = self
            .store()?
            .read(move |conn, repos| {
                let Some(account) = repos.accounts.find_by_user_id(conn, &id)? else {
                    return Ok(None);
                };
                // The launcher accounts are the authoritative names; a failure there falls back to the game account
                if let Ok(Some(launcher)) = repos.launcher_accounts.find_by_account(conn, &account.id)
                    && !launcher.display_name.is_empty()
                {
                    return Ok(Some(launcher.display_name));
                }
                Ok(Some(account.name))
            })
            .await?;
        Ok(name.unwrap_or_else(|| user.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn party_ids_are_the_room_spelling_of_the_global_id_then_the_build_id_in_base64() {
        let node: NodeId = "oregon-1".parse().unwrap();
        let global = GlobalId::new(IdKind::Party, &node);
        let id = make_party_id(&global, "647472");
        assert_eq!(id, format!("{}.oregon-1_NjQ3NDcy", global.ulid()));
        // the chat plugin finds the party from it, build suffix and all
        assert_eq!(
            isles_chat::ids::instance_id(&id, IdKind::Party, &node),
            Some(global.clone())
        );
        assert!(make_party_id(&global, "").ends_with('_'));
    }

    #[test]
    fn base64_pads_like_buffer() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64("392819_1.12.0".as_bytes()), "MzkyODE5XzEuMTIuMA==");
    }
}
