//! Portable Diesel table definitions matching `isles-core`'s 13 migrations across SQLite and Postgres.

diesel::table! {
    accounts (id) {
        id -> Text,
        user_id -> Text,
        name -> Text,
        notes -> BigInt,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    launcher_accounts (account_id) {
        account_id -> Text,
        email -> Nullable<Text>,
        display_name_normalized -> Text,
        display_name -> Text,
        password_hash -> Nullable<Text>,
        status -> Text,
        approval_status -> Nullable<Text>,
        approval_updated_at -> Nullable<Text>,
        approval_updated_by -> Nullable<Text>,
        approval_reason -> Nullable<Text>,
        roles -> Text,
        roles_updated_at -> Nullable<Text>,
        roles_updated_by -> Nullable<Text>,
        created_at -> Text,
        last_login_at -> Nullable<Text>,
        username_set -> Nullable<BigInt>,
        consent_terms_version -> Nullable<Text>,
        consent_privacy_version -> Nullable<Text>,
        consent_accepted_at -> Nullable<Text>,
        deletion_requested_at -> Nullable<Text>,
        deletion_scheduled_for -> Nullable<Text>,
        email_verified_at -> Nullable<Text>,
        email_verification_pending_since -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    auth_identities (provider, provider_subject) {
        provider -> Text,
        provider_subject -> Text,
        account_id -> Text,
        provider_username -> Text,
        provider_avatar_url -> Nullable<Text>,
        linked_at -> Text,
    }
}

diesel::table! {
    refresh_sessions (id) {
        id -> Text,
        token_hash -> Text,
        account_id -> Text,
        family_id -> Text,
        device_id -> Text,
        device_name -> Text,
        created_at -> Text,
        expires_at -> Text,
        revoked_at -> Nullable<Text>,
    }
}

diesel::table! {
    game_exchange_codes (code_hash) {
        code_hash -> Text,
        account_id -> Text,
        launcher_session_id -> Text,
        build_changelist -> BigInt,
        executable_sha256 -> Text,
        runtime_channel -> Text,
        runtime_sha256 -> Text,
        runtime_manifest_version -> Text,
        runtime_artifact_set_sha256 -> Text,
        realm_id -> Nullable<Text>,
        created_at -> Text,
        expires_at -> Text,
        consumed_at -> Nullable<Text>,
    }
}

diesel::table! {
    discord_oauth_transactions (state) {
        state -> Text,
        code_verifier -> Text,
        created_at -> Text,
        expires_at -> Text,
        consumed_at -> Nullable<Text>,
        completion_code_hash -> Nullable<Text>,
        completion_expires_at -> Nullable<Text>,
        completion_consumed_at -> Nullable<Text>,
        account_id -> Nullable<Text>,
        link_account_id -> Nullable<Text>,
    }
}

diesel::table! {
    email_codes (account_id, purpose) {
        account_id -> Text,
        purpose -> Text,
        code_hash -> Text,
        attempts -> BigInt,
        created_at -> Text,
        expires_at -> Text,
    }
}

diesel::table! {
    game_server_api_keys (key_hash) {
        key_hash -> Text,
    }
}

diesel::table! {
    user_api_keys (account_id) {
        account_id -> Text,
        key_hash -> Text,
    }
}

diesel::table! {
    characters (character_id) {
        character_id -> Text,
        account_id -> Text,
        ord -> BigInt,
        created_date -> Text,
        last_modified_date -> Text,
        name -> Text,
        update_version -> BigInt,
        data -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    loadouts (character_id) {
        character_id -> Text,
        account_id -> Text,
        #[sql_name = "loadouts"]
        slots_json -> Text,
        persistent -> Text,
        active_index -> BigInt,
        unlocked_total_slots -> Nullable<BigInt>,
        revision -> BigInt,
        bootstrap_version -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    inventories (character_id) {
        character_id -> Text,
        account_id -> Nullable<Text>,
        instanced_items -> Text,
        stacked_items -> Text,
        revision -> BigInt,
        bootstrap_version -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    inventory_transactions (account_id, character_id, transaction_id) {
        account_id -> Text,
        character_id -> Text,
        transaction_id -> Text,
        request_hash -> Nullable<Text>,
        status -> Text,
        result -> Nullable<Text>,
        created_at -> Text,
        completed_at -> Nullable<Text>,
        economy_source -> Nullable<Text>,
        economy_session_id -> Nullable<Text>,
        extra -> Nullable<Text>,
    }
}

diesel::table! {
    bounty_states (account_id) {
        account_id -> Text,
        bounties -> Text,
        draft_data -> Text,
        daily_reward_credited_on -> Nullable<Text>,
        daily_premium_draft_granted_on -> Nullable<Text>,
        update_version -> BigInt,
        created_at -> Text,
        updated_at -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    cooldown_states (account_id) {
        account_id -> Text,
        entries -> Text,
        update_version -> BigInt,
        created_at -> Text,
        updated_at -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    entitlements (account_id, name) {
        account_id -> Text,
        name -> Text,
        duration -> Double,
        activated_date -> Nullable<Text>,
        granted_at -> Nullable<Text>,
        source_sku_id -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    wallets (account_id) {
        account_id -> Text,
        bootstrap_version -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    wallet_balances (account_id, catalog_id) {
        account_id -> Text,
        catalog_id -> Text,
        balance -> Double,
        ordinal -> BigInt,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    economy_rollup_counters (account_id, day, currency, path) {
        account_id -> Text,
        day -> Text,
        currency -> Text,
        path -> Text,
        value -> Double,
        ttl_at -> BigInt,
    }
}

diesel::table! {
    friend_edges (id) {
        id -> BigInt,
        owner -> Text,
        other -> Text,
        other_user_id -> Text,
        status -> Text,
        direction -> Nullable<Text>,
        created -> Text,
        favorite -> Bool,
    }
}

diesel::table! {
    reforged_invites (code) {
        code -> Text,
        uses_remaining -> BigInt,
        infinite_uses -> Bool,
        note -> Nullable<Text>,
        created_at -> Text,
    }
}

diesel::table! {
    admin_sessions (id) {
        id -> Text,
        token_hash -> Text,
        account -> Text,
        csrf_token -> Text,
        created_at -> Text,
        expires_at -> Text,
        ip -> Text,
        user_agent -> Text,
        revoked_at -> Nullable<Text>,
        ttl_at -> BigInt,
    }
}

diesel::table! {
    admin_audit (seq) {
        seq -> BigInt,
        id -> Text,
        actor_user_id -> Text,
        target_user_id -> Nullable<Text>,
        action -> Text,
        old_state -> Nullable<Text>,
        new_state -> Nullable<Text>,
        reason -> Nullable<Text>,
        ip -> Text,
        request_id -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    launcher_notices (id) {
        id -> BigInt,
        version -> BigInt,
        updated_at -> Nullable<Text>,
        updated_by -> Nullable<Text>,
        announcements -> Text,
        tags -> Text,
    }
}

diesel::table! {
    operations_policy (id) {
        id -> BigInt,
        guard_enforcement -> Text,
        diagnostics_profile -> Text,
        version -> BigInt,
        updated_at -> Nullable<Text>,
        updated_by -> Nullable<Text>,
        diagnostics_expires_at -> Nullable<Text>,
    }
}

diesel::table! {
    launcher_guard_sessions (guard_session_id) {
        guard_session_id -> Text,
        account -> Text,
        role -> Text,
        public_key_base64 -> Text,
        manifest_id -> Text,
        manifest_sequence -> BigInt,
        channel -> Text,
        challenge -> Text,
        last_sequence -> BigInt,
        status -> Text,
        risk_score -> Double,
        created_at -> Text,
        expires_at -> Text,
        last_heartbeat_at -> Nullable<Text>,
        detail -> Text,
        ttl_at -> BigInt,
    }
}

diesel::table! {
    guard_findings (id) {
        id -> BigInt,
        file_name -> Text,
        sha256 -> Text,
        signer -> Text,
        size -> BigInt,
        first_seen_at -> Text,
        last_seen_at -> Text,
        session_count -> BigInt,
        account_count -> BigInt,
        account_ids -> Text,
        ttl_at -> BigInt,
    }
}

diesel::table! {
    progression_tracks (account, progression_id) {
        account -> Text,
        progression_id -> Text,
        progress -> BigInt,
        confirmed_fremium_rank -> BigInt,
        confirmed_premium_rank -> BigInt,
        update_version -> BigInt,
        seq -> BigInt,
        created_at -> Text,
        updated_at -> Text,
        reforge -> Nullable<Text>,
        prestige_claimed -> Nullable<BigInt>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    progression_objectives (account, objective_id) {
        account -> Text,
        objective_id -> Text,
        value -> BigInt,
        completed_count -> BigInt,
        seq -> BigInt,
        created_at -> Text,
        updated_at -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    progression_objective_events (id) {
        id -> Text,
        account -> Text,
        raw_body -> Text,
        received_at -> Text,
    }
}

diesel::table! {
    progression_grants (grant_id) {
        grant_id -> Text,
        account -> Text,
        progression_id -> Text,
        amount -> BigInt,
        request_hash -> Text,
        status -> Text,
        result -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    escalation_progress (account, season_id) {
        account -> Text,
        season_id -> Text,
        escalation_level -> BigInt,
        next_level_xp -> BigInt,
        talents_progress -> Text,
        unlock_progress -> Text,
        update_version -> BigInt,
        created_at -> Text,
        updated_at -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    player_journey (account) {
        account -> Text,
        nodes -> Text,
        update_version -> BigInt,
        revision -> BigInt,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    breadcrumbs (character_id) {
        character_id -> Text,
        account -> Text,
        items -> Text,
        update_version -> BigInt,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    encountered_content (character_id) {
        character_id -> Text,
        account -> Text,
        content -> Text,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    parties (id) {
        id -> Text,
        client_id -> Text,
        revision -> BigInt,
        leader -> Text,
        build_id -> Text,
        activity -> Text,
        active_hunt_session_id -> Nullable<Text>,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    party_members (account) {
        account -> Text,
        party -> Text,
        position -> BigInt,
        joined_at -> Text,
    }
}

diesel::table! {
    party_invites (recipient, sender) {
        recipient -> Text,
        sender -> Text,
        party -> Text,
        party_revision -> BigInt,
        sending_display_name -> Text,
        sending_platform -> Text,
        status -> Text,
        created_at -> Text,
        expires_at -> Text,
        consumed_at -> Nullable<Text>,
        seq -> BigInt,
    }
}

diesel::table! {
    party_outbox (id) {
        id -> Text,
        party -> Text,
        party_revision -> BigInt,
        event_type -> Text,
        recipients -> Text,
        payload -> Text,
        state -> Text,
        created_at -> Text,
        claimed_at -> Nullable<Text>,
        claim_owner -> Nullable<Text>,
        published_at -> Nullable<Text>,
        seq -> BigInt,
    }
}
