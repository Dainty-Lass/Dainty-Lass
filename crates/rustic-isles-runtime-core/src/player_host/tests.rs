use super::*;
use std::collections::HashMap;

const SESSION: &str = "0f3c9a2e-4b1d-4e8a-9c7f-1a2b3c4d5e6f";
const LAUNCH: &str = "host-0123456789abcdef0123456789abcdef";
const SUFFIX: &str = "0123456789abcdef0123456789abcdef";

fn w(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn valid_env() -> HashMap<&'static str, String> {
    HashMap::from([
        (BIND_ADDRESS, "127.0.0.1".to_owned()),
        (SESSION_ID, SESSION.to_owned()),
        (SESSION_GENERATION, "7".to_owned()),
        (CAPABILITY, "A".repeat(43)),
        (CONTROL_PIPE, format!("{PIPE_PREFIX}control.{SUFFIX}")),
        (READY_PIPE, format!("{PIPE_PREFIX}ready.{SUFFIX}")),
        (LAUNCH_ID, LAUNCH.to_owned()),
        (LAUNCHER_PID, "4242".to_owned()),
    ])
}

fn launch(full: bool, entry_1: &str) -> Launch {
    Launch {
        mode: if full { Mode::Full } else { Mode::Manual },
        api_key: None,
        port: 8791,
        map_path: w("/Game/Map"),
        behemoth: None,
        matchmaker_hunt_id: None,
        expected_players: None,
        own_address: None,
        raw_entry_1: Some(w(entry_1)),
        watchdog: full,
    }
}

fn argv(multihome: &str) -> Vec<Vec<u16>> {
    [
        "game.exe",
        "-",
        "8791",
        "/Game/Map",
        "x",
        "y",
        "1",
        "127.0.0.1:8791",
        multihome,
        "-server",
    ]
    .iter()
    .map(|entry| w(entry))
    .collect()
}

fn check_with(env: &HashMap<&'static str, String>, launch: &Launch, argv: &[Vec<u16>]) -> Verdict {
    bootstrap(
        |name, cap| Raw::from_text(env.get(name).map(String::as_str), cap),
        |name| env.contains_key(name),
        launch,
        argv,
    )
}

fn check(env: &HashMap<&'static str, String>) -> Verdict {
    check_with(env, &launch(true, "-"), &argv("-MULTIHOME=127.0.0.1"))
}

fn failed(verdict: Verdict) -> Option<&'static str> {
    match verdict {
        Verdict::Valid(_) => None,
        Verdict::Invalid { failed_check, .. } => failed_check,
    }
}

fn with(name: &'static str, value: &str) -> Option<&'static str> {
    let mut env = valid_env();
    env.insert(name, value.to_owned());
    failed(check(&env))
}

#[test]
fn valid_bootstrap() {
    let Verdict::Valid(bootstrap) = check(&valid_env()) else {
        panic!("valid");
    };
    assert_eq!(bootstrap.session, SESSION);
    assert_eq!(bootstrap.generation, "7");
    assert_eq!(bootstrap.launch_id, LAUNCH);
    assert_eq!(bootstrap.launcher_pid, 4242);
    assert_eq!(
        bootstrap.ready_pipe,
        format!(r"\\.\pipe\RusticInternalServer.ready.{SUFFIX}")
    );
    assert!(!bootstrap.role.hub && !bootstrap.guard_opt_in);
    assert!(!format!("{bootstrap:?}").contains(SESSION), "Debug hides the values");
}

#[test]
fn each_check_names_its_label() {
    let env = valid_env();
    assert_eq!(
        failed(check_with(&env, &launch(false, "-"), &argv("-MULTIHOME=127.0.0.1"))),
        Some("full launch")
    );
    assert_eq!(
        failed(check_with(&env, &launch(true, "x"), &argv("-MULTIHOME=127.0.0.1"))),
        Some("argv 1")
    );
    assert_eq!(
        failed(check_with(&env, &launch(true, "-"), &argv("-nothing"))),
        Some("MULTIHOME")
    );
    assert_eq!(
        failed(check_with(&env, &launch(true, "-"), &argv("-multihome=127.0.0.1"))),
        None
    );
    assert_eq!(with(BIND_ADDRESS, "127.0.0.2"), Some(BIND_ADDRESS));
    assert_eq!(with(SESSION_ID, &"a".repeat(35)), Some(SESSION_ID));
    assert_eq!(with(SESSION_ID, &"a".repeat(37)), Some(SESSION_ID));
    assert_eq!(with(SESSION_ID, &format!("{}/", "a".repeat(35))), Some(SESSION_ID));
    assert_eq!(with(SESSION_GENERATION, ""), Some(SESSION_GENERATION));
    assert_eq!(with(SESSION_GENERATION, "12345678901"), Some(SESSION_GENERATION));
    assert_eq!(with(SESSION_GENERATION, "1a"), Some(SESSION_GENERATION));
    assert_eq!(with(SESSION_GENERATION, "0007"), None);
    assert_eq!(with(CAPABILITY, &"A".repeat(42)), Some(CAPABILITY));
    assert_eq!(with(CAPABILITY, &"A".repeat(44)), Some(CAPABILITY));
    assert_eq!(
        with(
            CONTROL_PIPE,
            &format!(r"\\.\pipe\MysticInternalServer.control.{SUFFIX}")
        ),
        Some(CONTROL_PIPE)
    );
    assert_eq!(
        with(READY_PIPE, &format!("{PIPE_PREFIX}{}", "a".repeat(15))),
        Some(READY_PIPE)
    );
    assert_eq!(with(READY_PIPE, &format!("{PIPE_PREFIX}{}", "a".repeat(16))), None);
    assert_eq!(with(READY_PIPE, &format!("{PIPE_PREFIX}{}", "a".repeat(96))), None);
    assert_eq!(
        with(READY_PIPE, &format!("{PIPE_PREFIX}{}", "a".repeat(97))),
        Some(READY_PIPE)
    );
    assert_eq!(
        with(READY_PIPE, &format!(r"{PIPE_PREFIX}ready\{SUFFIX}")),
        Some(READY_PIPE)
    );
    assert_eq!(with(LAUNCH_ID, &"a".repeat(15)), Some(LAUNCH_ID));
    assert_eq!(with(LAUNCHER_PID, "0"), Some(LAUNCHER_PID));
    assert_eq!(with(LAUNCHER_PID, "4294967296"), Some(LAUNCHER_PID));
    assert_eq!(with(LAUNCHER_PID, "4294967295"), None);
    let mut missing = valid_env();
    missing.remove(LAUNCHER_PID);
    assert_eq!(failed(check(&missing)), Some(LAUNCHER_PID));
}

fn role(entries: &[(&'static str, &str)]) -> Result<LauncherRole, &'static str> {
    let mut env = valid_env();
    for (name, value) in entries {
        env.insert(name, (*value).to_owned());
    }
    match check(&env) {
        Verdict::Valid(bootstrap) => Ok(bootstrap.role),
        Verdict::Invalid {
            failed_check: None,
            role_problem: Some(problem),
        } => Err(problem),
        other => panic!("{other:?}"),
    }
}

fn hunt() -> LauncherRole {
    LauncherRole {
        hub: false,
        max_players: None,
        hub_afk_seconds: None,
    }
}

#[test]
fn roles() {
    use problems::*;
    assert_eq!(role(&[]), Ok(hunt()));
    assert_eq!(role(&[(HOSTED_ROLE, "HUNT")]), Ok(hunt()));
    assert_eq!(role(&[(HOSTED_ROLE, "HUB")]), Ok(LauncherRole { hub: true, ..hunt() }));
    assert_eq!(role(&[(HOSTED_ROLE, "hub")]), Err(ROLE_INVALID));
    assert_eq!(role(&[(HOSTED_ROLE, "")]), Err(ROLE_MALFORMED));
    assert_eq!(role(&[(HOSTED_ROLE, "HUNTER")]), Err(ROLE_INVALID));
    assert_eq!(role(&[(HOSTED_ROLE, "HUNTERSXY")]), Err(ROLE_MALFORMED));
    assert_eq!(role(&[(HOSTED_ROLE, "HU\u{1}B")]), Err(ROLE_MALFORMED));
    assert_eq!(role(&[(HOSTED_ROLE, "HU\u{7F}B")]), Err(ROLE_MALFORMED));
    let max = |value: &str| role(&[(HOSTED_ROLE, "HUB"), (MAX_PLAYERS, value)]).map(|role| role.max_players);
    assert_eq!(max("0"), Err(MAX_PLAYERS_INVALID));
    assert_eq!(max("1"), Ok(Some(1)));
    assert_eq!(max("64"), Ok(Some(64)));
    assert_eq!(max("65"), Err(MAX_PLAYERS_INVALID));
    assert_eq!(max("012"), Err(MAX_PLAYERS_INVALID));
    assert_eq!(max("+5"), Err(MAX_PLAYERS_INVALID));
    assert_eq!(max("123456"), Err(MAX_PLAYERS_MALFORMED));
    assert_eq!(max(""), Err(MAX_PLAYERS_MALFORMED));
    let afk = |value: &str| role(&[(HOSTED_ROLE, "HUB"), (HUB_AFK_SECONDS, value)]).map(|role| role.hub_afk_seconds);
    assert_eq!(afk("0"), Ok(Some(0)));
    assert_eq!(afk("7200"), Ok(Some(7200)));
    assert_eq!(afk("7201"), Err(AFK_INVALID));
    assert_eq!(afk("1\t"), Err(AFK_MALFORMED));
    // A hunt with numbers is valid; they are ignored by the hosted role
    let numbers = role(&[(MAX_PLAYERS, "8"), (HUB_AFK_SECONDS, "60")]).unwrap();
    assert_eq!(
        numbers,
        LauncherRole {
            hub: false,
            max_players: Some(8),
            hub_afk_seconds: Some(60)
        }
    );
    assert_eq!(numbers.hosted(), HostedRole::from_hunt());
    // The first problem stops the check
    assert_eq!(role(&[(HOSTED_ROLE, "x"), (MAX_PLAYERS, "")]), Err(ROLE_INVALID));
}

impl HostedRole {
    fn from_hunt() -> Self {
        Self {
            hub: false,
            max_players: None,
            hub_afk_seconds: None,
        }
    }
}

#[test]
fn problem_texts() {
    use problems::*;
    assert_eq!(ROLE_MALFORMED, "RUSTIC_ISLES_HOSTED_ROLE is set but empty or malformed");
    assert_eq!(ROLE_INVALID, "RUSTIC_ISLES_HOSTED_ROLE is neither HUNT nor HUB");
    assert_eq!(
        MAX_PLAYERS_MALFORMED,
        "RUSTIC_ISLES_MAX_PLAYERS is set but empty or malformed"
    );
    assert_eq!(
        MAX_PLAYERS_INVALID,
        "RUSTIC_ISLES_MAX_PLAYERS is not a whole number from 1 to 64"
    );
    assert_eq!(
        AFK_MALFORMED,
        "RUSTIC_ISLES_HUB_AFK_SECONDS is set but empty or malformed"
    );
    assert_eq!(
        AFK_INVALID,
        "RUSTIC_ISLES_HUB_AFK_SECONDS is not a whole number from 0 to 7200"
    );
}

#[test]
fn bootstrap_and_role_problems_together() {
    let mut env = valid_env();
    env.insert(BIND_ADDRESS, "0.0.0.0".to_owned());
    env.insert(HOSTED_ROLE, "BOTH".to_owned());
    let verdict = check(&env);
    assert_eq!(
        verdict,
        Verdict::Invalid {
            failed_check: Some(BIND_ADDRESS),
            role_problem: Some(problems::ROLE_INVALID),
        }
    );
    assert_eq!(
        invalid_lines(Some(BIND_ADDRESS), Some(problems::ROLE_INVALID)),
        [
            "[MitRuntime] [PlayerHost] invalid player-host bootstrap (RUSTIC_ISLES_BIND_ADDRESS); terminating before \
             listen",
            "[MitRuntime] [HostedRole] invalid launcher role: RUSTIC_ISLES_HOSTED_ROLE is neither HUNT nor HUB",
        ]
    );
    assert_eq!(
        invalid_lines(None, Some(problems::ROLE_INVALID))[0],
        "[MitRuntime] [PlayerHost] invalid player-host bootstrap (hosted role); terminating before listen"
    );
    assert_eq!(invalid_lines(Some("argv 1"), None).len(), 1);
}

#[test]
fn messages() {
    let fence = fence_request(SESSION, "7");
    assert_eq!(fence, format!("RIS1 FENCE session={SESSION} generation=7\n"));
    assert_eq!(
        fence_ack(SESSION, "7"),
        format!("RIS1 FENCE_ACK session={SESSION} generation=7\n")
    );
    assert_eq!(
        ready_message(SESSION, "7", LAUNCH, 8791),
        format!("RIS1 READY session={SESSION} generation=7 launch={LAUNCH} port=8791 bind=127.0.0.1\n")
    );
    assert!(is_fence(fence.as_bytes(), SESSION, "7"));
    assert!(!is_fence(format!("{}\r\n", fence.trim_end()).as_bytes(), SESSION, "7"));
    assert!(!is_fence(fence.trim_end().as_bytes(), SESSION, "7"));
    assert!(!is_fence(fence.as_bytes(), SESSION, "8"));
    assert!(!is_fence(fence.replace("RIS1", "MYS1").as_bytes(), SESSION, "7"));
    assert!(!is_fence(&[b'R'; 512], SESSION, "7"));
}

#[test]
fn role_and_guard_lines() {
    let hub = LauncherRole {
        hub: true,
        max_players: Some(16),
        hub_afk_seconds: None,
    };
    assert_eq!(
        role_line(&hub),
        "[MitRuntime] [HostedRole] launcher role=HUB (persistent hub) maxPlayers=16 (launcher) hubAfkSeconds=not \
         set (host setting or default)"
    );
    let hub = LauncherRole {
        hub: true,
        max_players: None,
        hub_afk_seconds: Some(600),
    };
    assert_eq!(
        role_line(&hub),
        "[MitRuntime] [HostedRole] launcher role=HUB (persistent hub) maxPlayers=not set (host setting or default) \
         hubAfkSeconds=600 (launcher)"
    );
    assert_eq!(
        role_line(&hunt()),
        "[MitRuntime] [HostedRole] launcher role=HUNT (disposable)"
    );
    assert_eq!(
        role_line(&LauncherRole {
            max_players: Some(4),
            ..hunt()
        }),
        "[MitRuntime] [HostedRole] launcher role=HUNT (disposable) - RUSTIC_ISLES_MAX_PLAYERS / \
         RUSTIC_ISLES_HUB_AFK_SECONDS are set but ignored (they apply to a HUB only)"
    );
    assert_eq!(
        guard_line(true),
        "[MitRuntime] [HuntGuard] no evidence producer in this runtime: the hunt is unmonitored and earns no rewards \
         (opt-in 1)"
    );
    assert!(guard_line(false).ends_with("(opt-in not set)"));
    let mut env = valid_env();
    env.insert(HUNT_GUARD_EVENTS, "1".to_owned());
    assert!(matches!(check(&env), Verdict::Valid(bootstrap) if bootstrap.guard_opt_in));
}

#[test]
fn hosted_role_keeps_numbers_only_for_a_hub() {
    let hub = LauncherRole {
        hub: true,
        max_players: Some(16),
        hub_afk_seconds: Some(0),
    };
    assert_eq!(
        hub.hosted(),
        HostedRole {
            hub: true,
            max_players: Some(16),
            hub_afk_seconds: Some(0)
        }
    );
}

#[test]
fn no_mystic_paradox_names_in_the_runtime_sources() {
    // RT-8: the alias and the legacy ready line are gone (17.4's grep, for the runtime crates)
    let needles = [
        concat!("MYSTIC", "PARADOX_"),
        concat!("MY", "S1"),
        concat!("Mystic", "InternalServer"),
        concat!("x-mystic", "-host-capability"),
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut stack = vec![root.join("src"), root.join("../rustic-isles-runtime/src")];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                seen += 1;
                let text = std::fs::read_to_string(&path).unwrap();
                for needle in needles {
                    // This file's own refusal tests spell the old pipe and tag on purpose
                    if path.ends_with("player_host/tests.rs") {
                        continue;
                    }
                    assert!(!text.contains(needle), "{} holds {needle}", path.display());
                }
            }
        }
    }
    assert!(seen > 20);
}
