//! Platinum: stored as many ledgers, displayed as one number (`platinumWallet.ts`), and free unless the host says
//! otherwise (`platinumStoreFree.ts`).
//!
//! The 1.12 catalog has 13 `CURRENCY_PLATINUM*` rows, real-money accounting buckets (which storefront took the payment,
//! paid or promotional). The client reads exactly one key, `CURRENCY_PLATINUM`, so the server holds the split and
//! publishes the aggregate. Pure, so the aggregation and the spend rules test on their own.

use serde_json::{Map, Value};

use crate::settings::Settings;

pub const PLATINUM_AGGREGATE_ID: &str = "CURRENCY_PLATINUM";

/// Spend order, most disposable first: promotional platinum goes before anything a payment produced.
pub const PLATINUM_SPEND_ORDER: [&str; 13] = [
    "CURRENCY_PLATINUM_CS",
    "CURRENCY_PLATINUM_UNIV",
    "CURRENCY_PLATINUM_PLATFORM_BONUS",
    "CURRENCY_PLATINUM_EPIC_BONUS",
    "CURRENCY_PLATINUM_XBOX_BONUS",
    "CURRENCY_PLATINUM_PSN_BONUS",
    "CURRENCY_PLATINUM_SWITCH_BONUS",
    PLATINUM_AGGREGATE_ID,
    "CURRENCY_PLATINUM_PLATFORM",
    "CURRENCY_PLATINUM_EPIC",
    "CURRENCY_PLATINUM_XBOX",
    "CURRENCY_PLATINUM_PSN",
    "CURRENCY_PLATINUM_SWITCH",
];

/// A stray `CURRENCY_PLATINUM`-prefixed id that isn't a catalog row is not spendable platinum.
pub fn is_platinum_catalog_id(catalog_id: &str) -> bool {
    PLATINUM_SPEND_ORDER.contains(&catalog_id)
}

fn held(balances: &[(String, f64)], id: &str) -> Option<f64> {
    balances.iter().find(|(k, _)| k == id).map(|(_, v)| *v)
}

/// Total spendable platinum across every bucket: what the client is shown.
pub fn sum_platinum(balances: &[(String, f64)]) -> f64 {
    PLATINUM_SPEND_ORDER
        .iter()
        .filter_map(|id| held(balances, id))
        .filter(|amount| amount.is_finite())
        .sum()
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlatinumSpend {
    Ok(Vec<(String, f64)>),
    Short { available: f64, requested: f64 },
}

/// `ResolvePlatinumSpend`: a debit as per-bucket deltas, drawing down `PLATINUM_SPEND_ORDER` until covered. Never a
/// partial plan: when the total is short the whole spend is refused.
pub fn resolve_platinum_spend(balances: &[(String, f64)], amount: f64) -> PlatinumSpend {
    let requested = amount.trunc();
    if !requested.is_finite() || requested <= 0.0 {
        return PlatinumSpend::Ok(Vec::new());
    }
    let available = sum_platinum(balances);
    if available < requested {
        return PlatinumSpend::Short { available, requested };
    }
    let mut deltas = Vec::new();
    let mut outstanding = requested;
    for id in PLATINUM_SPEND_ORDER {
        if outstanding <= 0.0 {
            break;
        }
        let Some(have) = held(balances, id).filter(|h| h.is_finite() && *h > 0.0) else {
            continue;
        };
        let take = have.min(outstanding);
        deltas.push((id.to_owned(), -take));
        outstanding -= take;
    }
    PlatinumSpend::Ok(deltas)
}

/// The id a balance is also reported under: `id_currency_` plus the rest lower-cased.
pub fn id_form(catalog_id: &str) -> String {
    format!(
        "id_currency_{}",
        catalog_id
            .strip_prefix("CURRENCY_")
            .unwrap_or(catalog_id)
            .to_lowercase()
    )
}

fn number(value: f64) -> Value {
    super::util::num(value)
}

/// `ProjectBalances`: every stored currency under both naming forms, then the platinum aggregate written over both of
/// its forms. Applied identically by `GET /balance` and `POST /reconcile`.
pub fn project_balances(balances: &[(String, f64)], base: Map<String, Value>) -> Map<String, Value> {
    let mut out = base;
    for (catalog_id, amount) in balances {
        out.insert(catalog_id.clone(), number(*amount));
        out.insert(id_form(catalog_id), number(*amount));
    }
    // Only when the wallet holds platinum somewhere, so an account with none keeps what the caller's base said
    let total = sum_platinum(balances);
    if total != 0.0 || held(balances, PLATINUM_AGGREGATE_ID).is_some() {
        out.insert(PLATINUM_AGGREGATE_ID.into(), number(total));
        out.insert("id_currency_platinum".into(), number(total));
    }
    out
}

/// `PlatinumStoreFree`: Platinum prices are 0 unless `PLATINUM_STORE_FREE=0`.
pub fn platinum_store_free(settings: &Settings) -> bool {
    settings.raw("PLATINUM_STORE_FREE") != Some("0")
}

/// `EffectivePrice`.
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

    fn wallet(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    fn base(pairs: &[(&str, f64)]) -> Map<String, Value> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), number(*v))).collect()
    }

    fn get(map: &Map<String, Value>, key: &str) -> f64 {
        map.get(key).and_then(Value::as_f64).unwrap_or(f64::NAN)
    }

    #[test]
    fn hunt_pass_platinum_in_the_univ_bucket_is_displayed_as_the_aggregate() {
        let projected = project_balances(
            &wallet(&[("CURRENCY_PLATINUM_UNIV", 50.0)]),
            base(&[("CURRENCY_PLATINUM", 0.0), ("id_currency_platinum", 0.0)]),
        );
        assert_eq!(get(&projected, "CURRENCY_PLATINUM"), 50.0);
        assert_eq!(get(&projected, "id_currency_platinum"), 50.0);
        assert_eq!(get(&projected, "CURRENCY_PLATINUM_UNIV"), 50.0);
        assert_eq!(get(&projected, "id_currency_platinum_univ"), 50.0);
    }

    #[test]
    fn the_aggregate_sums_every_bucket() {
        let w = wallet(&[
            ("CURRENCY_PLATINUM", 5.0),
            ("CURRENCY_PLATINUM_UNIV", 50.0),
            ("CURRENCY_PLATINUM_PLATFORM", 1000.0),
            ("CURRENCY_PLATINUM_PLATFORM_BONUS", 150.0),
            ("CURRENCY_PLATINUM_EPIC", 200.0),
            ("CURRENCY_NOTES", 6200.0),
        ]);
        assert_eq!(sum_platinum(&w), 1405.0);
        let projected = project_balances(&w, Map::new());
        assert_eq!(get(&projected, "CURRENCY_PLATINUM"), 1405.0);
        assert_eq!(get(&projected, "CURRENCY_NOTES"), 6200.0);
    }

    #[test]
    fn no_platinum_keeps_the_captured_stub_value() {
        let projected = project_balances(
            &wallet(&[("CURRENCY_NOTES", 5000.0)]),
            base(&[("CURRENCY_PLATINUM", 0.0), ("id_currency_platinum", 0.0)]),
        );
        assert_eq!(get(&projected, "CURRENCY_PLATINUM"), 0.0);
        assert_eq!(get(&projected, "id_currency_platinum"), 0.0);
        assert!(!project_balances(&wallet(&[("CURRENCY_NOTES", 1.0)]), Map::new()).contains_key("CURRENCY_PLATINUM"));
    }

    #[test]
    fn a_spend_consumes_promotional_buckets_first() {
        let plan = resolve_platinum_spend(
            &wallet(&[
                ("CURRENCY_PLATINUM_PLATFORM", 1000.0),
                ("CURRENCY_PLATINUM_UNIV", 50.0),
                ("CURRENCY_PLATINUM_CS", 25.0),
            ]),
            60.0,
        );
        assert_eq!(
            plan,
            PlatinumSpend::Ok(vec![
                ("CURRENCY_PLATINUM_CS".into(), -25.0),
                ("CURRENCY_PLATINUM_UNIV".into(), -35.0)
            ])
        );
    }

    #[test]
    fn a_spend_spills_into_paid_buckets_only_when_free_ones_are_gone() {
        let plan = resolve_platinum_spend(
            &wallet(&[("CURRENCY_PLATINUM_UNIV", 50.0), ("CURRENCY_PLATINUM_PLATFORM", 1000.0)]),
            250.0,
        );
        assert_eq!(
            plan,
            PlatinumSpend::Ok(vec![
                ("CURRENCY_PLATINUM_UNIV".into(), -50.0),
                ("CURRENCY_PLATINUM_PLATFORM".into(), -200.0)
            ])
        );
    }

    #[test]
    fn an_exact_total_spend_leaves_zero() {
        let w = wallet(&[
            ("CURRENCY_PLATINUM_UNIV", 50.0),
            ("CURRENCY_PLATINUM_EPIC_BONUS", 100.0),
            ("CURRENCY_PLATINUM_EPIC", 100.0),
        ]);
        let PlatinumSpend::Ok(deltas) = resolve_platinum_spend(&w, 250.0) else {
            panic!("covered");
        };
        let applied: Vec<(String, f64)> = w
            .iter()
            .map(|(k, v)| {
                let delta: f64 = deltas.iter().filter(|(d, _)| d == k).map(|(_, x)| *x).sum();
                (k.clone(), v + delta)
            })
            .collect();
        assert_eq!(sum_platinum(&applied), 0.0);
    }

    #[test]
    fn an_overdraw_is_rejected_outright() {
        let plan = resolve_platinum_spend(
            &wallet(&[("CURRENCY_PLATINUM_UNIV", 50.0), ("CURRENCY_PLATINUM_CS", 25.0)]),
            100.0,
        );
        assert_eq!(
            plan,
            PlatinumSpend::Short {
                available: 75.0,
                requested: 100.0
            }
        );
    }

    #[test]
    fn the_aggregate_and_every_bucket_are_platinum_look_alikes_are_not() {
        assert!(is_platinum_catalog_id(PLATINUM_AGGREGATE_ID));
        assert!(is_platinum_catalog_id("CURRENCY_PLATINUM_UNIV"));
        assert!(is_platinum_catalog_id("CURRENCY_PLATINUM_SWITCH_BONUS"));
        assert!(!is_platinum_catalog_id("CURRENCY_PLATINUM_STEAM"));
        assert!(!is_platinum_catalog_id("ID_CURRENCY_PLATINUM"));
        assert!(!is_platinum_catalog_id("CURRENCY_NOTES"));
    }

    #[test]
    fn platinum_prices_are_free_by_default() {
        let settings = Settings::default();
        assert_eq!(effective_price(&settings, "id_currency_platinum", 1000.0), 0.0);
        assert_eq!(effective_price(&settings, "ID_CURRENCY_PLATINUM", 250.0), 0.0);
        assert_eq!(effective_price(&settings, "id_currency_marks_steel", 300.0), 300.0);
        assert_eq!(effective_price(&settings, "id_currency_seasonal_coin", 40.0), 40.0);
    }

    #[test]
    fn platinum_store_free_zero_brings_the_prices_back() {
        let settings = Settings::from_pairs([("PLATINUM_STORE_FREE", "0")]);
        assert_eq!(effective_price(&settings, "id_currency_platinum", 1000.0), 1000.0);
    }
}
