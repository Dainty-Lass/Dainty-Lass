//! What a player host's launcher hands the MIT game runtime (spec-runtime-r10 5, 7.4; RX3): the launch variables,
//! argv and one control-pipe message. Sections split by 0x01: `NAME=VALUE` lines, NUL-separated argv, the message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_runtime_core::config::auth::AuthMode;
use rustic_isles_runtime_core::config::launch::parse_launch;
use rustic_isles_runtime_core::env::Raw;
use rustic_isles_runtime_core::player_host::{PIPE_PREFIX, Verdict, bootstrap, fence_request, is_fence};
use std::collections::HashMap;

fn safe(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn pipe(text: &str) -> bool {
    text.len() <= 240 && text.strip_prefix(PIPE_PREFIX).is_some_and(|rest| safe(rest, 16, 96))
}

fuzz_target!(|data: &[u8]| {
    let mut sections = data.splitn(3, |&b| b == 1);
    let env_text = String::from_utf8_lossy(sections.next().unwrap_or_default());
    let argv_text = String::from_utf8_lossy(sections.next().unwrap_or_default());
    let message = sections.next().unwrap_or_default();
    let env: HashMap<String, String> = env_text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let argv: Vec<Vec<u16>> = argv_text.split('\0').map(|arg| arg.encode_utf16().collect()).collect();
    let line: Vec<u16> = argv_text.encode_utf16().collect();
    let Ok(launch) = parse_launch(&argv, &line, AuthMode::PlayerHost) else {
        return;
    };
    let verdict = bootstrap(
        |name, cap| Raw::from_text(env.get(name).map(String::as_str), cap),
        |name| env.contains_key(name),
        &launch,
        &argv,
    );
    if let Verdict::Valid(boot) = verdict {
        assert_eq!(
            env.get("RUSTIC_ISLES_BIND_ADDRESS").map(String::as_str),
            Some("127.0.0.1")
        );
        assert!(safe(&boot.session, 36, 36));
        assert!((1..=10).contains(&boot.generation.len()) && boot.generation.bytes().all(|b| b.is_ascii_digit()));
        assert!(safe(&boot.capability, 43, 43));
        assert!(pipe(&boot.control_pipe) && pipe(&boot.ready_pipe));
        assert!(safe(&boot.launch_id, 16, 96));
        assert!(boot.launcher_pid != 0);
        assert!(argv.len() > 8);
        assert!(
            argv.iter()
                .skip(1)
                .any(|arg| String::from_utf16_lossy(arg).eq_ignore_ascii_case("-MULTIHOME=127.0.0.1"))
        );
        if let Some(max) = boot.role.max_players {
            assert!((1..=64).contains(&max));
        }
        if let Some(afk) = boot.role.hub_afk_seconds {
            assert!(afk <= 7200);
        }
        let hosted = boot.role.hosted();
        assert!(hosted.hub || (hosted.max_players.is_none() && hosted.hub_afk_seconds.is_none()));
        assert_eq!(
            is_fence(message, &boot.session, &boot.generation),
            message == fence_request(&boot.session, &boot.generation).as_bytes()
        );
    }
    let fence = fence_request("s", "1");
    assert_eq!(is_fence(message, "s", "1"), message == fence.as_bytes());
});
