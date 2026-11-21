// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `ApplyCurrencyDeltas` (spec §9.1), the prestige gate (§9.2) and the Sparkforge conversion, on the bounties
//! part's wallet primitives (W1 to W6), all inside the inventory transaction.

use dainty_service_kit::{js, jsjson};
use rustic_isles_store::{AccountId, Repositories, Tx};
use serde_json::Value;

use crate::bounties::{challenge_rewards, platinum, wallet};
use crate::inventory::ports::{CurrencyChange, WalletError};
use crate::inventory::prestige::ProgressionConfig;
use crate::util::{num, num0, trunc_i64};

/// Aetherspark: 200 of it removed buys one Aetherheart without banked prestige.
pub const AETHERSPARK: &str = "CURRENCY_PJM_PRESTIGE_EMPTY";
pub const AETHERHEART: &str = "CURRENCY_PJM_PRESTIGE_FILLED";
const SPARKS_PER_HEART: i64 = 200;

fn touch(touched: &mut Vec<(String, f64)>, id: &str, balance: f64) {
    match touched.iter_mut().find(|(seen, _)| seen == id) {
        Some((_, value)) => *value = balance,
        None => touched.push((id.to_owned(), balance)),
    }
}

fn currency_id(entry: &Value) -> Option<&str> {
    entry
        .get("catalogId")
        .and_then(Value::as_str)
        .filter(|id| id.starts_with("CURRENCY_"))
}

pub(crate) fn apply_currency_deltas(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    change: &CurrencyChange<'_>,
) -> Result<Vec<(String, f64)>, WalletError> {
    let mut touched = Vec::new();
    if change.user_id.is_empty() || change.user_id == "INVALID" {
        return Ok(touched);
    }
    let removes = change.remove_stacked.as_array().map_or(&[][..], Vec::as_slice);
    let sparks: i64 = removes
        .iter()
        .filter(|r| r.get("catalogId").and_then(Value::as_str) == Some(AETHERSPARK))
        .filter_map(|r| js::safe_integer(r.get("quantity")))
        .filter(|q| *q > 0)
        .sum();
    let mut hearts = sparks / SPARKS_PER_HEART;
    let coin = change.shared.version().tables.seasonal_coin;
    let account = change.account;
    for add in change.add_stacked {
        let Some(id) = currency_id(add) else { continue };
        let q = num(add.get("quantity"));
        let mut gate = true;
        if change.allow_challenge_reward_funding {
            let funded =
                challenge_rewards::fund_challenge_reward(tx, repos, coin, account, id, q, Some(change.transaction_id))?;
            if funded.is_some() {
                gate = false;
            } else if challenge_rewards::is_daily_challenge_reward_limited(tx, repos, coin, account, id, q)? {
                tracing::warn!(
                    "transactionId {}: {id} x{} skipped: today's daily challenge reward is already paid",
                    change.transaction_id,
                    jsjson::number(q)
                );
                continue;
            }
        }
        if gate && change.operator_test_funding {
            gate = false;
        }
        if gate {
            let mut needed = q;
            if id == AETHERHEART && hearts > 0 && js::safe_integer(add.get("quantity")).is_some_and(|n| n > 0) {
                let converted = trunc_i64(q).min(hearts);
                hearts -= converted;
                #[allow(clippy::cast_precision_loss, reason = "`converted` is at most q, a safe integer")]
                let converted = converted as f64;
                needed = q - converted;
            }
            spend_banked_prestige(tx, repos, account, change.user_id, id, needed, change.prestige)?;
        }
        let balance = wallet::add_currency(tx, repos, account, change.starter, id, q)?;
        touch(&mut touched, id, balance);
    }
    for removal in removes {
        let Some(id) = currency_id(removal) else { continue };
        let q = num0(removal.get("quantity"));
        if id == platinum::PLATINUM_AGGREGATE_ID {
            for (bucket, balance) in wallet::spend_platinum(tx, repos, account, change.starter, q)? {
                touch(&mut touched, &bucket, balance);
            }
        } else {
            let balance = wallet::add_currency(tx, repos, account, change.starter, id, -q)?;
            touch(&mut touched, id, balance);
        }
    }
    Ok(touched)
}

/// `SpendBankedPrestigeForReward` (§9.2): a reward a prestige track pays per level is credited only against levels
/// banked past the track's cap.
pub(crate) fn spend_banked_prestige(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    user_id: &str,
    catalog_id: &str,
    quantity: f64,
    config: &ProgressionConfig,
) -> Result<(), WalletError> {
    if !(quantity.is_finite() && quantity > 0.0) {
        return Ok(());
    }
    let sources = config.prestige_sources(catalog_id);
    if sources.is_empty() {
        return Ok(());
    }
    let mut needed = quantity;
    let mut total = 0.0;
    let refuse = |total: f64| {
        WalletError::InsufficientPrestige(format!(
            "Insufficient banked prestige: user {user_id} requested {} of {catalog_id} but only {} banked across \
             eligible tracks",
            jsjson::number(quantity),
            jsjson::number(total)
        ))
    };
    for source in sources {
        let track = repos
            .progression_tracks
            .get(tx.conn(), account, &source.progression_id)?;
        #[allow(
            clippy::cast_precision_loss,
            reason = "stored counters are i64; as floats they stay exact well past any real track"
        )]
        let (claimed_before, progress) = track.as_ref().map_or((0.0, 0.0), |t| {
            (t.prestige_claimed.unwrap_or(0) as f64, t.progress as f64)
        });
        let levels = ((progress - source.cap).max(0.0) / source.xp_per_level).floor();
        let banked = (levels - claimed_before).max(0.0);
        let available = banked * source.reward_per_level;
        total += available;
        if needed > 0.0 && available > 0.0 {
            let take = needed.min(available);
            let spend = take / source.reward_per_level;
            if spend.fract() != 0.0 {
                continue;
            }
            let min_progress = trunc_i64(source.cap + (claimed_before + spend) * source.xp_per_level);
            let claimed = repos.progression_tracks.claim_prestige_levels(
                tx,
                account,
                &source.progression_id,
                trunc_i64(claimed_before),
                trunc_i64(spend),
                min_progress,
            )?;
            if !claimed {
                return Err(refuse(total));
            }
            needed -= spend * source.reward_per_level;
        }
    }
    if needed > 0.0 {
        return Err(refuse(total));
    }
    Ok(())
}
