// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `reforged-host network` (DN3): the cluster CA and the node certificates, and a look at this server's
//! place in a network.
//!
//! - `ca-init` makes the CA file (certificate and key) on the owner's own machine. It is the one thing that can
//!   sign a node, so it is never put on a node: `issue-node-cert` takes it by `--ca <file>` each time and
//!   stores nothing of it.
//! - `issue-node-cert` writes a node's three files: its certificate, its key and the CA's certificate. The
//!   owner copies them to that node's `.workspace/tls` folder, and lists the node under `[[network.nodes]]` on
//!   every node that should answer or call it.
//! - `list` shows what a CA file has issued (`--ca`), or this server's `[network]` and its node files.
//! - `status` pings every peer over the node link with this server's own certificate (DN12's harness reads it).
//!
//! No checkout is needed except by `list` without `--ca`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use network_plugin::cluster::{self, ClusterCa, DEFAULT_NODE_DAYS, NodeCertInfo, NodeCertRequest, NodeCredentials};
use network_plugin::{Directory, DirectoryConfig, PeerState, TlsLinkConfig, TlsPeerLink};
use service_kit::env::Env;

use crate::Outcome;
use crate::cli::{CaInitArgs, IssueNodeCertArgs, JoinArgs, ListArgs, NetworkArgs, NetworkCommand, StatusArgs};
use crate::error::HostError;
use crate::hostconfig::{CLUSTER_CA_FILE, NODE_CERT_FILE, NODE_KEY_FILE, load_host_config, node_database_config};
use crate::paths::Paths;

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

pub fn run(args: &NetworkArgs, paths: Option<&Paths>) -> Outcome {
    match &args.command {
        NetworkCommand::CaInit(a) => ca_init(a),
        NetworkCommand::IssueNodeCert(a) => issue_node_cert(a),
        NetworkCommand::List(a) => list(a, paths),
        NetworkCommand::Status(a) => status(a, paths),
        NetworkCommand::Join(a) => join(a, paths),
    }
}

fn date(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0).map_or_else(|| "?".into(), |t| t.format("%Y-%m-%d").to_string())
}

/// Written owner-only where the file system has modes. `force` replaces a file that is there.
fn write_file(file: &Path, text: &str, secret: bool, force: bool) -> Result<(), HostError> {
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    #[cfg(unix)]
    if secret {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let _ = secret;
    use std::io::Write;
    let mut handle = options.open(file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            bad(format!("{} exists; add --force to replace it", file.display()))
        } else {
            HostError::io(file, e)
        }
    })?;
    handle.write_all(text.as_bytes()).map_err(|e| HostError::io(file, e))
}

fn ca_init(a: &CaInitArgs) -> Outcome {
    let out = PathBuf::from(a.out.as_deref().unwrap_or("cluster-ca.pem"));
    let ca = ClusterCa::create(cluster::CA_DAYS).map_err(|e| bad(e.to_string()))?;
    write_file(&out, &ca.to_pem(), true, a.force)?;
    let out = std::path::absolute(&out).unwrap_or(out);
    println!(
        "Cluster CA: {}\n\nThis file signs every node's certificate and holds the CA's key. Keep it on your own\n\
         machine and back it up; never copy it to a node. If it is lost or leaks, make a new CA and issue\n\
         every node's certificate again.\n\nNext: reforged-host network issue-node-cert --ca {} --node <id> --region <region> --host <address>",
        out.display(),
        out.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn index_file(ca_file: &Path) -> PathBuf {
    ca_file.with_extension("issued.json")
}

fn read_index(ca_file: &Path) -> Result<Vec<NodeCertInfo>, HostError> {
    let file = index_file(ca_file);
    match std::fs::read_to_string(&file) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| bad(format!("{}: {e}", file.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(HostError::io(&file, e)),
    }
}

fn issue_node_cert(a: &IssueNodeCertArgs) -> Outcome {
    let ca_file = PathBuf::from(&a.ca);
    let ca = ClusterCa::read(&ca_file).map_err(|e| bad(e.to_string()))?;
    let request = NodeCertRequest {
        node: a.node.parse().map_err(|e| bad(format!("--node: {e}")))?,
        region: a.region.parse().map_err(|e| bad(format!("--region: {e}")))?,
        hosts: a.host.clone(),
        days: match a.days.as_deref() {
            None => DEFAULT_NODE_DAYS,
            Some(days) => days
                .parse()
                .map_err(|_| bad(format!("--days must be a whole number of days, not {days:?}")))?,
        },
    };
    let issued = ca.issue(&request).map_err(|e| bad(e.to_string()))?;
    let out = PathBuf::from(a.out.clone().unwrap_or_else(|| a.node.clone()));
    let files = [
        (NODE_CERT_FILE, &issued.cert_pem, false),
        (NODE_KEY_FILE, &issued.key_pem, true),
        (CLUSTER_CA_FILE, &issued.ca_cert_pem, false),
    ];
    for (name, _, _) in &files {
        let file = out.join(name);
        if file.exists() && !a.force {
            return Err(bad(format!("{} exists; add --force to replace it", file.display())));
        }
    }
    for (name, text, secret) in files {
        write_file(&out.join(name), text, secret, true)?;
    }
    let mut index = read_index(&ca_file)?;
    let earlier = index
        .iter()
        .filter(|c| c.identity.node == request.node)
        .map(|c| date(c.not_after))
        .collect::<Vec<_>>();
    index.push(issued.info.clone());
    let text = serde_json::to_string_pretty(&index).map_err(|e| bad(e.to_string()))?;
    write_file(&index_file(&ca_file), &(text + "\n"), false, true)?;
    let info = &issued.info;
    let out = std::path::absolute(&out).unwrap_or(out);
    println!(
        "Node {} in {}: {}\nNames:       {}\nValid:       {} to {}\nFingerprint: {}\n\n\
         Copy {NODE_CERT_FILE}, {NODE_KEY_FILE} (private) and {CLUSTER_CA_FILE} to that server's .workspace/tls folder,\n\
         then list it under [[network.nodes]] in the host.toml of every node that should answer or call it.",
        info.identity.node,
        info.identity.region,
        out.display(),
        info.hosts.join(", "),
        date(info.not_before),
        date(info.not_after),
        info.fingerprint
    );
    if !earlier.is_empty() {
        println!(
            "\nThis node had a certificate before (valid until {}); it still works. To cut a node off, remove it from the\n\
             other nodes' [[network.nodes]]: a certificate alone doesn't let it in.",
            earlier.join(", ")
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn state(info: &NodeCertInfo, now: i64) -> &'static str {
    if info.not_after <= now {
        "expired"
    } else if info.not_after - now < 30 * 86_400 {
        "expires soon"
    } else {
        "valid"
    }
}

fn list(a: &ListArgs, paths: Option<&Paths>) -> Outcome {
    let now = chrono::Utc::now().timestamp();
    if let Some(ca) = &a.ca {
        let ca_file = PathBuf::from(ca);
        ClusterCa::read(&ca_file).map_err(|e| bad(e.to_string()))?;
        let issued = read_index(&ca_file)?;
        if issued.is_empty() {
            println!("This CA has issued no node certificates yet.");
        }
        for info in &issued {
            println!(
                "{:<20} {:<14} {} to {}  {:<12} {}",
                info.identity.node,
                info.identity.region,
                date(info.not_before),
                date(info.not_after),
                state(info, now),
                info.hosts.join(", ")
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    let paths = paths
        .ok_or_else(|| bad("run this in the server's folder, or give --ca <the CA file> to list what it issued"))?;
    let config = load_host_config(paths)?.ok_or_else(|| bad("no host.toml: run `reforged-host setup` first"))?;
    let Some(network) = &config.network else {
        println!("This server is not in a network ([network] is not in host.toml).");
        return Ok(ExitCode::SUCCESS);
    };
    println!("This node: {} in {}", network.node, network.region);
    if network.nodes.is_empty() {
        println!("Peers:     none ([[network.nodes]] is empty): node links are off");
    }
    for peer in &network.nodes {
        println!(
            "Peer:      {:<20} {:<14} {}:{}{}",
            peer.id,
            peer.region,
            peer.host,
            peer.port,
            if peer.fingerprint.is_some() {
                ""
            } else {
                "  (no fingerprint: not listed to players)"
            }
        );
    }
    let (cert, key, ca) = (
        paths.tls.join(NODE_CERT_FILE),
        paths.tls.join(NODE_KEY_FILE),
        paths.tls.join(CLUSTER_CA_FILE),
    );
    match NodeCredentials::load(&cert, &key, &ca) {
        Ok(credentials) => {
            let info = credentials.info();
            let said = &info.identity;
            println!(
                "Node certificate: {} in {}, {} to {}, {}",
                said.node,
                said.region,
                date(info.not_before),
                date(info.not_after),
                state(info, now)
            );
            if said.node != network.node || said.region != network.region {
                println!(
                    "  but this node is {} in {}: issue the right certificate",
                    network.node, network.region
                );
            }
        }
        Err(error) if network.nodes.is_empty() => println!("Node certificate: none ({error})"),
        Err(error) => {
            println!("Node certificate: NOT USABLE: {error}");
            return Ok(ExitCode::FAILURE);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Milliseconds to a tenth, as the launcher's region ping reports them.
fn tenths(time: Duration) -> f64 {
    (time.as_micros() as f64 / 100.0).round() / 10.0
}

struct PeerStatus {
    node: String,
    region: String,
    state: PeerState,
    rtt: Option<Duration>,
    skew_ms: Option<i64>,
    error: Option<String>,
}

/// The first ping of a peer pays the TLS handshake and is not counted; the next `count` run on the kept connection.
async fn probe(link: &TlsPeerLink, count: u32) -> Vec<PeerStatus> {
    let mut pings = tokio::task::JoinSet::new();
    for peer in link.health() {
        let link = link.clone();
        pings.spawn(async move {
            let mut times = Vec::new();
            let mut skew = None;
            let mut error = None;
            for attempt in 0..=count {
                match link.ping(&peer.node).await {
                    Ok(report) if attempt > 0 => {
                        times.push(report.rtt);
                        skew = Some(report.skew_ms);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        error = Some(e.to_string());
                        break;
                    }
                }
            }
            times.sort_unstable();
            let health = link.peer_health(&peer.node);
            PeerStatus {
                node: peer.node.to_string(),
                region: peer.region.to_string(),
                state: health.as_ref().map_or(PeerState::Unknown, |h| h.state),
                rtt: times.get(times.len() / 2).copied(),
                skew_ms: skew.or_else(|| health.and_then(|h| h.skew_ms)),
                error,
            }
        });
    }
    let mut out = Vec::new();
    while let Some(done) = pings.join_next().await {
        if let Ok(status) = done {
            out.push(status);
        }
    }
    // The order [[network.nodes]] lists them in
    let order: Vec<_> = link.health().iter().map(|h| h.node.to_string()).collect();
    out.sort_by_key(|s| order.iter().position(|n| *n == s.node));
    out
}

/// The node's own view, as its gateway is configured: the gateway.env `setup` wrote (`GATEWAY_NODE_*`), which needs no
/// checkout, so a front container can run it.
fn status(a: &StatusArgs, paths: Option<&Paths>) -> Outcome {
    let file = match (&a.env_file, paths) {
        (Some(file), _) => PathBuf::from(file),
        (None, Some(paths)) => paths.config.join("gateway.env"),
        (None, None) => return Err(bad("give --env-file <the gateway.env setup wrote>")),
    };
    let env = Env::load(Some(&file)).map_err(|e| bad(format!("{}: {e}", file.display())))?;
    let setting = |name: &str| {
        env.get(name).map(PathBuf::from).ok_or_else(|| {
            bad(format!(
                "{name} is not in {}: this server lists no peers",
                file.display()
            ))
        })
    };
    let credentials = NodeCredentials::load(
        &setting("GATEWAY_NODE_CERT")?,
        &setting("GATEWAY_NODE_KEY")?,
        &setting("GATEWAY_NODE_CA")?,
    )
    .map_err(|e| bad(format!("node certificate: {e}")))?;
    let peers: Vec<cluster::Peer> = serde_json::from_str(env.get("GATEWAY_NETWORK_NODES").unwrap_or("[]"))
        .map_err(|e| bad(format!("GATEWAY_NETWORK_NODES: {e}")))?;
    if peers.is_empty() {
        return Err(bad("this server lists no peers ([[network.nodes]] in host.toml)"));
    }
    let me = credentials.identity().clone();
    let link = TlsPeerLink::new(TlsLinkConfig::new(credentials, peers)).map_err(|e| bad(e.to_string()))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| bad(format!("cannot start the runtime: {e}")))?;
    let peers = runtime.block_on(probe(&link, a.count.clamp(1, 100)));
    let all_up = peers.iter().all(|p| p.state == PeerState::Up);
    if a.json {
        let peers: Vec<_> = peers
            .iter()
            .map(|p| {
                serde_json::json!({
                    "node": p.node,
                    "region": p.region,
                    "state": p.state,
                    "rttMs": p.rtt.map(tenths),
                    "skewMs": p.skew_ms,
                    "error": p.error,
                })
            })
            .collect();
        let report = serde_json::json!({ "node": me.node, "region": me.region, "peers": peers });
        println!("{report}");
    } else {
        println!("This node: {} in {}", me.node, me.region);
        for p in &peers {
            let state = serde_json::to_value(p.state)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            let rtt = p.rtt.map_or_else(|| "-".to_owned(), |t| format!("{} ms", tenths(t)));
            println!(
                "Peer:      {:<20} {:<14} {:<10} rtt {:>9}  skew {} ms{}",
                p.node,
                p.region,
                state,
                rtt,
                p.skew_ms.map_or_else(|| "-".to_owned(), |s| s.to_string()),
                p.error.as_ref().map_or_else(String::new, |e| format!("  ({e})"))
            );
        }
    }
    Ok(if all_up { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// Joins an existing server's accounts to the network directory (DN13): its accounts get this server as
/// their home (`dir.home_of`), existing invites and API keys keep working, and any username clash with
/// another server's account keeps the earlier-registered account's name while giving the later one a
/// `_2` / `_3` suffix (shortened to fit 16 characters) and a free rename (`needsUsername = true`).
fn join(a: &JoinArgs, paths: Option<&Paths>) -> Outcome {
    let paths = paths.ok_or_else(|| bad("run `reforged-host network join` in the server's folder"))?;
    let config = load_host_config(paths)?.ok_or_else(|| bad("no host.toml: run `reforged-host setup` first"))?;
    let network = config
        .network
        .as_ref()
        .ok_or_else(|| bad("this server has no [network] section in host.toml"))?;
    let credentials = NodeCredentials::load(
        &paths.tls.join(NODE_CERT_FILE),
        &paths.tls.join(NODE_KEY_FILE),
        &paths.tls.join(CLUSTER_CA_FILE),
    )
    .map_err(|e| bad(format!("node certificate: {e}")))?;
    let link = Arc::new(
        TlsPeerLink::new(TlsLinkConfig::new(credentials, network.nodes.clone())).map_err(|e| bad(e.to_string()))?,
    );
    let leader = network.effective_leader();
    let dir_config = DirectoryConfig::in_network(network.node.clone(), leader);
    let db_config = node_database_config(&config, paths)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| bad(format!("cannot start the runtime: {e}")))?;
    let (outcome, head) = runtime.block_on(async {
        let db = store::Database::open(
            &db_config,
            &store::DatabaseOptions::default(),
            &network_plugin::directory::SCHEMA,
        )
        .map_err(|e| bad(format!("database: {e}")))?;
        let dir = Directory::new(db.clone(), dir_config, link)
            .await
            .map_err(|e| bad(format!("directory: {e}")))?;
        let outcome = dir
            .join_host_database(store::Actor::Host(crate::commands::history::ACTOR.into()), &db)
            .await
            .map_err(|e| bad(format!("join: {e}")))?;
        let head = dir.head().await.unwrap_or(0);
        Ok::<_, HostError>((outcome, head))
    })?;
    if a.json {
        let clashes: Vec<_> = outcome
            .clashed
            .iter()
            .map(|c| {
                serde_json::json!({
                    "account": c.account,
                    "home": c.home,
                    "previousUsername": c.old_username.as_str(),
                    "assignedUsername": c.new_username.as_str(),
                })
            })
            .collect();
        let report = serde_json::json!({
            "node": network.node,
            "region": network.region,
            "applied": outcome.entries.len(),
            "head": head,
            "clashes": clashes,
        });
        println!("{report}");
    } else {
        println!(
            "Joined {} in {} to the directory (head {}, {} change(s) applied).",
            network.node,
            network.region,
            head,
            outcome.entries.len()
        );
        if outcome.clashed.is_empty() {
            println!("No username clashes.");
        } else {
            println!("Username clashes resolved ({}):", outcome.clashed.len());
            for clash in &outcome.clashed {
                println!(
                    "  {} ({}): {} -> {} (free rename at next login)",
                    clash.account,
                    clash.home,
                    clash.old_username.as_str(),
                    clash.new_username.as_str()
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A node's files and its gateway.env in a folder, listing `peers` (JSON) as its peers.
    fn gateway_env(dir: &Path, peers: &str) -> String {
        let ca = ClusterCa::create(30).unwrap();
        let issued = ca
            .issue(&NodeCertRequest {
                node: "oregon-1".parse().unwrap(),
                region: "oregon".parse().unwrap(),
                hosts: vec!["127.0.0.1".into()],
                days: 30,
            })
            .unwrap();
        for (name, text) in [
            (NODE_CERT_FILE, &issued.cert_pem),
            (NODE_KEY_FILE, &issued.key_pem),
            (CLUSTER_CA_FILE, &issued.ca_cert_pem),
        ] {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let file = dir.join("gateway.env");
        // Forward slashes: a Windows path's backslashes are not a dotenv value
        let path = |name: &str| dir.join(name).display().to_string().replace('\\', "/");
        let text = format!(
            "GATEWAY_NODE_CERT={}\nGATEWAY_NODE_KEY={}\nGATEWAY_NODE_CA={}\nGATEWAY_NETWORK_NODES='{peers}'\n",
            path(NODE_CERT_FILE),
            path(NODE_KEY_FILE),
            path(CLUSTER_CA_FILE),
        );
        std::fs::write(&file, text).unwrap();
        file.display().to_string()
    }

    fn status_of(env_file: String) -> Outcome {
        status(
            &StatusArgs {
                env_file: Some(env_file),
                count: 1,
                json: true,
            },
            None,
        )
    }

    #[test]
    fn status_needs_a_server_with_node_links() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gateway.env");
        std::fs::write(&file, "GATEWAY_TLS=1\n").unwrap();
        let error = status_of(file.display().to_string()).unwrap_err().to_string();
        assert!(error.contains("GATEWAY_NODE_CERT"), "{error}");
        assert!(
            status(
                &StatusArgs {
                    env_file: None,
                    count: 1,
                    json: false
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn status_needs_peers_and_fails_when_one_is_down() {
        let dir = tempfile::tempdir().unwrap();
        let error = status_of(gateway_env(dir.path(), "[]")).unwrap_err().to_string();
        assert!(error.contains("no peers"), "{error}");

        // Nothing listens on port 1: the peer is down, which is a failing exit and not an error
        let peer = r#"[{"id":"uk-1","region":"uk","host":"127.0.0.1","port":1}]"#;
        let outcome = status_of(gateway_env(dir.path(), peer)).unwrap();
        assert_eq!(format!("{outcome:?}"), format!("{:?}", ExitCode::FAILURE));
    }
}
