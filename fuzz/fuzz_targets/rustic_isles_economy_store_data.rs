//! What Rustic Isles' store reads from the host's store files (spec-rustic-isles-economy-bounties §13): arbitrary JSON
//! as each store file and as the fountain file, then the catalog, the lookup of arbitrary text, the listing and the
//! purchase plan. Nothing panics; a plan's removes are never negative; an accepted fountain offer always passes the
//! fountain rules again; a listing always has its keys in order.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles::{GameDataRoot, Settings};
use rustic_isles_economy::bounties::store::{Catalog, SECTION_ORDER, listing, parse_fountain_bundle, plan_purchase};
use serde_json::Value;

const LISTING_KEYS: [&str; 14] = [
    "prices",
    "maxAllowed",
    "remaining",
    "duplicateInstancedItems",
    "images",
    "tags",
    "scheduledTags",
    "items",
    "entitlements",
    "skuProgression",
    "loadoutSlots",
    "availableFrom",
    "availableTo",
    "timeAvailabilityReason",
];

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(4, '\u{1}');
    let (file, fountain, lookup) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    // One folder per fuzzing process, rewritten on every input
    let dir = std::env::temp_dir().join(format!("isles-economy-fuzz-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for name in ["ladyluck_store.json", "reward_cache_store.json", "platinum_store.json"] {
        let _ = std::fs::write(dir.join(name), file);
    }
    let _ = std::fs::write(dir.join("fountain_daily_bundle.json"), fountain);
    let root = GameDataRoot::new(Some(dir.clone()), None);
    if let Ok(raw) = serde_json::from_str::<Value>(fountain) {
        let (offers, problems) = parse_fountain_bundle(&raw, &root);
        if problems.is_empty() {
            let again = Value::Array(offers.iter().map(|o| Value::Object(o.raw.clone())).collect());
            assert!(parse_fountain_bundle(&again, &root).1.is_empty());
        }
    }
    let catalog = Catalog::load(&root, &SECTION_ORDER);
    let _ = catalog.lookup(lookup);
    let settings = Settings::from_pairs([("PLATINUM_STORE_FREE", "0")]);
    for tag in SECTION_ORDER {
        for sku in catalog.skus(tag) {
            let shown = listing(&settings, sku);
            let keys: Vec<&str> = shown
                .as_object()
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            let tail: Vec<&str> = keys
                .iter()
                .copied()
                .skip_while(|k| *k != "prices")
                .take(LISTING_KEYS.len())
                .collect();
            assert_eq!(tail, LISTING_KEYS);
            let fresh = || String::from("FRESH");
            if let Ok(plan) = plan_purchase(&settings, sku, tag, "user", 0, &fresh) {
                for remove in &plan.remove_stacked {
                    assert!(remove.get("quantity").and_then(Value::as_f64).is_some_and(|q| q >= 0.0));
                }
            }
        }
    }
});
