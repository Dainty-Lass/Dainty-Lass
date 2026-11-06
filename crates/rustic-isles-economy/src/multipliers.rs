// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The test multipliers a private server (friends over Tailscale) may set (inventory spec §10, progression §4.7):
//! the shared reader, the reward half `POST /inventory` applies and the XP half progression's grant applies.

use std::net::Ipv4Addr;

use rustic_isles::Settings;
use serde_json::Value;
use service_kit::js;

use crate::util::{is_whole, js_number_text, num, number_value};

pub const MAX_TEST_MULTIPLIER: f64 = 100.0;

/// Both factors, and why they are off when they are (1 and 1 then).
#[derive(Debug, Clone, PartialEq)]
pub struct TestMultipliers {
    pub xp: f64,
    pub rewards: f64,
    pub reason: Option<String>,
}

impl TestMultipliers {
    pub const OFF: Self = Self {
        xp: 1.0,
        rewards: 1.0,
        reason: None,
    };

    /// Whether either factor applies.
    pub fn is_on(&self) -> bool {
        self.reason.is_none() && (self.xp != 1.0 || self.rewards != 1.0)
    }

    /// The module's start-up line (core §12 leaves the logging to the composer).
    pub fn startup_line(&self) -> String {
        if let Some(why) = &self.reason {
            return format!("[TestMultiplier] off ({why})");
        }
        if !self.is_on() {
            return String::from("[TestMultiplier] off (not set)");
        }
        let (xp, rewards) = (
            service_kit::jsjson::number(self.xp),
            service_kit::jsjson::number(self.rewards),
        );
        format!("[TestMultiplier] ON: XP x{xp}, rewards x{rewards} (private Tailscale server)")
    }
}

/// A setting's factor: blank or unset is 1, anything else JavaScript's `Number` of it, finite and within 1..=100.
fn parse_factor(raw: Option<&str>) -> Option<f64> {
    match raw.map(js::trim) {
        None | Some("") => Some(1.0),
        Some(text) => {
            let value = js_number_text(text);
            (value.is_finite() && (1.0..=MAX_TEST_MULTIPLIER).contains(&value)).then_some(value)
        }
    }
}

/// `ReadTestMultipliers` (§10.1): the first failing condition names the reason; with none, both factors apply.
pub fn read_test_multipliers(settings: &Settings) -> TestMultipliers {
    let xp = parse_factor(settings.raw("REFORGED_XP_MULTIPLIER"));
    let rewards = parse_factor(settings.raw("REFORGED_REWARD_MULTIPLIER"));
    let reason = match (xp, rewards) {
        (None, _) | (_, None) => Some("a multiplier is not a number from 1 to 100"),
        (Some(1.0), Some(1.0)) => Some("not set"),
        _ if settings.raw("REFORGED_SERVER_MODE") != Some("private") => Some("this is not a private server"),
        _ if !is_tailscale_host(settings.raw("REFORGED_SERVER_HOST").unwrap_or_default()) => {
            Some("the server host is not a Tailscale address")
        }
        _ => None,
    };
    match reason {
        Some(why) => TestMultipliers {
            reason: Some(why.to_owned()),
            ..TestMultipliers::OFF
        },
        None => TestMultipliers {
            xp: xp.unwrap_or(1.0),
            rewards: rewards.unwrap_or(1.0),
            reason: None,
        },
    }
}

/// An IPv4 address in 100.64.0.0/10, or a `*.ts.net` name of `a-z`, `0-9`, `-` labels.
pub fn is_tailscale_host(text: &str) -> bool {
    let host = js::trim(text).to_lowercase();
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        let [a, b, ..] = ip.octets();
        return a == 100 && (64..=127).contains(&b);
    }
    let Some(rest) = host.strip_suffix(".ts.net") else {
        return false;
    };
    !rest.is_empty()
        && rest.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// A body `source` whose loot the multiplier touches.
pub fn is_reward_source(source: Option<&Value>) -> bool {
    match source.and_then(Value::as_str) {
        Some(text) => {
            matches!(text, "PlayerLoot" | "MISC" | "patrol_chest" | "quest") || text.starts_with("BUNDLE_GATHER_")
        }
        None => false,
    }
}

const MULTIPLIED_PREFIXES: [&str; 4] = ["CURRENCY_NOTES", "CURRENCY_PJM_", "BREAK_", "GATHER_"];

/// A stacked add the multiplier changes: a loot catalog id with a positive whole quantity, not excluded.
pub fn is_multiplied_reward_item(entry: &Value, excluded: &dyn Fn(&str, f64) -> bool) -> bool {
    let Some(catalog_id) = entry.get("catalogId").and_then(Value::as_str) else {
        return false;
    };
    let q = num(entry.get("quantity"));
    MULTIPLIED_PREFIXES.iter().any(|p| catalog_id.starts_with(p)) && is_whole(q) && q > 0.0 && !excluded(catalog_id, q)
}

/// `MultiplyRewardItems` (§10.2): the multiplied list, or none when nothing changed.
pub fn multiply_reward_items(
    add_stacked: &[Value],
    source: Option<&Value>,
    rewards: f64,
    excluded: &dyn Fn(&str, f64) -> bool,
) -> Option<Vec<Value>> {
    if rewards == 1.0 || !is_reward_source(source) {
        return None;
    }
    let mut changed = false;
    let out = add_stacked
        .iter()
        .map(|entry| {
            if !is_multiplied_reward_item(entry, excluded) {
                return entry.clone();
            }
            let q = num(entry.get("quantity"));
            let mut next = entry.clone();
            if let Some(map) = next.as_object_mut() {
                let multiplied = (q * rewards).floor();
                if multiplied != q {
                    changed = true;
                }
                map.insert("quantity".into(), number_value(multiplied));
            }
            next
        })
        .collect();
    changed.then_some(out)
}

/// The XP hook (progression §4.7): factor 1 keeps `amount`, else `min(max, max(amount, floor(amount * xp)))`.
pub fn multiply_xp_grant(amount: f64, max: f64, multipliers: &TestMultipliers) -> f64 {
    if multipliers.xp == 1.0 {
        return amount;
    }
    max.min(amount.max((amount * multipliers.xp).floor()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Settings {
        Settings::from_pairs(pairs.iter().copied())
    }

    #[test]
    fn readings() {
        let base = [
            ("REFORGED_SERVER_MODE", "private"),
            ("REFORGED_SERVER_HOST", "100.101.102.103"),
            ("REFORGED_XP_MULTIPLIER", "5"),
            ("REFORGED_REWARD_MULTIPLIER", "3"),
        ];
        let on = read_test_multipliers(&settings(&base));
        assert_eq!((on.xp, on.rewards, on.reason), (5.0, 3.0, None));
        let mut named = base;
        named[1].1 = "my-pc.tail1234.ts.net";
        assert!(read_test_multipliers(&settings(&named)).is_on());
        let mut hex = base;
        hex[2].1 = "0x5";
        assert_eq!(read_test_multipliers(&settings(&hex)).xp, 5.0);
        for (key, value, reason) in [
            ("REFORGED_SERVER_MODE", "public", "this is not a private server"),
            (
                "REFORGED_SERVER_HOST",
                "203.0.113.7",
                "the server host is not a Tailscale address",
            ),
            (
                "REFORGED_SERVER_HOST",
                "192.168.1.10",
                "the server host is not a Tailscale address",
            ),
            (
                "REFORGED_SERVER_HOST",
                "hunts.example.net",
                "the server host is not a Tailscale address",
            ),
            (
                "REFORGED_XP_MULTIPLIER",
                "1000",
                "a multiplier is not a number from 1 to 100",
            ),
            (
                "REFORGED_REWARD_MULTIPLIER",
                "0.5",
                "a multiplier is not a number from 1 to 100",
            ),
            (
                "REFORGED_XP_MULTIPLIER",
                "abc",
                "a multiplier is not a number from 1 to 100",
            ),
        ] {
            let mut pairs = base.to_vec();
            pairs.retain(|(k, _)| *k != key);
            pairs.push((key, value));
            let off = read_test_multipliers(&settings(&pairs));
            assert_eq!(off.reason.as_deref(), Some(reason), "{key}={value}");
            assert_eq!((off.xp, off.rewards), (1.0, 1.0));
        }
        let unset = read_test_multipliers(&settings(&[("REFORGED_SERVER_MODE", "private")]));
        assert_eq!(unset.reason.as_deref(), Some("not set"));
        assert_eq!(unset.startup_line(), "[TestMultiplier] off (not set)");
        assert_eq!(
            on_line(),
            "[TestMultiplier] ON: XP x5, rewards x3 (private Tailscale server)"
        );
    }

    fn on_line() -> String {
        TestMultipliers {
            xp: 5.0,
            rewards: 3.0,
            reason: None,
        }
        .startup_line()
    }

    #[test]
    fn tailscale_hosts() {
        for host in ["100.64.0.1", "100.127.255.255", "a.b.ts.net", " My-PC.ts.net "] {
            assert!(is_tailscale_host(host), "{host}");
        }
        for host in [
            "100.63.255.255",
            "100.128.0.1",
            ".ts.net",
            "a..ts.net",
            "a_b.ts.net",
            "ts.net",
            "",
        ] {
            assert!(!is_tailscale_host(host), "{host}");
        }
    }

    #[test]
    fn rewards() {
        let none = |_: &str, _: f64| false;
        let items = vec![
            json!({"catalogId": "CURRENCY_NOTES", "quantity": 10, "x": 1}),
            json!({"catalogId": "COSMETIC", "quantity": 1}),
            json!({"catalogId": "GATHER_ORE", "quantity": 1.5}),
            json!({"catalogId": "BREAK_PART", "quantity": 0}),
            json!({"catalogId": "CURRENCY_PJM_PRESTIGE_FILLED", "quantity": 2}),
        ];
        let excluded = |id: &str, _: f64| id == "CURRENCY_PJM_PRESTIGE_FILLED";
        let out = multiply_reward_items(&items, Some(&json!("quest")), 3.0, &excluded).unwrap();
        assert_eq!(out[0], json!({"catalogId": "CURRENCY_NOTES", "quantity": 30, "x": 1}));
        assert_eq!(&out[1..], &items[1..]);
        assert_eq!(multiply_reward_items(&items, Some(&json!("Craft")), 3.0, &none), None);
        assert_eq!(
            multiply_reward_items(&items, Some(&json!("BUNDLE_GATHER_X")), 1.0, &none),
            None
        );
        assert!(multiply_reward_items(&items, Some(&json!("BUNDLE_GATHER_X")), 2.0, &none).is_some());
    }

    #[test]
    fn xp() {
        let m = |xp| TestMultipliers {
            xp,
            rewards: 1.0,
            reason: None,
        };
        for (a, xp, want) in [
            (125.0, 5.0, 625.0),
            (50_000.0, 5.0, 100_000.0),
            (125.0, 1.0, 125.0),
            (3.0, 1.5, 4.0),
        ] {
            assert_eq!(multiply_xp_grant(a, 100_000.0, &m(xp)), want);
        }
    }
}
