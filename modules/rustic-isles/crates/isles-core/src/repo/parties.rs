//! RU5 multipliers, matchmaking, parties and the game servers' posts. Parties, invites and the outbox get global ids
//! (DD18) when the area ports them; matchmaking tickets stay in memory (DD15).

use super::{AccountId, AnyConnection, Json, Tx, repository};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyActivity {
    Idle,
    Matchmaking,
    HostSelecting,
    StartingHunt,
    InHunt,
    Recovering,
}

/// `PartyRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyRecord {
    pub party_id: String,
    pub revision: i64,
    pub leader: AccountId,
    pub members: Vec<AccountId>,
    pub build_id: String,
    pub activity: PartyActivity,
    pub active_hunt_session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteStatus {
    Pending,
    Accepted,
    Revoked,
    Expired,
}

/// `PartyInviteRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyInviteRecord {
    pub invite_id: String,
    pub party_id: String,
    pub party_revision: i64,
    pub sender: AccountId,
    pub recipient: AccountId,
    pub sending_display_name: String,
    pub sending_platform: String,
    pub status: InviteStatus,
    pub created_at: String,
    pub expires_at: String,
}

/// `PartyOutboxRecord`: an event for the chat plugin to publish (`party-events@`).
#[derive(Debug, Clone, PartialEq)]
pub struct PartyOutboxRecord {
    pub event_id: String,
    pub party_id: String,
    pub party_revision: i64,
    /// `PARTY_CREATED`, `PARTY_INVITE_CREATED`...
    pub event_type: String,
    pub recipients: Vec<AccountId>,
    pub payload: Json,
    /// `PENDING`, `PROCESSING` or `PUBLISHED`
    pub state: String,
    pub created_at: String,
    pub claimed_at: Option<String>,
    pub claim_owner: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInvite {
    pub sender: AccountId,
    pub recipient: AccountId,
    pub build_id: String,
    pub sender_display_name: String,
    pub sender_platform: String,
    pub new_party_id: String,
    pub now: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptInvite {
    pub recipient: AccountId,
    pub inviter: AccountId,
    pub build_id: String,
    pub new_party_id: String,
    pub now: String,
    pub max_party_size: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartyEvent {
    pub party_id: String,
    pub party_revision: i64,
    pub event_type: String,
    pub recipients: Vec<AccountId>,
    pub payload: Json,
    pub now: String,
}

repository! {
    pub trait PartyRepository("parties") {
        fn find_by_id(&self, conn: &mut AnyConnection, party_id: &str) -> Option<PartyRecord>;
        fn find_by_member(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<PartyRecord>;
        fn get_or_create_solo(&self, tx: &mut Tx<'_>, account: &AccountId, build_id: &str, new_party_id: &str,
            now: &str) -> PartyRecord;
        fn create_invite(&self, tx: &mut Tx<'_>, invite: &NewInvite) -> ();
        fn list_pending_invites(&self, conn: &mut AnyConnection, recipient: &AccountId, now: &str)
            -> Vec<PartyInviteRecord>;
        fn accept_invite(&self, tx: &mut Tx<'_>, accept: &AcceptInvite) -> Option<PartyRecord>;
        fn leave(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> ();
        fn kick(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> ();
        fn promote(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> ();
        fn set_activity(&self, tx: &mut Tx<'_>, party_id: &str, expected_revision: i64, activity: PartyActivity,
            active_hunt_session_id: Option<&str>, now: &str) -> Option<PartyRecord>;
        fn append_event(&self, tx: &mut Tx<'_>, event: &PartyEvent) -> ();
        fn claim_next_outbox(&self, tx: &mut Tx<'_>, owner: &str, now: &str, stale_before: &str)
            -> Option<PartyOutboxRecord>;
        fn mark_outbox_published(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str, published_at: &str) -> bool;
        fn release_outbox(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str) -> ();
    }
}
