//! `isles-db export-mongo` (RB13): exports the live SQLite or Postgres `store::Database` back into
//! MongoDB collections in Rustic Isles 1.14.7's document shape for one-release rollback.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::migrate::{DbError, MigrateArea, TargetState, open_target_database};
use crate::mongo::MongoClient;
use crate::verify::read_target_state;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    pub from_source: String,
    pub to_uri: String,
    pub areas: Vec<MigrateArea>,
    pub dry_run: bool,
    pub allow_remote_postgres: bool,
}

impl ExportOptions {
    pub fn effective_areas(&self) -> BTreeSet<MigrateArea> {
        if self.areas.is_empty() {
            MigrateArea::ALL.into_iter().collect()
        } else {
            self.areas.iter().copied().collect()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportReport {
    pub dry_run: bool,
    pub counts: BTreeMap<String, usize>,
}

fn user_id_for(state: &TargetState, account_id: &str) -> String {
    state
        .accounts
        .get(account_id)
        .map(|a| a.user_id.clone())
        .unwrap_or_else(|| account_id.to_owned())
}

fn party_client_id_for(state: &TargetState, party_gid: &str) -> String {
    state
        .parties
        .get(party_gid)
        .map(|p| p.client_id.clone())
        .unwrap_or_else(|| party_gid.to_owned())
}

fn parse_json_str(raw: &str, fallback: Value) -> Value {
    serde_json::from_str(raw).unwrap_or(fallback)
}

fn insert_opt_str(doc: &mut Map<String, Value>, key: &str, val: &Option<String>) {
    if let Some(s) = val {
        doc.insert(key.to_owned(), Value::String(s.clone()));
    }
}

fn insert_dotted_f64(doc: &mut Map<String, Value>, dotted_path: &str, value: f64) {
    let parts: Vec<&str> = dotted_path.split('.').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return;
    }
    insert_parts(doc, &parts, value);
}

fn insert_parts(cur: &mut Map<String, Value>, parts: &[&str], value: f64) {
    if parts.len() == 1 {
        cur.insert(parts[0].to_owned(), json!(value));
        return;
    }
    let entry = cur
        .entry(parts[0].to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Some(sub) = entry.as_object_mut() {
        insert_parts(sub, &parts[1..], value);
    }
}

fn ledger_id(user_id: &str, character_id: &str, transaction_id: &str) -> String {
    let encoded = serde_json::to_string(&[user_id, character_id, transaction_id]).unwrap_or_default();
    let digest = Sha256::digest(encoded.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Converts `TargetState` from `store::Database` back into Rustic Isles 1.14.7 MongoDB collections.
pub fn state_to_mongo_collections(
    state: &TargetState,
    areas: &BTreeSet<MigrateArea>,
) -> BTreeMap<String, Vec<Map<String, Value>>> {
    let mut out: BTreeMap<String, Vec<Map<String, Value>>> = BTreeMap::new();

    if areas.contains(&MigrateArea::Auth) {
        let mut accounts_docs = Vec::with_capacity(state.accounts.len());
        for acc in state.accounts.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(acc.user_id.clone()));
            doc.insert("userId".into(), Value::String(acc.user_id.clone()));
            doc.insert("name".into(), Value::String(acc.name.clone()));
            doc.insert("notes".into(), json!(acc.notes));

            if let Some(la) = state.launcher_accounts.get(&acc.id) {
                insert_opt_str(&mut doc, "email", &la.email);
                doc.insert("displayName".into(), Value::String(la.display_name.clone()));
                doc.insert(
                    "displayNameNormalized".into(),
                    Value::String(la.display_name_normalized.clone()),
                );
                insert_opt_str(&mut doc, "passwordHash", &la.password_hash);
                doc.insert("status".into(), Value::String(la.status.clone()));
                insert_opt_str(&mut doc, "approvalStatus", &la.approval_status);
                insert_opt_str(&mut doc, "approvalUpdatedAt", &la.approval_updated_at);
                insert_opt_str(&mut doc, "approvalUpdatedBy", &la.approval_updated_by);
                insert_opt_str(&mut doc, "approvalReason", &la.approval_reason);
                doc.insert("roles".into(), parse_json_str(&la.roles, json!(["user"])));
                insert_opt_str(&mut doc, "rolesUpdatedAt", &la.roles_updated_at);
                insert_opt_str(&mut doc, "rolesUpdatedBy", &la.roles_updated_by);
                doc.insert("createdAt".into(), Value::String(la.created_at.clone()));
                insert_opt_str(&mut doc, "lastLoginAt", &la.last_login_at);
                if let Some(us) = la.username_set {
                    doc.insert("usernameSet".into(), json!(us));
                }
                insert_opt_str(&mut doc, "consentTermsVersion", &la.consent_terms_version);
                insert_opt_str(&mut doc, "consentPrivacyVersion", &la.consent_privacy_version);
                insert_opt_str(&mut doc, "consentAcceptedAt", &la.consent_accepted_at);
                insert_opt_str(&mut doc, "deletionRequestedAt", &la.deletion_requested_at);
                insert_opt_str(&mut doc, "deletionScheduledFor", &la.deletion_scheduled_for);
                insert_opt_str(&mut doc, "emailVerifiedAt", &la.email_verified_at);
                insert_opt_str(
                    &mut doc,
                    "emailVerificationPendingSince",
                    &la.email_verification_pending_since,
                );
            }
            accounts_docs.push(doc);
        }
        out.insert("accounts".into(), accounts_docs);

        let mut ai_docs = Vec::with_capacity(state.auth_identities.len());
        for ai in state.auth_identities.values() {
            let mut doc = Map::new();
            doc.insert(
                "_id".into(),
                Value::String(format!("{}:{}", ai.provider, ai.provider_subject)),
            );
            doc.insert("userId".into(), Value::String(user_id_for(state, &ai.account_id)));
            doc.insert("provider".into(), Value::String(ai.provider.clone()));
            doc.insert("providerSubject".into(), Value::String(ai.provider_subject.clone()));
            doc.insert("providerUsername".into(), Value::String(ai.provider_username.clone()));
            insert_opt_str(&mut doc, "providerAvatarUrl", &ai.provider_avatar_url);
            doc.insert("linkedAt".into(), Value::String(ai.linked_at.clone()));
            ai_docs.push(doc);
        }
        out.insert("authIdentities".into(), ai_docs);

        let mut rs_docs = Vec::with_capacity(state.refresh_sessions.len());
        for rs in state.refresh_sessions.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(rs.id.clone()));
            doc.insert("sessionId".into(), Value::String(rs.id.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &rs.account_id)));
            doc.insert("tokenHash".into(), Value::String(rs.token_hash.clone()));
            doc.insert("familyId".into(), Value::String(rs.family_id.clone()));
            doc.insert("deviceId".into(), Value::String(rs.device_id.clone()));
            doc.insert("deviceName".into(), Value::String(rs.device_name.clone()));
            doc.insert("createdAt".into(), Value::String(rs.created_at.clone()));
            doc.insert("expiresAt".into(), Value::String(rs.expires_at.clone()));
            insert_opt_str(&mut doc, "revokedAt", &rs.revoked_at);
            rs_docs.push(doc);
        }
        out.insert("refreshSessions".into(), rs_docs);

        let mut gec_docs = Vec::with_capacity(state.game_exchange_codes.len());
        for gec in state.game_exchange_codes.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(gec.code_hash.clone()));
            doc.insert("codeHash".into(), Value::String(gec.code_hash.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &gec.account_id)));
            doc.insert(
                "launcherSessionId".into(),
                Value::String(gec.launcher_session_id.clone()),
            );
            doc.insert("buildChangelist".into(), json!(gec.build_changelist));
            doc.insert("executableSha256".into(), Value::String(gec.executable_sha256.clone()));
            doc.insert("runtimeChannel".into(), Value::String(gec.runtime_channel.clone()));
            doc.insert("runtimeSha256".into(), Value::String(gec.runtime_sha256.clone()));
            doc.insert(
                "runtimeManifestVersion".into(),
                Value::String(gec.runtime_manifest_version.clone()),
            );
            doc.insert(
                "runtimeArtifactSetSha256".into(),
                Value::String(gec.runtime_artifact_set_sha256.clone()),
            );
            insert_opt_str(&mut doc, "realmId", &gec.realm_id);
            doc.insert("createdAt".into(), Value::String(gec.created_at.clone()));
            doc.insert("expiresAt".into(), Value::String(gec.expires_at.clone()));
            insert_opt_str(&mut doc, "consumedAt", &gec.consumed_at);
            gec_docs.push(doc);
        }
        out.insert("gameExchangeCodes".into(), gec_docs);

        let mut do_docs = Vec::with_capacity(state.discord_oauth_transactions.len());
        for d in state.discord_oauth_transactions.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(d.state.clone()));
            doc.insert("state".into(), Value::String(d.state.clone()));
            doc.insert("codeVerifier".into(), Value::String(d.code_verifier.clone()));
            doc.insert("createdAt".into(), Value::String(d.created_at.clone()));
            doc.insert("expiresAt".into(), Value::String(d.expires_at.clone()));
            insert_opt_str(&mut doc, "consumedAt", &d.consumed_at);
            insert_opt_str(&mut doc, "completionCodeHash", &d.completion_code_hash);
            insert_opt_str(&mut doc, "completionExpiresAt", &d.completion_expires_at);
            insert_opt_str(&mut doc, "completionConsumedAt", &d.completion_consumed_at);
            if let Some(acc) = &d.account_id {
                doc.insert("userId".into(), Value::String(user_id_for(state, acc)));
            }
            if let Some(link_acc) = &d.link_account_id {
                doc.insert("linkUserId".into(), Value::String(user_id_for(state, link_acc)));
            }
            do_docs.push(doc);
        }
        out.insert("discordOAuthTransactions".into(), do_docs);

        let mut ec_docs = Vec::with_capacity(state.email_codes.len());
        for ec in state.email_codes.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &ec.account_id);
            doc.insert("_id".into(), Value::String(format!("{uid}:{}", ec.purpose)));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("purpose".into(), Value::String(ec.purpose.clone()));
            doc.insert("codeHash".into(), Value::String(ec.code_hash.clone()));
            doc.insert("attempts".into(), json!(ec.attempts));
            doc.insert("createdAt".into(), Value::String(ec.created_at.clone()));
            doc.insert("expiresAt".into(), Value::String(ec.expires_at.clone()));
            ec_docs.push(doc);
        }
        out.insert("emailCodes".into(), ec_docs);

        let mut gs_docs = Vec::with_capacity(state.game_server_api_keys.len());
        for key_hash in &state.game_server_api_keys {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(key_hash.clone()));
            doc.insert("keyHash".into(), Value::String(key_hash.clone()));
            gs_docs.push(doc);
        }
        out.insert("gameServerApiKeys".into(), gs_docs);

        let mut uk_docs = Vec::with_capacity(state.user_api_keys.len());
        for (acc_id, key_hash) in &state.user_api_keys {
            let mut doc = Map::new();
            let uid = user_id_for(state, acc_id);
            doc.insert("_id".into(), Value::String(uid.clone()));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("keyHash".into(), Value::String(key_hash.clone()));
            uk_docs.push(doc);
        }
        out.insert("userApiKeys".into(), uk_docs);
    }

    if areas.contains(&MigrateArea::Characters) {
        let mut char_docs = Vec::with_capacity(state.characters.len());
        for c in state.characters.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(c.character_id.clone()));
            doc.insert("characterId".into(), Value::String(c.character_id.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &c.account_id)));
            doc.insert("ord".into(), json!(c.ord));
            doc.insert("createdDate".into(), Value::String(c.created_date.clone()));
            doc.insert("lastModifiedDate".into(), Value::String(c.last_modified_date.clone()));
            doc.insert("name".into(), Value::String(c.name.clone()));
            doc.insert("updateVersion".into(), json!(c.update_version));
            doc.insert("data".into(), parse_json_str(&c.data, json!({})));
            char_docs.push(doc);
        }
        out.insert("characters".into(), char_docs);

        let mut ld_docs = Vec::with_capacity(state.loadouts.len());
        for ld in state.loadouts.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(ld.character_id.clone()));
            doc.insert("characterId".into(), Value::String(ld.character_id.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &ld.account_id)));
            doc.insert("loadouts".into(), parse_json_str(&ld.slots_json, json!([])));
            doc.insert("persistent".into(), parse_json_str(&ld.persistent, json!({})));
            doc.insert("activeIndex".into(), json!(ld.active_index));
            if let Some(uts) = ld.unlocked_total_slots {
                doc.insert("unlockedTotalSlots".into(), json!(uts));
            }
            doc.insert("revision".into(), json!(ld.revision));
            insert_opt_str(&mut doc, "bootstrapVersion", &ld.bootstrap_version);
            ld_docs.push(doc);
        }
        out.insert("loadouts".into(), ld_docs);
    }

    if areas.contains(&MigrateArea::Inventory) {
        let mut inv_docs = Vec::with_capacity(state.inventories.len());
        for inv in state.inventories.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(inv.character_id.clone()));
            doc.insert("characterId".into(), Value::String(inv.character_id.clone()));
            let uid = inv
                .account_id
                .as_deref()
                .map(|a| user_id_for(state, a))
                .or_else(|| {
                    state
                        .characters
                        .get(&inv.character_id)
                        .map(|c| user_id_for(state, &c.account_id))
                })
                .unwrap_or_default();
            if !uid.is_empty() {
                doc.insert("userId".into(), Value::String(uid));
            }
            doc.insert("instancedItems".into(), parse_json_str(&inv.instanced_items, json!([])));
            doc.insert("stackedItems".into(), parse_json_str(&inv.stacked_items, json!([])));
            doc.insert("revision".into(), json!(inv.revision));
            insert_opt_str(&mut doc, "bootstrapVersion", &inv.bootstrap_version);
            inv_docs.push(doc);
        }
        out.insert("inventories".into(), inv_docs);

        let mut itx_docs = Vec::with_capacity(state.inventory_transactions.len());
        for tx in state.inventory_transactions.values() {
            let mut doc = Map::new();
            if let Some(extra_raw) = &tx.extra {
                if let Ok(Value::Object(extra_map)) = serde_json::from_str::<Value>(extra_raw) {
                    for (k, v) in extra_map {
                        doc.insert(k, v);
                    }
                } else {
                    doc.insert("extra".into(), Value::String(extra_raw.clone()));
                }
            }
            let uid = user_id_for(state, &tx.account_id);
            doc.insert(
                "_id".into(),
                Value::String(ledger_id(&uid, &tx.character_id, &tx.transaction_id)),
            );
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("characterId".into(), Value::String(tx.character_id.clone()));
            doc.insert("transactionId".into(), Value::String(tx.transaction_id.clone()));
            insert_opt_str(&mut doc, "requestHash", &tx.request_hash);
            doc.insert("status".into(), Value::String(tx.status.clone()));
            if let Some(res) = &tx.result {
                doc.insert("result".into(), parse_json_str(res, json!({})));
            }
            doc.insert("createdAt".into(), Value::String(tx.created_at.clone()));
            insert_opt_str(&mut doc, "completedAt", &tx.completed_at);
            insert_opt_str(&mut doc, "economySource", &tx.economy_source);
            insert_opt_str(&mut doc, "economySessionId", &tx.economy_session_id);
            itx_docs.push(doc);
        }
        out.insert("inventoryTransactions".into(), itx_docs);
    }

    if areas.contains(&MigrateArea::Progression) {
        let mut pj_docs = Vec::with_capacity(state.player_journey.len());
        for pj in state.player_journey.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &pj.account);
            doc.insert("_id".into(), Value::String(uid.clone()));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("nodes".into(), parse_json_str(&pj.nodes, json!({})));
            doc.insert("updateVersion".into(), json!(pj.update_version));
            doc.insert("revision".into(), json!(pj.revision));
            pj_docs.push(doc);
        }
        out.insert("playerJourney".into(), pj_docs);

        let mut pt_docs = Vec::with_capacity(state.progression_tracks.len());
        for pt in state.progression_tracks.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &pt.account);
            doc.insert("_id".into(), Value::String(format!("{uid}::{}", pt.progression_id)));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("progressionId".into(), Value::String(pt.progression_id.clone()));
            doc.insert("progress".into(), json!(pt.progress));
            doc.insert("confirmedFremiumRank".into(), json!(pt.confirmed_fremium_rank));
            doc.insert("confirmedPremiumRank".into(), json!(pt.confirmed_premium_rank));
            doc.insert("updateVersion".into(), json!(pt.update_version));
            doc.insert("createdAt".into(), Value::String(pt.created_at.clone()));
            doc.insert("updatedAt".into(), Value::String(pt.updated_at.clone()));
            if let Some(reforge) = &pt.reforge {
                doc.insert("reforge".into(), parse_json_str(reforge, json!({})));
            }
            if let Some(pc) = pt.prestige_claimed {
                doc.insert("prestigeClaimed".into(), json!(pc));
            }
            pt_docs.push(doc);
        }
        out.insert("progressionTracks".into(), pt_docs);

        let mut po_docs = Vec::with_capacity(state.progression_objectives.len());
        for po in state.progression_objectives.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &po.account);
            doc.insert("_id".into(), Value::String(format!("{uid}::{}", po.objective_id)));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("objectiveId".into(), Value::String(po.objective_id.clone()));
            doc.insert("value".into(), json!(po.value));
            doc.insert("completedCount".into(), json!(po.completed_count));
            doc.insert("createdAt".into(), Value::String(po.created_at.clone()));
            doc.insert("updatedAt".into(), Value::String(po.updated_at.clone()));
            po_docs.push(doc);
        }
        out.insert("progressionObjectives".into(), po_docs);

        let mut poe_docs = Vec::with_capacity(state.progression_objective_events.len());
        for poe in state.progression_objective_events.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(poe.id.clone()));
            doc.insert("id".into(), Value::String(poe.id.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &poe.account)));
            doc.insert("rawBody".into(), parse_json_str(&poe.raw_body, json!({})));
            doc.insert("receivedAt".into(), Value::String(poe.received_at.clone()));
            poe_docs.push(doc);
        }
        out.insert("progressionObjectiveEvents".into(), poe_docs);

        let mut pg_docs = Vec::with_capacity(state.progression_grants.len());
        for pg in state.progression_grants.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &pg.account);
            doc.insert("_id".into(), Value::String(pg.grant_id.clone()));
            doc.insert("grantId".into(), Value::String(pg.grant_id.clone()));
            doc.insert("userId".into(), Value::String(uid.clone()));
            doc.insert("progressionId".into(), Value::String(pg.progression_id.clone()));
            doc.insert("amount".into(), json!(pg.amount));
            doc.insert("requestHash".into(), Value::String(pg.request_hash.clone()));
            doc.insert("status".into(), Value::String(pg.status.clone()));
            let mut res_val = parse_json_str(&pg.result, json!({}));
            if let Some(m) = res_val.as_object_mut() {
                m.insert("userId".into(), Value::String(uid));
            }
            doc.insert("result".into(), res_val);
            doc.insert("createdAt".into(), Value::String(pg.created_at.clone()));
            pg_docs.push(doc);
        }
        out.insert("progressionTransactions".into(), pg_docs);

        let mut ep_docs = Vec::with_capacity(state.escalation_progress.len());
        for ep in state.escalation_progress.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &ep.account);
            doc.insert("_id".into(), Value::String(format!("{uid}::{}", ep.season_id)));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("seasonId".into(), Value::String(ep.season_id.clone()));
            doc.insert("escalationLevel".into(), json!(ep.escalation_level));
            doc.insert("nextLevelXp".into(), json!(ep.next_level_xp));
            doc.insert(
                "talentsProgress".into(),
                parse_json_str(&ep.talents_progress, json!([])),
            );
            doc.insert("unlockProgress".into(), parse_json_str(&ep.unlock_progress, json!([])));
            doc.insert("updateVersion".into(), json!(ep.update_version));
            doc.insert("createdAt".into(), Value::String(ep.created_at.clone()));
            doc.insert("updatedAt".into(), Value::String(ep.updated_at.clone()));
            ep_docs.push(doc);
        }
        out.insert("escalationProgress".into(), ep_docs);

        let mut bc_docs = Vec::with_capacity(state.breadcrumbs.len());
        for bc in state.breadcrumbs.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(bc.character_id.clone()));
            doc.insert("characterId".into(), Value::String(bc.character_id.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &bc.account)));
            doc.insert("breadcrumbs".into(), parse_json_str(&bc.items, json!([])));
            doc.insert("updateVersion".into(), json!(bc.update_version));
            bc_docs.push(doc);
        }
        out.insert("breadcrumbs".into(), bc_docs);

        let mut ec_docs = Vec::with_capacity(state.encountered_content.len());
        for ec in state.encountered_content.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(ec.character_id.clone()));
            doc.insert("characterId".into(), Value::String(ec.character_id.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &ec.account)));
            doc.insert("encounteredcontent".into(), parse_json_str(&ec.content, json!([])));
            ec_docs.push(doc);
        }
        out.insert("encounteredContent".into(), ec_docs);
    } else if areas.contains(&MigrateArea::Characters) {
        // When only Characters is exported, include playerJourney so loadout slot reconciliation has journey context.
        let mut pj_docs = Vec::with_capacity(state.player_journey.len());
        for pj in state.player_journey.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &pj.account);
            doc.insert("_id".into(), Value::String(uid.clone()));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("nodes".into(), parse_json_str(&pj.nodes, json!({})));
            doc.insert("updateVersion".into(), json!(pj.update_version));
            doc.insert("revision".into(), json!(pj.revision));
            pj_docs.push(doc);
        }
        out.insert("playerJourney".into(), pj_docs);
    }

    if areas.contains(&MigrateArea::Bounties) {
        let mut bs_docs = Vec::with_capacity(state.bounty_states.len());
        for bs in state.bounty_states.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &bs.account_id);
            doc.insert("_id".into(), Value::String(uid.clone()));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("bounties".into(), parse_json_str(&bs.bounties, json!([])));
            doc.insert("draftData".into(), parse_json_str(&bs.draft_data, json!({})));
            insert_opt_str(&mut doc, "dailyRewardCreditedOn", &bs.daily_reward_credited_on);
            insert_opt_str(
                &mut doc,
                "dailyPremiumDraftGrantedOn",
                &bs.daily_premium_draft_granted_on,
            );
            doc.insert("updateVersion".into(), json!(bs.update_version));
            doc.insert("createdAt".into(), Value::String(bs.created_at.clone()));
            doc.insert("updatedAt".into(), Value::String(bs.updated_at.clone()));
            bs_docs.push(doc);
        }
        out.insert("bountyStates".into(), bs_docs);

        let mut cs_docs = Vec::with_capacity(state.cooldown_states.len());
        for cs in state.cooldown_states.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &cs.account_id);
            doc.insert("_id".into(), Value::String(uid.clone()));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("entries".into(), parse_json_str(&cs.entries, json!({})));
            doc.insert("updateVersion".into(), json!(cs.update_version));
            doc.insert("createdAt".into(), Value::String(cs.created_at.clone()));
            doc.insert("updatedAt".into(), Value::String(cs.updated_at.clone()));
            cs_docs.push(doc);
        }
        out.insert("cooldownStates".into(), cs_docs);

        let mut ent_by_acc: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for ent in state.entitlements.values() {
            let mut item = Map::new();
            item.insert("name".into(), Value::String(ent.name.clone()));
            item.insert("duration".into(), json!(ent.duration));
            insert_opt_str(&mut item, "activatedDate", &ent.activated_date);
            insert_opt_str(&mut item, "grantedAt", &ent.granted_at);
            insert_opt_str(&mut item, "sourceSkuId", &ent.source_sku_id);
            ent_by_acc
                .entry(ent.account_id.clone())
                .or_default()
                .push(Value::Object(item));
        }
        let mut ent_docs = Vec::with_capacity(ent_by_acc.len());
        for (acc_id, list) in ent_by_acc {
            let mut doc = Map::new();
            let uid = user_id_for(state, &acc_id);
            doc.insert("_id".into(), Value::String(uid.clone()));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("entitlements".into(), Value::Array(list));
            ent_docs.push(doc);
        }
        out.insert("entitlements".into(), ent_docs);

        let mut w_docs = Vec::with_capacity(state.wallets.len());
        for w in state.wallets.values() {
            let mut doc = Map::new();
            let uid = user_id_for(state, &w.account_id);
            doc.insert("_id".into(), Value::String(uid.clone()));
            doc.insert("userId".into(), Value::String(uid));
            insert_opt_str(&mut doc, "bootstrapVersion", &w.bootstrap_version);
            let mut sorted_bal = w.balances.clone();
            sorted_bal.sort_by_key(|b| b.2);
            let mut bal_map = Map::new();
            for (cid, amount, _ord) in sorted_bal {
                bal_map.insert(cid, json!(amount));
            }
            doc.insert("balances".into(), Value::Object(bal_map));
            w_docs.push(doc);
        }
        out.insert("wallets".into(), w_docs);

        type EconGroupMap = BTreeMap<(String, String, String), (i64, Vec<(String, f64)>)>;
        let mut econ_grouped: EconGroupMap = BTreeMap::new();
        for ec in state.economy_rollup_counters.values() {
            let entry = econ_grouped
                .entry((ec.account_id.clone(), ec.day.clone(), ec.currency.clone()))
                .or_insert_with(|| (ec.ttl_at, Vec::new()));
            entry.1.push((ec.path.clone(), ec.value));
        }
        let mut econ_docs = Vec::with_capacity(econ_grouped.len());
        for ((acc_id, day, currency), (ttl_at, counters)) in econ_grouped {
            let mut doc = Map::new();
            let uid = user_id_for(state, &acc_id);
            doc.insert("_id".into(), Value::String(format!("{uid}:{day}:{currency}")));
            doc.insert("userId".into(), Value::String(uid));
            doc.insert("day".into(), Value::String(day));
            doc.insert("currency".into(), Value::String(currency));
            doc.insert("ttlAt".into(), json!(ttl_at));
            for (path, val) in counters {
                insert_dotted_f64(&mut doc, &path, val);
            }
            econ_docs.push(doc);
        }
        out.insert("economyDaily".into(), econ_docs);
    }

    if areas.contains(&MigrateArea::Parties) {
        let mut party_docs = Vec::with_capacity(state.parties.len());
        let mut membership_docs = Vec::new();
        for p in state.parties.values() {
            let mut sorted_members = p.members.clone();
            sorted_members.sort_by_key(|m| m.1);
            let mut member_uids = Vec::with_capacity(sorted_members.len());
            for (m_acc, _pos, joined_at) in sorted_members {
                let m_uid = user_id_for(state, &m_acc);
                member_uids.push(Value::String(m_uid.clone()));
                let mut m_doc = Map::new();
                m_doc.insert("_id".into(), Value::String(m_uid.clone()));
                m_doc.insert("partyId".into(), Value::String(p.client_id.clone()));
                m_doc.insert("userId".into(), Value::String(m_uid));
                m_doc.insert("joinedAt".into(), Value::String(joined_at));
                membership_docs.push(m_doc);
            }
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(p.client_id.clone()));
            doc.insert("partyId".into(), Value::String(p.client_id.clone()));
            doc.insert("revision".into(), json!(p.revision));
            doc.insert("leaderPlayerId".into(), Value::String(user_id_for(state, &p.leader)));
            doc.insert("buildId".into(), Value::String(p.build_id.clone()));
            doc.insert("activity".into(), Value::String(p.activity.clone()));
            insert_opt_str(&mut doc, "activeHuntSessionId", &p.active_hunt_session_id);
            doc.insert("createdAt".into(), Value::String(p.created_at.clone()));
            doc.insert("updatedAt".into(), Value::String(p.updated_at.clone()));
            doc.insert("members".into(), Value::Array(member_uids));
            party_docs.push(doc);
        }
        out.insert("parties".into(), party_docs);
        out.insert("partyMemberships".into(), membership_docs);

        let mut pi_docs = Vec::with_capacity(state.party_invites.len());
        for pi in state.party_invites.values() {
            let mut doc = Map::new();
            let r_uid = user_id_for(state, &pi.recipient);
            let s_uid = user_id_for(state, &pi.sender);
            doc.insert("_id".into(), Value::String(format!("{r_uid}\u{0000}{s_uid}")));
            doc.insert("recipientPlayerId".into(), Value::String(r_uid));
            doc.insert("sendingPlayerId".into(), Value::String(s_uid));
            doc.insert("partyId".into(), Value::String(party_client_id_for(state, &pi.party)));
            doc.insert("partyRevision".into(), json!(pi.party_revision));
            doc.insert(
                "sendingDisplayName".into(),
                Value::String(pi.sending_display_name.clone()),
            );
            doc.insert("sendingPlatform".into(), Value::String(pi.sending_platform.clone()));
            doc.insert("status".into(), Value::String(pi.status.clone()));
            doc.insert("createdAt".into(), Value::String(pi.created_at.clone()));
            doc.insert("expiresAt".into(), Value::String(pi.expires_at.clone()));
            insert_opt_str(&mut doc, "consumedAt", &pi.consumed_at);
            pi_docs.push(doc);
        }
        out.insert("partyInvites".into(), pi_docs);

        let mut po_docs = Vec::with_capacity(state.party_outbox.len());
        for po in state.party_outbox.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(po.id.clone()));
            doc.insert("eventId".into(), Value::String(po.id.clone()));
            doc.insert("partyId".into(), Value::String(po.party.clone()));
            doc.insert("partyRevision".into(), json!(po.party_revision));
            doc.insert("eventType".into(), Value::String(po.event_type.clone()));
            doc.insert("recipientAccountIds".into(), parse_json_str(&po.recipients, json!([])));
            doc.insert("payload".into(), parse_json_str(&po.payload, json!({})));
            doc.insert("state".into(), Value::String(po.state.clone()));
            doc.insert("createdAt".into(), Value::String(po.created_at.clone()));
            insert_opt_str(&mut doc, "claimedAt", &po.claimed_at);
            insert_opt_str(&mut doc, "claimOwner", &po.claim_owner);
            insert_opt_str(&mut doc, "publishedAt", &po.published_at);
            po_docs.push(doc);
        }
        out.insert("partyOutbox".into(), po_docs);

        let mut fe_docs = Vec::with_capacity(state.friend_edges.len());
        for fe in state.friend_edges.values() {
            let mut doc = Map::new();
            let owner_uid = user_id_for(state, &fe.owner);
            doc.insert("_id".into(), Value::String(format!("{owner_uid}:{}", fe.other_user_id)));
            doc.insert("ownerId".into(), Value::String(owner_uid));
            doc.insert("otherId".into(), Value::String(fe.other_user_id.clone()));
            doc.insert("status".into(), Value::String(fe.status.clone()));
            insert_opt_str(&mut doc, "direction", &fe.direction);
            doc.insert("created".into(), Value::String(fe.created.clone()));
            doc.insert("favorite".into(), Value::Bool(fe.favorite));
            fe_docs.push(doc);
        }
        out.insert("friendships".into(), fe_docs);
    }

    if areas.contains(&MigrateArea::Reforged) {
        let mut ri_docs = Vec::with_capacity(state.reforged_invites.len());
        for ri in state.reforged_invites.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(ri.code.clone()));
            doc.insert("invitecode".into(), Value::String(ri.code.clone()));
            doc.insert("usesRemaining".into(), json!(ri.uses_remaining));
            doc.insert("infiniteUses".into(), Value::Bool(ri.infinite_uses));
            insert_opt_str(&mut doc, "note", &ri.note);
            doc.insert("createdAt".into(), Value::String(ri.created_at.clone()));
            ri_docs.push(doc);
        }
        out.insert("reforgedInvites".into(), ri_docs);

        let mut as_docs = Vec::with_capacity(state.admin_sessions.len());
        for s in state.admin_sessions.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(s.id.clone()));
            doc.insert("id".into(), Value::String(s.id.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &s.account)));
            doc.insert("tokenHash".into(), Value::String(s.token_hash.clone()));
            doc.insert("csrfToken".into(), Value::String(s.csrf_token.clone()));
            doc.insert("createdAt".into(), Value::String(s.created_at.clone()));
            doc.insert("expiresAt".into(), Value::String(s.expires_at.clone()));
            doc.insert("ip".into(), Value::String(s.ip.clone()));
            doc.insert("userAgent".into(), Value::String(s.user_agent.clone()));
            insert_opt_str(&mut doc, "revokedAt", &s.revoked_at);
            doc.insert("ttlAt".into(), json!(s.ttl_at));
            as_docs.push(doc);
        }
        out.insert("adminSessions".into(), as_docs);

        let mut aa_docs = Vec::with_capacity(state.admin_audit.len());
        for a in state.admin_audit.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(a.id.clone()));
            doc.insert("id".into(), Value::String(a.id.clone()));
            doc.insert("actorUserId".into(), Value::String(a.actor_user_id.clone()));
            insert_opt_str(&mut doc, "targetUserId", &a.target_user_id);
            doc.insert("action".into(), Value::String(a.action.clone()));
            if let Some(old_st) = &a.old_state {
                doc.insert("oldState".into(), parse_json_str(old_st, json!({})));
            }
            if let Some(new_st) = &a.new_state {
                doc.insert("newState".into(), parse_json_str(new_st, json!({})));
            }
            insert_opt_str(&mut doc, "reason", &a.reason);
            doc.insert("ip".into(), Value::String(a.ip.clone()));
            doc.insert("requestId".into(), Value::String(a.request_id.clone()));
            doc.insert("createdAt".into(), Value::String(a.created_at.clone()));
            aa_docs.push(doc);
        }
        out.insert("adminAudit".into(), aa_docs);

        let mut ln_docs = Vec::new();
        if let Some(ln) = &state.launcher_notices {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String("current".into()));
            doc.insert("version".into(), json!(ln.version));
            insert_opt_str(&mut doc, "updatedAt", &ln.updated_at);
            insert_opt_str(&mut doc, "updatedBy", &ln.updated_by);
            doc.insert("announcements".into(), parse_json_str(&ln.announcements, json!([])));
            doc.insert("tags".into(), parse_json_str(&ln.tags, json!([])));
            ln_docs.push(doc);
        }
        out.insert("launcherNotices".into(), ln_docs);

        let mut op_docs = Vec::new();
        if let Some(op) = &state.operations_policy {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String("global".into()));
            doc.insert("guardEnforcement".into(), Value::String(op.guard_enforcement.clone()));
            doc.insert(
                "diagnosticsProfile".into(),
                Value::String(op.diagnostics_profile.clone()),
            );
            doc.insert("version".into(), json!(op.version));
            insert_opt_str(&mut doc, "updatedAt", &op.updated_at);
            insert_opt_str(&mut doc, "updatedBy", &op.updated_by);
            insert_opt_str(&mut doc, "diagnosticsExpiresAt", &op.diagnostics_expires_at);
            op_docs.push(doc);
        }
        out.insert("operationsPolicy".into(), op_docs);

        let mut gs_docs = Vec::with_capacity(state.launcher_guard_sessions.len());
        for gs in state.launcher_guard_sessions.values() {
            let mut doc = Map::new();
            doc.insert("_id".into(), Value::String(gs.guard_session_id.clone()));
            doc.insert("guardSessionId".into(), Value::String(gs.guard_session_id.clone()));
            doc.insert("userId".into(), Value::String(user_id_for(state, &gs.account)));
            doc.insert("role".into(), Value::String(gs.role.clone()));
            doc.insert("publicKeyBase64".into(), Value::String(gs.public_key_base64.clone()));
            doc.insert("manifestId".into(), Value::String(gs.manifest_id.clone()));
            doc.insert("manifestSequence".into(), json!(gs.manifest_sequence));
            doc.insert("channel".into(), Value::String(gs.channel.clone()));
            doc.insert("challenge".into(), Value::String(gs.challenge.clone()));
            doc.insert("lastSequence".into(), json!(gs.last_sequence));
            doc.insert("status".into(), Value::String(gs.status.clone()));
            doc.insert("riskScore".into(), json!(gs.risk_score));
            doc.insert("createdAt".into(), Value::String(gs.created_at.clone()));
            doc.insert("expiresAt".into(), Value::String(gs.expires_at.clone()));
            insert_opt_str(&mut doc, "lastHeartbeatAt", &gs.last_heartbeat_at);
            doc.insert("detail".into(), parse_json_str(&gs.detail, json!({})));
            doc.insert("ttlAt".into(), json!(gs.ttl_at));
            gs_docs.push(doc);
        }
        out.insert("launcherGuardSessions".into(), gs_docs);

        let mut gf_docs = Vec::with_capacity(state.guard_findings.len());
        for gf in state.guard_findings.values() {
            let mut doc = Map::new();
            doc.insert("fileName".into(), Value::String(gf.file_name.clone()));
            doc.insert("sha256".into(), Value::String(gf.sha256.clone()));
            doc.insert("signer".into(), Value::String(gf.signer.clone()));
            doc.insert("size".into(), json!(gf.size));
            doc.insert("firstSeenAt".into(), Value::String(gf.first_seen_at.clone()));
            doc.insert("lastSeenAt".into(), Value::String(gf.last_seen_at.clone()));
            doc.insert("sessionCount".into(), json!(gf.session_count));
            doc.insert("accountCount".into(), json!(gf.account_count));
            let parsed_accs = parse_json_str(&gf.account_ids, json!([]));
            let mut uid_arr = Vec::new();
            if let Some(arr) = parsed_accs.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        uid_arr.push(Value::String(user_id_for(state, s)));
                    }
                }
            }
            doc.insert("accountIds".into(), Value::Array(uid_arr));
            doc.insert("ttlAt".into(), json!(gf.ttl_at));
            gf_docs.push(doc);
        }
        out.insert("guardFindings".into(), gf_docs);
    }

    out
}

/// Runs `isles-db export-mongo` end-to-end.
pub async fn run_export_mongo(opts: &ExportOptions) -> Result<ExportReport, DbError> {
    let db = open_target_database(&opts.from_source, opts.allow_remote_postgres)?;
    let state = db
        .read(move |conn| read_target_state(conn, false).map_err(DbError::from))
        .await?;
    let areas = opts.effective_areas();
    let collections = state_to_mongo_collections(&state, &areas);

    let mut report = ExportReport {
        dry_run: opts.dry_run,
        counts: BTreeMap::new(),
    };
    for (col_name, docs) in &collections {
        report.counts.insert(col_name.clone(), docs.len());
    }

    if !opts.dry_run {
        let mut client = MongoClient::connect(&opts.to_uri)?;
        for (col_name, docs) in &collections {
            client.replace_collection(col_name, docs)?;
        }
    }

    Ok(report)
}
