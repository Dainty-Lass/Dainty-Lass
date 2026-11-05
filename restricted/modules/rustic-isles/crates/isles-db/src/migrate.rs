//! `isles-db migrate`: extracts all collections from MongoDB, applies DD18 deterministic global ID
//! mapping and the five RB11 data transforms, and writes every row into `store::Database` under
//! `store::Actor::System("migrate")` (`system:migrate`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use diesel::prelude::*;
use isles_core::db::SCHEMA;
use isles_core::ids::AccountId;
use module_api::{GlobalId, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use store::{AnyConnection, Database, DatabaseConfig, DatabaseOptions, PostgresConfig};

use crate::mongo::{MongoClient, MongoError};
use crate::schema::{
    accounts, admin_audit, admin_sessions, auth_identities, bounty_states, breadcrumbs, characters, cooldown_states,
    discord_oauth_transactions, economy_rollup_counters, email_codes, encountered_content, entitlements,
    escalation_progress, friend_edges, game_exchange_codes, game_server_api_keys, guard_findings, inventories,
    inventory_transactions, launcher_accounts, launcher_guard_sessions, launcher_notices, loadouts, operations_policy,
    parties, party_invites, party_members, party_outbox, player_journey, progression_grants,
    progression_objective_events, progression_objectives, progression_tracks, reforged_invites, refresh_sessions,
    user_api_keys, wallet_balances, wallets,
};
use crate::transforms::{
    as_bool, as_epoch_ms, as_f64_or, as_i64_or, as_iso_string, as_opt_iso_string, as_str_opt, as_str_or,
    deterministic_account_id, deterministic_party_id, flatten_economy_rollup, migrate_launcher_approval,
    parse_json_or_value, reconcile_loadout_slots, repair_starter_and_banner_instances, to_json_text,
    transform_inventory_arrays, try_load_storage_policy,
};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("mongo: {0}")]
    Mongo(#[from] MongoError),
    #[error("store: {0}")]
    Store(#[from] store::Error),
    #[error("diesel: {0}")]
    Diesel(#[from] diesel::result::Error),
    #[error("invalid argument: {0}")]
    Config(String),
    #[error("invalid document in {collection}: {detail}")]
    InvalidDocument { collection: String, detail: String },
    #[error("verification failed: {0} collection(s) did not match")]
    VerificationFailed(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MigrateArea {
    Auth,
    Characters,
    Inventory,
    Progression,
    Bounties,
    Parties,
    Reforged,
}

impl MigrateArea {
    pub const ALL: [Self; 7] = [
        Self::Auth,
        Self::Characters,
        Self::Inventory,
        Self::Progression,
        Self::Bounties,
        Self::Parties,
        Self::Reforged,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Characters => "characters",
            Self::Inventory => "inventory",
            Self::Progression => "progression",
            Self::Bounties => "bounties",
            Self::Parties => "parties",
            Self::Reforged => "reforged",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, DbError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auth" | "accounts" => Ok(Self::Auth),
            "characters" | "loadouts" | "characters-loadouts" | "characters_loadouts" => Ok(Self::Characters),
            "inventory" | "inventory-reforge" | "inventory_reforge" => Ok(Self::Inventory),
            "progression" => Ok(Self::Progression),
            "bounties" | "bounties-store" | "bounties_store" | "wallets" => Ok(Self::Bounties),
            "parties" | "matchmaking" | "chat" | "friendships" => Ok(Self::Parties),
            "reforged" | "reforged-api" | "reforged_api" | "admin" | "guard" => Ok(Self::Reforged),
            other => Err(DbError::Config(format!(
                "unknown area {other:?}; valid areas: auth, characters, inventory, progression, bounties, parties, reforged"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MigrateOptions {
    pub from_uri: String,
    pub to_target: String,
    pub node: NodeId,
    pub dry_run: bool,
    pub areas: Vec<MigrateArea>,
    pub skip_invalid: bool,
    pub game_data_dir: Option<PathBuf>,
    pub allow_remote_postgres: bool,
}

impl MigrateOptions {
    pub fn effective_areas(&self) -> BTreeSet<MigrateArea> {
        if self.areas.is_empty() {
            MigrateArea::ALL.into_iter().collect()
        } else {
            self.areas.iter().copied().collect()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrateReport {
    pub dry_run: bool,
    pub counts: BTreeMap<String, usize>,
    pub skipped_invalid: usize,
    pub rb11_launcher_approvals_backfilled: usize,
    pub rb11_inventory_arrays_transformed: usize,
    pub rb11_starter_instances_repaired: usize,
    pub rb11_loadout_slots_reconciled: usize,
}

pub fn parse_postgres_url(url: &str, allow_remote: bool) -> Result<PostgresConfig, DbError> {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .ok_or_else(|| DbError::Config("Postgres URL must start with postgres:// or postgresql://".into()))?;
    let (before_query, _) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, db_raw) = before_query
        .split_once('/')
        .ok_or_else(|| DbError::Config("Postgres URL must include /<database>".into()))?;
    let database = if db_raw.trim_matches('/').is_empty() {
        "postgres".to_owned()
    } else {
        db_raw.trim_matches('/').to_owned()
    };

    let (creds, host_port) = match authority.rsplit_once('@') {
        Some((c, h)) => (Some(c), h),
        None => (None, authority),
    };
    let (user, password) = if let Some(c) = creds {
        match c.split_once(':') {
            Some((u, p)) => (u.to_owned(), p.to_owned()),
            None => (c.to_owned(), String::new()),
        }
    } else {
        ("postgres".to_owned(), String::new())
    };
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        let port: u16 = p
            .parse()
            .map_err(|_| DbError::Config(format!("invalid Postgres port in {host_port:?}")))?;
        (h.to_owned(), port)
    } else {
        (host_port.to_owned(), 5432)
    };
    let cfg = PostgresConfig {
        host,
        port,
        user,
        database,
        password,
        allow_remote,
    };
    cfg.check_address().map_err(DbError::Config)?;
    Ok(cfg)
}

pub fn open_target_database(to: &str, allow_remote_postgres: bool) -> Result<Database, DbError> {
    let trimmed = to.trim();
    if trimmed.is_empty() {
        return Err(DbError::Config("--to target cannot be empty".into()));
    }
    let config = if trimmed.starts_with("postgres://") || trimmed.starts_with("postgresql://") {
        DatabaseConfig::Postgres(parse_postgres_url(trimmed, allow_remote_postgres)?)
    } else {
        let path = trimmed.strip_prefix("sqlite://").unwrap_or(trimmed);
        DatabaseConfig::Sqlite {
            file: PathBuf::from(path),
        }
    };
    Ok(Database::open(&config, &DatabaseOptions::default(), &SCHEMA)?)
}

// ---------------------------------------------------------------------------
// Canonical row types shared by `migrate` and `verify`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountCanon {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub notes: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LauncherAccountCanon {
    pub account_id: String,
    pub email: Option<String>,
    pub display_name_normalized: String,
    pub display_name: String,
    pub password_hash: Option<String>,
    pub status: String,
    pub approval_status: Option<String>,
    pub approval_updated_at: Option<String>,
    pub approval_updated_by: Option<String>,
    pub approval_reason: Option<String>,
    pub roles: String,
    pub roles_updated_at: Option<String>,
    pub roles_updated_by: Option<String>,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub username_set: Option<i64>,
    pub consent_terms_version: Option<String>,
    pub consent_privacy_version: Option<String>,
    pub consent_accepted_at: Option<String>,
    pub deletion_requested_at: Option<String>,
    pub deletion_scheduled_for: Option<String>,
    pub email_verified_at: Option<String>,
    pub email_verification_pending_since: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthIdentityCanon {
    pub provider: String,
    pub provider_subject: String,
    pub account_id: String,
    pub provider_username: String,
    pub provider_avatar_url: Option<String>,
    pub linked_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefreshSessionCanon {
    pub id: String,
    pub token_hash: String,
    pub account_id: String,
    pub family_id: String,
    pub device_id: String,
    pub device_name: String,
    pub created_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameExchangeCodeCanon {
    pub code_hash: String,
    pub account_id: String,
    pub launcher_session_id: String,
    pub build_changelist: i64,
    pub executable_sha256: String,
    pub runtime_channel: String,
    pub runtime_sha256: String,
    pub runtime_manifest_version: String,
    pub runtime_artifact_set_sha256: String,
    pub realm_id: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordOAuthCanon {
    pub state: String,
    pub code_verifier: String,
    pub created_at: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
    pub completion_code_hash: Option<String>,
    pub completion_expires_at: Option<String>,
    pub completion_consumed_at: Option<String>,
    pub account_id: Option<String>,
    pub link_account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmailCodeCanon {
    pub account_id: String,
    pub purpose: String,
    pub code_hash: String,
    pub attempts: i64,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterCanon {
    pub character_id: String,
    pub account_id: String,
    pub ord: i64,
    pub created_date: String,
    pub last_modified_date: String,
    pub name: String,
    pub update_version: i64,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadoutCanon {
    pub character_id: String,
    pub account_id: String,
    pub slots_json: String,
    pub persistent: String,
    pub active_index: i64,
    pub unlocked_total_slots: Option<i64>,
    pub revision: i64,
    pub bootstrap_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InventoryCanon {
    pub character_id: String,
    pub account_id: Option<String>,
    pub instanced_items: String,
    pub stacked_items: String,
    pub revision: i64,
    pub bootstrap_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InventoryTxCanon {
    pub account_id: String,
    pub character_id: String,
    pub transaction_id: String,
    pub request_hash: Option<String>,
    pub status: String,
    pub result: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub economy_source: Option<String>,
    pub economy_session_id: Option<String>,
    pub extra: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BountyStateCanon {
    pub account_id: String,
    pub bounties: String,
    pub draft_data: String,
    pub daily_reward_credited_on: Option<String>,
    pub daily_premium_draft_granted_on: Option<String>,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CooldownStateCanon {
    pub account_id: String,
    pub entries: String,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntitlementCanon {
    pub account_id: String,
    pub name: String,
    pub duration: f64,
    pub activated_date: Option<String>,
    pub granted_at: Option<String>,
    pub source_sku_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalletCanon {
    pub account_id: String,
    pub bootstrap_version: Option<String>,
    pub balances: Vec<(String, f64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EconomyCounterCanon {
    pub account_id: String,
    pub day: String,
    pub currency: String,
    pub path: String,
    pub value: f64,
    pub ttl_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FriendEdgeCanon {
    pub owner: String,
    pub other: String,
    pub other_user_id: String,
    pub status: String,
    pub direction: Option<String>,
    pub created: String,
    pub favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InviteCanon {
    pub code: String,
    pub uses_remaining: i64,
    pub infinite_uses: bool,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminSessionCanon {
    pub id: String,
    pub token_hash: String,
    pub account: String,
    pub csrf_token: String,
    pub created_at: String,
    pub expires_at: String,
    pub ip: String,
    pub user_agent: String,
    pub revoked_at: Option<String>,
    pub ttl_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminAuditCanon {
    pub id: String,
    pub actor_user_id: String,
    pub target_user_id: Option<String>,
    pub action: String,
    pub old_state: Option<String>,
    pub new_state: Option<String>,
    pub reason: Option<String>,
    pub ip: String,
    pub request_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LauncherNoticesCanon {
    pub id: i64,
    pub version: i64,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
    pub announcements: String,
    pub tags: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationsPolicyCanon {
    pub id: i64,
    pub guard_enforcement: String,
    pub diagnostics_profile: String,
    pub version: i64,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
    pub diagnostics_expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardSessionCanon {
    pub guard_session_id: String,
    pub account: String,
    pub role: String,
    pub public_key_base64: String,
    pub manifest_id: String,
    pub manifest_sequence: i64,
    pub channel: String,
    pub challenge: String,
    pub last_sequence: i64,
    pub status: String,
    pub risk_score: f64,
    pub created_at: String,
    pub expires_at: String,
    pub last_heartbeat_at: Option<String>,
    pub detail: String,
    pub ttl_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardFindingCanon {
    pub file_name: String,
    pub sha256: String,
    pub signer: String,
    pub size: i64,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub session_count: i64,
    pub account_count: i64,
    pub account_ids: String,
    pub ttl_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressionTrackCanon {
    pub account: String,
    pub progression_id: String,
    pub progress: i64,
    pub confirmed_fremium_rank: i64,
    pub confirmed_premium_rank: i64,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
    pub reforge: Option<String>,
    pub prestige_claimed: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressionObjectiveCanon {
    pub account: String,
    pub objective_id: String,
    pub value: i64,
    pub completed_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressionEventCanon {
    pub id: String,
    pub account: String,
    pub raw_body: String,
    pub received_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressionGrantCanon {
    pub grant_id: String,
    pub account: String,
    pub progression_id: String,
    pub amount: i64,
    pub request_hash: String,
    pub status: String,
    pub result: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscalationProgressCanon {
    pub account: String,
    pub season_id: String,
    pub escalation_level: i64,
    pub next_level_xp: i64,
    pub talents_progress: String,
    pub unlock_progress: String,
    pub update_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerJourneyCanon {
    pub account: String,
    pub nodes: String,
    pub update_version: i64,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreadcrumbCanon {
    pub character_id: String,
    pub account: String,
    pub items: String,
    pub update_version: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncounteredContentCanon {
    pub character_id: String,
    pub account: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartyCanon {
    pub id: String,
    pub client_id: String,
    pub revision: i64,
    pub leader: String,
    pub build_id: String,
    pub activity: String,
    pub active_hunt_session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub members: Vec<(String, i64, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartyInviteCanon {
    pub recipient: String,
    pub sender: String,
    pub party: String,
    pub party_revision: i64,
    pub sending_display_name: String,
    pub sending_platform: String,
    pub status: String,
    pub created_at: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartyOutboxCanon {
    pub id: String,
    pub party: String,
    pub party_revision: i64,
    pub event_type: String,
    pub recipients: String,
    pub payload: String,
    pub state: String,
    pub created_at: String,
    pub claimed_at: Option<String>,
    pub claim_owner: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TargetState {
    pub accounts: BTreeMap<String, AccountCanon>,
    pub launcher_accounts: BTreeMap<String, LauncherAccountCanon>,
    pub auth_identities: BTreeMap<(String, String), AuthIdentityCanon>,
    pub refresh_sessions: BTreeMap<String, RefreshSessionCanon>,
    pub game_exchange_codes: BTreeMap<String, GameExchangeCodeCanon>,
    pub discord_oauth_transactions: BTreeMap<String, DiscordOAuthCanon>,
    pub email_codes: BTreeMap<(String, String), EmailCodeCanon>,
    pub game_server_api_keys: BTreeSet<String>,
    pub user_api_keys: BTreeMap<String, String>,
    pub characters: BTreeMap<String, CharacterCanon>,
    pub loadouts: BTreeMap<String, LoadoutCanon>,
    pub inventories: BTreeMap<String, InventoryCanon>,
    pub inventory_transactions: BTreeMap<(String, String, String), InventoryTxCanon>,
    pub bounty_states: BTreeMap<String, BountyStateCanon>,
    pub cooldown_states: BTreeMap<String, CooldownStateCanon>,
    pub entitlements: BTreeMap<(String, String), EntitlementCanon>,
    pub wallets: BTreeMap<String, WalletCanon>,
    pub economy_rollup_counters: BTreeMap<(String, String, String, String), EconomyCounterCanon>,
    pub friend_edges: BTreeMap<(String, String), FriendEdgeCanon>,
    pub reforged_invites: BTreeMap<String, InviteCanon>,
    pub admin_sessions: BTreeMap<String, AdminSessionCanon>,
    pub admin_audit: BTreeMap<String, AdminAuditCanon>,
    pub launcher_notices: Option<LauncherNoticesCanon>,
    pub operations_policy: Option<OperationsPolicyCanon>,
    pub launcher_guard_sessions: BTreeMap<String, GuardSessionCanon>,
    pub guard_findings: BTreeMap<(String, String), GuardFindingCanon>,
    pub progression_tracks: BTreeMap<(String, String), ProgressionTrackCanon>,
    pub progression_objectives: BTreeMap<(String, String), ProgressionObjectiveCanon>,
    pub progression_objective_events: BTreeMap<String, ProgressionEventCanon>,
    pub progression_grants: BTreeMap<String, ProgressionGrantCanon>,
    pub escalation_progress: BTreeMap<(String, String), EscalationProgressCanon>,
    pub player_journey: BTreeMap<String, PlayerJourneyCanon>,
    pub breadcrumbs: BTreeMap<String, BreadcrumbCanon>,
    pub encountered_content: BTreeMap<String, EncounteredContentCanon>,
    pub parties: BTreeMap<String, PartyCanon>,
    pub party_invites: BTreeMap<(String, String), PartyInviteCanon>,
    pub party_outbox: BTreeMap<String, PartyOutboxCanon>,
}

pub type ExistingIdMaps = (HashMap<String, AccountId>, HashMap<String, GlobalId>);

/// Reads existing `user_id -> AccountId` and `client_id -> GlobalId` mappings from the target database
/// so re-running migration on a database with live accounts reuses their global IDs.
pub fn load_existing_id_maps(conn: &mut AnyConnection) -> Result<ExistingIdMaps, DbError> {
    let acc_rows = accounts::table
        .select((accounts::id, accounts::user_id))
        .load::<(String, String)>(conn)?;
    let mut user_map = HashMap::with_capacity(acc_rows.len());
    for (id_str, user_id) in acc_rows {
        if let Ok(acc) = id_str.parse::<AccountId>() {
            user_map.insert(user_id, acc);
        }
    }

    let party_rows = parties::table
        .select((parties::id, parties::client_id))
        .load::<(String, String)>(conn)?;
    let mut party_map = HashMap::with_capacity(party_rows.len());
    for (id_str, client_id) in party_rows {
        if let Ok(gid) = id_str.parse::<GlobalId>() {
            party_map.insert(client_id, gid);
        }
    }

    Ok((user_map, party_map))
}

struct IdResolver {
    node: NodeId,
    users: HashMap<String, AccountId>,
    parties: HashMap<String, GlobalId>,
}

impl IdResolver {
    fn resolve_account(&mut self, state: &mut TargetState, user_id: &str) -> Result<AccountId, DbError> {
        let trimmed = user_id.trim();
        if trimmed.is_empty() {
            return Err(DbError::Config("empty userId".into()));
        }
        let acc = if let Some(existing) = self.users.get(trimmed) {
            existing.clone()
        } else {
            let derived = deterministic_account_id(&self.node, trimmed).map_err(DbError::Config)?;
            self.users.insert(trimmed.to_owned(), derived.clone());
            derived
        };
        let id_str = acc.to_string();
        state.accounts.entry(id_str.clone()).or_insert_with(|| AccountCanon {
            id: id_str,
            user_id: trimmed.to_owned(),
            name: trimmed.to_owned(),
            notes: 0,
        });
        Ok(acc)
    }

    fn resolve_party(&mut self, party_id: &str) -> Result<GlobalId, DbError> {
        let trimmed = party_id.trim();
        if trimmed.is_empty() {
            return Err(DbError::Config("empty partyId".into()));
        }
        if let Some(existing) = self.parties.get(trimmed) {
            return Ok(existing.clone());
        }
        let derived = deterministic_party_id(&self.node, trimmed).map_err(DbError::Config)?;
        self.parties.insert(trimmed.to_owned(), derived.clone());
        Ok(derived)
    }
}

fn doc_user_id(doc: &Map<String, Value>) -> Option<String> {
    as_str_opt(
        doc.get("userId")
            .or_else(|| doc.get("accountId"))
            .or_else(|| doc.get("_id")),
    )
}

fn handle_invalid(
    skip_invalid: bool,
    report: &mut MigrateReport,
    collection: &str,
    detail: impl Into<String>,
) -> Result<(), DbError> {
    let detail = detail.into();
    if skip_invalid {
        tracing::warn!("skipping invalid document in {collection}: {detail}");
        report.skipped_invalid += 1;
        Ok(())
    } else {
        Err(DbError::InvalidDocument {
            collection: collection.to_owned(),
            detail,
        })
    }
}

/// Extracts and transforms all selected areas from MongoDB into `TargetState`.
pub fn extract_from_mongo(
    client: &mut MongoClient,
    existing_users: HashMap<String, AccountId>,
    existing_parties: HashMap<String, GlobalId>,
    opts: &MigrateOptions,
) -> Result<(TargetState, MigrateReport), DbError> {
    let areas = opts.effective_areas();
    let mut resolver = IdResolver {
        node: opts.node.clone(),
        users: existing_users,
        parties: existing_parties,
    };
    let mut state = TargetState::default();
    let mut report = MigrateReport {
        dry_run: opts.dry_run,
        ..MigrateReport::default()
    };
    let default_now = "2026-01-01T00:00:00.000Z";

    // Always read `accounts` first if Auth is selected (or to populate display names/notes).
    if areas.contains(&MigrateArea::Auth) {
        let docs = client.find_all("accounts")?;
        for doc in docs {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "accounts", "missing userId/_id")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            let name =
                as_str_opt(doc.get("name").or_else(|| doc.get("displayName"))).unwrap_or_else(|| user_id.clone());
            let notes = as_i64_or(doc.get("notes"), 0);
            state.accounts.insert(
                acc_str.clone(),
                AccountCanon {
                    id: acc_str.clone(),
                    user_id: user_id.clone(),
                    name: name.clone(),
                    notes,
                },
            );

            // A document in `accounts` with `displayNameNormalized` (or `status` / `roles`) is also a launcher account.
            if doc.contains_key("displayNameNormalized")
                || doc.contains_key("passwordHash")
                || doc.contains_key("roles")
                || doc.contains_key("approvalStatus")
            {
                let created_at = as_iso_string(doc.get("createdAt"), default_now);
                let had_approval = doc
                    .get("approvalStatus")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty())
                    || doc
                        .get("approval")
                        .and_then(Value::as_object)
                        .and_then(|o| o.get("status"))
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty());
                let (approval_status, approval_updated_at, approval_updated_by, approval_reason) =
                    migrate_launcher_approval(&doc, &created_at);
                if !had_approval {
                    report.rb11_launcher_approvals_backfilled += 1;
                }

                let display_name = as_str_opt(doc.get("displayName")).unwrap_or_else(|| name.clone());
                let display_name_normalized =
                    as_str_opt(doc.get("displayNameNormalized")).unwrap_or_else(|| display_name.to_ascii_lowercase());
                let roles_val = parse_json_or_value(doc.get("roles"), json!(["user"]));
                let roles = serde_json::to_string(&roles_val).unwrap_or_else(|_| "[\"user\"]".into());
                let consent = doc.get("consent").and_then(Value::as_object);
                let deletion = doc.get("deletion").and_then(Value::as_object);
                let username_set = doc.get("usernameSet").map(|v| as_i64_or(Some(v), 0));

                state.launcher_accounts.insert(
                    acc_str.clone(),
                    LauncherAccountCanon {
                        account_id: acc_str,
                        email: as_str_opt(doc.get("email")),
                        display_name_normalized,
                        display_name,
                        password_hash: as_str_opt(doc.get("passwordHash")),
                        status: as_str_or(doc.get("status"), "active"),
                        approval_status: Some(approval_status),
                        approval_updated_at,
                        approval_updated_by,
                        approval_reason,
                        roles,
                        roles_updated_at: as_opt_iso_string(doc.get("rolesUpdatedAt")),
                        roles_updated_by: as_str_opt(doc.get("rolesUpdatedBy")),
                        created_at,
                        last_login_at: as_opt_iso_string(doc.get("lastLoginAt")),
                        username_set,
                        consent_terms_version: as_str_opt(
                            doc.get("consentTermsVersion")
                                .or_else(|| consent.and_then(|c| c.get("termsVersion"))),
                        ),
                        consent_privacy_version: as_str_opt(
                            doc.get("consentPrivacyVersion")
                                .or_else(|| consent.and_then(|c| c.get("privacyVersion"))),
                        ),
                        consent_accepted_at: as_opt_iso_string(
                            doc.get("consentAcceptedAt")
                                .or_else(|| consent.and_then(|c| c.get("acceptedAt"))),
                        ),
                        deletion_requested_at: as_opt_iso_string(
                            doc.get("deletionRequestedAt")
                                .or_else(|| deletion.and_then(|d| d.get("requestedAt"))),
                        ),
                        deletion_scheduled_for: as_opt_iso_string(
                            doc.get("deletionScheduledFor")
                                .or_else(|| deletion.and_then(|d| d.get("scheduledFor"))),
                        ),
                        email_verified_at: as_opt_iso_string(doc.get("emailVerifiedAt")),
                        email_verification_pending_since: as_opt_iso_string(doc.get("emailVerificationPendingSince")),
                    },
                );
            }
        }

        for doc in client.find_all("authIdentities")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "authIdentities", "missing userId")?;
                continue;
            };
            let Some(provider) = as_str_opt(doc.get("provider")) else {
                handle_invalid(opts.skip_invalid, &mut report, "authIdentities", "missing provider")?;
                continue;
            };
            let Some(subject) = as_str_opt(doc.get("providerSubject").or_else(|| doc.get("subject"))) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "authIdentities",
                    "missing providerSubject",
                )?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            state.auth_identities.insert(
                (provider.clone(), subject.clone()),
                AuthIdentityCanon {
                    provider,
                    provider_subject: subject.clone(),
                    account_id: acc.to_string(),
                    provider_username: as_str_or(doc.get("providerUsername").or_else(|| doc.get("username")), &subject),
                    provider_avatar_url: as_str_opt(doc.get("providerAvatarUrl").or_else(|| doc.get("avatarUrl"))),
                    linked_at: as_iso_string(doc.get("linkedAt").or_else(|| doc.get("createdAt")), default_now),
                },
            );
        }

        for doc in client.find_all("refreshSessions")? {
            let Some(id) = as_str_opt(
                doc.get("sessionId")
                    .or_else(|| doc.get("id"))
                    .or_else(|| doc.get("_id")),
            ) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "refreshSessions",
                    "missing sessionId/_id",
                )?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "refreshSessions", "missing userId")?;
                continue;
            };
            let Some(token_hash) = as_str_opt(doc.get("tokenHash")) else {
                handle_invalid(opts.skip_invalid, &mut report, "refreshSessions", "missing tokenHash")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            state.refresh_sessions.insert(
                id.clone(),
                RefreshSessionCanon {
                    id: id.clone(),
                    token_hash,
                    account_id: acc.to_string(),
                    family_id: as_str_or(doc.get("familyId"), &id),
                    device_id: as_str_or(doc.get("deviceId"), "unknown"),
                    device_name: as_str_or(doc.get("deviceName"), "Launcher"),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    expires_at: as_iso_string(doc.get("expiresAt"), default_now),
                    revoked_at: as_opt_iso_string(doc.get("revokedAt")),
                },
            );
        }

        for doc in client.find_all("gameExchangeCodes")? {
            let Some(code_hash) = as_str_opt(doc.get("codeHash").or_else(|| doc.get("_id"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "gameExchangeCodes", "missing codeHash")?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "gameExchangeCodes", "missing userId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            state.game_exchange_codes.insert(
                code_hash.clone(),
                GameExchangeCodeCanon {
                    code_hash,
                    account_id: acc.to_string(),
                    launcher_session_id: as_str_or(doc.get("launcherSessionId"), ""),
                    build_changelist: as_i64_or(doc.get("buildChangelist"), 0),
                    executable_sha256: as_str_or(doc.get("executableSha256"), ""),
                    runtime_channel: as_str_or(doc.get("runtimeChannel"), "stable"),
                    runtime_sha256: as_str_or(doc.get("runtimeSha256"), ""),
                    runtime_manifest_version: as_str_or(doc.get("runtimeManifestVersion"), ""),
                    runtime_artifact_set_sha256: as_str_or(doc.get("runtimeArtifactSetSha256"), ""),
                    realm_id: as_str_opt(doc.get("realmId")),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    expires_at: as_iso_string(doc.get("expiresAt"), default_now),
                    consumed_at: as_opt_iso_string(doc.get("consumedAt")),
                },
            );
        }

        for doc in client.find_all("discordOAuthTransactions")? {
            let Some(st) = as_str_opt(doc.get("state").or_else(|| doc.get("_id"))) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "discordOAuthTransactions",
                    "missing state",
                )?;
                continue;
            };
            let account_id = if let Some(uid) = as_str_opt(doc.get("userId").or_else(|| doc.get("accountId"))) {
                Some(resolver.resolve_account(&mut state, &uid)?.to_string())
            } else {
                None
            };
            let link_account_id =
                if let Some(uid) = as_str_opt(doc.get("linkUserId").or_else(|| doc.get("linkAccountId"))) {
                    Some(resolver.resolve_account(&mut state, &uid)?.to_string())
                } else {
                    None
                };
            state.discord_oauth_transactions.insert(
                st.clone(),
                DiscordOAuthCanon {
                    state: st,
                    code_verifier: as_str_or(doc.get("codeVerifier").or_else(|| doc.get("pkceVerifier")), ""),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    expires_at: as_iso_string(doc.get("expiresAt"), default_now),
                    consumed_at: as_opt_iso_string(doc.get("consumedAt")),
                    completion_code_hash: as_str_opt(doc.get("completionCodeHash")),
                    completion_expires_at: as_opt_iso_string(doc.get("completionExpiresAt")),
                    completion_consumed_at: as_opt_iso_string(doc.get("completionConsumedAt")),
                    account_id,
                    link_account_id,
                },
            );
        }

        for doc in client.find_all("emailCodes")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "emailCodes", "missing userId")?;
                continue;
            };
            let Some(raw_purpose) = as_str_opt(doc.get("purpose")) else {
                handle_invalid(opts.skip_invalid, &mut report, "emailCodes", "missing purpose")?;
                continue;
            };
            let purpose = match raw_purpose.as_str() {
                "verify_email" => "verify".to_owned(),
                "password_reset" => "reset".to_owned(),
                "discord_unlink" => "unlink".to_owned(),
                other => other.to_owned(),
            };
            let Some(code_hash) = as_str_opt(doc.get("codeHash")) else {
                handle_invalid(opts.skip_invalid, &mut report, "emailCodes", "missing codeHash")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            state.email_codes.insert(
                (acc_str.clone(), purpose.clone()),
                EmailCodeCanon {
                    account_id: acc_str,
                    purpose,
                    code_hash,
                    attempts: as_i64_or(doc.get("attempts"), 0),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    expires_at: as_iso_string(doc.get("expiresAt"), default_now),
                },
            );
        }

        for doc in client.find_all("gameServerApiKeys")? {
            let Some(key_hash) = as_str_opt(doc.get("keyHash").or_else(|| doc.get("key")).or_else(|| doc.get("_id")))
            else {
                handle_invalid(opts.skip_invalid, &mut report, "gameServerApiKeys", "missing keyHash")?;
                continue;
            };
            state.game_server_api_keys.insert(key_hash);
        }

        for doc in client.find_all("userApiKeys")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "userApiKeys", "missing userId")?;
                continue;
            };
            let Some(key_hash) = as_str_opt(doc.get("keyHash").or_else(|| doc.get("key"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "userApiKeys", "missing keyHash")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            state.user_api_keys.insert(acc.to_string(), key_hash);
        }
    }

    // Progression (read before Characters/Loadouts when both are selected so `playerJourney` is ready for RB11 `migrate_loadout_slots`).
    let need_journey_for_loadouts = areas.contains(&MigrateArea::Characters);
    let mut journey_by_account: HashMap<String, Value> = HashMap::new();
    if areas.contains(&MigrateArea::Progression) || need_journey_for_loadouts {
        for doc in client.find_all("playerJourney")? {
            let Some(user_id) = doc_user_id(&doc) else {
                if areas.contains(&MigrateArea::Progression) {
                    handle_invalid(opts.skip_invalid, &mut report, "playerJourney", "missing userId")?;
                }
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            let nodes_val = parse_json_or_value(doc.get("nodes"), json!({}));
            journey_by_account.insert(acc_str.clone(), nodes_val.clone());
            if areas.contains(&MigrateArea::Progression) {
                state.player_journey.insert(
                    acc_str.clone(),
                    PlayerJourneyCanon {
                        account: acc_str,
                        nodes: serde_json::to_string(&nodes_val).unwrap_or_else(|_| "{}".into()),
                        update_version: as_i64_or(doc.get("updateVersion"), 0),
                        revision: as_i64_or(doc.get("revision"), 0),
                    },
                );
            }
        }
    }

    if areas.contains(&MigrateArea::Progression) {
        for doc in client.find_all("progressionTracks")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "progressionTracks", "missing userId")?;
                continue;
            };
            let Some(progression_id) = as_str_opt(doc.get("progressionId")) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "progressionTracks",
                    "missing progressionId",
                )?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            let reforge = doc
                .get("reforge")
                .filter(|v| !v.is_null())
                .map(|v| to_json_text(Some(v), "{}"));
            let prestige_claimed = doc.get("prestigeClaimed").filter(|v| !v.is_null()).map(|v| {
                if let Some(b) = v.as_bool() {
                    i64::from(b)
                } else {
                    as_i64_or(Some(v), 0)
                }
            });
            state.progression_tracks.insert(
                (acc_str.clone(), progression_id.clone()),
                ProgressionTrackCanon {
                    account: acc_str,
                    progression_id,
                    progress: as_i64_or(doc.get("progress"), 0),
                    confirmed_fremium_rank: as_i64_or(doc.get("confirmedFremiumRank"), 0),
                    confirmed_premium_rank: as_i64_or(doc.get("confirmedPremiumRank"), 0),
                    update_version: as_i64_or(doc.get("updateVersion"), 0),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    updated_at: as_iso_string(doc.get("updatedAt"), default_now),
                    reforge,
                    prestige_claimed,
                },
            );
        }

        for doc in client.find_all("progressionObjectives")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "progressionObjectives",
                    "missing userId",
                )?;
                continue;
            };
            let Some(objective_id) = as_str_opt(doc.get("objectiveId")) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "progressionObjectives",
                    "missing objectiveId",
                )?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            state.progression_objectives.insert(
                (acc_str.clone(), objective_id.clone()),
                ProgressionObjectiveCanon {
                    account: acc_str,
                    objective_id,
                    value: as_i64_or(doc.get("value"), 0),
                    completed_count: as_i64_or(doc.get("completedCount"), 0),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    updated_at: as_iso_string(doc.get("updatedAt"), default_now),
                },
            );
        }

        for doc in client.find_all("progressionObjectiveEvents")? {
            let Some(id) = as_str_opt(doc.get("id").or_else(|| doc.get("_id"))) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "progressionObjectiveEvents",
                    "missing _id",
                )?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "progressionObjectiveEvents",
                    "missing userId",
                )?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            state.progression_objective_events.insert(
                id.clone(),
                ProgressionEventCanon {
                    id,
                    account: acc.to_string(),
                    raw_body: to_json_text(doc.get("rawBody").or_else(|| doc.get("body")), "{}"),
                    received_at: as_iso_string(doc.get("receivedAt").or_else(|| doc.get("createdAt")), default_now),
                },
            );
        }

        for doc in client.find_all("progressionTransactions")? {
            let Some(grant_id) = as_str_opt(
                doc.get("grantId")
                    .or_else(|| doc.get("transactionId"))
                    .or_else(|| doc.get("_id")),
            ) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "progressionTransactions",
                    "missing grantId",
                )?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "progressionTransactions",
                    "missing userId",
                )?;
                continue;
            };
            let Some(progression_id) = as_str_opt(doc.get("progressionId")) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "progressionTransactions",
                    "missing progressionId",
                )?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            let mut result_val = doc.get("result").cloned().unwrap_or_else(|| serde_json::json!({}));
            if let Value::Object(ref mut m) = result_val {
                m.insert("userId".into(), Value::String(acc_str.clone()));
            }
            state.progression_grants.insert(
                grant_id.clone(),
                ProgressionGrantCanon {
                    grant_id,
                    account: acc_str,
                    progression_id,
                    amount: as_i64_or(doc.get("amount"), 0),
                    request_hash: as_str_or(doc.get("requestHash"), ""),
                    status: as_str_or(doc.get("status"), "applied"),
                    result: to_json_text(Some(&result_val), "{}"),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                },
            );
        }

        for doc in client.find_all("escalationProgress")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "escalationProgress", "missing userId")?;
                continue;
            };
            let Some(season_id) = as_str_opt(doc.get("seasonId")) else {
                handle_invalid(opts.skip_invalid, &mut report, "escalationProgress", "missing seasonId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            state.escalation_progress.insert(
                (acc_str.clone(), season_id.clone()),
                EscalationProgressCanon {
                    account: acc_str,
                    season_id,
                    escalation_level: as_i64_or(doc.get("escalationLevel"), 0),
                    next_level_xp: as_i64_or(doc.get("nextLevelXp"), 0),
                    talents_progress: to_json_text(doc.get("talentsProgress"), "[]"),
                    unlock_progress: to_json_text(doc.get("unlockProgress"), "[]"),
                    update_version: as_i64_or(doc.get("updateVersion"), 0),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    updated_at: as_iso_string(doc.get("updatedAt"), default_now),
                },
            );
        }

        for doc in client.find_all("breadcrumbs")? {
            let Some(character_id) = as_str_opt(doc.get("characterId").or_else(|| doc.get("_id"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "breadcrumbs", "missing characterId")?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "breadcrumbs", "missing userId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            state.breadcrumbs.insert(
                character_id.clone(),
                BreadcrumbCanon {
                    character_id,
                    account: acc.to_string(),
                    items: to_json_text(doc.get("breadcrumbs").or_else(|| doc.get("items")), "[]"),
                    update_version: as_i64_or(doc.get("updateVersion"), 0),
                },
            );
        }

        for doc in client.find_all("encounteredContent")? {
            let Some(character_id) = as_str_opt(doc.get("characterId").or_else(|| doc.get("_id"))) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "encounteredContent",
                    "missing characterId",
                )?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "encounteredContent", "missing userId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            state.encountered_content.insert(
                character_id.clone(),
                EncounteredContentCanon {
                    character_id,
                    account: acc.to_string(),
                    content: to_json_text(doc.get("encounteredcontent").or_else(|| doc.get("content")), "[]"),
                },
            );
        }
    }

    // Characters, Loadouts, and Inventories (coordinated for RB11 starter/banner instance repair and loadout slot reconciliation).
    if areas.contains(&MigrateArea::Characters) || areas.contains(&MigrateArea::Inventory) {
        let storage_policy = try_load_storage_policy(opts.game_data_dir.as_deref());

        if areas.contains(&MigrateArea::Characters) {
            let mut ord_counter = 0i64;
            for doc in client.find_all("characters")? {
                let Some(character_id) = as_str_opt(doc.get("characterId").or_else(|| doc.get("_id"))) else {
                    handle_invalid(opts.skip_invalid, &mut report, "characters", "missing characterId")?;
                    continue;
                };
                let Some(user_id) = doc_user_id(&doc) else {
                    handle_invalid(opts.skip_invalid, &mut report, "characters", "missing userId")?;
                    continue;
                };
                ord_counter += 1;
                let acc = resolver.resolve_account(&mut state, &user_id)?;
                let created_date = as_iso_string(doc.get("createdDate").or_else(|| doc.get("createdAt")), default_now);
                let last_modified_date = as_iso_string(
                    doc.get("lastModifiedDate").or_else(|| doc.get("updatedAt")),
                    &created_date,
                );
                state.characters.insert(
                    character_id.clone(),
                    CharacterCanon {
                        character_id: character_id.clone(),
                        account_id: acc.to_string(),
                        ord: as_i64_or(doc.get("ord"), ord_counter),
                        created_date,
                        last_modified_date,
                        name: as_str_or(doc.get("name"), &character_id),
                        update_version: as_i64_or(doc.get("updateVersion"), 0),
                        data: to_json_text(doc.get("data"), "{}"),
                    },
                );
            }
        }

        // Load raw inventories and loadouts so RB11 starter/banner instance repair can cross-reference them when both exist.
        let inv_docs = if areas.contains(&MigrateArea::Inventory) || areas.contains(&MigrateArea::Characters) {
            client.find_all("inventories")?
        } else {
            Vec::new()
        };
        let loadout_docs = if areas.contains(&MigrateArea::Characters) {
            client.find_all("loadouts")?
        } else {
            Vec::new()
        };

        struct PendingInv {
            user_id: String,
            account_id: Option<String>,
            instanced: Vec<Value>,
            stacked: Vec<Value>,
            revision: i64,
            bootstrap_version: Option<String>,
        }
        let mut pending_invs: BTreeMap<String, PendingInv> = BTreeMap::new();
        for doc in inv_docs {
            let Some(character_id) = as_str_opt(doc.get("characterId").or_else(|| doc.get("_id"))) else {
                if areas.contains(&MigrateArea::Inventory) {
                    handle_invalid(opts.skip_invalid, &mut report, "inventories", "missing characterId")?;
                }
                continue;
            };
            let user_id = doc_user_id(&doc).unwrap_or_default();
            let account_id = if !user_id.is_empty() {
                Some(resolver.resolve_account(&mut state, &user_id)?.to_string())
            } else {
                state.characters.get(&character_id).map(|c| c.account_id.clone())
            };
            let (instanced, stacked) = transform_inventory_arrays(
                doc.get("instancedItems"),
                doc.get("stackedItems"),
                storage_policy.as_deref(),
            );
            report.rb11_inventory_arrays_transformed += 1;
            pending_invs.insert(
                character_id,
                PendingInv {
                    user_id,
                    account_id,
                    instanced,
                    stacked,
                    revision: as_i64_or(doc.get("revision"), 0),
                    bootstrap_version: as_str_opt(doc.get("bootstrapVersion")),
                },
            );
        }

        struct PendingLoadout {
            user_id: String,
            account_id: String,
            slots: Vec<Value>,
            persistent: Value,
            active_index: i64,
            unlocked_total_slots: Option<f64>,
            revision: i64,
            bootstrap_version: Option<String>,
        }
        let mut pending_loadouts: BTreeMap<String, PendingLoadout> = BTreeMap::new();
        for doc in loadout_docs {
            let Some(character_id) = as_str_opt(doc.get("characterId").or_else(|| doc.get("_id"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "loadouts", "missing characterId")?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "loadouts", "missing userId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let slots_val = parse_json_or_value(doc.get("loadouts"), Value::Array(Vec::new()));
            let slots = slots_val.as_array().cloned().unwrap_or_default();
            let persistent = parse_json_or_value(doc.get("persistent"), json!({}));
            let unlocked = doc.get("unlockedTotalSlots").and_then(Value::as_f64);
            pending_loadouts.insert(
                character_id,
                PendingLoadout {
                    user_id,
                    account_id: acc.to_string(),
                    slots,
                    persistent,
                    active_index: as_i64_or(doc.get("activeIndex"), 0),
                    unlocked_total_slots: unlocked,
                    revision: as_i64_or(doc.get("revision"), 0),
                    bootstrap_version: as_str_opt(doc.get("bootstrapVersion")),
                },
            );
        }

        // Apply RB11 `repair_starter_and_banner_instances` and `reconcile_loadout_slots`
        let all_char_ids: BTreeSet<String> = pending_invs.keys().chain(pending_loadouts.keys()).cloned().collect();
        for cid in all_char_ids {
            let mut inv_opt = pending_invs.remove(&cid);
            let mut ld_opt = pending_loadouts.remove(&cid);

            match (&mut inv_opt, &mut ld_opt) {
                (Some(inv), Some(ld)) => {
                    let uid = if !inv.user_id.is_empty() {
                        inv.user_id.as_str()
                    } else {
                        ld.user_id.as_str()
                    };
                    repair_starter_and_banner_instances(
                        uid,
                        &cid,
                        &mut inv.instanced,
                        Some(&mut ld.slots),
                        Some(&mut ld.persistent),
                    );
                    report.rb11_starter_instances_repaired += 1;
                }
                (Some(inv), None) => {
                    repair_starter_and_banner_instances(&inv.user_id, &cid, &mut inv.instanced, None, None);
                    report.rb11_starter_instances_repaired += 1;
                }
                (None, Some(ld)) => {
                    let mut dummy_instanced = Vec::new();
                    repair_starter_and_banner_instances(
                        &ld.user_id,
                        &cid,
                        &mut dummy_instanced,
                        Some(&mut ld.slots),
                        Some(&mut ld.persistent),
                    );
                    report.rb11_starter_instances_repaired += 1;
                }
                (None, None) => {}
            }

            if let Some(mut ld) = ld_opt
                && areas.contains(&MigrateArea::Characters)
            {
                let journey = journey_by_account.get(&ld.account_id);
                let (unlocked_total, clamped_active) =
                    reconcile_loadout_slots(&mut ld.slots, ld.unlocked_total_slots, Some(ld.active_index), journey);
                report.rb11_loadout_slots_reconciled += 1;
                state.loadouts.insert(
                    cid.clone(),
                    LoadoutCanon {
                        character_id: cid.clone(),
                        account_id: ld.account_id,
                        slots_json: serde_json::to_string(&ld.slots).unwrap_or_else(|_| "[]".into()),
                        persistent: serde_json::to_string(&ld.persistent).unwrap_or_else(|_| "{}".into()),
                        active_index: clamped_active,
                        unlocked_total_slots: Some(unlocked_total),
                        revision: ld.revision,
                        bootstrap_version: ld.bootstrap_version,
                    },
                );
            }

            if let Some(inv) = inv_opt
                && areas.contains(&MigrateArea::Inventory)
            {
                state.inventories.insert(
                    cid.clone(),
                    InventoryCanon {
                        character_id: cid,
                        account_id: inv.account_id,
                        instanced_items: serde_json::to_string(&inv.instanced).unwrap_or_else(|_| "[]".into()),
                        stacked_items: serde_json::to_string(&inv.stacked).unwrap_or_else(|_| "[]".into()),
                        revision: inv.revision,
                        bootstrap_version: inv.bootstrap_version,
                    },
                );
            }
        }

        if areas.contains(&MigrateArea::Inventory) {
            for doc in client.find_all("inventoryTransactions")? {
                let Some(user_id) = doc_user_id(&doc) else {
                    handle_invalid(
                        opts.skip_invalid,
                        &mut report,
                        "inventoryTransactions",
                        "missing userId",
                    )?;
                    continue;
                };
                let Some(character_id) = as_str_opt(doc.get("characterId")) else {
                    handle_invalid(
                        opts.skip_invalid,
                        &mut report,
                        "inventoryTransactions",
                        "missing characterId",
                    )?;
                    continue;
                };
                let Some(transaction_id) = as_str_opt(doc.get("transactionId").or_else(|| doc.get("_id"))) else {
                    handle_invalid(
                        opts.skip_invalid,
                        &mut report,
                        "inventoryTransactions",
                        "missing transactionId",
                    )?;
                    continue;
                };
                let acc = resolver.resolve_account(&mut state, &user_id)?;
                let acc_str = acc.to_string();
                let result = doc
                    .get("result")
                    .filter(|v| !v.is_null())
                    .map(|v| to_json_text(Some(v), "{}"));

                let mut extra_map = Map::new();
                for (k, v) in &doc {
                    if !matches!(
                        k.as_str(),
                        "_id"
                            | "userId"
                            | "accountId"
                            | "characterId"
                            | "transactionId"
                            | "requestHash"
                            | "status"
                            | "result"
                            | "createdAt"
                            | "completedAt"
                            | "economySource"
                            | "economySessionId"
                    ) {
                        extra_map.insert(k.clone(), v.clone());
                    }
                }
                let extra = if extra_map.is_empty() {
                    as_str_opt(doc.get("extra"))
                } else {
                    Some(Value::Object(extra_map).to_string())
                };

                state.inventory_transactions.insert(
                    (acc_str.clone(), character_id.clone(), transaction_id.clone()),
                    InventoryTxCanon {
                        account_id: acc_str,
                        character_id,
                        transaction_id,
                        request_hash: as_str_opt(doc.get("requestHash")),
                        status: as_str_or(doc.get("status"), "completed"),
                        result,
                        created_at: as_iso_string(doc.get("createdAt"), default_now),
                        completed_at: as_opt_iso_string(doc.get("completedAt")),
                        economy_source: as_str_opt(doc.get("economySource")),
                        economy_session_id: as_str_opt(doc.get("economySessionId")),
                        extra,
                    },
                );
            }
        }
    }

    // Area: Bounties (`bountyStates`, `cooldownStates`, `entitlements`, `wallets`, `economyDaily`)
    if areas.contains(&MigrateArea::Bounties) {
        for doc in client.find_all("bountyStates")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "bountyStates", "missing userId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            state.bounty_states.insert(
                acc_str.clone(),
                BountyStateCanon {
                    account_id: acc_str,
                    bounties: to_json_text(doc.get("bounties"), "[]"),
                    draft_data: to_json_text(doc.get("draftData"), "{}"),
                    daily_reward_credited_on: as_str_opt(doc.get("dailyRewardCreditedOn")),
                    daily_premium_draft_granted_on: as_str_opt(doc.get("dailyPremiumDraftGrantedOn")),
                    update_version: as_i64_or(doc.get("updateVersion"), 0),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    updated_at: as_iso_string(doc.get("updatedAt"), default_now),
                },
            );
        }

        for doc in client.find_all("cooldownStates")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "cooldownStates", "missing userId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            state.cooldown_states.insert(
                acc_str.clone(),
                CooldownStateCanon {
                    account_id: acc_str,
                    entries: to_json_text(doc.get("entries").or_else(|| doc.get("cooldowns")), "{}"),
                    update_version: as_i64_or(doc.get("updateVersion"), 0),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    updated_at: as_iso_string(doc.get("updatedAt"), default_now),
                },
            );
        }

        for doc in client.find_all("entitlements")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "entitlements", "missing userId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            let list = parse_json_or_value(doc.get("entitlements"), Value::Array(Vec::new()));
            if let Some(items) = list.as_array() {
                for item in items {
                    let Some(obj) = item.as_object() else {
                        continue;
                    };
                    let Some(name) = as_str_opt(obj.get("name")) else {
                        continue;
                    };
                    state.entitlements.insert(
                        (acc_str.clone(), name.clone()),
                        EntitlementCanon {
                            account_id: acc_str.clone(),
                            name,
                            duration: as_f64_or(obj.get("duration"), 0.0),
                            activated_date: as_opt_iso_string(obj.get("activatedDate")),
                            granted_at: as_opt_iso_string(obj.get("grantedAt")),
                            source_sku_id: as_str_opt(obj.get("sourceSkuId")),
                        },
                    );
                }
            }
        }

        for doc in client.find_all("wallets")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "wallets", "missing userId")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            let balances_val = parse_json_or_value(doc.get("balances"), json!({}));
            let mut balances = Vec::new();
            if let Some(map) = balances_val.as_object() {
                for (ord, (cid, val)) in map.iter().enumerate() {
                    if let Some(amount) = val.as_f64() {
                        balances.push((cid.clone(), amount, ord as i64));
                    }
                }
            }
            state.wallets.insert(
                acc_str.clone(),
                WalletCanon {
                    account_id: acc_str,
                    bootstrap_version: as_str_opt(doc.get("bootstrapVersion")),
                    balances,
                },
            );
        }

        for doc in client.find_all("economyDaily")? {
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "economyDaily", "missing userId")?;
                continue;
            };
            let Some(day) = as_str_opt(doc.get("day")) else {
                handle_invalid(opts.skip_invalid, &mut report, "economyDaily", "missing day")?;
                continue;
            };
            let Some(currency) = as_str_opt(doc.get("currency")) else {
                handle_invalid(opts.skip_invalid, &mut report, "economyDaily", "missing currency")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let acc_str = acc.to_string();
            let ttl_at = as_epoch_ms(doc.get("ttlAt").or_else(|| doc.get("expiresAt")), 0);
            for (path, value) in flatten_economy_rollup(&doc) {
                state.economy_rollup_counters.insert(
                    (acc_str.clone(), day.clone(), currency.clone(), path.clone()),
                    EconomyCounterCanon {
                        account_id: acc_str.clone(),
                        day: day.clone(),
                        currency: currency.clone(),
                        path,
                        value,
                        ttl_at,
                    },
                );
            }
        }
    }

    // Area: Parties (`parties`, `partyMemberships`, `partyInvites`, `partyOutbox`, `friendships`)
    if areas.contains(&MigrateArea::Parties) {
        let mut membership_joined: HashMap<(String, String), String> = HashMap::new();
        for doc in client.find_all("partyMemberships")? {
            if let (Some(uid), Some(pid)) = (doc_user_id(&doc), as_str_opt(doc.get("partyId"))) {
                let joined = as_iso_string(doc.get("joinedAt"), default_now);
                membership_joined.insert((pid, uid), joined);
            }
        }

        for doc in client.find_all("parties")? {
            let Some(client_id) = as_str_opt(doc.get("partyId").or_else(|| doc.get("_id"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "parties", "missing partyId/_id")?;
                continue;
            };
            let Some(leader_uid) = as_str_opt(doc.get("leaderPlayerId").or_else(|| doc.get("leader"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "parties", "missing leaderPlayerId")?;
                continue;
            };
            let party_gid = resolver.resolve_party(&client_id)?;
            let party_id_str = party_gid.to_string();
            let leader_acc = resolver.resolve_account(&mut state, &leader_uid)?;
            let created_at = as_iso_string(doc.get("createdAt"), default_now);
            let updated_at = as_iso_string(doc.get("updatedAt"), &created_at);

            let mut members = Vec::new();
            if let Some(arr) = doc.get("members").and_then(Value::as_array) {
                for (pos, m) in arr.iter().enumerate() {
                    if let Some(uid) = m.as_str() {
                        let m_acc = resolver.resolve_account(&mut state, uid)?;
                        let joined = membership_joined
                            .get(&(client_id.clone(), uid.to_owned()))
                            .cloned()
                            .unwrap_or_else(|| created_at.clone());
                        members.push((m_acc.to_string(), pos as i64, joined));
                    }
                }
            }
            if members.is_empty() {
                members.push((leader_acc.to_string(), 0, created_at.clone()));
            }

            state.parties.insert(
                party_id_str.clone(),
                PartyCanon {
                    id: party_id_str,
                    client_id,
                    revision: as_i64_or(doc.get("revision"), 1),
                    leader: leader_acc.to_string(),
                    build_id: as_str_or(doc.get("buildId"), "647472"),
                    activity: as_str_or(doc.get("activity"), "IDLE"),
                    active_hunt_session_id: as_str_opt(doc.get("activeHuntSessionId")),
                    created_at,
                    updated_at,
                    members,
                },
            );
        }

        for doc in client.find_all("partyInvites")? {
            let Some(recipient_uid) = as_str_opt(doc.get("recipientPlayerId").or_else(|| doc.get("recipient"))) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "partyInvites",
                    "missing recipientPlayerId",
                )?;
                continue;
            };
            let Some(sender_uid) = as_str_opt(doc.get("sendingPlayerId").or_else(|| doc.get("sender"))) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "partyInvites",
                    "missing sendingPlayerId",
                )?;
                continue;
            };
            let Some(party_client_id) = as_str_opt(doc.get("partyId")) else {
                handle_invalid(opts.skip_invalid, &mut report, "partyInvites", "missing partyId")?;
                continue;
            };
            let recipient_acc = resolver.resolve_account(&mut state, &recipient_uid)?;
            let sender_acc = resolver.resolve_account(&mut state, &sender_uid)?;
            let party_gid = resolver.resolve_party(&party_client_id)?;
            let r_str = recipient_acc.to_string();
            let s_str = sender_acc.to_string();
            state.party_invites.insert(
                (r_str.clone(), s_str.clone()),
                PartyInviteCanon {
                    recipient: r_str,
                    sender: s_str,
                    party: party_gid.to_string(),
                    party_revision: as_i64_or(doc.get("partyRevision"), 1),
                    sending_display_name: as_str_or(doc.get("sendingDisplayName"), &sender_uid),
                    sending_platform: as_str_or(doc.get("sendingPlatform"), "PC"),
                    status: as_str_or(doc.get("status"), "PENDING"),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    expires_at: as_iso_string(doc.get("expiresAt"), default_now),
                    consumed_at: as_opt_iso_string(doc.get("consumedAt")),
                },
            );
        }

        for doc in client.find_all("partyOutbox")? {
            let Some(id) = as_str_opt(doc.get("eventId").or_else(|| doc.get("_id"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "partyOutbox", "missing eventId/_id")?;
                continue;
            };
            let Some(party_client_id) = as_str_opt(doc.get("partyId")) else {
                handle_invalid(opts.skip_invalid, &mut report, "partyOutbox", "missing partyId")?;
                continue;
            };
            let recipients = to_json_text(doc.get("recipientAccountIds").or_else(|| doc.get("recipients")), "[]");
            state.party_outbox.insert(
                id.clone(),
                PartyOutboxCanon {
                    id,
                    party: party_client_id,
                    party_revision: as_i64_or(doc.get("partyRevision"), 1),
                    event_type: as_str_or(doc.get("eventType"), "PARTY_CREATED"),
                    recipients,
                    payload: to_json_text(doc.get("payload"), "{}"),
                    state: as_str_or(doc.get("state"), "PENDING"),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    claimed_at: as_opt_iso_string(doc.get("claimedAt")),
                    claim_owner: as_str_opt(doc.get("claimOwner")),
                    published_at: as_opt_iso_string(doc.get("publishedAt")),
                },
            );
        }

        for doc in client.find_all("friendships")? {
            let Some(owner_uid) = as_str_opt(doc.get("ownerId")) else {
                handle_invalid(opts.skip_invalid, &mut report, "friendships", "missing ownerId")?;
                continue;
            };
            let Some(other_uid) = as_str_opt(doc.get("otherId")) else {
                handle_invalid(opts.skip_invalid, &mut report, "friendships", "missing otherId")?;
                continue;
            };
            let owner_acc = resolver.resolve_account(&mut state, &owner_uid)?;
            let other_acc = resolver.resolve_account(&mut state, &other_uid)?;
            let owner_str = owner_acc.to_string();
            let other_str = other_acc.to_string();
            state.friend_edges.insert(
                (owner_str.clone(), other_str.clone()),
                FriendEdgeCanon {
                    owner: owner_str,
                    other: other_str,
                    other_user_id: other_uid,
                    status: as_str_or(doc.get("status"), "ACCEPTED"),
                    direction: as_str_opt(doc.get("direction")),
                    created: as_iso_string(doc.get("created"), default_now),
                    favorite: as_bool(doc.get("favorite"), false),
                },
            );
        }
    }

    // Area: Reforged (`reforgedInvites`, `adminSessions`, `adminAudit`, `launcherNotices`, `operationsPolicy`, `launcherGuardSessions`, `guardFindings`)
    if areas.contains(&MigrateArea::Reforged) {
        for doc in client.find_all("reforgedInvites")? {
            let Some(code) = as_str_opt(
                doc.get("invitecode")
                    .or_else(|| doc.get("code"))
                    .or_else(|| doc.get("_id")),
            ) else {
                handle_invalid(opts.skip_invalid, &mut report, "reforgedInvites", "missing invitecode")?;
                continue;
            };
            state.reforged_invites.insert(
                code.clone(),
                InviteCanon {
                    code,
                    uses_remaining: as_i64_or(doc.get("usesRemaining").or_else(|| doc.get("uses_remaining")), 1),
                    infinite_uses: as_bool(doc.get("infiniteUses").or_else(|| doc.get("infinite_uses")), false),
                    note: as_str_opt(doc.get("note")),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                },
            );
        }

        for doc in client.find_all("adminSessions")? {
            let Some(id) = as_str_opt(doc.get("id").or_else(|| doc.get("_id"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "adminSessions", "missing id/_id")?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(opts.skip_invalid, &mut report, "adminSessions", "missing userId")?;
                continue;
            };
            let Some(token_hash) = as_str_opt(doc.get("tokenHash")) else {
                handle_invalid(opts.skip_invalid, &mut report, "adminSessions", "missing tokenHash")?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let expires_at = as_iso_string(doc.get("expiresAt"), default_now);
            let ttl_at = as_epoch_ms(
                doc.get("ttlAt"),
                as_epoch_ms(Some(&Value::String(expires_at.clone())), 0),
            );
            state.admin_sessions.insert(
                id.clone(),
                AdminSessionCanon {
                    id,
                    token_hash,
                    account: acc.to_string(),
                    csrf_token: as_str_or(doc.get("csrfToken"), ""),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                    expires_at,
                    ip: as_str_or(doc.get("ip"), "127.0.0.1"),
                    user_agent: as_str_or(doc.get("userAgent"), ""),
                    revoked_at: as_opt_iso_string(doc.get("revokedAt")),
                    ttl_at,
                },
            );
        }

        for doc in client.find_all("adminAudit")? {
            let Some(id) = as_str_opt(doc.get("id").or_else(|| doc.get("_id"))) else {
                handle_invalid(opts.skip_invalid, &mut report, "adminAudit", "missing id/_id")?;
                continue;
            };
            let old_state = doc
                .get("oldState")
                .filter(|v| !v.is_null())
                .map(|v| to_json_text(Some(v), "{}"));
            let new_state = doc
                .get("newState")
                .filter(|v| !v.is_null())
                .map(|v| to_json_text(Some(v), "{}"));
            state.admin_audit.insert(
                id.clone(),
                AdminAuditCanon {
                    id,
                    actor_user_id: as_str_or(doc.get("actorUserId"), "system"),
                    target_user_id: as_str_opt(doc.get("targetUserId")),
                    action: as_str_or(doc.get("action"), "UNKNOWN"),
                    old_state,
                    new_state,
                    reason: as_str_opt(doc.get("reason")),
                    ip: as_str_or(doc.get("ip"), "127.0.0.1"),
                    request_id: as_str_or(doc.get("requestId"), ""),
                    created_at: as_iso_string(doc.get("createdAt"), default_now),
                },
            );
        }

        if let Some(doc) = client.find_all("launcherNotices")?.into_iter().next() {
            state.launcher_notices = Some(LauncherNoticesCanon {
                id: 1,
                version: as_i64_or(doc.get("version"), 1),
                updated_at: as_opt_iso_string(doc.get("updatedAt")),
                updated_by: as_str_opt(doc.get("updatedBy")),
                announcements: to_json_text(doc.get("announcements"), "[]"),
                tags: to_json_text(doc.get("tags"), "[]"),
            });
        }

        if let Some(doc) = client.find_all("operationsPolicy")?.into_iter().next() {
            state.operations_policy = Some(OperationsPolicyCanon {
                id: 1,
                guard_enforcement: as_str_or(doc.get("guardEnforcement"), "ENFORCE"),
                diagnostics_profile: as_str_or(doc.get("diagnosticsProfile"), "PRODUCTION"),
                version: as_i64_or(doc.get("version"), 1),
                updated_at: as_opt_iso_string(doc.get("updatedAt")),
                updated_by: as_str_opt(doc.get("updatedBy")),
                diagnostics_expires_at: as_opt_iso_string(doc.get("diagnosticsExpiresAt")),
            });
        }

        for doc in client.find_all("launcherGuardSessions")? {
            let Some(gs_id) = as_str_opt(doc.get("guardSessionId").or_else(|| doc.get("_id"))) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "launcherGuardSessions",
                    "missing guardSessionId",
                )?;
                continue;
            };
            let Some(user_id) = doc_user_id(&doc) else {
                handle_invalid(
                    opts.skip_invalid,
                    &mut report,
                    "launcherGuardSessions",
                    "missing accountId",
                )?;
                continue;
            };
            let acc = resolver.resolve_account(&mut state, &user_id)?;
            let created_at = as_iso_string(doc.get("createdAt"), default_now);
            let detail = if let Some(d) = doc.get("detail") {
                to_json_text(Some(d), "{}")
            } else {
                json!({
                    "launcherVersion": as_str_or(doc.get("launcherVersion"), ""),
                    "lastSignals": doc.get("lastSignals").cloned().unwrap_or(Value::Null),
                    "reasons": parse_json_or_value(doc.get("reasons"), json!([])),
                    "history": parse_json_or_value(doc.get("history"), json!([])),
                })
                .to_string()
            };
            let ttl_at = as_epoch_ms(doc.get("ttlAt"), 0);
            state.launcher_guard_sessions.insert(
                gs_id.clone(),
                GuardSessionCanon {
                    guard_session_id: gs_id,
                    account: acc.to_string(),
                    role: if as_str_or(doc.get("role"), "client") == "host" {
                        "host".into()
                    } else {
                        "client".into()
                    },
                    public_key_base64: as_str_or(doc.get("publicKeyBase64"), ""),
                    manifest_id: as_str_or(doc.get("manifestId"), ""),
                    manifest_sequence: as_i64_or(doc.get("manifestSequence"), 0),
                    channel: as_str_or(doc.get("channel"), "stable"),
                    challenge: as_str_or(doc.get("challenge"), ""),
                    last_sequence: as_i64_or(doc.get("lastSequence"), 0),
                    status: as_str_or(doc.get("status"), "CHALLENGED"),
                    risk_score: as_f64_or(doc.get("riskScore"), 0.0),
                    created_at,
                    expires_at: as_iso_string(doc.get("expiresAt"), default_now),
                    last_heartbeat_at: as_opt_iso_string(doc.get("lastHeartbeatAt")),
                    detail,
                    ttl_at,
                },
            );
        }

        for doc in client.find_all("guardFindings")? {
            let Some(file_name) = as_str_opt(doc.get("fileName")) else {
                handle_invalid(opts.skip_invalid, &mut report, "guardFindings", "missing fileName")?;
                continue;
            };
            let Some(sha256) = as_str_opt(doc.get("sha256")) else {
                handle_invalid(opts.skip_invalid, &mut report, "guardFindings", "missing sha256")?;
                continue;
            };
            let raw_accounts = parse_json_or_value(doc.get("accountIds"), json!([]));
            let mut mapped_accounts = Vec::new();
            if let Some(arr) = raw_accounts.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        if s.starts_with("account:") {
                            mapped_accounts.push(s.to_owned());
                        } else if let Ok(acc) = resolver.resolve_account(&mut state, s) {
                            mapped_accounts.push(acc.to_string());
                        }
                    }
                }
            }
            state.guard_findings.insert(
                (file_name.clone(), sha256.clone()),
                GuardFindingCanon {
                    file_name,
                    sha256,
                    signer: as_str_or(doc.get("signer"), ""),
                    size: as_i64_or(doc.get("size"), 0),
                    first_seen_at: as_iso_string(doc.get("firstSeenAt"), default_now),
                    last_seen_at: as_iso_string(doc.get("lastSeenAt"), default_now),
                    session_count: as_i64_or(doc.get("sessionCount"), 1),
                    account_count: as_i64_or(doc.get("accountCount"), mapped_accounts.len() as i64),
                    account_ids: serde_json::to_string(&mapped_accounts).unwrap_or_else(|_| "[]".into()),
                    ttl_at: as_epoch_ms(doc.get("ttlAt"), 0),
                },
            );
        }
    }

    populate_counts(&state, &mut report);
    Ok((state, report))
}

fn populate_counts(state: &TargetState, report: &mut MigrateReport) {
    report.counts.insert("accounts".into(), state.accounts.len());
    report
        .counts
        .insert("launcher_accounts".into(), state.launcher_accounts.len());
    report
        .counts
        .insert("auth_identities".into(), state.auth_identities.len());
    report
        .counts
        .insert("refresh_sessions".into(), state.refresh_sessions.len());
    report
        .counts
        .insert("game_exchange_codes".into(), state.game_exchange_codes.len());
    report.counts.insert(
        "discord_oauth_transactions".into(),
        state.discord_oauth_transactions.len(),
    );
    report.counts.insert("email_codes".into(), state.email_codes.len());
    report
        .counts
        .insert("game_server_api_keys".into(), state.game_server_api_keys.len());
    report.counts.insert("user_api_keys".into(), state.user_api_keys.len());
    report.counts.insert("characters".into(), state.characters.len());
    report.counts.insert("loadouts".into(), state.loadouts.len());
    report.counts.insert("inventories".into(), state.inventories.len());
    report
        .counts
        .insert("inventory_transactions".into(), state.inventory_transactions.len());
    report.counts.insert("bounty_states".into(), state.bounty_states.len());
    report
        .counts
        .insert("cooldown_states".into(), state.cooldown_states.len());
    report.counts.insert("entitlements".into(), state.entitlements.len());
    report.counts.insert("wallets".into(), state.wallets.len());
    report
        .counts
        .insert("economy_rollup_counters".into(), state.economy_rollup_counters.len());
    report.counts.insert("friend_edges".into(), state.friend_edges.len());
    report
        .counts
        .insert("reforged_invites".into(), state.reforged_invites.len());
    report
        .counts
        .insert("admin_sessions".into(), state.admin_sessions.len());
    report.counts.insert("admin_audit".into(), state.admin_audit.len());
    report
        .counts
        .insert("launcher_notices".into(), usize::from(state.launcher_notices.is_some()));
    report.counts.insert(
        "operations_policy".into(),
        usize::from(state.operations_policy.is_some()),
    );
    report
        .counts
        .insert("launcher_guard_sessions".into(), state.launcher_guard_sessions.len());
    report
        .counts
        .insert("guard_findings".into(), state.guard_findings.len());
    report
        .counts
        .insert("progression_tracks".into(), state.progression_tracks.len());
    report
        .counts
        .insert("progression_objectives".into(), state.progression_objectives.len());
    report.counts.insert(
        "progression_objective_events".into(),
        state.progression_objective_events.len(),
    );
    report
        .counts
        .insert("progression_grants".into(), state.progression_grants.len());
    report
        .counts
        .insert("escalation_progress".into(), state.escalation_progress.len());
    report
        .counts
        .insert("player_journey".into(), state.player_journey.len());
    report.counts.insert("breadcrumbs".into(), state.breadcrumbs.len());
    report
        .counts
        .insert("encountered_content".into(), state.encountered_content.len());
    report.counts.insert("parties".into(), state.parties.len());
    report.counts.insert("party_invites".into(), state.party_invites.len());
    report.counts.insert("party_outbox".into(), state.party_outbox.len());
}

/// Writes `TargetState` idempotently into `conn` (called inside `Database::write` with `system:migrate`).
pub fn write_target_state(conn: &mut AnyConnection, state: &TargetState) -> Result<(), diesel::result::Error> {
    // 1. Accounts (must precede all foreign-key children)
    for row in state.accounts.values() {
        let updated = diesel::update(accounts::table.filter(accounts::id.eq(&row.id)))
            .set((
                accounts::user_id.eq(&row.user_id),
                accounts::name.eq(&row.name),
                accounts::notes.eq(row.notes),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(accounts::table)
                .values((
                    accounts::id.eq(&row.id),
                    accounts::user_id.eq(&row.user_id),
                    accounts::name.eq(&row.name),
                    accounts::notes.eq(row.notes),
                ))
                .execute(conn)?;
        }
    }

    // 2. Launcher accounts
    for row in state.launcher_accounts.values() {
        let vals = (
            launcher_accounts::email.eq(&row.email),
            launcher_accounts::display_name_normalized.eq(&row.display_name_normalized),
            launcher_accounts::display_name.eq(&row.display_name),
            launcher_accounts::password_hash.eq(&row.password_hash),
            launcher_accounts::status.eq(&row.status),
            launcher_accounts::approval_status.eq(&row.approval_status),
            launcher_accounts::approval_updated_at.eq(&row.approval_updated_at),
            launcher_accounts::approval_updated_by.eq(&row.approval_updated_by),
            launcher_accounts::approval_reason.eq(&row.approval_reason),
            launcher_accounts::roles.eq(&row.roles),
            launcher_accounts::roles_updated_at.eq(&row.roles_updated_at),
            launcher_accounts::roles_updated_by.eq(&row.roles_updated_by),
            launcher_accounts::created_at.eq(&row.created_at),
            launcher_accounts::last_login_at.eq(&row.last_login_at),
            launcher_accounts::username_set.eq(row.username_set),
            launcher_accounts::consent_terms_version.eq(&row.consent_terms_version),
            launcher_accounts::consent_privacy_version.eq(&row.consent_privacy_version),
            launcher_accounts::consent_accepted_at.eq(&row.consent_accepted_at),
            launcher_accounts::deletion_requested_at.eq(&row.deletion_requested_at),
            launcher_accounts::deletion_scheduled_for.eq(&row.deletion_scheduled_for),
            launcher_accounts::email_verified_at.eq(&row.email_verified_at),
            launcher_accounts::email_verification_pending_since.eq(&row.email_verification_pending_since),
        );
        let updated =
            diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(&row.account_id)))
                .set(vals)
                .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(launcher_accounts::table)
                .values((launcher_accounts::account_id.eq(&row.account_id), vals))
                .execute(conn)?;
        }
    }

    // 3. Auth identities
    for row in state.auth_identities.values() {
        let target = auth_identities::table
            .filter(auth_identities::provider.eq(&row.provider))
            .filter(auth_identities::provider_subject.eq(&row.provider_subject));
        let updated = diesel::update(target)
            .set((
                auth_identities::account_id.eq(&row.account_id),
                auth_identities::provider_username.eq(&row.provider_username),
                auth_identities::provider_avatar_url.eq(&row.provider_avatar_url),
                auth_identities::linked_at.eq(&row.linked_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(auth_identities::table)
                .values((
                    auth_identities::provider.eq(&row.provider),
                    auth_identities::provider_subject.eq(&row.provider_subject),
                    auth_identities::account_id.eq(&row.account_id),
                    auth_identities::provider_username.eq(&row.provider_username),
                    auth_identities::provider_avatar_url.eq(&row.provider_avatar_url),
                    auth_identities::linked_at.eq(&row.linked_at),
                ))
                .execute(conn)?;
        }
    }

    // 4. Refresh sessions
    for row in state.refresh_sessions.values() {
        let updated = diesel::update(refresh_sessions::table.filter(refresh_sessions::id.eq(&row.id)))
            .set((
                refresh_sessions::token_hash.eq(&row.token_hash),
                refresh_sessions::account_id.eq(&row.account_id),
                refresh_sessions::family_id.eq(&row.family_id),
                refresh_sessions::device_id.eq(&row.device_id),
                refresh_sessions::device_name.eq(&row.device_name),
                refresh_sessions::created_at.eq(&row.created_at),
                refresh_sessions::expires_at.eq(&row.expires_at),
                refresh_sessions::revoked_at.eq(&row.revoked_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(refresh_sessions::table)
                .values((
                    refresh_sessions::id.eq(&row.id),
                    refresh_sessions::token_hash.eq(&row.token_hash),
                    refresh_sessions::account_id.eq(&row.account_id),
                    refresh_sessions::family_id.eq(&row.family_id),
                    refresh_sessions::device_id.eq(&row.device_id),
                    refresh_sessions::device_name.eq(&row.device_name),
                    refresh_sessions::created_at.eq(&row.created_at),
                    refresh_sessions::expires_at.eq(&row.expires_at),
                    refresh_sessions::revoked_at.eq(&row.revoked_at),
                ))
                .execute(conn)?;
        }
    }

    // 5. Game exchange codes
    for row in state.game_exchange_codes.values() {
        let updated =
            diesel::update(game_exchange_codes::table.filter(game_exchange_codes::code_hash.eq(&row.code_hash)))
                .set((
                    game_exchange_codes::account_id.eq(&row.account_id),
                    game_exchange_codes::launcher_session_id.eq(&row.launcher_session_id),
                    game_exchange_codes::build_changelist.eq(row.build_changelist),
                    game_exchange_codes::executable_sha256.eq(&row.executable_sha256),
                    game_exchange_codes::runtime_channel.eq(&row.runtime_channel),
                    game_exchange_codes::runtime_sha256.eq(&row.runtime_sha256),
                    game_exchange_codes::runtime_manifest_version.eq(&row.runtime_manifest_version),
                    game_exchange_codes::runtime_artifact_set_sha256.eq(&row.runtime_artifact_set_sha256),
                    game_exchange_codes::realm_id.eq(&row.realm_id),
                    game_exchange_codes::created_at.eq(&row.created_at),
                    game_exchange_codes::expires_at.eq(&row.expires_at),
                    game_exchange_codes::consumed_at.eq(&row.consumed_at),
                ))
                .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(game_exchange_codes::table)
                .values((
                    game_exchange_codes::code_hash.eq(&row.code_hash),
                    game_exchange_codes::account_id.eq(&row.account_id),
                    game_exchange_codes::launcher_session_id.eq(&row.launcher_session_id),
                    game_exchange_codes::build_changelist.eq(row.build_changelist),
                    game_exchange_codes::executable_sha256.eq(&row.executable_sha256),
                    game_exchange_codes::runtime_channel.eq(&row.runtime_channel),
                    game_exchange_codes::runtime_sha256.eq(&row.runtime_sha256),
                    game_exchange_codes::runtime_manifest_version.eq(&row.runtime_manifest_version),
                    game_exchange_codes::runtime_artifact_set_sha256.eq(&row.runtime_artifact_set_sha256),
                    game_exchange_codes::realm_id.eq(&row.realm_id),
                    game_exchange_codes::created_at.eq(&row.created_at),
                    game_exchange_codes::expires_at.eq(&row.expires_at),
                    game_exchange_codes::consumed_at.eq(&row.consumed_at),
                ))
                .execute(conn)?;
        }
    }

    // 6. Discord OAuth transactions
    for row in state.discord_oauth_transactions.values() {
        let updated =
            diesel::update(discord_oauth_transactions::table.filter(discord_oauth_transactions::state.eq(&row.state)))
                .set((
                    discord_oauth_transactions::code_verifier.eq(&row.code_verifier),
                    discord_oauth_transactions::created_at.eq(&row.created_at),
                    discord_oauth_transactions::expires_at.eq(&row.expires_at),
                    discord_oauth_transactions::consumed_at.eq(&row.consumed_at),
                    discord_oauth_transactions::completion_code_hash.eq(&row.completion_code_hash),
                    discord_oauth_transactions::completion_expires_at.eq(&row.completion_expires_at),
                    discord_oauth_transactions::completion_consumed_at.eq(&row.completion_consumed_at),
                    discord_oauth_transactions::account_id.eq(&row.account_id),
                    discord_oauth_transactions::link_account_id.eq(&row.link_account_id),
                ))
                .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(discord_oauth_transactions::table)
                .values((
                    discord_oauth_transactions::state.eq(&row.state),
                    discord_oauth_transactions::code_verifier.eq(&row.code_verifier),
                    discord_oauth_transactions::created_at.eq(&row.created_at),
                    discord_oauth_transactions::expires_at.eq(&row.expires_at),
                    discord_oauth_transactions::consumed_at.eq(&row.consumed_at),
                    discord_oauth_transactions::completion_code_hash.eq(&row.completion_code_hash),
                    discord_oauth_transactions::completion_expires_at.eq(&row.completion_expires_at),
                    discord_oauth_transactions::completion_consumed_at.eq(&row.completion_consumed_at),
                    discord_oauth_transactions::account_id.eq(&row.account_id),
                    discord_oauth_transactions::link_account_id.eq(&row.link_account_id),
                ))
                .execute(conn)?;
        }
    }

    // 7. Email codes
    for row in state.email_codes.values() {
        let target = email_codes::table
            .filter(email_codes::account_id.eq(&row.account_id))
            .filter(email_codes::purpose.eq(&row.purpose));
        let updated = diesel::update(target)
            .set((
                email_codes::code_hash.eq(&row.code_hash),
                email_codes::attempts.eq(row.attempts),
                email_codes::created_at.eq(&row.created_at),
                email_codes::expires_at.eq(&row.expires_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(email_codes::table)
                .values((
                    email_codes::account_id.eq(&row.account_id),
                    email_codes::purpose.eq(&row.purpose),
                    email_codes::code_hash.eq(&row.code_hash),
                    email_codes::attempts.eq(row.attempts),
                    email_codes::created_at.eq(&row.created_at),
                    email_codes::expires_at.eq(&row.expires_at),
                ))
                .execute(conn)?;
        }
    }

    // 8. API keys
    for key_hash in &state.game_server_api_keys {
        let exists = game_server_api_keys::table
            .filter(game_server_api_keys::key_hash.eq(key_hash))
            .count()
            .get_result::<i64>(conn)?;
        if exists == 0 {
            diesel::insert_into(game_server_api_keys::table)
                .values(game_server_api_keys::key_hash.eq(key_hash))
                .execute(conn)?;
        }
    }
    for (account_id, key_hash) in &state.user_api_keys {
        let updated = diesel::update(user_api_keys::table.filter(user_api_keys::account_id.eq(account_id)))
            .set(user_api_keys::key_hash.eq(key_hash))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(user_api_keys::table)
                .values((
                    user_api_keys::account_id.eq(account_id),
                    user_api_keys::key_hash.eq(key_hash),
                ))
                .execute(conn)?;
        }
    }

    // 9. Characters & Loadouts
    for row in state.characters.values() {
        let updated = diesel::update(characters::table.filter(characters::character_id.eq(&row.character_id)))
            .set((
                characters::account_id.eq(&row.account_id),
                characters::ord.eq(row.ord),
                characters::created_date.eq(&row.created_date),
                characters::last_modified_date.eq(&row.last_modified_date),
                characters::name.eq(&row.name),
                characters::update_version.eq(row.update_version),
                characters::data.eq(&row.data),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(characters::table)
                .values((
                    characters::character_id.eq(&row.character_id),
                    characters::account_id.eq(&row.account_id),
                    characters::ord.eq(row.ord),
                    characters::created_date.eq(&row.created_date),
                    characters::last_modified_date.eq(&row.last_modified_date),
                    characters::name.eq(&row.name),
                    characters::update_version.eq(row.update_version),
                    characters::data.eq(&row.data),
                ))
                .execute(conn)?;
        }
    }

    for row in state.loadouts.values() {
        let updated = diesel::update(loadouts::table.filter(loadouts::character_id.eq(&row.character_id)))
            .set((
                loadouts::account_id.eq(&row.account_id),
                loadouts::slots_json.eq(&row.slots_json),
                loadouts::persistent.eq(&row.persistent),
                loadouts::active_index.eq(row.active_index),
                loadouts::unlocked_total_slots.eq(row.unlocked_total_slots),
                loadouts::revision.eq(row.revision),
                loadouts::bootstrap_version.eq(&row.bootstrap_version),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(loadouts::table)
                .values((
                    loadouts::character_id.eq(&row.character_id),
                    loadouts::account_id.eq(&row.account_id),
                    loadouts::slots_json.eq(&row.slots_json),
                    loadouts::persistent.eq(&row.persistent),
                    loadouts::active_index.eq(row.active_index),
                    loadouts::unlocked_total_slots.eq(row.unlocked_total_slots),
                    loadouts::revision.eq(row.revision),
                    loadouts::bootstrap_version.eq(&row.bootstrap_version),
                ))
                .execute(conn)?;
        }
    }

    // 10. Inventories & Inventory transactions
    for row in state.inventories.values() {
        let updated = diesel::update(inventories::table.filter(inventories::character_id.eq(&row.character_id)))
            .set((
                inventories::account_id.eq(&row.account_id),
                inventories::instanced_items.eq(&row.instanced_items),
                inventories::stacked_items.eq(&row.stacked_items),
                inventories::revision.eq(row.revision),
                inventories::bootstrap_version.eq(&row.bootstrap_version),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(inventories::table)
                .values((
                    inventories::character_id.eq(&row.character_id),
                    inventories::account_id.eq(&row.account_id),
                    inventories::instanced_items.eq(&row.instanced_items),
                    inventories::stacked_items.eq(&row.stacked_items),
                    inventories::revision.eq(row.revision),
                    inventories::bootstrap_version.eq(&row.bootstrap_version),
                ))
                .execute(conn)?;
        }
    }

    for row in state.inventory_transactions.values() {
        let target = inventory_transactions::table
            .filter(inventory_transactions::account_id.eq(&row.account_id))
            .filter(inventory_transactions::character_id.eq(&row.character_id))
            .filter(inventory_transactions::transaction_id.eq(&row.transaction_id));
        let updated = diesel::update(target)
            .set((
                inventory_transactions::request_hash.eq(&row.request_hash),
                inventory_transactions::status.eq(&row.status),
                inventory_transactions::result.eq(&row.result),
                inventory_transactions::created_at.eq(&row.created_at),
                inventory_transactions::completed_at.eq(&row.completed_at),
                inventory_transactions::economy_source.eq(&row.economy_source),
                inventory_transactions::economy_session_id.eq(&row.economy_session_id),
                inventory_transactions::extra.eq(&row.extra),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(inventory_transactions::table)
                .values((
                    inventory_transactions::account_id.eq(&row.account_id),
                    inventory_transactions::character_id.eq(&row.character_id),
                    inventory_transactions::transaction_id.eq(&row.transaction_id),
                    inventory_transactions::request_hash.eq(&row.request_hash),
                    inventory_transactions::status.eq(&row.status),
                    inventory_transactions::result.eq(&row.result),
                    inventory_transactions::created_at.eq(&row.created_at),
                    inventory_transactions::completed_at.eq(&row.completed_at),
                    inventory_transactions::economy_source.eq(&row.economy_source),
                    inventory_transactions::economy_session_id.eq(&row.economy_session_id),
                    inventory_transactions::extra.eq(&row.extra),
                ))
                .execute(conn)?;
        }
    }

    // 11. Bounties, Cooldowns, Entitlements, Wallets, Economy rollups
    for row in state.bounty_states.values() {
        let updated = diesel::update(bounty_states::table.filter(bounty_states::account_id.eq(&row.account_id)))
            .set((
                bounty_states::bounties.eq(&row.bounties),
                bounty_states::draft_data.eq(&row.draft_data),
                bounty_states::daily_reward_credited_on.eq(&row.daily_reward_credited_on),
                bounty_states::daily_premium_draft_granted_on.eq(&row.daily_premium_draft_granted_on),
                bounty_states::update_version.eq(row.update_version),
                bounty_states::created_at.eq(&row.created_at),
                bounty_states::updated_at.eq(&row.updated_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(bounty_states::table)
                .values((
                    bounty_states::account_id.eq(&row.account_id),
                    bounty_states::bounties.eq(&row.bounties),
                    bounty_states::draft_data.eq(&row.draft_data),
                    bounty_states::daily_reward_credited_on.eq(&row.daily_reward_credited_on),
                    bounty_states::daily_premium_draft_granted_on.eq(&row.daily_premium_draft_granted_on),
                    bounty_states::update_version.eq(row.update_version),
                    bounty_states::created_at.eq(&row.created_at),
                    bounty_states::updated_at.eq(&row.updated_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.cooldown_states.values() {
        let updated = diesel::update(cooldown_states::table.filter(cooldown_states::account_id.eq(&row.account_id)))
            .set((
                cooldown_states::entries.eq(&row.entries),
                cooldown_states::update_version.eq(row.update_version),
                cooldown_states::created_at.eq(&row.created_at),
                cooldown_states::updated_at.eq(&row.updated_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(cooldown_states::table)
                .values((
                    cooldown_states::account_id.eq(&row.account_id),
                    cooldown_states::entries.eq(&row.entries),
                    cooldown_states::update_version.eq(row.update_version),
                    cooldown_states::created_at.eq(&row.created_at),
                    cooldown_states::updated_at.eq(&row.updated_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.entitlements.values() {
        let target = entitlements::table
            .filter(entitlements::account_id.eq(&row.account_id))
            .filter(entitlements::name.eq(&row.name));
        let updated = diesel::update(target)
            .set((
                entitlements::duration.eq(row.duration),
                entitlements::activated_date.eq(&row.activated_date),
                entitlements::granted_at.eq(&row.granted_at),
                entitlements::source_sku_id.eq(&row.source_sku_id),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(entitlements::table)
                .values((
                    entitlements::account_id.eq(&row.account_id),
                    entitlements::name.eq(&row.name),
                    entitlements::duration.eq(row.duration),
                    entitlements::activated_date.eq(&row.activated_date),
                    entitlements::granted_at.eq(&row.granted_at),
                    entitlements::source_sku_id.eq(&row.source_sku_id),
                ))
                .execute(conn)?;
        }
    }

    for row in state.wallets.values() {
        let updated = diesel::update(wallets::table.filter(wallets::account_id.eq(&row.account_id)))
            .set(wallets::bootstrap_version.eq(&row.bootstrap_version))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(wallets::table)
                .values((
                    wallets::account_id.eq(&row.account_id),
                    wallets::bootstrap_version.eq(&row.bootstrap_version),
                ))
                .execute(conn)?;
        }
        for (cid, balance, ord) in &row.balances {
            let target = wallet_balances::table
                .filter(wallet_balances::account_id.eq(&row.account_id))
                .filter(wallet_balances::catalog_id.eq(cid));
            let b_upd = diesel::update(target)
                .set((wallet_balances::balance.eq(*balance), wallet_balances::ordinal.eq(*ord)))
                .execute(conn)?;
            if b_upd == 0 {
                diesel::insert_into(wallet_balances::table)
                    .values((
                        wallet_balances::account_id.eq(&row.account_id),
                        wallet_balances::catalog_id.eq(cid),
                        wallet_balances::balance.eq(*balance),
                        wallet_balances::ordinal.eq(*ord),
                    ))
                    .execute(conn)?;
            }
        }
    }

    for row in state.economy_rollup_counters.values() {
        let target = economy_rollup_counters::table
            .filter(economy_rollup_counters::account_id.eq(&row.account_id))
            .filter(economy_rollup_counters::day.eq(&row.day))
            .filter(economy_rollup_counters::currency.eq(&row.currency))
            .filter(economy_rollup_counters::path.eq(&row.path));
        let updated = diesel::update(target)
            .set((
                economy_rollup_counters::value.eq(row.value),
                economy_rollup_counters::ttl_at.eq(row.ttl_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(economy_rollup_counters::table)
                .values((
                    economy_rollup_counters::account_id.eq(&row.account_id),
                    economy_rollup_counters::day.eq(&row.day),
                    economy_rollup_counters::currency.eq(&row.currency),
                    economy_rollup_counters::path.eq(&row.path),
                    economy_rollup_counters::value.eq(row.value),
                    economy_rollup_counters::ttl_at.eq(row.ttl_at),
                ))
                .execute(conn)?;
        }
    }

    // 12. Progression
    let mut next_track_seq = progression_tracks::table
        .select(diesel::dsl::max(progression_tracks::seq))
        .first::<Option<i64>>(conn)?
        .unwrap_or(0);
    for row in state.progression_tracks.values() {
        let target = progression_tracks::table
            .filter(progression_tracks::account.eq(&row.account))
            .filter(progression_tracks::progression_id.eq(&row.progression_id));
        let updated = diesel::update(target)
            .set((
                progression_tracks::progress.eq(row.progress),
                progression_tracks::confirmed_fremium_rank.eq(row.confirmed_fremium_rank),
                progression_tracks::confirmed_premium_rank.eq(row.confirmed_premium_rank),
                progression_tracks::update_version.eq(row.update_version),
                progression_tracks::created_at.eq(&row.created_at),
                progression_tracks::updated_at.eq(&row.updated_at),
                progression_tracks::reforge.eq(&row.reforge),
                progression_tracks::prestige_claimed.eq(row.prestige_claimed),
            ))
            .execute(conn)?;
        if updated == 0 {
            next_track_seq += 1;
            diesel::insert_into(progression_tracks::table)
                .values((
                    progression_tracks::account.eq(&row.account),
                    progression_tracks::progression_id.eq(&row.progression_id),
                    progression_tracks::progress.eq(row.progress),
                    progression_tracks::confirmed_fremium_rank.eq(row.confirmed_fremium_rank),
                    progression_tracks::confirmed_premium_rank.eq(row.confirmed_premium_rank),
                    progression_tracks::update_version.eq(row.update_version),
                    progression_tracks::seq.eq(next_track_seq),
                    progression_tracks::created_at.eq(&row.created_at),
                    progression_tracks::updated_at.eq(&row.updated_at),
                    progression_tracks::reforge.eq(&row.reforge),
                    progression_tracks::prestige_claimed.eq(row.prestige_claimed),
                ))
                .execute(conn)?;
        }
    }

    let mut next_obj_seq = progression_objectives::table
        .select(diesel::dsl::max(progression_objectives::seq))
        .first::<Option<i64>>(conn)?
        .unwrap_or(0);
    for row in state.progression_objectives.values() {
        let target = progression_objectives::table
            .filter(progression_objectives::account.eq(&row.account))
            .filter(progression_objectives::objective_id.eq(&row.objective_id));
        let updated = diesel::update(target)
            .set((
                progression_objectives::value.eq(row.value),
                progression_objectives::completed_count.eq(row.completed_count),
                progression_objectives::created_at.eq(&row.created_at),
                progression_objectives::updated_at.eq(&row.updated_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            next_obj_seq += 1;
            diesel::insert_into(progression_objectives::table)
                .values((
                    progression_objectives::account.eq(&row.account),
                    progression_objectives::objective_id.eq(&row.objective_id),
                    progression_objectives::value.eq(row.value),
                    progression_objectives::completed_count.eq(row.completed_count),
                    progression_objectives::seq.eq(next_obj_seq),
                    progression_objectives::created_at.eq(&row.created_at),
                    progression_objectives::updated_at.eq(&row.updated_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.progression_objective_events.values() {
        let updated =
            diesel::update(progression_objective_events::table.filter(progression_objective_events::id.eq(&row.id)))
                .set((
                    progression_objective_events::account.eq(&row.account),
                    progression_objective_events::raw_body.eq(&row.raw_body),
                    progression_objective_events::received_at.eq(&row.received_at),
                ))
                .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(progression_objective_events::table)
                .values((
                    progression_objective_events::id.eq(&row.id),
                    progression_objective_events::account.eq(&row.account),
                    progression_objective_events::raw_body.eq(&row.raw_body),
                    progression_objective_events::received_at.eq(&row.received_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.progression_grants.values() {
        let updated = diesel::update(progression_grants::table.filter(progression_grants::grant_id.eq(&row.grant_id)))
            .set((
                progression_grants::account.eq(&row.account),
                progression_grants::progression_id.eq(&row.progression_id),
                progression_grants::amount.eq(row.amount),
                progression_grants::request_hash.eq(&row.request_hash),
                progression_grants::status.eq(&row.status),
                progression_grants::result.eq(&row.result),
                progression_grants::created_at.eq(&row.created_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(progression_grants::table)
                .values((
                    progression_grants::grant_id.eq(&row.grant_id),
                    progression_grants::account.eq(&row.account),
                    progression_grants::progression_id.eq(&row.progression_id),
                    progression_grants::amount.eq(row.amount),
                    progression_grants::request_hash.eq(&row.request_hash),
                    progression_grants::status.eq(&row.status),
                    progression_grants::result.eq(&row.result),
                    progression_grants::created_at.eq(&row.created_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.escalation_progress.values() {
        let target = escalation_progress::table
            .filter(escalation_progress::account.eq(&row.account))
            .filter(escalation_progress::season_id.eq(&row.season_id));
        let updated = diesel::update(target)
            .set((
                escalation_progress::escalation_level.eq(row.escalation_level),
                escalation_progress::next_level_xp.eq(row.next_level_xp),
                escalation_progress::talents_progress.eq(&row.talents_progress),
                escalation_progress::unlock_progress.eq(&row.unlock_progress),
                escalation_progress::update_version.eq(row.update_version),
                escalation_progress::created_at.eq(&row.created_at),
                escalation_progress::updated_at.eq(&row.updated_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(escalation_progress::table)
                .values((
                    escalation_progress::account.eq(&row.account),
                    escalation_progress::season_id.eq(&row.season_id),
                    escalation_progress::escalation_level.eq(row.escalation_level),
                    escalation_progress::next_level_xp.eq(row.next_level_xp),
                    escalation_progress::talents_progress.eq(&row.talents_progress),
                    escalation_progress::unlock_progress.eq(&row.unlock_progress),
                    escalation_progress::update_version.eq(row.update_version),
                    escalation_progress::created_at.eq(&row.created_at),
                    escalation_progress::updated_at.eq(&row.updated_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.player_journey.values() {
        let updated = diesel::update(player_journey::table.filter(player_journey::account.eq(&row.account)))
            .set((
                player_journey::nodes.eq(&row.nodes),
                player_journey::update_version.eq(row.update_version),
                player_journey::revision.eq(row.revision),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(player_journey::table)
                .values((
                    player_journey::account.eq(&row.account),
                    player_journey::nodes.eq(&row.nodes),
                    player_journey::update_version.eq(row.update_version),
                    player_journey::revision.eq(row.revision),
                ))
                .execute(conn)?;
        }
    }

    for row in state.breadcrumbs.values() {
        let updated = diesel::update(breadcrumbs::table.filter(breadcrumbs::character_id.eq(&row.character_id)))
            .set((
                breadcrumbs::account.eq(&row.account),
                breadcrumbs::items.eq(&row.items),
                breadcrumbs::update_version.eq(row.update_version),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(breadcrumbs::table)
                .values((
                    breadcrumbs::character_id.eq(&row.character_id),
                    breadcrumbs::account.eq(&row.account),
                    breadcrumbs::items.eq(&row.items),
                    breadcrumbs::update_version.eq(row.update_version),
                ))
                .execute(conn)?;
        }
    }

    for row in state.encountered_content.values() {
        let updated =
            diesel::update(encountered_content::table.filter(encountered_content::character_id.eq(&row.character_id)))
                .set((
                    encountered_content::account.eq(&row.account),
                    encountered_content::content.eq(&row.content),
                ))
                .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(encountered_content::table)
                .values((
                    encountered_content::character_id.eq(&row.character_id),
                    encountered_content::account.eq(&row.account),
                    encountered_content::content.eq(&row.content),
                ))
                .execute(conn)?;
        }
    }

    // 13. Parties & Friendships
    for row in state.parties.values() {
        let updated = diesel::update(parties::table.filter(parties::id.eq(&row.id)))
            .set((
                parties::client_id.eq(&row.client_id),
                parties::revision.eq(row.revision),
                parties::leader.eq(&row.leader),
                parties::build_id.eq(&row.build_id),
                parties::activity.eq(&row.activity),
                parties::active_hunt_session_id.eq(&row.active_hunt_session_id),
                parties::created_at.eq(&row.created_at),
                parties::updated_at.eq(&row.updated_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(parties::table)
                .values((
                    parties::id.eq(&row.id),
                    parties::client_id.eq(&row.client_id),
                    parties::revision.eq(row.revision),
                    parties::leader.eq(&row.leader),
                    parties::build_id.eq(&row.build_id),
                    parties::activity.eq(&row.activity),
                    parties::active_hunt_session_id.eq(&row.active_hunt_session_id),
                    parties::created_at.eq(&row.created_at),
                    parties::updated_at.eq(&row.updated_at),
                ))
                .execute(conn)?;
        }
        for (member_acc, pos, joined_at) in &row.members {
            let m_upd = diesel::update(party_members::table.filter(party_members::account.eq(member_acc)))
                .set((
                    party_members::party.eq(&row.id),
                    party_members::position.eq(*pos),
                    party_members::joined_at.eq(joined_at),
                ))
                .execute(conn)?;
            if m_upd == 0 {
                diesel::insert_into(party_members::table)
                    .values((
                        party_members::account.eq(member_acc),
                        party_members::party.eq(&row.id),
                        party_members::position.eq(*pos),
                        party_members::joined_at.eq(joined_at),
                    ))
                    .execute(conn)?;
            }
        }
    }

    let mut next_invite_seq = party_invites::table
        .select(diesel::dsl::max(party_invites::seq))
        .first::<Option<i64>>(conn)?
        .unwrap_or(0);
    for row in state.party_invites.values() {
        let target = party_invites::table
            .filter(party_invites::recipient.eq(&row.recipient))
            .filter(party_invites::sender.eq(&row.sender));
        let updated = diesel::update(target)
            .set((
                party_invites::party.eq(&row.party),
                party_invites::party_revision.eq(row.party_revision),
                party_invites::sending_display_name.eq(&row.sending_display_name),
                party_invites::sending_platform.eq(&row.sending_platform),
                party_invites::status.eq(&row.status),
                party_invites::created_at.eq(&row.created_at),
                party_invites::expires_at.eq(&row.expires_at),
                party_invites::consumed_at.eq(&row.consumed_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            next_invite_seq += 1;
            diesel::insert_into(party_invites::table)
                .values((
                    party_invites::recipient.eq(&row.recipient),
                    party_invites::sender.eq(&row.sender),
                    party_invites::party.eq(&row.party),
                    party_invites::party_revision.eq(row.party_revision),
                    party_invites::sending_display_name.eq(&row.sending_display_name),
                    party_invites::sending_platform.eq(&row.sending_platform),
                    party_invites::status.eq(&row.status),
                    party_invites::created_at.eq(&row.created_at),
                    party_invites::expires_at.eq(&row.expires_at),
                    party_invites::consumed_at.eq(&row.consumed_at),
                    party_invites::seq.eq(next_invite_seq),
                ))
                .execute(conn)?;
        }
    }

    let mut next_outbox_seq = party_outbox::table
        .select(diesel::dsl::max(party_outbox::seq))
        .first::<Option<i64>>(conn)?
        .unwrap_or(0);
    for row in state.party_outbox.values() {
        let updated = diesel::update(party_outbox::table.filter(party_outbox::id.eq(&row.id)))
            .set((
                party_outbox::party.eq(&row.party),
                party_outbox::party_revision.eq(row.party_revision),
                party_outbox::event_type.eq(&row.event_type),
                party_outbox::recipients.eq(&row.recipients),
                party_outbox::payload.eq(&row.payload),
                party_outbox::state.eq(&row.state),
                party_outbox::created_at.eq(&row.created_at),
                party_outbox::claimed_at.eq(&row.claimed_at),
                party_outbox::claim_owner.eq(&row.claim_owner),
                party_outbox::published_at.eq(&row.published_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            next_outbox_seq += 1;
            diesel::insert_into(party_outbox::table)
                .values((
                    party_outbox::id.eq(&row.id),
                    party_outbox::party.eq(&row.party),
                    party_outbox::party_revision.eq(row.party_revision),
                    party_outbox::event_type.eq(&row.event_type),
                    party_outbox::recipients.eq(&row.recipients),
                    party_outbox::payload.eq(&row.payload),
                    party_outbox::state.eq(&row.state),
                    party_outbox::created_at.eq(&row.created_at),
                    party_outbox::claimed_at.eq(&row.claimed_at),
                    party_outbox::claim_owner.eq(&row.claim_owner),
                    party_outbox::published_at.eq(&row.published_at),
                    party_outbox::seq.eq(next_outbox_seq),
                ))
                .execute(conn)?;
        }
    }

    for row in state.friend_edges.values() {
        let target = friend_edges::table
            .filter(friend_edges::owner.eq(&row.owner))
            .filter(friend_edges::other.eq(&row.other));
        let updated = diesel::update(target)
            .set((
                friend_edges::other_user_id.eq(&row.other_user_id),
                friend_edges::status.eq(&row.status),
                friend_edges::direction.eq(&row.direction),
                friend_edges::created.eq(&row.created),
                friend_edges::favorite.eq(row.favorite),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(friend_edges::table)
                .values((
                    friend_edges::owner.eq(&row.owner),
                    friend_edges::other.eq(&row.other),
                    friend_edges::other_user_id.eq(&row.other_user_id),
                    friend_edges::status.eq(&row.status),
                    friend_edges::direction.eq(&row.direction),
                    friend_edges::created.eq(&row.created),
                    friend_edges::favorite.eq(row.favorite),
                ))
                .execute(conn)?;
        }
    }

    // 14. Reforged API, Admin, Notices, Operations Policy, Guard
    for row in state.reforged_invites.values() {
        let updated = diesel::update(reforged_invites::table.filter(reforged_invites::code.eq(&row.code)))
            .set((
                reforged_invites::uses_remaining.eq(row.uses_remaining),
                reforged_invites::infinite_uses.eq(row.infinite_uses),
                reforged_invites::note.eq(&row.note),
                reforged_invites::created_at.eq(&row.created_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(reforged_invites::table)
                .values((
                    reforged_invites::code.eq(&row.code),
                    reforged_invites::uses_remaining.eq(row.uses_remaining),
                    reforged_invites::infinite_uses.eq(row.infinite_uses),
                    reforged_invites::note.eq(&row.note),
                    reforged_invites::created_at.eq(&row.created_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.admin_sessions.values() {
        let updated = diesel::update(admin_sessions::table.filter(admin_sessions::id.eq(&row.id)))
            .set((
                admin_sessions::token_hash.eq(&row.token_hash),
                admin_sessions::account.eq(&row.account),
                admin_sessions::csrf_token.eq(&row.csrf_token),
                admin_sessions::created_at.eq(&row.created_at),
                admin_sessions::expires_at.eq(&row.expires_at),
                admin_sessions::ip.eq(&row.ip),
                admin_sessions::user_agent.eq(&row.user_agent),
                admin_sessions::revoked_at.eq(&row.revoked_at),
                admin_sessions::ttl_at.eq(row.ttl_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(admin_sessions::table)
                .values((
                    admin_sessions::id.eq(&row.id),
                    admin_sessions::token_hash.eq(&row.token_hash),
                    admin_sessions::account.eq(&row.account),
                    admin_sessions::csrf_token.eq(&row.csrf_token),
                    admin_sessions::created_at.eq(&row.created_at),
                    admin_sessions::expires_at.eq(&row.expires_at),
                    admin_sessions::ip.eq(&row.ip),
                    admin_sessions::user_agent.eq(&row.user_agent),
                    admin_sessions::revoked_at.eq(&row.revoked_at),
                    admin_sessions::ttl_at.eq(row.ttl_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.admin_audit.values() {
        let updated = diesel::update(admin_audit::table.filter(admin_audit::id.eq(&row.id)))
            .set((
                admin_audit::actor_user_id.eq(&row.actor_user_id),
                admin_audit::target_user_id.eq(&row.target_user_id),
                admin_audit::action.eq(&row.action),
                admin_audit::old_state.eq(&row.old_state),
                admin_audit::new_state.eq(&row.new_state),
                admin_audit::reason.eq(&row.reason),
                admin_audit::ip.eq(&row.ip),
                admin_audit::request_id.eq(&row.request_id),
                admin_audit::created_at.eq(&row.created_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(admin_audit::table)
                .values((
                    admin_audit::id.eq(&row.id),
                    admin_audit::actor_user_id.eq(&row.actor_user_id),
                    admin_audit::target_user_id.eq(&row.target_user_id),
                    admin_audit::action.eq(&row.action),
                    admin_audit::old_state.eq(&row.old_state),
                    admin_audit::new_state.eq(&row.new_state),
                    admin_audit::reason.eq(&row.reason),
                    admin_audit::ip.eq(&row.ip),
                    admin_audit::request_id.eq(&row.request_id),
                    admin_audit::created_at.eq(&row.created_at),
                ))
                .execute(conn)?;
        }
    }

    if let Some(row) = &state.launcher_notices {
        let updated = diesel::update(launcher_notices::table.filter(launcher_notices::id.eq(row.id)))
            .set((
                launcher_notices::version.eq(row.version),
                launcher_notices::updated_at.eq(&row.updated_at),
                launcher_notices::updated_by.eq(&row.updated_by),
                launcher_notices::announcements.eq(&row.announcements),
                launcher_notices::tags.eq(&row.tags),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(launcher_notices::table)
                .values((
                    launcher_notices::id.eq(row.id),
                    launcher_notices::version.eq(row.version),
                    launcher_notices::updated_at.eq(&row.updated_at),
                    launcher_notices::updated_by.eq(&row.updated_by),
                    launcher_notices::announcements.eq(&row.announcements),
                    launcher_notices::tags.eq(&row.tags),
                ))
                .execute(conn)?;
        }
    }

    if let Some(row) = &state.operations_policy {
        let updated = diesel::update(operations_policy::table.filter(operations_policy::id.eq(row.id)))
            .set((
                operations_policy::guard_enforcement.eq(&row.guard_enforcement),
                operations_policy::diagnostics_profile.eq(&row.diagnostics_profile),
                operations_policy::version.eq(row.version),
                operations_policy::updated_at.eq(&row.updated_at),
                operations_policy::updated_by.eq(&row.updated_by),
                operations_policy::diagnostics_expires_at.eq(&row.diagnostics_expires_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(operations_policy::table)
                .values((
                    operations_policy::id.eq(row.id),
                    operations_policy::guard_enforcement.eq(&row.guard_enforcement),
                    operations_policy::diagnostics_profile.eq(&row.diagnostics_profile),
                    operations_policy::version.eq(row.version),
                    operations_policy::updated_at.eq(&row.updated_at),
                    operations_policy::updated_by.eq(&row.updated_by),
                    operations_policy::diagnostics_expires_at.eq(&row.diagnostics_expires_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.launcher_guard_sessions.values() {
        let updated = diesel::update(
            launcher_guard_sessions::table.filter(launcher_guard_sessions::guard_session_id.eq(&row.guard_session_id)),
        )
        .set((
            launcher_guard_sessions::account.eq(&row.account),
            launcher_guard_sessions::role.eq(&row.role),
            launcher_guard_sessions::public_key_base64.eq(&row.public_key_base64),
            launcher_guard_sessions::manifest_id.eq(&row.manifest_id),
            launcher_guard_sessions::manifest_sequence.eq(row.manifest_sequence),
            launcher_guard_sessions::channel.eq(&row.channel),
            launcher_guard_sessions::challenge.eq(&row.challenge),
            launcher_guard_sessions::last_sequence.eq(row.last_sequence),
            launcher_guard_sessions::status.eq(&row.status),
            launcher_guard_sessions::risk_score.eq(row.risk_score),
            launcher_guard_sessions::created_at.eq(&row.created_at),
            launcher_guard_sessions::expires_at.eq(&row.expires_at),
            launcher_guard_sessions::last_heartbeat_at.eq(&row.last_heartbeat_at),
            launcher_guard_sessions::detail.eq(&row.detail),
            launcher_guard_sessions::ttl_at.eq(row.ttl_at),
        ))
        .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(launcher_guard_sessions::table)
                .values((
                    launcher_guard_sessions::guard_session_id.eq(&row.guard_session_id),
                    launcher_guard_sessions::account.eq(&row.account),
                    launcher_guard_sessions::role.eq(&row.role),
                    launcher_guard_sessions::public_key_base64.eq(&row.public_key_base64),
                    launcher_guard_sessions::manifest_id.eq(&row.manifest_id),
                    launcher_guard_sessions::manifest_sequence.eq(row.manifest_sequence),
                    launcher_guard_sessions::channel.eq(&row.channel),
                    launcher_guard_sessions::challenge.eq(&row.challenge),
                    launcher_guard_sessions::last_sequence.eq(row.last_sequence),
                    launcher_guard_sessions::status.eq(&row.status),
                    launcher_guard_sessions::risk_score.eq(row.risk_score),
                    launcher_guard_sessions::created_at.eq(&row.created_at),
                    launcher_guard_sessions::expires_at.eq(&row.expires_at),
                    launcher_guard_sessions::last_heartbeat_at.eq(&row.last_heartbeat_at),
                    launcher_guard_sessions::detail.eq(&row.detail),
                    launcher_guard_sessions::ttl_at.eq(row.ttl_at),
                ))
                .execute(conn)?;
        }
    }

    for row in state.guard_findings.values() {
        let target = guard_findings::table
            .filter(guard_findings::file_name.eq(&row.file_name))
            .filter(guard_findings::sha256.eq(&row.sha256));
        let updated = diesel::update(target)
            .set((
                guard_findings::signer.eq(&row.signer),
                guard_findings::size.eq(row.size),
                guard_findings::first_seen_at.eq(&row.first_seen_at),
                guard_findings::last_seen_at.eq(&row.last_seen_at),
                guard_findings::session_count.eq(row.session_count),
                guard_findings::account_count.eq(row.account_count),
                guard_findings::account_ids.eq(&row.account_ids),
                guard_findings::ttl_at.eq(row.ttl_at),
            ))
            .execute(conn)?;
        if updated == 0 {
            diesel::insert_into(guard_findings::table)
                .values((
                    guard_findings::file_name.eq(&row.file_name),
                    guard_findings::sha256.eq(&row.sha256),
                    guard_findings::signer.eq(&row.signer),
                    guard_findings::size.eq(row.size),
                    guard_findings::first_seen_at.eq(&row.first_seen_at),
                    guard_findings::last_seen_at.eq(&row.last_seen_at),
                    guard_findings::session_count.eq(row.session_count),
                    guard_findings::account_count.eq(row.account_count),
                    guard_findings::account_ids.eq(&row.account_ids),
                    guard_findings::ttl_at.eq(row.ttl_at),
                ))
                .execute(conn)?;
        }
    }

    Ok(())
}

/// Runs `isles-db migrate` end-to-end.
pub async fn run_migrate(opts: &MigrateOptions) -> Result<MigrateReport, DbError> {
    let mut client = MongoClient::connect(&opts.from_uri)?;

    if opts.dry_run {
        let (_state, report) = extract_from_mongo(&mut client, HashMap::new(), HashMap::new(), opts)?;
        return Ok(report);
    }

    let db = open_target_database(&opts.to_target, opts.allow_remote_postgres)?;
    let (existing_users, existing_parties) = db.read(load_existing_id_maps).await?;
    let (state, report) = extract_from_mongo(&mut client, existing_users, existing_parties, opts)?;

    db.write(store::Actor::System("migrate".into()), move |tx| {
        write_target_state(tx.conn(), &state).map_err(DbError::from)
    })
    .await?;

    Ok(report)
}
