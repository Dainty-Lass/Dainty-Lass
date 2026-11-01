//! `setup` and `multipliers` (MIGRATION_PLAN.md RH3): the Node kit's tests for them (`kit.test.ts`,
//! `ports.test.ts`, `performance.test.ts`, `linux.test.ts`), the same host.json as the Node kit for
//! the same options (`tests/fixtures/node/rh3/rh3.json`, made by its `gen.ts` with the Node kit's own
//! code), and a fresh private and a fresh public setup into a temp folder with this PC's tools mocked.

#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clap::Parser;
use serde_json::Value;

use host::cli::{Cli, Command, MultipliersArgs, SetupArgs};
use host::commands::setup::firewall::firewall_script;
use host::commands::setup::options::{
    host_config_from_args, linux_from_args, performance_from_args, performance_summary, public_port_from_args,
};
use host::commands::setup::paradox::Layout;
use host::commands::setup::{SetupEnv, run_with};
use host::error::HostError;
use host::hostconfig::{
    DEFAULT_PERFORMANCE, HostConfig, LinuxOptions, Multipliers, PerformanceOptions, Platform, ServerMode,
    host_config_text, load_host_config, read_env_file, read_host_config, write_host_config,
};
use host::paths::{Paths, game_exe_relative};
use host::preset::PresetApplied;

const MONGOD: &str = "C:\\Program Files\\MongoDB\\Server\\8.0\\bin\\mongod.exe";

/// This PC, mocked: what Tailscale, PATH and mongod say, and what setup asked of the tools.
struct Mock {
    platform: Platform,
    tailscale: Option<String>,
    mongod: Option<String>,
    on_path: HashMap<String, String>,
    installs: RefCell<usize>,
    certificates: RefCell<Vec<Vec<String>>>,
}

impl Mock {
    fn new() -> Self {
        Self {
            platform: Platform::Windows,
            tailscale: Some("100.101.102.103".into()),
            mongod: Some(MONGOD.into()),
            on_path: HashMap::new(),
            installs: RefCell::new(0),
            certificates: RefCell::new(Vec::new()),
        }
    }
}

impl SetupEnv for Mock {
    fn platform(&self) -> Platform {
        self.platform
    }
    fn is_root(&self) -> Result<bool, HostError> {
        Ok(false)
    }
    fn tailscale_ipv4(&self) -> Result<Option<String>, HostError> {
        Ok(self.tailscale.clone())
    }
    fn find_mongod(&self) -> Result<Option<String>, HostError> {
        Ok(self.mongod.clone())
    }
    fn find_on_path(&self, name: &str) -> Result<Option<String>, HostError> {
        Ok(self.on_path.get(name).cloned())
    }
    fn source_commit(&self, _repo: &Path) -> Result<String, HostError> {
        Ok("abc123def456".into())
    }
    fn apply_preset(&self, _paths: &Paths) -> Result<Option<PresetApplied>, HostError> {
        Ok(None)
    }
    fn install_services(&self, layout: &Layout, _log_file: &Path, out: &mut dyn FnMut(&str)) -> Result<(), HostError> {
        *self.installs.borrow_mut() += 1;
        for (_, label) in host::commands::setup::install::services(layout) {
            out(&format!("- {label}: npm ci (mocked)"));
        }
        Ok(())
    }
    fn make_certificate(
        &self,
        _layout: &Layout,
        hosts: &[String],
        tls: &Path,
        _log_file: &Path,
    ) -> Result<(), HostError> {
        // As the gateway's make-cert does: a self-signed certificate for the server's names.
        self.certificates.borrow_mut().push(hosts.to_vec());
        let cert = rcgen::generate_simple_self_signed(hosts.to_vec()).unwrap();
        std::fs::create_dir_all(tls).unwrap();
        std::fs::write(tls.join("gateway-cert.pem"), cert.cert.pem()).unwrap();
        std::fs::write(tls.join("gateway-key.pem"), cert.signing_key.serialize_pem()).unwrap();
        Ok(())
    }
    fn gateway_program(&self) -> String {
        "C:\\rr\\gateway.exe".into()
    }
}

fn setup_args(args: &[&str]) -> SetupArgs {
    let mut argv = vec!["reforged-host", "setup"];
    argv.extend_from_slice(args);
    match Cli::try_parse_from(argv).unwrap().command {
        Command::Setup(s) => s,
        _ => unreachable!(),
    }
}

fn from_args(args: &[&str], existing: Option<&HostConfig>, env: &Mock) -> Result<HostConfig, String> {
    host_config_from_args(&setup_args(args), existing, env).map_err(|e| e.to_string())
}

/// Written and read back, as setup does.
fn round_trip(c: &HostConfig) -> HostConfig {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("host.json");
    write_host_config(&file, c).unwrap();
    read_host_config(&file).unwrap().unwrap()
}

// ------------------------------------------------------------------ the same as the Node kit

fn fixture() -> Value {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/node/rh3/rh3.json");
    serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
}

fn mock_for(detect: &Value) -> Mock {
    Mock {
        platform: if detect["platform"] == "linux" {
            Platform::Linux
        } else {
            Platform::Windows
        },
        tailscale: detect["tailscale"].as_str().map(str::to_owned),
        mongod: detect["mongod"].as_str().map(str::to_owned),
        ..Mock::new()
    }
}

/// The configs setup made for the fixture's cases, by name.
fn fixture_configs() -> HashMap<String, HostConfig> {
    let mut configs: HashMap<String, HostConfig> = HashMap::new();
    for case in fixture()["setups"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let args: Vec<&str> = case["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let before = case["before"].as_str().map(|b| &configs[b]);
        let env = mock_for(&case["detect"]);
        let made = from_args(&args, before, &env)
            .and_then(|c| host_config_text(&c).map(|t| (c, t)).map_err(|e| e.to_string()));
        match (made, case.get("error").and_then(Value::as_str)) {
            (Err(e), Some(expected)) => assert_eq!(e, expected, "{name}"),
            (Err(e), None) => panic!("{name}: the Node kit takes {args:?}, this one says {e}"),
            (Ok(_), Some(expected)) => panic!("{name}: the Node kit refuses {args:?}: {expected}"),
            (Ok((c, text)), None) => {
                assert_eq!(text, case["hostJson"].as_str().unwrap(), "{name}: host.json");
                let read = round_trip(&c);
                assert_eq!(performance_summary(&read), case["summary"].as_str().unwrap(), "{name}");
                configs.insert(name.to_owned(), read);
            }
        }
    }
    configs
}

#[test]
fn writes_the_host_json_the_node_kit_writes_and_refuses_what_it_refuses() {
    assert_eq!(fixture_configs().len(), 23);
}

#[test]
fn writes_the_node_kits_firewall_rules() {
    let configs = fixture_configs();
    for case in fixture()["firewall"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        assert_eq!(
            firewall_script(&configs[name], "C:\\node\\node.exe"),
            case["script"].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn makes_the_node_kits_invites() {
    let configs = fixture_configs();
    let fixture = fixture();
    for case in fixture["invites"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let link =
            host::invite::invite_for(&configs[name], case["code"].as_str().unwrap(), case["fp"].as_str()).unwrap();
        assert_eq!(link, case["link"].as_str().unwrap(), "{name}");
    }
    let no_fp = host::invite::format_invite_parts(&host::invite::InviteParts {
        mode: ServerMode::Public,
        host: "x.example".into(),
        port: 443,
        code: "ABCD".into(),
        name: "x".into(),
        fp: None,
        share: None,
    })
    .unwrap_err();
    assert_eq!(no_fp.to_string(), fixture["noFingerprint"].as_str().unwrap());
}

// ------------------------------------------------------------------ kit.test.ts

#[test]
fn builds_host_json_from_the_arguments_the_old_host_json_and_what_it_finds_on_this_pc() {
    let env = Mock::new();
    let c = from_args(&["--mode", "private", "--name", "Friday"], None, &env).unwrap();
    assert_eq!(c.host, "100.101.102.103", "private: this PC's Tailscale address");
    assert_eq!(c.game_address, "100.101.102.103");
    assert_eq!(c.mongo.uri, host::hostconfig::DEFAULT_MONGO_URI);
    assert!(c.mongo.manage);
    assert_eq!(c.mongo.mongod.as_deref(), Some(MONGOD));
    let public = from_args(&["--mode", "public", "--host", "203.0.113.7"], Some(&c), &env).unwrap();
    assert_eq!(public.game_address, "203.0.113.7");
    assert_eq!(public.name, "Friday", "kept from the old host.json");
    assert!(
        from_args(&["--mode", "public", "--host", "hunts.example.net"], None, &env)
            .unwrap_err()
            .contains("game-address")
    );
    let dns = from_args(
        &[
            "--mode",
            "public",
            "--host",
            "hunts.example.net",
            "--game-address",
            "203.0.113.7",
        ],
        None,
        &env,
    );
    assert_eq!(dns.unwrap().game_address, "203.0.113.7");
    assert!(from_args(&[], None, &env).unwrap_err().contains("--mode"));
    let offline = Mock {
        tailscale: None,
        ..Mock::new()
    };
    assert!(
        from_args(&["--mode", "private"], None, &offline)
            .unwrap_err()
            .contains("Tailscale")
    );
    let own = from_args(
        &[
            "--mode",
            "private",
            "--mongo-uri",
            "mongodb://db.local:27017/?replicaSet=rs0",
        ],
        None,
        &env,
    );
    assert!(!own.unwrap().mongo.manage);
}

#[test]
fn takes_test_multipliers_on_a_private_server_only_keeps_them_and_off_clears_them() {
    let env = Mock::new();
    let c = from_args(
        &["--mode", "private", "--xp-multiplier", "5", "--reward-multiplier", "x3"],
        None,
        &env,
    )
    .unwrap();
    let five_three = Some(Multipliers { xp: 5.0, rewards: 3.0 });
    assert_eq!(c.multipliers, five_three);
    assert_eq!(from_args(&[], Some(&c), &env).unwrap().multipliers, five_three);
    assert_eq!(from_args(&["--off"], Some(&c), &env).unwrap().multipliers, None);
    let ones = from_args(&["--xp-multiplier", "1", "--reward-multiplier", "1"], Some(&c), &env);
    assert_eq!(ones.unwrap().multipliers, None);
    let public = from_args(&["--mode", "public", "--host", "203.0.113.7"], Some(&c), &env).unwrap();
    assert_eq!(public.multipliers, None, "not carried into a public server");
    let refused = from_args(
        &["--mode", "public", "--host", "203.0.113.7", "--xp-multiplier", "5"],
        None,
        &env,
    );
    assert!(refused.unwrap_err().contains("private"));
    let big = from_args(&["--mode", "private", "--xp-multiplier", "500"], None, &env);
    assert!(big.unwrap_err().contains("1 to 100"));
}

#[test]
fn follows_a_new_tailscale_address_and_keeps_the_old_one_only_when_tailscale_is_not_running() {
    let at = |ip: Option<&str>| Mock {
        tailscale: ip.map(str::to_owned),
        ..Mock::new()
    };
    let old = from_args(
        &["--mode", "private", "--name", "Freeza"],
        None,
        &at(Some("100.117.171.17")),
    )
    .unwrap();
    let moved = from_args(&[], Some(&old), &at(Some("100.88.12.40"))).unwrap();
    assert_eq!(moved.host, "100.88.12.40", "another Tailscale account, another address");
    assert_eq!(moved.game_address, "100.88.12.40", "the game servers move with it");
    assert_eq!(moved.name, "Freeza");
    assert_eq!(from_args(&[], Some(&old), &at(None)).unwrap().host, "100.117.171.17");
    let wins = from_args(&["--host", "100.64.0.9"], Some(&old), &Mock::new()).unwrap();
    assert_eq!(wins.host, "100.64.0.9", "--host still wins");
}

#[test]
fn wants_a_full_path_for_game() {
    let env = Mock::new();
    let posix = from_args(
        &["--mode", "private", "--game", "/games/Dauntless 1.14.7/Dauntless"],
        None,
        &env,
    )
    .unwrap();
    if cfg!(windows) {
        assert!(
            posix
                .game_dir
                .unwrap()
                .ends_with("\\games\\Dauntless 1.14.7\\Dauntless"),
            "path.resolve: this drive"
        );
    } else {
        assert_eq!(posix.game_dir.as_deref(), Some("/games/Dauntless 1.14.7/Dauntless"));
    }
    let windows = from_args(
        &["--mode", "private", "--game", "D:\\Games\\Dauntless 1.14.7\\Dauntless"],
        None,
        &env,
    );
    assert!(windows.unwrap().game_dir.is_some());
    for relative in ["Games\\Dauntless", "^D:\\Games\\Dauntless^ 1.14.7\\Dauntless^"] {
        let error = from_args(&["--mode", "private", "--game", relative], None, &env).unwrap_err();
        assert!(error.contains("full path"), "{error}");
    }
}

#[test]
fn writes_firewall_rules_tailscale_only_in_private_mode_443_and_a_closed_allowlist_rule_in_public_mode() {
    let env = Mock::new();
    let base = from_args(&["--mode", "private", "--game", "D:\\Games\\Dauntless"], None, &env).unwrap();
    let private = firewall_script(&base, "C:\\node\\node.exe");
    assert!(private.contains("100.64.0.0/10"));
    assert!(private.contains("'61000'"));
    assert!(!private.contains("'443'"));
    // path.resolve: unchanged on Windows, under the working folder elsewhere.
    let exe = Path::new(base.game_dir.as_deref().unwrap()).join(game_exe_relative());
    let public = HostConfig {
        mode: ServerMode::Public,
        host: "203.0.113.7".into(),
        game_address: "203.0.113.7".into(),
        share_url: None,
        ..base
    };
    let public = firewall_script(&public, "C:\\node\\node.exe");
    assert!(public.contains("'443'"));
    assert!(public.contains("ReforgedRevival-GamePorts-Allowlist"));
    assert!(public.contains("'False'"));
    assert!(public.contains(&format!("'{}'", exe.display())));
}

// ------------------------------------------------------------------ ports.test.ts

fn moved() -> HostConfig {
    let public = from_args(
        &["--mode", "public", "--host", "203.0.113.7", "--public-port", "4443"],
        None,
        &Mock::new(),
    );
    public.unwrap()
}

#[test]
fn the_public_port_comes_from_public_port_is_kept_when_not_given_and_443_goes_back() {
    let moved = moved();
    let port = |args: &[&str], mode, existing: Option<&HostConfig>| {
        public_port_from_args(&setup_args(args), mode, existing).map_err(|e| e.to_string())
    };
    assert_eq!(
        port(&["--public-port", "4443"], ServerMode::Public, None),
        Ok(Some(4443.0))
    );
    assert_eq!(port(&[], ServerMode::Public, Some(&moved)), Ok(Some(4443.0)));
    assert_eq!(
        port(&["--public-port", "443"], ServerMode::Public, Some(&moved)),
        Ok(None)
    );
    assert_eq!(port(&[], ServerMode::Private, Some(&moved)), Ok(None));
    assert!(
        port(&["--public-port", "4443"], ServerMode::Private, None)
            .unwrap_err()
            .contains("public servers only")
    );
    assert!(
        port(&["--public-port", "web"], ServerMode::Public, None)
            .unwrap_err()
            .contains("port number")
    );
}

#[test]
fn the_moved_port_reaches_the_invite_the_launcher_reads() {
    let link = host::invite::invite_for(&moved(), "ABCD-EFGH-IJKL", Some(&"a".repeat(64))).unwrap();
    let parsed = protocol::invite::parse_invite(&link).unwrap();
    assert_eq!(parsed.port, 4443);
}

#[test]
fn tells_how_to_find_the_program_on_a_port() {
    use host::doctor::port_owner_hint_for;
    assert!(port_owner_hint_for(443, Platform::Linux).contains("ss -tlnp 'sport = :443'"));
    assert!(port_owner_hint_for(443, Platform::Windows).contains("netstat -ano | findstr :443"));
}

// ------------------------------------------------------------------ performance.test.ts

#[test]
fn performance_defaults_keeps_what_host_json_had_and_takes_each_option() {
    let perf = |args: &[&str], existing: Option<PerformanceOptions>| {
        performance_from_args(&setup_args(args), existing).map_err(|e| e.to_string())
    };
    assert_eq!(perf(&[], None), Ok(DEFAULT_PERFORMANCE));
    let set = perf(
        &[
            "--server-fps",
            "60",
            "--idle-fps",
            "0",
            "--training-at-boot",
            "--verbose-game-logs",
            "--no-memory-guard",
            "--min-free-ram",
            "512",
            "--hunt-ram",
            "2048",
            "--training-idle-minutes",
            "15",
        ],
        None,
    )
    .unwrap();
    assert_eq!(
        set,
        PerformanceOptions {
            server_fps: 60,
            idle_fps: 0,
            training_on_demand: false,
            training_idle_minutes: 15,
            verbose_game_logs: true,
            memory_guard: false,
            min_free_memory_mb: 512,
            hunt_memory_mb: 2048
        }
    );
    assert_eq!(perf(&[], Some(set)), Ok(set), "kept on the next setup");
    assert_eq!(
        perf(&["--training-on-demand", "--quiet-game-logs"], Some(set)),
        Ok(PerformanceOptions {
            training_on_demand: true,
            verbose_game_logs: false,
            ..set
        })
    );
    for (args, name) in [
        (["--server-fps", "5"], "--server-fps"),
        (["--idle-fps", "fast"], "--idle-fps"),
        (["--hunt-ram", "100"], "--hunt-ram"),
        (["--training-idle-minutes", "soon"], "--training-idle-minutes"),
    ] {
        assert!(perf(&args, None).unwrap_err().contains(name));
    }
    assert_eq!(
        perf(&["--training-idle-minutes", "0"], None)
            .unwrap()
            .training_idle_minutes,
        0
    );
    assert!(perf(&["--memory-guard"], Some(set)).unwrap().memory_guard);
}

#[test]
fn performance_goes_into_host_json_with_setup() {
    let env = Mock::new();
    let c = from_args(
        &["--mode", "public", "--host", "203.0.113.7", "--server-fps", "45"],
        None,
        &env,
    )
    .unwrap();
    assert_eq!(c.performance.unwrap().server_fps, 45);
    assert_eq!(round_trip(&c).performance.unwrap().server_fps, 45);
    assert_eq!(
        from_args(&[], Some(&c), &env).unwrap().performance.unwrap().server_fps,
        45
    );
    let public = from_args(&["--mode", "public", "--host", "203.0.113.7"], None, &env).unwrap();
    assert!(performance_summary(&public).contains(
        "30 fps, 10 fps after 15 s without players; Training Grounds on demand, stopped after 5 min without players; quiet game logs; at most 3 hunts at once (UDP 8770-8777), a new hunt only with 2560 MB free"
    ));
    assert_eq!(
        from_args(&["--max-hunts", "8"], Some(&public), &env)
            .unwrap()
            .max_hunt_servers,
        8
    );
}

// ------------------------------------------------------------------ linux.test.ts

#[test]
fn linux_reads_wine_proton_no_xvfb_and_firewall_interface_and_keeps_the_old_ones_otherwise() {
    let find = |n: &str| Ok((n == "wine64").then(|| "/usr/bin/wine64".to_owned()));
    let lx = |args: &[&str], existing: Option<&LinuxOptions>| {
        linux_from_args(&setup_args(args), existing, &find).map_err(|e| e.to_string())
    };
    let none = LinuxOptions {
        wine: None,
        proton: None,
        xvfb: true,
        firewall_interface: None,
    };
    assert_eq!(lx(&[], None), Ok(none));
    let set = lx(&["--wine", "wine64", "--no-xvfb", "--firewall-interface", "ens3"], None).unwrap();
    assert_eq!(
        set,
        LinuxOptions {
            wine: Some("/usr/bin/wine64".into()),
            proton: None,
            xvfb: false,
            firewall_interface: Some("ens3".into())
        }
    );
    assert_eq!(lx(&[], Some(&set)), Ok(set.clone()), "kept");
    assert_eq!(
        lx(
            &[
                "--xvfb",
                "--no-firewall-interface",
                "--proton",
                "/opt/GE-Proton10/proton"
            ],
            Some(&set)
        ),
        Ok(LinuxOptions {
            wine: Some("/usr/bin/wine64".into()),
            proton: Some("/opt/GE-Proton10/proton".into()),
            xvfb: true,
            firewall_interface: None
        })
    );
    assert!(lx(&["--wine", "nowine"], None).unwrap_err().contains("not found"));
    assert!(
        lx(&["--firewall-interface", "eth0; drop"], None)
            .unwrap_err()
            .contains("interface")
    );
}

#[test]
fn linux_options_are_in_host_json_on_linux_only() {
    let args = [
        "--mode",
        "public",
        "--host",
        "203.0.113.7",
        "--game",
        "/srv/dauntless/Dauntless",
    ];
    let linux = Mock {
        platform: Platform::Linux,
        tailscale: None,
        ..Mock::new()
    };
    assert_eq!(
        from_args(&args, None, &linux).unwrap().linux,
        Some(LinuxOptions {
            wine: None,
            proton: None,
            xvfb: true,
            firewall_interface: None
        })
    );
    assert_eq!(from_args(&args, None, &Mock::new()).unwrap().linux, None);
}

// ------------------------------------------------------------------ a fresh private and public setup

/// A checkout with what setup builds from (modules/rustic-isles and native/), and a game folder.
fn checkout() -> (tempfile::TempDir, Paths, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Reforged");
    let p = Paths::new(&root);
    let l = Layout::new(&p);
    for file in [
        l.backend_dir.join("package.json"),
        l.director_dir.join("package.json"),
        l.runtime_source.join("MysticParadox.vcxproj"),
        l.loader_dir.join("Cargo.toml"),
        l.exporter_dir.join("CatalogExporter.vcxproj"),
    ] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "{}").unwrap();
    }
    let game = dir.path().join("Games").join("Dauntless");
    let exe = game.join(game_exe_relative());
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(exe, "not really a game").unwrap();
    (dir, p, game)
}

fn setup(p: &Paths, args: &[&str], env: &Mock) -> (Result<std::process::ExitCode, String>, Vec<String>) {
    let mut lines = Vec::new();
    let r = run_with(p, &setup_args(args), env, &mut |l| lines.push(l.to_owned())).map_err(|e| e.to_string());
    (r, lines)
}

fn read(file: &Path) -> String {
    std::fs::read_to_string(file).unwrap()
}

#[test]
fn a_fresh_private_setup_then_again() {
    let (_dir, p, game) = checkout();
    let env = Mock::new();
    let game_arg = game.to_string_lossy().into_owned();
    let (r, lines) = setup(
        &p,
        &["--mode", "private", "--name", "Friday Hunts", "--game", &game_arg],
        &env,
    );
    assert!(r.is_ok(), "{r:?}\n{}", lines.join("\n"));
    let text = lines.join("\n");
    assert!(
        text.contains("\nPrivate server \"Friday Hunts\" at 100.101.102.103 (game servers on 100.101.102.103)"),
        "{text}"
    );
    assert!(
        text.contains("\n1. Mystic Paradox (modules/rustic-isles and native/ in this project)\n- in place"),
        "{text}"
    );
    assert!(text.contains("- backend: npm ci (mocked)"));
    assert!(text.contains("- secrets: made (kept from now on) in "));
    assert!(text.contains("- env files: backend.env, director.env, content.env, gateway.env, admin.env in "));
    assert!(!text.contains("! No "), "the game folder is there: {text}");
    assert!(text.ends_with("  reforged-host doctor shows what is still missing."));
    assert_eq!(*env.installs.borrow(), 1);

    let config = read_host_config(&p.host_toml).unwrap().unwrap();
    assert_eq!(config.mode, ServerMode::Private);
    assert_eq!(config.game_dir.as_deref(), Some(game_arg.as_str()));
    assert!(!p.host_json.exists(), "host.toml only");
    let toml = read(&p.host_toml);
    assert!(toml.starts_with("# This server's settings (reforged-host)."), "{toml}");
    assert!(
        toml.contains("\nversion = 1\n") && toml.contains("\nmode = \"private\"\n"),
        "{toml}"
    );
    assert!(
        toml.contains("\nhost = \"100.101.102.103\"\n") && toml.contains("\n[mongo]\n"),
        "{toml}"
    );
    let backend = read_env_file(&p.config.join("backend.env")).unwrap();
    let secrets = read_env_file(&p.config.join("secrets.env")).unwrap();
    assert_eq!(backend.get("REFORGED_ADMIN_KEY"), secrets.get("REFORGED_ADMIN_KEY"));
    assert!(!p.config.join("allowlist.env").exists(), "no allowlist in private mode");
    let firewall = read(&p.config.join("firewall.ps1"));
    assert!(firewall.contains("'C:\\rr\\gateway.exe' '100.64.0.0/10'"), "{firewall}");
    assert_eq!(read(&p.news_file), "{\n  \"items\": []\n}\n");
    for dir in [
        &p.runtime_publish,
        &p.branding,
        &p.gameserver_logs,
        &p.mongo_data,
        &Layout::new(&p).raw_game_data,
    ] {
        assert!(dir.is_dir(), "{}", dir.display());
    }
    assert!(
        env.certificates.borrow().is_empty(),
        "no certificate for a private server"
    );

    // Again, with --skip-install: the secrets are kept, the options too.
    let before = read(&p.config.join("secrets.env"));
    let (r, lines) = setup(&p, &["--skip-install"], &env);
    assert!(r.is_ok(), "{r:?}");
    assert!(lines.contains(&"\n2. Skipped installing and building (--skip-install)".to_owned()));
    assert!(lines.iter().any(|l| l.starts_with("- secrets: kept in ")));
    assert_eq!(read(&p.config.join("secrets.env")), before);
    assert_eq!(read_host_config(&p.host_toml).unwrap().unwrap().name, "Friday Hunts");
}

#[test]
fn setup_moves_a_node_kits_host_json_over_to_host_toml() {
    let (_dir, p, game) = checkout();
    let env = Mock::new();
    let game_arg = game.to_string_lossy().into_owned();
    setup(
        &p,
        &["--mode", "private", "--name", "Friday Hunts", "--game", &game_arg],
        &env,
    )
    .0
    .unwrap();
    // As a Node kit leaves it: host.json only.
    let config = read_host_config(&p.host_toml).unwrap().unwrap();
    write_host_config(&p.host_json, &config).unwrap();
    let json = read(&p.host_json);
    std::fs::remove_file(&p.host_toml).unwrap();
    assert_eq!(
        load_host_config(&p).unwrap().unwrap().name,
        "Friday Hunts",
        "read while there is no host.toml"
    );

    let (r, lines) = setup(&p, &["--skip-install"], &env);
    assert!(r.is_ok(), "{r:?}");
    assert!(
        lines.contains(&"(host.json is now host.toml; the old file is kept as host.json.bak)".to_owned()),
        "{lines:?}"
    );
    assert!(!p.host_json.exists());
    assert_eq!(read(&p.config.join("host.json.bak")), json, "kept as it was");
    let moved = read_host_config(&p.host_toml).unwrap().unwrap();
    assert_eq!(moved.name, "Friday Hunts");
    assert_eq!(moved.game_dir.as_deref(), Some(game_arg.as_str()));

    // Once moved, setup says nothing more about it.
    let (_, lines) = setup(&p, &["--skip-install"], &env);
    assert!(!lines.iter().any(|l| l.contains("host.json.bak")));
}

#[test]
fn setup_keeps_the_hosts_own_comments_and_edits_in_host_toml() {
    let (_dir, p, _game) = checkout();
    let env = Mock::new();
    setup(
        &p,
        &["--mode", "private", "--name", "Friday Hunts", "--skip-install"],
        &env,
    )
    .0
    .unwrap();
    let edited = read(&p.host_toml).replace(
        "max_hunt_servers = 3\n",
        "# two hunts are plenty on this PC\nmax_hunt_servers = 2 # was 3\n",
    );
    assert_ne!(edited, read(&p.host_toml));
    std::fs::write(&p.host_toml, &edited).unwrap();
    setup(&p, &["--name", "Saturday Hunts", "--skip-install"], &env)
        .0
        .unwrap();
    assert_eq!(
        read(&p.host_toml),
        edited.replace("name = \"Friday Hunts\"", "name = \"Saturday Hunts\"")
    );
    assert_eq!(load_host_config(&p).unwrap().unwrap().max_hunt_servers, 2);
}

// DF7: [database] is chosen in host.toml; setup keeps it and writes it into admin.env.
#[test]
fn setup_keeps_the_database_the_host_chose_in_host_toml() {
    let (_dir, p, _game) = checkout();
    let env = Mock::new();
    setup(&p, &["--mode", "private", "--skip-install"], &env).0.unwrap();
    let admin = read_env_file(&p.config.join("admin.env")).unwrap();
    assert_eq!(admin.get("REFORGED_DB_ENGINE"), Some("sqlite"));
    let edited = read(&p.host_toml) + "\n# Postgres in Docker\n[database]\nengine = \"postgres\" # was sqlite\n";
    std::fs::write(&p.host_toml, &edited).unwrap();
    setup(&p, &["--skip-install"], &env).0.unwrap();
    assert_eq!(read(&p.host_toml), edited, "kept as written");
    let admin = read_env_file(&p.config.join("admin.env")).unwrap();
    let secrets = read_env_file(&p.config.join("secrets.env")).unwrap();
    assert_eq!(admin.get("REFORGED_DB_ENGINE"), Some("postgres"));
    assert_eq!(admin.get("REFORGED_DB_HOST"), Some("127.0.0.1"));
    assert_eq!(admin.get("REFORGED_DB_PASSWORD"), secrets.get("REFORGED_DB_PASSWORD"));
    assert_eq!(secrets.get("REFORGED_DB_PASSWORD").map(str::len), Some(64));
}

#[test]
fn a_fresh_public_setup_makes_the_certificate_once() {
    let (_dir, p, _game) = checkout();
    let env = Mock {
        mongod: None,
        ..Mock::new()
    };
    let (r, lines) = setup(
        &p,
        &[
            "--mode",
            "public",
            "--host",
            "hunts.example.net",
            "--game-address",
            "203.0.113.7",
        ],
        &env,
    );
    assert!(r.is_ok(), "{r:?}\n{}", lines.join("\n"));
    assert_eq!(*env.certificates.borrow(), [["hunts.example.net", "203.0.113.7"]]);
    let fp = host::invite::certificate_fingerprint(&p.cert_file).unwrap();
    let text = lines.join("\n");
    assert!(
        text.contains(&format!("- certificate fingerprint (goes into every invite): {fp}")),
        "{text}"
    );
    if cfg!(feature = "paradox") {
        assert!(
            text.contains("\n! MongoDB was not found. Install MongoDB Community Server"),
            "{text}"
        );
    } else {
        assert!(
            !text.contains("MongoDB"),
            "SQLite is used by default without paradox: {text}"
        );
    }
    assert!(text.contains("\n! No game folder yet"), "{text}");
    assert!(p.config.join("allowlist.env").exists());
    assert!(
        read(&p.config.join("firewall.ps1"))
            .contains("'ReforgedRevival-Gateway' 'Dauntless Reforged Revival gateway (TCP 443)'")
    );
    let gateway = read_env_file(&p.config.join("gateway.env")).unwrap();
    assert_eq!(gateway.get("GATEWAY_PORT"), Some("443"));

    let (r, lines) = setup(&p, &["--skip-install"], &env);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(
        env.certificates.borrow().len(),
        1,
        "kept: a new one would change every invite"
    );
    assert!(lines.iter().any(|l| l.starts_with("- gateway certificate kept")));
    assert_eq!(host::invite::certificate_fingerprint(&p.cert_file).unwrap(), fp);
}

#[test]
fn setup_refuses_an_unquoted_value_and_an_incomplete_checkout() {
    let (_dir, p, _game) = checkout();
    let (r, _) = setup(
        &p,
        &["--mode", "private", "--game", "D:\\Games\\My", "Dauntless"],
        &Mock::new(),
    );
    assert!(
        r.unwrap_err()
            .starts_with("unexpected \"Dauntless\": put a value with spaces in quotes")
    );
    std::fs::remove_file(Layout::new(&p).loader_dir.join("Cargo.toml")).unwrap();
    let (r, _) = setup(&p, &["--mode", "private"], &Mock::new());
    assert!(
        r.unwrap_err()
            .starts_with("this checkout has no native/runtime-loader: run git submodule update")
    );
}

#[test]
fn setup_help_hides_paradox_rollback_flags_on_the_default_build_and_keeps_mongo_uri_for_migration() {
    let err = Cli::try_parse_from(["reforged-host", "setup", "--help"]).unwrap_err();
    let help = err.to_string();
    assert!(help.contains("--mongo-uri <uri>"), "{help}");
    assert!(help.contains("migrate pre-cutover player data from"), "{help}");
    for flag in ["--manage-mongo", "--no-manage-mongo", "--mongod", "--force-upstream"] {
        assert_eq!(
            help.contains(flag),
            cfg!(feature = "paradox"),
            "{flag} visibility in setup --help should match cfg!(feature = \"paradox\"):\n{help}"
        );
    }
    assert_eq!(help.contains("Download and patch"), cfg!(feature = "paradox"), "{help}");
}

// ------------------------------------------------------------------ multipliers

fn multipliers(p: &Paths, args: &[&str]) -> (Result<std::process::ExitCode, String>, Vec<String>) {
    let mut argv = vec!["reforged-host", "multipliers"];
    argv.extend_from_slice(args);
    let Command::Multipliers(m) = Cli::try_parse_from(argv).unwrap().command else {
        unreachable!()
    };
    let mut lines = Vec::new();
    let commit = |_: &Path| Ok("abc123def456".to_owned());
    let r = host::commands::multipliers::run_with(p, &m, Platform::Windows, &commit, &mut |l| lines.push(l.to_owned()));
    (r.map_err(|e| e.to_string()), lines)
}

#[test]
fn multipliers_change_only_the_multipliers_in_host_toml_and_the_backends_env() {
    let (_dir, p, _game) = checkout();
    let (r, lines) = multipliers(&p, &[]);
    assert_eq!(
        r.unwrap_err(),
        "not set up yet: run reforged-host setup first",
        "{lines:?}"
    );
    setup(&p, &["--mode", "private", "--skip-install"], &Mock::new())
        .0
        .unwrap();
    let before = read(&p.host_toml);

    let (r, lines) = multipliers(&p, &[]);
    assert!(r.is_ok());
    assert_eq!(lines, ["Test multipliers off."], "nothing asked: only says");

    let (r, lines) = multipliers(&p, &["--xp", "5", "--rewards", "x2.5"]);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(lines[0], "Test multipliers ON: XP x5, rewards x2.5.");
    assert!(lines[1].starts_with("Restart the server"));
    let after = read(&p.host_toml);
    // A new [multipliers] table at the end; everything else stays as it was.
    assert_eq!(after, format!("{before}\n[multipliers]\nxp = 5\nrewards = 2.5\n"));
    let backend = read_env_file(&p.config.join("backend.env")).unwrap();
    assert!(
        backend.iter().any(|(k, v)| k.contains("MULTIPLIER") && v == "5"),
        "{backend:?}"
    );

    let (_, lines) = multipliers(&p, &["--xp-multiplier", "500"]);
    assert!(lines.is_empty());
    assert_eq!(
        multipliers(&p, &["--xp-multiplier", "500"]).0.unwrap_err(),
        "--xp must be a number from 1 to 100 (got 500)"
    );
    multipliers(&p, &["--off"]).0.unwrap();
    assert_eq!(read(&p.host_toml), before, "--off: the file as it was");
}

#[test]
fn multipliers_args_default_is_nothing_asked() {
    assert!(!host::commands::multipliers::asked(&MultipliersArgs::default()));
    assert!(!host::commands::multipliers::asked(&MultipliersArgs {
        no_multipliers: true,
        ..Default::default()
    }));
}

// ------------------------------------------------------------------ editions: 1.4.4 is no longer served

#[test]
fn edition_is_gone_and_host_toml_may_only_say_reborn() {
    let env = Mock::new();
    let reborn = from_args(&["--mode", "private"], None, &env).unwrap();
    assert!(!host_config_text(&reborn).unwrap().contains("edition"));
    assert!(!reborn.to_toml().unwrap().contains(
        "
edition"
    ));
    let error = from_args(&["--mode", "private", "--edition", "legacy"], None, &env).unwrap_err();
    assert!(error.contains("--edition"), "{error}");
    let toml = reborn.to_toml().unwrap();
    let with = |edition: &str| {
        toml.replacen(
            "
mode = \"private\"
",
            &format!(
                "
mode = \"private\"
edition = \"{edition}\"
"
            ),
            1,
        )
    };
    let explicit = HostConfig::from_toml(&with("reborn")).unwrap();
    assert!(
        !explicit.to_toml().unwrap().contains(
            "
edition"
        ),
        "dropped when written"
    );
    for old in ["legacy", "both"] {
        let error = HostConfig::from_toml(&with(old)).unwrap_err().to_string();
        assert!(error.contains("no longer runs"), "{error}");
    }
    let error = HostConfig::from_toml(&with("classic")).unwrap_err().to_string();
    assert!(error.contains("edition must be reborn"), "{error}");
}

// ------------------------------------------------------------------ M1: setup migrates old-kit MongoDB data, start/serve refuse until migrated

#[cfg(all(feature = "isles-1-14-7", not(feature = "paradox")))]
mod m1_mongo_migration {
    use super::*;
    use isles_db::mongo::MockMongoServer;
    use serde_json::{Map, Value, json};
    use std::collections::BTreeMap;
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn seed_mongo_collections() -> BTreeMap<String, Vec<Map<String, Value>>> {
        let mut cols: BTreeMap<String, Vec<Map<String, Value>>> = BTreeMap::new();
        let doc = |val: Value| val.as_object().unwrap().clone();
        cols.insert(
            "accounts".into(),
            vec![doc(json!({
                "_id": "11111111-1111-4111-8111-111111111111",
                "userId": "11111111-1111-4111-8111-111111111111",
                "name": "SlayerOne",
                "notes": 2500,
                "email": "slayer@example.test",
                "displayNameNormalized": "slayerone",
                "displayName": "SlayerOne",
                "passwordHash": "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA",
                "status": "active",
                "approvalStatus": "approved",
                "approvalUpdatedAt": "2026-10-20T00:00:00.000Z",
                "approvalUpdatedBy": "system:signup",
                "roles": ["user"],
                "createdAt": "2026-10-20T00:00:00.000Z",
                "lastLoginAt": "2026-10-21T00:00:00.000Z",
                "usernameSet": 1
            }))],
        );
        cols.insert(
            "characters".into(),
            vec![doc(json!({
                "_id": "char-001",
                "characterId": "char-001",
                "userId": "11111111-1111-4111-8111-111111111111",
                "createdDate": "2026-10-20T00:00:00.000Z",
                "lastModifiedDate": "2026-10-20T00:00:00.000Z",
                "name": "SlayerOne",
                "updateVersion": 1,
                "data": { "primary_dye": "DYE_01" }
            }))],
        );
        cols.insert(
            "reforgedInvites".into(),
            vec![doc(json!({
                "_id": "INV-CUTOVER-01",
                "code": "INV-CUTOVER-01",
                "maxUses": 5,
                "uses": 1,
                "note": "Friday Hunts",
                "revoked": false,
                "createdAt": "2026-10-20T00:00:00.000Z"
            }))],
        );
        cols
    }

    fn write_old_kit_install(p: &Paths, game: &Path, mongo_uri: &str, manage_mongo: bool) {
        std::fs::create_dir_all(&p.config).unwrap();
        let host_json = json!({
            "version": 1,
            "mode": "private",
            "host": "100.101.102.103",
            "gameAddress": "100.101.102.103",
            "name": "Friday Hunts",
            "gameDir": game.to_string_lossy(),
            "registration": "INVITECODE",
            "shareUrl": null,
            "mongo": {
                "uri": mongo_uri,
                "manage": manage_mongo,
                "mongod": null
            },
            "maxHuntServers": 3
        });
        std::fs::write(&p.host_json, serde_json::to_string_pretty(&host_json).unwrap()).unwrap();
        std::fs::write(p.config.join("backend.env"), format!("MONGODB_URI={mongo_uri}\n")).unwrap();
        if manage_mongo {
            std::fs::create_dir_all(&p.mongo_data).unwrap();
            std::fs::write(p.mongo_data.join("WiredTiger"), "wiredtiger-header").unwrap();
        }
    }

    #[test]
    fn old_kit_install_with_mongo_data_and_no_marker_is_migrated_by_new_setup() {
        let (_dir, p, game) = checkout();
        let env = Mock::new();
        let seed = seed_mongo_collections();
        let mock_mongo = MockMongoServer::start(seed.clone(), 50).unwrap();
        let mongo_uri = mock_mongo.uri("reforged");
        write_old_kit_install(&p, &game, &mongo_uri, true);

        let marker = p.config.join(host::commands::update::MONGO_MIGRATED_MARKER);
        let sqlite_db = p.config.join(host::hostconfig::DATABASE_FILE);
        assert!(!marker.exists());
        assert!(!sqlite_db.exists());

        // An install made by an old kit runs `setup` with the new binary (as `update` from an old
        // kit invokes the newly installed binary's `setup`).
        let (r, lines) = setup(&p, &["--skip-install"], &env);
        assert!(r.is_ok(), "{r:?}\n{}", lines.join("\n"));
        let text = lines.join("\n");
        assert!(text.contains("migrating existing MongoDB data"), "{text}");
        assert!(
            text.contains("MongoDB migration verified") && text.contains("0 differences"),
            "{text}"
        );
        assert!(marker.is_file(), ".mongo-migrated marker written");
        assert!(sqlite_db.is_file(), "SQLite store populated");
        assert!(
            p.host_toml.is_file() && !p.host_json.exists(),
            "host.json moved to host.toml"
        );
        assert_eq!(
            std::fs::read_to_string(p.mongo_data.join("WiredTiger")).unwrap(),
            "wiredtiger-header",
            "managed MongoDB folder untouched"
        );
        assert_eq!(mock_mongo.snapshot(), seed, "source MongoDB collections never modified");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let verify = rt
            .block_on(isles_db::run_verify(&isles_db::MigrateOptions {
                from_uri: mongo_uri,
                to_target: sqlite_db.to_string_lossy().into_owned(),
                node: "local".parse().unwrap(),
                dry_run: false,
                areas: Vec::new(),
                skip_invalid: false,
                game_data_dir: None,
                allow_remote_postgres: false,
            }))
            .unwrap();
        assert!(verify.ok);
        let accounts = verify.collections.iter().find(|c| c.name == "accounts").unwrap();
        assert_eq!((accounts.expected_count, accounts.actual_count), (1, 1));

        // Running `setup` a second time sees `.mongo-migrated` and does not migrate again.
        let (r2, lines2) = setup(&p, &["--skip-install"], &env);
        assert!(r2.is_ok(), "{r2:?}");
        assert!(!lines2.iter().any(|l| l.contains("migrating existing MongoDB data")));
    }

    #[test]
    fn start_and_serve_refuse_before_migration_and_run_after() {
        let (_dir, p, game) = checkout();
        let env = Mock::new();
        let seed = seed_mongo_collections();
        let mock_mongo = MockMongoServer::start(seed, 50).unwrap();
        let mongo_uri = mock_mongo.uri("reforged");
        write_old_kit_install(&p, &game, &mongo_uri, true);

        // Before migration, both `start::run` (which dispatches to `serve::run` -> `serve::prepare`)
        // and `serve::compose` refuse when managed MongoDB data exists without `.mongo-migrated`.
        let err_start = host::commands::start::run(&p).unwrap_err().to_string();
        assert!(
            err_start.contains("unmigrated MongoDB data")
                && err_start.contains("reforged-host setup")
                && err_start.contains("--fresh-start"),
            "{err_start}"
        );
        let cfg = load_host_config(&p).unwrap().unwrap();
        let err_compose = match host::serve::compose(&p, &cfg, false, host::commands::start::console_printer()) {
            Ok(_) => panic!("compose must refuse unmigrated MongoDB data"),
            Err(e) => e.to_string(),
        };
        assert!(
            err_compose.contains("unmigrated MongoDB data") && err_compose.contains("reforged-host setup"),
            "{err_compose}"
        );

        // Also check when `.workspace/mongodb` is empty (external MongoDB) and the configured
        // MongoDB is reachable and holds documents.
        std::fs::remove_dir_all(&p.mongo_data).unwrap();
        write_old_kit_install(&p, &game, &mongo_uri, false);
        let err_ext = host::commands::start::run(&p).unwrap_err().to_string();
        assert!(
            err_ext.contains("unmigrated MongoDB data was found at")
                && err_ext.contains("reforged-host setup")
                && err_ext.contains("--fresh-start"),
            "{err_ext}"
        );

        // Now run `setup` to migrate the data.
        let (r, _) = setup(&p, &["--skip-install"], &env);
        assert!(r.is_ok(), "{r:?}");

        // After migration, `serve::compose` succeeds.
        let config = load_host_config(&p).unwrap().unwrap();
        let controller = host::serve::compose(&p, &config, false, host::commands::start::console_printer()).unwrap();
        drop(controller);
    }

    #[test]
    fn explicit_fresh_start_opt_out_works_via_cli_and_host_toml() {
        // 1. CLI opt-out: `reforged-host setup --fresh-start`
        let (_dir, p, game) = checkout();
        let env = Mock::new();
        let seed = seed_mongo_collections();
        let mock_mongo = MockMongoServer::start(seed.clone(), 50).unwrap();
        let mongo_uri = mock_mongo.uri("reforged");
        write_old_kit_install(&p, &game, &mongo_uri, true);

        let (r, lines) = setup(&p, &["--fresh-start", "--skip-install"], &env);
        assert!(r.is_ok(), "{r:?}\n{}", lines.join("\n"));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("skipping MongoDB migration") && l.contains("starting on a fresh database")),
            "{lines:?}"
        );
        let marker = p.config.join(host::commands::update::MONGO_MIGRATED_MARKER);
        assert!(marker.is_file());
        assert!(
            !p.config.join(host::hostconfig::DATABASE_FILE).exists(),
            "fresh start does not migrate MongoDB documents into dainty-lass.db"
        );
        assert_eq!(
            std::fs::read_to_string(p.mongo_data.join("WiredTiger")).unwrap(),
            "wiredtiger-header",
            "MongoDB folder never modified"
        );
        assert_eq!(mock_mongo.snapshot(), seed, "MongoDB documents never modified");
        let saved = read_host_config(&p.host_toml).unwrap().unwrap();
        assert!(saved.fresh_start());
        drop(host::serve::compose(&p, &saved, false, host::commands::start::console_printer()).unwrap());

        // 2. `host.toml` opt-out: `[database] fresh_start = true`
        let (_dir2, p2, game2) = checkout();
        write_old_kit_install(&p2, &game2, &mongo_uri, true);
        let mut hc = load_host_config(&p2).unwrap().unwrap();
        hc.database.get_or_insert_with(Default::default).fresh_start = Some(true);
        write_host_config(&p2.host_toml, &hc).unwrap();
        std::fs::remove_file(&p2.host_json).unwrap();
        std::fs::remove_file(p2.config.join(host::commands::update::MONGO_MIGRATED_MARKER)).ok();

        // Even before running setup again, `start`/`serve` honours `fresh_start = true` in `host.toml`.
        let hc2 = load_host_config(&p2).unwrap().unwrap();
        host::serve::ensure_server_files(&p2, &hc2).unwrap();
        drop(host::serve::compose(&p2, &hc2, false, host::commands::start::console_printer()).unwrap());
        std::fs::remove_file(p2.config.join(host::commands::update::MONGO_MIGRATED_MARKER)).ok();
        let (r2, lines2) = setup(&p2, &["--skip-install"], &env);
        assert!(r2.is_ok(), "{r2:?}");
        assert!(
            lines2
                .iter()
                .any(|l| l.contains("skipping MongoDB migration") && l.contains("starting on a fresh database"))
        );
        assert!(p2.config.join(host::commands::update::MONGO_MIGRATED_MARKER).is_file());
    }

    #[test]
    fn verify_difference_in_setup_leaves_server_unswitched() {
        let (_dir, p, game) = checkout();
        let env = Mock::new();
        let seed_migrate = seed_mongo_collections();
        let mut seed_verify = seed_mongo_collections();
        seed_verify.get_mut("accounts").unwrap().push(
            json!({
                "_id": "22222222-2222-4222-8222-222222222222",
                "userId": "22222222-2222-4222-8222-222222222222",
                "name": "ExtraSlayer",
                "notes": 0,
                "email": "extra@example.test",
                "displayNameNormalized": "extraslayer",
                "displayName": "ExtraSlayer",
                "passwordHash": "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA",
                "status": "active",
                "approvalStatus": "approved",
                "roles": ["user"],
                "createdAt": "2026-10-20T00:00:00.000Z",
                "usernameSet": 1
            })
            .as_object()
            .unwrap()
            .clone(),
        );

        let mongo_migrate = MockMongoServer::start(seed_migrate, 50).unwrap();
        let mongo_verify = MockMongoServer::start(seed_verify, 50).unwrap();
        let addr_migrate = mongo_migrate
            .uri("reforged")
            .strip_prefix("mongodb://")
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_owned();
        let addr_verify = mongo_verify
            .uri("reforged")
            .strip_prefix("mongodb://")
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_owned();

        // Forward the first two connections (`mongo_has_documents` + `run_migrate`) to `mongo_migrate`
        // and the third connection (`run_verify`) to `mongo_verify` so `isles-db verify` sees a real
        // mismatch in `accounts`.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let conn_count = Arc::new(AtomicUsize::new(0));
        let count_for_thread = Arc::clone(&conn_count);
        std::thread::spawn(move || {
            for incoming in listener.incoming().flatten() {
                let idx = count_for_thread.fetch_add(1, Ordering::SeqCst);
                let target = if idx < 2 {
                    addr_migrate.clone()
                } else {
                    addr_verify.clone()
                };
                std::thread::spawn(move || {
                    let Ok(upstream) = TcpStream::connect(&target) else {
                        return;
                    };
                    let (Ok(mut c_read), Ok(mut u_write)) = (incoming.try_clone(), upstream.try_clone()) else {
                        return;
                    };
                    let mut u_read = upstream;
                    let mut c_write = incoming;
                    let t1 = std::thread::spawn(move || {
                        let _ = std::io::copy(&mut c_read, &mut u_write);
                    });
                    let _ = std::io::copy(&mut u_read, &mut c_write);
                    let _ = t1.join();
                });
            }
        });

        let proxy_uri = format!("mongodb://{proxy_addr}/reforged");
        write_old_kit_install(&p, &game, &proxy_uri, true);
        let original_host_json = std::fs::read_to_string(&p.host_json).unwrap();
        let original_backend_env = std::fs::read_to_string(p.config.join("backend.env")).unwrap();

        let (r, lines) = setup(&p, &["--skip-install"], &env);
        let err = r.unwrap_err();
        assert!(
            err.contains("isles-db verify reported a difference")
                && err.contains("refusing to switch to the Rust backend"),
            "{err}\n{}",
            lines.join("\n")
        );
        assert!(
            !p.config.join(host::commands::update::MONGO_MIGRATED_MARKER).exists(),
            "no .mongo-migrated marker written when verify fails"
        );
        assert!(
            !p.config.join(host::hostconfig::DATABASE_FILE).exists(),
            "unverified dainty-lass.db removed when verify fails"
        );
        assert!(
            p.host_json.is_file() && !p.host_toml.exists(),
            "host.json not switched to host.toml when verify fails"
        );
        assert_eq!(std::fs::read_to_string(&p.host_json).unwrap(), original_host_json);
        assert_eq!(
            std::fs::read_to_string(p.config.join("backend.env")).unwrap(),
            original_backend_env,
            "backend.env not overwritten when verify fails"
        );
    }

    #[test]
    fn unreachable_configured_external_mongo_refuses_in_setup_and_start_unless_fresh_start_is_given() {
        let (_dir, p, game) = checkout();
        let env = Mock::new();
        let closed_port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let unreachable_uri = format!("mongodb://mongo_user:secret_pw@127.0.0.1:{closed_port}/reforged");
        write_old_kit_install(&p, &game, &unreachable_uri, false);
        assert!(
            !p.mongo_data.exists(),
            "external MongoDB does not use .workspace/mongodb"
        );
        let original_host_json = std::fs::read_to_string(&p.host_json).unwrap();
        let original_backend_env = std::fs::read_to_string(p.config.join("backend.env")).unwrap();

        // 1. `setup` refuses when configured external MongoDB is unreachable, redacts any URI
        //    password, names how to start it or opt out with `--fresh-start`, and leaves the
        //    server unswitched.
        let (r, _lines) = setup(&p, &["--skip-install"], &env);
        let err = r.unwrap_err();
        assert!(
            err.contains("MongoDB is configured at")
                && err.contains("not reachable")
                && err.contains("start MongoDB")
                && err.contains("--fresh-start")
                && !err.contains("secret_pw"),
            "{err}"
        );
        assert!(!p.config.join(host::commands::update::MONGO_MIGRATED_MARKER).exists());
        assert!(p.host_json.is_file() && !p.host_toml.exists());
        assert_eq!(std::fs::read_to_string(&p.host_json).unwrap(), original_host_json);
        assert_eq!(
            std::fs::read_to_string(p.config.join("backend.env")).unwrap(),
            original_backend_env
        );

        // 2. `start` also refuses while the configured external MongoDB is unreachable.
        let err_start = host::commands::start::run(&p).unwrap_err().to_string();
        assert!(
            err_start.contains("MongoDB is configured at")
                && err_start.contains("not reachable")
                && err_start.contains("--fresh-start")
                && !err_start.contains("secret_pw"),
            "{err_start}"
        );

        // 3. Explicit `--fresh-start` opt-out succeeds and lets the server start.
        let (r_fresh, lines_fresh) = setup(&p, &["--fresh-start", "--skip-install"], &env);
        assert!(r_fresh.is_ok(), "{r_fresh:?}\n{}", lines_fresh.join("\n"));
        assert!(p.config.join(host::commands::update::MONGO_MIGRATED_MARKER).is_file());
        let saved = read_host_config(&p.host_toml).unwrap().unwrap();
        drop(host::serve::compose(&p, &saved, false, host::commands::start::console_printer()).unwrap());
    }

    #[test]
    fn fresh_apply_and_toml_profile_succeed_without_mongodb() {
        // 1. Fresh `apply` (`setup --skip-install`) on a brand-new node whose `host.toml` was
        //    written by `docker/network/init.sh` (both with no `[mongo]` table and with the
        //    legacy `uri = "mongodb://127.0.0.1:27017/?replicaSet=rs0"` / `manage = false`).
        let (_dir, p, _game) = checkout();
        let env = Mock {
            platform: Platform::Linux,
            mongod: None,
            ..Mock::new()
        };
        std::fs::create_dir_all(&p.config).unwrap();
        let network_node_toml = "\
version = 1
mode = \"public\"
host = \"oregon.cluster.test\"
game_address = \"203.0.113.11\"
name = \"Network node oregon-1\"
registration = \"INVITECODE\"
max_hunt_servers = 1

[mongo]
uri = \"mongodb://127.0.0.1:27017/?replicaSet=rs0\"
manage = false

[linux]
wine = \"/nonexistent/wine\"
xvfb = true

[network]
node = \"oregon-1\"
region = \"oregon\"
";
        std::fs::write(&p.host_toml, network_node_toml).unwrap();
        assert!(!p.config.join(host::commands::update::MONGO_MIGRATED_MARKER).exists());
        assert!(!p.mongo_data.exists());

        let (r, lines) = setup(&p, &["--skip-install"], &env);
        assert!(r.is_ok(), "fresh apply must succeed: {r:?}\n{}", lines.join("\n"));
        let cfg = load_host_config(&p).unwrap().unwrap();
        assert!(host::commands::update::check_unmigrated_mongo_data(&p, &cfg).is_ok());

        // 2. Fresh `apply` under the `toml` profile: a brand-new server where `/config/host.toml`
        //    is copied into `.workspace/config/host.toml` (with no `[mongo]` table at all, or
        //    with the default `[mongo]` table) before `secrets.env`, `backend.env` or
        //    `.mongo-migrated` exist.
        let (_dir2, p2, _game2) = checkout();
        std::fs::create_dir_all(&p2.config).unwrap();
        let toml_profile_cfg = "\
version = 1
mode = \"public\"
host = \"smoke.example.org\"
game_address = \"203.0.113.7\"
name = \"Fresh Toml Profile\"
registration = \"INVITECODE\"
max_hunt_servers = 3
";
        std::fs::write(&p2.host_toml, toml_profile_cfg).unwrap();
        let (r2, lines2) = setup(&p2, &["--skip-install"], &env);
        assert!(
            r2.is_ok(),
            "fresh apply under toml profile must succeed: {r2:?}\n{}",
            lines2.join("\n")
        );
        let cfg2 = load_host_config(&p2).unwrap().unwrap();
        drop(host::serve::compose(&p2, &cfg2, false, host::commands::start::console_printer()).unwrap());

        // 3. An old kit's external, unreachable MongoDB (in `host.toml` or `host.json`) is still refused.
        let (_dir3, p3, _game3) = checkout();
        std::fs::create_dir_all(&p3.config).unwrap();
        let closed_port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let old_external_toml = format!(
            "\
version = 1
mode = \"public\"
host = \"smoke.example.org\"
game_address = \"203.0.113.7\"
name = \"Old External Mongo\"
registration = \"INVITECODE\"
max_hunt_servers = 3

[mongo]
uri = \"mongodb://mongo_user:secret_pw@127.0.0.1:{closed_port}/reforged\"
manage = false
"
        );
        std::fs::write(&p3.host_toml, old_external_toml).unwrap();
        let (r3, _) = setup(&p3, &["--skip-install"], &env);
        let err3 = r3.unwrap_err();
        assert!(
            err3.contains("MongoDB is configured at") && err3.contains("not reachable") && !err3.contains("secret_pw"),
            "{err3}"
        );
    }

    #[test]
    fn removing_fresh_start_from_host_toml_clears_opt_out_marker_and_runs_migration() {
        let (_dir, p, game) = checkout();
        let env = Mock::new();
        let seed = seed_mongo_collections();
        let mock_mongo = MockMongoServer::start(seed.clone(), 50).unwrap();
        let mongo_uri = mock_mongo.uri("reforged");
        write_old_kit_install(&p, &game, &mongo_uri, true);

        // 1. Run `setup --fresh-start`: writes the fresh-start marker and leaves SQLite unmigrated.
        let (r_fresh, _) = setup(&p, &["--fresh-start", "--skip-install"], &env);
        assert!(r_fresh.is_ok(), "{r_fresh:?}");
        let marker = p.config.join(host::commands::update::MONGO_MIGRATED_MARKER);
        let sqlite_db = p.config.join(host::hostconfig::DATABASE_FILE);
        assert!(marker.is_file());
        assert!(!sqlite_db.exists());

        // 2. Remove `fresh_start = true` from `host.toml`: `start` now refuses because the
        //    fresh-start marker is cleared and unmigrated MongoDB data is still present, and
        //    running `setup` migrates and verifies the MongoDB data into SQLite.
        let toml_without_fresh = std::fs::read_to_string(&p.host_toml)
            .unwrap()
            .replace("fresh_start = true\n", "");
        std::fs::write(&p.host_toml, toml_without_fresh).unwrap();
        let cfg = load_host_config(&p).unwrap().unwrap();
        assert!(!cfg.fresh_start());

        let err_start = host::commands::start::run(&p).unwrap_err().to_string();
        assert!(err_start.contains("unmigrated MongoDB data"), "{err_start}");

        let (r_mig, lines_mig) = setup(&p, &["--skip-install"], &env);
        assert!(r_mig.is_ok(), "{r_mig:?}\n{}", lines_mig.join("\n"));
        let text = lines_mig.join("\n");
        assert!(
            text.contains("MongoDB migration verified") && text.contains("0 differences"),
            "{text}"
        );
        assert!(sqlite_db.is_file());
        assert!(std::fs::read_to_string(&marker).unwrap().starts_with("migrated from "));
    }
}
