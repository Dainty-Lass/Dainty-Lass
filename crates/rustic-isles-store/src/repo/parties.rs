// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Parties, invites and the party outbox (spec §11.2, `PartyRepository.ts`). Every party change bumps its
//! revision (or fails with [`REVISION_CONFLICT`]) and writes its event in the same transaction.

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Nullable, Text};
use module_api::GlobalId;
use serde_json::{Map, Value};

use crate::sql::{account_id, count, sql};
use crate::{AccountId, AnyConnection, Json, RepoError, RepoResult, Tx, UserId};

/// The data error a lost revision race answers; the area retries when the error text contains it.
pub const REVISION_CONFLICT: &str = "PARTY_REVISION_CONFLICT";

macro_rules! text_enum {
    ($name:ident { $($variant:ident = $text:literal),* $(,)? }) => {
        /// A text column's values; each variant is stored as the text beside it in the invocation.
        #[allow(missing_docs, reason = "the invocation's variants take no doc comments; each stands beside its stored text")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),* }

        impl $name {
            /// The stored text.
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),* }
            }

            /// The value a stored text names; None for any other text.
            pub fn parse(text: &str) -> Option<Self> {
                match text { $($text => Some(Self::$variant),)* _ => None }
            }
        }
    };
}

text_enum!(PartyActivity {
    Idle = "IDLE",
    Matchmaking = "MATCHMAKING",
    HostSelecting = "HOST_SELECTING",
    StartingHunt = "STARTING_HUNT",
    InHunt = "IN_HUNT",
    Recovering = "RECOVERING",
});

text_enum!(InviteStatus {
    Pending = "PENDING",
    Accepted = "ACCEPTED",
    Revoked = "REVOKED",
    Expired = "EXPIRED",
});

text_enum!(PartyEventType {
    PartyCreated = "PARTY_CREATED",
    PartyInviteCreated = "PARTY_INVITE_CREATED",
    PartyMembershipChanged = "PARTY_MEMBERSHIP_CHANGED",
    PartyLeaderChanged = "PARTY_LEADER_CHANGED",
    PartyActivityChanged = "PARTY_ACTIVITY_CHANGED",
    PartyPresenceChanged = "PARTY_PRESENCE_CHANGED",
    PartyHuntStatusChanged = "PARTY_HUNT_STATUS_CHANGED",
});

text_enum!(OutboxState {
    Pending = "PENDING",
    Processing = "PROCESSING",
    Published = "PUBLISHED",
});

/// One party member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyMember {
    /// The member's account.
    pub account: AccountId,
    /// The member's userId.
    pub user_id: UserId,
}

/// A party with its members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyRecord {
    /// The party's network-wide id.
    pub id: GlobalId,
    /// Node's `partyId`: the `client_id` column
    pub party_id: String,
    /// Bumped on every change; a change names the revision it read.
    pub revision: i64,
    /// The party leader.
    pub leader: PartyMember,
    /// In join order
    pub members: Vec<PartyMember>,
    /// The game build the party was made on.
    pub build_id: String,
    /// What the party is doing.
    pub activity: PartyActivity,
    /// The hunt session it is in, if any.
    pub active_hunt_session_id: Option<String>,
    /// When it was created, as ISO text.
    pub created_at: String,
    /// When it last changed, as ISO text.
    pub updated_at: String,
}

impl PartyRecord {
    /// An event payload's party part.
    pub fn snapshot(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("partyId".into(), self.party_id.clone().into());
        out.insert("revision".into(), self.revision.into());
        out.insert("leaderPlayerId".into(), self.leader.user_id.as_str().to_owned().into());
        out.insert(
            "members".into(),
            self.members
                .iter()
                .map(|m| Value::from(m.user_id.as_str().to_owned()))
                .collect(),
        );
        out.insert("activity".into(), self.activity.as_str().into());
        out.insert(
            "activeHuntSessionId".into(),
            self.active_hunt_session_id.clone().map_or(Value::Null, Value::from),
        );
        out
    }

    fn user_ids(&self) -> Vec<UserId> {
        self.members.iter().map(|m| m.user_id.clone()).collect()
    }

    fn has(&self, account: &AccountId) -> bool {
        self.members.iter().any(|m| &m.account == account)
    }
}

/// A party invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyInviteRecord {
    /// `<recipient userId>` U+001F `<sender userId>`
    pub invite_id: String,
    /// The party's `partyId`.
    pub party_id: String,
    /// The party's revision when the invite was sent.
    pub party_revision: i64,
    /// Who sent it.
    pub sender: AccountId,
    /// Who it is for.
    pub recipient: AccountId,
    /// The sender's display name, as the client sent it.
    pub sending_display_name: String,
    /// The sender's platform, as the client sent it.
    pub sending_platform: String,
    /// Its state.
    pub status: InviteStatus,
    /// When it was sent, as ISO text.
    pub created_at: String,
    /// When it lapses, as ISO text.
    pub expires_at: String,
}

/// One party event in the outbox, delivered or not.
#[derive(Debug, Clone, PartialEq)]
pub struct PartyOutboxRecord {
    /// The event's id.
    pub event_id: String,
    /// The party's `partyId`.
    pub party_id: String,
    /// The party's revision the event describes.
    pub party_revision: i64,
    /// What happened.
    pub event_type: PartyEventType,
    /// Who is told.
    pub recipients: Vec<UserId>,
    /// The event body.
    pub payload: Json,
    /// Where delivery stands.
    pub state: OutboxState,
    /// When it was written, as ISO text.
    pub created_at: String,
    /// When a deliverer claimed it, as ISO text.
    pub claimed_at: Option<String>,
    /// The deliverer holding the claim.
    pub claim_owner: Option<String>,
    /// When it was delivered, as ISO text.
    pub published_at: Option<String>,
}

/// The ids a party made on demand will take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewParty {
    /// The network-wide id.
    pub id: GlobalId,
    /// The client-facing `partyId`.
    pub party_id: String,
}

/// An invite to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInvite {
    /// The inviting account.
    pub sender: AccountId,
    /// The invited account.
    pub recipient: AccountId,
    /// The game build, for a party made for the sender.
    pub build_id: String,
    /// The sender's display name.
    pub sender_display_name: String,
    /// The sender's platform.
    pub sender_platform: String,
    /// The ids of the sender's party if one must be made.
    pub new_party: NewParty,
    /// The current time, as ISO text.
    pub now: String,
    /// When the invite lapses, as ISO text.
    pub expires_at: String,
}

/// An invite to accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptInvite {
    /// The accepting account.
    pub recipient: AccountId,
    /// The account that sent the invite.
    pub inviter: AccountId,
    /// The game build, for a party made for the inviter.
    pub build_id: String,
    /// The ids of the inviter's party if one must be made.
    pub new_party: NewParty,
    /// The current time, as ISO text.
    pub now: String,
    /// The most members a party may have.
    pub max_party_size: u32,
}

/// A party event to write to the outbox.
#[derive(Debug, Clone, PartialEq)]
pub struct PartyEvent {
    /// The party's `partyId`.
    pub party_id: String,
    /// The party's revision it describes.
    pub party_revision: i64,
    /// What happened.
    pub event_type: PartyEventType,
    /// Who is told.
    pub recipients: Vec<UserId>,
    /// The event body.
    pub payload: Json,
    /// The current time, as ISO text.
    pub now: String,
}

store_trait! {
    /// Parties, invites and the outbox; every change bumps the party's revision and writes its event.
    pub trait PartyRepository = "parties" {
        /// The party with that network-wide id.
        fn find_by_id(&self, conn: &mut AnyConnection, id: &GlobalId) -> Option<PartyRecord>;
        /// The party with that client-facing `partyId`.
        fn find_by_party_id(&self, conn: &mut AnyConnection, client_id: &str) -> Option<PartyRecord>;
        /// The party the account is in.
        fn find_by_member(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<PartyRecord>;
        /// The account's party, made as a party of one (with its event) when it has none.
        fn get_or_create_solo(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            build_id: &str,
            new_party: &NewParty,
            now: &str,
        ) -> PartyRecord;
        /// Sends an invite from the sender's party, making the party if needed; sends nothing to oneself or to a member.
        fn create_invite(&self, tx: &mut Tx<'_>, invite: &NewInvite) -> ();
        /// The recipient's pending, unexpired invites, newest first.
        fn list_pending_invites(
            &self,
            conn: &mut AnyConnection,
            recipient: &AccountId,
            now: &str,
        ) -> Vec<PartyInviteRecord>;
        /// Moves the recipient into the inviter's party if the invite is live, the party issued it and has room; returns the recipient's party after.
        fn accept_invite(&self, tx: &mut Tx<'_>, accept: &AcceptInvite) -> Option<PartyRecord>;
        /// Takes the account out of its party.
        fn leave(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> ();
        /// The leader removes `target`; anyone else's call does nothing.
        fn kick(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> ();
        /// The leader hands leadership to member `target`; anyone else's call does nothing.
        fn promote(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> ();
        /// Sets the activity and hunt session if the revision is still `expected`; a lost race is a [`REVISION_CONFLICT`] error.
        fn set_activity(
            &self,
            tx: &mut Tx<'_>,
            id: &GlobalId,
            expected: i64,
            activity: PartyActivity,
            hunt_session: Option<&str>,
            now: &str,
        ) -> Option<PartyRecord>;
        /// Writes an event to the outbox.
        fn append_event(&self, tx: &mut Tx<'_>, event: &PartyEvent) -> ();
        /// Claims the oldest pending event, or one whose claim went stale before `stale_before`, for `owner`.
        fn claim_next_outbox(
            &self,
            tx: &mut Tx<'_>,
            owner: &str,
            now: &str,
            stale_before: &str,
        ) -> Option<PartyOutboxRecord>;
        /// Marks the event delivered if `owner` still holds it; returns whether it did.
        fn mark_outbox_published(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str, at: &str) -> bool;
        /// Gives the event back to the queue if `owner` holds it.
        fn release_outbox(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str) -> ();
    }
}

/// [`PartyRepository`] in SQL.
pub struct SqlParties;

// ------------------------------------------------------------------ reading

#[derive(QueryableByName)]
struct PartyRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    client_id: String,
    #[diesel(sql_type = BigInt)]
    revision: i64,
    #[diesel(sql_type = Text)]
    leader: String,
    #[diesel(sql_type = Text)]
    build_id: String,
    #[diesel(sql_type = Text)]
    activity: String,
    #[diesel(sql_type = Nullable<Text>)]
    active_hunt_session_id: Option<String>,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

#[derive(QueryableByName)]
struct MemberRow {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Nullable<Text>)]
    user_id: Option<String>,
}

/// The userId of an account, soft-deleted ones included.
fn user_id_of(conn: &mut AnyConnection, account: &str, missing: impl FnOnce() -> String) -> RepoResult<UserId> {
    sql("SELECT user_id AS t FROM accounts WHERE id = ?")
        .t(account)
        .first::<crate::sql::TextRow>(conn)?
        .map(|r| UserId::new(r.t))
        .ok_or_else(|| RepoError::Data(missing()))
}

fn decode_party(conn: &mut AnyConnection, row: PartyRow) -> RepoResult<PartyRecord> {
    let members = sql(
        "SELECT m.account AS account, a.user_id AS user_id FROM party_members m LEFT JOIN accounts a \
         ON a.id = m.account WHERE m.party = ? ORDER BY m.position ASC",
    )
    .t(&*row.id)
    .load::<MemberRow>(conn)?
    .into_iter()
    .map(|m| {
        let user_id = m
            .user_id
            .ok_or_else(|| RepoError::data(format!("party member {} has no account", m.account)))?;
        Ok(PartyMember {
            account: account_id(&m.account, "party_members.account")?,
            user_id: UserId::new(user_id),
        })
    })
    .collect::<RepoResult<Vec<_>>>()?;
    let leader = match members.iter().find(|m| m.account.to_string() == row.leader) {
        Some(member) => member.clone(),
        None => PartyMember {
            account: account_id(&row.leader, "parties.leader")?,
            user_id: user_id_of(conn, &row.leader, || {
                format!("party member {} has no account", row.leader)
            })?,
        },
    };
    Ok(PartyRecord {
        id: row
            .id
            .parse()
            .map_err(|_| RepoError::data(format!("parties.id \"{}\" is not a global id", row.id)))?,
        activity: PartyActivity::parse(&row.activity)
            .ok_or_else(|| RepoError::data(format!("parties.activity \"{}\"", row.activity)))?,
        party_id: row.client_id,
        revision: row.revision,
        leader,
        members,
        build_id: row.build_id,
        active_hunt_session_id: row.active_hunt_session_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn party_where(conn: &mut AnyConnection, clause: &str, value: &str) -> RepoResult<Option<PartyRecord>> {
    match sql(format!("SELECT * FROM parties WHERE {clause} = ?"))
        .t(value)
        .first::<PartyRow>(conn)?
    {
        Some(row) => decode_party(conn, row).map(Some),
        None => Ok(None),
    }
}

fn party_of(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<PartyRecord>> {
    let Some(party) = sql("SELECT party AS t FROM party_members WHERE account = ?")
        .t(account.to_string())
        .first::<crate::sql::TextRow>(conn)?
    else {
        return Ok(None);
    };
    party_where(conn, "id", &party.t)
}

fn reread(conn: &mut AnyConnection, id: &GlobalId) -> RepoResult<PartyRecord> {
    party_where(conn, "id", &id.to_string())?.ok_or_else(|| RepoError::data(REVISION_CONFLICT))
}

// ------------------------------------------------------------------ writing

/// The outbox write every party change ends with (system randomness for the id: spec §16 item 11).
fn write_event(conn: &mut AnyConnection, event: &PartyEvent) -> RepoResult<()> {
    let mut recipients: Vec<&str> = Vec::new();
    for r in &event.recipients {
        if !recipients.contains(&r.as_str()) {
            recipients.push(r.as_str());
        }
    }
    let seq = sql("SELECT MAX(seq) AS n FROM party_outbox").int(conn)? + 1;
    sql(
        "INSERT INTO party_outbox (id, party, party_revision, event_type, recipients, payload, state, created_at, seq) \
         VALUES (?, ?, ?, ?, ?, ?, 'PENDING', ?, ?)",
    )
    .t(rustic_isles::replay::random_uuid())
    .t(&*event.party_id)
    .i(event.party_revision)
    .t(event.event_type.as_str())
    .t(Value::from(recipients).to_string())
    .t(event.payload.to_string())
    .t(&*event.now)
    .i(seq)
    .run(conn)?;
    Ok(())
}

fn party_event(
    party: &PartyRecord,
    event_type: PartyEventType,
    recipients: Vec<UserId>,
    payload: Map<String, Value>,
    now: &str,
) -> PartyEvent {
    PartyEvent {
        party_id: party.party_id.clone(),
        party_revision: party.revision,
        event_type,
        recipients,
        payload: Value::Object(payload),
        now: now.to_owned(),
    }
}

/// Moves the party from the revision it was read at to + 1 with `leader`; a lost race is the conflict error.
fn bump(conn: &mut AnyConnection, party: &PartyRecord, leader: &AccountId, now: &str) -> RepoResult<()> {
    let moved = sql("UPDATE parties SET revision = ?, leader = ?, updated_at = ? WHERE id = ? AND revision = ?")
        .i(party.revision + 1)
        .t(leader.to_string())
        .t(now)
        .t(party.id.to_string())
        .i(party.revision)
        .run(conn)?;
    if moved == 0 {
        return Err(RepoError::data(REVISION_CONFLICT));
    }
    Ok(())
}

fn get_or_create_solo(
    conn: &mut AnyConnection,
    account: &AccountId,
    build_id: &str,
    new_party: &NewParty,
    now: &str,
) -> RepoResult<PartyRecord> {
    if let Some(party) = party_of(conn, account)? {
        return Ok(party);
    }
    let user_id = user_id_of(conn, &account.to_string(), || format!("{account} has no account"))?;
    sql(
        "INSERT INTO parties (id, client_id, revision, leader, build_id, activity, created_at, updated_at) \
         VALUES (?, ?, 1, ?, ?, 'IDLE', ?, ?)",
    )
    .t(new_party.id.to_string())
    .t(&*new_party.party_id)
    .t(account.to_string())
    .t(build_id)
    .t(now)
    .t(now)
    .run(conn)?;
    sql("INSERT INTO party_members (account, party, position, joined_at) VALUES (?, ?, 0, ?)")
        .t(account.to_string())
        .t(new_party.id.to_string())
        .t(now)
        .run(conn)?;
    let party = reread(conn, &new_party.id)?;
    let event = party_event(
        &party,
        PartyEventType::PartyCreated,
        vec![user_id],
        party.snapshot(),
        now,
    );
    write_event(conn, &event)?;
    Ok(party)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Removal {
    Leave,
    Kick,
    Move,
}

impl Removal {
    fn as_str(self) -> &'static str {
        match self {
            Self::Leave => "LEAVE",
            Self::Kick => "KICK",
            Self::Move => "MOVE",
        }
    }
}

fn remove_member(
    conn: &mut AnyConnection,
    party: &PartyRecord,
    target: &PartyMember,
    now: &str,
    removal: Removal,
    actor: Option<&PartyMember>,
) -> RepoResult<()> {
    sql("DELETE FROM party_members WHERE account = ?")
        .t(target.account.to_string())
        .run(conn)?;
    let remaining: Vec<&PartyMember> = party.members.iter().filter(|m| m.account != target.account).collect();
    let Some(first) = remaining.first() else {
        // At its read revision, unchecked
        sql("DELETE FROM parties WHERE id = ?")
            .t(party.id.to_string())
            .run(conn)?;
        let mut payload = Map::new();
        payload.insert("action".into(), "DISBAND".into());
        payload.insert("accountId".into(), target.user_id.as_str().to_owned().into());
        if let Some(actor) = actor {
            payload.insert("actorAccountId".into(), actor.user_id.as_str().to_owned().into());
        }
        payload.insert("members".into(), Value::Array(Vec::new()));
        let event = PartyEvent {
            party_id: party.party_id.clone(),
            party_revision: party.revision + 1,
            event_type: PartyEventType::PartyMembershipChanged,
            recipients: vec![target.user_id.clone()],
            payload: Value::Object(payload),
            now: now.to_owned(),
        };
        return write_event(conn, &event);
    };
    let leader = if party.leader.account == target.account {
        first.account.clone()
    } else {
        party.leader.account.clone()
    };
    bump(conn, party, &leader, now)?;
    let updated = reread(conn, &party.id)?;
    let mut payload = updated.snapshot();
    payload.insert("action".into(), removal.as_str().into());
    payload.insert("accountId".into(), target.user_id.as_str().to_owned().into());
    if let Some(actor) = actor {
        payload.insert("actorAccountId".into(), actor.user_id.as_str().to_owned().into());
    }
    let mut recipients: Vec<UserId> = remaining.iter().map(|m| m.user_id.clone()).collect();
    recipients.push(target.user_id.clone());
    let event = party_event(
        &updated,
        PartyEventType::PartyMembershipChanged,
        recipients,
        payload,
        now,
    );
    write_event(conn, &event)
}

/// The recipient's PENDING invite from the inviter is accepted; every other PENDING one of theirs revoked.
fn consume_invites(conn: &mut AnyConnection, recipient: &AccountId, inviter: &AccountId, now: &str) -> RepoResult<()> {
    sql(
        "UPDATE party_invites SET status = 'ACCEPTED', consumed_at = ? WHERE recipient = ? AND sender = ? \
         AND status = 'PENDING'",
    )
    .t(now)
    .t(recipient.to_string())
    .t(inviter.to_string())
    .run(conn)?;
    sql("UPDATE party_invites SET status = 'REVOKED', consumed_at = ? WHERE recipient = ? AND status = 'PENDING'")
        .t(now)
        .t(recipient.to_string())
        .run(conn)?;
    Ok(())
}

#[derive(QueryableByName)]
struct InviteRow {
    #[diesel(sql_type = Text)]
    recipient: String,
    #[diesel(sql_type = Text)]
    sender: String,
    #[diesel(sql_type = Text)]
    party: String,
    #[diesel(sql_type = BigInt)]
    party_revision: i64,
    #[diesel(sql_type = Text)]
    sending_display_name: String,
    #[diesel(sql_type = Text)]
    sending_platform: String,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    expires_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    client_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    recipient_user_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    sender_user_id: Option<String>,
}

fn invite_id(recipient: &str, sender: &str) -> String {
    format!("{recipient}\u{1f}{sender}")
}

#[derive(QueryableByName)]
struct OutboxRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    party: String,
    #[diesel(sql_type = BigInt)]
    party_revision: i64,
    #[diesel(sql_type = Text)]
    event_type: String,
    #[diesel(sql_type = Text)]
    recipients: String,
    #[diesel(sql_type = Text)]
    payload: String,
    #[diesel(sql_type = Text)]
    state: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    claimed_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    claim_owner: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    published_at: Option<String>,
}

impl OutboxRow {
    fn decode(self) -> RepoResult<PartyOutboxRecord> {
        let recipients: Vec<String> =
            serde_json::from_str(&self.recipients).map_err(|e| RepoError::data(format!("outbox recipients: {e}")))?;
        let payload: Value =
            serde_json::from_str(&self.payload).map_err(|e| RepoError::data(format!("outbox payload: {e}")))?;
        Ok(PartyOutboxRecord {
            event_type: PartyEventType::parse(&self.event_type)
                .ok_or_else(|| RepoError::data(format!("party_outbox.event_type \"{}\"", self.event_type)))?,
            state: OutboxState::parse(&self.state)
                .ok_or_else(|| RepoError::data(format!("party_outbox.state \"{}\"", self.state)))?,
            event_id: self.id,
            party_id: self.party,
            party_revision: self.party_revision,
            recipients: recipients.into_iter().map(UserId::new).collect(),
            payload,
            created_at: self.created_at,
            claimed_at: self.claimed_at,
            claim_owner: self.claim_owner,
            published_at: self.published_at,
        })
    }
}

impl PartyRepository for SqlParties {
    fn find_by_id(&self, conn: &mut AnyConnection, id: &GlobalId) -> RepoResult<Option<PartyRecord>> {
        party_where(conn, "id", &id.to_string())
    }

    fn find_by_party_id(&self, conn: &mut AnyConnection, client_id: &str) -> RepoResult<Option<PartyRecord>> {
        party_where(conn, "client_id", client_id)
    }

    fn find_by_member(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<PartyRecord>> {
        party_of(conn, account)
    }

    fn get_or_create_solo(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        build_id: &str,
        new_party: &NewParty,
        now: &str,
    ) -> RepoResult<PartyRecord> {
        get_or_create_solo(tx.conn(), account, build_id, new_party, now)
    }

    fn create_invite(&self, tx: &mut Tx<'_>, invite: &NewInvite) -> RepoResult<()> {
        if invite.sender == invite.recipient {
            return Ok(());
        }
        let conn = tx.conn();
        let party = get_or_create_solo(conn, &invite.sender, &invite.build_id, &invite.new_party, &invite.now)?;
        if party.has(&invite.recipient) {
            return Ok(());
        }
        let sender_user = user_id_of(conn, &invite.sender.to_string(), || {
            format!("{} has no account", invite.sender)
        })?;
        let recipient_user = user_id_of(conn, &invite.recipient.to_string(), || {
            format!("{} has no account", invite.recipient)
        })?;
        let seq = sql("SELECT MAX(seq) AS n FROM party_invites").int(conn)? + 1;
        // An update keeps the old consumed_at
        sql(
            "INSERT INTO party_invites (recipient, sender, party, party_revision, sending_display_name, \
             sending_platform, status, created_at, expires_at, seq) VALUES (?, ?, ?, ?, ?, ?, 'PENDING', ?, ?, ?) \
             ON CONFLICT (recipient, sender) DO UPDATE SET party = excluded.party, \
             party_revision = excluded.party_revision, sending_display_name = excluded.sending_display_name, \
             sending_platform = excluded.sending_platform, status = excluded.status, \
             created_at = excluded.created_at, expires_at = excluded.expires_at, seq = excluded.seq",
        )
        .t(invite.recipient.to_string())
        .t(invite.sender.to_string())
        .t(party.id.to_string())
        .i(party.revision)
        .t(&*invite.sender_display_name)
        .t(&*invite.sender_platform)
        .t(&*invite.now)
        .t(&*invite.expires_at)
        .i(seq)
        .run(conn)?;
        let mut payload = Map::new();
        payload.insert("partyId".into(), party.party_id.clone().into());
        payload.insert("partyRevision".into(), party.revision.into());
        payload.insert("sendingPlayerId".into(), sender_user.as_str().to_owned().into());
        payload.insert("recipientPlayerId".into(), recipient_user.as_str().to_owned().into());
        payload.insert("sendingDisplayName".into(), invite.sender_display_name.clone().into());
        payload.insert("sendingPlatform".into(), invite.sender_platform.clone().into());
        payload.insert("status".into(), "PENDING".into());
        payload.insert("createdAt".into(), invite.now.clone().into());
        payload.insert("expiresAt".into(), invite.expires_at.clone().into());
        payload.insert(
            "inviteId".into(),
            invite_id(recipient_user.as_str(), sender_user.as_str()).into(),
        );
        let event = party_event(
            &party,
            PartyEventType::PartyInviteCreated,
            vec![recipient_user],
            payload,
            &invite.now,
        );
        write_event(conn, &event)
    }

    fn list_pending_invites(
        &self,
        conn: &mut AnyConnection,
        recipient: &AccountId,
        now: &str,
    ) -> RepoResult<Vec<PartyInviteRecord>> {
        let rows = sql(
            "SELECT i.*, p.client_id AS client_id, r.user_id AS recipient_user_id, s.user_id AS sender_user_id \
             FROM party_invites i LEFT JOIN parties p ON p.id = i.party \
             LEFT JOIN accounts r ON r.id = i.recipient LEFT JOIN accounts s ON s.id = i.sender \
             WHERE i.recipient = ? AND i.status = 'PENDING' AND i.expires_at > ? \
             ORDER BY i.created_at DESC, i.seq DESC",
        )
        .t(recipient.to_string())
        .t(now)
        .load::<InviteRow>(conn)?;
        rows.into_iter()
            .map(|row| {
                let recipient_user = row
                    .recipient_user_id
                    .ok_or_else(|| RepoError::data(format!("{} has no account", row.recipient)))?;
                let sender_user = row
                    .sender_user_id
                    .ok_or_else(|| RepoError::data(format!("{} has no account", row.sender)))?;
                Ok(PartyInviteRecord {
                    invite_id: invite_id(&recipient_user, &sender_user),
                    party_id: row.client_id.unwrap_or(row.party),
                    party_revision: row.party_revision,
                    sender: account_id(&row.sender, "party_invites.sender")?,
                    recipient: account_id(&row.recipient, "party_invites.recipient")?,
                    sending_display_name: row.sending_display_name,
                    sending_platform: row.sending_platform,
                    status: InviteStatus::parse(&row.status)
                        .ok_or_else(|| RepoError::data(format!("party_invites.status \"{}\"", row.status)))?,
                    created_at: row.created_at,
                    expires_at: row.expires_at,
                })
            })
            .collect()
    }

    fn accept_invite(&self, tx: &mut Tx<'_>, a: &AcceptInvite) -> RepoResult<Option<PartyRecord>> {
        let conn = tx.conn();
        let invite = sql(
            "SELECT party AS t FROM party_invites WHERE recipient = ? AND sender = ? \
             AND status = 'PENDING' AND expires_at > ?",
        )
        .t(a.recipient.to_string())
        .t(a.inviter.to_string())
        .t(&*a.now)
        .first::<crate::sql::TextRow>(conn)?;
        let Some(invite) = invite else {
            return party_of(conn, &a.recipient);
        };
        let party = get_or_create_solo(conn, &a.inviter, &a.build_id, &a.new_party, &a.now)?;
        // An invite belongs to the party that issued it
        if party.id.to_string() != invite.t {
            return party_of(conn, &a.recipient);
        }
        if party.has(&a.recipient) {
            consume_invites(conn, &a.recipient, &a.inviter, &a.now)?;
            return reread(conn, &party.id).map(Some);
        }
        if party.members.len() >= a.max_party_size as usize {
            return party_of(conn, &a.recipient);
        }
        if let Some(current) = party_of(conn, &a.recipient)?
            && let Some(me) = current.members.iter().find(|m| m.account == a.recipient).cloned()
        {
            remove_member(conn, &current, &me, &a.now, Removal::Move, None)?;
        }
        bump(conn, &party, &party.leader.account, &a.now)?;
        let position = sql("SELECT MAX(position) AS n FROM party_members WHERE party = ?")
            .t(party.id.to_string())
            .first::<crate::sql::IntRow>(conn)?
            .and_then(|r| r.n)
            .map_or(0, |n| n + 1);
        sql("INSERT INTO party_members (account, party, position, joined_at) VALUES (?, ?, ?, ?)")
            .t(a.recipient.to_string())
            .t(party.id.to_string())
            .i(position)
            .t(&*a.now)
            .run(conn)?;
        consume_invites(conn, &a.recipient, &a.inviter, &a.now)?;
        let updated = reread(conn, &party.id)?;
        let joiner = updated
            .members
            .iter()
            .find(|m| m.account == a.recipient)
            .map(|m| m.user_id.as_str().to_owned())
            .unwrap_or_default();
        let mut payload = updated.snapshot();
        payload.insert("action".into(), "JOIN".into());
        payload.insert("accountId".into(), joiner.into());
        let event = party_event(
            &updated,
            PartyEventType::PartyMembershipChanged,
            updated.user_ids(),
            payload,
            &a.now,
        );
        write_event(conn, &event)?;
        Ok(Some(updated))
    }

    fn leave(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> RepoResult<()> {
        let conn = tx.conn();
        let Some(party) = party_of(conn, account)? else {
            return Ok(());
        };
        let Some(me) = party.members.iter().find(|m| &m.account == account).cloned() else {
            return Ok(());
        };
        remove_member(conn, &party, &me, now, Removal::Leave, None)
    }

    fn kick(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> RepoResult<()> {
        let conn = tx.conn();
        let Some(party) = party_of(conn, actor)? else {
            return Ok(());
        };
        if &party.leader.account != actor || actor == target {
            return Ok(());
        }
        let Some(member) = party.members.iter().find(|m| &m.account == target).cloned() else {
            return Ok(());
        };
        let leader = party.leader.clone();
        remove_member(conn, &party, &member, now, Removal::Kick, Some(&leader))
    }

    fn promote(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> RepoResult<()> {
        let conn = tx.conn();
        let Some(party) = party_of(conn, actor)? else {
            return Ok(());
        };
        if &party.leader.account != actor || actor == target || !party.has(target) {
            return Ok(());
        }
        bump(conn, &party, target, now)?;
        let updated = reread(conn, &party.id)?;
        let event = party_event(
            &updated,
            PartyEventType::PartyLeaderChanged,
            updated.user_ids(),
            updated.snapshot(),
            now,
        );
        write_event(conn, &event)
    }

    fn set_activity(
        &self,
        tx: &mut Tx<'_>,
        id: &GlobalId,
        expected: i64,
        activity: PartyActivity,
        hunt_session: Option<&str>,
        now: &str,
    ) -> RepoResult<Option<PartyRecord>> {
        let conn = tx.conn();
        match party_where(conn, "id", &id.to_string())? {
            Some(party) if party.revision == expected => {}
            _ => return Ok(None),
        }
        let moved = sql(
            "UPDATE parties SET revision = ?, activity = ?, active_hunt_session_id = ?, updated_at = ? \
             WHERE id = ? AND revision = ?",
        )
        .i(expected + 1)
        .t(activity.as_str())
        .ot(hunt_session)
        .t(now)
        .t(id.to_string())
        .i(expected)
        .run(conn)?;
        if moved == 0 {
            return Err(RepoError::data(REVISION_CONFLICT));
        }
        let updated = reread(conn, id)?;
        let event = party_event(
            &updated,
            PartyEventType::PartyActivityChanged,
            updated.user_ids(),
            updated.snapshot(),
            now,
        );
        write_event(conn, &event)?;
        Ok(Some(updated))
    }

    fn append_event(&self, tx: &mut Tx<'_>, event: &PartyEvent) -> RepoResult<()> {
        write_event(tx.conn(), event)
    }

    fn claim_next_outbox(
        &self,
        tx: &mut Tx<'_>,
        owner: &str,
        now: &str,
        stale_before: &str,
    ) -> RepoResult<Option<PartyOutboxRecord>> {
        let conn = tx.conn();
        let Some(next) = sql("SELECT id AS t FROM party_outbox WHERE state = 'PENDING' \
             OR (state = 'PROCESSING' AND claimed_at < ?) ORDER BY created_at ASC, seq ASC LIMIT 1")
        .t(stale_before)
        .first::<crate::sql::TextRow>(conn)?
        else {
            return Ok(None);
        };
        sql("UPDATE party_outbox SET state = 'PROCESSING', claim_owner = ?, claimed_at = ? WHERE id = ?")
            .t(owner)
            .t(now)
            .t(&*next.t)
            .run(conn)?;
        sql("SELECT * FROM party_outbox WHERE id = ?")
            .t(next.t)
            .first::<OutboxRow>(conn)?
            .map(OutboxRow::decode)
            .transpose()
    }

    fn mark_outbox_published(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str, at: &str) -> RepoResult<bool> {
        let changed = sql(
            "UPDATE party_outbox SET state = 'PUBLISHED', published_at = ?, claim_owner = NULL, \
             claimed_at = NULL WHERE id = ? AND state = 'PROCESSING' AND claim_owner = ?",
        )
        .t(at)
        .t(event_id)
        .t(owner)
        .run(tx.conn())?;
        Ok(changed == 1)
    }

    fn release_outbox(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str) -> RepoResult<()> {
        sql(
            "UPDATE party_outbox SET state = 'PENDING', claim_owner = NULL, claimed_at = NULL \
             WHERE id = ? AND state = 'PROCESSING' AND claim_owner = ?",
        )
        .t(event_id)
        .t(owner)
        .run(tx.conn())?;
        Ok(())
    }
}

/// The erasure's party step (spec §11.3.6): memberships deleted, a led party handed to its first remaining member
/// (no bump, no event), an emptied party removed with its outbox rows. The count: memberships plus parties removed.
pub(crate) fn erase_memberships(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<u64> {
    let parties: Vec<String> = sql("SELECT party AS t FROM party_members WHERE account = ?")
        .t(account.to_string())
        .load::<crate::sql::TextRow>(conn)?
        .into_iter()
        .map(|r| r.t)
        .collect();
    let mut total = count(
        sql("DELETE FROM party_members WHERE account = ?")
            .t(account.to_string())
            .run(conn)?,
    );
    for party in parties {
        let first = sql("SELECT account AS t FROM party_members WHERE party = ? ORDER BY position ASC LIMIT 1")
            .t(&*party)
            .first::<crate::sql::TextRow>(conn)?;
        match first {
            Some(next) => {
                sql("UPDATE parties SET leader = ? WHERE id = ? AND leader = ?")
                    .t(next.t)
                    .t(&*party)
                    .t(account.to_string())
                    .run(conn)?;
            }
            None => {
                // The outbox names a party by its client id; a global id is matched too
                sql(
                    "DELETE FROM party_outbox WHERE party = ? OR party IN (SELECT client_id FROM parties WHERE id = ?)",
                )
                .t(&*party)
                .t(&*party)
                .run(conn)?;
                total += count(sql("DELETE FROM parties WHERE id = ?").t(&*party).run(conn)?);
            }
        }
    }
    Ok(total)
}
