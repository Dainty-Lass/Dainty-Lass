//! `dainty-host install-game` (DE6) against a double for Epic's account service and library and a fake CDN
//! (`dainty_epic::testing`): three HTTPS hosts on 127.0.0.1:0 with the tests' own CA. Nothing reaches Epic, and no real
//! account or game file is involved. The fresh Linux host that installs for real is the maintainer's check.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dainty_epic::auth::{Accounts, AuthError, Grant, Pending, Session, Token};
use dainty_epic::download::{Control, Executable, Options};
use dainty_epic::library::{Library, LibraryError, Record};
use dainty_epic::manifest::InstallPath;
use dainty_epic::testing::{Cdn, EXE, Host, Version, cdn, version};
use dainty_host::cli::InstallGameArgs;
use dainty_host::commands::install_game::{Configured, Deps, configure, run_with};
use dainty_host::envfile::parse_env;
use dainty_host::epicinstall::{self, Files, Plan, load_session, save_session};
use dainty_host::error::HostError;
use dainty_host::hostconfig::{
    DEFAULT_MONGO_URI, HostConfig, Platform, Secrets, build_env_files, load_host_config, read_host_config,
    write_env_files, write_host_config,
};
use dainty_host::paths::{Paths, game_exe_relative};
use serde_json::json;
use sha2::{Digest, Sha256};

const CODE: &str = "0123456789abcdef0123456789abcdef";
const ACCESS: &str = "eg1~access-token-from-the-double";
const REFRESH: &str = "eg1~refresh-token-from-the-double";

/// Epic's account service: hands out a session for the code `CODE`, turns the refresh token over, and says what it
/// was asked.
#[derive(Default)]
struct Epic {
    seen: Mutex<Vec<String>>,
    /// Refreshing and ending a session fail as if Epic were down
    down: bool,
}

fn session(access: &str, refresh: &str, access_expires: i64) -> Session {
    Session::new(
        "8645b4947bbc4c0092a8b7236df169d1".into(),
        "Slayer".into(),
        Token::new(access.into()),
        access_expires,
        Token::new(refresh.into()),
        4_000_000_000,
    )
}

impl Accounts for Epic {
    fn token(&self, grant: Grant) -> Pending<'_, Session> {
        let (what, result) = match grant {
            Grant::Code(code) if code.expose() == CODE => {
                ("code".to_owned(), Ok(session(ACCESS, REFRESH, 4_000_000_000)))
            }
            Grant::Code(_) => ("code".to_owned(), Err(AuthError::Rejected)),
            Grant::Refresh(_) if self.down => ("refresh".to_owned(), Err(AuthError::Unreachable)),
            Grant::Refresh(_) => (
                "refresh".to_owned(),
                Ok(session("eg1~access-2", "eg1~refresh-2", 4_000_000_000)),
            ),
        };
        self.seen.lock().unwrap().push(what);
        Box::pin(async move { result })
    }

    fn end_session(&self, _: Token) -> Pending<'_, ()> {
        self.seen.lock().unwrap().push("end".into());
        let down = self.down;
        Box::pin(async move { if down { Err(AuthError::Unreachable) } else { Ok(()) } })
    }
}

/// The account's library: these app names, or unreadable.
struct Owns(Option<Vec<&'static str>>);

impl Library for Owns {
    async fn records(&self, _: &Session) -> Result<Vec<Record>, LibraryError> {
        let names = self.0.as_ref().ok_or(LibraryError::Unreachable)?;
        Ok(names
            .iter()
            .map(|app_name| Record {
                app_name: (*app_name).to_owned(),
                ..Default::default()
            })
            .collect())
    }
}

fn owner() -> Owns {
    Owns(Some(vec!["Fortnite", "Jackal"]))
}

struct Fixture {
    _dir: tempfile::TempDir,
    paths: Paths,
    v: Version,
    plan: Plan,
    cdn: Cdn,
    install_dir: PathBuf,
}

fn plan_for(v: &Version, cdn: &Cdn) -> Plan {
    let exe_sha: [u8; 32] = Sha256::digest(v.build.file_data(&v.build.files[1])).into();
    // The fake CDN serves the manifest beside the chunks, on its first host
    let root = cdn.bases[0].split("/Builds").next().unwrap();
    Plan {
        version: "1.14.7".into(),
        display_name: "Test build".into(),
        build_version: "1.14.7.647472".into(),
        pin: v.pin,
        sources: vec![format!("{root}/m/good.manifest")],
        executable: Executable {
            path: InstallPath::new(EXE).unwrap(),
            sha256: exe_sha,
        },
    }
}

async fn fixture(hosts: &[Host]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("Reforged"));
    let v = version();
    let cdn = cdn(hosts, v.files.clone()).await;
    let plan = plan_for(&v, &cdn);
    let install_dir = dir.path().join("games");
    Fixture {
        _dir: dir,
        paths,
        v,
        plan,
        cdn,
        install_dir,
    }
}

fn config(extra: serde_json::Value) -> HostConfig {
    let mut doc = json!({
        "version": 1, "mode": "private", "host": "100.101.102.103", "gameAddress": "100.101.102.103",
        "name": "Friday Hunts", "gameDir": null, "registration": "INVITECODE", "shareUrl": null,
        "mongo": { "uri": DEFAULT_MONGO_URI, "manage": true, "mongod": null }, "maxHuntServers": 3,
    });
    for (k, v) in extra.as_object().unwrap() {
        doc[k] = v.clone();
    }
    HostConfig::from_json(&doc).unwrap()
}

fn secrets() -> Secrets {
    let hex = |i: u32| format!("{i:064x}");
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

/// A host that ran setup: host.toml and the env files.
fn set_up(f: &Fixture, extra: serde_json::Value) {
    let c = config(extra);
    write_host_config(&f.paths.host_toml, &c).unwrap();
    write_env_files(
        &f.paths,
        &build_env_files(&c, &secrets(), &f.paths, "", Platform::Linux),
    )
    .unwrap();
}

fn options(f: &Fixture) -> Options {
    let mut options = Options::new(Arc::new(|_: &Path| Ok(u64::MAX)));
    options.bases = f.cdn.bases.clone();
    options.tls = Some(f.cdn.tls.clone());
    options.backoff = Arc::new(|_| Duration::ZERO);
    options.parallel = 3;
    options.timeout = Duration::from_secs(10);
    options
}

fn args(f: &Fixture, more: &[&str]) -> InstallGameArgs {
    InstallGameArgs {
        dir: Some(f.install_dir.to_string_lossy().into_owned()),
        code_file: more.contains(&"code").then(|| code_file(f)),
        sign_out: more.contains(&"sign-out"),
        version: more.iter().find_map(|m| m.strip_prefix("v=")).map(str::to_owned),
    }
}

fn code_file(f: &Fixture) -> String {
    let file = f._dir.path().join("code.txt");
    std::fs::write(&file, format!("{CODE}\n")).unwrap();
    file.to_string_lossy().into_owned()
}

struct Ran {
    result: Result<ExitCode, HostError>,
    lines: Vec<String>,
}

impl Ran {
    fn ok(&self) -> &Self {
        assert!(self.result.is_ok(), "{:?} {:?}", self.result, self.lines);
        self
    }

    fn error(&self) -> String {
        self.result.as_ref().unwrap_err().to_string()
    }
}

/// Runs the command once, the way the real one does, with these doubles. A code asked for on stdin is `stdin`.
async fn run(f: &Fixture, epic: &Epic, library: Owns, args: &InstallGameArgs, stdin: &str) -> Ran {
    let mut lines = Vec::new();
    let commit = |_: &Path| Ok(String::new());
    let plan = |_: &str| Ok(f.plan.clone());
    let mut input = stdin.as_bytes();
    let deps = Deps {
        accounts: epic,
        library,
        options: options(f),
        platform: Platform::Linux,
        source_commit: &commit,
        plan: &plan,
    };
    let result = run_with(&f.paths, args, deps, &Control::default(), &mut input, &mut |l| {
        lines.push(l.to_owned())
    })
    .await;
    Ran { result, lines }
}

fn tree(at: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(at) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            tree(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn assert_installed(f: &Fixture, folder: &Path) {
    for test in &f.v.build.files {
        let on_disk = std::fs::read(folder.join(&test.name)).unwrap();
        assert_eq!(on_disk, f.v.build.file_data(test), "{}", test.name);
    }
}

fn leaks(f: &Fixture, lines: &[String], secrets: &[&str]) {
    let all = lines.join("\n");
    for secret in secrets {
        assert!(!all.contains(secret), "{secret} in the output: {all}");
    }
    // And no file but the session file has them
    let mut files = Vec::new();
    tree(&f.paths.workspace, &mut files);
    for file in files {
        if file.ends_with(epicinstall::SESSION_FILE) || file.starts_with(&f.install_dir) {
            continue;
        }
        let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
        for secret in secrets {
            assert!(!text.contains(secret), "{secret} in {}", file.display());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_host_signs_in_with_a_pasted_code_and_installs_for_its_game_servers() {
    let f = fixture(&[Host::Good, Host::Corrupt, Host::Redirect]).await;
    set_up(&f, json!({}));
    let epic = Epic::default();
    let ran = run(&f, &epic, owner(), &args(&f, &[]), &format!("{CODE}\n")).await;
    ran.ok();

    let folder = f.install_dir.join("1.14.7");
    assert_installed(&f, &folder);
    assert!(
        ran.lines.iter().any(|l| l.contains("authorizationCode")),
        "the login steps: {:?}",
        ran.lines
    );
    assert!(
        ran.lines
            .iter()
            .any(|l| l.contains("https://www.epicgames.com/id/login"))
    );
    assert!(ran.lines.iter().any(|l| l.contains("Installing Test build")));
    assert!(
        ran.lines.iter().any(|l| l.trim_start().starts_with("100%")),
        "progress: {:?}",
        ran.lines
    );
    assert_eq!(*epic.seen.lock().unwrap(), ["code"]);

    // host.toml and the Director's env file follow
    let c = load_host_config(&f.paths).unwrap().unwrap();
    assert_eq!(c.game_dir.as_deref(), Some(folder.to_string_lossy().as_ref()));
    let director = parse_env(&std::fs::read_to_string(f.paths.config.join("director.env")).unwrap());
    assert_eq!(
        director.get("DAINTY_GAME_EXE"),
        Some(folder.join(game_exe_relative()).to_string_lossy().as_ref())
    );
    assert!(ran.lines.iter().any(|l| l.contains("game_dir =")), "{:?}", ran.lines);

    // The session is kept, owner-only, and is in no other file, flag or line of output
    let stored = load_session(&Files::of(&f.paths).session).unwrap().unwrap();
    assert_eq!(stored.refresh_token().expose(), REFRESH);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&Files::of(&f.paths).session)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    leaks(&f, &ran.lines, &[CODE, ACCESS, REFRESH]);
    assert!(
        !std::fs::read_to_string(&f.paths.host_toml).unwrap().contains("epic"),
        "host.toml holds no session"
    );
    // Nothing to resume once finished
    assert!(!Files::of(&f.paths).resume_file("1.14.7").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_second_run_needs_no_code_and_fetches_nothing() {
    let f = fixture(&[Host::Good]).await;
    set_up(&f, json!({}));
    let epic = Epic::default();
    run(&f, &epic, owner(), &args(&f, &["code"]), "").await.ok();
    let fetched = f.cdn.requests(0).len();
    assert!(fetched > 0);

    // No stdin at all: a stored session is used
    let again = run(&f, &epic, owner(), &args(&f, &[]), "").await;
    again.ok();
    assert!(
        again.lines.iter().any(|l| l.contains("Signed in to Epic as Slayer")),
        "{:?}",
        again.lines
    );
    assert!(
        again.lines.iter().any(|l| l.contains("already installed")),
        "{:?}",
        again.lines
    );
    assert_eq!(f.cdn.requests(0).len(), fetched);
    assert_eq!(*epic.seen.lock().unwrap(), ["code"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_without_dauntless_gets_nothing_downloaded_or_created() {
    let f = fixture(&[Host::Good]).await;
    set_up(&f, json!({}));
    let epic = Epic::default();
    // A library that cannot be read refuses (and is never cached)
    let unreadable = run(&f, &epic, Owns(None), &args(&f, &["code"]), "").await;
    assert!(
        unreadable.error().contains("could not be read"),
        "{}",
        unreadable.error()
    );
    assert!(f.cdn.requests(0).is_empty() && !f.install_dir.exists());
    // An account whose library lacks it is refused
    let ran = run(&f, &epic, Owns(Some(vec!["Fortnite"])), &args(&f, &["code"]), "").await;
    assert!(ran.error().contains("does not own Dauntless"), "{}", ran.error());
    assert!(f.cdn.requests(0).is_empty(), "no manifest, no chunk");
    assert!(!f.install_dir.exists(), "no folder");
    assert_eq!(load_host_config(&f.paths).unwrap().unwrap().game_dir, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_code_signs_nobody_in() {
    let f = fixture(&[Host::Good]).await;
    let epic = Epic::default();
    let ran = run(&f, &epic, owner(), &args(&f, &[]), "deadbeef\n").await;
    assert!(ran.error().contains("Epic refused the sign-in"), "{}", ran.error());
    assert!(!Files::of(&f.paths).session.exists());
    let bad = run(&f, &epic, owner(), &args(&f, &[]), "not a code!\n").await;
    assert!(
        bad.error().contains("not an Epic authorization code"),
        "{}",
        bad.error()
    );
    assert_eq!(*epic.seen.lock().unwrap(), ["code"], "the second never left the host");
    let closed = run(&f, &epic, owner(), &args(&f, &[]), "").await;
    assert!(closed.error().contains("no code"), "{}", closed.error());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_access_token_is_refreshed_and_the_new_refresh_token_stored() {
    let f = fixture(&[Host::Good]).await;
    let files = Files::of(&f.paths);
    save_session(&files.session, &session("eg1~stale-access", "eg1~old-refresh", 1_000)).unwrap();
    let epic = Epic::default();
    // No code is asked for: stdin is empty
    run(&f, &epic, owner(), &args(&f, &[]), "").await.ok();
    assert_eq!(*epic.seen.lock().unwrap(), ["refresh"]);
    let stored = load_session(&files.session).unwrap().unwrap();
    assert_eq!(stored.refresh_token().expose(), "eg1~refresh-2");
    assert_eq!(stored.access_token().expose(), "eg1~access-2");
}

#[tokio::test(flavor = "multi_thread")]
async fn epic_being_down_does_not_ask_for_another_code() {
    let f = fixture(&[Host::Good]).await;
    let files = Files::of(&f.paths);
    save_session(&files.session, &session("a", "r", 1_000)).unwrap();
    let epic = Epic {
        down: true,
        ..Epic::default()
    };
    let ran = run(&f, &epic, owner(), &args(&f, &[]), "").await;
    assert!(ran.error().contains("could not be reached"), "{}", ran.error());
    assert!(files.session.exists(), "the session is kept for the next try");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_killed_halfway_resumes_without_fetching_again() {
    let f = fixture(&[Host::Stall(3)]).await;
    set_up(&f, json!({}));
    let epic = Epic::default();
    let first = tokio::time::timeout(
        Duration::from_secs(4),
        run(&f, &epic, owner(), &args(&f, &["code"]), ""),
    )
    .await;
    assert!(first.is_err(), "the fourth chunk never comes");
    let files = Files::of(&f.paths);
    let recorded = std::fs::read_to_string(files.resume_file("1.14.7")).unwrap();
    assert!(
        recorded.lines().filter(|l| l.starts_with("c ")).count() >= 1,
        "{recorded}"
    );
    assert_eq!(
        load_host_config(&f.paths).unwrap().unwrap().game_dir,
        None,
        "not installed yet"
    );

    // The same command on a CDN that works: the session is reused and only what is missing is fetched
    let second = fixture_cdn(&f, &[Host::Good]).await;
    let ran = run(&second, &epic, owner(), &args(&f, &[]), "").await;
    ran.ok();
    assert_installed(&f, &f.install_dir.join("1.14.7"));
    let done = recorded.lines().filter(|l| l.starts_with("c ")).count();
    let asked: HashSet<String> = second
        .cdn
        .requests(0)
        .into_iter()
        .filter(|p| p.ends_with(".chunk"))
        .collect();
    assert_eq!(
        asked.len(),
        6 - done,
        "only the chunks the first run did not finish: {asked:?}"
    );
    assert!(asked.len() < 6 && done >= 1);
    assert_eq!(*epic.seen.lock().unwrap(), ["code"], "one sign-in for both runs");
}

/// The same host and folders, a different CDN.
async fn fixture_cdn(f: &Fixture, hosts: &[Host]) -> Fixture {
    let cdn = cdn(hosts, f.v.files.clone()).await;
    let plan = plan_for(&f.v, &cdn);
    Fixture {
        _dir: tempfile::tempdir().unwrap(),
        paths: f.paths.clone(),
        v: version(),
        plan,
        cdn,
        install_dir: f.install_dir.clone(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_out_ends_and_deletes_the_session_even_when_epic_is_down() {
    let f = fixture(&[Host::Good]).await;
    let files = Files::of(&f.paths);
    let epic = Epic::default();
    run(&f, &epic, owner(), &args(&f, &["code"]), "").await.ok();
    assert!(files.session.exists());
    let out = run(&f, &epic, owner(), &args(&f, &["sign-out"]), "").await;
    out.ok();
    assert!(!files.session.exists());
    assert!(
        !files.data.join(dainty_epic::library::CACHE_FILE).exists(),
        "the ownership answer goes too"
    );
    assert_eq!(epic.seen.lock().unwrap().last().map(String::as_str), Some("end"));

    save_session(&files.session, &session("a", "r", 4_000_000_000)).unwrap();
    let down = Epic {
        down: true,
        ..Epic::default()
    };
    let out = run(&f, &down, owner(), &args(&f, &["sign-out"]), "").await;
    out.ok();
    assert!(!files.session.exists());
    assert!(out.lines.iter().any(|l| l.contains("deleted")), "{:?}", out.lines);
    let none = run(&f, &down, owner(), &args(&f, &["sign-out"]), "").await;
    assert!(
        none.lines.iter().any(|l| l.contains("No Epic session")),
        "{:?}",
        none.lines
    );
}

#[test]
fn host_toml_follows_only_for_the_version_the_game_servers_run_on_a_server_that_runs_it() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("Reforged"));
    let commit = |_: &Path| Ok(String::new());
    let folder = "/srv/game/1.14.7";

    // No host.toml yet: setup is told the folder
    assert_eq!(
        configure(&paths, folder, Platform::Linux, &commit).unwrap(),
        Configured::NoSettings
    );
    assert!(!paths.host_toml.exists(), "nothing is made");

    // host.toml but no env files (setup not finished): the file only
    let c = config(json!({ "gameDir": "/old/Dauntless" }));
    write_host_config(&paths.host_toml, &c).unwrap();
    assert_eq!(
        configure(&paths, folder, Platform::Linux, &commit).unwrap(),
        Configured::Updated {
            previous: Some("/old/Dauntless".into()),
            env_files: false
        }
    );
    assert_eq!(
        read_host_config(&paths.host_toml).unwrap().unwrap().game_dir.as_deref(),
        Some(folder)
    );
    assert_eq!(
        configure(&paths, folder, Platform::Linux, &commit).unwrap(),
        Configured::Unchanged
    );
}

#[test]
fn the_host_keeps_the_hosts_own_comments_in_host_toml() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("Reforged"));
    let commit = |_: &Path| Ok(String::new());
    write_host_config(&paths.host_toml, &config(json!({}))).unwrap();
    let edited = std::fs::read_to_string(&paths.host_toml).unwrap().replace(
        "max_hunt_servers = 3\n",
        "# two hunts are plenty on this PC\nmax_hunt_servers = 2 # was 3\n",
    );
    std::fs::write(&paths.host_toml, &edited).unwrap();
    configure(&paths, "/srv/game/1.14.7", Platform::Linux, &commit).unwrap();
    let after = std::fs::read_to_string(&paths.host_toml).unwrap();
    assert!(
        after.contains("# two hunts are plenty on this PC\nmax_hunt_servers = 2 # was 3\n"),
        "{after}"
    );
    assert!(after.contains("game_dir = \"/srv/game/1.14.7\""), "{after}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_command_refuses_what_it_cannot_do_before_asking_for_anything() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("Reforged"));
    let epic = Epic::default();
    let commit = |_: &Path| Ok(String::new());
    let go = |args: InstallGameArgs| {
        let (paths, epic, commit) = (&paths, &epic, &commit);
        async move {
            let mut input: &[u8] = b"";
            let mut lines = Vec::new();
            let deps = Deps {
                accounts: epic,
                library: owner(),
                options: Options::new(Arc::new(|_: &Path| Ok(u64::MAX))),
                platform: Platform::Linux,
                source_commit: commit,
                plan: &Plan::for_version,
            };
            run_with(paths, &args, deps, &Control::default(), &mut input, &mut |l| {
                lines.push(l.to_owned())
            })
            .await
            .unwrap_err()
            .to_string()
        }
    };
    let unknown = go(InstallGameArgs {
        version: Some("9.9.9".into()),
        ..InstallGameArgs::default()
    })
    .await;
    assert!(unknown.contains("the versions are 1.14.7"), "{unknown}");
    let relative = go(InstallGameArgs {
        dir: Some("games".into()),
        ..InstallGameArgs::default()
    })
    .await;
    assert!(relative.contains("full path"), "{relative}");
    assert!(epic.seen.lock().unwrap().is_empty(), "Epic was never asked");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_executable_the_profile_pins_must_be_the_one_downloaded() {
    let mut f = fixture(&[Host::Good]).await;
    f.plan.executable.sha256 = [7; 32];
    set_up(&f, json!({}));
    let ran = run(&f, &Epic::default(), owner(), &args(&f, &["code"]), "").await;
    assert!(ran.error().contains("not the one this version pins"), "{}", ran.error());
    assert_eq!(
        load_host_config(&f.paths).unwrap().unwrap().game_dir,
        None,
        "host.toml is left alone"
    );
    assert!(!Files::of(&f.paths).installed_record("1.14.7").exists());
}
