// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Parties, invites and the outbox (spec §11.2): rows Y1-Y8 of §15.2.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{account, log, player, system, targets};
use module_api::{GlobalId, IdKind};
use rustic_isles_store::repo::parties::*;
use rustic_isles_store::{AccountId, IslesStore, RepoError};

fn new_party(store: &IslesStore, client: &str) -> NewParty {
    NewParty {
        id: GlobalId::new(IdKind::Party, store.node()),
        party_id: client.into(),
    }
}

/// Every outbox row, oldest first: `(event type, recipients, payload)`.
async fn outbox(store: &IslesStore) -> Vec<(PartyEventType, Vec<String>, serde_json::Value)> {
    let mut out = Vec::new();
    loop {
        let claimed = store
            .write(&system(), |tx, r| {
                r.parties
                    .claim_next_outbox(tx, "reader", "2026-10-09T00:00:00.000Z", "")
            })
            .await
            .unwrap();
        let Some(event) = claimed else { break };
        let id = event.event_id.clone();
        store
            .write(&system(), move |tx, r| {
                r.parties
                    .mark_outbox_published(tx, &id, "reader", "2026-10-09T00:00:00.000Z")
            })
            .await
            .unwrap();
        out.push((
            event.event_type,
            event.recipients.iter().map(|u| u.as_str().to_owned()).collect(),
            event.payload,
        ));
    }
    out
}

fn invite(sender: &AccountId, recipient: &AccountId, party: NewParty, now: &str) -> NewInvite {
    NewInvite {
        sender: sender.clone(),
        recipient: recipient.clone(),
        build_id: "b".into(),
        sender_display_name: "Sender".into(),
        sender_platform: "PC".into(),
        new_party: party,
        now: now.into(),
        expires_at: "2026-10-08T00:00:00.000Z".into(),
    }
}

fn accept(recipient: &AccountId, inviter: &AccountId, party: NewParty, max: u32) -> AcceptInvite {
    AcceptInvite {
        recipient: recipient.clone(),
        inviter: inviter.clone(),
        build_id: "b".into(),
        new_party: party,
        now: "2026-10-02T00:00:00.000Z".into(),
        max_party_size: max,
    }
}

#[tokio::test]
async fn y1_y2_y5_solo_invite_accept() {
    for t in targets() {
        let store = &t.store;
        let (a, b, c) = (
            account(store, "ua").await,
            account(store, "ub").await,
            account(store, "uc").await,
        );
        let (pa, pb, pc) = (new_party(store, "pa"), new_party(store, "pb"), new_party(store, "pc"));
        let w = a.clone();
        let p2 = pa.clone();
        let (solo, again) = store
            .write(&player(&a), move |tx, r| {
                Ok((
                    r.parties
                        .get_or_create_solo(tx, &w, "b", &p2, "2026-10-01T00:00:00.000Z")?,
                    r.parties
                        .get_or_create_solo(tx, &w, "b", &p2, "2026-10-01T00:00:00.000Z")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(solo, again);
        assert_eq!(
            (solo.revision, solo.members.len(), solo.activity),
            (1, 1, PartyActivity::Idle)
        );
        let events = outbox(store).await;
        assert_eq!(events.len(), 1, "one PARTY_CREATED ({})", t.engine);
        assert_eq!(events[0].0, PartyEventType::PartyCreated);
        assert_eq!(
            events[0].2.to_string(),
            r#"{"partyId":"pa","revision":1,"leaderPlayerId":"ua","members":["ua"],"activity":"IDLE","activeHuntSessionId":null}"#
        );

        let (i1, i2) = (
            invite(&a, &b, pa.clone(), "2026-10-01T00:00:01.000Z"),
            invite(&c, &b, pc.clone(), "2026-10-01T00:00:02.000Z"),
        );
        let wb = b.clone();
        let listed = store
            .write(&system(), move |tx, r| {
                r.parties.create_invite(tx, &i1)?;
                r.parties.create_invite(tx, &i2)?;
                r.parties
                    .list_pending_invites(tx.conn(), &wb, "2026-10-01T12:00:00.000Z")
            })
            .await
            .unwrap();
        assert_eq!(
            listed.iter().map(|i| i.party_id.as_str()).collect::<Vec<_>>(),
            ["pc", "pa"]
        );
        assert_eq!(listed[1].invite_id, "ub\u{1f}ua");
        let events = outbox(store).await;
        let invite_event = events
            .iter()
            .find(|e| e.0 == PartyEventType::PartyInviteCreated)
            .unwrap();
        assert_eq!(
            invite_event.2.to_string(),
            r#"{"partyId":"pa","partyRevision":1,"sendingPlayerId":"ua","recipientPlayerId":"ub","sendingDisplayName":"Sender","sendingPlatform":"PC","status":"PENDING","createdAt":"2026-10-01T00:00:01.000Z","expiresAt":"2026-10-08T00:00:00.000Z","inviteId":"ub\u001fua"}"#
        );

        // The size cap: a party of one at max 1 takes no one
        let full = accept(&b, &a, pb.clone(), 1);
        let kept = store
            .write(&system(), move |tx, r| r.parties.accept_invite(tx, &full))
            .await
            .unwrap();
        assert!(kept.is_none(), "b has no party yet");
        let ok = accept(&b, &a, pb.clone(), 4);
        let joined = store
            .write(&system(), move |tx, r| r.parties.accept_invite(tx, &ok))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (joined.party_id.as_str(), joined.revision, joined.members.len()),
            ("pa", 2, 2)
        );
        let wb = b.clone();
        let (left_pending, statuses) = store
            .read(move |conn, r| {
                use diesel::RunQueryDsl;
                #[derive(diesel::QueryableByName)]
                struct S {
                    #[diesel(sql_type = diesel::sql_types::Text)]
                    status: String,
                }
                let rows = diesel::sql_query("SELECT status FROM party_invites ORDER BY sender").load::<S>(conn)?;
                Ok((
                    r.parties.list_pending_invites(conn, &wb, "2026-10-01T12:00:00.000Z")?,
                    rows,
                ))
            })
            .await
            .unwrap();
        assert!(left_pending.is_empty());
        let mut statuses: Vec<_> = statuses.into_iter().map(|s| s.status).collect();
        statuses.sort();
        assert_eq!(statuses, ["ACCEPTED", "REVOKED"]);
        let events = outbox(store).await;
        let join = events
            .iter()
            .find(|e| e.0 == PartyEventType::PartyMembershipChanged)
            .unwrap();
        assert_eq!(join.1, ["ua", "ub"]);
        assert_eq!(
            join.2.to_string(),
            r#"{"partyId":"pa","revision":2,"leaderPlayerId":"ua","members":["ua","ub"],"activity":"IDLE","activeHuntSessionId":null,"action":"JOIN","accountId":"ub"}"#
        );
    }
}

#[tokio::test]
async fn y3_y4_moves_leaves_kicks_promotes_and_disbands() {
    for t in targets() {
        let store = &t.store;
        let (a, b, c) = (
            account(store, "ua").await,
            account(store, "ub").await,
            account(store, "uc").await,
        );
        let (pa, pb, pc) = (new_party(store, "pa"), new_party(store, "pb"), new_party(store, "pc"));
        let (wa, wb, wc) = (a.clone(), b.clone(), c.clone());
        let (pa2, pb2, pc2) = (pa.clone(), pb.clone(), pc.clone());
        store
            .write(&system(), move |tx, r| {
                let p = &r.parties;
                p.create_invite(tx, &invite(&wa, &wb, pa2.clone(), "2026-10-01T00:00:00.000Z"))?;
                p.accept_invite(tx, &accept(&wb, &wa, pb2.clone(), 4))?;
                // c invites b: b moves from a's party to c's
                p.create_invite(tx, &invite(&wc, &wb, pc2.clone(), "2026-10-01T00:00:01.000Z"))?;
                p.accept_invite(tx, &accept(&wb, &wc, pb2.clone(), 4))?;
                Ok(())
            })
            .await
            .unwrap();
        let events = outbox(store).await;
        let moved = events
            .iter()
            .find(|e| e.2.get("action").and_then(|a| a.as_str()) == Some("MOVE"))
            .unwrap();
        assert_eq!(moved.1, ["ua", "ub"], "the remaining members, then the target");
        assert_eq!(
            moved.2.to_string(),
            r#"{"partyId":"pa","revision":3,"leaderPlayerId":"ua","members":["ua"],"activity":"IDLE","activeHuntSessionId":null,"action":"MOVE","accountId":"ub"}"#
        );

        // An invite from a party the inviter has left is dead
        let (wa, wb, wc) = (a.clone(), b.clone(), c.clone());
        let (pa2, fresh) = (pa.clone(), new_party(store, "pa-new"));
        let dead = store
            .write(&system(), move |tx, r| {
                let p = &r.parties;
                p.create_invite(tx, &invite(&wa, &wc, pa2.clone(), "2026-10-01T00:00:02.000Z"))?;
                // a leaves (disbanding pa) before c accepts; a's new solo party did not issue the invite
                p.leave(tx, &wa, "2026-10-01T00:00:03.000Z")?;
                let answer = p.accept_invite(tx, &accept(&wc, &wa, fresh.clone(), 4))?;
                Ok((answer, p.find_by_member(tx.conn(), &wb)?))
            })
            .await
            .unwrap();
        assert_eq!(
            dead.0.map(|p| p.party_id),
            Some("pc".into()),
            "c stays in its own party"
        );
        let events = outbox(store).await;
        let disband = events
            .iter()
            .find(|e| e.2.get("action").and_then(|a| a.as_str()) == Some("DISBAND"))
            .unwrap();
        assert_eq!(
            disband.2.to_string(),
            r#"{"action":"DISBAND","accountId":"ua","members":[]}"#
        );

        // Leader rules in c's party (c leads, b is in it)
        let (wb, wc) = (b.clone(), c.clone());
        let (refused_kick, promoted, kicked) = store
            .write(&system(), move |tx, r| {
                let p = &r.parties;
                p.kick(tx, &wb, &wc, "2026-10-01T00:00:04.000Z")?;
                let refused = p.find_by_member(tx.conn(), &wc)?.unwrap().members.len();
                p.promote(tx, &wc, &wb, "2026-10-01T00:00:05.000Z")?;
                let promoted = p.find_by_member(tx.conn(), &wb)?.unwrap();
                p.kick(tx, &wb, &wc, "2026-10-01T00:00:06.000Z")?;
                Ok((refused, promoted, p.find_by_member(tx.conn(), &wc)?))
            })
            .await
            .unwrap();
        assert_eq!(refused_kick, 2, "only the leader kicks");
        assert_eq!(promoted.leader.account, b);
        assert!(kicked.is_none());
        let events = outbox(store).await;
        assert!(events.iter().any(|e| e.0 == PartyEventType::PartyLeaderChanged));
        let kick = events
            .iter()
            .find(|e| e.2.get("action").and_then(|a| a.as_str()) == Some("KICK"))
            .unwrap();
        assert_eq!(kick.2["actorAccountId"], "ub");
    }
}

#[tokio::test]
async fn y6_y8_activity_outbox_and_log() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "ua").await;
        let pa = new_party(store, "pa");
        let (w, p2) = (a.clone(), pa.clone());
        let party = store
            .write(&player(&a), move |tx, r| {
                r.parties
                    .get_or_create_solo(tx, &w, "b", &p2, "2026-10-01T00:00:00.000Z")
            })
            .await
            .unwrap();
        let id = party.id.clone();
        let (stale, set) = store
            .write(&player(&a), move |tx, r| {
                Ok((
                    r.parties.set_activity(
                        tx,
                        &id,
                        5,
                        PartyActivity::InHunt,
                        Some("h1"),
                        "2026-10-01T00:00:01.000Z",
                    )?,
                    r.parties.set_activity(
                        tx,
                        &id,
                        1,
                        PartyActivity::InHunt,
                        Some("h1"),
                        "2026-10-01T00:00:01.000Z",
                    )?,
                ))
            })
            .await
            .unwrap();
        assert!(stale.is_none());
        let set = set.unwrap();
        assert_eq!((set.revision, set.active_hunt_session_id.as_deref()), (2, Some("h1")));
        // A failed unit of work leaves neither the change nor its event
        let id = party.id.clone();
        let err = store
            .write(&player(&a), move |tx, r| {
                r.parties
                    .set_activity(tx, &id, 2, PartyActivity::Idle, None, "2026-10-01T00:00:02.000Z")?;
                Err::<(), _>(RepoError::Data(REVISION_CONFLICT.into()))
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains(REVISION_CONFLICT));
        let id = party.id.clone();
        let now = store
            .read(move |c, r| r.parties.find_by_id(c, &id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((now.revision, now.activity), (2, PartyActivity::InHunt));

        // Outbox: oldest first, one claim, stale reclaim, release, publish once
        let claim = |owner: &'static str, stale: &'static str| {
            let store = store.clone();
            async move {
                store
                    .write(&system(), move |tx, r| {
                        r.parties
                            .claim_next_outbox(tx, owner, "2026-10-02T00:00:00.000Z", stale)
                    })
                    .await
                    .unwrap()
            }
        };
        let first = claim("w1", "").await.unwrap();
        assert_eq!(first.event_type, PartyEventType::PartyCreated);
        let second = claim("w2", "").await.unwrap();
        assert_eq!(second.event_type, PartyEventType::PartyActivityChanged);
        assert!(claim("w3", "").await.is_none(), "both are claimed");
        let reclaimed = claim("w3", "2026-10-03T00:00:00.000Z").await.unwrap();
        assert_eq!(reclaimed.event_id, first.event_id, "a stale claim is taken over");
        let (e1, e2) = (first.event_id.clone(), second.event_id.clone());
        let (wrong_owner, published, again) = store
            .write(&system(), move |tx, r| {
                let p = &r.parties;
                let wrong = p.mark_outbox_published(tx, &e1, "w1", "t")?;
                p.release_outbox(tx, &e2, "w2")?;
                let published = p.mark_outbox_published(tx, &e1, "w3", "t")?;
                let again = p.mark_outbox_published(tx, &e1, "w3", "t")?;
                Ok((wrong, published, again))
            })
            .await
            .unwrap();
        assert_eq!((wrong_owner, published, again), (false, true, false));
        let released = claim("w4", "").await.unwrap();
        assert_eq!(released.event_id, second.event_id);

        let player_text = format!("player:{a}");
        let entries = log(store).await;
        for table in ["parties", "party_members", "party_outbox"] {
            assert!(
                entries
                    .iter()
                    .any(|(actor, t, _, _)| actor == &player_text && t == table),
                "{table}"
            );
        }
    }
}
