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
//!
//! No checkout is needed except by `list` without `--ca`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use network_plugin::cluster::{self, ClusterCa, DEFAULT_NODE_DAYS, NodeCertInfo, NodeCertRequest, NodeCredentials};

use crate::Outcome;
use crate::cli::{CaInitArgs, IssueNodeCertArgs, ListArgs, NetworkArgs, NetworkCommand};
use crate::error::HostError;
use crate::hostconfig::{CLUSTER_CA_FILE, NODE_CERT_FILE, NODE_KEY_FILE, load_host_config};
use crate::paths::Paths;

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

pub fn run(args: &NetworkArgs, paths: Option<&Paths>) -> Outcome {
    match &args.command {
        NetworkCommand::CaInit(a) => ca_init(a),
        NetworkCommand::IssueNodeCert(a) => issue_node_cert(a),
        NetworkCommand::List(a) => list(a, paths),
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
