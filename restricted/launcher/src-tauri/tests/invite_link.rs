//! dainty-launcher-kit's invite links and the single launcher (spec §2.7.8, row L34), as the shell's
//! `platform/invite_link.rs` tested them, and the game's console reader (§2.7.16) from `platform/game_console.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use dainty_launcher_kit::bridge::spec::EXTERNAL_TARGETS;
use dainty_launcher_kit::bridge::types::ExternalTarget;
use dainty_launcher_kit::platform::game_console::{Tracker, redact};
use dainty_launcher_kit::platform::invite_link::{
    self as kit, CLIENT_ID, Channel, Claim, MANAGER_ID, claim, invite_in_args,
};
use dainty_launcher_kit::platform::links::fixed_url;
use dainty_protocol::invite::invite_schemes;

const LINK: &str = "reforged-revival://join?v=1&host=100.64.0.1&port=61000&code=abcd&name=x";

fn invite(args: &[&str]) -> Option<String> {
    invite_in_args(args)
}

#[test]
fn the_invite_link_among_the_arguments() {
    assert_eq!(invite(&["launcher.exe", LINK]).as_deref(), Some(LINK));
    assert_eq!(
        invite(&[
            "launcher.exe",
            "REFORGED-REVIVAL://join?v=1&host=100.64.0.1&port=61000&code=abcd&name=x "
        ])
        .as_deref(),
        Some("REFORGED-REVIVAL://join?v=1&host=100.64.0.1&port=61000&code=abcd&name=x")
    );
    assert_eq!(
        invite(&["launcher.exe", "reforged-revival://join?v=9"]),
        None,
        "the first link counts, and it must be valid"
    );
    assert_eq!(invite(&["launcher.exe", "--flag"]), None);
    let unified = LINK.replacen("reforged-revival", "dauntless", 1);
    assert_eq!(invite(&["launcher.exe", &unified]).as_deref(), Some(unified.as_str()));
    let revived = LINK.replacen("reforged-revival", "dauntless-revived", 1);
    assert!(
        invite(&["launcher.exe", &revived]).is_none(),
        "a 1.4.4 invite is no link of ours"
    );
}

#[tokio::test]
async fn a_second_launcher_hands_its_arguments_to_the_first() {
    let name = if cfg!(windows) {
        format!(r"\\.\pipe\rr-launcher-test-{}", std::process::id())
    } else {
        std::env::temp_dir()
            .join(format!("rr-launcher-test-{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned()
    };
    let channel = Channel::at(name);
    let Claim::First(mut holder) = claim(&channel, &["first".into()]).await.unwrap() else {
        panic!("the first launcher holds the channel");
    };
    for _ in 0..2 {
        let second = tokio::spawn({
            let channel = Channel::at(channel.address().to_path_buf());
            async move { claim(&channel, &["second".into(), LINK.into()]).await.unwrap() }
        });
        let got = tokio::time::timeout(Duration::from_secs(5), holder.receive())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(second.await.unwrap(), Claim::HandedOver));
        assert_eq!(invite_in_args(&got).as_deref(), Some(LINK));
    }
}

// A drive-letter path is absolute only on Windows
#[cfg(windows)]
#[test]
fn windows_registers_the_scheme_for_the_current_user() {
    let exe = Path::new(r"C:\Users\me\AppData\Local\DaintyLass Launcher\launcher.exe");
    let values = kit::registry_values(exe).unwrap();
    assert_eq!(values.len(), 3 * invite_schemes().len(), "three values per scheme");
    assert_eq!(
        (values[0].key.as_str(), values[0].data.as_str()),
        (r"Software\Classes\dauntless", "URL:Dauntless invite")
    );
    let key = r"Software\Classes\reforged-revival";
    let row = |v: &kit::RegistryValue| (v.key.clone(), v.name.map(str::to_owned), v.data.clone());
    assert_eq!(
        values[3..6].iter().map(row).collect::<Vec<_>>(),
        [
            (key.to_owned(), None, "URL:Dainty Lass invite".to_owned()),
            (key.to_owned(), Some("URL Protocol".to_owned()), String::new()),
            (
                format!(r"{key}\shell\open\command"),
                None,
                format!("\"{}\" \"%1\"", exe.display())
            ),
        ]
    );
}

/// The shell wrote the values itself and stopped at the first that failed; dainty-launcher-kit runs `reg.exe` for each, and
/// one that cannot run fails the registration.
#[tokio::test]
async fn a_failed_registration_is_an_error() {
    let exe = Path::new(r"C:\Games\DaintyLass\dainty-launcher.exe");
    assert!(
        kit::register_windows(exe, Path::new("no-such-reg-tool.exe"))
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn linux_writes_a_handler_entry_for_the_appimage() {
    let mimes: Vec<String> = invite_schemes()
        .iter()
        .map(|scheme| format!("x-scheme-handler/{scheme}"))
        .collect();
    assert_eq!(
        kit::desktop_entry(Path::new("/home/me/Apps/Reforged Launcher.AppImage")).unwrap(),
        format!(
            "[Desktop Entry]\nType=Application\nName=Dainty Lass Launcher\nExec=\"/home/me/Apps/Reforged Launcher.AppImage\" %u\nMimeType={}\nNoDisplay=true\nTerminal=false\n",
            mimes.iter().map(|mime| format!("{mime};")).collect::<String>()
        )
    );
    assert!(kit::DESKTOP_FILE.ends_with(".desktop"));
}

#[tokio::test]
async fn a_relative_appimage_or_a_refusing_xdg_mime_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let refuse = std::ffi::OsStr::new("no-such-xdg-mime");
    assert!(
        kit::register_linux(Path::new("relative.AppImage"), dir.path(), refuse)
            .await
            .is_err()
    );
    if cfg!(unix) {
        assert!(
            kit::register_linux(Path::new("/x.AppImage"), dir.path(), refuse)
                .await
                .is_err()
        );
    }
}

/// An AppImage path is a Unix one: on Windows `/a/...` is not absolute, and dainty-launcher-kit refuses it there (as the
/// shell's comparison with dainty-launcher-kit already allowed for).
#[cfg(unix)]
#[test]
fn desktop_entries_escape_the_path() {
    let exec = |path: &str| {
        kit::desktop_entry(Path::new(path)).map(|entry| {
            entry
                .lines()
                .find_map(|line| line.strip_prefix("Exec="))
                .unwrap()
                .to_owned()
        })
    };
    assert_eq!(
        exec(r#"/a/$b`c"d\e%f.AppImage"#).as_deref(),
        Some(r#""/a/\\$b\\`c\\"d\\\\e%%f.AppImage" %u"#)
    );
    assert_eq!(exec("/a/b\nExec=evil"), None);
    assert_eq!(exec("relative.AppImage"), None);
}

/// What players and the OS see, over the grid the shell compared with dainty-launcher-kit (no difference at the cutover,
/// 9d45b770): the channel's name, the links in arguments and every external target's URL, against the shell's
/// answers recorded in `fixtures/invite-link.txt`.
#[test]
fn what_players_and_the_os_see() {
    let mut lines = Vec::new();
    for base in [CLIENT_ID, MANAGER_ID] {
        let args: Vec<OsString> = vec!["launcher.exe".into()];
        let address = Channel::current(base, &args).address().to_string_lossy().into_owned();
        // The user's name and the temporary folder vary by PC; the rest is the contract
        assert!(address.contains(base), "{address}");
    }
    for args in [
        vec!["launcher.exe"],
        vec!["launcher.exe", "dauntless://join?v=1&h=100.64.0.7&p=61000"],
        vec!["launcher.exe", "--flag", "reforged-revival://join?x"],
        vec!["launcher.exe", "dauntless-revived://join?x"],
        vec!["launcher.exe", "https://example.org"],
        vec!["launcher.exe", "DAUNTLESS://JOIN"],
    ] {
        lines.push(format!("invite in {args:?}: {:?}", invite(&args)));
    }
    for name in EXTERNAL_TARGETS {
        let target: ExternalTarget = serde_json::from_value(serde_json::json!(name)).unwrap();
        lines.push(format!("{name}: {:?}", fixed_url(target)));
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invite-link.txt");
    let actual = lines.join("\n") + "\n";
    if std::env::var("RR_UPDATE_GENERATED").is_ok_and(|v| v == "1") && !path.exists() {
        std::fs::write(&path, &actual).unwrap();
    }
    assert_eq!(actual, std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n"));
}

// ---------------------------------------------------------------- game_console.rs

#[test]
fn the_key_never_survives_a_line() {
    let key = concat!("RRK_", "abcdefghijklmnop", "abcdefghijklmnop");
    let line = format!("LogInit: Command Line:  -NoEAC -AUTH_PASSWORD={key} -epicapp=Archon");
    let out = redact(&line);
    assert!(!out.contains("abcdefghijklmnop"), "{out}");
    assert_eq!(
        out,
        "LogInit: Command Line:  -NoEAC -AUTH_PASSWORD=<hidden> -epicapp=Archon"
    );
    assert_eq!(
        redact(&format!("key {key}, again {key}")),
        "key RRK_<hidden>, again RRK_<hidden>"
    );
    assert_eq!(redact("-auth_password=\"x y\" -next"), "-auth_password=<hidden> -next");
    assert_eq!(redact("nothing here"), "nothing here");
}

fn rows(lines: &[&str], width: usize) -> Vec<String> {
    lines.iter().map(|line| format!("{line:width$}")).collect()
}

fn poll(tracker: &mut Tracker, buffer: &[String], cursor: usize) -> Vec<String> {
    tracker.poll(cursor, &mut |r| buffer.get(r).cloned())
}

#[test]
fn rows_above_the_cursor_are_read_once() {
    let mut tracker = Tracker::default();
    let mut buffer = rows(&["one", "two", "thr"], 6);
    assert_eq!(poll(&mut tracker, &buffer, 2), ["one", "two"]);
    assert!(poll(&mut tracker, &buffer, 2).is_empty());
    buffer.extend(rows(&["four", ""], 6));
    assert_eq!(poll(&mut tracker, &buffer, 4), ["thr", "four"]);
}

#[test]
fn a_full_row_joins_the_next() {
    let mut tracker = Tracker::default();
    let buffer = vec!["abcdef".to_owned(), "gh    ".to_owned(), "      ".to_owned()];
    assert_eq!(poll(&mut tracker, &buffer, 2), ["abcdefgh"]);
}

#[test]
fn a_scrolled_buffer_is_followed() {
    let mut tracker = Tracker::default();
    let buffer = rows(&["a", "b", "c", "d"], 4);
    assert_eq!(poll(&mut tracker, &buffer, 3), ["a", "b", "c"]);
    // Full: everything moved up two rows and two new lines came in at the bottom
    let scrolled = rows(&["c", "d", "e", "f"], 4);
    assert_eq!(poll(&mut tracker, &scrolled, 3), ["d", "e"]);
}

#[test]
fn a_cleared_buffer_starts_over() {
    let mut tracker = Tracker::default();
    assert_eq!(poll(&mut tracker, &rows(&["a", "b", "c"], 4), 3), ["a", "b", "c"]);
    assert_eq!(poll(&mut tracker, &rows(&["x", ""], 4), 1), ["x"]);
}
