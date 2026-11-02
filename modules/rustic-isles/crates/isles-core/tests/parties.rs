//! The party repository on both engines: `persistence/sqlite/repositories.social.test.ts`'s party tests ported (the
//! Mongo repository's contract, which the SQLite one shares), over accounts under global ids. The Postgres half runs
//! when `REFORGED_TEST_POSTGRES_URL` names a server.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

mod support;

use isles_core::repo::parties::{
    AcceptInvite, NewInvite, NewParty, OutboxState, PartyActivity, PartyEvent, PartyEventType, PartyRecord,
};
use isles_core::repo::{AccountId, RepoError, RepoResult, Repositories, Tx, UserId};
use module_api::{Actor, ActorContext, GlobalId, IdKind};
use serde_json::json;
use support::{Target, targets};

const NOW: &str = "2026-10-13T12:00:00.000Z";
const LATER: &str = "2026-10-13T12:01:00.000Z";
const EXPIRY: &str = "2026-10-13T12:10:00.000Z";

struct Realm {
    target: Target,
}

impl Realm {
    async fn account(&self, name: &str) -> AccountId {
        self.target.account(name).await
    }

    async fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Send + 'static,
    ) -> T {
        self.target
            .store
            .write(&ActorContext::new(Actor::System), f)
            .await
            .unwrap()
    }

    async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut isles_core::repo::AnyConnection, &Repositories) -> RepoResult<T> + Send + 'static,
    ) -> T {
        self.target.store.read(f).await.unwrap()
    }

    fn new_party(&self, id: &str) -> NewParty {
        NewParty {
            id: GlobalId::new(IdKind::Party, &self.target.node.node),
            party_id: id.to_owned(),
        }
    }

    async fn solo(&self, who: &AccountId, id: &str) -> PartyRecord {
        let (who, new_party) = (who.clone(), self.new_party(id));
        self.write(move |tx, repos| repos.parties.get_or_create_solo(tx, &who, "647472", &new_party, NOW))
            .await
    }

    async fn invite(&self, sender: &AccountId, recipient: &AccountId, now: &str, party_id: &str) {
        let invite = NewInvite {
            sender: sender.clone(),
            recipient: recipient.clone(),
            build_id: "647472".into(),
            sender_display_name: "sender".into(),
            sender_platform: "STEAM".into(),
            new_party: self.new_party(party_id),
            now: now.to_owned(),
            expires_at: EXPIRY.to_owned(),
        };
        self.write(move |tx, repos| repos.parties.create_invite(tx, &invite))
            .await;
    }

    async fn accept(&self, recipient: &AccountId, inviter: &AccountId, now: &str, max: u32) -> Option<PartyRecord> {
        let accept = AcceptInvite {
            recipient: recipient.clone(),
            inviter: inviter.clone(),
            build_id: "x".into(),
            new_party: self.new_party("unused"),
            now: now.to_owned(),
            max_party_size: max,
        };
        self.write(move |tx, repos| repos.parties.accept_invite(tx, &accept))
            .await
    }

    async fn by_id(&self, id: &str) -> Option<PartyRecord> {
        let id = id.to_owned();
        self.read(move |conn, repos| repos.parties.find_by_party_id(conn, &id))
            .await
    }

    async fn of(&self, member: &AccountId) -> Option<PartyRecord> {
        let member = member.clone();
        self.read(move |conn, repos| repos.parties.find_by_member(conn, &member))
            .await
    }

    /// Claims and publishes every event: (type, revision, recipients).
    async fn drain(&self) -> Vec<(PartyEventType, i64, Vec<String>)> {
        let mut seen = Vec::new();
        loop {
            let event = self
                .write(|tx, repos| {
                    repos.parties.claim_next_outbox(
                        tx,
                        "worker",
                        "2026-10-13T13:00:00.000Z",
                        "2026-10-13T12:59:00.000Z",
                    )
                })
                .await;
            let Some(event) = event else { return seen };
            seen.push((
                event.event_type,
                event.party_revision,
                event.recipients.iter().map(ToString::to_string).collect(),
            ));
            let id = event.event_id.clone();
            let published = self
                .write(move |tx, repos| {
                    repos
                        .parties
                        .mark_outbox_published(tx, &id, "worker", "2026-10-13T13:00:01.000Z")
                })
                .await;
            assert!(published);
        }
    }
}

fn users(party: &PartyRecord) -> Vec<String> {
    party.members.iter().map(|m| m.user_id.to_string()).collect()
}

async fn realms() -> Vec<Realm> {
    targets().into_iter().map(|target| Realm { target }).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_solo_party_is_created_once_per_account_and_announced() {
    for r in realms().await {
        let a = r.account("a").await;
        let first = r.solo(&a, "party-of-a").await;
        assert_eq!(first.party_id, "party-of-a", "{}", r.target.engine);
        assert_eq!(first.revision, 1);
        assert_eq!(users(&first), ["a"]);
        assert_eq!(first.leader.user_id, UserId("a".into()));
        assert_eq!(first.activity, PartyActivity::Idle);
        assert_eq!(first.id.kind(), IdKind::Party);
        let again = r.solo(&a, "other-id").await;
        assert_eq!(again.party_id, "party-of-a", "the existing party is returned");

        assert_eq!(r.of(&a).await.unwrap().party_id, "party-of-a");
        assert_eq!(r.by_id("party-of-a").await.unwrap().leader.user_id, UserId("a".into()));
        let nobody = r.account("nobody").await;
        assert!(r.of(&nobody).await.is_none());
        assert!(r.by_id("nope").await.is_none());
        let global = first.id.clone();
        assert_eq!(
            r.read(move |conn, repos| repos.parties.find_by_id(conn, &global))
                .await
                .unwrap()
                .party_id,
            "party-of-a"
        );
        assert_eq!(r.drain().await, [(PartyEventType::Created, 1, vec!["a".to_owned()])]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn invite_list_accept_with_the_party_size_cap_and_invite_consumption() {
    for r in realms().await {
        let [a, b, c, z] = ["a", "b", "c", "z"].map(|n| n.to_owned());
        let (a, b, c, z) = (
            r.account(&a).await,
            r.account(&b).await,
            r.account(&c).await,
            r.account(&z).await,
        );
        r.invite(&a, &b, LATER, "party-of-a").await;
        r.invite(&a, &c, "2026-10-13T12:02:00.000Z", "party-of-a").await;
        r.invite(&z, &c, "2026-10-13T12:03:00.000Z", "party-of-z").await;
        r.invite(&a, &a, LATER, "party-of-a").await; // an invite to yourself is dropped

        let for_c = {
            let c = c.clone();
            r.read(move |conn, repos| repos.parties.list_pending_invites(conn, &c, "2026-10-13T12:04:00.000Z"))
                .await
        };
        assert_eq!(
            for_c.iter().map(|i| i.sender.user_id.to_string()).collect::<Vec<_>>(),
            ["z", "a"],
            "newest first ({})",
            r.target.engine
        );
        assert_eq!(for_c[0].status, isles_core::repo::parties::InviteStatus::Pending);
        assert_eq!(for_c[1].invite_id, "c\u{1f}a");
        assert_eq!(for_c[1].party_id, "party-of-a");
        let expired = {
            let c = c.clone();
            r.read(move |conn, repos| repos.parties.list_pending_invites(conn, &c, "2026-10-13T12:11:00.000Z"))
                .await
        };
        assert!(expired.is_empty(), "expired invites are not listed");

        let accepted = r.accept(&b, &a, "2026-10-13T12:05:00.000Z", 2).await.unwrap();
        assert_eq!(users(&accepted), ["a", "b"]);
        assert_eq!(accepted.revision, 2);
        assert_eq!(r.of(&b).await.unwrap().party_id, "party-of-a");
        let b_invites = {
            let b = b.clone();
            r.read(move |conn, repos| repos.parties.list_pending_invites(conn, &b, LATER))
                .await
        };
        assert!(b_invites.is_empty(), "the accepted invite is consumed");

        assert!(
            r.accept(&c, &a, "2026-10-13T12:05:30.000Z", 2).await.is_none(),
            "a full party refuses; c has no party of their own"
        );
        assert_eq!(r.by_id("party-of-a").await.unwrap().members.len(), 2);

        let stranger = r.account("stranger").await;
        assert_eq!(
            r.accept(&b, &stranger, LATER, 4).await.unwrap().party_id,
            "party-of-a",
            "no such invite: the recipient's current party is returned"
        );

        let events = r.drain().await;
        assert_eq!(
            events.iter().map(|e| e.0).collect::<Vec<_>>(),
            [
                PartyEventType::Created,
                PartyEventType::InviteCreated,
                PartyEventType::InviteCreated,
                PartyEventType::Created,
                PartyEventType::InviteCreated,
                PartyEventType::MembershipChanged
            ]
        );
        assert_eq!(events[5].2, ["a", "b"]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn accepting_moves_a_member_out_of_their_old_party_and_an_invite_from_a_party_the_inviter_left_is_dead() {
    for r in realms().await {
        let (a, b, c, d, e) = (
            r.account("a").await,
            r.account("b").await,
            r.account("c").await,
            r.account("d").await,
            r.account("e").await,
        );
        r.invite(&a, &b, LATER, "party-of-a").await;
        r.accept(&b, &a, "2026-10-13T12:02:00.000Z", 4).await;
        r.invite(&d, &b, "2026-10-13T12:03:00.000Z", "party-of-d").await;
        r.accept(&b, &d, "2026-10-13T12:04:00.000Z", 4).await;
        assert_eq!(users(&r.of(&b).await.unwrap()), ["d", "b"], "{}", r.target.engine);
        assert_eq!(users(&r.by_id("party-of-a").await.unwrap()), ["a"]);

        // d leaves their party (b stays, as leader), then d starts a fresh party; d's old invite to c must not let c in
        r.invite(&d, &c, "2026-10-13T12:05:00.000Z", "party-of-d").await;
        let leaving = d.clone();
        r.write(move |tx, repos| repos.parties.leave(tx, &leaving, "2026-10-13T12:06:00.000Z"))
            .await;
        assert_eq!(r.by_id("party-of-d").await.unwrap().leader.user_id, UserId("b".into()));
        r.invite(&d, &e, "2026-10-13T12:07:00.000Z", "party-of-d-2").await;
        assert_eq!(r.of(&d).await.unwrap().party_id, "party-of-d-2");
        assert!(r.accept(&c, &d, "2026-10-13T12:08:00.000Z", 4).await.is_none());
        assert!(r.of(&c).await.is_none());
        assert_eq!(users(&r.by_id("party-of-d-2").await.unwrap()), ["d"]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_kicking_and_promoting_follow_the_leader_rules_and_the_last_member_disbands() {
    for r in realms().await {
        let (a, b, c) = (r.account("a").await, r.account("b").await, r.account("c").await);
        let zed = r.account("zed").await;
        for (inviter, recipient, now) in [
            (&a, &b, "2026-10-13T12:01:00.000Z"),
            (&a, &c, "2026-10-13T12:02:00.000Z"),
        ] {
            r.invite(inviter, recipient, now, "party-of-a").await;
            r.accept(recipient, inviter, now, 4).await;
        }
        let kick = |actor: &AccountId, target: &AccountId, now: &'static str| {
            let (actor, target) = (actor.clone(), target.clone());
            r.write(move |tx, repos| repos.parties.kick(tx, &actor, &target, now))
        };
        kick(&b, &c, "2026-10-13T12:03:00.000Z").await; // not the leader
        kick(&a, &a, "2026-10-13T12:03:00.000Z").await; // not yourself
        kick(&a, &zed, "2026-10-13T12:03:00.000Z").await; // not a member
        assert_eq!(
            users(&r.by_id("party-of-a").await.unwrap()),
            ["a", "b", "c"],
            "{}",
            r.target.engine
        );

        kick(&a, &c, "2026-10-13T12:04:00.000Z").await;
        assert_eq!(users(&r.by_id("party-of-a").await.unwrap()), ["a", "b"]);
        assert!(r.of(&c).await.is_none());

        let promote = |actor: &AccountId, target: &AccountId, now: &'static str| {
            let (actor, target) = (actor.clone(), target.clone());
            r.write(move |tx, repos| repos.parties.promote(tx, &actor, &target, now))
        };
        promote(&b, &a, "2026-10-13T12:05:00.000Z").await; // not the leader
        promote(&a, &a, "2026-10-13T12:05:00.000Z").await; // already the leader
        promote(&a, &b, "2026-10-13T12:05:00.000Z").await;
        assert_eq!(r.by_id("party-of-a").await.unwrap().leader.user_id, UserId("b".into()));

        let leave = |who: &AccountId, now: &'static str| {
            let who = who.clone();
            r.write(move |tx, repos| repos.parties.leave(tx, &who, now))
        };
        leave(&b, "2026-10-13T12:06:00.000Z").await; // the leader leaves: a takes over
        let left = r.by_id("party-of-a").await.unwrap();
        assert_eq!(users(&left), ["a"]);
        assert_eq!(left.leader.user_id, UserId("a".into()));
        leave(&zed, "2026-10-13T12:06:30.000Z").await;

        leave(&a, "2026-10-13T12:07:00.000Z").await;
        assert!(
            r.by_id("party-of-a").await.is_none(),
            "the last member disbands the party"
        );
        assert!(r.of(&a).await.is_none());

        let kinds: Vec<PartyEventType> = r.drain().await.iter().map(|e| e.0).collect();
        let count = |kind| kinds.iter().filter(|k| **k == kind).count();
        assert_eq!(
            count(PartyEventType::LeaderChanged),
            1,
            "only the real promotion is announced"
        );
        assert_eq!(
            count(PartyEventType::MembershipChanged),
            5,
            "2 joins + kick + a leave + the disband"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn membership_events_carry_node_s_payloads() {
    for r in realms().await {
        let (a, b) = (r.account("a").await, r.account("b").await);
        r.invite(&a, &b, LATER, "party-of-a").await;
        r.accept(&b, &a, "2026-10-13T12:02:00.000Z", 4).await;
        let kick_actor = a.clone();
        let kicked = b.clone();
        r.write(move |tx, repos| repos.parties.kick(tx, &kick_actor, &kicked, "2026-10-13T12:03:00.000Z"))
            .await;
        let mut payloads = Vec::new();
        loop {
            let Some(event) = r
                .write(|tx, repos| {
                    repos
                        .parties
                        .claim_next_outbox(tx, "w", "2026-10-13T13:00:00.000Z", "2026-10-13T12:59:00.000Z")
                })
                .await
            else {
                break;
            };
            payloads.push((event.event_type, event.payload.to_string()));
        }
        let by = |kind: PartyEventType| {
            payloads
                .iter()
                .filter(|(k, _)| *k == kind)
                .map(|(_, p)| p.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            by(PartyEventType::Created)[0],
            r#"{"partyId":"party-of-a","revision":1,"leaderPlayerId":"a","members":["a"],"activity":"IDLE","activeHuntSessionId":null}"#
        );
        assert_eq!(
            by(PartyEventType::InviteCreated)[0],
            r#"{"partyId":"party-of-a","partyRevision":1,"sendingPlayerId":"a","recipientPlayerId":"b","sendingDisplayName":"sender","sendingPlatform":"STEAM","status":"PENDING","createdAt":"2026-10-13T12:01:00.000Z","expiresAt":"2026-10-13T12:10:00.000Z","inviteId":"b\u001fa"}"#
        );
        let membership = by(PartyEventType::MembershipChanged);
        assert_eq!(
            membership[0],
            r#"{"partyId":"party-of-a","revision":2,"leaderPlayerId":"a","members":["a","b"],"activity":"IDLE","activeHuntSessionId":null,"action":"JOIN","accountId":"b"}"#,
            "{}",
            r.target.engine
        );
        assert_eq!(
            membership[1],
            r#"{"partyId":"party-of-a","revision":3,"leaderPlayerId":"a","members":["a"],"activity":"IDLE","activeHuntSessionId":null,"action":"KICK","accountId":"b","actorAccountId":"a"}"#
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn activity_changes_are_revision_guarded_and_a_failed_unit_of_work_leaves_neither_the_change_nor_its_event() {
    for r in realms().await {
        let a = r.account("a").await;
        let solo = r.solo(&a, "party-of-a").await;
        r.drain().await;
        let id = solo.id.clone();
        let set = |revision: i64, activity: PartyActivity, hunt: Option<&'static str>| {
            let id = id.clone();
            r.write(move |tx, repos| {
                repos
                    .parties
                    .set_activity(tx, &id, revision, activity, hunt, "2026-10-13T12:02:00.000Z")
            })
        };
        assert!(set(7, PartyActivity::InHunt, None).await.is_none(), "stale revision");
        let ghost = GlobalId::new(IdKind::Party, &r.target.node.node);
        assert!(
            r.write(move |tx, repos| repos
                .parties
                .set_activity(tx, &ghost, 1, PartyActivity::InHunt, None, LATER))
                .await
                .is_none()
        );
        let active = set(1, PartyActivity::InHunt, Some("hunt-1")).await.unwrap();
        assert_eq!((active.revision, active.activity), (2, PartyActivity::InHunt));
        assert_eq!(
            r.by_id("party-of-a").await.unwrap().active_hunt_session_id.as_deref(),
            Some("hunt-1")
        );

        // A failed unit of work: the activity change and the appended event roll back together
        let (id2, store) = (solo.id.clone(), &r.target.store);
        let crashed: RepoResult<()> = store
            .write(&ActorContext::new(Actor::System), move |tx, repos| {
                repos
                    .parties
                    .set_activity(tx, &id2, 2, PartyActivity::Idle, None, LATER)?;
                repos.parties.append_event(
                    tx,
                    &PartyEvent {
                        party_id: "party-of-a".into(),
                        party_revision: 3,
                        event_type: PartyEventType::PresenceChanged,
                        recipients: vec![UserId("a".into()), UserId("a".into())],
                        payload: json!({ "p": 1 }),
                        now: LATER.into(),
                    },
                )?;
                Err(RepoError::Data("crash".into()))
            })
            .await;
        assert!(crashed.is_err());
        assert_eq!(r.by_id("party-of-a").await.unwrap().revision, 2);
        assert_eq!(
            r.drain().await.iter().map(|e| e.0).collect::<Vec<_>>(),
            [PartyEventType::ActivityChanged],
            "no event for the rolled-back work ({})",
            r.target.engine
        );

        r.write(|tx, repos| {
            repos.parties.append_event(
                tx,
                &PartyEvent {
                    party_id: "party-of-a".into(),
                    party_revision: 2,
                    event_type: PartyEventType::PresenceChanged,
                    recipients: ["a", "a", "b"].map(|u| UserId(u.into())).to_vec(),
                    payload: json!({ "p": 1 }),
                    now: LATER.into(),
                },
            )
        })
        .await;
        assert_eq!(
            r.drain().await.iter().map(|e| e.2.clone()).collect::<Vec<_>>(),
            [["a", "b"]],
            "recipients are de-duplicated"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_outbox_is_oldest_first_claimed_by_one_worker_released_or_reclaimed_when_stale_and_published_once() {
    for r in realms().await {
        for index in 1..=3 {
            r.write(move |tx, repos| {
                repos.parties.append_event(
                    tx,
                    &PartyEvent {
                        party_id: "p".into(),
                        party_revision: index,
                        event_type: PartyEventType::PresenceChanged,
                        recipients: vec![UserId("a".into())],
                        payload: json!({ "Index": index }),
                        now: format!("2026-10-13T12:0{index}:00.000Z"),
                    },
                )
            })
            .await;
        }
        let claim = |owner: &'static str, now: &'static str, stale_before: &'static str| {
            r.write(move |tx, repos| repos.parties.claim_next_outbox(tx, owner, now, stale_before))
        };
        let publish = |id: String, owner: &'static str| {
            r.write(move |tx, repos| {
                repos
                    .parties
                    .mark_outbox_published(tx, &id, owner, "2026-10-13T13:00:00.000Z")
            })
        };
        let release =
            |id: String, owner: &'static str| r.write(move |tx, repos| repos.parties.release_outbox(tx, &id, owner));

        let first = claim("w1", "2026-10-13T12:10:00.000Z", "2026-10-13T12:00:00.000Z")
            .await
            .unwrap();
        assert_eq!(first.party_revision, 1, "{}", r.target.engine);
        assert_eq!(first.state, OutboxState::Processing);
        assert_eq!(first.claim_owner.as_deref(), Some("w1"));
        assert_eq!(first.payload, json!({ "Index": 1 }));
        let second = claim("w2", "2026-10-13T12:10:01.000Z", "2026-10-13T12:00:00.000Z")
            .await
            .unwrap();
        assert_eq!(second.party_revision, 2, "a claimed event is not handed out again");

        assert!(
            !publish(first.event_id.clone(), "w2").await,
            "only the claiming worker may publish"
        );
        assert!(publish(first.event_id.clone(), "w1").await);
        assert!(!publish(first.event_id.clone(), "w1").await, "published once");
        assert!(!publish("ghost".into(), "w1").await);

        release(second.event_id.clone(), "w1").await; // not the owner: nothing
        assert_eq!(
            claim("w3", "2026-10-13T12:10:02.000Z", "2026-10-13T12:00:00.000Z")
                .await
                .unwrap()
                .party_revision,
            3
        );
        release(second.event_id.clone(), "w2").await;
        let again = claim("w4", "2026-10-13T12:10:03.000Z", "2026-10-13T12:00:00.000Z")
            .await
            .unwrap();
        assert_eq!(
            again.event_id, second.event_id,
            "a released event is available again, oldest first"
        );
        assert_eq!(again.claim_owner.as_deref(), Some("w4"));

        // Nothing left unclaimed; a stale claim (older than the cut-off) is taken over
        assert!(
            claim("w5", "2026-10-13T12:10:04.000Z", "2026-10-13T12:00:00.000Z")
                .await
                .is_none()
        );
        let takeover = claim("w5", "2026-10-13T12:20:00.000Z", "2026-10-13T12:10:02.500Z")
            .await
            .unwrap();
        assert_eq!(
            takeover.party_revision, 3,
            "the claim at 12:10:02 is stale against a 12:10:02.5 cut-off; the older 12:10:03 one is not"
        );
        assert_eq!(takeover.claim_owner.as_deref(), Some("w5"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_party_write_is_in_the_change_log_under_its_actor() {
    for r in realms().await {
        let a = r.account("a").await;
        let (who, new_party) = (a.clone(), r.new_party("party-of-a"));
        let actor = ActorContext::new(Actor::Player {
            account: a.global().clone(),
        });
        r.target
            .store
            .account_write(&a, &actor, move |tx, repos| {
                repos.parties.get_or_create_solo(tx, &who, "647472", &new_party, NOW)
            })
            .await
            .unwrap();
        let log = r.target.store.database().changes(0, 100).await.unwrap();
        let tables: Vec<&str> = log
            .iter()
            .filter(|e| e.actor.starts_with("player:"))
            .map(|e| e.table_name.as_str())
            .collect();
        for table in ["parties", "party_members", "party_outbox"] {
            assert!(tables.contains(&table), "{table} in {tables:?} ({})", r.target.engine);
        }
    }
}

// ---- the outbox dispatcher (PartyOutboxDispatcher.ts) ----

#[tokio::test(flavor = "multi_thread")]
async fn the_dispatcher_publishes_each_event_once_as_a_headline_and_releases_one_that_fails() {
    use isles_chat::headline::Headline;
    use isles_core::areas::matchmaking::outbox::drain;
    use std::sync::{Arc, Mutex};

    for r in realms().await {
        let (a, b) = (r.account("a").await, r.account("b").await);
        r.invite(&a, &b, LATER, "party-of-a").await;
        let sent: Arc<Mutex<Vec<Headline>>> = Arc::default();
        let collect = {
            let sent = sent.clone();
            move |headline: Headline| -> module_api::BoxFuture<'static, Result<(), String>> {
                let sent = sent.clone();
                Box::pin(async move {
                    sent.lock().unwrap().push(headline);
                    Ok(())
                })
            }
        };
        let now = 1_760_356_800_000;
        assert_eq!(
            drain(&r.target.store, "w", now, &collect).await.unwrap(),
            2,
            "{}",
            r.target.engine
        );
        {
            let sent = sent.lock().unwrap();
            assert_eq!(sent.len(), 2);
            let created: serde_json::Value = serde_json::from_str(&sent[0].body).unwrap();
            assert_eq!(created["eventType"], "party.created");
            assert_eq!(created["partyId"], "party-of-a");
            assert_eq!(created["schemaVersion"], 1);
            assert_eq!(created["payload"]["leaderPlayerId"], "a");
            assert_eq!(sent[0].recipients.len(), 1);
            assert_eq!(sent[0].recipients[0].user_id, "a");
            assert_eq!(sent[0].recipients[0].account, *a.global());
            let invited: serde_json::Value = serde_json::from_str(&sent[1].body).unwrap();
            assert_eq!(invited["eventType"], "party.invited");
            assert_eq!(sent[1].recipients[0].account, *b.global());
        }
        assert_eq!(
            drain(&r.target.store, "w", now, &collect).await.unwrap(),
            0,
            "published once"
        );

        // A publish that fails releases the event and ends the pass; the next pass sends it
        let accepted = r.accept(&b, &a, "2026-10-13T12:05:00.000Z", 4).await;
        assert!(accepted.is_some());
        let failing = |_: Headline| -> module_api::BoxFuture<'static, Result<(), String>> {
            Box::pin(async { Err("plugin is away".to_owned()) })
        };
        assert_eq!(drain(&r.target.store, "w", now, &failing).await.unwrap(), 0);
        assert_eq!(drain(&r.target.store, "w", now, &collect).await.unwrap(), 1);
        let last: serde_json::Value = serde_json::from_str(&sent.lock().unwrap()[2].body).unwrap();
        assert_eq!(last["eventType"], "party.membership_changed");
        assert_eq!(last["payload"]["action"], "JOIN");
    }
}
