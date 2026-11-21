// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The allowlist's rules applied to the real firewall, which every other test only describes: nftables on Linux
//! (root or CAP_NET_ADMIN), Windows Firewall on Windows (an elevated prompt). Opt-in with `DAINTY_TEST_FIREWALL=1`;
//! `restricted/docker/test-in-image.sh` sets it when the container has the capability. Each works in a table or
//! rule of its own on ports nothing uses and removes it, pass or fail (docs/security/allowlist.md, live checks).
#![cfg(any(target_os = "linux", windows))]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use dainty_gateway::allowlist::firewall::{RulePlan, plan};

const PORTS: &str = "64770-64771";

fn wanted() -> bool {
    let on = std::env::var("DAINTY_TEST_FIREWALL").as_deref() == Ok("1");
    if !on {
        eprintln!("skipped: DAINTY_TEST_FIREWALL=1 changes the real firewall (root, CAP_NET_ADMIN or elevated)");
    }
    on
}

fn ips(list: &[&str]) -> RulePlan {
    plan(&list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(), PORTS).unwrap()
}

#[cfg(target_os = "linux")]
mod nftables {
    use std::process::Command;

    use dainty_gateway::allowlist::firewall::Backend;
    use dainty_gateway::allowlist::nft::{Nft, NftSettings, can_manage_nft, default_nft};

    use super::{PORTS, ips, wanted};

    const TABLE: &str = "dainty_lass_test";

    fn nft(args: &[&str]) -> std::process::Output {
        Command::new(default_nft()).args(args).output().unwrap()
    }

    fn listed() -> String {
        let out = nft(&["list", "table", "inet", TABLE]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    /// The members of one of the table's sets, as nft lists them, sorted.
    fn members(set: &str) -> Vec<String> {
        let listed = listed();
        let start = listed
            .find(&format!("set {set} {{"))
            .unwrap_or_else(|| panic!("no {set} in\n{listed}"));
        let body = &listed[start..];
        let body = &body[..body.find('}').unwrap()];
        let mut out: Vec<String> = body
            .split_once("elements = {")
            .map(|(_, elements)| {
                elements
                    .split(',')
                    .map(|e| e.trim().to_owned())
                    .filter(|e| !e.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    /// Deletes the table however the test ends.
    struct Cleanup;

    impl Drop for Cleanup {
        fn drop(&mut self) {
            _ = nft(&["delete", "table", "inet", TABLE]);
        }
    }

    fn backend(interface: Option<&str>) -> Backend {
        Backend::Nftables {
            nft: Nft::new(default_nft()),
            settings: NftSettings {
                table: TABLE.to_owned(),
                interface: interface.map(str::to_owned),
            },
        }
    }

    #[tokio::test]
    async fn the_rules_close_the_game_ports_and_admit_exactly_the_players_given() {
        if !wanted() {
            return;
        }
        let path = default_nft();
        assert!(
            can_manage_nft(&path).await,
            "{path} cannot list tables: the test needs root or CAP_NET_ADMIN"
        );
        let cleanup = Cleanup;
        let backend = backend(None);

        // Nobody admitted: the ports are dropped for everyone, loopback still accepted
        assert_eq!(backend.apply(&ips(&[])).await.unwrap(), None);
        let text = listed();
        // nft lists the script's priority -5 back by name, as filter - 5
        assert!(text.contains("type filter hook input priority"), "{text}");
        assert!(text.contains("policy accept;"), "{text}");
        assert!(text.contains("iifname \"lo\" accept"), "{text}");
        assert!(text.contains(&format!("udp dport {PORTS} drop")), "{text}");
        assert!(
            members("players4").is_empty() && members("players6").is_empty(),
            "{text}"
        );

        // Players admitted by family, ahead of the drop
        backend
            .apply(&ips(&["203.0.113.7", "198.51.100.20", "2001:db8::1"]))
            .await
            .unwrap();
        assert_eq!(members("players4"), ["198.51.100.20", "203.0.113.7"]);
        assert_eq!(members("players6"), ["2001:db8::1"]);
        let text = listed();
        let accept = text
            .find(&format!("ip saddr @players4 udp dport {PORTS} accept"))
            .unwrap();
        assert!(
            accept < text.find(&format!("udp dport {PORTS} drop")).unwrap(),
            "{text}"
        );
        assert!(
            text.contains(&format!("ip6 saddr @players6 udp dport {PORTS} accept")),
            "{text}"
        );

        // Each apply replaces the table whole: a player left out is gone
        backend.apply(&ips(&["198.51.100.20"])).await.unwrap();
        assert_eq!(members("players4"), ["198.51.100.20"]);
        assert!(members("players6").is_empty());

        // An interface narrows every game-port rule to it
        super::nftables::backend(Some("lo")).apply(&ips(&[])).await.unwrap();
        assert!(listed().contains(&format!("iifname \"lo\" udp dport {PORTS} drop")));

        drop(cleanup);
        assert!(
            !nft(&["list", "table", "inet", TABLE]).status.success(),
            "the table was left behind"
        );
    }
}

#[cfg(windows)]
mod windows_firewall {
    use dainty_gateway::allowlist::firewall::windows_change;
    use dainty_gateway::allowlist::windows::{apply_rule, is_elevated, read_rules, remove_rules};

    use super::{PORTS, ips, wanted};

    const RULE: &str = "Dainty Lass allowlist test (delete me)";

    /// Removes the rule however the test ends.
    struct Cleanup;

    impl Drop for Cleanup {
        fn drop(&mut self) {
            _ = remove_rules(RULE);
        }
    }

    fn apply(list: &[&str]) {
        apply_rule(&windows_change(&ips(list), RULE), RULE).unwrap();
    }

    #[test]
    fn the_rule_opens_the_game_ports_to_exactly_the_players_given_and_closes() {
        if !wanted() {
            return;
        }
        assert!(is_elevated(), "the test needs an elevated prompt");
        let cleanup = Cleanup;

        // Nobody admitted and no rule yet: nothing is made
        apply(&[]);
        assert!(read_rules(RULE).unwrap().is_empty());

        // Admitted: one enabled inbound UDP allow rule on the game ports, for exactly those addresses
        apply(&["203.0.113.7", "198.51.100.20"]);
        let rules = read_rules(RULE).unwrap();
        assert_eq!(rules.len(), 1, "{rules:?}");
        let rule = &rules[0];
        assert!(rule.enabled);
        assert_eq!((rule.protocol, rule.direction, rule.action), (17, 1, 1), "{rule:?}");
        assert_eq!(rule.local_ports, PORTS);
        for ip in ["203.0.113.7", "198.51.100.20"] {
            assert!(rule.remote_addresses.contains(ip), "{rule:?}");
        }

        // The same rule is set again, not a second one made
        apply(&["198.51.100.20"]);
        let rules = read_rules(RULE).unwrap();
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert!(!rules[0].remote_addresses.contains("203.0.113.7"), "{rules:?}");

        // Nobody admitted: the rule stays, disabled
        apply(&[]);
        assert!(read_rules(RULE).unwrap().iter().all(|r| !r.enabled));

        drop(cleanup);
        assert!(read_rules(RULE).unwrap().is_empty(), "the rule was left behind");
    }
}
