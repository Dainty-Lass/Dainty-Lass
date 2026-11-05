// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The tables the tool copies, declared here from the store spec §4 (spec §14.2: the repositories' revision
//! guards are wrong for a bulk copy), and the two things done with them: the one-transaction write of §9 and
//! the read-back of §10.2. Columns are listed rather than `table!`-declared, and every value is bound
//! (`store::portable`'s `?` numbering), so both engines run the same SQL.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use diesel::QueryableByName;
use diesel::prelude::*;
use diesel::query_builder::{BoxedSqlQuery, SqlQuery};
use diesel::sql_types::{BigInt, Bool, Double, Nullable, Text};
use serde_json::{Map, Value};
use store::portable::quote_ident;
use store::{AnyConnection, MultiBackend};

use crate::{Area, DbError};

pub(crate) type Doc = Map<String, Value>;
pub(crate) type Rows = BTreeMap<Vec<String>, Doc>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ty {
    T,
    I,
    F,
    B,
}

pub(crate) struct Col {
    /// The canonical record's field (§10.4)
    pub field: &'static str,
    pub column: &'static str,
    pub ty: Ty,
}

const fn t(field: &'static str) -> Col {
    Col {
        field,
        column: field,
        ty: Ty::T,
    }
}
const fn i(field: &'static str) -> Col {
    Col {
        field,
        column: field,
        ty: Ty::I,
    }
}
const fn f(field: &'static str) -> Col {
    Col {
        field,
        column: field,
        ty: Ty::F,
    }
}
const fn b(field: &'static str) -> Col {
    Col {
        field,
        column: field,
        ty: Ty::B,
    }
}

pub(crate) struct Table {
    pub name: &'static str,
    pub area: Area,
    /// The key is the first `key` columns
    pub cols: &'static [Col],
    pub key: usize,
    pub soft: bool,
    pub seq: bool,
}

use Area::{Auth, Bounties, Characters, Inventory, Parties, Progression, Reforged};

/// The 37 tables in verify's order (§10.3): area order, then the order given there.
#[rustfmt::skip]
pub(crate) const TABLES: &[Table] = &[
    Table { name: "accounts", area: Auth, key: 1, soft: true, seq: false,
        cols: &[t("id"), t("user_id"), t("name"), i("notes")] },
    Table { name: "launcher_accounts", area: Auth, key: 1, soft: true, seq: false, cols: &[
        t("account_id"), t("email"), t("display_name_normalized"), t("display_name"), t("password_hash"),
        t("status"), t("approval_status"), t("approval_updated_at"), t("approval_updated_by"),
        t("approval_reason"), t("roles"), t("roles_updated_at"), t("roles_updated_by"), t("created_at"),
        t("last_login_at"), i("username_set"), t("consent_terms_version"), t("consent_privacy_version"),
        t("consent_accepted_at"), t("deletion_requested_at"), t("deletion_scheduled_for"), t("email_verified_at"),
        t("email_verification_pending_since")] },
    Table { name: "auth_identities", area: Auth, key: 2, soft: false, seq: false, cols: &[
        t("provider"), t("provider_subject"), t("account_id"), t("provider_username"), t("provider_avatar_url"),
        t("linked_at")] },
    Table { name: "refresh_sessions", area: Auth, key: 1, soft: false, seq: false, cols: &[
        t("id"), t("token_hash"), t("account_id"), t("family_id"), t("device_id"), t("device_name"),
        t("created_at"), t("expires_at"), t("revoked_at")] },
    Table { name: "game_exchange_codes", area: Auth, key: 1, soft: false, seq: false, cols: &[
        t("code_hash"), t("account_id"), t("launcher_session_id"), i("build_changelist"), t("executable_sha256"),
        t("runtime_channel"), t("runtime_sha256"), t("runtime_manifest_version"), t("runtime_artifact_set_sha256"),
        t("realm_id"), t("created_at"), t("expires_at"), t("consumed_at")] },
    Table { name: "discord_oauth_transactions", area: Auth, key: 1, soft: false, seq: false, cols: &[
        t("state"), t("code_verifier"), t("created_at"), t("expires_at"), t("consumed_at"),
        t("completion_code_hash"), t("completion_expires_at"), t("completion_consumed_at"), t("account_id"),
        t("link_account_id")] },
    Table { name: "email_codes", area: Auth, key: 2, soft: false, seq: false, cols: &[
        t("account_id"), t("purpose"), t("code_hash"), i("attempts"), t("created_at"), t("expires_at")] },
    Table { name: "game_server_api_keys", area: Auth, key: 1, soft: false, seq: false, cols: &[t("key_hash")] },
    Table { name: "user_api_keys", area: Auth, key: 1, soft: false, seq: false,
        cols: &[t("account_id"), t("key_hash")] },
    Table { name: "characters", area: Characters, key: 1, soft: true, seq: false, cols: &[
        t("character_id"), t("account_id"), i("ord"), t("created_date"), t("last_modified_date"), t("name"),
        i("update_version"), t("data")] },
    Table { name: "loadouts", area: Characters, key: 1, soft: true, seq: false, cols: &[
        t("character_id"), t("account_id"), Col { field: "slots_json", column: "loadouts", ty: Ty::T },
        t("persistent"), i("active_index"), i("unlocked_total_slots"), i("revision"), t("bootstrap_version")] },
    Table { name: "inventories", area: Inventory, key: 1, soft: true, seq: false, cols: &[
        t("character_id"), t("account_id"), t("instanced_items"), t("stacked_items"), i("revision"),
        t("bootstrap_version")] },
    Table { name: "inventory_transactions", area: Inventory, key: 3, soft: false, seq: false, cols: &[
        t("account_id"), t("character_id"), t("transaction_id"), t("request_hash"), t("status"), t("result"),
        t("created_at"), t("completed_at"), t("economy_source"), t("economy_session_id"), t("extra")] },
    Table { name: "progression_tracks", area: Progression, key: 2, soft: true, seq: true, cols: &[
        t("account"), t("progression_id"), i("progress"), i("confirmed_fremium_rank"), i("confirmed_premium_rank"),
        i("update_version"), t("created_at"), t("updated_at"), t("reforge"), i("prestige_claimed")] },
    Table { name: "progression_objectives", area: Progression, key: 2, soft: true, seq: true, cols: &[
        t("account"), t("objective_id"), i("value"), i("completed_count"), t("created_at"), t("updated_at")] },
    Table { name: "progression_objective_events", area: Progression, key: 1, soft: false, seq: false,
        cols: &[t("id"), t("account"), t("raw_body"), t("received_at")] },
    Table { name: "progression_grants", area: Progression, key: 1, soft: false, seq: false, cols: &[
        t("grant_id"), t("account"), t("progression_id"), i("amount"), t("request_hash"), t("status"), t("result"),
        t("created_at")] },
    Table { name: "escalation_progress", area: Progression, key: 2, soft: true, seq: false, cols: &[
        t("account"), t("season_id"), i("escalation_level"), i("next_level_xp"), t("talents_progress"),
        t("unlock_progress"), i("update_version"), t("created_at"), t("updated_at")] },
    Table { name: "player_journey", area: Progression, key: 1, soft: true, seq: false,
        cols: &[t("account"), t("nodes"), i("update_version"), i("revision")] },
    Table { name: "breadcrumbs", area: Progression, key: 1, soft: true, seq: false,
        cols: &[t("character_id"), t("account"), t("items"), i("update_version")] },
    Table { name: "encountered_content", area: Progression, key: 1, soft: true, seq: false,
        cols: &[t("character_id"), t("account"), t("content")] },
    Table { name: "bounty_states", area: Bounties, key: 1, soft: true, seq: false, cols: &[
        t("account_id"), t("bounties"), t("draft_data"), t("daily_reward_credited_on"),
        t("daily_premium_draft_granted_on"), i("update_version"), t("created_at"), t("updated_at")] },
    Table { name: "cooldown_states", area: Bounties, key: 1, soft: true, seq: false,
        cols: &[t("account_id"), t("entries"), i("update_version"), t("created_at"), t("updated_at")] },
    Table { name: "entitlements", area: Bounties, key: 2, soft: true, seq: false, cols: &[
        t("account_id"), t("name"), f("duration"), t("activated_date"), t("granted_at"), t("source_sku_id")] },
    // `balances` is the child table's, attached on read
    Table { name: "wallets", area: Bounties, key: 1, soft: true, seq: false,
        cols: &[t("account_id"), t("bootstrap_version")] },
    Table { name: "economy_rollup_counters", area: Bounties, key: 4, soft: false, seq: false, cols: &[
        t("account_id"), t("day"), t("currency"), t("path"), f("value"), i("ttl_at")] },
    // `members` is the child table's
    Table { name: "parties", area: Parties, key: 1, soft: false, seq: false, cols: &[
        t("id"), t("client_id"), i("revision"), t("leader"), t("build_id"), t("activity"),
        t("active_hunt_session_id"), t("created_at"), t("updated_at")] },
    Table { name: "party_invites", area: Parties, key: 2, soft: false, seq: true, cols: &[
        t("recipient"), t("sender"), t("party"), i("party_revision"), t("sending_display_name"),
        t("sending_platform"), t("status"), t("created_at"), t("expires_at"), t("consumed_at")] },
    Table { name: "party_outbox", area: Parties, key: 1, soft: false, seq: true, cols: &[
        t("id"), t("party"), i("party_revision"), t("event_type"), t("recipients"), t("payload"), t("state"),
        t("created_at"), t("claimed_at"), t("claim_owner"), t("published_at")] },
    Table { name: "friend_edges", area: Parties, key: 2, soft: false, seq: false, cols: &[
        t("owner"), t("other"), t("other_user_id"), t("status"), t("direction"), t("created"), b("favorite")] },
    Table { name: "reforged_invites", area: Reforged, key: 1, soft: false, seq: false, cols: &[
        t("code"), i("uses_remaining"), b("infinite_uses"), t("note"), t("created_at")] },
    Table { name: "admin_sessions", area: Reforged, key: 1, soft: false, seq: false, cols: &[
        t("id"), t("token_hash"), t("account"), t("csrf_token"), t("created_at"), t("expires_at"), t("ip"),
        t("user_agent"), t("revoked_at"), i("ttl_at")] },
    Table { name: "admin_audit", area: Reforged, key: 1, soft: false, seq: false, cols: &[
        t("id"), t("actor_user_id"), t("target_user_id"), t("action"), t("old_state"), t("new_state"), t("reason"),
        t("ip"), t("request_id"), t("created_at")] },
    Table { name: "launcher_notices", area: Reforged, key: 1, soft: false, seq: false, cols: &[
        i("id"), i("version"), t("updated_at"), t("updated_by"), t("announcements"), t("tags")] },
    Table { name: "operations_policy", area: Reforged, key: 1, soft: false, seq: false, cols: &[
        i("id"), t("guard_enforcement"), t("diagnostics_profile"), i("version"), t("updated_at"), t("updated_by"),
        t("diagnostics_expires_at")] },
    Table { name: "launcher_guard_sessions", area: Reforged, key: 1, soft: false, seq: false, cols: &[
        t("guard_session_id"), t("account"), t("role"), t("public_key_base64"), t("manifest_id"),
        i("manifest_sequence"), t("channel"), t("challenge"), i("last_sequence"), t("status"), f("risk_score"),
        t("created_at"), t("expires_at"), t("last_heartbeat_at"), t("detail"), i("ttl_at")] },
    Table { name: "guard_findings", area: Reforged, key: 2, soft: false, seq: false, cols: &[
        t("file_name"), t("sha256"), t("signer"), i("size"), t("first_seen_at"), t("last_seen_at"),
        i("session_count"), i("account_count"), t("account_ids"), i("ttl_at")] },
];

const WALLET_BALANCES: Table = Table {
    name: "wallet_balances",
    area: Bounties,
    key: 2,
    soft: true,
    seq: false,
    cols: &[t("account_id"), t("catalog_id"), f("balance"), i("ordinal")],
};

/// Keyed by `account` alone: a member listed in two parties ends in the later one (§9.2).
const PARTY_MEMBERS: Table = Table {
    name: "party_members",
    area: Parties,
    key: 1,
    soft: false,
    seq: false,
    cols: &[t("account"), t("party"), i("position"), t("joined_at")],
};

/// §9.2's order: auth, characters, inventory, bounties, progression, parties, reforged.
const WRITE_AREAS: [Area; 7] = [Auth, Characters, Inventory, Bounties, Progression, Parties, Reforged];

pub(crate) fn table(name: &str) -> Option<&'static Table> {
    TABLES.iter().find(|t| t.name == name)
}

/// Rows of every table, keyed as §9.2 orders them; what migrate writes, verify compares and export reads.
#[derive(Clone, Default, PartialEq)]
pub struct TargetState {
    pub(crate) tables: BTreeMap<&'static str, Rows>,
    /// Accounts whose row is a placeholder: inserted when missing, never updated (coordinator item 3)
    pub(crate) implicit: BTreeSet<String>,
}

// By hand: the records hold password, token and key hashes (the decision "secrets never in Debug")
impl fmt::Debug for TargetState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let counts: BTreeMap<_, _> = self.tables.iter().map(|(k, v)| (*k, v.len())).collect();
        f.debug_struct("TargetState")
            .field("counts", &counts)
            .field("implicit", &self.implicit.len())
            .finish()
    }
}

impl TargetState {
    pub(crate) fn rows(&self, table: &str) -> impl Iterator<Item = (&Vec<String>, &Doc)> {
        self.tables.get(table).into_iter().flatten()
    }

    pub(crate) fn get(&self, table: &str, key: &[String]) -> Option<&Doc> {
        self.tables.get(table)?.get(key)
    }

    pub(crate) fn count(&self, table: &str) -> usize {
        self.tables.get(table).map_or(0, BTreeMap::len)
    }

    /// Puts a row under its key (the first `key` fields' texts); a later row with the same key replaces it.
    pub(crate) fn put(&mut self, table: &'static Table, row: Doc) {
        let key = key_of(table, &row);
        self.tables.entry(table.name).or_default().insert(key, row);
    }
}

/// A float column's value. JSON has no infinity, so a non-finite one (`"inf"` is accepted, P13) is carried as
/// its text, written back as the float, and read back the same way.
pub(crate) fn float(f: f64) -> Value {
    if f.is_finite() {
        Value::from(f)
    } else {
        Value::String(f.to_string())
    }
}

pub(crate) fn key_of(table: &Table, row: &Doc) -> Vec<String> {
    table.cols[..table.key]
        .iter()
        .map(|c| match row.get(c.field) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        })
        .collect()
}

// ------------------------------------------------------------------ bound SQL

enum Param {
    Text(Option<String>),
    Int(Option<i64>),
    Float(Option<f64>),
    Bool(Option<bool>),
}

fn param(ty: Ty, value: Option<&Value>) -> Param {
    let value = value.filter(|v| !v.is_null());
    match ty {
        Ty::T => Param::Text(value.map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })),
        Ty::I => Param::Int(value.and_then(Value::as_i64)),
        Ty::F => Param::Float(value.and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))),
        Ty::B => Param::Bool(value.and_then(Value::as_bool)),
    }
}

fn query(text: &str, params: Vec<Param>) -> Result<BoxedSqlQuery<'static, MultiBackend, SqlQuery>, DbError> {
    let text = store::portable::numbered(text).map_err(DbError::Config)?;
    let mut q = diesel::sql_query(text).into_boxed::<MultiBackend>();
    for p in params {
        q = match p {
            Param::Text(v) => q.bind::<Nullable<Text>, _>(v),
            Param::Int(v) => q.bind::<Nullable<BigInt>, _>(v),
            Param::Float(v) => q.bind::<Nullable<Double>, _>(v),
            Param::Bool(v) => q.bind::<Nullable<Bool>, _>(v),
        };
    }
    Ok(q)
}

#[derive(QueryableByName)]
struct IntRow {
    #[diesel(sql_type = Nullable<BigInt>)]
    n: Option<i64>,
}

fn key_params(table: &Table, row: &Doc) -> Vec<Param> {
    table.cols[..table.key]
        .iter()
        .map(|c| param(c.ty, row.get(c.field)))
        .collect()
}

fn where_key(table: &Table) -> String {
    table.cols[..table.key]
        .iter()
        .map(|c| format!("{} = ?", quote_ident(c.column)))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn exists(conn: &mut AnyConnection, table: &Table, row: &Doc) -> Result<bool, DbError> {
    let sql = format!(
        "SELECT 1 AS n FROM {} WHERE {}",
        quote_ident(table.name),
        where_key(table)
    );
    Ok(!query(&sql, key_params(table, row))?.load::<IntRow>(conn)?.is_empty())
}

fn insert(conn: &mut AnyConnection, table: &Table, row: &Doc, seq: Option<i64>) -> Result<(), DbError> {
    let mut columns: Vec<String> = table.cols.iter().map(|c| quote_ident(c.column)).collect();
    let mut params: Vec<Param> = table.cols.iter().map(|c| param(c.ty, row.get(c.field))).collect();
    if let Some(seq) = seq {
        columns.push(quote_ident("seq"));
        params.push(Param::Int(Some(seq)));
    }
    let marks = vec!["?"; columns.len()].join(", ");
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({marks})",
        quote_ident(table.name),
        columns.join(", ")
    );
    query(&sql, params)?.execute(conn)?;
    Ok(())
}

/// Update by key; when no row matched, insert. Returns whether it inserted.
fn upsert(conn: &mut AnyConnection, table: &Table, row: &Doc, seq: &mut Option<i64>) -> Result<bool, DbError> {
    let rest = &table.cols[table.key..];
    if !rest.is_empty() {
        let set = rest
            .iter()
            .map(|c| format!("{} = ?", quote_ident(c.column)))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE {} SET {set} WHERE {}",
            quote_ident(table.name),
            where_key(table)
        );
        let mut params: Vec<Param> = rest.iter().map(|c| param(c.ty, row.get(c.field))).collect();
        params.extend(key_params(table, row));
        if query(&sql, params)?.execute(conn)? > 0 {
            return Ok(false);
        }
    } else if exists(conn, table, row)? {
        return Ok(false);
    }
    let next = seq.map(|n| n + 1);
    insert(conn, table, row, next)?;
    if next.is_some() {
        *seq = next;
    }
    Ok(true)
}

fn max_seq(conn: &mut AnyConnection, table: &Table) -> Result<i64, DbError> {
    let sql = format!(
        "SELECT MAX({}) AS n FROM {}",
        quote_ident("seq"),
        quote_ident(table.name)
    );
    Ok(query(&sql, Vec::new())?
        .load::<IntRow>(conn)?
        .into_iter()
        .next()
        .and_then(|r| r.n)
        .unwrap_or(0))
}

/// The whole state in §9.2's order, inside the caller's one write transaction. Nothing is deleted, and
/// `deleted_at`, the timeout columns and the identity keys are never written.
pub(crate) fn write_state(conn: &mut AnyConnection, state: &TargetState) -> Result<(), DbError> {
    for area in WRITE_AREAS {
        for table in TABLES.iter().filter(|t| t.area == area) {
            let mut seq = if table.seq { Some(max_seq(conn, table)?) } else { None };
            for (_, row) in state.rows(table.name) {
                let implicit = table.name == "accounts"
                    && row
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| state.implicit.contains(id));
                if implicit {
                    if !exists(conn, table, row)? {
                        insert(conn, table, row, None)?;
                    }
                    continue;
                }
                upsert(conn, table, row, &mut seq)?;
                match table.name {
                    "wallets" => write_children(
                        conn,
                        &WALLET_BALANCES,
                        row,
                        "balances",
                        &["catalog_id", "balance", "ordinal"],
                    )?,
                    "parties" => write_children(
                        conn,
                        &PARTY_MEMBERS,
                        row,
                        "members",
                        &["account", "position", "joined_at"],
                    )?,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// `balances` and `members` are arrays of `[a, b, c]`; the parent's key is the child's first column
/// (wallets) or second (party members).
fn write_children(
    conn: &mut AnyConnection,
    child: &Table,
    parent: &Doc,
    field: &str,
    names: &[&str; 3],
) -> Result<(), DbError> {
    let Some(Value::Array(entries)) = parent.get(field) else {
        return Ok(());
    };
    let (parent_column, parent_field) = if child.name == "wallet_balances" {
        ("account_id", "account_id")
    } else {
        ("party", "id")
    };
    for entry in entries {
        let Some(parts) = entry.as_array() else { continue };
        let mut row = Doc::new();
        row.insert(
            parent_column.into(),
            parent.get(parent_field).cloned().unwrap_or(Value::Null),
        );
        for (name, value) in names.iter().zip(parts) {
            row.insert((*name).into(), value.clone());
        }
        upsert(conn, child, &row, &mut None)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ reading back

macro_rules! cells {
    ($($t:ident)*; $($i:ident)*; $($f:ident)*; $($b:ident)*) => {
        /// One row in typed slots, whatever the table: unused slots are selected as typed NULLs.
        #[derive(QueryableByName)]
        struct Cells {
            $(#[diesel(sql_type = Nullable<Text>)] $t: Option<String>,)*
            $(#[diesel(sql_type = Nullable<BigInt>)] $i: Option<i64>,)*
            $(#[diesel(sql_type = Nullable<Double>)] $f: Option<f64>,)*
            $(#[diesel(sql_type = Nullable<Bool>)] $b: Option<bool>,)*
        }

        const SLOTS: [(&[&str], &str); 4] = [
            (&[$(stringify!($t)),*], "TEXT"),
            (&[$(stringify!($i)),*], "BIGINT"),
            (&[$(stringify!($f)),*], "DOUBLE PRECISION"),
            (&[$(stringify!($b)),*], "BOOLEAN"),
        ];

        impl Cells {
            fn slots(self) -> Slots {
                Slots {
                    t: vec![$(self.$t),*],
                    i: vec![$(self.$i),*],
                    f: vec![$(self.$f),*],
                    b: vec![$(self.$b),*],
                }
            }
        }
    };
}

cells!(t0 t1 t2 t3 t4 t5 t6 t7 t8 t9 t10 t11 t12 t13 t14 t15 t16 t17 t18 t19 t20 t21 t22 t23;
    i0 i1 i2 i3 i4 i5; f0 f1; b0 b1);

struct Slots {
    t: Vec<Option<String>>,
    i: Vec<Option<i64>>,
    f: Vec<Option<f64>>,
    b: Vec<Option<bool>>,
}

fn kind(ty: Ty) -> usize {
    match ty {
        Ty::T => 0,
        Ty::I => 1,
        Ty::F => 2,
        Ty::B => 3,
    }
}

/// Every row of `table` (live rows only for a soft-delete table) as canonical records, in `order`.
fn select(conn: &mut AnyConnection, table: &Table, order: &str) -> Result<Vec<Doc>, DbError> {
    let mut used = [0usize; 4];
    let mut placed = Vec::with_capacity(table.cols.len());
    let mut select = Vec::new();
    for col in table.cols {
        let k = kind(col.ty);
        let slot = used[k];
        let name = SLOTS[k]
            .0
            .get(slot)
            .ok_or_else(|| DbError::Config(format!("{}: too many columns", table.name)))?;
        select.push(format!("{} AS {}", quote_ident(col.column), quote_ident(name)));
        placed.push((col, k, slot));
        used[k] += 1;
    }
    for (k, (names, sql_type)) in SLOTS.iter().enumerate() {
        for name in &names[used[k]..] {
            select.push(format!("CAST(NULL AS {sql_type}) AS {}", quote_ident(name)));
        }
    }
    let live = if table.soft {
        " WHERE \"deleted_at\" IS NULL"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {} FROM {}{live}{order}",
        select.join(", "),
        quote_ident(table.name)
    );
    let rows = query(&sql, Vec::new())?.load::<Cells>(conn)?;
    Ok(rows
        .into_iter()
        .map(|cells| {
            let mut slots = cells.slots();
            let mut doc = Doc::new();
            for (col, k, slot) in &placed {
                let value = match k {
                    0 => slots.t[*slot].take().map_or(Value::Null, Value::String),
                    1 => slots.i[*slot].map_or(Value::Null, Value::from),
                    2 => slots.f[*slot].map_or(Value::Null, float),
                    _ => slots.b[*slot].map_or(Value::Null, Value::Bool),
                };
                doc.insert(col.field.into(), value);
            }
            doc
        })
        .collect())
}

/// The target read back as §10.2 does, every table (callers pick the areas).
pub(crate) fn read_state(conn: &mut AnyConnection) -> Result<TargetState, DbError> {
    let mut state = TargetState::default();
    for table in TABLES {
        let single = matches!(table.name, "launcher_notices" | "operations_policy");
        let order = if single { " ORDER BY \"id\"" } else { "" };
        let mut rows = select(conn, table, order)?;
        if single {
            rows.truncate(1);
        }
        let rows_map = state.tables.entry(table.name).or_default();
        for row in rows {
            rows_map.insert(key_of(table, &row), row);
        }
    }
    attach(
        conn,
        &mut state,
        &WALLET_BALANCES,
        "wallets",
        "balances",
        " ORDER BY \"account_id\", \"ordinal\"",
    )?;
    attach(
        conn,
        &mut state,
        &PARTY_MEMBERS,
        "parties",
        "members",
        " ORDER BY \"party\", \"position\"",
    )?;
    Ok(state)
}

/// Children in `order` as `[b, c, d]` arrays on their parent; a child of a parent not read is dropped.
fn attach(
    conn: &mut AnyConnection,
    state: &mut TargetState,
    child: &Table,
    parent: &str,
    field: &str,
    order: &str,
) -> Result<(), DbError> {
    let rows = select(conn, child, order)?;
    let parents = state.tables.entry(table_name(parent)).or_default();
    for doc in parents.values_mut() {
        doc.insert(field.into(), Value::Array(Vec::new()));
    }
    let (link, rest): (&str, [&str; 3]) = if child.name == "wallet_balances" {
        ("account_id", ["catalog_id", "balance", "ordinal"])
    } else {
        ("party", ["account", "position", "joined_at"])
    };
    for row in rows {
        let Some(owner) = row.get(link).and_then(Value::as_str) else {
            continue;
        };
        if let Some(Value::Array(list)) = parents.get_mut(&vec![owner.to_owned()]).and_then(|d| d.get_mut(field)) {
            list.push(Value::Array(
                rest.iter()
                    .map(|k| row.get(*k).cloned().unwrap_or(Value::Null))
                    .collect(),
            ));
        }
    }
    Ok(())
}

fn table_name(name: &str) -> &'static str {
    table(name).map_or("", |t| t.name)
}

/// Accounts by stored user id, and parties by client id.
pub(crate) type ExistingIds = (Vec<(String, crate::ids::Existing)>, Vec<(String, String)>);

/// The target's accounts (deleted ones included) and parties, read before extraction (§6.2).
pub(crate) fn existing_ids(conn: &mut AnyConnection) -> Result<ExistingIds, DbError> {
    #[derive(QueryableByName)]
    struct AccountRow {
        #[diesel(sql_type = Text)]
        id: String,
        #[diesel(sql_type = Text)]
        user_id: String,
        #[diesel(sql_type = Text)]
        name: String,
        #[diesel(sql_type = BigInt)]
        notes: i64,
    }
    #[derive(QueryableByName)]
    struct PartyRow {
        #[diesel(sql_type = Text)]
        id: String,
        #[diesel(sql_type = Text)]
        client_id: String,
    }
    let accounts = query(
        "SELECT \"id\", \"user_id\", \"name\", \"notes\" FROM \"accounts\"",
        Vec::new(),
    )?
    .load::<AccountRow>(conn)?
    .into_iter()
    .map(|r| {
        let existing = crate::ids::Existing {
            id: r.id,
            name: r.name,
            notes: r.notes,
        };
        (r.user_id, existing)
    })
    .collect();
    let parties = query("SELECT \"id\", \"client_id\" FROM \"parties\"", Vec::new())?
        .load::<PartyRow>(conn)?
        .into_iter()
        .map(|r| (r.client_id, r.id))
        .collect();
    Ok((accounts, parties))
}
