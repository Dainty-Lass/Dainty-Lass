// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The `chat.*` home operations (spec §5.9, core §7.8), run on the account's home for another node's chat.
//! dainty-lass's plugin calls `chat.login` with actor System too, so the actor is not checked.

use dainty_module_api::{ErrorKind, HomeInvocation, ModuleError};
use rustic_isles::AccountId;
use serde_json::{Map, Value, json};

use crate::source::no_database;
use crate::{Player, StoreChat, authenticate};

fn player_json(player: &Player) -> Value {
    json!({"account": player.account.to_string(), "user_id": player.user_id, "name": player.name})
}

fn text<'a>(body: &'a Value, key: &str) -> &'a str {
    body.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `None` for an operation that is not one of chat's.
pub async fn home_operation(
    name: &str,
    chat: &StoreChat,
    invocation: &HomeInvocation,
) -> Option<Result<Value, ModuleError>> {
    if !matches!(name, "chat.login" | "chat.player" | "chat.party") {
        return None;
    }
    if chat.store().is_none() {
        return Some(Err(no_database()));
    }
    let account = match AccountId::try_from(invocation.account.clone()) {
        Ok(account) => account,
        Err(e) => return Some(Err(ModuleError::new(ErrorKind::Invalid, e.to_string()))),
    };
    let failed = |e: rustic_isles_store::RepoError| ModuleError::failed(e.to_string());
    Some(match name {
        "chat.login" => {
            // Steps 1-5 only: a lookup failure here is a refusal like the others
            let (authcid, token) = (text(&invocation.body, "authcid"), text(&invocation.body, "token"));
            match authenticate(authcid, token, chat).await {
                Err(refusal) => Err(ModuleError::new(ErrorKind::Invalid, refusal.as_str())),
                Ok(player) if &player.account != account.global() => Err(ModuleError::new(
                    ErrorKind::Invalid,
                    "token account does not match target account",
                )),
                Ok(player) => Ok(player_json(&player)),
            }
        }
        "chat.player" => chat.local_player(&account).await.map_err(failed).map(|p| match p {
            Some(player) => player_json(&player),
            None => {
                let mut out = Map::new();
                out.insert("player".into(), Value::Null);
                Value::Object(out)
            }
        }),
        _ => chat
            .local_party(&account)
            .await
            .map_err(failed)
            .map(|party| json!({"party_id": party})),
    })
}
