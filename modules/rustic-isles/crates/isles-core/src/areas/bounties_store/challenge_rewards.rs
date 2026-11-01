//! Season-challenge reward funding (`controllers/challengeRewards.ts`): the coins a completed challenge pays are charged
//! to exactly one claimed, uncredited challenge, and the credit is recorded on it in the same transaction as the coins.
//! The inventory area calls these from `POST /inventory`'s currency deltas. The coin and its per-claim ceiling are the
//! version's `SEASONAL_COIN` table: none is no funding rule.

use super::bounty_state::{daily_challenge_reward_limited, find_fundable_season_claim, with_reward_credit};
use super::util::{now_iso, now_ms};
use crate::repo::bounties::RewardCredit;
use crate::repo::{AccountId, RepoError, RepoResult, Repositories, Tx};
use crate::tables::SeasonalCoin;

/// `IsChallengeRewardGrant`: the version's coin, a positive quantity up to its per-claim ceiling.
pub fn is_challenge_reward_grant(coin: Option<&SeasonalCoin>, catalog_id: &str, quantity: f64) -> bool {
    coin.is_some_and(|coin| {
        coin.catalog_id == catalog_id
            && quantity.is_finite()
            && quantity > 0.0
            && quantity <= f64::from(coin.max_per_claim)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundedClaim {
    pub bounty_id: String,
    pub claimed_at: Option<String>,
}

/// `FundChallengeReward`: charges a challenge-reward grant to one claimed, uncredited challenge and records the credit on
/// it, inside the caller's transaction. `None` when nothing can fund it (the caller then applies the prestige gate).
pub fn fund_challenge_reward(
    repos: &Repositories,
    tx: &mut Tx<'_>,
    coin: Option<&SeasonalCoin>,
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
    let Some(claim) = find_fundable_season_claim(Some(&record), now_ms()) else {
        return Ok(None);
    };
    let bounty_id = claim
        .bounty
        .get("bounty_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let claimed_at = claim.claimed_at.clone();
    let credit = RewardCredit {
        catalog_id: catalog_id.to_owned(),
        amount: quantity,
        transaction_id: transaction_id.unwrap_or("unknown").to_owned(),
        at: now_iso(),
    };
    let next = with_reward_credit(&record, &bounty_id, credit, &now_iso()).map_err(RepoError::Data)?;
    // A plain version mismatch means the claim is no longer ours to charge
    let saved = repos
        .bounty_states
        .save_if_version(tx, &next, Some(record.update_version))?;
    Ok(saved.map(|_| FundedClaim { bounty_id, claimed_at }))
}

/// `IsDailyChallengeRewardLimited`: a challenge-reward grant that finds no fundable claim only because this game day's
/// daily reward was already paid. The wallet then skips the coins and keeps the rest of the transaction.
pub fn is_daily_challenge_reward_limited(
    repos: &Repositories,
    tx: &mut Tx<'_>,
    coin: Option<&SeasonalCoin>,
    account: &AccountId,
    catalog_id: &str,
    quantity: f64,
) -> RepoResult<bool> {
    if !is_challenge_reward_grant(coin, catalog_id, quantity) {
        return Ok(false);
    }
    let record = repos.bounty_states.get(tx.conn(), account)?;
    Ok(daily_challenge_reward_limited(record.as_ref(), now_ms()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const COIN: SeasonalCoin = SeasonalCoin {
        catalog_id: "CURRENCY_SEASONAL_COIN",
        max_per_claim: 1000,
    };

    #[test]
    fn a_challenge_reward_is_the_version_coin_up_to_the_ceiling() {
        assert!(is_challenge_reward_grant(Some(&COIN), "CURRENCY_SEASONAL_COIN", 400.0));
        assert!(is_challenge_reward_grant(Some(&COIN), "CURRENCY_SEASONAL_COIN", 1000.0));
        assert!(!is_challenge_reward_grant(
            Some(&COIN),
            "CURRENCY_SEASONAL_COIN",
            1001.0
        ));
        assert!(!is_challenge_reward_grant(Some(&COIN), "CURRENCY_SEASONAL_COIN", 0.0));
        assert!(!is_challenge_reward_grant(
            Some(&COIN),
            "CURRENCY_SEASONAL_COIN",
            f64::NAN
        ));
        assert!(!is_challenge_reward_grant(Some(&COIN), "CURRENCY_NOTES", 10.0));
        // No SEASONAL_COIN, no funding rule
        assert!(!is_challenge_reward_grant(None, "CURRENCY_SEASONAL_COIN", 400.0));
    }
}
