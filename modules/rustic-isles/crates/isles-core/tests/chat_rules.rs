//! The chat rules over the real store: what the plugin is told for a player who exists, a friend graph, a party area
//! that is not ported yet, and a module with no database. The rules' own cases are in `isles-chat`.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::Arc;

use isles_chat::login::{TokenRefused, TokenVerifier, authenticate};
use isles_chat::{ChatRules, ChatVersion, IslesChat, hub_sessions};
use isles_core::Settings;
use isles_core::areas::chat::rules::source;
use isles_core::db::IslesStore;
use isles_core::repo::auth::AccountRecord;
use isles_core::repo::chat::{FriendEdgeRecord, FriendshipDirection, FriendshipStatus};
use isles_core::{AccountId, UserId};
use module_api::chat::{ChatAnswer, ChatQuery, RoomKind, RoomRules};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, ErrorKind, GlobalId, IdKind, NodeIdentity};
use store::{DatabaseConfig, DatabaseOptions};

fn open(dir: &tempfile::TempDir) -> IslesStore {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    IslesStore::open(&config, &DatabaseOptions::default(), home, &node).unwrap()
}

async fn account(store: &IslesStore, user: &str) -> AccountRecord {
    let record = AccountRecord {
        id: AccountId::new(&NodeIdentity::standalone().node),
        user_id: UserId(user.into()),
        name: format!("Name of {user}"),
        notes: 0,
    };
    let (id, created) = (record.id.clone(), record.clone());
    let actor = ActorContext::new(Actor::Admin { key: "owner".into() });
    store
        .account_write(&id, &actor, move |tx, repos| repos.accounts.create(tx, &created))
        .await
        .unwrap();
    record
}

const RULES: IslesChat = IslesChat {
    version: ChatVersion::V1147,
};

struct Tokens;

impl TokenVerifier for Tokens {
    fn user_id(&self, token: &str) -> Result<Option<String>, TokenRefused> {
        token
            .strip_prefix("jwt:")
            .map(|user| Some(user.to_owned()))
            .ok_or(TokenRefused)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_plugin_is_told_the_jid_and_the_rooms_of_a_real_player() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let alice = account(&store, "4e0c1d1e-1f4f-4a39-9e2b-6f9a1d7c0b11").await;
    let eve = account(&store, "eve").await;
    let node = NodeIdentity::standalone();
    let chat = source(&node, Some(store.clone()), &Settings::default());

    let answer = RULES
        .answer(
            &chat,
            &ChatQuery::Jid {
                account: alice.id.global().clone(),
            },
        )
        .await
        .unwrap();
    let ChatAnswer::Jid(jid) = answer else { panic!("a JID") };
    assert_eq!(
        jid.to_string(),
        "4e0c1d1e-1f4f-4a39-9e2b-6f9a1d7c0b11@prod.ol.epicgames.com"
    );

    // The region room opens for the player matchmaking sent to a hub of the node's region, and for no one else
    let room = RoomKind::Region(node.region.clone());
    let rules = |account: &AccountRecord| {
        let query = ChatQuery::Room {
            room: room.clone(),
            account: account.id.global().clone(),
        };
        let chat = &chat;
        async move { RULES.answer(chat, &query).await }
    };
    let ChatAnswer::Room(shut) = rules(&alice).await.unwrap() else {
        panic!("rules")
    };
    assert_eq!(
        shut,
        RoomRules {
            may_join: false,
            may_speak: false,
            history: false
        }
    );
    hub_sessions().record(
        alice.id.global(),
        "179cf2a8-0716-49f5-9027-6e4876ba0d76",
        &node.region,
        isles_chat::rooms::now_millis(),
    );
    let ChatAnswer::Room(open) = rules(&alice).await.unwrap() else {
        panic!("rules")
    };
    assert!(open.may_join && open.may_speak && !open.history);
    let ChatAnswer::Room(still_shut) = rules(&eve).await.unwrap() else {
        panic!("rules")
    };
    assert!(!still_shut.may_join);

    // A login: the token's userId must be the authcid, and the account must exist
    let player = authenticate(&alice.user_id.0, &format!("jwt:{}", alice.user_id), &Tokens, &chat)
        .await
        .unwrap();
    assert_eq!(
        (player.account, player.name),
        (alice.id.global().clone(), alice.name.clone())
    );
    let ghost = authenticate("ghost", "jwt:ghost", &Tokens, &chat).await;
    assert_eq!(ghost.unwrap_err().as_str(), "account not eligible");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_party_room_waits_for_the_party_area_and_a_missing_account_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let alice = account(&store, "alice").await;
    let chat = source(&NodeIdentity::standalone(), Some(store), &Settings::default());
    // The parties repository is still RU4's stub: the rules cannot say who is in a party, so the join is refused
    // (the plugin treats any error but `unsupported` as a refusal)
    let party = GlobalId::new(IdKind::Party, &NodeIdentity::standalone().node);
    let query = ChatQuery::Room {
        room: RoomKind::Party(party),
        account: alice.id.global().clone(),
    };
    assert_eq!(
        RULES.answer(&chat, &query).await.unwrap_err().kind,
        ErrorKind::Unavailable
    );

    let stranger = GlobalId::new(IdKind::Account, &NodeIdentity::standalone().node);
    let error = RULES
        .answer(&chat, &ChatQuery::Jid { account: stranger })
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotFound);
    let hunt = GlobalId::new(IdKind::Hunt, &NodeIdentity::standalone().node);
    let not_an_account = ChatQuery::Jid { account: hunt };
    assert_eq!(
        RULES.answer(&chat, &not_an_account).await.unwrap_err().kind,
        ErrorKind::Invalid
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn friends_are_the_accepted_edges() {
    use isles_chat::ChatSource as _;
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let (alice, bob, carol) = (
        account(&store, "alice").await,
        account(&store, "bob").await,
        account(&store, "carol").await,
    );
    let edge = |owner: &AccountRecord, other: &AccountRecord, status, direction| FriendEdgeRecord {
        owner: owner.id.clone(),
        other: other.id.clone(),
        other_user_id: other.user_id.clone(),
        status,
        direction,
        created: "2026-10-20T12:00:00.000Z".into(),
        favorite: false,
    };
    let actor = ActorContext::new(Actor::Admin { key: "owner".into() });
    for e in [
        edge(&alice, &bob, FriendshipStatus::Accepted, None),
        edge(
            &alice,
            &carol,
            FriendshipStatus::Pending,
            Some(FriendshipDirection::Outbound),
        ),
    ] {
        store
            .write(&actor, move |tx, repos| repos.friendships.upsert(tx, &e))
            .await
            .unwrap();
    }
    let chat = source(&NodeIdentity::standalone(), Some(store), &Settings::default());
    assert_eq!(
        chat.friends(alice.id.global()).await.unwrap(),
        vec![bob.id.global().clone()]
    );
    assert!(chat.friends(carol.id.global()).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_database_the_jid_and_party_questions_are_unsupported_so_the_plugin_uses_its_defaults() {
    let chat = source(&NodeIdentity::standalone(), None, &Settings::default());
    let account = GlobalId::new(IdKind::Account, &NodeIdentity::standalone().node);
    for query in [
        ChatQuery::Jid {
            account: account.clone(),
        },
        ChatQuery::Room {
            room: RoomKind::Party(GlobalId::new(IdKind::Party, &NodeIdentity::standalone().node)),
            account,
        },
    ] {
        assert_eq!(
            RULES.answer(&chat, &query).await.unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }
}
