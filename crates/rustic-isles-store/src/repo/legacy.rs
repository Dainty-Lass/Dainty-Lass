// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Old saves copied in on demand (`legacy_imports`) and the one-time codes that claim one (`legacy_claims`), for the
//! legacy importer (`docs/port/spec-legacy-ramsgate.md`). One old account fills at most one account per source, and
//! an account takes at most one per source; a claim code is kept only as its SHA-256.

use crate::sql::{IntRow, TextRow, sql};
use crate::{AccountId, AnyConnection, RepoError, RepoResult, Tx};

/// The Ramsgate Reborn (Dauntless 1.12) MongoDB, the one source so far.
pub const RAMSGATE_REBORN: &str = "ramsgate-reborn";

/// How long a claim code stays redeemable.
pub const CLAIM_KEEP_MS: i64 = 7 * 86_400_000;

/// The old account whose save the account was given from `source`, if any.
pub fn imported_into(conn: &mut AnyConnection, source: &str, account: &AccountId) -> RepoResult<Option<String>> {
    Ok(
        sql("SELECT source_user AS t FROM legacy_imports WHERE source = ? AND account = ?")
            .t(source)
            .t(account.to_string())
            .first::<TextRow>(conn)?
            .map(|r| r.t),
    )
}

/// The account an old account's save already went to.
pub fn taken_by(conn: &mut AnyConnection, source: &str, source_user: &str) -> RepoResult<Option<AccountId>> {
    sql("SELECT account AS t FROM legacy_imports WHERE source = ? AND source_user = ?")
        .t(source)
        .t(source_user)
        .first::<TextRow>(conn)?
        .map(|r| crate::sql::account_id(&r.t, "legacy_imports.account"))
        .transpose()
}

/// One import, as it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRecord<'a> {
    /// The importer's source, `ramsgate-reborn`.
    pub source: &'a str,
    /// The old account's user id.
    pub source_user: &'a str,
    /// The account the save went into.
    pub account: &'a AccountId,
    /// `discord` or `claim`.
    pub matched_by: &'a str,
    /// Unix ms.
    pub imported_at: i64,
    /// The import's counts, as JSON text.
    pub report: &'a str,
}

/// Records an import in the transaction that writes it; refused when either side already has one, so a race
/// between two first logins rolls the second back.
pub fn record_import(tx: &mut Tx<'_>, record: &ImportRecord<'_>) -> RepoResult<()> {
    let conn = tx.conn();
    if taken_by(conn, record.source, record.source_user)?.is_some() {
        return Err(RepoError::data(format!(
            "the {} save {} is already in another account",
            record.source, record.source_user
        )));
    }
    if imported_into(conn, record.source, record.account)?.is_some() {
        return Err(RepoError::data(format!(
            "account {} already has a {} save",
            record.account, record.source
        )));
    }
    sql(
        "INSERT INTO legacy_imports (source, source_user, account, matched_by, imported_at, report) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .t(record.source)
    .t(record.source_user)
    .t(record.account.to_string())
    .t(record.matched_by)
    .i(record.imported_at)
    .t(record.report)
    .run(conn)?;
    Ok(())
}

/// Stores a new claim code's hash for an old account, redeemable for [`CLAIM_KEEP_MS`].
pub fn create_claim(
    tx: &mut Tx<'_>,
    code_hash: &str,
    source: &str,
    source_user: &str,
    created_by: &str,
    now_ms: i64,
) -> RepoResult<()> {
    sql(
        "INSERT INTO legacy_claims (code_hash, source, source_user, created_by, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .t(code_hash)
    .t(source)
    .t(source_user)
    .t(created_by)
    .i(now_ms)
    .i(now_ms.saturating_add(CLAIM_KEEP_MS))
    .run(tx.conn())?;
    Ok(())
}

/// What redeeming a claim code did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redeem {
    /// The code is now this account's; the old account it names.
    Done(String),
    /// No such code, or it ran out: the same answer, so a guess learns nothing.
    Unknown,
    /// Already redeemed, by this account or another.
    Used,
    /// The account already claimed a save from this source, or was already given one.
    AccountHasOne,
    /// That old account's save already went to an account.
    SaveTaken,
}

#[derive(diesel::QueryableByName)]
struct ClaimRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    source: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source_user: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    expires_at: i64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    redeemed_by: Option<String>,
}

/// Binds the code to `account`, for the importer to find at the account's first character read.
pub fn redeem_claim(tx: &mut Tx<'_>, code_hash: &str, account: &AccountId, now_ms: i64) -> RepoResult<Redeem> {
    let conn = tx.conn();
    let Some(row) = sql("SELECT source, source_user, expires_at, redeemed_by FROM legacy_claims WHERE code_hash = ?")
        .t(code_hash)
        .first::<ClaimRow>(conn)?
    else {
        return Ok(Redeem::Unknown);
    };
    if row.redeemed_by.is_some() {
        return Ok(Redeem::Used);
    }
    if row.expires_at <= now_ms {
        return Ok(Redeem::Unknown);
    }
    if redeemed_by(conn, &row.source, account)?.is_some() || imported_into(conn, &row.source, account)?.is_some() {
        return Ok(Redeem::AccountHasOne);
    }
    if taken_by(conn, &row.source, &row.source_user)?.is_some() {
        return Ok(Redeem::SaveTaken);
    }
    sql("UPDATE legacy_claims SET redeemed_by = ?, redeemed_at = ? WHERE code_hash = ? AND redeemed_by IS NULL")
        .t(account.to_string())
        .i(now_ms)
        .t(code_hash)
        .run(conn)?;
    Ok(Redeem::Done(row.source_user))
}

/// The old account an account's redeemed claim names, for `source`.
pub fn redeemed_by(conn: &mut AnyConnection, source: &str, account: &AccountId) -> RepoResult<Option<String>> {
    Ok(
        sql("SELECT source_user AS t FROM legacy_claims WHERE source = ? AND redeemed_by = ?")
            .t(source)
            .t(account.to_string())
            .first::<TextRow>(conn)?
            .map(|r| r.t),
    )
}

/// Deletes claim codes past their time that nobody redeemed; how many went.
pub fn prune_claims(tx: &mut Tx<'_>, now_ms: i64) -> RepoResult<u64> {
    let n = sql("SELECT COUNT(*) AS n FROM legacy_claims WHERE redeemed_by IS NULL AND expires_at <= ?")
        .i(now_ms)
        .first::<IntRow>(tx.conn())?
        .and_then(|r| r.n)
        .unwrap_or(0);
    sql("DELETE FROM legacy_claims WHERE redeemed_by IS NULL AND expires_at <= ?")
        .i(now_ms)
        .run(tx.conn())?;
    Ok(u64::try_from(n).unwrap_or(0))
}
