// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Platinum (bounties spec §4.8.2, §4.8.3): 13 stored buckets shown as one number, and the free-Platinum switch.

use rustic_isles::Settings;

/// W8
pub const PLATINUM_AGGREGATE_ID: &str = "CURRENCY_PLATINUM";
/// The name the Reforged unit's tester tool uses.
pub const AGGREGATE_ID: &str = PLATINUM_AGGREGATE_ID;

/// Most disposable first.
pub const SPEND_ORDER: [&str; 13] = [
    "CURRENCY_PLATINUM_CS",
    "CURRENCY_PLATINUM_UNIV",
    "CURRENCY_PLATINUM_PLATFORM_BONUS",
    "CURRENCY_PLATINUM_EPIC_BONUS",
    "CURRENCY_PLATINUM_XBOX_BONUS",
    "CURRENCY_PLATINUM_PSN_BONUS",
    "CURRENCY_PLATINUM_SWITCH_BONUS",
    "CURRENCY_PLATINUM",
    "CURRENCY_PLATINUM_PLATFORM",
    "CURRENCY_PLATINUM_EPIC",
    "CURRENCY_PLATINUM_XBOX",
    "CURRENCY_PLATINUM_PSN",
    "CURRENCY_PLATINUM_SWITCH",
];

/// Whether the id is one of the platinum buckets.
pub fn is_platinum_catalog_id(id: &str) -> bool {
    SPEND_ORDER.contains(&id)
}

/// The sum of the finite balances of the 13 buckets.
pub fn sum_platinum(balances: &[(String, f64)]) -> f64 {
    balances
        .iter()
        .filter(|(id, b)| is_platinum_catalog_id(id) && b.is_finite())
        .map(|(_, b)| *b)
        .sum()
}

/// Platinum is free unless `PLATINUM_STORE_FREE` is exactly `0`.
pub fn platinum_store_free(settings: &Settings) -> bool {
    settings.raw("PLATINUM_STORE_FREE") != Some("0")
}

/// 0 for a Platinum price while Platinum is free; else the price.
pub fn effective_price(settings: &Settings, currency_id: &str, price: f64) -> f64 {
    if platinum_store_free(settings) && currency_id.to_lowercase() == "id_currency_platinum" {
        0.0
    } else {
        price
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets() {
        assert!(is_platinum_catalog_id("CURRENCY_PLATINUM_UNIV"));
        assert!(!is_platinum_catalog_id("CURRENCY_PLATINUM_STEAM"));
        assert!(!is_platinum_catalog_id("ID_CURRENCY_PLATINUM"));
        let b = vec![
            ("CURRENCY_PLATINUM_UNIV".to_owned(), 25.0),
            ("CURRENCY_PLATINUM".to_owned(), 5.0),
            ("X".to_owned(), 9.0),
        ];
        assert_eq!(sum_platinum(&b), 30.0);
        let free = Settings::default();
        assert_eq!(effective_price(&free, "ID_CURRENCY_PLATINUM", 1000.0), 0.0);
        assert_eq!(effective_price(&free, "id_currency_marks_steel", 10.0), 10.0);
        let paid = Settings::from_pairs([("PLATINUM_STORE_FREE", "0")]);
        assert_eq!(effective_price(&paid, "id_currency_platinum", 1000.0), 1000.0);
    }
}
