// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Season challenge rewards (bounties spec §4.5, inventory's W5 to W7): a completed challenge is claimed by a bounty
//! save, then its coins are posted to `POST /inventory`; each claim pays once.

use std::time::Duration;

use rustic_isles::SeasonalCoin;
use rustic_isles_store::repo::bounties::{BountyGroup, BountyStateRecord, RewardCredit};
use rustic_isles_store::{AccountId, IslesStore, RepoError, RepoResult, Repositories, Tx};

use crate::bounties::state::{bounty_id, effective_group, is_claimed};
use crate::util::{iso, now_iso};

const DAY_MS: i64 = 86_400_000;
const GAME_DAY_OFFSET_MS: i64 = 17 * 3_600_000;

/// The version's coin, a finite quantity above 0 and at most its ceiling.
pub fn is_challenge_reward_grant(coin: Option<SeasonalCoin>, catalog_id: &str, quantity: f64) -> bool {
    let Some(coin) = coin else { return false };
    // The ceiling is small (1000 for 1.14.7)
    #[allow(clippy::cast_precision_loss)]
    let ceiling = coin.max_per_claim as f64;
    catalog_id == coin.catalog_id && quantity.is_finite() && quantity > 0.0 && quantity <= ceiling
}

/// The most recent 17:00 UTC at or before `ms`, as ISO text.
pub fn game_day_key(ms: i64) -> String {
    let start = (ms - GAME_DAY_OFFSET_MS).div_euclid(DAY_MS) * DAY_MS + GAME_DAY_OFFSET_MS;
    iso(start)
}

pub fn is_daily_paid(record: &BountyStateRecord, now_ms: i64) -> bool {
    record.daily_reward_credited_on.as_deref() == Some(game_day_key(now_ms).as_str())
}

fn fundable(record: &BountyStateRecord, index: usize, daily_paid: bool) -> bool {
    let entry = &record.bounties[index];
    if !is_claimed(entry) || entry.reward_credit.is_some() {
        return false;
    }
    match effective_group(entry) {
        BountyGroup::Weekly => true,
        BountyGroup::Daily => !daily_paid,
        _ => false,
    }
}

/// The claimed, uncredited season (or unpaid daily) challenge with the newest `claimedAt`; the first wins a tie.
pub fn fundable_claim(record: &BountyStateRecord, now_ms: i64) -> Option<usize> {
    let daily_paid = is_daily_paid(record, now_ms);
    let mut best: Option<usize> = None;
    for i in 0..record.bounties.len() {
        if !fundable(record, i, daily_paid) {
            continue;
        }
        let stamp = record.bounties[i].claimed_at.as_deref().unwrap_or("");
        match best {
            Some(b) if record.bounties[b].claimed_at.as_deref().unwrap_or("") >= stamp => {}
            _ => best = Some(i),
        }
    }
    best
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundedClaim {
    pub bounty_id: String,
    pub claimed_at: Option<String>,
}

/// W5: records the credit on one claimed, uncredited challenge (in the caller's transaction); none when there is
/// none or the record changed under us.
pub fn fund_challenge_reward(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    coin: Option<SeasonalCoin>,
    account: &AccountId,
    catalog_id: &str,
    quantity: f64,
    transaction_id: Option<&str>,
) -> RepoResult<Option<FundedClaim>> {
    if !is_challenge_reward_grant(coin, catalog_id, quantity) {
        return Ok(None);
    }
    let Some(record) = repos.bounty_states.get(tx.conn(), account)? else {
        return Ok(None);
    };
    let now_ms = rustic_isles::replay::now_ms();
    let Some(chosen) = fundable_claim(&record, now_ms) else {
        return Ok(None);
    };
    let daily_paid = is_daily_paid(&record, now_ms);
    let id = bounty_id(&record.bounties[chosen]).unwrap_or_default().to_owned();
    let claimed_at = record.bounties[chosen].claimed_at.clone();
    let daily = effective_group(&record.bounties[chosen]) == BountyGroup::Daily;
    let at = now_iso();
    let mut next = record.clone();
    for i in 0..next.bounties.len() {
        if bounty_id(&next.bounties[i]) != Some(id.as_str()) {
            continue;
        }
        if !fundable(&record, i, daily_paid) {
            return Err(RepoError::Data(format!("bounty {id} is not a fundable claim")));
        }
        next.bounties[i].reward_credit = Some(RewardCredit {
            catalog_id: catalog_id.to_owned(),
            amount: quantity,
            transaction_id: transaction_id.unwrap_or("unknown").to_owned(),
            at: at.clone(),
        });
    }
    if daily {
        next.daily_reward_credited_on = Some(game_day_key(now_ms));
    }
    next.update_version = record.update_version + 1;
    next.updated_at = at;
    match repos
        .bounty_states
        .save_if_version(tx, &next, Some(record.update_version))?
    {
        Some(_) => Ok(Some(FundedClaim {
            bounty_id: id,
            claimed_at,
        })),
        None => Ok(None),
    }
}

/// W6: a challenge-reward grant while today's daily is paid and another daily claim waits (inventory then skips
/// the coins and keeps the rest of the transaction).
pub fn is_daily_challenge_reward_limited(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    coin: Option<SeasonalCoin>,
    account: &AccountId,
    catalog_id: &str,
    quantity: f64,
) -> RepoResult<bool> {
    if !is_challenge_reward_grant(coin, catalog_id, quantity) {
        return Ok(false);
    }
    let Some(record) = repos.bounty_states.get(tx.conn(), account)? else {
        return Ok(false);
    };
    if !is_daily_paid(&record, rustic_isles::replay::now_ms()) {
        return Ok(false);
    }
    Ok(record.bounties.iter().any(|entry| {
        is_claimed(entry) && entry.reward_credit.is_none() && effective_group(entry) == BountyGroup::Daily
    }))
}

/// W7: the coin grant may arrive about 50 ms before the bounty save that claims it, so wait up to 1.5 s (a read
/// every 100 ms, the system clock) for a fundable claim. A read error reads as "not found".
pub async fn wait_for_fundable_season_claim(store: &IslesStore, account: &AccountId) -> bool {
    let deadline = crate::util::system_now_ms() + 1500;
    loop {
        let id = account.clone();
        let found = store
            .read(move |conn, repos| repos.bounty_states.get(conn, &id))
            .await
            .ok()
            .flatten()
            .is_some_and(|record| fundable_claim(&record, crate::util::system_now_ms()).is_some());
        if found {
            return true;
        }
        if crate::util::system_now_ms() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_and_days() {
        let coin = Some(SeasonalCoin {
            catalog_id: "CURRENCY_SEASONAL_COIN",
            max_per_claim: 1000,
        });
        assert!(is_challenge_reward_grant(coin, "CURRENCY_SEASONAL_COIN", 1000.0));
        for (id, q) in [
            ("CURRENCY_SEASONAL_COIN", 1001.0),
            ("CURRENCY_SEASONAL_COIN", 0.0),
            ("CURRENCY_SEASONAL_COIN", f64::NAN),
            ("CURRENCY_NOTES", 5.0),
        ] {
            assert!(!is_challenge_reward_grant(coin, id, q));
        }
        assert!(!is_challenge_reward_grant(None, "CURRENCY_SEASONAL_COIN", 5.0));
        let at = service_kit::js::date_parse("2026-10-10T16:59:59.999Z").unwrap();
        assert_eq!(game_day_key(at), "2026-10-09T17:00:00.000Z");
        assert_eq!(game_day_key(at + 1), "2026-10-10T17:00:00.000Z");
    }
}
