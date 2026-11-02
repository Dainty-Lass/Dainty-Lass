//! [`PartyRepository`] on `store`: a port of `SqlitePartyRepository` over Diesel's portable DSL, so it runs on both
//! engines (DD15). Each public method is one write, a part of the caller's transaction; the change log (DD23) and the
//! outbox are written in it.

use std::collections::HashMap;

use diesel::dsl::max;
use diesel::prelude::*;
use module_api::GlobalId;
use serde_json::{Map, Value, json};

use super::{
    AcceptInvite, InviteStatus, NewInvite, NewParty, OutboxState, PartyActivity, PartyEvent, PartyEventType,
    PartyInviteRecord, PartyMember, PartyOutboxRecord, PartyRecord, PartyRepository, REVISION_CONFLICT,
};
use crate::repo::{AccountId, AnyConnection, RepoError, RepoResult, Tx, UserId};

diesel::table! {
    parties (id) {
        id -> Text,
        client_id -> Text,
        revision -> BigInt,
        leader -> Text,
        build_id -> Text,
        activity -> Text,
        active_hunt_session_id -> Nullable<Text>,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    party_members (account) {
        account -> Text,
        party -> Text,
        position -> BigInt,
        joined_at -> Text,
    }
}

diesel::table! {
    party_invites (recipient, sender) {
        recipient -> Text,
        sender -> Text,
        party -> Text,
        party_revision -> BigInt,
        sending_display_name -> Text,
        sending_platform -> Text,
        status -> Text,
        created_at -> Text,
        expires_at -> Text,
        consumed_at -> Nullable<Text>,
        seq -> BigInt,
    }
}

diesel::table! {
    party_outbox (id) {
        id -> Text,
        party -> Text,
        party_revision -> BigInt,
        event_type -> Text,
        recipients -> Text,
        payload -> Text,
        state -> Text,
        created_at -> Text,
        claimed_at -> Nullable<Text>,
        claim_owner -> Nullable<Text>,
        published_at -> Nullable<Text>,
        seq -> BigInt,
    }
}

// Only the columns a party needs from RU4's accounts table
diesel::table! {
    accounts (id) {
        id -> Text,
        user_id -> Text,
    }
}

type PartyRow = (
    String,
    String,
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
);

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlParties;

fn data(message: impl Into<String>) -> RepoError {
    RepoError::Data(message.into())
}

fn account_id(text: &str) -> RepoResult<AccountId> {
    text.parse().map_err(|_| data(format!("{text:?} is not an account id")))
}

fn party_global(text: &str) -> RepoResult<GlobalId> {
    text.parse().map_err(|_| data(format!("{text:?} is not a party id")))
}

/// `userId`s by account id text; a deleted account keeps its row, so a party can still name it.
fn user_ids(conn: &mut AnyConnection, accounts: &[String]) -> RepoResult<HashMap<String, String>> {
    if accounts.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(self::accounts::table
        .filter(self::accounts::id.eq_any(accounts))
        .select((self::accounts::id, self::accounts::user_id))
        .load::<(String, String)>(conn)?
        .into_iter()
        .collect())
}

fn member(account: &str, users: &HashMap<String, String>) -> RepoResult<PartyMember> {
    let user = users
        .get(account)
        .ok_or_else(|| data(format!("party member {account} has no account")))?;
    Ok(PartyMember {
        account: account_id(account)?,
        user_id: UserId(user.clone()),
    })
}

fn user_of(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<String> {
    let text = account.to_string();
    user_ids(conn, std::slice::from_ref(&text))?
        .remove(&text)
        .ok_or_else(|| data(format!("{text} has no account")))
}

fn record(conn: &mut AnyConnection, row: PartyRow) -> RepoResult<PartyRecord> {
    let (id, party_id, revision, leader, build_id, activity, hunt, created_at, updated_at) = row;
    let accounts: Vec<String> = party_members::table
        .filter(party_members::party.eq(&id))
        .order(party_members::position.asc())
        .select(party_members::account)
        .load(conn)?;
    let mut wanted = accounts.clone();
    wanted.push(leader.clone());
    let users = user_ids(conn, &wanted)?;
    Ok(PartyRecord {
        id: party_global(&id)?,
        party_id,
        revision,
        leader: member(&leader, &users)?,
        members: accounts
            .iter()
            .map(|account| member(account, &users))
            .collect::<RepoResult<_>>()?,
        build_id,
        activity: PartyActivity::parse(&activity).ok_or_else(|| data(format!("party activity {activity:?}")))?,
        active_hunt_session_id: hunt,
        created_at,
        updated_at,
    })
}

fn find_by_id(conn: &mut AnyConnection, id: &str) -> RepoResult<Option<PartyRecord>> {
    let row = parties::table
        .find(id)
        .select(parties::all_columns)
        .first::<PartyRow>(conn)
        .optional()?;
    row.map(|row| record(conn, row)).transpose()
}

fn find_by_member(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<PartyRecord>> {
    let party = party_members::table
        .find(account.to_string())
        .select(party_members::party)
        .first::<String>(conn)
        .optional()?;
    match party {
        Some(party) => find_by_id(conn, &party),
        None => Ok(None),
    }
}

/// `SnapshotPayload`: userIds and Node's `partyId`, as the client's events carry them.
fn snapshot(party: &PartyRecord) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("partyId".into(), json!(party.party_id));
    map.insert("revision".into(), json!(party.revision));
    map.insert("leaderPlayerId".into(), json!(party.leader.user_id.as_str()));
    map.insert(
        "members".into(),
        Value::Array(party.members.iter().map(|m| json!(m.user_id.as_str())).collect()),
    );
    map.insert("activity".into(), json!(party.activity.as_str()));
    map.insert(
        "activeHuntSessionId".into(),
        party
            .active_hunt_session_id
            .as_deref()
            .map_or(Value::Null, |id| json!(id)),
    );
    map
}

struct Event<'a> {
    party_id: &'a str,
    revision: i64,
    kind: PartyEventType,
    recipients: Vec<String>,
    payload: Value,
    now: &'a str,
}

fn write_event(conn: &mut AnyConnection, event: Event<'_>) -> RepoResult<()> {
    let mut seen = Vec::new();
    for recipient in event.recipients {
        if !seen.contains(&recipient) {
            seen.push(recipient);
        }
    }
    let seq = party_outbox::table
        .select(max(party_outbox::seq))
        .first::<Option<i64>>(conn)?
        .unwrap_or(0)
        + 1;
    diesel::insert_into(party_outbox::table)
        .values((
            party_outbox::id.eq(uuid::Uuid::new_v4().to_string()),
            party_outbox::party.eq(event.party_id),
            party_outbox::party_revision.eq(event.revision),
            party_outbox::event_type.eq(event.kind.as_str()),
            party_outbox::recipients.eq(Value::Array(seen.into_iter().map(Value::String).collect()).to_string()),
            party_outbox::payload.eq(event.payload.to_string()),
            party_outbox::state.eq(OutboxState::Pending.as_str()),
            party_outbox::created_at.eq(event.now),
            party_outbox::seq.eq(seq),
        ))
        .execute(conn)?;
    Ok(())
}

fn next_position(conn: &mut AnyConnection, party: &str) -> RepoResult<i64> {
    Ok(party_members::table
        .filter(party_members::party.eq(party))
        .select(max(party_members::position))
        .first::<Option<i64>>(conn)?
        .map_or(0, |last| last + 1))
}

fn get_or_create_solo(
    conn: &mut AnyConnection,
    account: &AccountId,
    build_id: &str,
    new_party: &NewParty,
    now: &str,
) -> RepoResult<PartyRecord> {
    if let Some(existing) = find_by_member(conn, account)? {
        return Ok(existing);
    }
    let user = user_of(conn, account)?;
    let id = new_party.id.to_string();
    diesel::insert_into(parties::table)
        .values((
            parties::id.eq(&id),
            parties::client_id.eq(&new_party.party_id),
            parties::revision.eq(1_i64),
            parties::leader.eq(account.to_string()),
            parties::build_id.eq(build_id),
            parties::activity.eq(PartyActivity::Idle.as_str()),
            parties::created_at.eq(now),
            parties::updated_at.eq(now),
        ))
        .execute(conn)?;
    diesel::insert_into(party_members::table)
        .values((
            party_members::account.eq(account.to_string()),
            party_members::party.eq(&id),
            party_members::position.eq(0_i64),
            party_members::joined_at.eq(now),
        ))
        .execute(conn)?;
    let party = find_by_id(conn, &id)?.ok_or_else(|| data("a party just created is missing"))?;
    write_event(
        conn,
        Event {
            party_id: &party.party_id,
            revision: party.revision,
            kind: PartyEventType::Created,
            recipients: vec![user],
            payload: Value::Object(snapshot(&party)),
            now,
        },
    )?;
    Ok(party)
}

/// Moves a party to its next revision only if it is still at the revision it was read at; anything else is a conflict.
fn bump(conn: &mut AnyConnection, previous: &PartyRecord, leader: &PartyMember, now: &str) -> RepoResult<()> {
    let changed = diesel::update(
        parties::table
            .filter(parties::id.eq(previous.id.to_string()))
            .filter(parties::revision.eq(previous.revision)),
    )
    .set((
        parties::revision.eq(previous.revision + 1),
        parties::leader.eq(leader.account.to_string()),
        parties::updated_at.eq(now),
    ))
    .execute(conn)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(data(REVISION_CONFLICT))
    }
}

fn consume_recipient_invites(conn: &mut AnyConnection, recipient: &str, inviter: &str, now: &str) -> RepoResult<()> {
    diesel::update(
        party_invites::table
            .filter(party_invites::recipient.eq(recipient))
            .filter(party_invites::sender.eq(inviter))
            .filter(party_invites::status.eq(InviteStatus::Pending.as_str())),
    )
    .set((
        party_invites::status.eq(InviteStatus::Accepted.as_str()),
        party_invites::consumed_at.eq(Some(now)),
    ))
    .execute(conn)?;
    diesel::update(
        party_invites::table
            .filter(party_invites::recipient.eq(recipient))
            .filter(party_invites::sender.ne(inviter))
            .filter(party_invites::status.eq(InviteStatus::Pending.as_str())),
    )
    .set((
        party_invites::status.eq(InviteStatus::Revoked.as_str()),
        party_invites::consumed_at.eq(Some(now)),
    ))
    .execute(conn)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum Removal {
    Leave,
    Kick,
    Move,
}

impl Removal {
    const fn action(self) -> &'static str {
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
    diesel::delete(party_members::table.find(target.account.to_string())).execute(conn)?;
    let remaining: Vec<&PartyMember> = party.members.iter().filter(|m| m.account != target.account).collect();
    let Some(first) = remaining.first() else {
        diesel::delete(
            parties::table
                .filter(parties::id.eq(party.id.to_string()))
                .filter(parties::revision.eq(party.revision)),
        )
        .execute(conn)?;
        let mut payload = Map::new();
        payload.insert("action".into(), json!("DISBAND"));
        payload.insert("accountId".into(), json!(target.user_id.as_str()));
        if let Some(actor) = actor {
            payload.insert("actorAccountId".into(), json!(actor.user_id.as_str()));
        }
        payload.insert("members".into(), json!([]));
        return write_event(
            conn,
            Event {
                party_id: &party.party_id,
                revision: party.revision + 1,
                kind: PartyEventType::MembershipChanged,
                recipients: vec![target.user_id.to_string()],
                payload: Value::Object(payload),
                now,
            },
        );
    };
    let leader = if party.leader.account == target.account {
        (*first).clone()
    } else {
        party.leader.clone()
    };
    bump(conn, party, &leader, now)?;
    let next = PartyRecord {
        revision: party.revision + 1,
        members: remaining.iter().map(|m| (*m).clone()).collect(),
        leader,
        updated_at: now.to_owned(),
        ..party.clone()
    };
    let mut payload = snapshot(&next);
    payload.insert("action".into(), json!(removal.action()));
    payload.insert("accountId".into(), json!(target.user_id.as_str()));
    if let Some(actor) = actor {
        payload.insert("actorAccountId".into(), json!(actor.user_id.as_str()));
    }
    let mut recipients: Vec<String> = next.members.iter().map(|m| m.user_id.to_string()).collect();
    recipients.push(target.user_id.to_string());
    write_event(
        conn,
        Event {
            party_id: &next.party_id,
            revision: next.revision,
            kind: PartyEventType::MembershipChanged,
            recipients,
            payload: Value::Object(payload),
            now,
        },
    )
}

type InviteRow = (String, String, String, i64, String, String, String, String, String);

fn invite(conn: &mut AnyConnection, row: InviteRow) -> RepoResult<PartyInviteRecord> {
    let (recipient, sender, party, revision, name, platform, status, created_at, expires_at) = row;
    let users = user_ids(conn, &[recipient.clone(), sender.clone()])?;
    let client_id = parties::table
        .find(&party)
        .select(parties::client_id)
        .first::<String>(conn)
        .optional()?;
    let sender_member = member(&sender, &users)?;
    let recipient_member = member(&recipient, &users)?;
    Ok(PartyInviteRecord {
        invite_id: format!("{}\u{1f}{}", recipient_member.user_id, sender_member.user_id),
        // A disbanded party's invite is dead anyway; its global id stands in for the client's
        party_id: client_id.unwrap_or(party),
        party_revision: revision,
        sender: sender_member,
        recipient: recipient_member,
        sending_display_name: name,
        sending_platform: platform,
        status: InviteStatus::parse(&status).ok_or_else(|| data(format!("invite status {status:?}")))?,
        created_at,
        expires_at,
    })
}

type OutboxRow = (
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn outbox(conn: &mut AnyConnection, id: &str) -> RepoResult<Option<PartyOutboxRecord>> {
    let row = party_outbox::table
        .find(id)
        .select((
            party_outbox::id,
            party_outbox::party,
            party_outbox::party_revision,
            party_outbox::event_type,
            party_outbox::recipients,
            party_outbox::payload,
            party_outbox::state,
            party_outbox::created_at,
            party_outbox::claimed_at,
            party_outbox::claim_owner,
            party_outbox::published_at,
        ))
        .first::<OutboxRow>(conn)
        .optional()?;
    row.map(
        |(
            event_id,
            party_id,
            party_revision,
            kind,
            recipients,
            payload,
            state,
            created_at,
            claimed_at,
            owner,
            done,
        )| {
            let recipients: Vec<String> =
                serde_json::from_str(&recipients).map_err(|e| data(format!("outbox recipients: {e}")))?;
            Ok(PartyOutboxRecord {
                event_id,
                party_id,
                party_revision,
                event_type: PartyEventType::parse(&kind).ok_or_else(|| data(format!("event type {kind:?}")))?,
                recipients: recipients.into_iter().map(UserId).collect(),
                payload: serde_json::from_str(&payload).map_err(|e| data(format!("outbox payload: {e}")))?,
                state: OutboxState::parse(&state).ok_or_else(|| data(format!("outbox state {state:?}")))?,
                created_at,
                claimed_at,
                claim_owner: owner,
                published_at: done,
            })
        },
    )
    .transpose()
}

impl PartyRepository for SqlParties {
    fn find_by_id(&self, conn: &mut AnyConnection, party: &GlobalId) -> RepoResult<Option<PartyRecord>> {
        find_by_id(conn, &party.to_string())
    }

    fn find_by_party_id(&self, conn: &mut AnyConnection, party_id: &str) -> RepoResult<Option<PartyRecord>> {
        let id = parties::table
            .filter(parties::client_id.eq(party_id))
            .select(parties::id)
            .first::<String>(conn)
            .optional()?;
        match id {
            Some(id) => find_by_id(conn, &id),
            None => Ok(None),
        }
    }

    fn find_by_member(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<PartyRecord>> {
        find_by_member(conn, account)
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

    fn create_invite(&self, tx: &mut Tx<'_>, new: &NewInvite) -> RepoResult<()> {
        if new.recipient == new.sender {
            return Ok(());
        }
        let conn = tx.conn();
        let party = get_or_create_solo(conn, &new.sender, &new.build_id, &new.new_party, &new.now)?;
        if find_by_member(conn, &new.recipient)?.is_some_and(|theirs| theirs.id == party.id) {
            return Ok(());
        }
        let recipient_user = user_of(conn, &new.recipient)?;
        let sender_user = user_of(conn, &new.sender)?;
        let (sender, recipient) = (new.sender.to_string(), new.recipient.to_string());
        let seq = party_invites::table
            .select(max(party_invites::seq))
            .first::<Option<i64>>(conn)?
            .unwrap_or(0)
            + 1;
        let existing = party_invites::table
            .find((&recipient, &sender))
            .select(party_invites::seq)
            .first::<i64>(conn)
            .optional()?;
        // An upsert that sets: an older answered invite keeps its consumed_at
        if existing.is_some() {
            diesel::update(party_invites::table.find((&recipient, &sender)))
                .set((
                    party_invites::party.eq(party.id.to_string()),
                    party_invites::party_revision.eq(party.revision),
                    party_invites::sending_display_name.eq(&new.sender_display_name),
                    party_invites::sending_platform.eq(&new.sender_platform),
                    party_invites::status.eq(InviteStatus::Pending.as_str()),
                    party_invites::created_at.eq(&new.now),
                    party_invites::expires_at.eq(&new.expires_at),
                    party_invites::seq.eq(seq),
                ))
                .execute(conn)?;
        } else {
            diesel::insert_into(party_invites::table)
                .values((
                    party_invites::recipient.eq(&recipient),
                    party_invites::sender.eq(&sender),
                    party_invites::party.eq(party.id.to_string()),
                    party_invites::party_revision.eq(party.revision),
                    party_invites::sending_display_name.eq(&new.sender_display_name),
                    party_invites::sending_platform.eq(&new.sender_platform),
                    party_invites::status.eq(InviteStatus::Pending.as_str()),
                    party_invites::created_at.eq(&new.now),
                    party_invites::expires_at.eq(&new.expires_at),
                    party_invites::seq.eq(seq),
                ))
                .execute(conn)?;
        }
        let mut payload = Map::new();
        payload.insert("partyId".into(), json!(party.party_id));
        payload.insert("partyRevision".into(), json!(party.revision));
        payload.insert("sendingPlayerId".into(), json!(sender_user));
        payload.insert("recipientPlayerId".into(), json!(recipient_user));
        payload.insert("sendingDisplayName".into(), json!(new.sender_display_name));
        payload.insert("sendingPlatform".into(), json!(new.sender_platform));
        payload.insert("status".into(), json!("PENDING"));
        payload.insert("createdAt".into(), json!(new.now));
        payload.insert("expiresAt".into(), json!(new.expires_at));
        payload.insert("inviteId".into(), json!(format!("{recipient_user}\u{1f}{sender_user}")));
        write_event(
            conn,
            Event {
                party_id: &party.party_id,
                revision: party.revision,
                kind: PartyEventType::InviteCreated,
                recipients: vec![recipient_user],
                payload: Value::Object(payload),
                now: &new.now,
            },
        )
    }

    fn list_pending_invites(
        &self,
        conn: &mut AnyConnection,
        recipient: &AccountId,
        now: &str,
    ) -> RepoResult<Vec<PartyInviteRecord>> {
        let rows = party_invites::table
            .filter(party_invites::recipient.eq(recipient.to_string()))
            .filter(party_invites::status.eq(InviteStatus::Pending.as_str()))
            .filter(party_invites::expires_at.gt(now))
            .order((party_invites::created_at.desc(), party_invites::seq.desc()))
            .select((
                party_invites::recipient,
                party_invites::sender,
                party_invites::party,
                party_invites::party_revision,
                party_invites::sending_display_name,
                party_invites::sending_platform,
                party_invites::status,
                party_invites::created_at,
                party_invites::expires_at,
            ))
            .load::<InviteRow>(conn)?;
        rows.into_iter().map(|row| invite(conn, row)).collect()
    }

    fn accept_invite(&self, tx: &mut Tx<'_>, accept: &AcceptInvite) -> RepoResult<Option<PartyRecord>> {
        let conn = tx.conn();
        let (recipient, inviter) = (accept.recipient.to_string(), accept.inviter.to_string());
        let pending = party_invites::table
            .find((&recipient, &inviter))
            .filter(party_invites::status.eq(InviteStatus::Pending.as_str()))
            .filter(party_invites::expires_at.gt(&accept.now))
            .select(party_invites::party)
            .first::<String>(conn)
            .optional()?;
        let Some(invited_party) = pending else {
            return find_by_member(conn, &accept.recipient);
        };
        let target = get_or_create_solo(conn, &accept.inviter, &accept.build_id, &accept.new_party, &accept.now)?;
        let current = find_by_member(conn, &accept.recipient)?;
        // An invite belongs to the issuing party, not merely to the inviter account: if the inviter left or
        // switched parties, the old invite cannot become a capability to join their new party.
        if target.id.to_string() != invited_party {
            return Ok(current);
        }
        if current.as_ref().is_some_and(|c| c.id == target.id) {
            consume_recipient_invites(conn, &recipient, &inviter, &accept.now)?;
            return Ok(Some(target));
        }
        if target.members.len() >= accept.max_party_size as usize {
            return Ok(current);
        }
        let users = user_ids(conn, std::slice::from_ref(&recipient))?;
        let joining = member(&recipient, &users)?;
        if let Some(current) = &current {
            remove_member(conn, current, &joining, &accept.now, Removal::Move, None)?;
        }
        bump(conn, &target, &target.leader, &accept.now)?;
        let position = next_position(conn, &target.id.to_string())?;
        diesel::insert_into(party_members::table)
            .values((
                party_members::account.eq(&recipient),
                party_members::party.eq(target.id.to_string()),
                party_members::position.eq(position),
                party_members::joined_at.eq(&accept.now),
            ))
            .execute(conn)?;
        consume_recipient_invites(conn, &recipient, &inviter, &accept.now)?;
        let mut members = target.members.clone();
        members.push(joining.clone());
        let next = PartyRecord {
            revision: target.revision + 1,
            members,
            updated_at: accept.now.clone(),
            ..target
        };
        let mut payload = snapshot(&next);
        payload.insert("action".into(), json!("JOIN"));
        payload.insert("accountId".into(), json!(joining.user_id.as_str()));
        write_event(
            conn,
            Event {
                party_id: &next.party_id,
                revision: next.revision,
                kind: PartyEventType::MembershipChanged,
                recipients: next.members.iter().map(|m| m.user_id.to_string()).collect(),
                payload: Value::Object(payload),
                now: &accept.now,
            },
        )?;
        Ok(Some(next))
    }

    fn leave(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> RepoResult<()> {
        let conn = tx.conn();
        let Some(party) = find_by_member(conn, account)? else {
            return Ok(());
        };
        let Some(me) = party.members.iter().find(|m| &m.account == account).cloned() else {
            return Ok(());
        };
        remove_member(conn, &party, &me, now, Removal::Leave, None)
    }

    fn kick(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> RepoResult<()> {
        let conn = tx.conn();
        let Some(party) = find_by_member(conn, actor)? else {
            return Ok(());
        };
        if &party.leader.account != actor || actor == target {
            return Ok(());
        }
        let Some(target) = party.members.iter().find(|m| &m.account == target).cloned() else {
            return Ok(());
        };
        let leader = party.leader.clone();
        remove_member(conn, &party, &target, now, Removal::Kick, Some(&leader))
    }

    fn promote(&self, tx: &mut Tx<'_>, actor: &AccountId, target: &AccountId, now: &str) -> RepoResult<()> {
        let conn = tx.conn();
        let Some(party) = find_by_member(conn, actor)? else {
            return Ok(());
        };
        if &party.leader.account != actor || &party.leader.account == target {
            return Ok(());
        }
        let Some(target) = party.members.iter().find(|m| &m.account == target).cloned() else {
            return Ok(());
        };
        bump(conn, &party, &target, now)?;
        let next = PartyRecord {
            revision: party.revision + 1,
            leader: target,
            updated_at: now.to_owned(),
            ..party
        };
        write_event(
            conn,
            Event {
                party_id: &next.party_id,
                revision: next.revision,
                kind: PartyEventType::LeaderChanged,
                recipients: next.members.iter().map(|m| m.user_id.to_string()).collect(),
                payload: Value::Object(snapshot(&next)),
                now,
            },
        )
    }

    fn set_activity(
        &self,
        tx: &mut Tx<'_>,
        party: &GlobalId,
        expected_revision: i64,
        activity: PartyActivity,
        active_hunt_session_id: Option<&str>,
        now: &str,
    ) -> RepoResult<Option<PartyRecord>> {
        let conn = tx.conn();
        let Some(current) = find_by_id(conn, &party.to_string())? else {
            return Ok(None);
        };
        if current.revision != expected_revision {
            return Ok(None);
        }
        let changed = diesel::update(
            parties::table
                .filter(parties::id.eq(current.id.to_string()))
                .filter(parties::revision.eq(current.revision)),
        )
        .set((
            parties::revision.eq(current.revision + 1),
            parties::activity.eq(activity.as_str()),
            parties::active_hunt_session_id.eq(active_hunt_session_id),
            parties::updated_at.eq(now),
        ))
        .execute(conn)?;
        if changed != 1 {
            return Err(data(REVISION_CONFLICT));
        }
        let next = PartyRecord {
            revision: current.revision + 1,
            activity,
            active_hunt_session_id: active_hunt_session_id.map(str::to_owned),
            updated_at: now.to_owned(),
            ..current
        };
        write_event(
            conn,
            Event {
                party_id: &next.party_id,
                revision: next.revision,
                kind: PartyEventType::ActivityChanged,
                recipients: next.members.iter().map(|m| m.user_id.to_string()).collect(),
                payload: Value::Object(snapshot(&next)),
                now,
            },
        )?;
        Ok(Some(next))
    }

    fn append_event(&self, tx: &mut Tx<'_>, event: &PartyEvent) -> RepoResult<()> {
        write_event(
            tx.conn(),
            Event {
                party_id: &event.party_id,
                revision: event.party_revision,
                kind: event.event_type,
                recipients: event.recipients.iter().map(ToString::to_string).collect(),
                payload: event.payload.clone(),
                now: &event.now,
            },
        )
    }

    /// Oldest pending event, or a PROCESSING one whose claim is older than `stale_before`; marked as claimed by `owner`.
    fn claim_next_outbox(
        &self,
        tx: &mut Tx<'_>,
        owner: &str,
        now: &str,
        stale_before: &str,
    ) -> RepoResult<Option<PartyOutboxRecord>> {
        let conn = tx.conn();
        let id = party_outbox::table
            .filter(
                party_outbox::state
                    .eq(OutboxState::Pending.as_str())
                    .or(party_outbox::state
                        .eq(OutboxState::Processing.as_str())
                        .and(party_outbox::claimed_at.lt(stale_before))),
            )
            .order((party_outbox::created_at.asc(), party_outbox::seq.asc()))
            .select(party_outbox::id)
            .first::<String>(conn)
            .optional()?;
        let Some(id) = id else { return Ok(None) };
        diesel::update(party_outbox::table.find(&id))
            .set((
                party_outbox::state.eq(OutboxState::Processing.as_str()),
                party_outbox::claim_owner.eq(Some(owner)),
                party_outbox::claimed_at.eq(Some(now)),
            ))
            .execute(conn)?;
        outbox(conn, &id)
    }

    fn mark_outbox_published(
        &self,
        tx: &mut Tx<'_>,
        event_id: &str,
        owner: &str,
        published_at: &str,
    ) -> RepoResult<bool> {
        let changed = diesel::update(
            party_outbox::table
                .find(event_id)
                .filter(party_outbox::state.eq(OutboxState::Processing.as_str()))
                .filter(party_outbox::claim_owner.eq(owner)),
        )
        .set((
            party_outbox::state.eq(OutboxState::Published.as_str()),
            party_outbox::published_at.eq(Some(published_at)),
            party_outbox::claim_owner.eq(None::<String>),
            party_outbox::claimed_at.eq(None::<String>),
        ))
        .execute(tx.conn())?;
        Ok(changed == 1)
    }

    fn release_outbox(&self, tx: &mut Tx<'_>, event_id: &str, owner: &str) -> RepoResult<()> {
        diesel::update(
            party_outbox::table
                .find(event_id)
                .filter(party_outbox::state.eq(OutboxState::Processing.as_str()))
                .filter(party_outbox::claim_owner.eq(owner)),
        )
        .set((
            party_outbox::state.eq(OutboxState::Pending.as_str()),
            party_outbox::claim_owner.eq(None::<String>),
            party_outbox::claimed_at.eq(None::<String>),
        ))
        .execute(tx.conn())?;
        Ok(())
    }
}
