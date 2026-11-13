//! The expected-player count and the map URL (R1 7; SRV-1's text, which R9 writes into the engine).

use crate::config::launch::Launch;
use crate::{wide, wide_contains};

/// `expected_player_count`: 0 for none or the sentinel; any malformed entry makes the whole roster 0; clamped
/// to `0..=hub_cap`.
pub fn expected_player_count(expected_players: Option<&[u16]>, hub_cap: u32) -> u32 {
    let Some(roster) = expected_players else {
        return 0;
    };
    if roster.is_empty() || wide_contains(roster, &wide("NO_EXPECTED_PLAYERS")) {
        return 0;
    }
    let comma = u16::from(b',');
    let colon = u16::from(b':');
    let mut entries: Vec<&[u16]> = roster.split(|&unit| unit == comma).collect();
    // A trailing comma ends the walk without an empty entry
    if roster.last() == Some(&comma) {
        entries.pop();
    }
    let mut count: u32 = 0;
    for entry in entries {
        match entry.iter().position(|&unit| unit == colon) {
            Some(at) if at != 0 && at + 1 != entry.len() => count = count.saturating_add(1),
            _ => return 0,
        }
    }
    count.min(hub_cap)
}

fn present<'a>(value: &'a Option<Vec<u16>>, sentinel: &str) -> Option<&'a [u16]> {
    value.as_deref().filter(|value| !wide_contains(value, &wide(sentinel)))
}

/// `map_url(launch, hub_cap, listen)`: values go in verbatim; `?MaxPlayers=` always (without it the game caps
/// players at 1); `?listen` only when R9 asks.
pub fn map_url(launch: &Launch, hub_cap: u32, listen: bool) -> Vec<u16> {
    let mut url = launch.map_path.clone();
    let parts = [
        ("?MonsterClass=", present(&launch.behemoth, "NO_BEHEMOTH")),
        ("?HuntId=", present(&launch.matchmaker_hunt_id, "NO_MM_HUNTID")),
        (
            "?PlayerHuntIds=",
            present(&launch.expected_players, "NO_EXPECTED_PLAYERS"),
        ),
    ];
    for (key, value) in parts {
        if let Some(value) = value {
            url.extend(wide(key));
            url.extend_from_slice(value);
        }
    }
    url.extend(wide(&format!("?MaxPlayers={hub_cap}")));
    if listen {
        url.extend(wide("?listen"));
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::auth::AuthMode;
    use crate::config::launch::parse_launch;

    fn count(text: &str) -> u32 {
        expected_player_count(Some(&wide(text)), 10)
    }

    #[test]
    fn roster_count() {
        assert_eq!(expected_player_count(None, 10), 0);
        assert_eq!(count(""), 0);
        assert_eq!(count("NO_EXPECTED_PLAYERS"), 0);
        assert_eq!(count("a:b"), 1);
        assert_eq!(count("a:b,c:d"), 2);
        assert_eq!(count("a:b, c:d"), 2);
        assert_eq!(count("a:b,"), 1);
        assert_eq!(count("a:b,,c:d"), 0);
        assert_eq!(count(":b"), 0);
        assert_eq!(count("a:"), 0);
        assert_eq!(count("a"), 0);
        assert_eq!(count("a:b:c"), 1);
        let long = ["a:b"; 12].join(",");
        assert_eq!(count(&long), 10);
        assert_eq!(expected_player_count(Some(&wide("a:b,c:d")), 1), 1);
    }

    fn launch(behemoth: &str, hunt: &str, players: &str) -> Launch {
        let args: Vec<Vec<u16>> = [
            "game.exe",
            "K",
            "7777",
            "Map",
            behemoth,
            hunt,
            players,
            "1.2.3.4:7777",
            "-server",
        ]
        .iter()
        .map(|arg| wide(arg))
        .collect();
        parse_launch(&args, &[], AuthMode::GlobalGameserver).unwrap()
    }

    fn url(launch: &Launch, listen: bool) -> String {
        String::from_utf16(&map_url(launch, 4, listen)).unwrap()
    }

    #[test]
    fn sentinels() {
        assert_eq!(
            url(&launch("BP_Shrike", "h1", "a:b"), false),
            "Map?MonsterClass=BP_Shrike?HuntId=h1?PlayerHuntIds=a:b?MaxPlayers=4"
        );
        assert_eq!(
            url(&launch("x_NO_BEHEMOTH", "h1", "a:b"), false),
            "Map?HuntId=h1?PlayerHuntIds=a:b?MaxPlayers=4"
        );
        assert_eq!(
            url(&launch("BP", "NO_MM_HUNTID", "a:b"), false),
            "Map?MonsterClass=BP?PlayerHuntIds=a:b?MaxPlayers=4"
        );
        assert_eq!(
            url(&launch("BP", "h", "NO_EXPECTED_PLAYERS"), false),
            "Map?MonsterClass=BP?HuntId=h?MaxPlayers=4"
        );
        assert_eq!(
            url(&launch("NO_BEHEMOTH", "NO_MM_HUNTID", "NO_EXPECTED_PLAYERS"), true),
            "Map?MaxPlayers=4?listen"
        );
    }

    #[test]
    fn manual_mode_fields_absent() {
        let manual = parse_launch(&[wide("game.exe")], &[], AuthMode::GlobalGameserver).unwrap();
        assert_eq!(url(&manual, false), "Ramsgate?MaxPlayers=4");
        assert_eq!(url(&manual, true), "Ramsgate?MaxPlayers=4?listen");
    }
}
