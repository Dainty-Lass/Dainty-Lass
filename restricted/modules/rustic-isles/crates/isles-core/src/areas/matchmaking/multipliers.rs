//! `testMultipliers.ts` ([Dauntless Reforged Revival]): XP and reward multipliers for test servers, the one port of
//! it. No route of its own: progression's grant routes call [`multiply_xp_grant`], `POST /inventory`'s reward
//! transaction calls [`multiply_reward_items`] (as `inventory.ts` does), and the module logs [`TestMultipliers::startup_line`].
//!
//! A private server (friends over Tailscale) can multiply the XP and the loot its game servers grant. They are off
//! unless the host kit turns them on, and they never act on a public server: the host kit only writes them for
//! `--mode private`, and this checks again that the backend was set up as a private server on a Tailscale address.
//!
//! - XP: every game-server grant on `POST /progression/<user>/<track>/<amount>`, capped at the route's maximum.
//! - Rewards: the stacked items a game server ADDS in a reward transaction (its `source` is a hunt, a part break, a
//!   gathering node, a patrol chest or a quest), and only currencies and materials. Unlock tokens, cosmetics,
//!   equipment, season coins and prestige-gated rewards are never touched, nor is anything a crafting or a purchase
//!   removes.

use std::net::Ipv4Addr;

use serde_json::Value;
use service_kit::js;

use super::util::js_number;
use crate::areas::inventory::items::number_value;
use crate::settings::Settings;

pub const MAX_TEST_MULTIPLIER: f64 = 100.0;

/// The game's own `source` strings for reward transactions (seen in `[GrantMon]` lines of a 1.14.7 server).
const REWARD_SOURCES: [&str; 4] = ["PlayerLoot", "MISC", "patrol_chest", "quest"];
const REWARD_SOURCE_PREFIXES: [&str; 1] = ["BUNDLE_GATHER_"];

/// Currencies and materials. Everything else (TOKEN_*, EM_*, BNC_*, DYE_*, TITLE_*...) keeps its quantity.
const MULTIPLIED_ITEM_PREFIXES: [&str; 4] = ["CURRENCY_NOTES", "CURRENCY_PJM_", "BREAK_", "GATHER_"];

#[derive(Debug, Clone, PartialEq)]
pub struct TestMultipliers {
    pub xp: f64,
    pub rewards: f64,
    /// Why they are off when they are, for the start-up log line
    pub reason: Option<String>,
}

impl TestMultipliers {
    pub const OFF: Self = Self {
        xp: 1.0,
        rewards: 1.0,
        reason: None,
    };

    fn off(reason: &str) -> Self {
        Self {
            reason: Some(reason.to_owned()),
            ..Self::OFF
        }
    }

    /// The start-up log line: `[TestMultiplier] ON: XP x5, rewards x3 (private Tailscale server)` or
    /// `[TestMultiplier] off (not set)`.
    pub fn startup_line(&self) -> String {
        if self.xp != 1.0 || self.rewards != 1.0 {
            format!(
                "[TestMultiplier] ON: XP x{}, rewards x{} (private Tailscale server)",
                self.xp, self.rewards
            )
        } else {
            format!("[TestMultiplier] off ({})", self.reason.as_deref().unwrap_or("x1"))
        }
    }
}

/// `IsTailscaleHost`: a Tailscale IPv4 (100.64.0.0/10) or a `*.ts.net` name.
pub fn is_tailscale_host(host: &str) -> bool {
    let host = js::trim(host).to_lowercase();
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        let [a, b, ..] = ip.octets();
        return a == 100 && (64..=127).contains(&b);
    }
    host.strip_suffix(".ts.net").is_some_and(|name| {
        !name.is_empty()
            && name.split('.').all(|label| {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
    })
}

fn parse_multiplier(raw: Option<&str>) -> Option<f64> {
    let Some(raw) = raw.filter(|r| !js::trim(r).is_empty()) else {
        return Some(1.0);
    };
    let value = js_number(raw);
    (value.is_finite() && (1.0..=MAX_TEST_MULTIPLIER).contains(&value)).then_some(value)
}

/// `ReadTestMultipliers`, from the env file's `REFORGED_*` settings.
pub fn read_test_multipliers(settings: &Settings) -> TestMultipliers {
    let xp = parse_multiplier(settings.raw("REFORGED_XP_MULTIPLIER"));
    let rewards = parse_multiplier(settings.raw("REFORGED_REWARD_MULTIPLIER"));
    let (Some(xp), Some(rewards)) = (xp, rewards) else {
        return TestMultipliers::off(&format!("a multiplier is not a number from 1 to {MAX_TEST_MULTIPLIER}"));
    };
    if xp == 1.0 && rewards == 1.0 {
        return TestMultipliers::off("not set");
    }
    if settings.raw("REFORGED_SERVER_MODE") != Some("private") {
        return TestMultipliers::off("this is not a private server");
    }
    if !is_tailscale_host(settings.raw("REFORGED_SERVER_HOST").unwrap_or_default()) {
        return TestMultipliers::off("the server host is not a Tailscale address");
    }
    TestMultipliers {
        xp,
        rewards,
        reason: None,
    }
}

/// `MultiplyXpGrant`: `min(max, max(amount, floor(amount * xp)))`.
pub fn multiply_xp_grant(amount: f64, max_amount: f64, multipliers: &TestMultipliers) -> f64 {
    if multipliers.xp == 1.0 {
        return amount;
    }
    max_amount.min(amount.max((amount * multipliers.xp).floor()))
}

/// `IsRewardSource`: `typeof source === "string"` and one of the game's reward sources.
pub fn is_reward_source(source: Option<&Value>) -> bool {
    matches!(source, Some(Value::String(source))
        if REWARD_SOURCES.contains(&source.as_str()) || REWARD_SOURCE_PREFIXES.iter().any(|p| source.starts_with(p)))
}

/// `IsMultipliedRewardItem`.
pub fn is_multiplied_reward_item(catalog_id: Option<&Value>) -> bool {
    matches!(catalog_id, Some(Value::String(id)) if MULTIPLIED_ITEM_PREFIXES.iter().any(|p| id.starts_with(p)))
}

/// `MultiplyRewardItems`: the stacked items to add with the reward multiplier applied, or `None` when nothing
/// changes. `exclude` says no for an item that must keep its quantity whatever its prefix (prestige-gated,
/// challenge-funded). Each changed item keeps its other fields and their order.
pub fn multiply_reward_items(
    source: Option<&Value>,
    stacked_items_to_add: &[Value],
    exclude: &dyn Fn(&str, f64) -> bool,
    multipliers: &TestMultipliers,
) -> Option<Vec<Value>> {
    if multipliers.rewards == 1.0 || !is_reward_source(source) {
        return None;
    }
    let mut changed = false;
    let out = stacked_items_to_add
        .iter()
        .map(|item| {
            // `Number(Item?.quantity)`, so a numeric string counts as Node counts it
            let quantity = js::number(item.get("quantity"));
            let catalog_id = item.get("catalogId");
            let keep = !is_multiplied_reward_item(catalog_id)
                || !(quantity.is_finite() && quantity.fract() == 0.0)
                || quantity <= 0.0
                || catalog_id
                    .and_then(Value::as_str)
                    .is_some_and(|id| exclude(id, quantity));
            if keep {
                return item.clone();
            }
            changed = true;
            let mut multiplied = item.clone();
            if let Value::Object(fields) = &mut multiplied {
                fields.insert(
                    "quantity".into(),
                    number_value((quantity * multipliers.rewards).floor()),
                );
            }
            multiplied
        })
        .collect();
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The JavaScript call: a string source, and an array or not (Node leaves a non-array as it is).
    fn multiply(
        source: Option<&str>,
        items: &Value,
        exclude: &dyn Fn(&str, f64) -> bool,
        multipliers: &TestMultipliers,
    ) -> Option<Value> {
        let source = source.map(|s| json!(s));
        multiply_reward_items(source.as_ref(), items.as_array()?, exclude, multipliers).map(Value::Array)
    }

    fn private() -> Settings {
        Settings::from_pairs([
            ("REFORGED_SERVER_MODE", "private"),
            ("REFORGED_SERVER_HOST", "100.101.102.103"),
            ("REFORGED_XP_MULTIPLIER", "5"),
            ("REFORGED_REWARD_MULTIPLIER", "3"),
        ])
    }

    fn with(base: &[(&str, &str)], over: &[(&str, Option<&str>)]) -> Settings {
        let mut pairs: Vec<(String, String)> = base.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        for (key, value) in over {
            pairs.retain(|(k, _)| k != key);
            if let Some(value) = value {
                pairs.push(((*key).into(), (*value).into()));
            }
        }
        Settings::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
    }

    const PRIVATE: [(&str, &str); 4] = [
        ("REFORGED_SERVER_MODE", "private"),
        ("REFORGED_SERVER_HOST", "100.101.102.103"),
        ("REFORGED_XP_MULTIPLIER", "5"),
        ("REFORGED_REWARD_MULTIPLIER", "3"),
    ];

    fn on(xp: f64, rewards: f64) -> TestMultipliers {
        TestMultipliers {
            xp,
            rewards,
            reason: None,
        }
    }

    #[test]
    fn the_multipliers_act_only_on_a_private_server_at_a_tailscale_address() {
        assert_eq!(read_test_multipliers(&private()), on(5.0, 3.0));
        assert_eq!(
            read_test_multipliers(&with(
                &PRIVATE,
                &[("REFORGED_SERVER_HOST", Some("my-pc.tail1234.ts.net"))]
            )),
            on(5.0, 3.0)
        );
        let refused = [
            with(&PRIVATE, &[("REFORGED_SERVER_MODE", Some("public"))]),
            with(&PRIVATE, &[("REFORGED_SERVER_MODE", None)]),
            with(&PRIVATE, &[("REFORGED_SERVER_HOST", Some("203.0.113.7"))]),
            with(&PRIVATE, &[("REFORGED_SERVER_HOST", Some("192.168.1.10"))]),
            with(&PRIVATE, &[("REFORGED_SERVER_HOST", Some("hunts.example.net"))]),
            with(&PRIVATE, &[("REFORGED_XP_MULTIPLIER", Some("1000"))]),
            with(&PRIVATE, &[("REFORGED_REWARD_MULTIPLIER", Some("0.5"))]),
            with(&PRIVATE, &[("REFORGED_XP_MULTIPLIER", Some("abc"))]),
            Settings::from_pairs([
                ("REFORGED_SERVER_MODE", "private"),
                ("REFORGED_SERVER_HOST", "100.101.102.103"),
            ]),
        ];
        for settings in refused {
            let m = read_test_multipliers(&settings);
            assert_eq!((m.xp, m.rewards), (1.0, 1.0), "{settings:?}");
            assert!(m.reason.is_some());
        }
        assert!(!is_tailscale_host("100.63.255.255"));
        assert!(!is_tailscale_host("100.128.0.1"));
        assert!(is_tailscale_host("100.64.0.1"));
        assert!(!is_tailscale_host(".ts.net"));
        assert!(!is_tailscale_host("evil.example/x.ts.net"));
    }

    #[test]
    fn the_start_up_line_says_why_they_are_off() {
        assert_eq!(
            read_test_multipliers(&Settings::default()).startup_line(),
            "[TestMultiplier] off (not set)"
        );
        assert_eq!(
            read_test_multipliers(&private()).startup_line(),
            "[TestMultiplier] ON: XP x5, rewards x3 (private Tailscale server)"
        );
    }

    #[test]
    fn xp_grants_are_multiplied_and_capped_at_the_route_maximum() {
        assert_eq!(multiply_xp_grant(125.0, 100_000.0, &on(5.0, 1.0)), 625.0);
        assert_eq!(multiply_xp_grant(50_000.0, 100_000.0, &on(5.0, 1.0)), 100_000.0);
        assert_eq!(multiply_xp_grant(125.0, 100_000.0, &on(1.0, 1.0)), 125.0);
        assert_eq!(multiply_xp_grant(3.0, 100_000.0, &on(1.5, 1.0)), 4.0);
    }

    #[test]
    fn only_currencies_and_materials_of_a_reward_transaction_are_multiplied() {
        let never = |_: &str, _: f64| false;
        let multipliers = on(1.0, 3.0);
        let loot = json!([
            { "catalogId": "TOKEN_KILL_COUNT_LERAWR_BETA", "quantity": 1 },
            { "catalogId": "CURRENCY_NOTES", "quantity": 100 },
            { "catalogId": "BREAK_LERAWR_HIDE", "quantity": 3 },
            { "catalogId": "CURRENCY_PJM_WEAPON", "quantity": 10 },
            { "catalogId": "GATHER_COMMON_01", "quantity": 1 },
        ]);
        assert_eq!(
            multiply(Some("PlayerLoot"), &loot, &never, &multipliers),
            Some(json!([
                { "catalogId": "TOKEN_KILL_COUNT_LERAWR_BETA", "quantity": 1 },
                { "catalogId": "CURRENCY_NOTES", "quantity": 300 },
                { "catalogId": "BREAK_LERAWR_HIDE", "quantity": 9 },
                { "catalogId": "CURRENCY_PJM_WEAPON", "quantity": 30 },
                { "catalogId": "GATHER_COMMON_01", "quantity": 3 },
            ]))
        );
        assert_eq!(
            multiply(
                Some("BUNDLE_GATHER_COMMON_01"),
                &json!([{ "catalogId": "GATHER_COMMON_01", "quantity": 1 }]),
                &never,
                &multipliers
            ),
            Some(json!([{ "catalogId": "GATHER_COMMON_01", "quantity": 3 }]))
        );
        let quest = json!([{ "catalogId": "TOKEN_UNLOCK_WEAPON_AXE", "quantity": 1 }, { "catalogId": "DYE_PURPLE06_DURABLE", "quantity": 1 }]);
        assert_eq!(multiply(Some("quest"), &quest, &never, &multipliers), None);
    }

    #[test]
    fn crafting_purchases_excluded_items_and_an_x1_server_keep_their_quantities() {
        let never = |_: &str, _: f64| false;
        let multipliers = on(1.0, 3.0);
        let notes = json!([{ "catalogId": "CURRENCY_NOTES", "quantity": 100 }]);
        for source in [
            Some("armorrecipe"),
            Some("PJM Unlock: GliderUpgrade_0"),
            Some("UBountyComponent::GrantBounty"),
            Some("default"),
            None,
        ] {
            assert_eq!(multiply(source, &notes, &never, &multipliers), None, "{source:?}");
        }
        assert_eq!(multiply(Some("PlayerLoot"), &notes, &never, &on(1.0, 1.0)), None);
        let prestige = json!([{ "catalogId": "CURRENCY_PJM_PRESTIGE_EMPTY", "quantity": 5 }]);
        assert_eq!(
            multiply(
                Some("patrol_chest"),
                &prestige,
                &|id: &str, _: f64| id == "CURRENCY_PJM_PRESTIGE_EMPTY",
                &multipliers
            ),
            None
        );
        assert_eq!(
            multiply(Some("quest"), &json!("not a list"), &never, &multipliers),
            None
        );
    }

    #[test]
    fn hosts_and_quantities_read_as_node_reads_them() {
        assert!(is_tailscale_host(" 100.127.255.255 "));
        assert!(!is_tailscale_host("100.063.0.1"));
        assert!(is_tailscale_host("MY-BOX.tail1234.TS.NET"));
        assert!(!is_tailscale_host("ts.net"));
        assert!(!is_tailscale_host("evil.ts.net.example.com"));
        assert!(!is_tailscale_host(""));
        let multipliers = on(1.0, 1.5);
        // `Number("4")` is 4; other fields keep their place; a fraction or a non-number keeps its quantity
        let items = json!([
            { "catalogId": "GATHER_ORE", "quantity": "4", "extra": 1 },
            { "catalogId": "CURRENCY_NOTES", "quantity": 2.5 },
            { "catalogId": "BREAK_X", "quantity": "lots" },
            { "catalogId": "CURRENCY_NOTES", "quantity": 7 },
        ]);
        assert_eq!(
            multiply(Some("MISC"), &items, &|_: &str, _: f64| false, &multipliers),
            Some(json!([
                { "catalogId": "GATHER_ORE", "quantity": 6, "extra": 1 },
                { "catalogId": "CURRENCY_NOTES", "quantity": 2.5 },
                { "catalogId": "BREAK_X", "quantity": "lots" },
                { "catalogId": "CURRENCY_NOTES", "quantity": 10 },
            ]))
        );
        assert_eq!(
            multiply_reward_items(
                Some(&json!(7)),
                &[json!({ "catalogId": "CURRENCY_NOTES", "quantity": 1 })],
                &|_: &str, _: f64| false,
                &on(1.0, 3.0)
            ),
            None
        );
    }
}
