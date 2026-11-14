//! The Node kit's tests of host.json, the secrets and the service env files
//! (`legacy/ReforgedHost/test/config.test.ts`), with the host.json and env-file cases of
//! `ports.test.ts`, `performance.test.ts` and `linux.test.ts`. The cases of those files that test
//! setup's option readers, invites, the firewall scripts, doctor and MongoDB belong to the rows that
//! port that code (RH3, RH5).

#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::path::Path;

use dainty_host::envfile::parse_env;
use dainty_host::hostconfig::{
    DEFAULT_MONGO_URI, DEFAULT_PERFORMANCE, EnvEntries, HostConfig, Platform, QUIET_GAME_LOG_CMDS, Secrets,
    build_env_files, game_ports, gateway_port, load_or_create_secrets, read_host_config, validate, write_env_files,
    write_host_config,
};
use dainty_host::paths::{Paths, game_exe_relative};
use serde_json::{Value, json};

fn private() -> Value {
    json!({
        "version": 1,
        "mode": "private",
        "host": "100.101.102.103",
        "gameAddress": "100.101.102.103",
        "name": "Friday Hunts",
        "gameDir": "D:\\Games\\Dauntless",
        "registration": "INVITECODE",
        "shareUrl": null,
        "mongo": { "uri": DEFAULT_MONGO_URI, "manage": true, "mongod": null },
        "maxHuntServers": 3,
    })
}

fn public() -> Value {
    with(
        private(),
        json!({ "mode": "public", "host": "hunts.example.net", "gameAddress": "203.0.113.7" }),
    )
}

/// `{ ...base, ...changes }`
fn with(mut base: Value, changes: Value) -> Value {
    if let (Some(b), Some(c)) = (base.as_object_mut(), changes.as_object()) {
        for (k, v) in c {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

fn config(value: Value) -> HostConfig {
    HostConfig::from_json(&value).unwrap()
}

fn error(value: Value) -> String {
    validate(&value).unwrap_err().to_string()
}

fn get<'a>(entries: &'a EnvEntries, key: &str) -> Option<&'a str> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn default_performance() -> Value {
    let d = DEFAULT_PERFORMANCE;
    json!({
        "serverFps": d.server_fps, "idleFps": d.idle_fps, "trainingOnDemand": d.training_on_demand,
        "trainingIdleMinutes": d.training_idle_minutes, "verboseGameLogs": d.verbose_game_logs,
        "memoryGuard": d.memory_guard, "minFreeMemoryMb": d.min_free_memory_mb, "huntMemoryMb": d.hunt_memory_mb,
    })
}

fn fast_secrets() -> Secrets {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let hex = |i: u32| format!("{:064x}", u64::from(n) * 1000 + u64::from(i));
    Secrets {
        gateway_secret: hex(1),
        allowlist_secret: hex(2),
        admin_key: hex(3),
        gameserver_key: hex(4),
        api_key_hash_secret: hex(5),
        metrics_token: hex(6),
        jwt_private_key_b64: hex(7),
        jwt_public_key_b64: hex(8),
        admin_api_key: hex(9),
        discord_bot_token: String::new(),
        operator_secret: hex(10),
        database_password: hex(11),
        dashboard_ingest_key: hex(12),
    }
}

// ------------------------------------------------------------------ host.json

#[test]
fn accepts_a_private_server_on_a_tailscale_address_and_a_public_one_on_a_public_address_or_name() {
    assert_eq!(validate(&private()).unwrap()["host"], "100.101.102.103");
    assert_eq!(
        validate(&with(private(), json!({ "host": "hostpc.tail1234.ts.net" }))).unwrap()["host"],
        "hostpc.tail1234.ts.net"
    );
    assert_eq!(validate(&public()).unwrap()["host"], "hunts.example.net");
    assert_eq!(
        validate(&with(public(), json!({ "host": "203.0.113.7" }))).unwrap()["mode"],
        "public"
    );
}

#[test]
fn refuses_addresses_that_cannot_work_for_the_mode_and_other_bad_values() {
    assert!(error(with(private(), json!({ "host": "192.168.1.20" }))).contains("Tailscale"));
    assert!(error(with(private(), json!({ "gameAddress": "203.0.113.7" }))).contains("Tailscale"));
    assert!(error(with(public(), json!({ "host": "192.168.1.20" }))).contains("public"));
    assert!(error(with(public(), json!({ "host": "100.101.102.103" }))).contains("public"));
    assert!(error(with(public(), json!({ "gameAddress": "hunts.example.net" }))).contains("IPv4"));
    assert!(error(with(private(), json!({ "name": "  " }))).contains("name"));
    assert!(error(with(private(), json!({ "maxHuntServers": 11 }))).contains("maxHuntServers"));
    assert!(error(with(private(), json!({ "maxHuntServers": 0 }))).contains("maxHuntServers"));
    assert_eq!(
        validate(&with(private(), json!({ "maxHuntServers": 10 }))).unwrap()["maxHuntServers"],
        10
    );
    assert!(error(with(private(), json!({ "shareUrl": "https://evil.example/" }))).contains("shareUrl"));
    assert!(
        error(with(
            private(),
            json!({ "mongo": { "uri": "http://x", "manage": true, "mongod": null } })
        ))
        .contains("mongo")
    );
}

#[test]
fn is_written_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("host.json");
    let mut c = config(private());
    c.name = "  Friday\u{7} Hunts ".into();
    write_host_config(&file, &c).unwrap();
    assert_eq!(read_host_config(&file).unwrap().unwrap().name, "Friday Hunts");
    assert!(read_host_config(&dir.path().join("none.json")).unwrap().is_none());
}

// ------------------------------------------------------------------ settings.toml

/// A config written to settings.toml and read back.
fn through_toml(c: &HostConfig) -> (HostConfig, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    write_host_config(&file, c).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    (read_host_config(&file).unwrap().unwrap(), text)
}

#[test]
fn settings_toml_holds_every_field_of_host_json() {
    let full = with(
        public(),
        json!({
            "publicPort": 4443,
            "shareUrl": null,
            "mongo": { "uri": DEFAULT_MONGO_URI, "manage": true, "mongod": "/usr/bin/mongod" },
            "performance": {
                "serverFps": 0, "idleFps": 1, "trainingOnDemand": false, "trainingIdleMinutes": 0,
                "verboseGameLogs": true, "memoryGuard": false, "minFreeMemoryMb": 0, "huntMemoryMb": 256,
            },
            "linux": { "wine": "/usr/bin/wine", "proton": null, "xvfb": false, "firewallInterface": "eth0" },
        }),
    );
    let tests = [
        config(full),
        config(with(
            private(),
            json!({ "multipliers": { "xp": 5, "rewards": 2.5 }, "gameDir": null }),
        )),
    ];
    for c in tests {
        let (back, text) = through_toml(&c);
        assert_eq!(
            (
                back.mode,
                &back.host,
                &back.game_address,
                &back.name,
                &back.game_dir,
                back.registration
            ),
            (c.mode, &c.host, &c.game_address, &c.name, &c.game_dir, c.registration),
            "{text}"
        );
        assert_eq!(
            (&back.share_url, &back.mongo, back.max_hunt_servers),
            (&c.share_url, &c.mongo, c.max_hunt_servers)
        );
        assert_eq!(
            (back.multipliers, back.public_port),
            (c.multipliers, c.public_port),
            "{text}"
        );
        assert_eq!((back.performance, &back.linux), (c.performance, &c.linux), "{text}");
        assert!(
            !text.contains("null") && !text.contains("Fps"),
            "no null, snake_case keys: {text}"
        );
    }
}

#[test]
fn settings_toml_errors_name_its_own_keys() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    let (_, text) = through_toml(&config(private()));
    let error = |text: &str| {
        std::fs::write(&file, text).unwrap();
        read_host_config(&file).unwrap_err().to_string()
    };
    let e = error(&text.replace("max_hunt_servers = 3", "max_hunt_servers = 99"));
    assert!(e.contains("max_hunt_servers must be 1 to 10"), "{e}");
    let e = error(&format!("{text}\n[performance]\nserver_fps = 5\n"));
    assert!(
        e.contains("performance.server_fps must be 0 (no cap) or 10 to 240"),
        "{e}"
    );
    let e = error(&text.replace("game_address", "gameAddress"));
    assert!(e.contains("did you mean game_address?"), "{e}");
    assert!(e.contains("settings.toml"), "names the file: {e}");
}

// ------------------------------------------------------------------ secrets

#[test]
fn secrets_are_made_once_and_kept_and_a_missing_one_is_added_without_touching_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let mut made = 0;
    let mut make = || {
        made += 1;
        Ok(fast_secrets())
    };
    let (first, created) = load_or_create_secrets(dir.path(), &mut make).unwrap();
    assert!(created);
    let (again, created) = load_or_create_secrets(dir.path(), &mut make).unwrap();
    assert!(!created);
    assert_eq!(again, first);
    let file = dir.path().join("secrets.env");
    let text = std::fs::read_to_string(&file).unwrap();
    let without: String = text
        .split_inclusive('\n')
        .map(|line| {
            if line.starts_with("DAINTY_ADMIN_KEY=") {
                "\n"
            } else {
                line
            }
        })
        .collect();
    std::fs::write(&file, without).unwrap();
    let (third, created) = load_or_create_secrets(dir.path(), &mut make).unwrap();
    assert!(created);
    assert_ne!(third.admin_key, first.admin_key);
    assert_eq!(third.gateway_secret, first.gateway_secret);
    assert_eq!(third.jwt_private_key_b64, first.jwt_private_key_b64);
    assert_eq!(made, 2);
}

#[test]
fn secrets_under_their_names_from_before_the_rename_move_to_the_new_ones_with_the_same_values() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = load_or_create_secrets(dir.path(), || Ok(fast_secrets())).unwrap();
    let file = dir.path().join("secrets.env");
    let text = std::fs::read_to_string(&file).unwrap();
    let old = text.replace("RUSTIC_ISLES_", "MYSTICPARADOX_");
    assert!(old.contains("MYSTICPARADOX_METRICS_TOKEN=") && !old.contains("RUSTIC_ISLES_"));
    std::fs::write(&file, old).unwrap();

    let (moved, created) = load_or_create_secrets(dir.path(), || -> Result<Secrets, _> {
        panic!("a renamed secret is never made again")
    })
    .unwrap();
    assert!(!created);
    assert_eq!(moved, first);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("MYSTICPARADOX_"), "{text}");
    assert!(text.contains(&format!("RUSTIC_ISLES_METRICS_TOKEN={}", first.metrics_token)));
    assert!(text.contains(&format!(
        "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET={}",
        first.operator_secret
    )));

    // A file with both keeps the new name's value and leaves the old line alone
    std::fs::write(
        &file,
        format!(
            "{text}MYSTICPARADOX_METRICS_TOKEN=stale
"
        ),
    )
    .unwrap();
    let (both, _) = load_or_create_secrets(dir.path(), || -> Result<Secrets, _> { panic!("kept") }).unwrap();
    assert_eq!(both.metrics_token, first.metrics_token);
}

// ------------------------------------------------------------------ the service env files

fn win_paths() -> Paths {
    Paths::new("C:\\rr")
}

#[test]
fn the_admin_api_gets_its_keys_on_loopback_and_the_backend_accepts_its_signed_calls() {
    let s = fast_secrets();
    let p = win_paths();
    let f = build_env_files(
        &config(with(public(), json!({"maxHuntServers": 5}))),
        &s,
        &p,
        "",
        Platform::Windows,
    );
    assert_eq!(get(&f.backend, "RUSTIC_ISLES_MANAGER_ADMIN"), Some("on"));
    assert_eq!(
        get(&f.backend, "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET"),
        Some(s.operator_secret.as_str())
    );
    assert!(
        f.files()
            .iter()
            .any(|(name, entries)| *name == "admin" && entries.is_some())
    );
    // Read back by the admin API as `admin serve --env-file admin.env` reads it
    let env = dainty_service_kit::env::Env::from_pairs(f.admin.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let read = dainty_admin::config::Config::from_env(&env).unwrap();
    assert_eq!(
        (read.port, read.bind_hosts.clone()),
        (41003, vec!["127.0.0.1".to_owned()])
    );
    assert_eq!(read.backend_url, "http://127.0.0.1:41000");
    assert_eq!(read.director_url, "http://127.0.0.1:41001");
    assert_eq!(read.api_key, s.admin_api_key);
    assert_eq!(read.admin_key, s.admin_key);
    assert_eq!(read.operator_secret, s.operator_secret);
    assert_eq!(read.max_hunt_servers, 5);
    assert_eq!(read.server_mode, dainty_admin::config::ServerMode::Public);
    // In the config folder, kept with secrets.env in the host's backups
    assert_eq!(
        get(&f.admin, "DAINTY_ADMIN_API_TIMEOUTS_FILE"),
        p.config.join("admin-timeouts.json").to_str()
    );
}

#[test]
fn the_database_engine_goes_into_admin_env_and_reads_back_as_store_reads_it() {
    let s = fast_secrets();
    let p = win_paths();
    let read = |doc: Value| {
        let f = build_env_files(&config(doc), &s, &p, "", Platform::Windows);
        assert!(
            f.backend.iter().all(|(k, _)| !k.starts_with("DAINTY_DB_")),
            "the Node backend keeps Mongo"
        );
        let env = dainty_service_kit::env::Env::from_pairs(f.admin.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        dainty_admin::config::Config::from_env(&env).unwrap().database
    };
    // No [database], as in every file before DF7: SQLite in the config folder.
    assert_eq!(
        read(private()),
        dainty_store::DatabaseConfig::Sqlite {
            file: p.config.join("dainty-lass.db")
        }
    );
    let postgres = read(with(
        private(),
        json!({"database": {"engine": "postgres", "postgresPort": 5433, "postgresUser": "hunts"}}),
    ));
    let dainty_store::DatabaseConfig::Postgres(pg) = postgres else {
        panic!("postgres: {postgres:?}")
    };
    assert_eq!(
        (pg.host.as_str(), pg.port, pg.user.as_str(), pg.database.as_str()),
        ("127.0.0.1", 5433, "hunts", "dainty")
    );
    assert_eq!(pg.password, s.database_password, "the secret from secrets.env");
    assert!(!pg.allow_remote);
    let remote = read(with(
        private(),
        json!({"database": {"engine": "postgres", "postgresHost": "10.0.0.5", "allowRemote": true}}),
    ));
    assert!(
        matches!(remote, dainty_store::DatabaseConfig::Postgres(ref pg) if pg.allow_remote && pg.host == "10.0.0.5")
    );
}

#[test]
fn the_guard_goes_into_admin_env_and_reads_back_as_the_admin_api_reads_it() {
    let s = fast_secrets();
    let p = win_paths();
    let read = |doc: Value| {
        let f = build_env_files(&config(doc), &s, &p, "", Platform::Windows);
        let env = dainty_service_kit::env::Env::from_pairs(f.admin.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        (
            dainty_admin::config::Config::from_env(&env).unwrap().guard,
            f.admin.iter().any(|(k, _)| k.starts_with("DAINTY_GUARD_")),
        )
    };
    // No [guard], as in every file before DH6: the defaults, and no line added to admin.env
    let (guard, written) = read(private());
    assert_eq!(guard, dainty_admin::guard::GuardSettings::default());
    assert!(!written);
    let (guard, written) = read(with(
        private(),
        json!({"guard": {"banLimit": 3, "banWindowMinutes": 30, "banGuard": "approval", "revertLimit": 20,
                         "revertGuard": "approval", "revokeGuard": "off", "delayMinutes": 45, "expireHours": 6}}),
    ));
    assert!(written);
    assert_eq!(
        guard,
        dainty_admin::guard::GuardSettings {
            ban_limit: 3,
            ban_window_minutes: 30,
            ban_guard: dainty_admin::guard::Rule::Approval,
            revert_limit: 20,
            revert_guard: dainty_admin::guard::Rule::Approval,
            revoke_guard: dainty_admin::guard::Rule::Off,
            delay_minutes: 45,
            expire_hours: 6,
        }
    );
    // A setting left out keeps its default
    let (guard, _) = read(with(private(), json!({"guard": {"banGuard": "approval"}})));
    assert_eq!(
        guard,
        dainty_admin::guard::GuardSettings {
            ban_guard: dainty_admin::guard::Rule::Approval,
            ..dainty_admin::guard::GuardSettings::default()
        }
    );
    // The file keeps what the host wrote, and a bad value stops setup
    let kept = config(with(private(), json!({"guard": {"delayMinutes": 15}})));
    assert_eq!(kept.to_json().unwrap()["guard"], json!({"delayMinutes": 15}));
    for bad in [
        json!({"banLimit": 0}),
        json!({"banLimit": 1.5}),
        json!({"banWindowMinutes": 1441}),
        json!({"revertLimit": 1000001}),
        json!({"delayMinutes": 0}),
        json!({"expireHours": 721}),
        json!({"banGuard": "Delay"}),
        json!({"revokeGuard": true}),
        json!("delay"),
    ] {
        assert!(
            HostConfig::from_json(&with(private(), json!({ "guard": bad }))).is_err(),
            "{bad}"
        );
    }
    let toml = HostConfig::from_toml(
        "version = 1
mode = \"private\"
host = \"100.101.102.103\"
game_address = \"100.101.102.103\"
         name = \"x\"
registration = \"INVITECODE\"
max_hunt_servers = 3
         [mongo]
uri = \"mongodb://127.0.0.1:27017/\"
manage = false
         [guard]
ban_limit = 5
revoke_guard = \"approval\"
",
    )
    .unwrap();
    assert_eq!(toml.guard.as_ref().and_then(|g| g.ban_limit), Some(5));
    assert_eq!(toml.guard.and_then(|g| g.revoke_guard).as_deref(), Some("approval"));
}

#[test]
fn a_postgres_server_off_this_machine_needs_allow_remote() {
    let far = with(
        private(),
        json!({"database": {"engine": "postgres", "postgresHost": "db.example.org"}}),
    );
    let e = HostConfig::from_json(&far).unwrap_err().to_string();
    assert!(e.contains("not encrypted"), "{e}");
    let allowed = with(
        private(),
        json!({"database": {"engine": "postgres", "postgresHost": "db.example.org", "allowRemote": true}}),
    );
    assert!(HostConfig::from_json(&allowed).is_ok());
    let sqlite_with_a_host = with(private(), json!({"database": {"postgresHost": "db.example.org"}}));
    assert!(
        HostConfig::from_json(&sqlite_with_a_host).is_ok(),
        "only Postgres connects"
    );
    for bad in [
        json!({"engine": "mysql"}),
        json!({"postgresPort": 0}),
        json!({"postgresUser": ""}),
        json!({"allowRemote": "yes"}),
        json!({"freshStart": "yes"}),
        json!("postgres"),
    ] {
        assert!(
            HostConfig::from_json(&with(private(), json!({ "database": bad }))).is_err(),
            "{bad}"
        );
    }
    let fresh = config(with(private(), json!({"database": {"freshStart": true}})));
    assert!(fresh.fresh_start());
    let (back, text) = through_toml(&fresh);
    assert!(back.fresh_start(), "{text}");
    assert!(text.contains("fresh_start = true"), "{text}");
    let e = HostConfig::from_toml(
        "version = 1\nmode = \"private\"\nhost = \"100.101.102.103\"\ngame_address = \"100.101.102.103\"\n\
         name = \"x\"\nregistration = \"INVITECODE\"\nmax_hunt_servers = 3\n\
         [mongo]\nuri = \"mongodb://127.0.0.1:27017/\"\nmanage = false\n\
         [database]\nengine = \"postgres\"\npostgres_host = \"203.0.113.4\"\n",
    )
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("database.postgres_host") && e.contains("database.allow_remote"),
        "{e}"
    );
}

#[test]
fn private_mode_the_backend_behind_the_gateway_on_the_tailscale_address_in_plain_http_no_allowlist() {
    let s = fast_secrets();
    let p = win_paths();
    let f = build_env_files(&config(private()), &s, &p, "abc123", Platform::Windows);
    assert_eq!(get(&f.backend, "AUTH_MODE"), Some("APIKEY"));
    assert_eq!(get(&f.backend, "NODE_ENV"), Some("production"));
    assert_eq!(get(&f.backend, "RUSTIC_ISLES_SERVICE_ROLE"), Some("api"));
    assert_eq!(get(&f.backend, "BIND_HOST"), Some("127.0.0.1"));
    assert_eq!(get(&f.backend, "DAINTY_HTTP_ONLY"), Some("true"));
    assert_eq!(get(&f.backend, "QOS_TARGET_URL"), Some("http://127.0.0.1:61100/QoS"));
    assert_eq!(get(&f.backend, "REALTIME_XMPP_DEV_CITY_MUC"), Some("false"));
    assert_eq!(
        get(&f.backend, "HUNT_GO_NOW_MIN_DELAY_MS"),
        Some("1"),
        "no 10-20 s wait (with the build locked) after the hunt server is ready"
    );
    assert_eq!(get(&f.backend, "HUNT_GO_NOW_MAX_DELAY_MS"), Some("1000"));
    assert_eq!(get(&f.backend, "REALTIME_XMPP_ALLOWED_HOSTS"), Some("100.101.102.103"));
    assert_eq!(get(&f.backend, "GAMESERVER_API_KEYS"), Some(s.gameserver_key.as_str()));
    assert_eq!(
        get(&f.director, "METAGAME_API_KEY"),
        Some(s.gameserver_key.as_str()),
        "the Director's game servers use the backend's game-server key"
    );
    assert_eq!(get(&f.director, "MY_IP"), Some("100.101.102.103"));
    assert_eq!(get(&f.director, "DAINTY_BACKEND_URL"), Some("http://127.0.0.1:41000"));
    assert_eq!(get(&f.director, "PORT_RANGE_BEGIN"), Some("8770"));
    assert_eq!(get(&f.director, "PORT_RANGE_END"), Some("8777"));
    assert!(
        get(&f.director, "GAMESERVER_BINARY_PATH")
            .unwrap()
            .ends_with("Dauntless-Win64-Shipping.exe")
    );
    assert_eq!(get(&f.gateway, "GATEWAY_TLS"), Some("0"));
    assert_eq!(get(&f.gateway, "GATEWAY_BIND"), Some("100.101.102.103"));
    assert_eq!(get(&f.gateway, "GATEWAY_PORT"), Some("61000"));
    assert_eq!(get(&f.gateway, "GATEWAY_ALLOWLIST"), Some("0"));
    assert_eq!(f.allowlist, None);
    assert_eq!(
        get(&f.content, "CONTENT_RUNTIME_DIR"),
        Some(p.runtime_publish.to_str().unwrap())
    );
}

#[test]
fn public_mode_tls_on_443_with_the_certificate_and_the_allowlist_helper_for_real_on_windows_and_linux() {
    let s = fast_secrets();
    let p = win_paths();
    let c = config(public());
    let f = build_env_files(&c, &s, &p, "", Platform::Windows);
    let allowlist = f.allowlist.clone().unwrap();
    assert_eq!(get(&f.gateway, "GATEWAY_TLS"), Some("1"));
    assert_eq!(get(&f.gateway, "GATEWAY_BIND"), Some("0.0.0.0"));
    assert_eq!(get(&f.gateway, "GATEWAY_PORT"), Some("443"));
    assert_eq!(get(&f.gateway, "GATEWAY_CERT"), Some(p.cert_file.to_str().unwrap()));
    assert_eq!(get(&f.gateway, "ALLOWLIST_SECRET"), Some(s.allowlist_secret.as_str()));
    assert_eq!(get(&allowlist, "ALLOWLIST_SECRET"), Some(s.allowlist_secret.as_str()));
    assert_eq!(get(&allowlist, "ALLOWLIST_DRY_RUN"), Some("0"));
    assert_eq!(get(&allowlist, "ALLOWLIST_FIREWALL"), Some("windows"));
    let linux = build_env_files(&c, &s, &p, "", Platform::Linux).allowlist.unwrap();
    assert_eq!(get(&linux, "ALLOWLIST_DRY_RUN"), Some("0"));
    assert_eq!(get(&linux, "ALLOWLIST_FIREWALL"), Some("nftables"));
    let other = build_env_files(&c, &s, &p, "", Platform::Other).allowlist.unwrap();
    assert_eq!(get(&other, "ALLOWLIST_DRY_RUN"), Some("1"));
    assert_eq!(get(&f.director, "MY_IP"), Some("203.0.113.7"));
    assert_eq!(
        get(&f.backend, "REALTIME_XMPP_ALLOWED_HOSTS"),
        Some("hunts.example.net")
    );
}

#[test]
fn test_multipliers_only_a_private_servers_backend_gets_them_always_with_its_mode_and_host() {
    let s = fast_secrets();
    let p = win_paths();
    let on = build_env_files(
        &config(with(private(), json!({ "multipliers": { "xp": 5, "rewards": 3 } }))),
        &s,
        &p,
        "",
        Platform::Windows,
    );
    assert_eq!(get(&on.backend, "DAINTY_SERVER_MODE"), Some("private"));
    assert_eq!(get(&on.backend, "DAINTY_SERVER_HOST"), Some("100.101.102.103"));
    assert_eq!(get(&on.backend, "DAINTY_XP_MULTIPLIER"), Some("5"));
    assert_eq!(get(&on.backend, "DAINTY_REWARD_MULTIPLIER"), Some("3"));
    let off = build_env_files(&config(private()), &s, &p, "", Platform::Windows);
    assert_eq!(get(&off.backend, "DAINTY_XP_MULTIPLIER"), None);
    let public_files = build_env_files(&config(public()), &s, &p, "", Platform::Windows);
    assert_eq!(get(&public_files.backend, "DAINTY_SERVER_MODE"), Some("public"));
    assert_eq!(get(&public_files.backend, "DAINTY_XP_MULTIPLIER"), None);
    assert!(error(with(public(), json!({ "multipliers": { "xp": 5, "rewards": 1 } }))).contains("private"));
    assert!(error(with(private(), json!({ "multipliers": { "xp": 101, "rewards": 1 } }))).contains("1 to 100"));
    assert!(error(with(private(), json!({ "multipliers": { "xp": 0.5, "rewards": 1 } }))).contains("1 to 100"));
    assert_eq!(
        validate(&with(public(), json!({ "multipliers": { "xp": 1, "rewards": 1 } }))).unwrap()["mode"],
        "public"
    );
}

#[test]
fn are_written_owner_only_and_the_allowlist_file_goes_away_in_private_mode() {
    let s = fast_secrets();
    let root = tempfile::tempdir().unwrap();
    let lp = Paths::new(root.path());
    write_env_files(&lp, &build_env_files(&config(public()), &s, &lp, "", Platform::Linux)).unwrap();
    assert!(lp.config.join("allowlist.env").exists());
    write_env_files(&lp, &build_env_files(&config(private()), &s, &lp, "", Platform::Linux)).unwrap();
    assert!(!lp.config.join("allowlist.env").exists());
    let backend = parse_env(&std::fs::read_to_string(lp.config.join("backend.env")).unwrap());
    assert_eq!(backend.get("DAINTY_ADMIN_KEY"), Some(s.admin_key.as_str()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(lp.config.join("backend.env"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}

// ------------------------------------------------------------------ ports.test.ts

fn moved() -> Value {
    with(public_on_ip(), json!({ "publicPort": 4443 }))
}

fn public_on_ip() -> Value {
    with(
        public(),
        json!({ "host": "203.0.113.7", "gameDir": "/srv/dauntless/Dauntless", "mongo": { "uri": DEFAULT_MONGO_URI, "manage": true, "mongod": "/usr/bin/mongod" } }),
    )
}

#[test]
fn the_public_gateways_port_is_443_unless_moved_and_61000_for_a_private_server() {
    assert_eq!(gateway_port(&config(public_on_ip())), 443);
    assert_eq!(gateway_port(&config(moved())), 4443);
    assert_eq!(gateway_port(&config(private())), 61000);
}

#[test]
fn the_public_port_is_checked_a_real_port_not_one_the_server_uses_itself_public_servers_only() {
    assert_eq!(validate(&moved()).unwrap()["publicPort"], 4443);
    for bad in [0, 70000, 61000, 41000, 41005, 41010, 27017, 8777, 8766, 61100] {
        assert!(
            error(with(public_on_ip(), json!({ "publicPort": bad }))).contains("publicPort"),
            "{bad}"
        );
    }
    assert!(
        error(with(
            public_on_ip(),
            json!({ "mode": "private", "host": "100.101.102.103", "gameAddress": "100.101.102.103", "publicPort": 4443 })
        ))
        .contains("public servers only")
    );
}

#[test]
fn the_public_port_reaches_the_gateways_env_file() {
    let p = Paths::new("/home/reforged/Reforged");
    let f = build_env_files(&config(moved()), &fast_secrets(), &p, "", Platform::Linux);
    assert_eq!(get(&f.gateway, "GATEWAY_PORT"), Some("4443"));
}

// ------------------------------------------------------------------ performance.test.ts

fn perf_public() -> Value {
    with(
        public_on_ip(),
        json!({ "name": "Friday Hunts", "gameDir": "/home/reforged/games/Dauntless" }),
    )
}

fn perf_with(changes: Value) -> Value {
    with(
        perf_public(),
        json!({ "performance": with(default_performance(), changes) }),
    )
}

#[test]
fn caps_the_game_servers_at_30_fps_starts_training_on_demand_and_keeps_the_game_logs_quiet() {
    let s = fast_secrets();
    let p = Paths::new("/home/reforged/Reforged");
    for platform in [Platform::Linux, Platform::Windows] {
        let f = build_env_files(&config(perf_public()), &s, &p, "", platform);
        assert_eq!(get(&f.director, "DAINTY_SERVER_MAX_FPS"), Some("30"));
        assert_eq!(get(&f.director, "DAINTY_SERVER_IDLE_FPS"), Some("10"));
        assert_eq!(get(&f.director, "TRAINING_ON_DEMAND"), Some("1"));
        assert_eq!(get(&f.director, "GAMESERVER_LOG_CMDS"), Some(QUIET_GAME_LOG_CMDS));
        assert_eq!(get(&f.director, "GAMESERVER_MEMORY_GUARD"), Some("1"));
        assert_eq!(
            get(&f.director, "TRAINING_IDLE_STOP_MINUTES"),
            Some("5"),
            "on demand, and stopped again after 5 min empty"
        );
        let exe = Path::new("/home/reforged/games/Dauntless").join(game_exe_relative());
        assert_eq!(
            get(&f.director, "GAMESERVER_RUNTIME_LOG_DIR"),
            exe.parent().and_then(Path::to_str)
        );
        assert_eq!(get(&f.director, "GAMESERVER_MIN_FREE_MB"), Some("1024"));
        assert_eq!(get(&f.director, "GAMESERVER_HUNT_MEMORY_MB"), Some("1536"));
    }
}

#[test]
fn keeps_udp_8770_8777_up_to_6_hunts_and_reaches_one_port_further_down_per_hunt_above_that() {
    let s = fast_secrets();
    let p = Paths::new("/home/reforged/Reforged");
    let hunts = |n: u32| config(with(perf_public(), json!({ "maxHuntServers": n })));
    assert_eq!(game_ports(&hunts(3)), (8770, 8777));
    assert_eq!(game_ports(&hunts(6)), (8770, 8777));
    assert_eq!(game_ports(&hunts(8)), (8768, 8777));
    assert_eq!(game_ports(&hunts(10)), (8766, 8777));
    let f = build_env_files(&hunts(8), &s, &p, "", Platform::Linux);
    assert_eq!(
        get(&f.director, "PORT_RANGE_BEGIN"),
        Some("8768"),
        "Ramsgate 8777, Training 8776, eight hunts 8768-8775"
    );
    assert_eq!(get(&f.director, "PORT_RANGE_END"), Some("8777"));
    assert_eq!(get(&f.director, "MAX_CONCURRENT_HUNT_SERVERS"), Some("8"));
    assert_eq!(get(f.allowlist.as_ref().unwrap(), "ALLOWLIST_PORTS"), Some("8768-8777"));
    let three = build_env_files(&config(perf_public()), &s, &p, "", Platform::Linux);
    assert_eq!(
        get(three.allowlist.as_ref().unwrap(), "ALLOWLIST_PORTS"),
        Some("8770-8777")
    );
}

#[test]
fn follows_host_json_no_cap_training_at_boot_upstreams_verbose_logs() {
    let s = fast_secrets();
    let p = Paths::new("/home/reforged/Reforged");
    let c = config(perf_with(json!({
        "serverFps": 0, "idleFps": 0, "trainingOnDemand": false, "verboseGameLogs": true, "memoryGuard": false,
    })));
    let f = build_env_files(&c, &s, &p, "", Platform::Linux);
    assert_eq!(get(&f.director, "DAINTY_SERVER_MAX_FPS"), Some("0"));
    assert_eq!(get(&f.director, "DAINTY_SERVER_IDLE_FPS"), Some("0"));
    assert_eq!(get(&f.director, "TRAINING_ON_DEMAND"), Some("0"));
    assert_eq!(
        get(&f.director, "GAMESERVER_LOG_CMDS"),
        None,
        "the Director keeps its own default"
    );
    assert_eq!(get(&f.director, "GAMESERVER_MEMORY_GUARD"), Some("0"));
    assert_eq!(
        get(&f.director, "TRAINING_IDLE_STOP_MINUTES"),
        Some("0"),
        "at boot: never stopped"
    );
    let no_game = build_env_files(
        &config(with(perf_public(), json!({ "gameDir": null }))),
        &s,
        &p,
        "",
        Platform::Linux,
    );
    assert_eq!(
        get(&no_game.director, "GAMESERVER_RUNTIME_LOG_DIR"),
        None,
        "no game folder yet"
    );
}

#[test]
fn writes_only_values_the_director_accepts() {
    // The Director's check for GAMESERVER_LOG_CMDS (patch 0021).
    assert!(QUIET_GAME_LOG_CMDS.len() <= 512);
    assert!(
        QUIET_GAME_LOG_CMDS
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_, ".contains(c))
    );
}

#[test]
fn refuses_frame_rates_and_switches_the_runtime_would_not_take() {
    for (changes, name) in [
        (json!({ "serverFps": 5 }), "serverFps"),
        (json!({ "serverFps": 241 }), "serverFps"),
        (json!({ "serverFps": 29.5 }), "serverFps"),
        (json!({ "idleFps": -1 }), "idleFps"),
        (json!({ "idleFps": 300 }), "idleFps"),
        (json!({ "trainingOnDemand": "yes" }), "trainingOnDemand"),
        (json!({ "huntMemoryMb": 100 }), "huntMemoryMb"),
        (json!({ "minFreeMemoryMb": -1 }), "minFreeMemoryMb"),
        (json!({ "trainingIdleMinutes": 1441 }), "trainingIdleMinutes"),
    ] {
        let message = error(perf_with(changes.clone()));
        assert!(message.contains(name), "{changes}: {message}");
    }
    // A host.json written before the memory guard existed: the defaults fill the new fields.
    let older = with(
        perf_public(),
        json!({ "performance": { "serverFps": 30, "idleFps": 10, "trainingOnDemand": true, "verboseGameLogs": false } }),
    );
    let f = build_env_files(
        &config(older),
        &fast_secrets(),
        &Paths::new("/home/reforged/Reforged"),
        "",
        Platform::Linux,
    );
    assert_eq!(get(&f.director, "GAMESERVER_MEMORY_GUARD"), Some("1"));
    assert_eq!(
        validate(&perf_with(json!({ "idleFps": 1 }))).unwrap()["performance"]["idleFps"],
        1
    );
}

// ------------------------------------------------------------------ linux.test.ts

fn linux_public() -> Value {
    with(
        public_on_ip(),
        json!({
            "name": "Freeza Hunts",
            "linux": { "wine": null, "proton": null, "xvfb": true, "firewallInterface": "eth0" },
        }),
    )
}

#[test]
fn linux_options_are_checked() {
    assert_eq!(validate(&linux_public()).unwrap()["linux"]["firewallInterface"], "eth0");
    let lx = |changes: Value| {
        with(
            linux_public(),
            json!({ "linux": with(json!({ "wine": null, "proton": null, "xvfb": true, "firewallInterface": "eth0" }), changes) }),
        )
    };
    assert!(error(lx(json!({ "wine": "wine" }))).contains("linux.wine"));
    assert!(error(lx(json!({ "firewallInterface": "a b" }))).contains("firewallInterface"));
}

#[test]
fn linux_the_director_starts_the_wine_launcher_with_the_executable_the_prefix_and_a_longer_timeout() {
    let s = fast_secrets();
    let p = Paths::new("/home/reforged/Reforged");
    let f = build_env_files(&config(linux_public()), &s, &p, "", Platform::Linux);
    let exe = Path::new("/srv/dauntless/Dauntless").join(game_exe_relative());
    assert_eq!(
        get(&f.director, "GAMESERVER_BINARY_PATH"),
        p.gameserver_wrapper.to_str()
    );
    assert_eq!(get(&f.director, "DAINTY_GAME_EXE"), exe.to_str());
    assert_eq!(get(&f.director, "DAINTY_WINEPREFIX"), p.wine_prefix.to_str());
    assert_eq!(get(&f.director, "DAINTY_XVFB"), Some("1"));
    assert_eq!(get(&f.director, "DAINTY_WINE"), None, "found on PATH when not set");
    assert_eq!(get(&f.director, "GAMESERVER_READY_TIMEOUT_MS"), Some("90000"));
    let allowlist = f.allowlist.unwrap();
    assert_eq!(get(&allowlist, "ALLOWLIST_DRY_RUN"), Some("0"));
    assert_eq!(get(&allowlist, "ALLOWLIST_FIREWALL"), Some("nftables"));
    assert_eq!(get(&allowlist, "ALLOWLIST_NFT_TABLE"), Some("dainty_lass"));
    assert_eq!(get(&allowlist, "ALLOWLIST_INTERFACE"), Some("eth0"));
    let wine = config(with(
        linux_public(),
        json!({ "linux": { "wine": "/usr/bin/wine64", "proton": null, "xvfb": false, "firewallInterface": "eth0" } }),
    ));
    let f = build_env_files(&wine, &s, &p, "", Platform::Linux);
    assert_eq!(get(&f.director, "DAINTY_WINE"), Some("/usr/bin/wine64"));
    assert_eq!(get(&f.director, "DAINTY_XVFB"), Some("0"));
}

#[test]
fn windows_keeps_starting_the_executable_itself() {
    let p = Paths::new("/home/reforged/Reforged");
    let f = build_env_files(&config(linux_public()), &fast_secrets(), &p, "", Platform::Windows);
    let exe = Path::new("/srv/dauntless/Dauntless").join(game_exe_relative());
    assert_eq!(get(&f.director, "GAMESERVER_BINARY_PATH"), exe.to_str());
    assert_eq!(get(&f.director, "DAINTY_GAME_EXE"), None);
    assert_eq!(get(&f.director, "GAMESERVER_READY_TIMEOUT_MS"), Some("30000"));
    assert_eq!(get(f.allowlist.as_ref().unwrap(), "ALLOWLIST_INTERFACE"), None);
}

// ------------------------------------------------------------------ node links (DN3)

const NODES: &str = "\
version = 1
mode = \"public\"
name = \"Oregon\"
host = \"hunts.example.net\"
game_address = \"203.0.113.7\"
registration = \"INVITECODE\"
max_hunt_servers = 3

[mongo]
uri = \"mongodb://127.0.0.1:27017\"
manage = true

# Mine
[network]
node = \"oregon-1\"
region = \"oregon\"

[[network.nodes]]
id = \"uk-1\"
region = \"uk\"
host = \"uk.example.org\"
port = 443
fingerprint = \"abababababababababababababababababababababababababababababababab\"

[[network.nodes]]
id = \"de-1\"
region = \"germany\"
host = \"198.51.100.4\"
port = 8443
";

fn read_toml(text: &str) -> HostConfig {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    std::fs::write(&file, text).unwrap();
    read_host_config(&file).unwrap().unwrap()
}

#[test]
fn the_peers_round_trip_through_settings_toml_and_a_new_file() {
    let config = read_toml(NODES);
    let network = config.network.clone().unwrap();
    assert_eq!(
        network
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.port))
            .collect::<Vec<_>>(),
        [("uk-1", 443), ("de-1", 8443)]
    );
    // Written back as read, comments and all, and changed in place
    let (back, text) = through_toml(&config);
    assert_eq!(back.network, config.network);
    assert!(text.contains("[[network.nodes]]"), "{text}");
    let mut more = config.clone();
    more.network.as_mut().unwrap().nodes.pop();
    let (back, text) = through_toml(&more);
    assert_eq!(back.network.unwrap().nodes.len(), 1);
    assert!(text.contains("# Mine"), "{text}");
    // A file made from host.json says what the list is
    let fresh = HostConfig::from_json(&config.to_json().unwrap()).unwrap();
    let (back, text) = through_toml(&fresh);
    assert_eq!(back.network, config.network);
    assert!(text.contains("The other nodes of the network"), "{text}");
    // A server with a network and no peers writes no list: today's server
    let mut alone = config.clone();
    alone.network.as_mut().unwrap().nodes.clear();
    let (back, text) = through_toml(&alone);
    assert!(
        !text.contains("network.nodes") && back.network.unwrap().nodes.is_empty(),
        "{text}"
    );
}

#[test]
fn the_peers_are_checked() {
    for (bad, said) in [
        (
            "id = \"uk-1\"\nregion = \"uk\"\nhost = \"UK.example.org\"\nport = 443\n",
            "host",
        ),
        (
            "id = \"uk-1\"\nregion = \"uk\"\nhost = \"a.example\"\nport = 0\n",
            "port",
        ),
        (
            "id = \"uk-1\"\nregion = \"uk\"\nhost = \"a.example\"\nport = 443\nfingerprint = \"AB\"\n",
            "fingerprint",
        ),
        (
            "id = \"oregon-1\"\nregion = \"oregon\"\nhost = \"a.example\"\nport = 443\n",
            "itself",
        ),
        (
            "id = \"uk-1\"\nregion = \"uk\"\nhost = \"a.example\"\nport = 443\nkey = 1\n",
            "unknown field",
        ),
        ("id = \"uk-1\"\nregion = \"uk\"\nhost = \"a.example\"\n", "port"),
    ] {
        let text = NODES.split("[[network.nodes]]").next().unwrap().to_owned() + "[[network.nodes]]\n" + bad;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        std::fs::write(&file, text).unwrap();
        let error = read_host_config(&file).unwrap_err().to_string();
        assert!(error.contains(said), "{bad:?}: {error}");
    }
    // Twice, and over the limit
    let twice =
        format!("{NODES}\n[[network.nodes]]\nid = \"uk-1\"\nregion = \"uk\"\nhost = \"a.example\"\nport = 443\n");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.toml"), twice).unwrap();
    assert!(
        read_host_config(&dir.path().join("settings.toml"))
            .unwrap_err()
            .to_string()
            .contains("twice")
    );
    // Peers need the TLS port
    let mut json = read_toml(NODES).to_json().unwrap();
    json["mode"] = json!("private");
    json["host"] = json!("100.101.102.103");
    json["gameAddress"] = json!("100.101.102.103");
    assert!(error(json).contains("mode = \"public\""));
}

#[test]
fn the_gateway_reads_back_the_peers_setup_wrote() {
    let config = read_toml(NODES);
    let f = build_env_files(&config, &fast_secrets(), &win_paths(), "", Platform::Windows);
    assert_eq!(get(&f.gateway, "GATEWAY_NODE_ID"), Some("oregon-1"));
    assert_eq!(get(&f.gateway, "GATEWAY_NODE_REGION"), Some("oregon"));
    for (key, file) in [
        ("GATEWAY_NODE_CERT", "node-cert.pem"),
        ("GATEWAY_NODE_KEY", "node-key.pem"),
        ("GATEWAY_NODE_CA", "cluster-ca-cert.pem"),
    ] {
        assert!(get(&f.gateway, key).is_some_and(|path| path.ends_with(file)), "{key}");
    }
    // Through the file setup writes, as `gateway serve --env-file gateway.env` reads it
    let text = dainty_host::envfile::format_env(&[], &f.gateway).unwrap();
    let read = dainty_host::envfile::parse_env(&text);
    let env = dainty_service_kit::env::Env::from_pairs(read.iter());
    let gateway = dainty_gateway::config::Config::from_env(&env).unwrap();
    let links = gateway.node_links.unwrap();
    assert_eq!(links.identity.node.as_str(), "oregon-1");
    assert_eq!(links.peers, config.network.unwrap().nodes);
    // Only the peer with a fingerprint is listed to the launcher
    assert_eq!(gateway.network.len(), 1);
    assert_eq!(gateway.network[0].host, "uk.example.org");
}

#[test]
fn a_server_without_peers_writes_no_node_settings() {
    for doc in [
        public(),
        with(public(), json!({"network": {"node": "oregon-1", "region": "oregon"}})),
    ] {
        let f = build_env_files(&config(doc), &fast_secrets(), &win_paths(), "", Platform::Windows);
        assert!(
            f.gateway
                .iter()
                .all(|(key, _)| !key.starts_with("GATEWAY_NODE") && key != "GATEWAY_NETWORK_NODES")
        );
    }
}

#[test]
fn switches_round_trip_through_settings_toml_setup_and_backend_env() {
    let toml_text = format!(
        "{NODES}\n[switches]\nreforge = \"log\"\nmastery_persistence = \"off\"\nbounty_persistence = \"off\"\ncooldown_persistence = \"off\"\naccount_erasure = \"off\"\nlog_purge = \"off\"\nrate_limit = \"off\"\nrate_limit_per_minute = 300\ngameserver_rate_limit_per_minute = 4500\nhost_rate_limit_per_minute = 5000\n"
    );
    let parsed = read_toml(&toml_text);
    let switches = parsed.switches.as_ref().expect("switches parsed");
    assert_eq!(switches.reforge.as_deref(), Some("log"));
    assert_eq!(switches.mastery_persistence.as_deref(), Some("off"));
    assert_eq!(switches.bounty_persistence.as_deref(), Some("off"));
    assert_eq!(switches.cooldown_persistence.as_deref(), Some("off"));
    assert_eq!(switches.account_erasure.as_deref(), Some("off"));
    assert_eq!(switches.log_purge.as_deref(), Some("off"));
    assert_eq!(switches.rate_limit.as_deref(), Some("off"));
    assert_eq!(switches.rate_limit_per_minute, Some(300));
    assert_eq!(switches.gameserver_rate_limit_per_minute, Some(4500));
    assert_eq!(switches.host_rate_limit_per_minute, Some(5000));

    let (back, out_toml) = through_toml(&parsed);
    assert_eq!(back.switches, parsed.switches);
    assert!(out_toml.contains("[switches]"), "{out_toml}");
    assert!(out_toml.contains("bounty_persistence = \"off\""), "{out_toml}");
    assert!(out_toml.contains("cooldown_persistence = \"off\""), "{out_toml}");
    assert!(out_toml.contains("rate_limit = \"off\""), "{out_toml}");
    assert!(out_toml.contains("rate_limit_per_minute = 300"), "{out_toml}");
    assert!(
        out_toml.contains("gameserver_rate_limit_per_minute = 4500"),
        "{out_toml}"
    );
    assert!(out_toml.contains("host_rate_limit_per_minute = 5000"), "{out_toml}");

    let f = build_env_files(&back, &fast_secrets(), &win_paths(), "", Platform::Windows);
    let text = dainty_host::envfile::format_env(&[], &f.backend).unwrap();
    let read = dainty_host::envfile::parse_env(&text);
    assert_eq!(read.get("RUSTIC_ISLES_REFORGE"), Some("log"));
    assert_eq!(read.get("RUSTIC_ISLES_MASTERY_PERSISTENCE"), Some("off"));
    assert_eq!(read.get("RUSTIC_ISLES_BOUNTY_PERSISTENCE"), Some("off"));
    assert_eq!(read.get("RUSTIC_ISLES_COOLDOWN_PERSISTENCE"), Some("off"));
    assert_eq!(read.get("RUSTIC_ISLES_ACCOUNT_ERASURE"), Some("off"));
    assert_eq!(read.get("RUSTIC_ISLES_LOG_PURGE"), Some("off"));
    assert_eq!(read.get("RUSTIC_ISLES_RATE_LIMIT"), Some("off"));
    assert_eq!(read.get("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE"), Some("300"));
    assert_eq!(read.get("RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE"), Some("4500"));
    assert_eq!(read.get("RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE"), Some("5000"));

    // Absent switches leave the rate-limit env vars unset so the backend's own defaults (on, 600, 6000, 6000) apply.
    let default_files = build_env_files(&config(public()), &fast_secrets(), &win_paths(), "", Platform::Windows);
    let default_read =
        dainty_host::envfile::parse_env(&dainty_host::envfile::format_env(&[], &default_files.backend).unwrap());
    assert_eq!(default_read.get("RUSTIC_ISLES_RATE_LIMIT"), None);
    assert_eq!(default_read.get("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE"), None);
    assert_eq!(default_read.get("RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE"), None);
    assert_eq!(default_read.get("RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE"), None);
    for mongo_key in [
        "MONGODB_URI",
        "MONGODB_DB",
        "MONGODB_APP_NAME",
        "MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS",
    ] {
        assert_eq!(
            default_read.get(mongo_key).is_some(),
            cfg!(feature = "paradox"),
            "{mongo_key} should be written to backend.env only on a paradox build"
        );
    }

    let bad_doc = with(public(), json!({"switches": {"bountyPersistence": "maybe"}}));
    assert!(error(bad_doc).contains("switches.bountyPersistence"));
    let bad_rate = with(public(), json!({"switches": {"rateLimitPerMinute": 0}}));
    assert!(error(bad_rate).contains("switches.rateLimitPerMinute"));
}
