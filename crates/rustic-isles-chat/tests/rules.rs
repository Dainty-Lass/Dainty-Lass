// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The chat rules over a source in memory and over a temporary store (spec §11.2, rules table).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::Arc;

use module_api::chat::{ChatAnswer, ChatQuery, RoomKind, RoomRules};
use module_api::{
    Actor, ActorContext, BoxFuture, ErrorKind, GlobalId, HomeInvocation, IdKind, ModuleError, ModuleId, NodeId,
    NodeIdentity, OperationName, Region,
};
use rustic_isles::replay::{ReplayScope, with_replay_scope};
use rustic_isles::testing::sample_version;
use rustic_isles::{AccountId, Settings, Shared, SharedParts, UserId};
use rustic_isles_chat::ids::room_id_text;
use rustic_isles_chat::{
    ChatRules, ChatSettings, ChatSource, HubSessions, InvalidToken, IslesChat, Player, StoreChat, TokenCheck,
    home_operation,
};
use rustic_isles_store::repo::auth::OperationalStatus;
use rustic_isles_store::repo::chat::{FriendEdgeRecord, FriendshipDirection, FriendshipStatus};
use rustic_isles_store::repo::parties::NewParty;
use rustic_isles_store::test_support::TempStore;
use rustic_isles_store::{IslesStore, Repositories, Stub};
use serde_json::{Map, Value, json};

const OPEN: RoomRules = RoomRules {
    may_join: true,
    may_speak: true,
    history: false,
};
const REFUSED: RoomRules = RoomRules {
    may_join: false,
    may_speak: false,
    history: false,
};

fn module() -> ModuleId {
    ModuleId::new("isles-1-14-7").unwrap()
}

fn shared(pairs: &[(&str, &str)]) -> Arc<Shared> {
    let (private, public) = rustic_isles_auth::test_support::throwaway_signing_keys();
    let mut all = vec![
        ("AUTH_SIGNING_PRIVKEY_B64", private.as_str()),
        ("AUTH_SIGNING_PUBKEY_B64", public.as_str()),
    ];
    all.extend_from_slice(pairs);
    let mut parts = SharedParts::bare(sample_version());
    parts.settings = Settings::from_pairs(all);
    Shared::new(parts).unwrap()
}

struct World {
    temp: TempStore,
    shared: Arc<Shared>,
    chat: StoreChat,
    hubs: HubSessions,
}

impl World {
    fn new(pairs: &[(&str, &str)]) -> Self {
        let temp = TempStore::open().unwrap();
        let shared = shared(pairs);
        let chat = StoreChat::for_module(&shared, module(), Some(temp.store.clone()));
        Self {
            temp,
            shared,
            chat,
            hubs: HubSessions::new(),
        }
    }

    fn store(&self) -> &IslesStore {
        &self.temp.store
    }

    async fn ask(&self, query: ChatQuery) -> Result<ChatAnswer, ModuleError> {
        IslesChat.answer(&self.chat, &self.hubs, &query).await
    }

    async fn room(&self, room: RoomKind, account: &AccountId) -> Result<ChatAnswer, ModuleError> {
        self.ask(ChatQuery::Room {
            room,
            account: account.global().clone(),
        })
        .await
    }

    async fn party(&self, member: &AccountId, party_id: impl Fn(&GlobalId) -> String) -> GlobalId {
        let id = GlobalId::new(IdKind::Party, self.store().node());
        let new_party = NewParty {
            id: id.clone(),
            party_id: party_id(&id),
        };
        let member = member.clone();
        self.store()
            .write(&common::system(), move |tx, repos| {
                repos
                    .parties
                    .get_or_create_solo(tx, &member, "647472", &new_party, "2026-10-20T12:00:00.000Z")
            })
            .await
            .unwrap();
        id
    }
}

fn kind(result: Result<ChatAnswer, ModuleError>) -> (ErrorKind, String) {
    let error = result.unwrap_err();
    (error.kind, error.message)
}

#[tokio::test(flavor = "multi_thread")]
async fn jids() {
    let world = World::new(&[]);
    let one = common::account(world.store(), "u-1").await;
    let answer = world
        .ask(ChatQuery::Jid {
            account: one.global().clone(),
        })
        .await;
    let ChatAnswer::Jid(jid) = answer.unwrap() else {
        panic!()
    };
    assert_eq!(
        (jid.to_string(), jid.resource()),
        ("u-1@prod.ol.epicgames.com".to_owned(), None)
    );

    let unknown = AccountId::new(world.store().node());
    let (k, message) = kind(
        world
            .ask(ChatQuery::Jid {
                account: unknown.global().clone(),
            })
            .await,
    );
    assert_eq!((k, message), (ErrorKind::NotFound, format!("no account {unknown}")));
    let hunt = GlobalId::new(IdKind::Hunt, world.store().node());
    assert_eq!(
        kind(world.ask(ChatQuery::Jid { account: hunt }).await).0,
        ErrorKind::Invalid
    );
    let spaced = common::account(world.store(), "has space").await;
    let (k, message) = kind(
        world
            .ask(ChatQuery::Jid {
                account: spaced.global().clone(),
            })
            .await,
    );
    assert_eq!(k, ErrorKind::Invalid);
    assert!(message.starts_with("the userId is not a JID local part ("), "{message}");

    // No database
    let bare = StoreChat::for_module(&world.shared, module(), None);
    let query = ChatQuery::Jid {
        account: one.global().clone(),
    };
    let error = IslesChat.answer(&bare, &world.hubs, &query).await.unwrap_err();
    assert_eq!(
        (error.kind, error.message.as_str()),
        (ErrorKind::Unsupported, "chat without a database")
    );
    let party = GlobalId::new(IdKind::Party, world.store().node());
    let query = ChatQuery::Room {
        room: RoomKind::Party(party),
        account: one.global().clone(),
    };
    assert_eq!(
        IslesChat.answer(&bare, &world.hubs, &query).await.unwrap_err().kind,
        ErrorKind::Unsupported
    );
    // Region and hunt rules read no store
    let query = ChatQuery::Room {
        room: RoomKind::Hunt(GlobalId::new(IdKind::Hunt, world.store().node())),
        account: one.global().clone(),
    };
    assert_eq!(
        IslesChat.answer(&bare, &world.hubs, &query).await,
        Ok(ChatAnswer::Room(OPEN))
    );
    let login = ChatQuery::Login {
        authcid: "u-1".into(),
        token: "t".into(),
    };
    let error = IslesChat.answer(&bare, &world.hubs, &login).await.unwrap_err();
    assert_eq!(
        (error.kind, error.message.as_str()),
        (ErrorKind::Unsupported, "chat login without a token verifier")
    );
    assert!(!format!("{bare:?}").contains("PRIVATE"));
}

#[tokio::test(flavor = "multi_thread")]
async fn party_rooms() {
    let world = World::new(&[]);
    let alice = common::account(world.store(), "alice").await;
    let bob = common::account(world.store(), "bob").await;
    let carol = common::account(world.store(), "carol").await;
    let ours = world.party(&alice, |id| format!("{}_QnVpbGQ=", room_id_text(id))).await;
    assert_eq!(
        world.room(RoomKind::Party(ours.clone()), &alice).await,
        Ok(ChatAnswer::Room(OPEN))
    );
    assert_eq!(
        world.room(RoomKind::Party(ours.clone()), &bob).await,
        Ok(ChatAnswer::Room(REFUSED))
    );
    let theirs = world.party(&carol, ToString::to_string).await;
    assert_eq!(
        world.room(RoomKind::Party(theirs.clone()), &carol).await,
        Ok(ChatAnswer::Room(OPEN))
    );
    assert_eq!(
        world.room(RoomKind::Party(ours.clone()), &carol).await,
        Ok(ChatAnswer::Room(REFUSED))
    );
    assert_eq!(
        world.room(RoomKind::Party(theirs), &alice).await,
        Ok(ChatAnswer::Room(REFUSED))
    );

    // The lookup fails
    let failing = Repositories {
        parties: Arc::new(Stub),
        ..Repositories::on_store()
    };
    let store = world.store().clone().with_repos(failing);
    let chat = StoreChat::for_module(&world.shared, module(), Some(store));
    let query = ChatQuery::Room {
        room: RoomKind::Party(ours),
        account: alice.global().clone(),
    };
    let error = IslesChat.answer(&chat, &world.hubs, &query).await.unwrap_err();
    assert_eq!(
        (error.kind, error.message.as_str()),
        (ErrorKind::Unavailable, "parties: find_by_member is not implemented yet")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn region_hunt_and_network_rooms() {
    let world = World::new(&[]);
    let alice = common::account(world.store(), "alice").await;
    let bob = common::account(world.store(), "bob").await;
    let local = Region::new("local").unwrap();
    world.hubs.record(alice.global(), "hub-1", &local, 0);
    let at_one = ReplayScope::new(Some(1), None);
    let answers = with_replay_scope(at_one, async {
        [
            world.room(RoomKind::Region(local.clone()), &alice).await,
            world.room(RoomKind::Region(local.clone()), &bob).await,
            world
                .room(RoomKind::Region(Region::new("germany").unwrap()), &alice)
                .await,
        ]
    })
    .await;
    assert_eq!(
        answers,
        [
            Ok(ChatAnswer::Room(OPEN)),
            Ok(ChatAnswer::Room(REFUSED)),
            Ok(ChatAnswer::Room(REFUSED))
        ]
    );
    let hunt = RoomKind::Hunt(GlobalId::new(IdKind::Hunt, world.store().node()));
    assert_eq!(world.room(hunt.clone(), &bob).await, Ok(ChatAnswer::Room(OPEN)));
    assert_eq!(
        world.room(RoomKind::Network, &alice).await,
        Ok(ChatAnswer::Room(REFUSED))
    );

    let dev = World::new(&[
        ("REALTIME_XMPP_DEV_CITY_MUC", " true "),
        ("REALTIME_XMPP_DEV_HUNT_MUC", "false"),
    ]);
    let anyone = AccountId::new(dev.store().node());
    assert_eq!(
        dev.room(RoomKind::Region(local), &anyone).await,
        Ok(ChatAnswer::Room(OPEN))
    );
    assert_eq!(dev.room(hunt, &anyone).await, Ok(ChatAnswer::Room(REFUSED)));
}

async fn room_of(world: &World, name: &str) -> Option<RoomKind> {
    match world.ask(ChatQuery::RoomOf { name: name.into() }).await.unwrap() {
        ChatAnswer::RoomOf(room) => room,
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn room_names() {
    let world = World::new(&[]);
    let node = NodeId::new("local").unwrap();
    let party = GlobalId::new(IdKind::Party, &NodeId::new("oregon-1").unwrap());
    assert_eq!(
        room_of(&world, &format!("Party-{}_QnVpbGQ=", room_id_text(&party))).await,
        Some(RoomKind::Party(party))
    );
    let uuid = "0189ABCD-EF01-7234-8567-89ABCDEF0123";
    let Some(RoomKind::Hunt(hunt)) = room_of(&world, &format!("Hunt-{uuid}")).await else {
        panic!()
    };
    assert_eq!(
        (hunt.owner(), u128::from(hunt.ulid())),
        (&node, 0x0189_ABCD_EF01_7234_8567_89AB_CDEF_0123)
    );
    let account = AccountId::new(&node);
    let germany = Region::new("germany").unwrap();
    world.hubs.record(
        account.global(),
        "Ramsgate_7f3e",
        &germany,
        rustic_isles::replay::now_ms(),
    );
    assert_eq!(
        room_of(&world, "City-Ramsgate_7f3e").await,
        Some(RoomKind::Region(germany.clone()))
    );
    assert_eq!(room_of(&world, "City-germany").await, Some(RoomKind::Region(germany)));
    assert_eq!(
        room_of(&world, "City-1abc").await,
        Some(RoomKind::Region(Region::new("local").unwrap()))
    );
    for none in ["City-", "General", "Party-xyz", "party-x", "Guild-1"] {
        assert_eq!(room_of(&world, none).await, None, "{none}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn friends_and_whispers() {
    let world = World::new(&[]);
    let alice = common::account(world.store(), "alice").await;
    let mut others = Vec::new();
    for (user, status) in [
        ("bob", FriendshipStatus::Accepted),
        ("carol", FriendshipStatus::Pending),
        ("dave", FriendshipStatus::Blocked),
        ("erin", FriendshipStatus::Accepted),
    ] {
        let other = common::account(world.store(), user).await;
        let record = FriendEdgeRecord {
            owner: alice.clone(),
            other: other.clone(),
            other_user_id: UserId::new(user),
            status,
            direction: Some(FriendshipDirection::Outbound),
            created: "2026-10-20T12:00:00.000Z".into(),
            favorite: false,
        };
        world
            .store()
            .write(&common::system(), move |tx, repos| {
                repos.friendships.upsert(tx, &record)
            })
            .await
            .unwrap();
        others.push(other);
    }
    let answer = world
        .ask(ChatQuery::Friends {
            account: alice.global().clone(),
        })
        .await;
    assert_eq!(
        answer,
        Ok(ChatAnswer::Friends(vec![
            others[0].global().clone(),
            others[3].global().clone()
        ]))
    );
    let whisper = ChatQuery::MayWhisper {
        from: alice.global().clone(),
        to: others[2].global().clone(),
    };
    assert_eq!(world.ask(whisper).await, Ok(ChatAnswer::MayWhisper(true)));
}

#[tokio::test(flavor = "multi_thread")]
async fn account_of() {
    let world = World::new(&[]);
    let u1 = common::account(world.store(), "u-1").await;
    let alice = common::launcher(world.store(), "u-2", "Alice", |_| {}).await;
    common::launcher(world.store(), "u-3", "Banned", |r| r.status = OperationalStatus::Banned).await;
    let ask = |local: &str| world.ask(ChatQuery::AccountOf { local: local.into() });
    assert_eq!(ask("u-1").await, Ok(ChatAnswer::AccountOf(Some(u1.global().clone()))));
    assert_eq!(
        ask(" Alice ").await,
        Ok(ChatAnswer::AccountOf(Some(alice.global().clone())))
    );
    assert_eq!(
        ask("u-2").await,
        Ok(ChatAnswer::AccountOf(Some(alice.global().clone())))
    );
    assert_eq!(ask("banned").await, Ok(ChatAnswer::AccountOf(None)));
    assert_eq!(ask("u-3").await, Ok(ChatAnswer::AccountOf(None)));
    assert_eq!(ask("  ").await, Ok(ChatAnswer::AccountOf(None)));
}

/// A source in memory: token `ok:<user>` is valid, `alice` is a player, `ghost` has no account, and a lookup of
/// `broken` fails.
struct Fake {
    node: NodeIdentity,
    alice: Player,
}

struct OkTokens;

impl TokenCheck for OkTokens {
    fn validate(&self, token: &str) -> Result<Map<String, Value>, InvalidToken> {
        let user = token.strip_prefix("ok:").ok_or(InvalidToken)?;
        let mut claims = Map::new();
        claims.insert("userId".into(), user.into());
        Ok(claims)
    }
}

impl ChatSource for Fake {
    fn node(&self) -> &NodeIdentity {
        &self.node
    }

    fn settings(&self) -> ChatSettings {
        ChatSettings::default()
    }

    fn token_check(&self) -> Option<&dyn TokenCheck> {
        Some(&OkTokens)
    }

    fn player_by_account<'a>(&'a self, _: &'a GlobalId) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async { Ok(None) })
    }

    fn player_by_user_id<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, Result<Option<Player>, ModuleError>> {
        Box::pin(async move {
            match user_id {
                "alice" => Ok(Some(self.alice.clone())),
                "broken" => Err(ModuleError::new(ErrorKind::Unavailable, "database: gone")),
                _ => Ok(None),
            }
        })
    }

    fn account_of<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Option<GlobalId>, ModuleError>> {
        Box::pin(async { Ok(None) })
    }

    fn cross_node_login<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Option<Player>> {
        Box::pin(async { None })
    }

    fn party_of<'a>(&'a self, _: &'a GlobalId) -> BoxFuture<'a, Result<Option<String>, ModuleError>> {
        Box::pin(async { Ok(None) })
    }

    fn friends<'a>(&'a self, _: &'a GlobalId) -> BoxFuture<'a, Result<Vec<GlobalId>, ModuleError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[tokio::test]
async fn login_rules() {
    let node = NodeIdentity::standalone();
    let alice = Player {
        account: GlobalId::new(IdKind::Account, &node.node),
        user_id: "alice".into(),
        name: "Alice".into(),
    };
    let fake = Fake {
        node,
        alice: alice.clone(),
    };
    let hubs = HubSessions::new();
    let login = |authcid: &str, token: &str| {
        let query = ChatQuery::Login {
            authcid: authcid.into(),
            token: token.into(),
        };
        let fake = &fake;
        let hubs = &hubs;
        async move { IslesChat.answer(fake, hubs, &query).await }
    };
    assert_eq!(
        login("alice", "ok:alice").await,
        Ok(ChatAnswer::Login {
            account: alice.account.clone(),
            name: "Alice".into()
        })
    );
    let refused = |text: &str| Err(ModuleError::new(ErrorKind::Invalid, text));
    assert_eq!(login("alice", "forged").await, refused("jwt verify failed"));
    assert_eq!(login("", "ok:").await, refused("no userId in token"));
    assert_eq!(login("mallory", "ok:alice").await, refused("authcid/token mismatch"));
    assert_eq!(login("alice", "ok:mallory").await, refused("authcid/token mismatch"));
    assert_eq!(login("ghost", "ok:ghost").await, refused("account not eligible"));
    assert_eq!(
        login("__launcher_client_credentials__", "ok:__launcher_client_credentials__").await,
        refused("client-credentials token not allowed")
    );
    assert_eq!(
        login("broken", "ok:broken").await,
        Err(ModuleError::new(ErrorKind::Unavailable, "account lookup error"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn login_over_real_tokens_and_a_store() {
    let world = World::new(&[]);
    let alice = common::launcher(world.store(), "alice", "Alice", |_| {}).await;
    common::launcher(world.store(), "banned", "Banned", |r| {
        r.status = OperationalStatus::Banned
    })
    .await;
    let auth = world.shared.auth();
    let token = auth.sign_game_token("alice").unwrap();
    let login = |authcid: &str, token: &str| {
        world.ask(ChatQuery::Login {
            authcid: authcid.into(),
            token: token.into(),
        })
    };
    assert_eq!(
        login("alice", &token).await,
        Ok(ChatAnswer::Login {
            account: alice.global().clone(),
            name: "Alice".into()
        })
    );
    let launcher = auth.sign_launcher_token("alice", "sid-1").unwrap();
    assert_eq!(
        login("alice", &launcher).await,
        Err(ModuleError::new(ErrorKind::Invalid, "jwt verify failed"))
    );
    let banned = auth.sign_game_token("banned").unwrap();
    assert_eq!(
        login("banned", &banned).await,
        Err(ModuleError::new(ErrorKind::Invalid, "account not eligible"))
    );
    // An account id of another node as authcid, with no network to ask: refused as the token was
    let foreign = GlobalId::new(IdKind::Account, &NodeId::new("elsewhere").unwrap());
    assert_eq!(
        login(&foreign.to_string(), &token).await,
        Err(ModuleError::new(ErrorKind::Invalid, "authcid/token mismatch"))
    );
}

fn invocation(account: &AccountId, operation: &str, body: Value) -> HomeInvocation {
    HomeInvocation {
        from: NodeId::new("elsewhere").unwrap(),
        account: account.global().clone(),
        actor: ActorContext::new(Actor::System),
        operation: OperationName::new(operation).unwrap(),
        body,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn home_operations() {
    let world = World::new(&[]);
    let alice = common::launcher(world.store(), "alice", "Alice", |_| {}).await;
    let bob = common::account(world.store(), "bob").await;
    let token = world.shared.auth().sign_game_token("alice").unwrap();
    let run = |account: &AccountId, operation: &str, body: Value| {
        let call = invocation(account, operation, body);
        let chat = &world.chat;
        let operation = operation.to_owned();
        async move { home_operation(&operation, chat, &call).await }
    };

    let body = json!({"authcid": "alice", "token": token});
    assert_eq!(
        run(&alice, "chat.login", body.clone()).await,
        Some(Ok(
            json!({"account": alice.to_string(), "user_id": "alice", "name": "Alice"})
        ))
    );
    assert_eq!(
        run(&bob, "chat.login", body).await,
        Some(Err(ModuleError::new(
            ErrorKind::Invalid,
            "token account does not match target account"
        )))
    );
    assert_eq!(
        run(&alice, "chat.login", json!({"authcid": 7})).await,
        Some(Err(ModuleError::new(ErrorKind::Invalid, "jwt verify failed")))
    );
    let player = run(&bob, "chat.player", json!({})).await.unwrap().unwrap();
    assert_eq!(
        service_kit::jsjson::stringify(&player),
        format!(r#"{{"account":"{bob}","user_id":"bob","name":"bob name"}}"#)
    );
    let unknown = AccountId::new(world.store().node());
    assert_eq!(
        run(&unknown, "chat.player", json!({})).await,
        Some(Ok(json!({"player": null})))
    );
    assert_eq!(
        run(&bob, "chat.party", json!({})).await,
        Some(Ok(json!({"party_id": null})))
    );
    world.party(&bob, |_| "p1_QnVpbGQ=".into()).await;
    assert_eq!(
        run(&bob, "chat.party", json!({})).await,
        Some(Ok(json!({"party_id": "p1_QnVpbGQ="})))
    );
    assert_eq!(run(&bob, "inventory.read", json!({})).await, None);
}

#[test]
fn settings_table() {
    let read = |pairs: &[(&str, &str)]| {
        let s = ChatSettings::from_settings(&Settings::from_pairs(pairs.iter().copied()));
        (s.dev_city_muc, s.dev_hunt_muc)
    };
    assert_eq!(read(&[]), (false, true));
    assert_eq!(
        ChatSettings::default(),
        ChatSettings::from_settings(&Settings::default())
    );
    let both = |value: &'static str| {
        [
            ("REALTIME_XMPP_DEV_CITY_MUC", value),
            ("REALTIME_XMPP_DEV_HUNT_MUC", value),
        ]
    };
    assert_eq!(read(&both(" true ")), (true, true));
    assert_eq!(read(&both("")), (false, true));
    assert_eq!(read(&both("false")), (false, false));
    assert_eq!(read(&both("TRUE")), (false, true));
    assert_eq!(read(&both(" false ")), (false, false));
}
