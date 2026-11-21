// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `export-mongo` (RB13's one-release rollback; spec §11): the store's records back into Node's collections.
//! Times go out as strings and `ttlAt` as numbers (§17 item 9); fidelity to Node's own types is not claimed.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Map, Value, json};

use crate::Area;
use crate::tables::{Doc, TargetState};
use crate::verify::{is_seed_policy, sha256_hex};

/// MongoDB documents by collection name, in the order export-mongo writes them.
pub type Collections = BTreeMap<String, Vec<Map<String, Value>>>;

/// One document under construction: keys in the order they are added, `?` keys skipped when null.
struct D(Doc);

impl D {
    fn with_id(id: impl Into<Value>) -> Self {
        let mut doc = Doc::new();
        doc.insert("_id".into(), id.into());
        Self(doc)
    }

    fn v(mut self, key: &str, value: Value) -> Self {
        self.0.insert(key.into(), value);
        self
    }

    fn opt(self, key: &str, value: Value) -> Self {
        if value.is_null() { self } else { self.v(key, value) }
    }
}

fn get(row: &Doc, field: &str) -> Value {
    row.get(field).cloned().unwrap_or(Value::Null)
}

fn text(row: &Doc, field: &str) -> String {
    row.get(field).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// A JSON text column as a value; text that does not parse is the default.
fn parsed(row: &Doc, field: &str, default: Value) -> Value {
    row.get(field)
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(default)
}

fn parsed_opt(row: &Doc, field: &str, default: Value) -> Value {
    if get(row, field).is_null() {
        Value::Null
    } else {
        parsed(row, field, default)
    }
}

struct Users(HashMap<String, String>);

impl Users {
    /// The user id of a live account, or the account's own text when none has it (P9).
    fn of(&self, account: &str) -> String {
        self.0.get(account).cloned().unwrap_or_else(|| account.to_owned())
    }

    fn of_field(&self, row: &Doc, field: &str) -> Value {
        match row.get(field).and_then(Value::as_str) {
            Some(account) => Value::String(self.of(account)),
            None => Value::Null,
        }
    }
}

/// The documents of every collection of `areas`, each produced even when empty. `state` is the store read
/// back (§10.2); its untouched seed policy is not exported.
pub fn state_to_mongo_collections(state: &TargetState, areas: &BTreeSet<Area>) -> Collections {
    let users = Users(
        state
            .rows("accounts")
            .map(|(_, r)| (text(r, "id"), text(r, "user_id")))
            .collect(),
    );
    let mut out = Collections::new();
    let mut add = |name: &str, docs: Vec<Doc>| {
        out.insert(name.into(), docs);
    };
    let rows = |table: &str| state.rows(table).map(|(_, r)| r).collect::<Vec<_>>();
    let has = |a: Area| areas.contains(&a);
    if has(Area::Auth) {
        add("accounts", accounts(state));
        add(
            "authIdentities",
            rows("auth_identities")
                .into_iter()
                .map(|r| {
                    D::with_id(format!("{}:{}", text(r, "provider"), text(r, "provider_subject")))
                        .v("userId", users.of_field(r, "account_id"))
                        .v("provider", get(r, "provider"))
                        .v("providerSubject", get(r, "provider_subject"))
                        .v("providerUsername", get(r, "provider_username"))
                        .opt("providerAvatarUrl", get(r, "provider_avatar_url"))
                        .v("linkedAt", get(r, "linked_at"))
                        .0
                })
                .collect(),
        );
        add(
            "refreshSessions",
            rows("refresh_sessions")
                .into_iter()
                .map(|r| {
                    D::with_id(get(r, "id"))
                        .v("sessionId", get(r, "id"))
                        .v("userId", users.of_field(r, "account_id"))
                        .v("tokenHash", get(r, "token_hash"))
                        .v("familyId", get(r, "family_id"))
                        .v("deviceId", get(r, "device_id"))
                        .v("deviceName", get(r, "device_name"))
                        .v("createdAt", get(r, "created_at"))
                        .v("expiresAt", get(r, "expires_at"))
                        .opt("revokedAt", get(r, "revoked_at"))
                        .0
                })
                .collect(),
        );
        add(
            "gameExchangeCodes",
            rows("game_exchange_codes")
                .into_iter()
                .map(|r| {
                    D::with_id(get(r, "code_hash"))
                        .v("codeHash", get(r, "code_hash"))
                        .v("userId", users.of_field(r, "account_id"))
                        .v("launcherSessionId", get(r, "launcher_session_id"))
                        .v("buildChangelist", get(r, "build_changelist"))
                        .v("executableSha256", get(r, "executable_sha256"))
                        .v("runtimeChannel", get(r, "runtime_channel"))
                        .v("runtimeSha256", get(r, "runtime_sha256"))
                        .v("runtimeManifestVersion", get(r, "runtime_manifest_version"))
                        .v("runtimeArtifactSetSha256", get(r, "runtime_artifact_set_sha256"))
                        .opt("realmId", get(r, "realm_id"))
                        .v("createdAt", get(r, "created_at"))
                        .v("expiresAt", get(r, "expires_at"))
                        .opt("consumedAt", get(r, "consumed_at"))
                        .0
                })
                .collect(),
        );
        add(
            "discordOAuthTransactions",
            rows("discord_oauth_transactions")
                .into_iter()
                .map(|r| {
                    D::with_id(get(r, "state"))
                        .v("state", get(r, "state"))
                        .v("codeVerifier", get(r, "code_verifier"))
                        .v("createdAt", get(r, "created_at"))
                        .v("expiresAt", get(r, "expires_at"))
                        .opt("consumedAt", get(r, "consumed_at"))
                        .opt("completionCodeHash", get(r, "completion_code_hash"))
                        .opt("completionExpiresAt", get(r, "completion_expires_at"))
                        .opt("completionConsumedAt", get(r, "completion_consumed_at"))
                        .opt("userId", users.of_field(r, "account_id"))
                        .opt("linkUserId", users.of_field(r, "link_account_id"))
                        .0
                })
                .collect(),
        );
        add(
            "emailCodes",
            rows("email_codes")
                .into_iter()
                .map(|r| {
                    let user = users.of(&text(r, "account_id"));
                    // The store's purposes, not mapped back (P14)
                    D::with_id(format!("{user}:{}", text(r, "purpose")))
                        .v("userId", Value::String(user))
                        .v("purpose", get(r, "purpose"))
                        .v("codeHash", get(r, "code_hash"))
                        .v("attempts", get(r, "attempts"))
                        .v("createdAt", get(r, "created_at"))
                        .v("expiresAt", get(r, "expires_at"))
                        .0
                })
                .collect(),
        );
        add(
            "gameServerApiKeys",
            rows("game_server_api_keys")
                .into_iter()
                .map(|r| D::with_id(get(r, "key_hash")).v("keyHash", get(r, "key_hash")).0)
                .collect(),
        );
        add(
            "userApiKeys",
            rows("user_api_keys")
                .into_iter()
                .map(|r| {
                    let user = users.of(&text(r, "account_id"));
                    D::with_id(user.clone())
                        .v("userId", Value::String(user))
                        .v("keyHash", get(r, "key_hash"))
                        .0
                })
                .collect(),
        );
    }
    if has(Area::Characters) {
        add(
            "characters",
            rows("characters")
                .into_iter()
                .map(|r| {
                    D::with_id(get(r, "character_id"))
                        .v("characterId", get(r, "character_id"))
                        .v("userId", users.of_field(r, "account_id"))
                        .v("ord", get(r, "ord"))
                        .v("createdDate", get(r, "created_date"))
                        .v("lastModifiedDate", get(r, "last_modified_date"))
                        .v("name", get(r, "name"))
                        .v("updateVersion", get(r, "update_version"))
                        .v("data", parsed(r, "data", json!({})))
                        .0
                })
                .collect(),
        );
        add(
            "loadouts",
            rows("loadouts")
                .into_iter()
                .map(|r| {
                    D::with_id(get(r, "character_id"))
                        .v("characterId", get(r, "character_id"))
                        .v("userId", users.of_field(r, "account_id"))
                        .v("loadouts", parsed(r, "slots_json", json!([])))
                        .v("persistent", parsed(r, "persistent", json!({})))
                        .v("activeIndex", get(r, "active_index"))
                        .opt("unlockedTotalSlots", get(r, "unlocked_total_slots"))
                        .v("revision", get(r, "revision"))
                        .opt("bootstrapVersion", get(r, "bootstrap_version"))
                        .0
                })
                .collect(),
        );
    }
    // Characters alone replaces the whole of playerJourney too (parity, §17 item 10)
    if has(Area::Progression) || has(Area::Characters) {
        add(
            "playerJourney",
            rows("player_journey")
                .into_iter()
                .map(|r| {
                    let user = users.of(&text(r, "account"));
                    D::with_id(user.clone())
                        .v("userId", Value::String(user))
                        .v("nodes", parsed(r, "nodes", json!({})))
                        .v("updateVersion", get(r, "update_version"))
                        .v("revision", get(r, "revision"))
                        .0
                })
                .collect(),
        );
    }
    if has(Area::Inventory) {
        let character_accounts: HashMap<String, String> = state
            .rows("characters")
            .map(|(_, r)| (text(r, "character_id"), text(r, "account_id")))
            .collect();
        add(
            "inventories",
            rows("inventories")
                .into_iter()
                .map(|r| {
                    let account = r
                        .get("account_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| character_accounts.get(&text(r, "character_id")).cloned());
                    D::with_id(get(r, "character_id"))
                        .v("characterId", get(r, "character_id"))
                        .opt("userId", account.map_or(Value::Null, |a| Value::String(users.of(&a))))
                        .v("instancedItems", parsed(r, "instanced_items", json!([])))
                        .v("stackedItems", parsed(r, "stacked_items", json!([])))
                        .v("revision", get(r, "revision"))
                        .opt("bootstrapVersion", get(r, "bootstrap_version"))
                        .0
                })
                .collect(),
        );
        add(
            "inventoryTransactions",
            rows("inventory_transactions")
                .into_iter()
                .map(|r| ledger(r, &users))
                .collect(),
        );
    }
    if has(Area::Progression) {
        let two = |r: &Doc, field: &str| format!("{}::{}", users.of(&text(r, "account")), text(r, field));
        add(
            "progressionTracks",
            rows("progression_tracks")
                .into_iter()
                .map(|r| {
                    D::with_id(two(r, "progression_id"))
                        .v("userId", users.of_field(r, "account"))
                        .v("progressionId", get(r, "progression_id"))
                        .v("progress", get(r, "progress"))
                        .v("confirmedFremiumRank", get(r, "confirmed_fremium_rank"))
                        .v("confirmedPremiumRank", get(r, "confirmed_premium_rank"))
                        .v("updateVersion", get(r, "update_version"))
                        .v("createdAt", get(r, "created_at"))
                        .v("updatedAt", get(r, "updated_at"))
                        .opt("reforge", parsed_opt(r, "reforge", json!({})))
                        .opt("prestigeClaimed", get(r, "prestige_claimed"))
                        .0
                })
                .collect(),
        );
        add(
            "progressionObjectives",
            rows("progression_objectives")
                .into_iter()
                .map(|r| {
                    D::with_id(two(r, "objective_id"))
                        .v("userId", users.of_field(r, "account"))
                        .v("objectiveId", get(r, "objective_id"))
                        .v("value", get(r, "value"))
                        .v("completedCount", get(r, "completed_count"))
                        .v("createdAt", get(r, "created_at"))
                        .v("updatedAt", get(r, "updated_at"))
                        .0
                })
                .collect(),
        );
        add(
            "progressionObjectiveEvents",
            rows("progression_objective_events")
                .into_iter()
                .map(|r| {
                    D::with_id(get(r, "id"))
                        .v("id", get(r, "id"))
                        .v("userId", users.of_field(r, "account"))
                        .v("rawBody", parsed(r, "raw_body", json!({})))
                        .v("receivedAt", get(r, "received_at"))
                        .0
                })
                .collect(),
        );
        add(
            "progressionTransactions",
            rows("progression_grants")
                .into_iter()
                .map(|r| {
                    let user = users.of(&text(r, "account"));
                    let mut result = parsed(r, "result", json!({}));
                    if let Value::Object(o) = &mut result {
                        o.insert("userId".into(), Value::String(user.clone()));
                    }
                    D::with_id(get(r, "grant_id"))
                        .v("grantId", get(r, "grant_id"))
                        .v("userId", Value::String(user))
                        .v("progressionId", get(r, "progression_id"))
                        .v("amount", get(r, "amount"))
                        .v("requestHash", get(r, "request_hash"))
                        .v("status", get(r, "status"))
                        .v("result", result)
                        .v("createdAt", get(r, "created_at"))
                        .0
                })
                .collect(),
        );
        add(
            "escalationProgress",
            rows("escalation_progress")
                .into_iter()
                .map(|r| {
                    D::with_id(two(r, "season_id"))
                        .v("userId", users.of_field(r, "account"))
                        .v("seasonId", get(r, "season_id"))
                        .v("escalationLevel", get(r, "escalation_level"))
                        .v("nextLevelXp", get(r, "next_level_xp"))
                        .v("talentsProgress", parsed(r, "talents_progress", json!([])))
                        .v("unlockProgress", parsed(r, "unlock_progress", json!([])))
                        .v("updateVersion", get(r, "update_version"))
                        .v("createdAt", get(r, "created_at"))
                        .v("updatedAt", get(r, "updated_at"))
                        .0
                })
                .collect(),
        );
        add(
            "breadcrumbs",
            rows("breadcrumbs")
                .into_iter()
                .map(|r| {
                    D::with_id(get(r, "character_id"))
                        .v("characterId", get(r, "character_id"))
                        .v("userId", users.of_field(r, "account"))
                        .v("breadcrumbs", parsed(r, "items", json!([])))
                        .v("updateVersion", get(r, "update_version"))
                        .0
                })
                .collect(),
        );
        add(
            "encounteredContent",
            rows("encountered_content")
                .into_iter()
                .map(|r| {
                    D::with_id(get(r, "character_id"))
                        .v("characterId", get(r, "character_id"))
                        .v("userId", users.of_field(r, "account"))
                        .v("encounteredcontent", parsed(r, "content", json!([])))
                        .0
                })
                .collect(),
        );
    }
    if has(Area::Bounties) {
        bounties(state, &users, &mut add);
    }
    if has(Area::Parties) {
        parties(state, &users, &mut add);
    }
    if has(Area::Reforged) {
        reforged(state, &users, &mut add);
    }
    out
}

fn accounts(state: &TargetState) -> Vec<Doc> {
    state
        .rows("accounts")
        .map(|(key, r)| {
            let mut d = D::with_id(get(r, "user_id"))
                .v("userId", get(r, "user_id"))
                .v("name", get(r, "name"))
                .v("notes", get(r, "notes"));
            if let Some(l) = state.get("launcher_accounts", key) {
                // Flat names, never the nested consent, deletion or approval shapes
                d = d
                    .opt("email", get(l, "email"))
                    .v("displayName", get(l, "display_name"))
                    .v("displayNameNormalized", get(l, "display_name_normalized"))
                    .opt("passwordHash", get(l, "password_hash"))
                    .v("status", get(l, "status"))
                    .opt("approvalStatus", get(l, "approval_status"))
                    .opt("approvalUpdatedAt", get(l, "approval_updated_at"))
                    .opt("approvalUpdatedBy", get(l, "approval_updated_by"))
                    .opt("approvalReason", get(l, "approval_reason"))
                    .v("roles", parsed(l, "roles", json!(["user"])))
                    .opt("rolesUpdatedAt", get(l, "roles_updated_at"))
                    .opt("rolesUpdatedBy", get(l, "roles_updated_by"))
                    .v("createdAt", get(l, "created_at"))
                    .opt("lastLoginAt", get(l, "last_login_at"))
                    .opt("usernameSet", get(l, "username_set"))
                    .opt("consentTermsVersion", get(l, "consent_terms_version"))
                    .opt("consentPrivacyVersion", get(l, "consent_privacy_version"))
                    .opt("consentAcceptedAt", get(l, "consent_accepted_at"))
                    .opt("deletionRequestedAt", get(l, "deletion_requested_at"))
                    .opt("deletionScheduledFor", get(l, "deletion_scheduled_for"))
                    .opt("emailVerifiedAt", get(l, "email_verified_at"))
                    .opt(
                        "emailVerificationPendingSince",
                        get(l, "email_verification_pending_since"),
                    );
            }
            d.0
        })
        .collect()
}

/// `_id` is the SHA-256 of `[uid, character, transaction]`; the extras come first, the core keys override.
fn ledger(r: &Doc, users: &Users) -> Doc {
    let user = users.of(&text(r, "account_id"));
    let id = sha256_hex(&json!([user, text(r, "character_id"), text(r, "transaction_id")]).to_string());
    let mut doc = match r.get("extra").and_then(Value::as_str) {
        None => Doc::new(),
        Some(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Object(extra)) => extra,
            _ => {
                let mut d = Doc::new();
                d.insert("extra".into(), Value::String(s.to_owned()));
                d
            }
        },
    };
    doc.insert("_id".into(), Value::String(id));
    D(doc)
        .v("userId", Value::String(user))
        .v("characterId", get(r, "character_id"))
        .v("transactionId", get(r, "transaction_id"))
        .opt("requestHash", get(r, "request_hash"))
        .v("status", get(r, "status"))
        .opt("result", parsed_opt(r, "result", json!({})))
        .v("createdAt", get(r, "created_at"))
        .opt("completedAt", get(r, "completed_at"))
        .opt("economySource", get(r, "economy_source"))
        .opt("economySessionId", get(r, "economy_session_id"))
        .0
}

fn bounties(state: &TargetState, users: &Users, add: &mut impl FnMut(&str, Vec<Doc>)) {
    let rows = |table: &str| state.rows(table).map(|(_, r)| r).collect::<Vec<_>>();
    add(
        "bountyStates",
        rows("bounty_states")
            .into_iter()
            .map(|r| {
                let user = users.of(&text(r, "account_id"));
                D::with_id(user.clone())
                    .v("userId", Value::String(user))
                    .v("bounties", parsed(r, "bounties", json!([])))
                    .v("draftData", parsed(r, "draft_data", json!({})))
                    .opt("dailyRewardCreditedOn", get(r, "daily_reward_credited_on"))
                    .opt("dailyPremiumDraftGrantedOn", get(r, "daily_premium_draft_granted_on"))
                    .v("updateVersion", get(r, "update_version"))
                    .v("createdAt", get(r, "created_at"))
                    .v("updatedAt", get(r, "updated_at"))
                    .0
            })
            .collect(),
    );
    add(
        "cooldownStates",
        rows("cooldown_states")
            .into_iter()
            .map(|r| {
                let user = users.of(&text(r, "account_id"));
                D::with_id(user.clone())
                    .v("userId", Value::String(user))
                    .v("entries", parsed(r, "entries", json!({})))
                    .v("updateVersion", get(r, "update_version"))
                    .v("createdAt", get(r, "created_at"))
                    .v("updatedAt", get(r, "updated_at"))
                    .0
            })
            .collect(),
    );
    // One document per account, entries in name order: the rows' key order
    let mut by_account: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for r in rows("entitlements") {
        let entry = D(Doc::new())
            .v("name", get(r, "name"))
            .v("duration", get(r, "duration"))
            .opt("activatedDate", get(r, "activated_date"))
            .opt("grantedAt", get(r, "granted_at"))
            .opt("sourceSkuId", get(r, "source_sku_id"))
            .0;
        by_account
            .entry(text(r, "account_id"))
            .or_default()
            .push(Value::Object(entry));
    }
    add(
        "entitlements",
        by_account
            .into_iter()
            .map(|(account, entries)| {
                let user = users.of(&account);
                D::with_id(user.clone())
                    .v("userId", Value::String(user))
                    .v("entitlements", Value::Array(entries))
                    .0
            })
            .collect(),
    );
    add(
        "wallets",
        rows("wallets")
            .into_iter()
            .map(|r| {
                let user = users.of(&text(r, "account_id"));
                let mut balances = Map::new();
                for entry in r.get("balances").and_then(Value::as_array).into_iter().flatten() {
                    if let Some([catalog, amount, _]) = entry.as_array().map(Vec::as_slice) {
                        balances.insert(catalog.as_str().unwrap_or_default().to_owned(), amount.clone());
                    }
                }
                D::with_id(user.clone())
                    .v("userId", Value::String(user))
                    .opt("bootstrapVersion", get(r, "bootstrap_version"))
                    .v("balances", Value::Object(balances))
                    .0
            })
            .collect(),
    );
    let mut daily: BTreeMap<(String, String, String), Vec<&Doc>> = BTreeMap::new();
    for r in rows("economy_rollup_counters") {
        daily
            .entry((text(r, "account_id"), text(r, "day"), text(r, "currency")))
            .or_default()
            .push(r);
    }
    add(
        "economyDaily",
        daily
            .into_iter()
            .map(|((account, day, currency), counters)| {
                let user = users.of(&account);
                let mut doc = D::with_id(format!("{user}:{day}:{currency}"))
                    .v("userId", Value::String(user))
                    .v("day", Value::String(day))
                    .v("currency", Value::String(currency))
                    .v("ttlAt", counters.first().map_or(json!(0), |c| get(c, "ttl_at")))
                    .0;
                for counter in counters {
                    let segments: Vec<&str> = counter
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .split('.')
                        .filter(|s| !s.is_empty())
                        .collect();
                    nest(&mut doc, &segments, get(counter, "value"));
                }
                doc
            })
            .collect(),
    );
}

/// The counter at `path` as nested objects; a later path through a number replaces it with an object.
fn nest(doc: &mut Doc, path: &[&str], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut at = doc;
    for segment in parents {
        let slot = at.entry(*segment).or_insert_with(|| Value::Object(Map::new()));
        if !slot.is_object() {
            *slot = Value::Object(Map::new());
        }
        let Value::Object(next) = slot else { return };
        at = next;
    }
    at.insert((*last).to_owned(), value);
}

fn parties(state: &TargetState, users: &Users, add: &mut impl FnMut(&str, Vec<Doc>)) {
    let rows = |table: &str| state.rows(table).map(|(_, r)| r).collect::<Vec<_>>();
    let mut memberships = Vec::new();
    let mut client_ids: HashMap<String, String> = HashMap::new();
    let parties: Vec<Doc> = rows("parties")
        .into_iter()
        .map(|r| {
            client_ids.insert(text(r, "id"), text(r, "client_id"));
            let mut members = Vec::new();
            for entry in r.get("members").and_then(Value::as_array).into_iter().flatten() {
                let Some([account, _, joined]) = entry.as_array().map(Vec::as_slice) else {
                    continue;
                };
                let user = users.of(account.as_str().unwrap_or_default());
                members.push(Value::String(user.clone()));
                memberships.push(
                    D::with_id(user.clone())
                        .v("partyId", get(r, "client_id"))
                        .v("userId", Value::String(user))
                        .v("joinedAt", joined.clone())
                        .0,
                );
            }
            D::with_id(get(r, "client_id"))
                .v("partyId", get(r, "client_id"))
                .v("revision", get(r, "revision"))
                .v("leaderPlayerId", users.of_field(r, "leader"))
                .v("buildId", get(r, "build_id"))
                .v("activity", get(r, "activity"))
                .opt("activeHuntSessionId", get(r, "active_hunt_session_id"))
                .v("createdAt", get(r, "created_at"))
                .v("updatedAt", get(r, "updated_at"))
                .v("members", Value::Array(members))
                .0
        })
        .collect();
    add("parties", parties);
    add("partyMemberships", memberships);
    add(
        "partyInvites",
        rows("party_invites")
            .into_iter()
            .map(|r| {
                let (recipient, sender) = (users.of(&text(r, "recipient")), users.of(&text(r, "sender")));
                let party = text(r, "party");
                D::with_id(format!("{recipient}\u{0}{sender}"))
                    .v("recipientPlayerId", Value::String(recipient))
                    .v("sendingPlayerId", Value::String(sender))
                    .v(
                        "partyId",
                        Value::String(client_ids.get(&party).cloned().unwrap_or(party)),
                    )
                    .v("partyRevision", get(r, "party_revision"))
                    .v("sendingDisplayName", get(r, "sending_display_name"))
                    .v("sendingPlatform", get(r, "sending_platform"))
                    .v("status", get(r, "status"))
                    .v("createdAt", get(r, "created_at"))
                    .v("expiresAt", get(r, "expires_at"))
                    .opt("consumedAt", get(r, "consumed_at"))
                    .0
            })
            .collect(),
    );
    add(
        "partyOutbox",
        rows("party_outbox")
            .into_iter()
            .map(|r| {
                D::with_id(get(r, "id"))
                    .v("eventId", get(r, "id"))
                    .v("partyId", get(r, "party"))
                    .v("partyRevision", get(r, "party_revision"))
                    .v("eventType", get(r, "event_type"))
                    .v("recipientAccountIds", parsed(r, "recipients", json!([])))
                    .v("payload", parsed(r, "payload", json!({})))
                    .v("state", get(r, "state"))
                    .v("createdAt", get(r, "created_at"))
                    .opt("claimedAt", get(r, "claimed_at"))
                    .opt("claimOwner", get(r, "claim_owner"))
                    .opt("publishedAt", get(r, "published_at"))
                    .0
            })
            .collect(),
    );
    add(
        "friendships",
        rows("friend_edges")
            .into_iter()
            .map(|r| {
                let owner = users.of(&text(r, "owner"));
                D::with_id(format!("{owner}:{}", text(r, "other_user_id")))
                    .v("ownerId", Value::String(owner))
                    .v("otherId", get(r, "other_user_id"))
                    .v("status", get(r, "status"))
                    .opt("direction", get(r, "direction"))
                    .v("created", get(r, "created"))
                    .v("favorite", get(r, "favorite"))
                    .0
            })
            .collect(),
    );
}

fn reforged(state: &TargetState, users: &Users, add: &mut impl FnMut(&str, Vec<Doc>)) {
    let rows = |table: &str| state.rows(table).map(|(_, r)| r).collect::<Vec<_>>();
    add(
        "reforgedInvites",
        rows("reforged_invites")
            .into_iter()
            .map(|r| {
                D::with_id(get(r, "code"))
                    .v("invitecode", get(r, "code"))
                    .v("usesRemaining", get(r, "uses_remaining"))
                    .v("infiniteUses", get(r, "infinite_uses"))
                    .opt("note", get(r, "note"))
                    .v("createdAt", get(r, "created_at"))
                    .opt("expiresAt", get(r, "expires_at"))
                    .0
            })
            .collect(),
    );
    add(
        "adminSessions",
        rows("admin_sessions")
            .into_iter()
            .map(|r| {
                D::with_id(get(r, "id"))
                    .v("id", get(r, "id"))
                    .v("userId", users.of_field(r, "account"))
                    .v("tokenHash", get(r, "token_hash"))
                    .v("csrfToken", get(r, "csrf_token"))
                    .v("createdAt", get(r, "created_at"))
                    .v("expiresAt", get(r, "expires_at"))
                    .v("ip", get(r, "ip"))
                    .v("userAgent", get(r, "user_agent"))
                    .opt("revokedAt", get(r, "revoked_at"))
                    .v("ttlAt", get(r, "ttl_at"))
                    .0
            })
            .collect(),
    );
    add(
        "adminAudit",
        rows("admin_audit")
            .into_iter()
            .map(|r| {
                D::with_id(get(r, "id"))
                    .v("id", get(r, "id"))
                    .v("actorUserId", get(r, "actor_user_id"))
                    .opt("targetUserId", get(r, "target_user_id"))
                    .v("action", get(r, "action"))
                    .opt("oldState", parsed_opt(r, "old_state", json!({})))
                    .opt("newState", parsed_opt(r, "new_state", json!({})))
                    .opt("reason", get(r, "reason"))
                    .v("ip", get(r, "ip"))
                    .v("requestId", get(r, "request_id"))
                    .v("createdAt", get(r, "created_at"))
                    .0
            })
            .collect(),
    );
    add(
        "launcherNotices",
        rows("launcher_notices")
            .into_iter()
            .map(|r| {
                D::with_id("current")
                    .v("version", get(r, "version"))
                    .opt("updatedAt", get(r, "updated_at"))
                    .opt("updatedBy", get(r, "updated_by"))
                    .v("announcements", parsed(r, "announcements", json!([])))
                    .v("tags", parsed(r, "tags", json!([])))
                    .0
            })
            .collect(),
    );
    add(
        "operationsPolicy",
        rows("operations_policy")
            .into_iter()
            .filter(|r| !is_seed_policy(r))
            .map(|r| {
                D::with_id("global")
                    .v("guardEnforcement", get(r, "guard_enforcement"))
                    .v("diagnosticsProfile", get(r, "diagnostics_profile"))
                    .v("version", get(r, "version"))
                    .opt("updatedAt", get(r, "updated_at"))
                    .opt("updatedBy", get(r, "updated_by"))
                    .opt("diagnosticsExpiresAt", get(r, "diagnostics_expires_at"))
                    .0
            })
            .collect(),
    );
    add(
        "launcherGuardSessions",
        rows("launcher_guard_sessions")
            .into_iter()
            .map(|r| {
                D::with_id(get(r, "guard_session_id"))
                    .v("guardSessionId", get(r, "guard_session_id"))
                    .v("userId", users.of_field(r, "account"))
                    .v("role", get(r, "role"))
                    .v("publicKeyBase64", get(r, "public_key_base64"))
                    .v("manifestId", get(r, "manifest_id"))
                    .v("manifestSequence", get(r, "manifest_sequence"))
                    .v("channel", get(r, "channel"))
                    .v("challenge", get(r, "challenge"))
                    .v("lastSequence", get(r, "last_sequence"))
                    .v("status", get(r, "status"))
                    .v("riskScore", get(r, "risk_score"))
                    .v("createdAt", get(r, "created_at"))
                    .v("expiresAt", get(r, "expires_at"))
                    .opt("lastHeartbeatAt", get(r, "last_heartbeat_at"))
                    .v("detail", parsed(r, "detail", json!({})))
                    .v("ttlAt", get(r, "ttl_at"))
                    .0
            })
            .collect(),
    );
    // No `_id`: MongoDB assigns an ObjectId
    add(
        "guardFindings",
        rows("guard_findings")
            .into_iter()
            .map(|r| {
                let ids: Vec<Value> = match parsed(r, "account_ids", json!([])) {
                    Value::Array(ids) => ids
                        .iter()
                        .map(|id| Value::String(users.of(id.as_str().unwrap_or_default())))
                        .collect(),
                    _ => Vec::new(),
                };
                D(Doc::new())
                    .v("fileName", get(r, "file_name"))
                    .v("sha256", get(r, "sha256"))
                    .v("signer", get(r, "signer"))
                    .v("size", get(r, "size"))
                    .v("firstSeenAt", get(r, "first_seen_at"))
                    .v("lastSeenAt", get(r, "last_seen_at"))
                    .v("sessionCount", get(r, "session_count"))
                    .v("accountCount", get(r, "account_count"))
                    .v("accountIds", Value::Array(ids))
                    .v("ttlAt", get(r, "ttl_at"))
                    .0
            })
            .collect(),
    );
}
