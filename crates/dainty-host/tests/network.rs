#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

//! `dainty-host network` (DN3): the CA, a node's certificate, and what stays off a node.

use clap::Parser as _;
use dainty_host::cli::Cli;
use dainty_host::commands::network;

fn run(args: &[&str]) -> Result<(), String> {
    let cli =
        Cli::try_parse_from(std::iter::once("dainty-host").chain(args.iter().copied())).map_err(|e| e.to_string())?;
    let dainty_host::cli::Command::Network(args) = &cli.command else {
        return Err("not the network command".into());
    };
    network::run(args, None).map(|_| ()).map_err(|e| e.to_string())
}

#[test]
fn the_owner_makes_a_ca_and_issues_a_nodes_files_from_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    let (ca, uk) = (path("ca.pem"), path("uk-1"));
    run(&["network", "ca-init", "--out", &ca]).unwrap();
    assert!(std::fs::read_to_string(&ca).unwrap().contains("PRIVATE KEY"));
    // Not over a CA that is there
    assert!(
        run(&["network", "ca-init", "--out", &ca])
            .unwrap_err()
            .contains("--force")
    );

    run(&[
        "network",
        "issue-node-cert",
        "--ca",
        &ca,
        "--node",
        "uk-1",
        "--region",
        "uk",
        "--host",
        "127.0.0.1",
        "--host",
        "uk.example.org",
        "--out",
        &uk,
        "--days",
        "90",
    ])
    .unwrap();
    let read = |name: &str| std::fs::read_to_string(dir.path().join("uk-1").join(name)).unwrap();
    // The node holds its own certificate and key and the CA's certificate: never the CA's key
    assert!(read("node-key.pem").contains("PRIVATE KEY"));
    assert!(!read("node-cert.pem").contains("PRIVATE KEY") && !read("cluster-ca-cert.pem").contains("PRIVATE KEY"));
    let creds = network_plugin::cluster::NodeCredentials::load(
        &dir.path().join("uk-1/node-cert.pem"),
        &dir.path().join("uk-1/node-key.pem"),
        &dir.path().join("uk-1/cluster-ca-cert.pem"),
    )
    .unwrap();
    assert_eq!(creds.identity().node.as_str(), "uk-1");
    assert_eq!(creds.info().hosts, ["127.0.0.1", "uk.example.org"]);
    // The CA keeps a list of what it issued
    let index = std::fs::read_to_string(dir.path().join("ca.issued.json")).unwrap();
    assert!(index.contains("uk-1") && !index.contains("PRIVATE"));
    run(&["network", "list", "--ca", &ca]).unwrap();

    // Not over a node's files, and not from a node's copy of the CA
    let again = [
        "network",
        "issue-node-cert",
        "--ca",
        &ca,
        "--node",
        "uk-1",
        "--region",
        "uk",
        "--host",
        "a.example",
        "--out",
        &uk,
    ];
    assert!(run(&again).unwrap_err().contains("--force"));
    let mut forced = again.to_vec();
    forced.push("--force");
    run(&forced).unwrap();
    let copy = path("uk-1/cluster-ca-cert.pem");
    let error = run(&[
        "network",
        "issue-node-cert",
        "--ca",
        &copy,
        "--node",
        "de-1",
        "--region",
        "de",
        "--host",
        "a.example",
    ])
    .unwrap_err();
    assert!(error.contains("no private key"), "{error}");
}

#[test]
fn bad_names_and_addresses_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("ca.pem").to_string_lossy().into_owned();
    let out = dir.path().join("n").to_string_lossy().into_owned();
    run(&["network", "ca-init", "--out", &ca]).unwrap();
    let issue = |node: &str, region: &str, host: &str, days: &str| {
        run(&[
            "network",
            "issue-node-cert",
            "--ca",
            &ca,
            "--node",
            node,
            "--region",
            region,
            "--host",
            host,
            "--days",
            days,
            "--out",
            &out,
        ])
    };
    assert!(issue("UK", "uk", "a.example", "30").unwrap_err().contains("--node"));
    assert!(
        issue("uk-1", "U K", "a.example", "30")
            .unwrap_err()
            .contains("--region")
    );
    assert!(issue("uk-1", "uk", "not a host", "30").is_err());
    assert!(issue("uk-1", "uk", "a.example", "0").is_err());
    assert!(issue("uk-1", "uk", "a.example", "soon").unwrap_err().contains("--days"));
    assert!(!dir.path().join("n").exists(), "nothing written for a refused request");
    assert!(run(&["network", "issue-node-cert", "--node", "a"]).is_err());
}
