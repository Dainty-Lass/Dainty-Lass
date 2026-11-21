// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `migrate`'s extraction (spec §7): the source's collections, in §7.2's order, into the rows of every table,
//! with the RB11 repairs of §8 on the way. Verify runs the same extraction for its expected side.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rustic_isles_economy::inventory::storage_policy::{StoragePolicy, migrate_inventory_storage_arrays};
use serde_json::{Value, json};

use crate::ids::Resolver;
use crate::mongo::MongoClient;
use crate::read::{
    DEFAULT_TIME, Doc, bool_or, float_or, int_or, json_text_or, ms_or, pick, text, text_or, time, time_or, value_or,
};
use crate::repair::{LegacyStarter, LoadoutSide, reconcile_slots, repair_starter};
use crate::tables::{TargetState, float, table};
use crate::{Area, DbError};

/// Where the documents come from: MongoDB, or collections in memory (the fuzz target, the tests).
pub trait Source {
    fn collection(&mut self, name: &str) -> Result<Vec<Doc>, DbError>;
}

impl Source for MongoClient {
    fn collection(&mut self, name: &str) -> Result<Vec<Doc>, DbError> {
        Ok(self.find_all(name)?)
    }
}

impl Source for BTreeMap<String, Vec<Doc>> {
    fn collection(&mut self, name: &str) -> Result<Vec<Doc>, DbError> {
        Ok(self.get(name).cloned().unwrap_or_default())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counters {
    pub skipped_invalid: usize,
    pub approvals: usize,
    pub inventory_arrays: usize,
    pub starter_instances: usize,
    pub loadout_slots: usize,
}

pub(crate) struct Settings<'a> {
    pub areas: &'a BTreeSet<Area>,
    pub skip_invalid: bool,
    pub policy: Option<&'a StoragePolicy>,
    pub starter: &'a LegacyStarter,
}

enum DocErr {
    Invalid(&'static str),
    Fatal(DbError),
}

impl From<DbError> for DocErr {
    fn from(e: DbError) -> Self {
        Self::Fatal(e)
    }
}

type Step = Result<(), DocErr>;

fn need(value: Option<String>, detail: &'static str) -> Result<String, DocErr> {
    value.ok_or(DocErr::Invalid(detail))
}

/// The document's user id: `userId|accountId|_id`, text or none.
fn uid(d: &Doc) -> Option<String> {
    text(pick(d, &["userId", "accountId", "_id"]))
}

fn row(value: Value) -> Doc {
    match value {
        Value::Object(map) => map,
        _ => Doc::new(),
    }
}

/// `a` when the key is present at all (even null), else `b.c`.
fn either<'a>(d: &'a Doc, a: &str, b: &str, c: &str) -> Option<&'a Value> {
    if d.contains_key(a) {
        d.get(a)
    } else {
        d.get(b).and_then(|o| o.get(c))
    }
}

/// A string at `a`, else a string at `b.c`.
fn string_either<'a>(d: &'a Doc, a: &str, b: &str, c: &str) -> Option<&'a str> {
    match d.get(a) {
        Some(Value::String(s)) => Some(s),
        _ => d.get(b).and_then(|o| o.get(c)).and_then(Value::as_str),
    }
}

/// A column that is null when the source is, and JSON text otherwise.
fn json_or_null(v: Option<&Value>, default: &str) -> Value {
    match v {
        None | Some(Value::Null) => Value::Null,
        some => Value::String(json_text_or(some, default)),
    }
}

fn array(v: Value) -> Vec<Value> {
    match v {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

struct PendingInventory {
    user: String,
    account: Option<String>,
    instanced: Vec<Value>,
    stacked: Vec<Value>,
    revision: i64,
    bootstrap: Option<String>,
}

struct PendingLoadout {
    user: String,
    account: String,
    slots: Vec<Value>,
    persistent: Value,
    active_index: i64,
    persisted: Option<f64>,
    revision: i64,
    bootstrap: Option<String>,
}

pub(crate) struct Extraction {
    pub state: TargetState,
    pub counters: Counters,
}

struct Extractor<'a> {
    settings: &'a Settings<'a>,
    resolver: Resolver,
    state: TargetState,
    counters: Counters,
    journeys: HashMap<String, Value>,
    character_accounts: HashMap<String, String>,
    inventories: BTreeMap<String, PendingInventory>,
    loadouts: BTreeMap<String, PendingLoadout>,
    memberships: HashMap<(String, String), String>,
    ord: i64,
}

/// The whole extraction: every selected area's rows, the implicit accounts, and the RB11 counters.
pub(crate) fn extract(
    source: &mut dyn Source,
    settings: &Settings<'_>,
    resolver: Resolver,
) -> Result<Extraction, DbError> {
    let mut x = Extractor {
        settings,
        resolver,
        state: TargetState::default(),
        counters: Counters::default(),
        journeys: HashMap::new(),
        character_accounts: HashMap::new(),
        inventories: BTreeMap::new(),
        loadouts: BTreeMap::new(),
        memberships: HashMap::new(),
        ord: 0,
    };
    let has = |a: Area| settings.areas.contains(&a);
    if has(Area::Auth) {
        x.each(source, "accounts", Extractor::accounts)?;
        x.each(source, "authIdentities", Extractor::auth_identities)?;
        x.each(source, "refreshSessions", Extractor::refresh_sessions)?;
        x.each(source, "gameExchangeCodes", Extractor::game_exchange_codes)?;
        x.each(source, "discordOAuthTransactions", Extractor::discord)?;
        x.each(source, "emailCodes", Extractor::email_codes)?;
        x.each(source, "gameServerApiKeys", Extractor::game_server_keys)?;
        x.each(source, "userApiKeys", Extractor::user_keys)?;
    }
    if has(Area::Progression) || has(Area::Characters) {
        x.each(source, "playerJourney", Extractor::player_journey)?;
    }
    if has(Area::Progression) {
        x.each(source, "progressionTracks", Extractor::progression_tracks)?;
        x.each(source, "progressionObjectives", Extractor::progression_objectives)?;
        x.each(source, "progressionObjectiveEvents", Extractor::objective_events)?;
        x.each(source, "progressionTransactions", Extractor::progression_grants)?;
        x.each(source, "escalationProgress", Extractor::escalation)?;
        x.each(source, "breadcrumbs", Extractor::breadcrumbs)?;
        x.each(source, "encounteredContent", Extractor::encountered)?;
    }
    if has(Area::Characters) {
        x.each(source, "characters", Extractor::characters)?;
    }
    if has(Area::Characters) || has(Area::Inventory) {
        x.each(source, "inventories", Extractor::inventories)?;
    }
    if has(Area::Characters) {
        x.each(source, "loadouts", Extractor::loadouts)?;
    }
    x.repairs();
    if has(Area::Inventory) {
        x.each(source, "inventoryTransactions", Extractor::inventory_transactions)?;
    }
    if has(Area::Bounties) {
        x.each(source, "bountyStates", Extractor::bounty_states)?;
        x.each(source, "cooldownStates", Extractor::cooldown_states)?;
        x.each(source, "entitlements", Extractor::entitlements)?;
        x.each(source, "wallets", Extractor::wallets)?;
        x.each(source, "economyDaily", Extractor::economy_daily)?;
    }
    if has(Area::Parties) {
        x.each(source, "partyMemberships", Extractor::memberships)?;
        x.each(source, "parties", Extractor::parties)?;
        x.each(source, "partyInvites", Extractor::party_invites)?;
        x.each(source, "partyOutbox", Extractor::party_outbox)?;
        x.each(source, "friendships", Extractor::friendships)?;
    }
    if has(Area::Reforged) {
        x.each(source, "reforgedInvites", Extractor::reforged_invites)?;
        x.each(source, "adminSessions", Extractor::admin_sessions)?;
        x.each(source, "adminAudit", Extractor::admin_audit)?;
        x.first(source, "launcherNotices", Extractor::launcher_notices)?;
        x.first(source, "operationsPolicy", Extractor::operations_policy)?;
        x.each(source, "launcherGuardSessions", Extractor::guard_sessions)?;
        x.each(source, "guardFindings", Extractor::guard_findings)?;
    }
    Ok(Extraction {
        state: x.state,
        counters: x.counters,
    })
}

impl Extractor<'_> {
    fn has(&self, area: Area) -> bool {
        self.settings.areas.contains(&area)
    }

    fn each(
        &mut self,
        source: &mut dyn Source,
        collection: &'static str,
        mut f: impl FnMut(&mut Self, &Doc) -> Step,
    ) -> Result<(), DbError> {
        for doc in source.collection(collection)? {
            let step = f(self, &doc);
            self.handle(collection, step)?;
        }
        Ok(())
    }

    /// Notices and policy: the first document in natural order, the rest ignored (P17).
    fn first(
        &mut self,
        source: &mut dyn Source,
        collection: &'static str,
        f: impl FnOnce(&mut Self, &Doc) -> Step,
    ) -> Result<(), DbError> {
        if let Some(doc) = source.collection(collection)?.first() {
            let step = f(self, doc);
            self.handle(collection, step)?;
        }
        Ok(())
    }

    fn handle(&mut self, collection: &'static str, step: Step) -> Result<(), DbError> {
        match step {
            Ok(()) => Ok(()),
            Err(DocErr::Fatal(e)) => Err(e),
            Err(DocErr::Invalid(detail)) if self.settings.skip_invalid => {
                // Fixed texts only: never a document's contents
                tracing::warn!("skipping invalid document in {collection}: {detail}");
                self.counters.skipped_invalid += 1;
                Ok(())
            }
            Err(DocErr::Invalid(detail)) => Err(DbError::InvalidDocument {
                collection: collection.to_owned(),
                detail: detail.to_owned(),
            }),
        }
    }

    fn put(&mut self, name: &str, value: Value) {
        if let Some(t) = table(name) {
            self.state.put(t, row(value));
        }
    }

    /// The account of a user id, with a placeholder row unless one is already extracted (§6.2). Over an
    /// account the target already has, the placeholder carries its stored name and notes (it is insert-only).
    fn acct(&mut self, user: &str) -> Result<String, DocErr> {
        let id = self.resolver.account(user)?;
        let key = vec![id.clone()];
        if self.state.get("accounts", &key).is_none() {
            let trimmed = user.trim();
            let (name, notes) = self
                .resolver
                .existing
                .get(&id)
                .map_or((trimmed.to_owned(), 0), |e| (e.name.clone(), e.notes));
            self.put(
                "accounts",
                json!({"id": id, "user_id": trimmed, "name": name, "notes": notes}),
            );
            self.state.implicit.insert(id.clone());
        }
        Ok(id)
    }

    // ------------------------------------------------------------------ auth

    fn accounts(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId/_id")?;
        let id = self.acct(&user)?;
        let name = text(pick(d, &["name", "displayName"])).unwrap_or_else(|| user.clone());
        self.state.implicit.remove(&id);
        self.put(
            "accounts",
            json!({"id": id, "user_id": user, "name": name, "notes": int_or(d.get("notes"), 0)}),
        );
        let launcher = ["displayNameNormalized", "passwordHash", "roles", "approvalStatus"]
            .iter()
            .any(|k| d.contains_key(*k));
        if !launcher {
            return Ok(());
        }
        let display_name = text(d.get("displayName")).unwrap_or(name);
        let normalized = text(d.get("displayNameNormalized")).unwrap_or_else(|| display_name.to_ascii_lowercase());
        let created_at = time_or(d.get("createdAt"), DEFAULT_TIME);
        let (status, at, by, reason) = self.approval(d, &created_at);
        let username_set = d.contains_key("usernameSet").then(|| int_or(d.get("usernameSet"), 0));
        self.put(
            "launcher_accounts",
            json!({
                "account_id": id,
                "email": text(d.get("email")),
                "display_name_normalized": normalized,
                "display_name": display_name,
                "password_hash": text(d.get("passwordHash")),
                "status": text_or(d.get("status"), "active"),
                "approval_status": status,
                "approval_updated_at": at,
                "approval_updated_by": by,
                "approval_reason": reason,
                "roles": value_or(d.get("roles"), json!(["user"])).to_string(),
                "roles_updated_at": time(d.get("rolesUpdatedAt")),
                "roles_updated_by": text(d.get("rolesUpdatedBy")),
                "created_at": created_at,
                "last_login_at": time(d.get("lastLoginAt")),
                "username_set": username_set,
                "consent_terms_version": text(either(d, "consentTermsVersion", "consent", "termsVersion")),
                "consent_privacy_version": text(either(d, "consentPrivacyVersion", "consent", "privacyVersion")),
                "consent_accepted_at": time(either(d, "consentAcceptedAt", "consent", "acceptedAt")),
                "deletion_requested_at": time(either(d, "deletionRequestedAt", "deletion", "requestedAt")),
                "deletion_scheduled_for": time(either(d, "deletionScheduledFor", "deletion", "scheduledFor")),
                "email_verified_at": time(d.get("emailVerifiedAt")),
                "email_verification_pending_since": time(d.get("emailVerificationPendingSince")),
            }),
        );
        Ok(())
    }

    /// RB11 `migrate_launcher_approval` (§8.1): an account from before approvals is approved.
    fn approval(&mut self, d: &Doc, created_at: &str) -> (String, Option<String>, Option<String>, Option<String>) {
        let status = match d.get("approvalStatus") {
            Some(Value::String(s)) => Some(s.as_str()),
            _ => d.get("approval").and_then(|a| a.get("status")).and_then(Value::as_str),
        }
        .map(str::trim)
        .filter(|s| !s.is_empty());
        let Some(status) = status else {
            self.counters.approvals += 1;
            return (
                "approved".into(),
                Some(created_at.to_owned()),
                Some("migration:0.1.73".into()),
                None,
            );
        };
        let at = time(either(d, "approvalUpdatedAt", "approval", "updatedAt"));
        let non_empty = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_owned);
        let by = non_empty(string_either(d, "approvalUpdatedBy", "approval", "updatedBy"));
        let reason = non_empty(string_either(d, "approvalReason", "approval", "reason"));
        (status.to_owned(), at, by, reason)
    }

    fn auth_identities(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let provider = need(text(d.get("provider")), "missing provider")?;
        let subject = need(
            text(pick(d, &["providerSubject", "subject"])),
            "missing providerSubject",
        )?;
        let account = self.acct(&user)?;
        self.put(
            "auth_identities",
            json!({
                "provider": provider,
                "provider_subject": subject,
                "account_id": account,
                "provider_username": text_or(pick(d, &["providerUsername", "username"]), &subject),
                "provider_avatar_url": text(pick(d, &["providerAvatarUrl", "avatarUrl"])),
                "linked_at": time_or(pick(d, &["linkedAt", "createdAt"]), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn refresh_sessions(&mut self, d: &Doc) -> Step {
        let id = need(text(pick(d, &["sessionId", "id", "_id"])), "missing sessionId/_id")?;
        let user = need(uid(d), "missing userId")?;
        let token = need(text(d.get("tokenHash")), "missing tokenHash")?;
        let account = self.acct(&user)?;
        self.put(
            "refresh_sessions",
            json!({
                "id": id,
                "token_hash": token,
                "account_id": account,
                "family_id": text_or(d.get("familyId"), &id),
                "device_id": text_or(d.get("deviceId"), "unknown"),
                "device_name": text_or(d.get("deviceName"), "Launcher"),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "expires_at": time_or(d.get("expiresAt"), DEFAULT_TIME),
                "revoked_at": time(d.get("revokedAt")),
            }),
        );
        Ok(())
    }

    fn game_exchange_codes(&mut self, d: &Doc) -> Step {
        let hash = need(text(pick(d, &["codeHash", "_id"])), "missing codeHash")?;
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        let s = |k: &str| text_or(d.get(k), "");
        self.put(
            "game_exchange_codes",
            json!({
                "code_hash": hash,
                "account_id": account,
                "launcher_session_id": s("launcherSessionId"),
                "build_changelist": int_or(d.get("buildChangelist"), 0),
                "executable_sha256": s("executableSha256"),
                "runtime_channel": text_or(d.get("runtimeChannel"), "stable"),
                "runtime_sha256": s("runtimeSha256"),
                "runtime_manifest_version": s("runtimeManifestVersion"),
                "runtime_artifact_set_sha256": s("runtimeArtifactSetSha256"),
                "realm_id": text(d.get("realmId")),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "expires_at": time_or(d.get("expiresAt"), DEFAULT_TIME),
                "consumed_at": time(d.get("consumedAt")),
            }),
        );
        Ok(())
    }

    fn discord(&mut self, d: &Doc) -> Step {
        let state = need(text(pick(d, &["state", "_id"])), "missing state")?;
        let account = text(pick(d, &["userId", "accountId"]))
            .map(|u| self.acct(&u))
            .transpose()?;
        let link = text(pick(d, &["linkUserId", "linkAccountId"]))
            .map(|u| self.acct(&u))
            .transpose()?;
        self.put(
            "discord_oauth_transactions",
            json!({
                "state": state,
                "code_verifier": text_or(pick(d, &["codeVerifier", "pkceVerifier"]), ""),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "expires_at": time_or(d.get("expiresAt"), DEFAULT_TIME),
                "consumed_at": time(d.get("consumedAt")),
                "completion_code_hash": text(d.get("completionCodeHash")),
                "completion_expires_at": time(d.get("completionExpiresAt")),
                "completion_consumed_at": time(d.get("completionConsumedAt")),
                "account_id": account,
                "link_account_id": link,
            }),
        );
        Ok(())
    }

    fn email_codes(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let purpose = need(text(d.get("purpose")), "missing purpose")?;
        let hash = need(text(d.get("codeHash")), "missing codeHash")?;
        let account = self.acct(&user)?;
        let purpose = match purpose.as_str() {
            "verify_email" => "verify".to_owned(),
            "password_reset" => "reset".to_owned(),
            "discord_unlink" => "unlink".to_owned(),
            _ => purpose,
        };
        self.put(
            "email_codes",
            json!({
                "account_id": account,
                "purpose": purpose,
                "code_hash": hash,
                "attempts": int_or(d.get("attempts"), 0),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "expires_at": time_or(d.get("expiresAt"), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn game_server_keys(&mut self, d: &Doc) -> Step {
        let hash = need(text(pick(d, &["keyHash", "key", "_id"])), "missing keyHash")?;
        self.put("game_server_api_keys", json!({"key_hash": hash}));
        Ok(())
    }

    fn user_keys(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let hash = need(text(pick(d, &["keyHash", "key"])), "missing keyHash")?;
        let account = self.acct(&user)?;
        self.put("user_api_keys", json!({"account_id": account, "key_hash": hash}));
        Ok(())
    }

    // ------------------------------------------------------------------ progression

    fn player_journey(&mut self, d: &Doc) -> Step {
        let progression = self.has(Area::Progression);
        let Some(user) = uid(d) else {
            return if progression {
                Err(DocErr::Invalid("missing userId"))
            } else {
                Ok(())
            };
        };
        let account = self.acct(&user)?;
        let nodes = value_or(d.get("nodes"), json!({}));
        self.journeys.insert(account.clone(), nodes.clone());
        if progression {
            self.put(
                "player_journey",
                json!({
                    "account": account,
                    "nodes": nodes.to_string(),
                    "update_version": int_or(d.get("updateVersion"), 0),
                    "revision": int_or(d.get("revision"), 0),
                }),
            );
        }
        Ok(())
    }

    fn progression_tracks(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let progression = need(text(d.get("progressionId")), "missing progressionId")?;
        let account = self.acct(&user)?;
        let prestige = match d.get("prestigeClaimed") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::Bool(b)) => json!(i64::from(*b)),
            some => json!(int_or(some, 0)),
        };
        self.put(
            "progression_tracks",
            json!({
                "account": account,
                "progression_id": progression,
                "progress": int_or(d.get("progress"), 0),
                "confirmed_fremium_rank": int_or(d.get("confirmedFremiumRank"), 0),
                "confirmed_premium_rank": int_or(d.get("confirmedPremiumRank"), 0),
                "update_version": int_or(d.get("updateVersion"), 0),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "updated_at": time_or(d.get("updatedAt"), DEFAULT_TIME),
                "reforge": json_or_null(d.get("reforge"), "{}"),
                "prestige_claimed": prestige,
            }),
        );
        Ok(())
    }

    fn progression_objectives(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let objective = need(text(d.get("objectiveId")), "missing objectiveId")?;
        let account = self.acct(&user)?;
        self.put(
            "progression_objectives",
            json!({
                "account": account,
                "objective_id": objective,
                "value": int_or(d.get("value"), 0),
                "completed_count": int_or(d.get("completedCount"), 0),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "updated_at": time_or(d.get("updatedAt"), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn objective_events(&mut self, d: &Doc) -> Step {
        let id = need(text(pick(d, &["id", "_id"])), "missing _id")?;
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        self.put(
            "progression_objective_events",
            json!({
                "id": id,
                "account": account,
                "raw_body": json_text_or(pick(d, &["rawBody", "body"]), "{}"),
                "received_at": time_or(pick(d, &["receivedAt", "createdAt"]), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn progression_grants(&mut self, d: &Doc) -> Step {
        let grant = need(text(pick(d, &["grantId", "transactionId", "_id"])), "missing grantId")?;
        let user = need(uid(d), "missing userId")?;
        let progression = need(text(d.get("progressionId")), "missing progressionId")?;
        let account = self.acct(&user)?;
        // The result names the account by its global id (P8)
        let result = match d.get("result") {
            None => "{}".to_owned(),
            Some(Value::Object(o)) => {
                let mut o = o.clone();
                o.insert("userId".into(), Value::String(account.clone()));
                Value::Object(o).to_string()
            }
            some => json_text_or(some, "{}"),
        };
        self.put(
            "progression_grants",
            json!({
                "grant_id": grant,
                "account": account,
                "progression_id": progression,
                "amount": int_or(d.get("amount"), 0),
                "request_hash": text_or(d.get("requestHash"), ""),
                "status": text_or(d.get("status"), "applied"),
                "result": result,
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn escalation(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let season = need(text(d.get("seasonId")), "missing seasonId")?;
        let account = self.acct(&user)?;
        self.put(
            "escalation_progress",
            json!({
                "account": account,
                "season_id": season,
                "escalation_level": int_or(d.get("escalationLevel"), 0),
                "next_level_xp": int_or(d.get("nextLevelXp"), 0),
                "talents_progress": json_text_or(d.get("talentsProgress"), "[]"),
                "unlock_progress": json_text_or(d.get("unlockProgress"), "[]"),
                "update_version": int_or(d.get("updateVersion"), 0),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "updated_at": time_or(d.get("updatedAt"), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn breadcrumbs(&mut self, d: &Doc) -> Step {
        let character = need(text(pick(d, &["characterId", "_id"])), "missing characterId")?;
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        self.put(
            "breadcrumbs",
            json!({
                "character_id": character,
                "account": account,
                "items": json_text_or(pick(d, &["breadcrumbs", "items"]), "[]"),
                "update_version": int_or(d.get("updateVersion"), 0),
            }),
        );
        Ok(())
    }

    fn encountered(&mut self, d: &Doc) -> Step {
        let character = need(text(pick(d, &["characterId", "_id"])), "missing characterId")?;
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        self.put(
            "encountered_content",
            json!({
                "character_id": character,
                "account": account,
                // Node's own lower-case key
                "content": json_text_or(pick(d, &["encounteredcontent", "content"]), "[]"),
            }),
        );
        Ok(())
    }

    // ------------------------------------------------------------------ characters and inventory

    fn characters(&mut self, d: &Doc) -> Step {
        let character = need(text(pick(d, &["characterId", "_id"])), "missing characterId")?;
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        // Across all accounts (P7)
        let ord = int_or(d.get("ord"), self.ord);
        self.ord += 1;
        let created = time_or(pick(d, &["createdDate", "createdAt"]), DEFAULT_TIME);
        self.character_accounts.insert(character.clone(), account.clone());
        self.put(
            "characters",
            json!({
                "character_id": character,
                "account_id": account,
                "ord": ord,
                "created_date": created,
                "last_modified_date": time_or(pick(d, &["lastModifiedDate", "updatedAt"]), &created),
                "name": text_or(d.get("name"), &character),
                "update_version": int_or(d.get("updateVersion"), 0),
                "data": json_text_or(d.get("data"), "{}"),
            }),
        );
        Ok(())
    }

    fn inventories(&mut self, d: &Doc) -> Step {
        let Some(character) = text(pick(d, &["characterId", "_id"])) else {
            return if self.has(Area::Inventory) {
                Err(DocErr::Invalid("missing characterId"))
            } else {
                Ok(())
            };
        };
        let user = uid(d).unwrap_or_default();
        let account = if user.is_empty() {
            self.character_accounts.get(&character).cloned()
        } else {
            Some(self.acct(&user)?)
        };
        let mut instanced = array(value_or(d.get("instancedItems"), json!([])));
        let mut stacked = array(value_or(d.get("stackedItems"), json!([])));
        // RB11 `migrate_inventory_storage_arrays` (§8.2); its change and issue lists are not logged
        if let Some(policy) = self.settings.policy {
            let migrated = migrate_inventory_storage_arrays(policy, instanced, stacked);
            instanced = migrated.instanced;
            stacked = migrated.stacked;
        }
        self.counters.inventory_arrays += 1;
        self.inventories.insert(
            character,
            PendingInventory {
                user,
                account,
                instanced,
                stacked,
                revision: int_or(d.get("revision"), 0),
                bootstrap: text(d.get("bootstrapVersion")),
            },
        );
        Ok(())
    }

    fn loadouts(&mut self, d: &Doc) -> Step {
        let character = need(text(pick(d, &["characterId", "_id"])), "missing characterId")?;
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        let persisted = match d.get("unlockedTotalSlots") {
            Some(Value::Number(n)) => n.as_f64(),
            _ => None,
        };
        self.loadouts.insert(
            character,
            PendingLoadout {
                user,
                account,
                slots: array(value_or(d.get("loadouts"), json!([]))),
                persistent: value_or(d.get("persistent"), json!({})),
                active_index: int_or(d.get("activeIndex"), 0),
                persisted,
                revision: int_or(d.get("revision"), 0),
                bootstrap: text(d.get("bootstrapVersion")),
            },
        );
        Ok(())
    }

    /// §8.3 to §8.5 over the pending inventories and loadouts.
    fn repairs(&mut self) {
        let starter = self.settings.starter;
        let mut inventories = std::mem::take(&mut self.inventories);
        let mut loadouts = std::mem::take(&mut self.loadouts);
        let characters: BTreeSet<String> = inventories.keys().chain(loadouts.keys()).cloned().collect();
        for character in &characters {
            match (inventories.get_mut(character), loadouts.get_mut(character)) {
                (Some(inv), Some(lo)) => {
                    let user = if inv.user.is_empty() {
                        lo.user.clone()
                    } else {
                        inv.user.clone()
                    };
                    let side = LoadoutSide {
                        slots: &mut lo.slots,
                        persistent: &mut lo.persistent,
                    };
                    repair_starter(starter, &user, character, &mut inv.instanced, Some(side));
                }
                (Some(inv), None) => repair_starter(starter, &inv.user, character, &mut inv.instanced, None),
                (None, Some(lo)) => {
                    // Banner and glider ids that exist nowhere (P6)
                    let mut throwaway = Vec::new();
                    let side = LoadoutSide {
                        slots: &mut lo.slots,
                        persistent: &mut lo.persistent,
                    };
                    repair_starter(starter, &lo.user, character, &mut throwaway, Some(side));
                }
                (None, None) => continue,
            }
            self.counters.starter_instances += 1;
        }
        if self.has(Area::Characters) {
            for (character, mut lo) in std::mem::take(&mut loadouts) {
                let (unlocked, active) = reconcile_slots(
                    &mut lo.slots,
                    self.journeys.get(&lo.account),
                    lo.persisted,
                    lo.active_index,
                );
                self.counters.loadout_slots += 1;
                self.put(
                    "loadouts",
                    json!({
                        "character_id": character,
                        "account_id": lo.account,
                        "slots_json": Value::Array(lo.slots).to_string(),
                        "persistent": lo.persistent.to_string(),
                        "active_index": active,
                        "unlocked_total_slots": unlocked,
                        "revision": lo.revision,
                        "bootstrap_version": lo.bootstrap,
                    }),
                );
            }
        }
        if self.has(Area::Inventory) {
            for (character, inv) in std::mem::take(&mut inventories) {
                self.put(
                    "inventories",
                    json!({
                        "character_id": character,
                        "account_id": inv.account,
                        "instanced_items": Value::Array(inv.instanced).to_string(),
                        "stacked_items": Value::Array(inv.stacked).to_string(),
                        "revision": inv.revision,
                        "bootstrap_version": inv.bootstrap,
                    }),
                );
            }
        }
    }

    fn inventory_transactions(&mut self, d: &Doc) -> Step {
        const CORE: [&str; 12] = [
            "_id",
            "userId",
            "accountId",
            "characterId",
            "transactionId",
            "requestHash",
            "status",
            "result",
            "createdAt",
            "completedAt",
            "economySource",
            "economySessionId",
        ];
        let user = need(uid(d), "missing userId")?;
        let character = need(text(d.get("characterId")), "missing characterId")?;
        let transaction = need(text(pick(d, &["transactionId", "_id"])), "missing transactionId")?;
        let account = self.acct(&user)?;
        let extra: Doc = d
            .iter()
            .filter(|(k, _)| !CORE.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let extra = if extra.is_empty() {
            Value::Null
        } else {
            Value::String(Value::Object(extra).to_string())
        };
        self.put(
            "inventory_transactions",
            json!({
                "account_id": account,
                "character_id": character,
                "transaction_id": transaction,
                "request_hash": text(d.get("requestHash")),
                "status": text_or(d.get("status"), "completed"),
                "result": json_or_null(d.get("result"), "{}"),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "completed_at": time(d.get("completedAt")),
                "economy_source": text(d.get("economySource")),
                "economy_session_id": text(d.get("economySessionId")),
                "extra": extra,
            }),
        );
        Ok(())
    }

    // ------------------------------------------------------------------ bounties

    fn bounty_states(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        self.put(
            "bounty_states",
            json!({
                "account_id": account,
                "bounties": json_text_or(d.get("bounties"), "[]"),
                "draft_data": json_text_or(d.get("draftData"), "{}"),
                "daily_reward_credited_on": text(d.get("dailyRewardCreditedOn")),
                "daily_premium_draft_granted_on": text(d.get("dailyPremiumDraftGrantedOn")),
                "update_version": int_or(d.get("updateVersion"), 0),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "updated_at": time_or(d.get("updatedAt"), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn cooldown_states(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        self.put(
            "cooldown_states",
            json!({
                "account_id": account,
                "entries": json_text_or(pick(d, &["entries", "cooldowns"]), "{}"),
                "update_version": int_or(d.get("updateVersion"), 0),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "updated_at": time_or(d.get("updatedAt"), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn entitlements(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        for entry in array(value_or(d.get("entitlements"), json!([]))) {
            let Some(entry) = entry.as_object() else { continue };
            let Some(name) = text(entry.get("name")) else { continue };
            self.put(
                "entitlements",
                json!({
                    "account_id": account,
                    "name": name,
                    "duration": float(float_or(entry.get("duration"), 0.0)),
                    "activated_date": time(entry.get("activatedDate")),
                    "granted_at": time(entry.get("grantedAt")),
                    "source_sku_id": text(entry.get("sourceSkuId")),
                }),
            );
        }
        Ok(())
    }

    fn wallets(&mut self, d: &Doc) -> Step {
        let user = need(uid(d), "missing userId")?;
        let account = self.acct(&user)?;
        let mut balances = Vec::new();
        if let Value::Object(entries) = value_or(d.get("balances"), json!({})) {
            // The ordinal is the entry's position among all entries: a skipped one leaves a gap
            for (position, (catalog, amount)) in entries.iter().enumerate() {
                if let Some(amount) = amount.as_f64() {
                    balances.push(json!([catalog, float(amount), position]));
                }
            }
        }
        self.put(
            "wallets",
            json!({"account_id": account, "bootstrap_version": text(d.get("bootstrapVersion")), "balances": balances}),
        );
        Ok(())
    }

    fn economy_daily(&mut self, d: &Doc) -> Step {
        const SKIP: [&str; 7] = ["_id", "userId", "day", "currency", "ttlAt", "expiresAt", "__v"];
        let user = need(uid(d), "missing userId")?;
        let day = need(text(d.get("day")), "missing day")?;
        let currency = need(text(d.get("currency")), "missing currency")?;
        let account = self.acct(&user)?;
        let ttl = ms_or(pick(d, &["ttlAt", "expiresAt"]), 0);
        let mut leaves = Vec::new();
        for (key, value) in d.iter().filter(|(k, _)| !SKIP.contains(&k.as_str())) {
            flatten(key.clone(), value, &mut leaves);
        }
        for (path, value) in leaves {
            self.put(
                "economy_rollup_counters",
                json!({
                    "account_id": account,
                    "day": day,
                    "currency": currency,
                    "path": path,
                    "value": float(value),
                    "ttl_at": ttl,
                }),
            );
        }
        Ok(())
    }

    // ------------------------------------------------------------------ parties

    fn memberships(&mut self, d: &Doc) -> Step {
        if let (Some(user), Some(party)) = (uid(d), text(d.get("partyId"))) {
            self.memberships
                .insert((party, user), time_or(d.get("joinedAt"), DEFAULT_TIME));
        }
        Ok(())
    }

    fn parties(&mut self, d: &Doc) -> Step {
        let client = need(text(pick(d, &["partyId", "_id"])), "missing partyId/_id")?;
        let leader = need(text(pick(d, &["leaderPlayerId", "leader"])), "missing leaderPlayerId")?;
        let id = self.resolver.party(&client)?;
        let leader = self.acct(&leader)?;
        let created = time_or(d.get("createdAt"), DEFAULT_TIME);
        let mut members = Vec::new();
        if let Some(Value::Array(list)) = d.get("members") {
            for (position, entry) in list.iter().enumerate() {
                let Some(member) = entry.as_str() else { continue };
                let account = self.acct(member)?;
                let joined = self
                    .memberships
                    .get(&(client.clone(), member.to_owned()))
                    .cloned()
                    .unwrap_or_else(|| created.clone());
                members.push(json!([account, position, joined]));
            }
        }
        if members.is_empty() {
            members.push(json!([leader, 0, created]));
        }
        self.put(
            "parties",
            json!({
                "id": id,
                "client_id": client,
                "revision": int_or(d.get("revision"), 1),
                "leader": leader,
                "build_id": text_or(d.get("buildId"), "647472"),
                "activity": text_or(d.get("activity"), "IDLE"),
                "active_hunt_session_id": text(d.get("activeHuntSessionId")),
                "created_at": created,
                "updated_at": time_or(d.get("updatedAt"), &created),
                "members": members,
            }),
        );
        Ok(())
    }

    fn party_invites(&mut self, d: &Doc) -> Step {
        let recipient = need(
            text(pick(d, &["recipientPlayerId", "recipient"])),
            "missing recipientPlayerId",
        )?;
        let sender = need(text(pick(d, &["sendingPlayerId", "sender"])), "missing sendingPlayerId")?;
        let party = need(text(d.get("partyId")), "missing partyId")?;
        let recipient_id = self.acct(&recipient)?;
        let sender_id = self.acct(&sender)?;
        let party = self.resolver.party(&party)?;
        self.put(
            "party_invites",
            json!({
                "recipient": recipient_id,
                "sender": sender_id,
                "party": party,
                "party_revision": int_or(d.get("partyRevision"), 1),
                "sending_display_name": text_or(d.get("sendingDisplayName"), &sender),
                "sending_platform": text_or(d.get("sendingPlatform"), "PC"),
                "status": text_or(d.get("status"), "PENDING"),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "expires_at": time_or(d.get("expiresAt"), DEFAULT_TIME),
                "consumed_at": time(d.get("consumedAt")),
            }),
        );
        Ok(())
    }

    fn party_outbox(&mut self, d: &Doc) -> Step {
        let id = need(text(pick(d, &["eventId", "_id"])), "missing eventId/_id")?;
        let party = need(text(d.get("partyId")), "missing partyId")?;
        self.put(
            "party_outbox",
            json!({
                "id": id,
                // Node's party id and recipients as they are (P8)
                "party": party,
                "party_revision": int_or(d.get("partyRevision"), 1),
                "event_type": text_or(d.get("eventType"), "PARTY_CREATED"),
                "recipients": json_text_or(pick(d, &["recipientAccountIds", "recipients"]), "[]"),
                "payload": json_text_or(d.get("payload"), "{}"),
                "state": text_or(d.get("state"), "PENDING"),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "claimed_at": time(d.get("claimedAt")),
                "claim_owner": text(d.get("claimOwner")),
                "published_at": time(d.get("publishedAt")),
            }),
        );
        Ok(())
    }

    fn friendships(&mut self, d: &Doc) -> Step {
        let owner = need(text(d.get("ownerId")), "missing ownerId")?;
        let other = need(text(d.get("otherId")), "missing otherId")?;
        let owner_id = self.acct(&owner)?;
        let other_id = self.acct(&other)?;
        self.put(
            "friend_edges",
            json!({
                "owner": owner_id,
                "other": other_id,
                "other_user_id": other,
                "status": text_or(d.get("status"), "ACCEPTED"),
                "direction": text(d.get("direction")),
                "created": time_or(d.get("created"), DEFAULT_TIME),
                "favorite": bool_or(d.get("favorite"), false),
            }),
        );
        Ok(())
    }

    // ------------------------------------------------------------------ reforged

    fn reforged_invites(&mut self, d: &Doc) -> Step {
        let code = need(text(pick(d, &["invitecode", "code", "_id"])), "missing invitecode")?;
        self.put(
            "reforged_invites",
            json!({
                "code": code,
                "uses_remaining": int_or(pick(d, &["usesRemaining", "uses_remaining"]), 1),
                "infinite_uses": bool_or(pick(d, &["infiniteUses", "infinite_uses"]), false),
                "note": text(d.get("note")),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "expires_at": text(d.get("expiresAt")),
            }),
        );
        Ok(())
    }

    fn admin_sessions(&mut self, d: &Doc) -> Step {
        let id = need(text(pick(d, &["id", "_id"])), "missing id/_id")?;
        let user = need(uid(d), "missing userId")?;
        let token = need(text(d.get("tokenHash")), "missing tokenHash")?;
        let account = self.acct(&user)?;
        let expires = time_or(d.get("expiresAt"), DEFAULT_TIME);
        let expires_ms = ms_or(Some(&Value::String(expires.clone())), 0);
        self.put(
            "admin_sessions",
            json!({
                "id": id,
                "token_hash": token,
                "account": account,
                "csrf_token": text_or(d.get("csrfToken"), ""),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "expires_at": expires,
                "ip": text_or(d.get("ip"), "127.0.0.1"),
                "user_agent": text_or(d.get("userAgent"), ""),
                "revoked_at": time(d.get("revokedAt")),
                "ttl_at": ms_or(d.get("ttlAt"), expires_ms),
            }),
        );
        Ok(())
    }

    fn admin_audit(&mut self, d: &Doc) -> Step {
        let id = need(text(pick(d, &["id", "_id"])), "missing id/_id")?;
        // Node user ids, not mapped (P8)
        self.put(
            "admin_audit",
            json!({
                "id": id,
                "actor_user_id": text_or(d.get("actorUserId"), "system"),
                "target_user_id": text(d.get("targetUserId")),
                "action": text_or(d.get("action"), "UNKNOWN"),
                "old_state": json_or_null(d.get("oldState"), "{}"),
                "new_state": json_or_null(d.get("newState"), "{}"),
                "reason": text(d.get("reason")),
                "ip": text_or(d.get("ip"), "127.0.0.1"),
                "request_id": text_or(d.get("requestId"), ""),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
            }),
        );
        Ok(())
    }

    fn launcher_notices(&mut self, d: &Doc) -> Step {
        self.put(
            "launcher_notices",
            json!({
                "id": 1,
                "version": int_or(d.get("version"), 1),
                "updated_at": time(d.get("updatedAt")),
                "updated_by": text(d.get("updatedBy")),
                "announcements": json_text_or(d.get("announcements"), "[]"),
                "tags": json_text_or(d.get("tags"), "[]"),
            }),
        );
        Ok(())
    }

    fn operations_policy(&mut self, d: &Doc) -> Step {
        self.put(
            "operations_policy",
            json!({
                "id": 1,
                "guard_enforcement": text_or(d.get("guardEnforcement"), "ENFORCE"),
                "diagnostics_profile": text_or(d.get("diagnosticsProfile"), "PRODUCTION"),
                "version": int_or(d.get("version"), 1),
                "updated_at": time(d.get("updatedAt")),
                "updated_by": text(d.get("updatedBy")),
                "diagnostics_expires_at": time(d.get("diagnosticsExpiresAt")),
            }),
        );
        Ok(())
    }

    fn guard_sessions(&mut self, d: &Doc) -> Step {
        let id = need(text(pick(d, &["guardSessionId", "_id"])), "missing guardSessionId")?;
        let user = need(uid(d), "missing accountId")?;
        let account = self.acct(&user)?;
        let role = if text_or(d.get("role"), "client") == "host" {
            "host"
        } else {
            "client"
        };
        let detail = if d.contains_key("detail") {
            json_text_or(d.get("detail"), "{}")
        } else {
            json!({
                "launcherVersion": text_or(d.get("launcherVersion"), ""),
                "lastSignals": d.get("lastSignals").cloned().unwrap_or(Value::Null),
                "reasons": value_or(d.get("reasons"), json!([])),
                "history": value_or(d.get("history"), json!([])),
            })
            .to_string()
        };
        let s = |k: &str| text_or(d.get(k), "");
        self.put(
            "launcher_guard_sessions",
            json!({
                "guard_session_id": id,
                "account": account,
                "role": role,
                "public_key_base64": s("publicKeyBase64"),
                "manifest_id": s("manifestId"),
                "manifest_sequence": int_or(d.get("manifestSequence"), 0),
                "channel": text_or(d.get("channel"), "stable"),
                "challenge": s("challenge"),
                "last_sequence": int_or(d.get("lastSequence"), 0),
                "status": text_or(d.get("status"), "CHALLENGED"),
                "risk_score": float(float_or(d.get("riskScore"), 0.0)),
                "created_at": time_or(d.get("createdAt"), DEFAULT_TIME),
                "expires_at": time_or(d.get("expiresAt"), DEFAULT_TIME),
                "last_heartbeat_at": time(d.get("lastHeartbeatAt")),
                "detail": detail,
                "ttl_at": ms_or(d.get("ttlAt"), 0),
            }),
        );
        Ok(())
    }

    fn guard_findings(&mut self, d: &Doc) -> Step {
        let file = need(text(d.get("fileName")), "missing fileName")?;
        let sha = need(text(d.get("sha256")), "missing sha256")?;
        let mut accounts = Vec::new();
        for entry in array(value_or(d.get("accountIds"), json!([]))) {
            let Value::String(entry) = entry else { continue };
            if entry.starts_with("account:") {
                accounts.push(Value::String(entry));
            } else if !entry.trim().is_empty() {
                accounts.push(Value::String(self.acct(&entry)?));
            }
        }
        let count = i64::try_from(accounts.len()).unwrap_or(i64::MAX);
        self.put(
            "guard_findings",
            json!({
                "file_name": file,
                "sha256": sha,
                "signer": text_or(d.get("signer"), ""),
                "size": int_or(d.get("size"), 0),
                "first_seen_at": time_or(d.get("firstSeenAt"), DEFAULT_TIME),
                "last_seen_at": time_or(d.get("lastSeenAt"), DEFAULT_TIME),
                "session_count": int_or(d.get("sessionCount"), 1),
                "account_count": int_or(d.get("accountCount"), count),
                "account_ids": Value::Array(accounts).to_string(),
                "ttl_at": ms_or(d.get("ttlAt"), 0),
            }),
        );
        Ok(())
    }
}

/// Number leaves as `a.b.c` paths; any other leaf is dropped.
fn flatten(path: String, value: &Value, out: &mut Vec<(String, f64)>) {
    match value {
        Value::Number(n) => out.push((path, n.as_f64().unwrap_or(0.0))),
        Value::Object(map) => {
            for (k, v) in map {
                flatten(format!("{path}.{k}"), v, out);
            }
        }
        _ => {}
    }
}
