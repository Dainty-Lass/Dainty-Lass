//! `testMultipliers.ts`, the reward half: a private test server (friends over Tailscale) can multiply the loot its
//! game servers grant. Off unless the host kit turns it on, and checked again here: only `REFORGED_SERVER_MODE=private`
//! on a Tailscale address multiplies anything. The XP half is the progression area's.

use std::net::Ipv4Addr;

use serde_json::Value;
use service_kit::js;

use super::items::number_value;
use crate::settings::Settings;

pub const MAX_TEST_MULTIPLIER: f64 = 100.0;

const REWARD_SOURCES: [&str; 4] = ["PlayerLoot", "MISC", "patrol_chest", "quest"];
const REWARD_SOURCE_PREFIXES: [&str; 1] = ["BUNDLE_GATHER_"];
/// Currencies and materials; everything else keeps its quantity
const MULTIPLIED_ITEM_PREFIXES: [&str; 4] = ["CURRENCY_NOTES", "CURRENCY_PJM_", "BREAK_", "GATHER_"];

pub fn is_tailscale_host(host: &str) -> bool {
    let host = js::trim(host).to_lowercase();
    if let Ok(address) = host.parse::<Ipv4Addr>() {
        let [a, b, ..] = address.octets();
        return a == 100 && (64..=127).contains(&b);
    }
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 3
        && labels.iter().all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && labels[labels.len() - 2..] == ["ts", "net"]
}

fn parse_multiplier(raw: Option<&str>) -> Option<f64> {
    let Some(raw) = raw.filter(|raw| !js::trim(raw).is_empty()) else {
        return Some(1.0);
    };
    let value = js::number(Some(&Value::String(raw.to_owned())));
    (value.is_finite() && (1.0..=MAX_TEST_MULTIPLIER).contains(&value)).then_some(value)
}

/// `ReadTestMultipliers().rewards`.
pub fn reward_multiplier(settings: &Settings) -> f64 {
    let xp = parse_multiplier(settings.raw("REFORGED_XP_MULTIPLIER"));
    let rewards = parse_multiplier(settings.raw("REFORGED_REWARD_MULTIPLIER"));
    let (Some(xp), Some(rewards)) = (xp, rewards) else {
        return 1.0;
    };
    if xp == 1.0 && rewards == 1.0 {
        return 1.0;
    }
    if settings.raw("REFORGED_SERVER_MODE") != Some("private") {
        return 1.0;
    }
    if !is_tailscale_host(settings.raw("REFORGED_SERVER_HOST").unwrap_or_default()) {
        return 1.0;
    }
    rewards
}

fn is_reward_source(source: Option<&Value>) -> bool {
    matches!(source, Some(Value::String(source))
        if REWARD_SOURCES.contains(&source.as_str()) || REWARD_SOURCE_PREFIXES.iter().any(|p| source.starts_with(p)))
}

fn is_multiplied_reward_item(catalog_id: Option<&Value>) -> bool {
    matches!(catalog_id, Some(Value::String(id)) if MULTIPLIED_ITEM_PREFIXES.iter().any(|p| id.starts_with(p)))
}

/// `MultiplyRewardItems`: the stacked items to add with the reward multiplier applied, or `None` when nothing changes.
/// `exclude` says no for an item that must keep its quantity whatever its prefix (prestige-gated, challenge-funded).
pub fn multiply_reward_items(
    multiplier: f64,
    source: Option<&Value>,
    stacked_items_to_add: &[Value],
    exclude: &dyn Fn(&str, f64) -> bool,
) -> Option<Vec<Value>> {
    if multiplier == 1.0 || !is_reward_source(source) {
        return None;
    }
    let mut changed = false;
    let out = stacked_items_to_add
        .iter()
        .map(|item| {
            let quantity = js::number(item.get("quantity"));
            let skip = !is_multiplied_reward_item(item.get("catalogId"))
                || !(quantity.is_finite() && quantity.fract() == 0.0)
                || quantity <= 0.0
                || item
                    .get("catalogId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| exclude(id, quantity));
            if skip {
                return item.clone();
            }
            changed = true;
            let mut multiplied = item.clone();
            if let Value::Object(fields) = &mut multiplied {
                fields.insert("quantity".into(), number_value((quantity * multiplier).floor()));
            }
            multiplied
        })
        .collect();
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Settings {
        Settings::from_pairs(pairs.iter().copied())
    }

    fn private(rewards: &str) -> Settings {
        settings(&[
            ("REFORGED_REWARD_MULTIPLIER", rewards),
            ("REFORGED_SERVER_MODE", "private"),
            ("REFORGED_SERVER_HOST", "100.101.102.103"),
        ])
    }

    #[test]
    fn tailscale_hosts() {
        assert!(is_tailscale_host("100.64.0.1"));
        assert!(is_tailscale_host(" 100.127.255.255 "));
        assert!(!is_tailscale_host("100.128.0.1"));
        assert!(!is_tailscale_host("100.063.0.1"));
        assert!(!is_tailscale_host("192.168.0.1"));
        assert!(is_tailscale_host("my-box.tail1234.ts.net"));
        assert!(!is_tailscale_host("ts.net"));
        assert!(!is_tailscale_host("evil.ts.net.example.com"));
        assert!(!is_tailscale_host(""));
    }

    #[test]
    fn the_multiplier_acts_only_on_a_private_tailscale_server() {
        assert_eq!(reward_multiplier(&private("3")), 3.0);
        assert_eq!(reward_multiplier(&Settings::default()), 1.0);
        assert_eq!(reward_multiplier(&private("101")), 1.0);
        assert_eq!(reward_multiplier(&private("abc")), 1.0);
        let public = settings(&[("REFORGED_REWARD_MULTIPLIER", "3"), ("REFORGED_SERVER_MODE", "public")]);
        assert_eq!(reward_multiplier(&public), 1.0);
        let off_net = settings(&[
            ("REFORGED_REWARD_MULTIPLIER", "3"),
            ("REFORGED_SERVER_MODE", "private"),
            ("REFORGED_SERVER_HOST", "example.com"),
        ]);
        assert_eq!(reward_multiplier(&off_net), 1.0);
        // an invalid XP multiplier turns both off
        let broken = settings(&[
            ("REFORGED_XP_MULTIPLIER", "0"),
            ("REFORGED_REWARD_MULTIPLIER", "3"),
            ("REFORGED_SERVER_MODE", "private"),
            ("REFORGED_SERVER_HOST", "100.101.102.103"),
        ]);
        assert_eq!(reward_multiplier(&broken), 1.0);
    }

    #[test]
    fn only_reward_sources_and_materials_are_multiplied() {
        let items = vec![
            json!({"catalogId": "CURRENCY_NOTES", "quantity": 100}),
            json!({"catalogId": "TOKEN_X", "quantity": 1}),
            json!({"catalogId": "GATHER_ORE", "quantity": 3, "extra": 1}),
            json!({"catalogId": "CURRENCY_PJM_WEAPON", "quantity": 4}),
        ];
        let never = |_: &str, _: f64| false;
        let out = multiply_reward_items(2.0, Some(&json!("PlayerLoot")), &items, &never).unwrap();
        assert_eq!(out[0]["quantity"], 200);
        assert_eq!(out[1]["quantity"], 1);
        assert_eq!(out[2], json!({"catalogId": "GATHER_ORE", "quantity": 6, "extra": 1}));
        assert!(multiply_reward_items(2.0, Some(&json!("BUNDLE_GATHER_7")), &items, &never).is_some());
        assert!(multiply_reward_items(2.0, Some(&json!("store")), &items, &never).is_none());
        assert!(multiply_reward_items(2.0, None, &items, &never).is_none());
        assert!(multiply_reward_items(1.0, Some(&json!("MISC")), &items, &never).is_none());
        let keep_pjm = |id: &str, _: f64| id.starts_with("CURRENCY_PJM_");
        let out = multiply_reward_items(2.0, Some(&json!("quest")), &items, &keep_pjm).unwrap();
        assert_eq!(out[3]["quantity"], 4);
        let out = multiply_reward_items(1.5, Some(&json!("quest")), &items[..1], &never).unwrap();
        assert_eq!(out[0]["quantity"], 150);
    }
}
