//! `isles-db verify`: compares every MongoDB collection against the target SQLite or Postgres
//! database (per-collection record counts and canonical SHA-256 content hashes).

use std::collections::BTreeMap;

use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use store::AnyConnection;

use crate::migrate::{
    AccountCanon, AdminAuditCanon, AdminSessionCanon, AuthIdentityCanon, BountyStateCanon, BreadcrumbCanon,
    CharacterCanon, CooldownStateCanon, DbError, DiscordOAuthCanon, EconomyCounterCanon, EmailCodeCanon,
    EncounteredContentCanon, EntitlementCanon, EscalationProgressCanon, FriendEdgeCanon, GameExchangeCodeCanon,
    GuardFindingCanon, GuardSessionCanon, InventoryCanon, InventoryTxCanon, InviteCanon, LauncherAccountCanon,
    LauncherNoticesCanon, LoadoutCanon, MigrateArea, MigrateOptions, OperationsPolicyCanon, PartyCanon,
    PartyInviteCanon, PartyOutboxCanon, PlayerJourneyCanon, ProgressionEventCanon, ProgressionGrantCanon,
    ProgressionObjectiveCanon, ProgressionTrackCanon, RefreshSessionCanon, TargetState, WalletCanon,
    extract_from_mongo, load_existing_id_maps, open_target_database,
};
use crate::mongo::MongoClient;
use crate::schema::{
    accounts, admin_audit, admin_sessions, auth_identities, bounty_states, breadcrumbs, characters, cooldown_states,
    discord_oauth_transactions, economy_rollup_counters, email_codes, encountered_content, entitlements,
    escalation_progress, friend_edges, game_exchange_codes, game_server_api_keys, guard_findings, inventories,
    inventory_transactions, launcher_accounts, launcher_guard_sessions, launcher_notices, loadouts, operations_policy,
    parties, party_invites, party_members, party_outbox, player_journey, progression_grants,
    progression_objective_events, progression_objectives, progression_tracks, reforged_invites, refresh_sessions,
    user_api_keys, wallet_balances, wallets,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionVerification {
    pub name: String,
    pub area: MigrateArea,
    pub expected_count: usize,
    pub actual_count: usize,
    pub expected_sha256: String,
    pub actual_sha256: String,
    pub ok: bool,
    pub mismatches: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    pub ok: bool,
    pub collections: Vec<CollectionVerification>,
}

/// Reads the live state of all tables back from the target SQLite or Postgres database.
pub fn read_target_state(
    conn: &mut AnyConnection,
    has_expected_ops_policy: bool,
) -> Result<TargetState, diesel::result::Error> {
    let mut state = TargetState::default();

    // 1. accounts
    let acc_rows = accounts::table
        .filter(accounts::deleted_at.is_null())
        .select((accounts::id, accounts::user_id, accounts::name, accounts::notes))
        .load::<(String, String, String, i64)>(conn)?;
    for (id, user_id, name, notes) in acc_rows {
        state.accounts.insert(
            id.clone(),
            AccountCanon {
                id,
                user_id,
                name,
                notes,
            },
        );
    }

    // 2. launcher_accounts
    type LauncherTuple = (
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let la_rows = launcher_accounts::table
        .filter(launcher_accounts::deleted_at.is_null())
        .select((
            launcher_accounts::account_id,
            launcher_accounts::email,
            launcher_accounts::display_name_normalized,
            launcher_accounts::display_name,
            launcher_accounts::password_hash,
            launcher_accounts::status,
            launcher_accounts::approval_status,
            launcher_accounts::approval_updated_at,
            launcher_accounts::approval_updated_by,
            launcher_accounts::approval_reason,
            launcher_accounts::roles,
            launcher_accounts::roles_updated_at,
            launcher_accounts::roles_updated_by,
            launcher_accounts::created_at,
            launcher_accounts::last_login_at,
            launcher_accounts::username_set,
            launcher_accounts::consent_terms_version,
            launcher_accounts::consent_privacy_version,
            launcher_accounts::consent_accepted_at,
            launcher_accounts::deletion_requested_at,
            launcher_accounts::deletion_scheduled_for,
            launcher_accounts::email_verified_at,
            launcher_accounts::email_verification_pending_since,
        ))
        .load::<LauncherTuple>(conn)?;
    for r in la_rows {
        state.launcher_accounts.insert(
            r.0.clone(),
            LauncherAccountCanon {
                account_id: r.0,
                email: r.1,
                display_name_normalized: r.2,
                display_name: r.3,
                password_hash: r.4,
                status: r.5,
                approval_status: r.6,
                approval_updated_at: r.7,
                approval_updated_by: r.8,
                approval_reason: r.9,
                roles: r.10,
                roles_updated_at: r.11,
                roles_updated_by: r.12,
                created_at: r.13,
                last_login_at: r.14,
                username_set: r.15,
                consent_terms_version: r.16,
                consent_privacy_version: r.17,
                consent_accepted_at: r.18,
                deletion_requested_at: r.19,
                deletion_scheduled_for: r.20,
                email_verified_at: r.21,
                email_verification_pending_since: r.22,
            },
        );
    }

    // 3. auth_identities
    let ai_rows = auth_identities::table
        .select((
            auth_identities::provider,
            auth_identities::provider_subject,
            auth_identities::account_id,
            auth_identities::provider_username,
            auth_identities::provider_avatar_url,
            auth_identities::linked_at,
        ))
        .load::<(String, String, String, String, Option<String>, String)>(conn)?;
    for (provider, provider_subject, account_id, provider_username, provider_avatar_url, linked_at) in ai_rows {
        state.auth_identities.insert(
            (provider.clone(), provider_subject.clone()),
            AuthIdentityCanon {
                provider,
                provider_subject,
                account_id,
                provider_username,
                provider_avatar_url,
                linked_at,
            },
        );
    }

    // 4. refresh_sessions
    let rs_rows = refresh_sessions::table
        .select((
            refresh_sessions::id,
            refresh_sessions::token_hash,
            refresh_sessions::account_id,
            refresh_sessions::family_id,
            refresh_sessions::device_id,
            refresh_sessions::device_name,
            refresh_sessions::created_at,
            refresh_sessions::expires_at,
            refresh_sessions::revoked_at,
        ))
        .load::<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
        )>(conn)?;
    for (id, token_hash, account_id, family_id, device_id, device_name, created_at, expires_at, revoked_at) in rs_rows {
        state.refresh_sessions.insert(
            id.clone(),
            RefreshSessionCanon {
                id,
                token_hash,
                account_id,
                family_id,
                device_id,
                device_name,
                created_at,
                expires_at,
                revoked_at,
            },
        );
    }

    // 5. game_exchange_codes
    type GecTuple = (
        String,
        String,
        String,
        i64,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
    );
    let gec_rows = game_exchange_codes::table
        .select((
            game_exchange_codes::code_hash,
            game_exchange_codes::account_id,
            game_exchange_codes::launcher_session_id,
            game_exchange_codes::build_changelist,
            game_exchange_codes::executable_sha256,
            game_exchange_codes::runtime_channel,
            game_exchange_codes::runtime_sha256,
            game_exchange_codes::runtime_manifest_version,
            game_exchange_codes::runtime_artifact_set_sha256,
            game_exchange_codes::realm_id,
            game_exchange_codes::created_at,
            game_exchange_codes::expires_at,
            game_exchange_codes::consumed_at,
        ))
        .load::<GecTuple>(conn)?;
    for r in gec_rows {
        state.game_exchange_codes.insert(
            r.0.clone(),
            GameExchangeCodeCanon {
                code_hash: r.0,
                account_id: r.1,
                launcher_session_id: r.2,
                build_changelist: r.3,
                executable_sha256: r.4,
                runtime_channel: r.5,
                runtime_sha256: r.6,
                runtime_manifest_version: r.7,
                runtime_artifact_set_sha256: r.8,
                realm_id: r.9,
                created_at: r.10,
                expires_at: r.11,
                consumed_at: r.12,
            },
        );
    }

    // 6. discord_oauth_transactions
    type DiscordTuple = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let do_rows = discord_oauth_transactions::table
        .select((
            discord_oauth_transactions::state,
            discord_oauth_transactions::code_verifier,
            discord_oauth_transactions::created_at,
            discord_oauth_transactions::expires_at,
            discord_oauth_transactions::consumed_at,
            discord_oauth_transactions::completion_code_hash,
            discord_oauth_transactions::completion_expires_at,
            discord_oauth_transactions::completion_consumed_at,
            discord_oauth_transactions::account_id,
            discord_oauth_transactions::link_account_id,
        ))
        .load::<DiscordTuple>(conn)?;
    for r in do_rows {
        state.discord_oauth_transactions.insert(
            r.0.clone(),
            DiscordOAuthCanon {
                state: r.0,
                code_verifier: r.1,
                created_at: r.2,
                expires_at: r.3,
                consumed_at: r.4,
                completion_code_hash: r.5,
                completion_expires_at: r.6,
                completion_consumed_at: r.7,
                account_id: r.8,
                link_account_id: r.9,
            },
        );
    }

    // 7. email_codes
    let ec_rows = email_codes::table
        .select((
            email_codes::account_id,
            email_codes::purpose,
            email_codes::code_hash,
            email_codes::attempts,
            email_codes::created_at,
            email_codes::expires_at,
        ))
        .load::<(String, String, String, i64, String, String)>(conn)?;
    for (account_id, purpose, code_hash, attempts, created_at, expires_at) in ec_rows {
        state.email_codes.insert(
            (account_id.clone(), purpose.clone()),
            EmailCodeCanon {
                account_id,
                purpose,
                code_hash,
                attempts,
                created_at,
                expires_at,
            },
        );
    }

    // 8. API keys
    let gs_keys = game_server_api_keys::table
        .select(game_server_api_keys::key_hash)
        .load::<String>(conn)?;
    for k in gs_keys {
        state.game_server_api_keys.insert(k);
    }
    let u_keys = user_api_keys::table
        .select((user_api_keys::account_id, user_api_keys::key_hash))
        .load::<(String, String)>(conn)?;
    for (acc, k) in u_keys {
        state.user_api_keys.insert(acc, k);
    }

    // 9. characters & loadouts
    let char_rows = characters::table
        .filter(characters::deleted_at.is_null())
        .select((
            characters::character_id,
            characters::account_id,
            characters::ord,
            characters::created_date,
            characters::last_modified_date,
            characters::name,
            characters::update_version,
            characters::data,
        ))
        .load::<(String, String, i64, String, String, String, i64, String)>(conn)?;
    for (character_id, account_id, ord, created_date, last_modified_date, name, update_version, data) in char_rows {
        state.characters.insert(
            character_id.clone(),
            CharacterCanon {
                character_id,
                account_id,
                ord,
                created_date,
                last_modified_date,
                name,
                update_version,
                data,
            },
        );
    }

    let ld_rows = loadouts::table
        .filter(loadouts::deleted_at.is_null())
        .select((
            loadouts::character_id,
            loadouts::account_id,
            loadouts::slots_json,
            loadouts::persistent,
            loadouts::active_index,
            loadouts::unlocked_total_slots,
            loadouts::revision,
            loadouts::bootstrap_version,
        ))
        .load::<(String, String, String, String, i64, Option<i64>, i64, Option<String>)>(conn)?;
    for (
        character_id,
        account_id,
        slots_json,
        persistent,
        active_index,
        unlocked_total_slots,
        revision,
        bootstrap_version,
    ) in ld_rows
    {
        state.loadouts.insert(
            character_id.clone(),
            LoadoutCanon {
                character_id,
                account_id,
                slots_json,
                persistent,
                active_index,
                unlocked_total_slots,
                revision,
                bootstrap_version,
            },
        );
    }

    // 10. inventories & inventory_transactions
    let inv_rows = inventories::table
        .filter(inventories::deleted_at.is_null())
        .select((
            inventories::character_id,
            inventories::account_id,
            inventories::instanced_items,
            inventories::stacked_items,
            inventories::revision,
            inventories::bootstrap_version,
        ))
        .load::<(String, Option<String>, String, String, i64, Option<String>)>(conn)?;
    for (character_id, account_id, instanced_items, stacked_items, revision, bootstrap_version) in inv_rows {
        state.inventories.insert(
            character_id.clone(),
            InventoryCanon {
                character_id,
                account_id,
                instanced_items,
                stacked_items,
                revision,
                bootstrap_version,
            },
        );
    }

    type InvTxTuple = (
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let itx_rows = inventory_transactions::table
        .select((
            inventory_transactions::account_id,
            inventory_transactions::character_id,
            inventory_transactions::transaction_id,
            inventory_transactions::request_hash,
            inventory_transactions::status,
            inventory_transactions::result,
            inventory_transactions::created_at,
            inventory_transactions::completed_at,
            inventory_transactions::economy_source,
            inventory_transactions::economy_session_id,
            inventory_transactions::extra,
        ))
        .load::<InvTxTuple>(conn)?;
    for r in itx_rows {
        state.inventory_transactions.insert(
            (r.0.clone(), r.1.clone(), r.2.clone()),
            InventoryTxCanon {
                account_id: r.0,
                character_id: r.1,
                transaction_id: r.2,
                request_hash: r.3,
                status: r.4,
                result: r.5,
                created_at: r.6,
                completed_at: r.7,
                economy_source: r.8,
                economy_session_id: r.9,
                extra: r.10,
            },
        );
    }

    // 11. bounties
    let bs_rows = bounty_states::table
        .filter(bounty_states::deleted_at.is_null())
        .select((
            bounty_states::account_id,
            bounty_states::bounties,
            bounty_states::draft_data,
            bounty_states::daily_reward_credited_on,
            bounty_states::daily_premium_draft_granted_on,
            bounty_states::update_version,
            bounty_states::created_at,
            bounty_states::updated_at,
        ))
        .load::<(
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
            String,
            String,
        )>(conn)?;
    for (
        account_id,
        bounties,
        draft_data,
        daily_reward_credited_on,
        daily_premium_draft_granted_on,
        update_version,
        created_at,
        updated_at,
    ) in bs_rows
    {
        state.bounty_states.insert(
            account_id.clone(),
            BountyStateCanon {
                account_id,
                bounties,
                draft_data,
                daily_reward_credited_on,
                daily_premium_draft_granted_on,
                update_version,
                created_at,
                updated_at,
            },
        );
    }

    let cs_rows = cooldown_states::table
        .filter(cooldown_states::deleted_at.is_null())
        .select((
            cooldown_states::account_id,
            cooldown_states::entries,
            cooldown_states::update_version,
            cooldown_states::created_at,
            cooldown_states::updated_at,
        ))
        .load::<(String, String, i64, String, String)>(conn)?;
    for (account_id, entries, update_version, created_at, updated_at) in cs_rows {
        state.cooldown_states.insert(
            account_id.clone(),
            CooldownStateCanon {
                account_id,
                entries,
                update_version,
                created_at,
                updated_at,
            },
        );
    }

    let ent_rows = entitlements::table
        .filter(entitlements::deleted_at.is_null())
        .select((
            entitlements::account_id,
            entitlements::name,
            entitlements::duration,
            entitlements::activated_date,
            entitlements::granted_at,
            entitlements::source_sku_id,
        ))
        .load::<(String, String, f64, Option<String>, Option<String>, Option<String>)>(conn)?;
    for (account_id, name, duration, activated_date, granted_at, source_sku_id) in ent_rows {
        state.entitlements.insert(
            (account_id.clone(), name.clone()),
            EntitlementCanon {
                account_id,
                name,
                duration,
                activated_date,
                granted_at,
                source_sku_id,
            },
        );
    }

    let w_rows = wallets::table
        .filter(wallets::deleted_at.is_null())
        .select((wallets::account_id, wallets::bootstrap_version))
        .load::<(String, Option<String>)>(conn)?;
    let wb_rows = wallet_balances::table
        .filter(wallet_balances::deleted_at.is_null())
        .order((wallet_balances::account_id.asc(), wallet_balances::ordinal.asc()))
        .select((
            wallet_balances::account_id,
            wallet_balances::catalog_id,
            wallet_balances::balance,
            wallet_balances::ordinal,
        ))
        .load::<(String, String, f64, i64)>(conn)?;
    for (account_id, bootstrap_version) in w_rows {
        state.wallets.insert(
            account_id.clone(),
            WalletCanon {
                account_id,
                bootstrap_version,
                balances: Vec::new(),
            },
        );
    }
    for (account_id, cid, balance, ord) in wb_rows {
        if let Some(w) = state.wallets.get_mut(&account_id) {
            w.balances.push((cid, balance, ord));
        }
    }

    let ec_rollup_rows = economy_rollup_counters::table
        .select((
            economy_rollup_counters::account_id,
            economy_rollup_counters::day,
            economy_rollup_counters::currency,
            economy_rollup_counters::path,
            economy_rollup_counters::value,
            economy_rollup_counters::ttl_at,
        ))
        .load::<(String, String, String, String, f64, i64)>(conn)?;
    for (account_id, day, currency, path, value, ttl_at) in ec_rollup_rows {
        state.economy_rollup_counters.insert(
            (account_id.clone(), day.clone(), currency.clone(), path.clone()),
            EconomyCounterCanon {
                account_id,
                day,
                currency,
                path,
                value,
                ttl_at,
            },
        );
    }

    // 12. progression
    let pt_rows = progression_tracks::table
        .filter(progression_tracks::deleted_at.is_null())
        .select((
            progression_tracks::account,
            progression_tracks::progression_id,
            progression_tracks::progress,
            progression_tracks::confirmed_fremium_rank,
            progression_tracks::confirmed_premium_rank,
            progression_tracks::update_version,
            progression_tracks::created_at,
            progression_tracks::updated_at,
            progression_tracks::reforge,
            progression_tracks::prestige_claimed,
        ))
        .load::<(
            String,
            String,
            i64,
            i64,
            i64,
            i64,
            String,
            String,
            Option<String>,
            Option<i64>,
        )>(conn)?;
    for (
        account,
        progression_id,
        progress,
        confirmed_fremium_rank,
        confirmed_premium_rank,
        update_version,
        created_at,
        updated_at,
        reforge,
        prestige_claimed,
    ) in pt_rows
    {
        state.progression_tracks.insert(
            (account.clone(), progression_id.clone()),
            ProgressionTrackCanon {
                account,
                progression_id,
                progress,
                confirmed_fremium_rank,
                confirmed_premium_rank,
                update_version,
                created_at,
                updated_at,
                reforge,
                prestige_claimed,
            },
        );
    }

    let po_rows = progression_objectives::table
        .filter(progression_objectives::deleted_at.is_null())
        .select((
            progression_objectives::account,
            progression_objectives::objective_id,
            progression_objectives::value,
            progression_objectives::completed_count,
            progression_objectives::created_at,
            progression_objectives::updated_at,
        ))
        .load::<(String, String, i64, i64, String, String)>(conn)?;
    for (account, objective_id, value, completed_count, created_at, updated_at) in po_rows {
        state.progression_objectives.insert(
            (account.clone(), objective_id.clone()),
            ProgressionObjectiveCanon {
                account,
                objective_id,
                value,
                completed_count,
                created_at,
                updated_at,
            },
        );
    }

    let poe_rows = progression_objective_events::table
        .select((
            progression_objective_events::id,
            progression_objective_events::account,
            progression_objective_events::raw_body,
            progression_objective_events::received_at,
        ))
        .load::<(String, String, String, String)>(conn)?;
    for (id, account, raw_body, received_at) in poe_rows {
        state.progression_objective_events.insert(
            id.clone(),
            ProgressionEventCanon {
                id,
                account,
                raw_body,
                received_at,
            },
        );
    }

    let pg_rows = progression_grants::table
        .select((
            progression_grants::grant_id,
            progression_grants::account,
            progression_grants::progression_id,
            progression_grants::amount,
            progression_grants::request_hash,
            progression_grants::status,
            progression_grants::result,
            progression_grants::created_at,
        ))
        .load::<(String, String, String, i64, String, String, String, String)>(conn)?;
    for (grant_id, account, progression_id, amount, request_hash, status, result, created_at) in pg_rows {
        state.progression_grants.insert(
            grant_id.clone(),
            ProgressionGrantCanon {
                grant_id,
                account,
                progression_id,
                amount,
                request_hash,
                status,
                result,
                created_at,
            },
        );
    }

    let ep_rows = escalation_progress::table
        .filter(escalation_progress::deleted_at.is_null())
        .select((
            escalation_progress::account,
            escalation_progress::season_id,
            escalation_progress::escalation_level,
            escalation_progress::next_level_xp,
            escalation_progress::talents_progress,
            escalation_progress::unlock_progress,
            escalation_progress::update_version,
            escalation_progress::created_at,
            escalation_progress::updated_at,
        ))
        .load::<(String, String, i64, i64, String, String, i64, String, String)>(conn)?;
    for (
        account,
        season_id,
        escalation_level,
        next_level_xp,
        talents_progress,
        unlock_progress,
        update_version,
        created_at,
        updated_at,
    ) in ep_rows
    {
        state.escalation_progress.insert(
            (account.clone(), season_id.clone()),
            EscalationProgressCanon {
                account,
                season_id,
                escalation_level,
                next_level_xp,
                talents_progress,
                unlock_progress,
                update_version,
                created_at,
                updated_at,
            },
        );
    }

    let pj_rows = player_journey::table
        .filter(player_journey::deleted_at.is_null())
        .select((
            player_journey::account,
            player_journey::nodes,
            player_journey::update_version,
            player_journey::revision,
        ))
        .load::<(String, String, i64, i64)>(conn)?;
    for (account, nodes, update_version, revision) in pj_rows {
        state.player_journey.insert(
            account.clone(),
            PlayerJourneyCanon {
                account,
                nodes,
                update_version,
                revision,
            },
        );
    }

    let bc_rows = breadcrumbs::table
        .filter(breadcrumbs::deleted_at.is_null())
        .select((
            breadcrumbs::character_id,
            breadcrumbs::account,
            breadcrumbs::items,
            breadcrumbs::update_version,
        ))
        .load::<(String, String, String, i64)>(conn)?;
    for (character_id, account, items, update_version) in bc_rows {
        state.breadcrumbs.insert(
            character_id.clone(),
            BreadcrumbCanon {
                character_id,
                account,
                items,
                update_version,
            },
        );
    }

    let ec_content_rows = encountered_content::table
        .filter(encountered_content::deleted_at.is_null())
        .select((
            encountered_content::character_id,
            encountered_content::account,
            encountered_content::content,
        ))
        .load::<(String, String, String)>(conn)?;
    for (character_id, account, content) in ec_content_rows {
        state.encountered_content.insert(
            character_id.clone(),
            EncounteredContentCanon {
                character_id,
                account,
                content,
            },
        );
    }

    // 13. parties & friendships
    let p_rows = parties::table
        .select((
            parties::id,
            parties::client_id,
            parties::revision,
            parties::leader,
            parties::build_id,
            parties::activity,
            parties::active_hunt_session_id,
            parties::created_at,
            parties::updated_at,
        ))
        .load::<(
            String,
            String,
            i64,
            String,
            String,
            String,
            Option<String>,
            String,
            String,
        )>(conn)?;
    let pm_rows = party_members::table
        .order((party_members::party.asc(), party_members::position.asc()))
        .select((
            party_members::account,
            party_members::party,
            party_members::position,
            party_members::joined_at,
        ))
        .load::<(String, String, i64, String)>(conn)?;
    for (id, client_id, revision, leader, build_id, activity, active_hunt_session_id, created_at, updated_at) in p_rows
    {
        state.parties.insert(
            id.clone(),
            PartyCanon {
                id,
                client_id,
                revision,
                leader,
                build_id,
                activity,
                active_hunt_session_id,
                created_at,
                updated_at,
                members: Vec::new(),
            },
        );
    }
    for (account, party_id, pos, joined_at) in pm_rows {
        if let Some(p) = state.parties.get_mut(&party_id) {
            p.members.push((account, pos, joined_at));
        }
    }

    let pi_rows = party_invites::table
        .select((
            party_invites::recipient,
            party_invites::sender,
            party_invites::party,
            party_invites::party_revision,
            party_invites::sending_display_name,
            party_invites::sending_platform,
            party_invites::status,
            party_invites::created_at,
            party_invites::expires_at,
            party_invites::consumed_at,
        ))
        .load::<(
            String,
            String,
            String,
            i64,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
        )>(conn)?;
    for (
        recipient,
        sender,
        party,
        party_revision,
        sending_display_name,
        sending_platform,
        status,
        created_at,
        expires_at,
        consumed_at,
    ) in pi_rows
    {
        state.party_invites.insert(
            (recipient.clone(), sender.clone()),
            PartyInviteCanon {
                recipient,
                sender,
                party,
                party_revision,
                sending_display_name,
                sending_platform,
                status,
                created_at,
                expires_at,
                consumed_at,
            },
        );
    }

    let po_outbox_rows = party_outbox::table
        .select((
            party_outbox::id,
            party_outbox::party,
            party_outbox::party_revision,
            party_outbox::event_type,
            party_outbox::recipients,
            party_outbox::payload,
            party_outbox::state,
            party_outbox::created_at,
            party_outbox::claimed_at,
            party_outbox::claim_owner,
            party_outbox::published_at,
        ))
        .load::<(
            String,
            String,
            i64,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        )>(conn)?;
    for (
        id,
        party,
        party_revision,
        event_type,
        recipients,
        payload,
        st,
        created_at,
        claimed_at,
        claim_owner,
        published_at,
    ) in po_outbox_rows
    {
        state.party_outbox.insert(
            id.clone(),
            PartyOutboxCanon {
                id,
                party,
                party_revision,
                event_type,
                recipients,
                payload,
                state: st,
                created_at,
                claimed_at,
                claim_owner,
                published_at,
            },
        );
    }

    let fe_rows = friend_edges::table
        .select((
            friend_edges::owner,
            friend_edges::other,
            friend_edges::other_user_id,
            friend_edges::status,
            friend_edges::direction,
            friend_edges::created,
            friend_edges::favorite,
        ))
        .load::<(String, String, String, String, Option<String>, String, bool)>(conn)?;
    for (owner, other, other_user_id, status, direction, created, favorite) in fe_rows {
        state.friend_edges.insert(
            (owner.clone(), other.clone()),
            FriendEdgeCanon {
                owner,
                other,
                other_user_id,
                status,
                direction,
                created,
                favorite,
            },
        );
    }

    // 14. reforged
    let ri_rows = reforged_invites::table
        .select((
            reforged_invites::code,
            reforged_invites::uses_remaining,
            reforged_invites::infinite_uses,
            reforged_invites::note,
            reforged_invites::created_at,
        ))
        .load::<(String, i64, bool, Option<String>, String)>(conn)?;
    for (code, uses_remaining, infinite_uses, note, created_at) in ri_rows {
        state.reforged_invites.insert(
            code.clone(),
            InviteCanon {
                code,
                uses_remaining,
                infinite_uses,
                note,
                created_at,
            },
        );
    }

    let as_rows = admin_sessions::table
        .select((
            admin_sessions::id,
            admin_sessions::token_hash,
            admin_sessions::account,
            admin_sessions::csrf_token,
            admin_sessions::created_at,
            admin_sessions::expires_at,
            admin_sessions::ip,
            admin_sessions::user_agent,
            admin_sessions::revoked_at,
            admin_sessions::ttl_at,
        ))
        .load::<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            i64,
        )>(conn)?;
    for (id, token_hash, account, csrf_token, created_at, expires_at, ip, user_agent, revoked_at, ttl_at) in as_rows {
        state.admin_sessions.insert(
            id.clone(),
            AdminSessionCanon {
                id,
                token_hash,
                account,
                csrf_token,
                created_at,
                expires_at,
                ip,
                user_agent,
                revoked_at,
                ttl_at,
            },
        );
    }

    let aa_rows = admin_audit::table
        .select((
            admin_audit::id,
            admin_audit::actor_user_id,
            admin_audit::target_user_id,
            admin_audit::action,
            admin_audit::old_state,
            admin_audit::new_state,
            admin_audit::reason,
            admin_audit::ip,
            admin_audit::request_id,
            admin_audit::created_at,
        ))
        .load::<(
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            String,
            String,
        )>(conn)?;
    for (id, actor_user_id, target_user_id, action, old_state, new_state, reason, ip, request_id, created_at) in aa_rows
    {
        state.admin_audit.insert(
            id.clone(),
            AdminAuditCanon {
                id,
                actor_user_id,
                target_user_id,
                action,
                old_state,
                new_state,
                reason,
                ip,
                request_id,
                created_at,
            },
        );
    }

    let ln_row = launcher_notices::table
        .select((
            launcher_notices::id,
            launcher_notices::version,
            launcher_notices::updated_at,
            launcher_notices::updated_by,
            launcher_notices::announcements,
            launcher_notices::tags,
        ))
        .first::<(i64, i64, Option<String>, Option<String>, String, String)>(conn)
        .optional()?;
    if let Some((id, version, updated_at, updated_by, announcements, tags)) = ln_row {
        state.launcher_notices = Some(LauncherNoticesCanon {
            id,
            version,
            updated_at,
            updated_by,
            announcements,
            tags,
        });
    }

    let op_row = operations_policy::table
        .select((
            operations_policy::id,
            operations_policy::guard_enforcement,
            operations_policy::diagnostics_profile,
            operations_policy::version,
            operations_policy::updated_at,
            operations_policy::updated_by,
            operations_policy::diagnostics_expires_at,
        ))
        .first::<(i64, String, String, i64, Option<String>, Option<String>, Option<String>)>(conn)
        .optional()?;
    if let Some((id, guard_enforcement, diagnostics_profile, version, updated_at, updated_by, diagnostics_expires_at)) =
        op_row
    {
        let is_unmodified_seed = !has_expected_ops_policy
            && id == 1
            && guard_enforcement == "ENFORCE"
            && diagnostics_profile == "PRODUCTION"
            && version == 0
            && updated_at.is_none()
            && updated_by.is_none()
            && diagnostics_expires_at.is_none();
        if !is_unmodified_seed {
            state.operations_policy = Some(OperationsPolicyCanon {
                id,
                guard_enforcement,
                diagnostics_profile,
                version,
                updated_at,
                updated_by,
                diagnostics_expires_at,
            });
        }
    }

    type GsTuple = (
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
        String,
        i64,
        String,
        f64,
        String,
        String,
        Option<String>,
        String,
        i64,
    );
    let gs_rows = launcher_guard_sessions::table
        .select((
            launcher_guard_sessions::guard_session_id,
            launcher_guard_sessions::account,
            launcher_guard_sessions::role,
            launcher_guard_sessions::public_key_base64,
            launcher_guard_sessions::manifest_id,
            launcher_guard_sessions::manifest_sequence,
            launcher_guard_sessions::channel,
            launcher_guard_sessions::challenge,
            launcher_guard_sessions::last_sequence,
            launcher_guard_sessions::status,
            launcher_guard_sessions::risk_score,
            launcher_guard_sessions::created_at,
            launcher_guard_sessions::expires_at,
            launcher_guard_sessions::last_heartbeat_at,
            launcher_guard_sessions::detail,
            launcher_guard_sessions::ttl_at,
        ))
        .load::<GsTuple>(conn)?;
    for r in gs_rows {
        state.launcher_guard_sessions.insert(
            r.0.clone(),
            GuardSessionCanon {
                guard_session_id: r.0,
                account: r.1,
                role: r.2,
                public_key_base64: r.3,
                manifest_id: r.4,
                manifest_sequence: r.5,
                channel: r.6,
                challenge: r.7,
                last_sequence: r.8,
                status: r.9,
                risk_score: r.10,
                created_at: r.11,
                expires_at: r.12,
                last_heartbeat_at: r.13,
                detail: r.14,
                ttl_at: r.15,
            },
        );
    }

    let gf_rows = guard_findings::table
        .select((
            guard_findings::file_name,
            guard_findings::sha256,
            guard_findings::signer,
            guard_findings::size,
            guard_findings::first_seen_at,
            guard_findings::last_seen_at,
            guard_findings::session_count,
            guard_findings::account_count,
            guard_findings::account_ids,
            guard_findings::ttl_at,
        ))
        .load::<(String, String, String, i64, String, String, i64, i64, String, i64)>(conn)?;
    for (
        file_name,
        sha256,
        signer,
        size,
        first_seen_at,
        last_seen_at,
        session_count,
        account_count,
        account_ids,
        ttl_at,
    ) in gf_rows
    {
        state.guard_findings.insert(
            (file_name.clone(), sha256.clone()),
            GuardFindingCanon {
                file_name,
                sha256,
                signer,
                size,
                first_seen_at,
                last_seen_at,
                session_count,
                account_count,
                account_ids,
                ttl_at,
            },
        );
    }

    Ok(state)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn compare_keyed_collection<V: Serialize>(
    name: &str,
    area: MigrateArea,
    expected: &BTreeMap<String, V>,
    actual: &BTreeMap<String, V>,
) -> CollectionVerification {
    let mut exp_hasher = Sha256::new();
    let mut exp_hashes = BTreeMap::new();
    for (k, v) in expected {
        let json = serde_json::to_string(v).unwrap_or_default();
        let h = sha256_hex(json.as_bytes());
        exp_hasher.update(k.as_bytes());
        exp_hasher.update(b":");
        exp_hasher.update(h.as_bytes());
        exp_hasher.update(b"\n");
        exp_hashes.insert(k.clone(), h);
    }

    let mut act_hasher = Sha256::new();
    let mut act_hashes = BTreeMap::new();
    for (k, v) in actual {
        let json = serde_json::to_string(v).unwrap_or_default();
        let h = sha256_hex(json.as_bytes());
        act_hasher.update(k.as_bytes());
        act_hasher.update(b":");
        act_hasher.update(h.as_bytes());
        act_hasher.update(b"\n");
        act_hashes.insert(k.clone(), h);
    }

    let expected_sha256 = sha256_hex(&exp_hasher.finalize());
    let actual_sha256 = sha256_hex(&act_hasher.finalize());
    let mut mismatches = Vec::new();

    for (k, exp_h) in &exp_hashes {
        match act_hashes.get(k) {
            None => mismatches.push(format!("missing record {k}")),
            Some(act_h) if act_h != exp_h => {
                mismatches.push(format!("content hash mismatch for {k}: expected {exp_h}, got {act_h}"))
            }
            Some(_) => {}
        }
    }
    for k in act_hashes.keys() {
        if !exp_hashes.contains_key(k) {
            mismatches.push(format!("unexpected extra record {k}"));
        }
    }

    CollectionVerification {
        name: name.to_owned(),
        area,
        expected_count: expected.len(),
        actual_count: actual.len(),
        ok: expected.len() == actual.len() && expected_sha256 == actual_sha256 && mismatches.is_empty(),
        expected_sha256,
        actual_sha256,
        mismatches,
    }
}

fn map_tuple2<V: Clone>(src: &BTreeMap<(String, String), V>) -> BTreeMap<String, V> {
    src.iter().map(|((a, b), v)| (format!("{a}:{b}"), v.clone())).collect()
}

fn map_tuple3<V: Clone>(src: &BTreeMap<(String, String, String), V>) -> BTreeMap<String, V> {
    src.iter()
        .map(|((a, b, c), v)| (format!("{a}:{b}:{c}"), v.clone()))
        .collect()
}

fn map_tuple4<V: Clone>(src: &BTreeMap<(String, String, String, String), V>) -> BTreeMap<String, V> {
    src.iter()
        .map(|((a, b, c, d), v)| (format!("{a}:{b}:{c}:{d}"), v.clone()))
        .collect()
}

/// Compares `expected` and `actual` states across all selected areas.
pub fn compare_states(expected: &TargetState, actual: &TargetState, opts: &MigrateOptions) -> VerifyReport {
    let areas = opts.effective_areas();
    let mut cols = Vec::new();

    if areas.contains(&MigrateArea::Auth) {
        let act_accounts: BTreeMap<String, AccountCanon> = if opts.areas.is_empty() {
            actual.accounts.clone()
        } else {
            actual
                .accounts
                .iter()
                .filter(|(k, _)| expected.accounts.contains_key(*k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };
        cols.push(compare_keyed_collection(
            "accounts",
            MigrateArea::Auth,
            &expected.accounts,
            &act_accounts,
        ));
        cols.push(compare_keyed_collection(
            "launcher_accounts",
            MigrateArea::Auth,
            &expected.launcher_accounts,
            &actual.launcher_accounts,
        ));
        cols.push(compare_keyed_collection(
            "auth_identities",
            MigrateArea::Auth,
            &map_tuple2(&expected.auth_identities),
            &map_tuple2(&actual.auth_identities),
        ));
        cols.push(compare_keyed_collection(
            "refresh_sessions",
            MigrateArea::Auth,
            &expected.refresh_sessions,
            &actual.refresh_sessions,
        ));
        cols.push(compare_keyed_collection(
            "game_exchange_codes",
            MigrateArea::Auth,
            &expected.game_exchange_codes,
            &actual.game_exchange_codes,
        ));
        cols.push(compare_keyed_collection(
            "discord_oauth_transactions",
            MigrateArea::Auth,
            &expected.discord_oauth_transactions,
            &actual.discord_oauth_transactions,
        ));
        cols.push(compare_keyed_collection(
            "email_codes",
            MigrateArea::Auth,
            &map_tuple2(&expected.email_codes),
            &map_tuple2(&actual.email_codes),
        ));
        let exp_gs: BTreeMap<String, String> = expected
            .game_server_api_keys
            .iter()
            .map(|k| (k.clone(), k.clone()))
            .collect();
        let act_gs: BTreeMap<String, String> = actual
            .game_server_api_keys
            .iter()
            .map(|k| (k.clone(), k.clone()))
            .collect();
        cols.push(compare_keyed_collection(
            "game_server_api_keys",
            MigrateArea::Auth,
            &exp_gs,
            &act_gs,
        ));
        cols.push(compare_keyed_collection(
            "user_api_keys",
            MigrateArea::Auth,
            &expected.user_api_keys,
            &actual.user_api_keys,
        ));
    }

    if areas.contains(&MigrateArea::Characters) {
        cols.push(compare_keyed_collection(
            "characters",
            MigrateArea::Characters,
            &expected.characters,
            &actual.characters,
        ));
        cols.push(compare_keyed_collection(
            "loadouts",
            MigrateArea::Characters,
            &expected.loadouts,
            &actual.loadouts,
        ));
    }

    if areas.contains(&MigrateArea::Inventory) {
        cols.push(compare_keyed_collection(
            "inventories",
            MigrateArea::Inventory,
            &expected.inventories,
            &actual.inventories,
        ));
        cols.push(compare_keyed_collection(
            "inventory_transactions",
            MigrateArea::Inventory,
            &map_tuple3(&expected.inventory_transactions),
            &map_tuple3(&actual.inventory_transactions),
        ));
    }

    if areas.contains(&MigrateArea::Progression) {
        cols.push(compare_keyed_collection(
            "progression_tracks",
            MigrateArea::Progression,
            &map_tuple2(&expected.progression_tracks),
            &map_tuple2(&actual.progression_tracks),
        ));
        cols.push(compare_keyed_collection(
            "progression_objectives",
            MigrateArea::Progression,
            &map_tuple2(&expected.progression_objectives),
            &map_tuple2(&actual.progression_objectives),
        ));
        cols.push(compare_keyed_collection(
            "progression_objective_events",
            MigrateArea::Progression,
            &expected.progression_objective_events,
            &actual.progression_objective_events,
        ));
        cols.push(compare_keyed_collection(
            "progression_grants",
            MigrateArea::Progression,
            &expected.progression_grants,
            &actual.progression_grants,
        ));
        cols.push(compare_keyed_collection(
            "escalation_progress",
            MigrateArea::Progression,
            &map_tuple2(&expected.escalation_progress),
            &map_tuple2(&actual.escalation_progress),
        ));
        cols.push(compare_keyed_collection(
            "player_journey",
            MigrateArea::Progression,
            &expected.player_journey,
            &actual.player_journey,
        ));
        cols.push(compare_keyed_collection(
            "breadcrumbs",
            MigrateArea::Progression,
            &expected.breadcrumbs,
            &actual.breadcrumbs,
        ));
        cols.push(compare_keyed_collection(
            "encountered_content",
            MigrateArea::Progression,
            &expected.encountered_content,
            &actual.encountered_content,
        ));
    }

    if areas.contains(&MigrateArea::Bounties) {
        cols.push(compare_keyed_collection(
            "bounty_states",
            MigrateArea::Bounties,
            &expected.bounty_states,
            &actual.bounty_states,
        ));
        cols.push(compare_keyed_collection(
            "cooldown_states",
            MigrateArea::Bounties,
            &expected.cooldown_states,
            &actual.cooldown_states,
        ));
        cols.push(compare_keyed_collection(
            "entitlements",
            MigrateArea::Bounties,
            &map_tuple2(&expected.entitlements),
            &map_tuple2(&actual.entitlements),
        ));
        cols.push(compare_keyed_collection(
            "wallets",
            MigrateArea::Bounties,
            &expected.wallets,
            &actual.wallets,
        ));
        cols.push(compare_keyed_collection(
            "economy_rollup_counters",
            MigrateArea::Bounties,
            &map_tuple4(&expected.economy_rollup_counters),
            &map_tuple4(&actual.economy_rollup_counters),
        ));
    }

    if areas.contains(&MigrateArea::Parties) {
        cols.push(compare_keyed_collection(
            "parties",
            MigrateArea::Parties,
            &expected.parties,
            &actual.parties,
        ));
        cols.push(compare_keyed_collection(
            "party_invites",
            MigrateArea::Parties,
            &map_tuple2(&expected.party_invites),
            &map_tuple2(&actual.party_invites),
        ));
        cols.push(compare_keyed_collection(
            "party_outbox",
            MigrateArea::Parties,
            &expected.party_outbox,
            &actual.party_outbox,
        ));
        cols.push(compare_keyed_collection(
            "friend_edges",
            MigrateArea::Parties,
            &map_tuple2(&expected.friend_edges),
            &map_tuple2(&actual.friend_edges),
        ));
    }

    if areas.contains(&MigrateArea::Reforged) {
        cols.push(compare_keyed_collection(
            "reforged_invites",
            MigrateArea::Reforged,
            &expected.reforged_invites,
            &actual.reforged_invites,
        ));
        cols.push(compare_keyed_collection(
            "admin_sessions",
            MigrateArea::Reforged,
            &expected.admin_sessions,
            &actual.admin_sessions,
        ));
        cols.push(compare_keyed_collection(
            "admin_audit",
            MigrateArea::Reforged,
            &expected.admin_audit,
            &actual.admin_audit,
        ));

        let mut exp_ln = BTreeMap::new();
        if let Some(ln) = &expected.launcher_notices {
            exp_ln.insert("current".into(), ln.clone());
        }
        let mut act_ln = BTreeMap::new();
        if let Some(ln) = &actual.launcher_notices {
            act_ln.insert("current".into(), ln.clone());
        }
        cols.push(compare_keyed_collection(
            "launcher_notices",
            MigrateArea::Reforged,
            &exp_ln,
            &act_ln,
        ));

        let mut exp_op = BTreeMap::new();
        if let Some(op) = &expected.operations_policy {
            exp_op.insert("current".into(), op.clone());
        }
        let mut act_op = BTreeMap::new();
        if let Some(op) = &actual.operations_policy {
            act_op.insert("current".into(), op.clone());
        }
        cols.push(compare_keyed_collection(
            "operations_policy",
            MigrateArea::Reforged,
            &exp_op,
            &act_op,
        ));

        cols.push(compare_keyed_collection(
            "launcher_guard_sessions",
            MigrateArea::Reforged,
            &expected.launcher_guard_sessions,
            &actual.launcher_guard_sessions,
        ));
        cols.push(compare_keyed_collection(
            "guard_findings",
            MigrateArea::Reforged,
            &map_tuple2(&expected.guard_findings),
            &map_tuple2(&actual.guard_findings),
        ));
    }

    let ok = cols.iter().all(|c| c.ok);
    VerifyReport { ok, collections: cols }
}

/// Runs `isles-db verify` end-to-end.
pub async fn run_verify(opts: &MigrateOptions) -> Result<VerifyReport, DbError> {
    let mut client = MongoClient::connect(&opts.from_uri)?;
    let db = open_target_database(&opts.to_target, opts.allow_remote_postgres)?;
    let (existing_users, existing_parties) = db.read(load_existing_id_maps).await?;
    let (expected, _mig_report) = extract_from_mongo(&mut client, existing_users, existing_parties, opts)?;
    let has_expected_ops_policy = expected.operations_policy.is_some();
    let actual = db
        .read(move |conn| read_target_state(conn, has_expected_ops_policy).map_err(DbError::from))
        .await?;

    let report = compare_states(&expected, &actual, opts);
    if !report.ok {
        let failed = report.collections.iter().filter(|c| !c.ok).count();
        return Err(DbError::VerificationFailed(failed));
    }
    Ok(report)
}
