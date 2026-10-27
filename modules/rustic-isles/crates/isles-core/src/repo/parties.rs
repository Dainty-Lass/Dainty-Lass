//! RU5 multipliers, matchmaking, parties and the game servers' posts. Parties get global ids (DD18) and live on
//! `store` with an outbox (`persistence/contracts/PartyRepository.ts`); matchmaking tickets stay in memory (DD15).
//!
//! A party's key is its global id; `party_id` is Node's `partyId` (`<hex>_<base64 buildId>`), the one the client sees
//! and sends back. Members are global account ids with their `userId` beside them, which is what events and the
//! client carry.

use module_api::GlobalId;

use super::{AccountId, AnyConnection, Json, Tx, UserId, repository};

mod sql;

pub use sql::SqlParties;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyActivity {
    Idle,
    Matchmaking,
    HostSelecting,
    StartingHunt,
    InHunt,
    Recovering,
}

impl PartyActivity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Matchmaking => "MATCHMAKING",
            Self::HostSelecting => "HOST_SELECTING",
            Self::StartingHunt => "STARTING_HUNT",
            Self::InHunt => "IN_HUNT",
            Self::Recovering => "RECOVERING",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "IDLE" => Self::Idle,
            "MATCHMAKING" => Self::Matchmaking,
            "HOST_SELECTING" => Self::HostSelecting,
            "STARTING_HUNT" => Self::StartingHunt,
            "IN_HUNT" => Self::InHunt,
            "RECOVERING" => Self::Recovering,
            _ => return None,
        })
    }
}

/// An account as a party names it: the global id the data is keyed by, and the `userId` the client sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyMember {
    pub account: AccountId,
    pub user_id: UserId,
}

/// `PartyRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyRecord {
    pub id: GlobalId,
    /// Node's `partyId`
    pub party_id: String,
    pub revision: i64,
    pub leader: PartyMember,
    /// Join order: the first one takes over when the leader leaves
    pub members: Vec<PartyMember>,
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

impl InviteStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Accepted => "ACCEPTED",
            Self::Revoked => "REVOKED",
            Self::Expired => "EXPIRED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "PENDING" => Self::Pending,
            "ACCEPTED" => Self::Accepted,
            "REVOKED" => Self::Revoked,
            "EXPIRED" => Self::Expired,
            _ => return None,
        })
    }
}

/// `PartyInviteRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyInviteRecord {
    /// `<recipient userId>\u{1f}<sender userId>`, opaque: it is only echoed in events
    pub invite_id: String,
    /// Node's `partyId`
    pub party_id: String,
    pub party_revision: i64,
    pub sender: PartyMember,
    pub recipient: PartyMember,
    pub sending_display_name: String,
    pub sending_platform: String,
    pub status: InviteStatus,
    pub created_at: String,
    pub expires_at: String,
}

/// `PartyEventType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyEventType {
    Created,
    InviteCreated,
    MembershipChanged,
    LeaderChanged,
    ActivityChanged,
    PresenceChanged,
    HuntStatusChanged,
}

impl PartyEventType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "PARTY_CREATED",
            Self::InviteCreated => "PARTY_INVITE_CREATED",
            Self::MembershipChanged => "PARTY_MEMBERSHIP_CHANGED",
            Self::LeaderChanged => "PARTY_LEADER_CHANGED",
            Self::ActivityChanged => "PARTY_ACTIVITY_CHANGED",
            Self::PresenceChanged => "PARTY_PRESENCE_CHANGED",
            Self::HuntStatusChanged => "PARTY_HUNT_STATUS_CHANGED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "PARTY_CREATED" => Self::Created,
            "PARTY_INVITE_CREATED" => Self::InviteCreated,
            "PARTY_MEMBERSHIP_CHANGED" => Self::MembershipChanged,
            "PARTY_LEADER_CHANGED" => Self::LeaderChanged,
            "PARTY_ACTIVITY_CHANGED" => Self::ActivityChanged,
            "PARTY_PRESENCE_CHANGED" => Self::PresenceChanged,
            "PARTY_HUNT_STATUS_CHANGED" => Self::HuntStatusChanged,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    Pending,
    Processing,
    Published,
}

impl OutboxState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Processing => "PROCESSING",
            Self::Published => "PUBLISHED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "PENDING" => Self::Pending,
            "PROCESSING" => Self::Processing,
            "PUBLISHED" => Self::Published,
            _ => return None,
        })
    }
}

/// `PartyOutboxRecord`: an event for the chat plugin to publish (`party-events@`).
#[derive(Debug, Clone, PartialEq)]
pub struct PartyOutboxRecord {
    pub event_id: String,
    /// Node's `partyId`
    pub party_id: String,
    pub party_revision: i64,
    pub event_type: PartyEventType,
    /// `userId`s, in the order they were given and without repeats
    pub recipients: Vec<UserId>,
    pub payload: Json,
    pub state: OutboxState,
    pub created_at: String,
    pub claimed_at: Option<String>,
    pub claim_owner: Option<String>,
    pub published_at: Option<String>,
}

/// The ids of a party about to be created (`MakePartyId`'s result, with its global id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewParty {
    pub id: GlobalId,
    pub party_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInvite {
    pub sender: AccountId,
    pub recipient: AccountId,
    pub build_id: String,
    pub sender_display_name: String,
    pub sender_platform: String,
    pub new_party: NewParty,
    pub now: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptInvite {
    pub recipient: AccountId,
    pub inviter: AccountId,
    pub build_id: String,
    pub new_party: NewParty,
    pub now: String,
    pub max_party_size: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartyEvent {
    /// Node's `partyId`
    pub party_id: String,
    pub party_revision: i64,
    pub event_type: PartyEventType,
    pub recipients: Vec<UserId>,
    pub payload: Json,
    pub now: String,
}

/// Why a write conflicted; the controller retries it (Node's `PARTY_REVISION_CONFLICT`, three attempts).
pub const REVISION_CONFLICT: &str = "PARTY_REVISION_CONFLICT";

repository! {
    pub trait PartyRepository("parties") {
        fn find_by_id(&self, conn: &mut AnyConnection, party: &GlobalId) -> Option<PartyRecord>;
        /// By Node's `partyId`, the one the client names
        fn find_by_party_id(&self, conn: &mut AnyConnection, party_id: &str) -> Option<PartyRecord>;
        fn find_by_member(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<PartyRecord>;
        fn get_or_create_solo(&self, tx: &mut Tx<'_>, account: &AccountId, build_id: &str, new_party: &NewParty,
            now: &str) -> PartyRecord;
        fn create_invite(&self, tx: &mut Tx<'_>, invite: &NewInvite) -> ();
        /// Newest first
        fn list_pending_invites(&self, conn: &mut AnyConnection, recipient: &AccountId, now: &str)
            -> Vec<PartyInviteRecord>;
        fn accept_invite(&self, tx: &mut Tx<'_>, accept: &AcceptInvite) -> Option<PartyRecord>;
        fn leave(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> ();
        fn kick(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> ();
        fn promote(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> ();
        fn set_activity(&self, tx: &mut Tx<'_>, party: &GlobalId, expected_revision: i64, activity: PartyActivity,
            active_hunt_session_id: Option<&str>, now: &str) -> Option<PartyRecord>;
        fn append_event(&self, tx: &mut Tx<'_>, event: &PartyEvent) -> ();
        fn claim_next_outbox(&self, tx: &mut Tx<'_>, owner: &str, now: &str, stale_before: &str)
            -> Option<PartyOutboxRecord>;
        fn mark_outbox_published(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str, published_at: &str) -> bool;
        fn release_outbox(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str) -> ();
    }
}
