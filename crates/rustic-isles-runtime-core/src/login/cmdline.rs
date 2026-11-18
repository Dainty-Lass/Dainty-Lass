//! CL-1 (R4 3): the engine command line served with the switches the launcher and the Director put on the OS one,
//! and the `MULTIHOME` rules R8, R9 and R10 share.

use super::{contains_ignore_ascii_case, eq_ignore_ascii_case};
use crate::config::backend::{is_dotted_quad, is_ipv6};
use crate::{wide, wide_contains};

/// R5's flag; R4 reads it once for the culture switch (patch 0031). The two halves share no state.
pub const CULTURE_FLAG: &str = "DISABLE_SERVER_LOCALE_FIX";
/// An effective `MULTIHOME` this long or longer is ignored.
pub const MAX_MULTIHOME_UNITS: usize = 64;
const MAX_IP_LITERAL_UNITS: usize = 45;
const TAG: &str = "[MitRuntime] [CommandLine]";
/// Switches a client carries from its OS line: the Shipping engine's allow-list drops them, and only a headless bot
/// client (spec-runtime-sim 8) is started with them; the launcher never passes them.
pub const CLIENT_HEADLESS_SWITCHES: [&str; 3] = ["-nullrhi", "-norenderthread", "-nothreadtimeout"];

/// What the served line says about the culture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Culture {
    /// `-CULTURE=en-US` was appended.
    EnUs,
    /// The line already had a culture.
    Engine,
    /// A client, or the flag is present.
    Off,
}

/// One `MULTIHOME` outcome of R4 3.4, for its log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultihomeNote {
    /// Step 2: the OS value (an IP literal) was appended.
    Carried(String),
    /// Step 2: the OS value is not an IP literal; its length.
    NotCarried(usize),
    /// Step 3: the engine's own value, which differs from the OS line's, was kept.
    Kept(Vec<u16>),
    /// Step 4: the effective value is not an IP literal (or too long); its length.
    Invalid(usize),
}

/// The line CL-1 serves and what its log line reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    /// The served line, without its NUL.
    pub line: Vec<u16>,
    /// The engine's own line had `-NoEAC`.
    pub engine_no_eac: bool,
    /// The engine's own line had `-DisableEOS`.
    pub engine_disable_eos: bool,
    /// The engine's own line had `-nullrhi`.
    pub engine_null_rhi: bool,
    /// The OS line has `-nullrhi`.
    pub os_null_rhi: bool,
    /// The culture switch.
    pub culture: Culture,
    /// The effective `MULTIHOME` the engine will bind (R4 3.5).
    pub multihome: Option<String>,
    /// The `MULTIHOME` outcomes, in step order.
    pub notes: Vec<MultihomeNote>,
}

fn has(line: &[u16], switch: &str) -> bool {
    wide_contains(line, &wide(switch))
}

fn append(line: &mut Vec<u16>, switch: &[u16]) {
    line.push(u16::from(b' '));
    line.extend_from_slice(switch);
}

/// R4 3.3: the engine's line with the missing switches appended, in the spec's order.
pub fn build_served_line(
    engine: &[u16],
    os_argv: &[Vec<u16>],
    os_line: &[u16],
    server: bool,
    culture_fix: bool,
) -> Served {
    let mut line = engine.to_vec();
    let os_null_rhi = has(os_line, "-nullrhi");
    let (engine_no_eac, engine_disable_eos, engine_null_rhi) = (
        has(engine, "-NoEAC"),
        has(engine, "-DisableEOS"),
        has(engine, "-nullrhi"),
    );
    // The loose substring tests are the AGPL's: `-NoEACx` counts as present
    for switch in ["-NoEAC", "-DisableEOS"] {
        if !has(&line, switch) {
            append(&mut line, &wide(switch));
        }
    }
    if server && os_null_rhi && !has(&line, "-nullrhi") {
        append(&mut line, &wide("-nullrhi"));
    }
    if !server {
        for switch in CLIENT_HEADLESS_SWITCHES {
            if has(os_line, switch) && !has(&line, switch) {
                append(&mut line, &wide(switch));
            }
        }
    }
    let mut notes = Vec::new();
    let mut effective = engine_multihome_in(&line);
    if server && let Some(os) = os_multihome_in(os_argv) {
        if effective.is_empty() {
            if is_ip_literal(&os) {
                let mut switch = wide("-MULTIHOME=");
                switch.extend_from_slice(&os);
                append(&mut line, &switch);
                notes.push(MultihomeNote::Carried(String::from_utf16_lossy(&os)));
                effective = os;
            } else {
                notes.push(MultihomeNote::NotCarried(os.len()));
            }
        } else if !eq_ignore_ascii_case(&os, &effective) {
            notes.push(MultihomeNote::Kept(effective.clone()));
        }
    }
    if !effective.is_empty() && (!is_ip_literal(&effective) || effective.len() >= MAX_MULTIHOME_UNITS) {
        notes.push(MultihomeNote::Invalid(effective.len()));
        effective.clear();
    }
    let culture = if !server || !culture_fix {
        Culture::Off
    } else if contains_ignore_ascii_case(&line, "-culture=") {
        Culture::Engine
    } else {
        append(&mut line, &wide("-CULTURE=en-US"));
        Culture::EnUs
    };
    Served {
        line,
        engine_no_eac,
        engine_disable_eos,
        engine_null_rhi,
        os_null_rhi,
        culture,
        multihome: (!effective.is_empty()).then(|| String::from_utf16_lossy(&effective)),
        notes,
    }
}

fn is_ascii_alphanumeric(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|byte| byte.is_ascii_alphanumeric())
}

fn stops_value(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|byte| matches!(byte, b' ' | b'\t' | b',' | b'"' | b')'))
}

/// The engine-line scan, as `FParse::Value` finds `MULTIHOME=`: outside double quotes, not after a letter or digit,
/// case-insensitive; the value runs to a space, tab, `,`, `"`, `)` or the end. Empty when not found.
pub fn engine_multihome_in(line: &[u16]) -> Vec<u16> {
    let key = wide("MULTIHOME=");
    let mut quoted = false;
    let mut previous: Option<u16> = None;
    for (at, &unit) in line.iter().enumerate() {
        let rest = &line[at..];
        if !quoted
            && !previous.is_some_and(is_ascii_alphanumeric)
            && rest.len() >= key.len()
            && eq_ignore_ascii_case(&rest[..key.len()], &key)
        {
            let value = &rest[key.len()..];
            let end = value.iter().position(|&unit| stops_value(unit)).unwrap_or(value.len());
            return value[..end].to_vec();
        }
        if unit == u16::from(b'"') {
            quoted = !quoted;
        }
        previous = Some(unit);
    }
    Vec::new()
}

/// The OS-line scan: the first argument after argv 0 starting with `-MULTIHOME=` (case-insensitive); the rest of it
/// verbatim, an empty value included.
pub fn os_multihome_in(argv: &[Vec<u16>]) -> Option<Vec<u16>> {
    let key = wide("-MULTIHOME=");
    argv.iter()
        .skip(1)
        .find(|arg| arg.len() >= key.len() && eq_ignore_ascii_case(&arg[..key.len()], &key))
        .map(|arg| arg[key.len()..].to_vec())
}

/// A dotted quad or an IPv6 address by R1's strict grammars, at most 45 units, all ASCII (R4 departure 2).
pub fn is_ip_literal(value: &[u16]) -> bool {
    if value.len() > MAX_IP_LITERAL_UNITS || !value.iter().all(|&unit| unit < 0x80) {
        return false;
    }
    let text = String::from_utf16_lossy(value);
    is_dotted_quad(&text) || is_ipv6(&text)
}

fn bit(value: bool) -> u8 {
    u8::from(value)
}

impl Served {
    /// The one summary line (R4 3.6). Never the line itself: it carries the exchange code or the gameserver key.
    pub fn log_line(&self) -> String {
        let culture = match self.culture {
            Culture::EnUs => "en-US",
            Culture::Engine => "engine",
            Culture::Off => "off",
        };
        format!(
            "{TAG} engine line served: NoEAC={} DisableEOS={} NullRHI={} engineNoEAC={} engineDisableEOS={} \
             engineNullRHI={} osNullRHI={} Culture={culture} Multihome={}",
            bit(has(&self.line, "-NoEAC")),
            bit(has(&self.line, "-DisableEOS")),
            bit(has(&self.line, "-nullrhi")),
            bit(self.engine_no_eac),
            bit(self.engine_disable_eos),
            bit(self.engine_null_rhi),
            bit(self.os_null_rhi),
            self.multihome.as_deref().unwrap_or("none"),
        )
    }

    /// The `MULTIHOME` lines, in step order. A value that failed the IP-literal test is logged by length only.
    pub fn note_lines(&self) -> Vec<String> {
        self.notes
            .iter()
            .map(|note| match note {
                MultihomeNote::Carried(value) => format!("{TAG} MULTIHOME={value} carried from the OS command line"),
                MultihomeNote::NotCarried(len) => format!(
                    "{TAG} MULTIHOME on the OS command line is not an IP literal ({len} chars); not carried, the \
                     engine binds every interface"
                ),
                MultihomeNote::Kept(value) if is_ip_literal(value) => format!(
                    "{TAG} the engine line already has MULTIHOME={}, which differs from the OS command line's; the \
                     engine's is kept",
                    String::from_utf16_lossy(value)
                ),
                MultihomeNote::Kept(value) => format!(
                    "{TAG} the engine line already has a MULTIHOME ({} chars), which differs from the OS command \
                     line's; the engine's is kept",
                    value.len()
                ),
                MultihomeNote::Invalid(len) => {
                    format!("{TAG} the engine line's MULTIHOME is not an IP literal ({len} chars); ignored")
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str) -> Vec<u16> {
        wide(text)
    }

    fn argv(args: &[&str]) -> Vec<Vec<u16>> {
        args.iter().map(|arg| w(arg)).collect()
    }

    fn text(served: &Served) -> String {
        String::from_utf16_lossy(&served.line)
    }

    fn client(engine: &str) -> Served {
        build_served_line(&w(engine), &argv(&["game.exe"]), &w("game.exe"), false, true)
    }

    #[test]
    fn client_gets_both_switches_in_order() {
        let served = client("game.exe -AUTH_TYPE=exchangecode");
        assert_eq!(text(&served), "game.exe -AUTH_TYPE=exchangecode -NoEAC -DisableEOS");
        assert!(!served.engine_no_eac && !served.engine_disable_eos);
        assert_eq!(served.culture, Culture::Off);
        assert!(served.notes.is_empty());
    }

    #[test]
    fn line_with_both_is_unchanged() {
        let served = client("game.exe -DisableEOS -NoEAC");
        assert_eq!(text(&served), "game.exe -DisableEOS -NoEAC");
        assert!(served.engine_no_eac && served.engine_disable_eos);
    }

    #[test]
    fn substring_tests_are_loose() {
        assert_eq!(text(&client("game.exe -NoEACx")), "game.exe -NoEACx -DisableEOS");
        assert_eq!(text(&client("game.exe -noeac")), "game.exe -noeac -NoEAC -DisableEOS");
    }

    #[test]
    fn null_rhi_from_the_os_line() {
        let os = w("server.exe -server -nullrhi");
        let server = build_served_line(&w("server.exe -server"), &[], &os, true, false);
        assert_eq!(text(&server), "server.exe -server -NoEAC -DisableEOS -nullrhi");
        assert!(server.os_null_rhi && !server.engine_null_rhi);
        let client = build_served_line(&w("game.exe"), &[], &os, false, false);
        assert_eq!(
            text(&client),
            "game.exe -NoEAC -DisableEOS -nullrhi",
            "a headless bot client"
        );
        let bot = w("game.exe -AUTH_TYPE=exchangecode -nullrhi -norenderthread -nothreadtimeout");
        let served = build_served_line(&w("game.exe -AUTH_TYPE=exchangecode"), &[], &bot, false, false);
        assert_eq!(
            text(&served),
            "game.exe -AUTH_TYPE=exchangecode -NoEAC -DisableEOS -nullrhi -norenderthread -nothreadtimeout"
        );
        let player = build_served_line(
            &w("game.exe"),
            &[],
            &w("game.exe -AUTH_TYPE=exchangecode"),
            false,
            false,
        );
        assert_eq!(
            text(&player),
            "game.exe -NoEAC -DisableEOS",
            "a launcher's player: nothing added"
        );
        let server_only = build_served_line(&w("s"), &[], &w("s -norenderthread"), true, false);
        assert_eq!(
            text(&server_only),
            "s -NoEAC -DisableEOS",
            "a server keeps its own rule"
        );
        let none = build_served_line(&w("server.exe -server"), &[], &w("server.exe -server"), true, false);
        assert!(!has(&none.line, "-nullrhi"));
        let already = build_served_line(&w("s -nullrhi"), &[], &os, true, false);
        assert_eq!(text(&already), "s -nullrhi -NoEAC -DisableEOS");
    }

    #[test]
    fn culture() {
        let os = w("s");
        let kept = build_served_line(&w("s -Culture=de-DE"), &[], &os, true, true);
        assert_eq!(text(&kept), "s -Culture=de-DE -NoEAC -DisableEOS");
        assert_eq!(kept.culture, Culture::Engine);
        let off = build_served_line(&w("s"), &[], &os, true, false);
        assert_eq!(text(&off), "s -NoEAC -DisableEOS");
        assert_eq!(off.culture, Culture::Off);
        let on = build_served_line(&w("s"), &[], &os, true, true);
        assert_eq!(text(&on), "s -NoEAC -DisableEOS -CULTURE=en-US");
        assert_eq!(on.culture, Culture::EnUs);
    }

    #[test]
    fn full_server_order() {
        let os_argv = argv(&["server.exe", "/Game/Map", "-server", "-nullrhi", "-MULTIHOME=127.0.0.1"]);
        let served = build_served_line(
            &w("server.exe /Game/Map -server"),
            &os_argv,
            &w("x -nullrhi"),
            true,
            true,
        );
        assert_eq!(
            text(&served),
            "server.exe /Game/Map -server -NoEAC -DisableEOS -nullrhi -MULTIHOME=127.0.0.1 -CULTURE=en-US"
        );
        assert_eq!(served.multihome.as_deref(), Some("127.0.0.1"));
        assert_eq!(
            served.log_line(),
            "[MitRuntime] [CommandLine] engine line served: NoEAC=1 DisableEOS=1 NullRHI=1 engineNoEAC=0 \
             engineDisableEOS=0 engineNullRHI=0 osNullRHI=1 Culture=en-US Multihome=127.0.0.1"
        );
    }

    #[test]
    fn engine_scan() {
        let scan = |line: &str| String::from_utf16_lossy(&engine_multihome_in(&w(line)));
        assert_eq!(scan("s -MULTIHOME=1.2.3.4"), "1.2.3.4");
        assert_eq!(scan("s multihome=1.2.3.4 -x"), "1.2.3.4");
        assert_eq!(scan("MULTIHOME=::1"), "::1");
        assert_eq!(scan("s XMULTIHOME=1.2.3.4"), "");
        assert_eq!(scan("s 9MULTIHOME=1.2.3.4"), "");
        assert_eq!(scan("s \"-MULTIHOME=1.2.3.4\""), "");
        assert_eq!(scan("s \"q\" -MULTIHOME=1.2.3.4"), "1.2.3.4");
        for stop in [" ", "\t", ",", "\"", ")"] {
            assert_eq!(scan(&format!("-MULTIHOME=1.2.3.4{stop}rest")), "1.2.3.4", "{stop:?}");
        }
        assert_eq!(scan("s"), "");
        assert_eq!(scan("-MULTIHOME="), "");
    }

    #[test]
    fn os_scan() {
        let scan = |args: &[&str]| os_multihome_in(&argv(args)).map(|value| String::from_utf16_lossy(&value));
        assert_eq!(scan(&["-MULTIHOME=1.1.1.1"]), None, "argv 0 is ignored");
        assert_eq!(
            scan(&["x", "-MULTIHOME=1.1.1.1", "-MULTIHOME=2.2.2.2"]),
            Some("1.1.1.1".into())
        );
        assert_eq!(scan(&["x", "-multihome=Abc"]), Some("Abc".into()));
        assert_eq!(scan(&["x", "-MULTIHOME="]), Some(String::new()), "empty is found");
        assert_eq!(scan(&["x", "MULTIHOME=1.1.1.1"]), None);
        assert_eq!(scan(&["x"]), None);
    }

    fn multihome(engine: &str, os: &[&str], server: bool) -> Served {
        build_served_line(&w(engine), &argv(os), &w(""), server, false)
    }

    #[test]
    fn multihome_outcomes() {
        let carried = multihome("s", &["s", "-MULTIHOME=127.0.0.1"], true);
        assert_eq!(carried.notes, [MultihomeNote::Carried("127.0.0.1".into())]);
        assert_eq!(carried.multihome.as_deref(), Some("127.0.0.1"));
        assert_eq!(
            carried.note_lines(),
            ["[MitRuntime] [CommandLine] MULTIHOME=127.0.0.1 carried from the OS command line"]
        );

        let not_carried = multihome("s", &["s", "-MULTIHOME=localhost"], true);
        assert_eq!(not_carried.notes, [MultihomeNote::NotCarried(9)]);
        assert_eq!(not_carried.multihome, None);
        assert_eq!(text(&not_carried), "s -NoEAC -DisableEOS");
        assert!(!not_carried.note_lines()[0].contains("localhost"));

        let kept = multihome("s -MULTIHOME=10.0.0.1", &["s", "-MULTIHOME=127.0.0.1"], true);
        assert_eq!(kept.notes, [MultihomeNote::Kept(w("10.0.0.1"))]);
        assert_eq!(kept.multihome.as_deref(), Some("10.0.0.1"));
        assert!(kept.note_lines()[0].contains("MULTIHOME=10.0.0.1, which differs"));
        let same = multihome("s -MULTIHOME=FE80::1", &["s", "-MULTIHOME=fe80::1"], true);
        assert!(same.notes.is_empty(), "compared case-insensitively");

        let invalid = multihome("s -MULTIHOME=my-host", &["s"], true);
        assert_eq!(invalid.notes, [MultihomeNote::Invalid(7)]);
        assert_eq!(invalid.multihome, None);
        assert!(!invalid.note_lines()[0].contains("my-host"));
        let kept_invalid = multihome("s -MULTIHOME=my-host", &["s", "-MULTIHOME=127.0.0.1"], true);
        assert_eq!(
            kept_invalid.notes,
            [MultihomeNote::Kept(w("my-host")), MultihomeNote::Invalid(7)]
        );
        assert!(kept_invalid.note_lines().iter().all(|line| !line.contains("my-host")));

        let client = multihome("g", &["g", "-MULTIHOME=127.0.0.1"], false);
        assert!(
            client.notes.is_empty() && client.multihome.is_none(),
            "a client never carries"
        );
        let client_invalid = multihome("g -MULTIHOME=x", &["g"], false);
        assert_eq!(client_invalid.notes, [MultihomeNote::Invalid(1)]);
    }

    #[test]
    fn the_64_unit_limit() {
        let long = "x".repeat(MAX_MULTIHOME_UNITS);
        let served = multihome(&format!("s -MULTIHOME={long}"), &["s"], true);
        assert_eq!(served.notes, [MultihomeNote::Invalid(64)]);
        let long_os = format!("-MULTIHOME={long}");
        let served = multihome("s", &["s", &long_os], true);
        assert_eq!(served.notes, [MultihomeNote::NotCarried(64)]);
    }

    #[test]
    fn ip_literals() {
        for good in ["127.0.0.1", "::1", "fe80::1", "0.0.0.0", "2001:db8::10.0.0.1"] {
            assert!(is_ip_literal(&w(good)), "{good}");
        }
        for bad in [
            "127.0.0.01",
            "[::1]",
            "fe80::1%3",
            "localhost",
            "",
            "1.2.3",
            "256.0.0.1",
        ] {
            assert!(!is_ip_literal(&w(bad)), "{bad}");
        }
        let mut non_ascii = w("127.0.0.1");
        non_ascii[0] = 0xFF11;
        assert!(!is_ip_literal(&non_ascii));
    }
}
