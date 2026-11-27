// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Claim codes for an old Ramsgate Reborn save (`docs/port/spec-legacy-ramsgate.md`): staff make one for a player
//! whose old account had no Discord link, the player redeems it before their first play, and the importer copies
//! that save at the account's first character read. Only the code's SHA-256 is stored; the code is shown once.

use axum::response::Response;
use dainty_module_api::{Actor, ActorContext};
use http::StatusCode;
use rustic_isles::Call;
use rustic_isles_store::repo::legacy::{self, CLAIM_KEEP_MS, RAMSGATE_REBORN, Redeem};
use rustic_isles_store::repo::reforged::AdminAuditRecord;
use serde_json::Value;

use crate::admin::{AdminContext, admin_actor, admin_error, admin_internal, fresh_id};
use crate::launcher_api::key_holder;
use crate::util::{self, iso, json, now_ms, object, store_of};

/// Crockford's base32: no I, L, O or U, so a code read aloud or retyped survives.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// 20 characters, 100 bits.
const LENGTH: usize = 20;
/// An old account's user id, as the admin types it.
const MAX_SOURCE_USER: usize = 128;

/// A new code, grouped `XXXXX-XXXXX-XXXXX-XXXXX`.
pub fn new_code() -> String {
    let bytes = util::random_bytes::<LENGTH>();
    let chars: Vec<char> = bytes
        .iter()
        .map(|b| char::from(ALPHABET[usize::from(b & 31)]))
        .collect();
    chars
        .chunks(5)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// The code as stored, or none when it can't be one: case and dashes or spaces ignored, O read as 0 and I or L as 1.
pub fn normalize(text: &str) -> Option<String> {
    let mut out = String::with_capacity(LENGTH);
    for c in text.chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !ALPHABET.contains(&(c as u8)) || out.len() == LENGTH {
            return None;
        }
        out.push(c);
    }
    (out.len() == LENGTH).then_some(out)
}

/// What the store keeps of a code.
pub fn code_hash(normalized: &str) -> String {
    util::sha256_hex(format!("legacy-claim:{normalized}").as_bytes())
}

fn refused(status: StatusCode, error: &str) -> Response {
    json(status, &object(vec![("error", Value::from(error))]))
}

/// `POST /dainty/api/LegacyClaim` `{"Code"}`: binds the code's old save to the key holder's account.
pub async fn redeem(call: Call) -> Response {
    let Call {
        shared,
        route,
        mut request,
        request_id,
        ..
    } = call;
    let Some(store) = store_of(&shared) else {
        return util::no_store(route, &request_id);
    };
    let Some((holder, _)) = key_holder(&shared, &store, request.headers()).await else {
        return refused(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    if !holder.eligible() {
        return refused(StatusCode::FORBIDDEN, "account_disabled");
    }
    let body = match util::read_json(&mut request).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    let code = util::field(body.as_ref(), "Code")
        .and_then(Value::as_str)
        .and_then(normalize);
    let Some(code) = code else {
        return refused(StatusCode::BAD_REQUEST, "invalid_code");
    };
    let (account, hash, now) = (holder.account.clone(), code_hash(&code), now_ms());
    let actor = ActorContext::new(Actor::Player {
        account: holder.account.global().clone(),
    });
    let written = store
        .account_write(&holder.account, &actor, move |tx, repos| {
            // The import runs only before the first character exists
            if !repos.characters.find_many_by_account(tx.conn(), &account)?.is_empty() {
                return Ok(None);
            }
            legacy::redeem_claim(tx, &hash, &account, now).map(Some)
        })
        .await;
    match written {
        Ok(Some(Redeem::Done(_))) => util::ok(&object(vec![("Status", Value::from("claimed"))])),
        Ok(Some(Redeem::Unknown)) => refused(StatusCode::NOT_FOUND, "unknown_code"),
        Ok(Some(Redeem::Used)) => refused(StatusCode::CONFLICT, "code_used"),
        Ok(Some(Redeem::AccountHasOne)) => refused(StatusCode::CONFLICT, "account_has_save"),
        Ok(Some(Redeem::SaveTaken)) => refused(StatusCode::CONFLICT, "save_taken"),
        Ok(None) => refused(StatusCode::CONFLICT, "already_played"),
        Err(e) => {
            tracing::error!("[LegacyClaim] redeem failed: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// `POST /admin/v1/legacy/claims` `{"sourceUser", "reason"}`: a new code for that old account, shown only here.
pub async fn create(ctx: AdminContext) -> Response {
    let Some(caller) = ctx.caller.clone() else {
        return admin_internal("the claim had no caller");
    };
    let source_user = ctx
        .request
        .field("sourceUser")
        .and_then(Value::as_str)
        .map(util::trim)
        .unwrap_or_default()
        .to_owned();
    if source_user.is_empty() || source_user.chars().count() > MAX_SOURCE_USER {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_LEGACY_SOURCE_USER",
            "The old account's user id is required.",
        );
    }
    let reason = ctx
        .request
        .field("reason")
        .and_then(Value::as_str)
        .map(|r| util::take_chars(util::trim(r), 500))
        .unwrap_or_default();
    if reason.chars().count() < 3 {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_LEGACY_REASON",
            "A reason of at least 3 characters is required.",
        );
    }
    let code = new_code();
    let Some(normalized) = normalize(&code) else {
        return admin_internal("a new claim code did not read back");
    };
    let now = now_ms();
    let request_id = fresh_id();
    let audit = AdminAuditRecord {
        id: fresh_id(),
        actor_user_id: caller.user_id.clone(),
        target_user_id: None,
        action: "legacy.claim.create".to_owned(),
        old_state: None,
        new_state: Some(object(vec![
            ("source", Value::from(RAMSGATE_REBORN)),
            ("sourceUser", Value::from(source_user.clone())),
        ])),
        reason: Some(reason),
        ip: ctx.request.client_ip.clone(),
        request_id: request_id.clone(),
        created_at: iso(now),
    };
    let (hash, actor, old) = (code_hash(&normalized), caller.user_id.clone(), source_user.clone());
    let written = ctx
        .store
        .write(&admin_actor(&caller), move |tx, repos| {
            if let Some(account) = legacy::taken_by(tx.conn(), RAMSGATE_REBORN, &old)? {
                return Ok(Some(account));
            }
            legacy::prune_claims(tx, now)?;
            legacy::create_claim(tx, &hash, RAMSGATE_REBORN, &old, &actor, now)?;
            repos.admin.append_audit(tx, &audit)?;
            Ok(None)
        })
        .await;
    match written {
        Ok(None) => {
            tracing::info!(
                "[AdminLegacy] claim code made for {source_user} actor={}",
                caller.user_id
            );
            util::ok(&object(vec![
                ("requestId", Value::from(request_id)),
                ("code", Value::from(code)),
                ("expiresAt", Value::from(iso(now.saturating_add(CLAIM_KEEP_MS)))),
            ]))
        }
        Ok(Some(_)) => admin_error(
            StatusCode::CONFLICT,
            "ADMIN_LEGACY_SAVE_TAKEN",
            "That old account's save is already in an account.",
        ),
        Err(e) => admin_internal(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_read_back_however_they_are_typed() {
        for _ in 0..50 {
            let code = new_code();
            assert_eq!(code.len(), 23);
            let normalized = normalize(&code).unwrap();
            assert_eq!(
                normalize(&code.to_lowercase().replace('-', " ")).as_deref(),
                Some(&*normalized)
            );
        }
        assert_eq!(
            normalize("oiL00-00000-00000-0000a").as_deref(),
            Some("0110000000000000000A")
        );
        for bad in [
            "",
            "ABCDE",
            "ABCDE-ABCDE-ABCDE-ABCDEF",
            "ABCDE-ABCDE-ABCDE-ABCDU",
            "ÄBCDE-ABCDE-ABCDE-ABCDE",
        ] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
        assert_ne!(code_hash("0110000000000000000A"), code_hash("0110000000000000000B"));
    }
}
