// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! nftables (Linux): the helper's own table, replaced whole in one nft transaction, so there is never a moment
//! with the game ports open to everyone or the table missing. nft runs at a fixed path with a fixed argument
//! list, never through PATH or a shell, and every value is checked again where the script is written.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::json;
use service_kit::linelog;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::firewall::{RulePlan, plan};
use crate::json_string;

/// The table's default name.
pub const NFT_TABLE: &str = "reforged_revival";

const RUN_LIMIT: Duration = Duration::from_secs(30);
const CHECK_LIMIT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftSettings {
    pub table: String,
    pub interface: Option<String>,
}

/// An ASCII letter, then up to 31 letters, digits or `_`.
pub fn check_nft_table(name: &str) -> Result<String, String> {
    let ok = name.len() <= 32
        && name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if ok {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "ALLOWLIST_NFT_TABLE must be a letter then up to 31 letters, digits or _ (got {})",
            json_string(name)
        ))
    }
}

/// An ASCII letter or digit, then letters, digits or `_`, `.`, `@`, `-`, at most 15 in all.
pub fn check_interface(name: &str) -> Result<String, String> {
    let ok = name.len() <= 15
        && name.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'@' | b'-'));
    if ok {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "ALLOWLIST_INTERFACE must be a network interface name like eth0 (got {})",
            json_string(name)
        ))
    }
}

pub fn check_nft_binary(path: &str) -> Result<String, String> {
    if path.starts_with('/') {
        Ok(path.to_owned())
    } else {
        Err("ALLOWLIST_NFT must be the absolute path of nft (the helper never looks it up in PATH)".to_owned())
    }
}

/// The first of `/usr/sbin/nft` and `/sbin/nft` that exists, else `/usr/sbin/nft`.
pub fn default_nft() -> String {
    ["/usr/sbin/nft", "/sbin/nft"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .unwrap_or("/usr/sbin/nft")
        .to_owned()
}

fn set_lines(out: &mut String, name: &str, kind: &str, members: &[&str]) {
    out.push_str(&format!("\tset {name} {{\n\t\ttype {kind}\n"));
    if !members.is_empty() {
        out.push_str(&format!("\t\telements = {{ {} }}\n", members.join(", ")));
    }
    out.push_str("\t}\n");
}

/// The whole script for `plan`, after `plan` and both name checks run again.
pub fn nft_script(rule: &RulePlan, settings: &NftSettings) -> Result<String, String> {
    let rule = plan(&rule.ips, &rule.ports)?;
    let table = check_nft_table(&settings.table)?;
    let iif = match &settings.interface {
        Some(name) => format!("iifname \"{}\" ", check_interface(name)?),
        None => String::new(),
    };
    let (v6, v4): (Vec<&str>, Vec<&str>) = rule.ips.iter().map(String::as_str).partition(|ip| ip.contains(':'));
    let ports = if rule.ports.contains(',') {
        format!("{{ {} }}", rule.ports.split(',').collect::<Vec<_>>().join(", "))
    } else {
        rule.ports.clone()
    };
    let mut out = format!("table inet {table} {{}}\ndelete table inet {table}\ntable inet {table} {{\n");
    set_lines(&mut out, "players4", "ipv4_addr", &v4);
    set_lines(&mut out, "players6", "ipv6_addr", &v6);
    out.push_str(
        "\tchain input {\n\t\ttype filter hook input priority -5; policy accept;\n\t\tiifname \"lo\" accept\n",
    );
    out.push_str(&format!("\t\t{iif}ip saddr @players4 udp dport {ports} accept\n"));
    out.push_str(&format!("\t\t{iif}ip6 saddr @players6 udp dport {ports} accept\n"));
    out.push_str(&format!("\t\t{iif}udp dport {ports} drop\n"));
    out.push_str("\t}\n}\n");
    Ok(out)
}

/// How nft takes the script: unknown until a run settles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Unknown,
    Stdin,
    File,
}

/// nft at a fixed path.
#[derive(Debug)]
pub struct Nft {
    path: String,
    feed: Mutex<Feed>,
}

/// A private folder (`reforged-nft-*`, 0700) holding `rules.nft` (0600, created new); both go with the guard.
pub fn private_script(script: &str) -> std::io::Result<(tempfile::TempDir, std::path::PathBuf)> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("reforged-nft-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let dir = builder.tempdir()?;
    let file = dir.path().join("rules.nft");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    std::io::Write::write_all(&mut options.open(&file)?, script.as_bytes())?;
    Ok((dir, file))
}

/// The first 1000 characters (trimmed) of stderr's last 4000 bytes.
fn detail(stderr: &[u8]) -> String {
    let tail = &stderr[stderr.len().saturating_sub(4000)..];
    String::from_utf8_lossy(tail)
        .trim()
        .chars()
        .take(1000)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Runs nft with `args`, `stdin` written to it, within `limit`.
async fn run(path: &str, args: &[&str], stdin: Option<&str>, limit: Duration) -> Result<(), String> {
    let mut command = tokio::process::Command::new(path);
    command
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| format!("could not run {path}: {e}"))?;
    let feeding = child.stdin.take();
    let mut stderr = child.stderr.take();
    let work = async {
        if let (Some(mut pipe), Some(script)) = (feeding, stdin) {
            // A broken pipe is ignored: the exit code says what happened
            _ = pipe.write_all(script.as_bytes()).await;
            drop(pipe);
        }
        let mut errors = Vec::new();
        if let Some(stderr) = stderr.as_mut() {
            _ = stderr.read_to_end(&mut errors).await;
        }
        let status = child.wait().await.map_err(|e| format!("could not run {path}: {e}"))?;
        if status.success() {
            return Ok(());
        }
        let code = status.code().map_or_else(|| "a signal".to_owned(), |c| c.to_string());
        let detail = detail(&errors);
        Err(if detail.is_empty() {
            format!("nft exited with {code}")
        } else {
            format!("nft exited with {code}: {detail}")
        })
    };
    tokio::time::timeout(limit, work)
        .await
        .unwrap_or_else(|_| Err(format!("nft did not finish within {} ms", limit.as_millis())))
}

impl Nft {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            feed: Mutex::new(Feed::Unknown),
        }
    }

    pub fn feed(&self) -> Feed {
        *self.feed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn settle(&self, feed: Feed) {
        *self.feed.lock().unwrap_or_else(PoisonError::into_inner) = feed;
    }

    async fn with_file(&self, script: &str) -> Result<(), String> {
        let (dir, file) = private_script(script).map_err(|e| format!("could not write the nft script: {e}"))?;
        let file = file.to_string_lossy().into_owned();
        let result = run(&self.path, &["-f", &file], None, RUN_LIMIT).await;
        drop(dir);
        result
    }

    /// Applies a script: stdin first, a private file when nft refuses stdin; whichever works is kept.
    pub async fn apply(&self, script: &str) -> Result<(), String> {
        match self.feed() {
            Feed::Stdin => run(&self.path, &["-f", "-"], Some(script), RUN_LIMIT).await,
            Feed::File => self.with_file(script).await,
            Feed::Unknown => match run(&self.path, &["-f", "-"], Some(script), RUN_LIMIT).await {
                Ok(()) => {
                    self.settle(Feed::Stdin);
                    Ok(())
                }
                Err(stdin_error) => {
                    self.with_file(script).await?;
                    linelog::warn(
                        "nft refused the script on stdin; using a private file from now on",
                        json!({ "detail": stdin_error }),
                    );
                    self.settle(Feed::File);
                    Ok(())
                }
            },
        }
    }
}

/// Read-only, at start in real mode: the path is a file and `list tables` works.
pub async fn can_manage_nft(path: &str) -> bool {
    Path::new(path).is_file() && run(path, &["list", "tables"], None, CHECK_LIMIT).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(ips: &[&str], ports: &str) -> RulePlan {
        plan(&ips.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(), ports).unwrap()
    }

    #[test]
    fn names() {
        assert!(check_nft_table("reforged_revival").is_ok());
        for bad in ["1table", "my-table", "_t", &"t".repeat(33)] {
            assert!(check_nft_table(bad).is_err(), "{bad}");
        }
        assert!(check_nft_table(&"t".repeat(32)).is_ok());
        for good in ["eth0", "ens3", "wg0", "br-lan", "eth0.100", "0abc"] {
            assert!(check_interface(good).is_ok(), "{good}");
        }
        for bad in ["a\"b", "a b", "../x", "-eth0", "eth0:1", &"e".repeat(16)] {
            assert!(check_interface(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_exact_script() {
        let settings = NftSettings {
            table: "reforged_revival".into(),
            interface: Some("eth0".into()),
        };
        let script = nft_script(
            &rule(&["203.0.113.7", "2001:db8::1", "198.51.100.2"], "8770-8777"),
            &settings,
        )
        .unwrap();
        assert_eq!(
            script,
            "table inet reforged_revival {}\n\
             delete table inet reforged_revival\n\
             table inet reforged_revival {\n\
             \tset players4 {\n\
             \t\ttype ipv4_addr\n\
             \t\telements = { 203.0.113.7, 198.51.100.2 }\n\
             \t}\n\
             \tset players6 {\n\
             \t\ttype ipv6_addr\n\
             \t\telements = { 2001:db8::1 }\n\
             \t}\n\
             \tchain input {\n\
             \t\ttype filter hook input priority -5; policy accept;\n\
             \t\tiifname \"lo\" accept\n\
             \t\tiifname \"eth0\" ip saddr @players4 udp dport 8770-8777 accept\n\
             \t\tiifname \"eth0\" ip6 saddr @players6 udp dport 8770-8777 accept\n\
             \t\tiifname \"eth0\" udp dport 8770-8777 drop\n\
             \t}\n\
             }\n"
        );
    }

    #[test]
    fn closed_and_listed_ports() {
        let settings = NftSettings {
            table: "t".into(),
            interface: None,
        };
        let closed = nft_script(&rule(&[], "8770-8777"), &settings).unwrap();
        assert!(!closed.contains("elements"));
        assert_eq!(closed.lines().count(), 3 + 3 + 3 + 8);
        assert!(closed.contains("\t\tudp dport 8770-8777 drop\n"));
        let list = nft_script(&rule(&["203.0.113.7"], "8770-8777,64000-64009,64020"), &settings).unwrap();
        assert!(list.contains("udp dport { 8770-8777, 64000-64009, 64020 } drop"));
        assert_eq!(list.lines().count(), 3 + 3 + 3 + 8 + 1);
        // Nothing unchecked reaches the script
        let forged = RulePlan {
            ips: vec!["1.2.3.4 } accept; #".into()],
            ports: "8770".into(),
            enabled: true,
        };
        assert!(nft_script(&forged, &settings).is_err());
        let bad_table = NftSettings {
            table: "t; flush ruleset".into(),
            interface: None,
        };
        assert!(nft_script(&rule(&[], "8770"), &bad_table).is_err());
    }

    #[test]
    fn stderr_detail() {
        let long = vec![b'x'; 5000];
        assert_eq!(detail(&long).len(), 1000);
        assert_eq!(detail(b"  oops \n"), "oops");
    }

    #[test]
    fn private_file_is_new_and_goes_away() {
        let (dir, file) = private_script("table inet t {}\n").unwrap();
        assert!(
            dir.path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("reforged-nft-")
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "table inet t {}\n");
        let path = dir.path().to_owned();
        drop(dir);
        assert!(!path.exists());
    }
}
