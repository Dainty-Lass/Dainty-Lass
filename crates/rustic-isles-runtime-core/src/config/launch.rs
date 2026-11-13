//! Server launch arguments (R1 6, surface 2.4): positional with more than 8 entries, the manual test mode
//! otherwise. Values stay UTF-16 because the engine takes them back (the map URL).

use crate::config::auth::AuthMode;
use crate::{wide, wide_find};

/// The exit code Init ends a full launch with when its port is refused (R1 call 2).
pub const PORT_EXIT_CODE: u32 = 0xE106;
/// The manual mode's port.
pub const MANUAL_PORT: u16 = 8777;
/// The manual mode's map.
pub const MANUAL_MAP: &str = "Ramsgate";

/// Which form the command line had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// More than 8 entries, from the Director or the launcher.
    Full,
    /// 8 or fewer: started by hand.
    Manual,
}

/// The parsed launch. Never logged: the command line, `api_key`, `raw_entry_1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Full or manual.
    pub mode: Mode,
    /// The gameserver key (entry 1), kept only for a DeployServer gameserver.
    pub api_key: Option<Vec<u16>>,
    /// The listen port.
    pub port: u16,
    /// Entry 3, or the manual map.
    pub map_path: Vec<u16>,
    /// Entry 4.
    pub behemoth: Option<Vec<u16>>,
    /// Entry 5.
    pub matchmaker_hunt_id: Option<Vec<u16>>,
    /// Entry 6.
    pub expected_players: Option<Vec<u16>>,
    /// Entry 7, the server's own `ip:port`.
    pub own_address: Option<Vec<u16>>,
    /// Entry 1 as given (R10 requires `-` for a player host).
    pub raw_entry_1: Option<Vec<u16>>,
    /// The empty-server watchdog: on for a full launch.
    pub watchdog: bool,
}

/// A full launch's port was not a plain decimal port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortError;

/// A full launch's port: 1 to 5 ASCII digits, no leading zero, 1 to 65535.
pub fn strict_port(text: &[u16]) -> Option<u16> {
    if text.is_empty() || text.len() > 5 || text[0] == u16::from(b'0') {
        return None;
    }
    let mut value: u32 = 0;
    for &unit in text {
        let digit = u8::try_from(unit).ok().filter(u8::is_ascii_digit)?;
        value = value * 10 + u32::from(digit - b'0');
    }
    u16::try_from(value).ok().filter(|&port| port != 0)
}

/// The manual mode's `Port=` digits: the run right after the first `Port=`, 1 to 5 digits, 1 to 65535.
pub fn manual_port(narrow: &[u16]) -> u16 {
    let key = wide("Port=");
    let Some(at) = wide_find(narrow, &key) else {
        return MANUAL_PORT;
    };
    let rest = &narrow[at + key.len()..];
    let digits = rest
        .iter()
        .take_while(|&&unit| (u16::from(b'0')..=u16::from(b'9')).contains(&unit))
        .count();
    if !(1..=5).contains(&digits) {
        return MANUAL_PORT;
    }
    let value = rest[..digits]
        .iter()
        .fold(0u32, |value, &unit| value * 10 + u32::from(unit - u16::from(b'0')));
    u16::try_from(value)
        .ok()
        .filter(|&port| port != 0)
        .unwrap_or(MANUAL_PORT)
}

/// Parses `args` (`CommandLineToArgvW`, entry 0 the program) and the narrow command line (the manual mode's
/// `Port=`). `Err` ends the process with `PORT_EXIT_CODE` in Init.
pub fn parse_launch(args: &[Vec<u16>], narrow: &[u16], auth: AuthMode) -> Result<Launch, PortError> {
    if args.len() > 8 {
        let entry = |index: usize| args.get(index).cloned();
        let port = strict_port(&args[2]).ok_or(PortError)?;
        return Ok(Launch {
            mode: Mode::Full,
            api_key: if auth == AuthMode::GlobalGameserver {
                entry(1)
            } else {
                None
            },
            port,
            map_path: args[3].clone(),
            behemoth: entry(4),
            matchmaker_hunt_id: entry(5),
            expected_players: entry(6),
            own_address: entry(7),
            raw_entry_1: entry(1),
            watchdog: true,
        });
    }
    // The parsed argv entry, not the raw line (R1 departure 4); empty counts as absent
    let map_path = match args.get(1) {
        Some(first) if !first.is_empty() && first[0] != u16::from(b'-') => {
            let end = first
                .iter()
                .position(|&unit| unit == u16::from(b'?'))
                .unwrap_or(first.len());
            first[..end].to_vec()
        }
        _ => wide(MANUAL_MAP),
    };
    Ok(Launch {
        mode: Mode::Manual,
        api_key: None,
        port: manual_port(narrow),
        map_path,
        behemoth: None,
        matchmaker_hunt_id: None,
        expected_players: None,
        own_address: None,
        raw_entry_1: args.get(1).cloned(),
        watchdog: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<Vec<u16>> {
        list.iter().map(|arg| wide(arg)).collect()
    }

    fn full(port: &str) -> Vec<Vec<u16>> {
        args(&[
            "C:\\Games\\Dauntless Dir\\Dauntless-Win64-Shipping.exe",
            "KEY",
            port,
            "/Game/Maps/Hunt",
            "/Game/Behemoths/Shrike",
            "hunt-1",
            "a:1, b:2",
            "1.2.3.4:7777",
            "-server",
        ])
    }

    #[test]
    fn full_launch() {
        let launch = parse_launch(&full("7777"), &[], AuthMode::GlobalGameserver).unwrap();
        assert_eq!(launch.mode, Mode::Full);
        assert_eq!(launch.port, 7777);
        assert_eq!(launch.api_key, Some(wide("KEY")));
        assert_eq!(launch.map_path, wide("/Game/Maps/Hunt"));
        assert_eq!(launch.behemoth, Some(wide("/Game/Behemoths/Shrike")));
        assert_eq!(launch.matchmaker_hunt_id, Some(wide("hunt-1")));
        assert_eq!(launch.expected_players, Some(wide("a:1, b:2")));
        assert_eq!(launch.own_address, Some(wide("1.2.3.4:7777")));
        assert!(launch.watchdog);
        let host = parse_launch(&full("7777"), &[], AuthMode::PlayerHost).unwrap();
        assert_eq!((host.api_key, host.raw_entry_1), (None, Some(wide("KEY"))));
    }

    #[test]
    fn extra_entries_ignored() {
        let mut list = full("80");
        list.extend(args(&["x", "y", "z"]));
        assert_eq!(list.len(), 12);
        let launch = parse_launch(&list, &[], AuthMode::GlobalGameserver).unwrap();
        assert_eq!((launch.port, launch.own_address), (80, Some(wide("1.2.3.4:7777"))));
    }

    #[test]
    fn port_errors() {
        for bad in ["0", "65536", "+80", " 80", "80x", "", "080", "123456"] {
            assert_eq!(
                parse_launch(&full(bad), &[], AuthMode::GlobalGameserver),
                Err(PortError),
                "{bad:?}"
            );
        }
        assert_eq!(
            parse_launch(&full("65535"), &[], AuthMode::GlobalGameserver)
                .unwrap()
                .port,
            65_535
        );
        assert_eq!(
            parse_launch(&full("1"), &[], AuthMode::GlobalGameserver).unwrap().port,
            1
        );
    }

    #[test]
    fn manual_mode() {
        let line = wide("\"C:\\Program Files\\D\\game.exe\" Ramsgate_Hub?listen -server Port=9000");
        let launch = parse_launch(
            &args(&[
                "C:\\Program Files\\D\\game.exe",
                "Ramsgate_Hub?listen",
                "-server",
                "Port=9000",
            ]),
            &line,
            AuthMode::GlobalGameserver,
        )
        .unwrap();
        assert_eq!((launch.mode, launch.port), (Mode::Manual, 9000));
        assert_eq!(launch.map_path, wide("Ramsgate_Hub"));
        assert!(!launch.watchdog);
        assert_eq!(
            (launch.api_key, launch.behemoth, launch.expected_players),
            (None, None, None)
        );
        let launch = parse_launch(
            &args(&["game.exe", "-log"]),
            &wide("game.exe -log"),
            AuthMode::GlobalGameserver,
        )
        .unwrap();
        assert_eq!((launch.port, launch.map_path), (8777, wide("Ramsgate")));
        let launch = parse_launch(
            &args(&["game.exe"]),
            &wide("game.exe Port=x"),
            AuthMode::GlobalGameserver,
        )
        .unwrap();
        assert_eq!(launch.port, 8777);
        assert_eq!(
            parse_launch(&[], &[], AuthMode::GlobalGameserver).unwrap().map_path,
            wide("Ramsgate")
        );
    }

    #[test]
    fn manual_ports() {
        assert_eq!(manual_port(&wide("x Port=1")), 1);
        assert_eq!(manual_port(&wide("x Port=65535 Port=2")), 65_535);
        assert_eq!(manual_port(&wide("x Port=65536")), 8777);
        assert_eq!(manual_port(&wide("x Port=0")), 8777);
        assert_eq!(manual_port(&wide("x Port=123456")), 8777);
        assert_eq!(manual_port(&wide("x Port= 80")), 8777);
        assert_eq!(manual_port(&wide("x port=80")), 8777);
        assert_eq!(manual_port(&wide("x Port=80abc")), 80);
    }
}
