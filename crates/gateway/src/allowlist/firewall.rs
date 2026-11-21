// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The firewall plan, and the backends that bring a firewall to it. Nothing reaches a firewall without passing
//! through [`plan`] again, whoever the caller.

use serde_json::{Value, json};

use super::config::FirewallKind;
use super::nft::{Nft, NftSettings, nft_script};
use super::ports::check_port_list;
use crate::ip::{AllowlistCheck, check_allowlist_ip};
use crate::json_string;

/// The rule's id, as the firewall's own tools name it.
pub const RULE_NAME: &str = "DaintyLass-GamePorts-Allowlist";
pub const RULE_DISPLAY_NAME: &str = "DaintyLass game ports (allowlist)";
pub const RULE_GROUPING: &str = "DaintyLass";
pub const RULE_DESCRIPTION: &str =
    "Opens the game ports to players who logged in through the gateway. Managed by the allowlist helper.";

/// Addresses (as given, in order) and the checked ports; enabled when it has an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePlan {
    pub ips: Vec<String>,
    pub ports: String,
    pub enabled: bool,
}

/// Each address a canonical single address (private allowed), the ports a checked list.
pub fn plan(ips: &[String], ports: &str) -> Result<RulePlan, String> {
    let ports = check_port_list(ports)?;
    for ip in ips {
        match check_allowlist_ip(Some(ip), true) {
            AllowlistCheck::Allowed(parsed) if parsed.canonical == *ip => {}
            _ => return Err(format!("refusing to change the firewall with {}", json_string(ip))),
        }
    }
    Ok(RulePlan {
        ips: ips.to_vec(),
        ports,
        enabled: !ips.is_empty(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRule {
    pub display_name: String,
    pub description: String,
    pub grouping: String,
    pub direction: String,
    pub action: String,
    pub protocol: String,
    pub local_ports: String,
    pub remote_addresses: String,
    pub profiles: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsChange {
    /// Every rule with the name disabled: Windows then drops the game ports' traffic by default
    Close,
    Open(Box<WindowsRule>),
}

impl WindowsChange {
    /// As the dry run writes it.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Close => json!({ "enabled": false }),
            Self::Open(r) => json!({
                "enabled": true,
                "displayName": r.display_name,
                "direction": r.direction,
                "action": r.action,
                "protocol": r.protocol,
                "localPorts": r.local_ports,
                "remoteAddresses": r.remote_addresses,
                "profiles": r.profiles,
            }),
        }
    }
}

pub fn windows_change(plan: &RulePlan, name: &str) -> WindowsChange {
    if !plan.enabled {
        return WindowsChange::Close;
    }
    WindowsChange::Open(Box::new(WindowsRule {
        display_name: name.to_owned(),
        description: RULE_DESCRIPTION.to_owned(),
        grouping: RULE_GROUPING.to_owned(),
        direction: "in".to_owned(),
        action: "allow".to_owned(),
        protocol: "udp".to_owned(),
        local_ports: plan.ports.clone(),
        remote_addresses: plan.ips.join(","),
        profiles: "all".to_owned(),
    }))
}

/// Where a plan goes.
#[derive(Debug)]
pub enum Backend {
    /// Never touches the firewall: every change is only described
    DryRun {
        firewall: FirewallKind,
        nft: NftSettings,
        rule_name: String,
    },
    Nftables {
        nft: Nft,
        settings: NftSettings,
    },
    Windows {
        rule_name: String,
    },
}

impl Backend {
    pub fn firewall(&self) -> FirewallKind {
        match self {
            Self::DryRun { firewall, .. } => *firewall,
            Self::Nftables { .. } => FirewallKind::Nftables,
            Self::Windows { .. } => FirewallKind::Windows,
        }
    }

    /// `windows-firewall` or `linux-nftables`, dry run included.
    pub fn name(&self) -> &'static str {
        self.firewall().backend_name()
    }

    pub fn is_dry_run(&self) -> bool {
        matches!(self, Self::DryRun { .. })
    }

    /// The `dry_run` event's fields for `plan`; the real backends describe a plan the same way.
    pub fn describe(&self, rule: &RulePlan) -> Result<Value, String> {
        let rule = plan(&rule.ips, &rule.ports)?;
        let firewall = self.firewall();
        let mut fields = json!({
            "firewall": firewall.as_str(),
            "rule": "",
            "enabled": rule.enabled,
            "ips": rule.ips,
            "ports": rule.ports,
            "backend": self.name(),
        });
        let (name, extra_key, extra) = match (self, firewall) {
            (Self::DryRun { nft, .. }, FirewallKind::Nftables) | (Self::Nftables { settings: nft, .. }, _) => (
                format!("table inet {}", nft.table),
                "script",
                Value::String(nft_script(&rule, nft)?),
            ),
            (Self::DryRun { rule_name, .. } | Self::Windows { rule_name }, _) => {
                (rule_name.clone(), "windows", windows_change(&rule, rule_name).to_json())
            }
        };
        if let Value::Object(map) = &mut fields {
            map.insert("rule".into(), Value::String(name));
            map.insert(extra_key.into(), extra);
        }
        Ok(fields)
    }

    /// Brings the firewall to `plan`; a dry run returns what it would have done.
    pub async fn apply(&self, rule: &RulePlan) -> Result<Option<Value>, String> {
        let rule = plan(&rule.ips, &rule.ports)?;
        match self {
            Self::DryRun { .. } => self.describe(&rule).map(Some),
            Self::Nftables { nft, settings } => {
                let script = nft_script(&rule, settings)?;
                nft.apply(&script).await.map(|()| None)
            }
            Self::Windows { rule_name } => {
                let change = windows_change(&rule, rule_name);
                let name = rule_name.clone();
                apply_windows(change, name).await.map(|()| None)
            }
        }
    }
}

#[cfg(windows)]
async fn apply_windows(change: WindowsChange, name: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || super::windows::apply_rule(&change, &name))
        .await
        .map_err(|e| format!("Windows Firewall: {e}"))?
}

#[cfg(not(windows))]
async fn apply_windows(_change: WindowsChange, _name: String) -> Result<(), String> {
    Err(
        "ALLOWLIST_FIREWALL=windows needs Windows (Windows Firewall); on Linux use ALLOWLIST_FIREWALL=nftables"
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(ips: &[&str]) -> Vec<String> {
        ips.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn plans() {
        let p = plan(&list(&["203.0.113.7", "10.0.0.1", "2001:db8::1"]), "08770-8777").unwrap();
        assert_eq!(p.ports, "8770-8777");
        assert!(p.enabled);
        assert!(!plan(&[], "8770").unwrap().enabled);
        for bad in [
            "2001:DB8::1",
            "1.2.3.4/32",
            "0.0.0.0",
            "any",
            "",
            "010.1.1.1",
            "224.0.0.1",
            "1.2.3.4\"; rm -rf /",
        ] {
            assert_eq!(
                plan(&list(&[bad]), "8770").unwrap_err(),
                format!("refusing to change the firewall with {}", json_string(bad)),
                "{bad}"
            );
        }
        assert!(plan(&[], "8770,").is_err());
    }

    #[test]
    fn windows_changes() {
        let open = windows_change(
            &plan(&list(&["203.0.113.7", "198.51.100.2"]), "8770-8777,64000-64009").unwrap(),
            RULE_DISPLAY_NAME,
        );
        assert_eq!(
            open.to_json(),
            json!({"enabled":true,"displayName":"DaintyLass game ports (allowlist)","direction":"in","action":"allow",
                "protocol":"udp","localPorts":"8770-8777,64000-64009","remoteAddresses":"203.0.113.7,198.51.100.2","profiles":"all"})
        );
        assert_eq!(
            windows_change(&plan(&[], "8770").unwrap(), "x").to_json(),
            json!({"enabled":false})
        );
        assert_eq!(
            serde_json::to_string(&windows_change(&plan(&[], "8770").unwrap(), "x").to_json()).unwrap(),
            r#"{"enabled":false}"#
        );
    }

    #[tokio::test]
    async fn dry_runs_describe_and_never_apply() {
        let nft = Backend::DryRun {
            firewall: FirewallKind::Nftables,
            nft: NftSettings {
                table: "t".into(),
                interface: None,
            },
            rule_name: RULE_DISPLAY_NAME.into(),
        };
        let p = plan(&list(&["203.0.113.7"]), "8770").unwrap();
        let described = nft.apply(&p).await.unwrap().unwrap();
        assert_eq!(described["firewall"], "nftables");
        assert_eq!(described["rule"], "table inet t");
        assert_eq!(described["backend"], "linux-nftables");
        assert!(
            described["script"]
                .as_str()
                .unwrap()
                .contains("elements = { 203.0.113.7 }")
        );
        let windows = Backend::DryRun {
            firewall: FirewallKind::Windows,
            nft: NftSettings {
                table: "t".into(),
                interface: None,
            },
            rule_name: RULE_DISPLAY_NAME.into(),
        };
        let described = windows.apply(&p).await.unwrap().unwrap();
        assert_eq!(described["rule"], RULE_DISPLAY_NAME);
        assert_eq!(described["windows"]["remoteAddresses"], "203.0.113.7");
        assert_eq!(described["backend"], "windows-firewall");
    }
}
