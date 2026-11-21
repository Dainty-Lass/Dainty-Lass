// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `/reset` through the chat rules over a temporary store: nothing changes without the confirm, the confirm leaves a
//! new account's save, and other slash lines are not the module's.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::Arc;

use dainty_module_api::chat::{ChatAnswer, ChatQuery};
use dainty_module_api::{ErrorKind, ModuleError, ModuleId};
use rustic_isles::testing::sample_version;
use rustic_isles::{AccountId, Settings, Shared, SharedParts};
use rustic_isles_chat::{ChatRules, HubSessions, IslesChat, StoreChat};
use rustic_isles_store::IslesStore;
use rustic_isles_store::repo::characters::CharacterRecord;
use rustic_isles_store::test_support::TempStore;

fn shared() -> Arc<Shared> {
    let (private, public) = rustic_isles_auth::test_support::throwaway_signing_keys();
    let mut parts = SharedParts::bare(sample_version());
    parts.settings = Settings::from_pairs(vec![
        ("AUTH_SIGNING_PRIVKEY_B64", private.as_str()),
        ("AUTH_SIGNING_PUBKEY_B64", public.as_str()),
    ]);
    Shared::new(parts).unwrap()
}

struct World {
    temp: TempStore,
    chat: StoreChat,
    hubs: HubSessions,
}

impl World {
    fn new() -> Self {
        let temp = TempStore::open().unwrap();
        let chat = StoreChat::for_module(
            &shared(),
            ModuleId::new("isles-1-14-7").unwrap(),
            Some(temp.store.clone()),
        );
        Self {
            temp,
            chat,
            hubs: HubSessions::new(),
        }
    }

    fn store(&self) -> &IslesStore {
        &self.temp.store
    }

    async fn say(&self, account: &AccountId, line: &str, lang: Option<&str>) -> Result<String, ModuleError> {
        let query = ChatQuery::Command {
            account: account.global().clone(),
            line: line.to_owned(),
            lang: lang.map(str::to_owned),
        };
        match IslesChat.answer(&self.chat, &self.hubs, &query).await? {
            ChatAnswer::Reply(text) => Ok(text),
            other => panic!("{other:?}"),
        }
    }

    async fn characters(&self, account: &AccountId) -> Vec<String> {
        let w = account.clone();
        self.store()
            .read(move |conn, r| r.characters.find_many_by_account(conn, &w))
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.character_id)
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reset_needs_its_confirm_and_leaves_a_new_save() {
    let world = World::new();
    let a = common::account(world.store(), "u-1").await;
    let w = a.clone();
    world
        .store()
        .write(&common::system(), move |tx, r| {
            r.characters.create(
                tx,
                &CharacterRecord {
                    character_id: "c1".into(),
                    account: w.clone(),
                    created_date: "d".into(),
                    last_modified_date: "d".into(),
                    name: "n".into(),
                    update_version: 0,
                    data: "{}".into(),
                },
            )?;
            r.progression_tracks.increment(tx, &w, "xp", 3).map(|_| ())
        })
        .await
        .unwrap();

    // A confirm with nothing asked, and the ask itself, change nothing
    let early = world.say(&a, "/reset confirm", None).await.unwrap();
    assert!(early.contains("Type /reset first"), "{early}");
    let asked = world.say(&a, "/reset", None).await.unwrap();
    assert!(asked.contains("/reset confirm") && asked.contains("rejoin"), "{asked}");
    assert_eq!(world.characters(&a).await, ["c1"]);

    // Another player's confirm doesn't take this one's ask
    let b = common::account(world.store(), "u-2").await;
    assert!(world.say(&b, "/reset confirm", None).await.unwrap().contains("first"));
    assert_eq!(world.characters(&a).await, ["c1"]);

    let done = world.say(&a, "/RESET Confirm", None).await.unwrap();
    assert!(done.contains("was reset") && done.contains("rejoin"), "{done}");
    assert!(world.characters(&a).await.is_empty());
    let w = a.clone();
    let (account, xp) = world
        .store()
        .read(move |conn, r| Ok((r.accounts.find(conn, &w)?, r.progression_tracks.get(conn, &w, "xp")?)))
        .await
        .unwrap();
    assert_eq!(account.unwrap().name, "u-1 name");
    assert!(xp.is_none());

    // A day before the next, in the game's language
    let soon = world.say(&a, "/reset", Some("pt-BR")).await.unwrap();
    assert!(soon.contains("24 horas") && soon.ends_with("UTC."), "{soon}");
    assert!(world.say(&a, "/reset confirm", None).await.unwrap().contains("first"));
}

#[tokio::test(flavor = "multi_thread")]
async fn other_slash_lines_are_not_the_modules() {
    let world = World::new();
    let a = common::account(world.store(), "u-1").await;
    for line in ["/shrug", "/resets", "reset", "hello /reset"] {
        let error = world.say(&a, line, None).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unsupported, "{line}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reset_without_a_database_still_answers() {
    let temp = TempStore::open().unwrap();
    let chat = StoreChat::for_module(&shared(), ModuleId::new("isles-1-14-7").unwrap(), None);
    let a = common::account(&temp.store, "u-1").await;
    let query = ChatQuery::Command {
        account: a.global().clone(),
        line: "/reset".into(),
        lang: None,
    };
    let answer = IslesChat.answer(&chat, &HubSessions::new(), &query).await.unwrap();
    assert_eq!(
        answer,
        ChatAnswer::Reply(rustic_isles_chat::reset::failed(None)),
        "never relayed, even where saves can't be reset"
    );
}
