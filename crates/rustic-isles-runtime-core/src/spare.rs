//! The warm spare hunt server (prototype): a hunt server the Director starts before anyone asks, held in the map
//! getter (SRV-1) on the call the engine makes just before it loads its first map, until a command file names the
//! hunt. Engine init (about 4.5 s of a hunt's 5) is then already done when a party asks.

use crate::config::launch::Launch;
use crate::wide;

/// On a server with this variable at `1`, SRV-1 holds for a hunt.
pub const VARIABLE: &str = "RUSTIC_ISLES_SPARE";
/// Overrides which SRV-1 call holds (1-based).
pub const HOLD_CALL_VARIABLE: &str = "RUSTIC_ISLES_SPARE_HOLD_CALL";
/// The call that holds unless the variable says otherwise: the first after the engine's early burst of reads, 0.2 s
/// before `StartGameInstance` (1.14.7, 2026-11-21: calls 1-13 at once early in init, 14 to 20 just before the load).
pub const HOLD_CALL: u32 = 14;
/// How often the held thread looks for the command file.
pub const POLL_MS: u32 = 25;
/// The most bytes read from the command file.
pub const MAX_BYTES: usize = 16 * 1024;

/// The command file for a port, next to the executable.
pub fn file_name(port: u16) -> String {
    format!("rustic_isles_spare_port{port}.txt")
}

/// The hunt the Director handed the spare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunt {
    /// The map path, with any `?` options the Director adds (Gauntlet's game mode and level).
    pub map: Vec<u16>,
    /// The Behemoth class, if any.
    pub behemoth: Option<Vec<u16>>,
    /// The matchmaker hunt id, if any.
    pub hunt_id: Option<Vec<u16>>,
    /// The roster (`user:hunt,...`), if any.
    pub players: Option<Vec<u16>>,
    /// The trials week an arena hunt keeps.
    pub week: Option<u32>,
}

/// Why a command file was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No `map=` line.
    NoMap,
    /// A line that is not `key=value` with a known key, or a value with a control character.
    BadLine(usize),
    /// A `week=` that is not 1 to 999.
    BadWeek,
}

fn value(text: &str) -> Option<Vec<u16>> {
    (!text.is_empty()).then(|| wide(text))
}

/// `parse(text)`: `key=value` lines (`map`, `behemoth`, `hunt`, `players`, `week`); blank lines and `#` comments
/// are skipped; the last of a key wins. Values are taken verbatim, as the Director's arguments are.
pub fn parse(text: &str) -> Result<Hunt, Refusal> {
    let mut hunt = Hunt {
        map: Vec::new(),
        behemoth: None,
        hunt_id: None,
        players: None,
        week: None,
    };
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            return Err(Refusal::BadLine(index + 1));
        };
        if raw.chars().any(char::is_control) {
            return Err(Refusal::BadLine(index + 1));
        }
        match key.trim() {
            "map" => hunt.map = wide(raw),
            "behemoth" => hunt.behemoth = value(raw),
            "hunt" => hunt.hunt_id = value(raw),
            "players" => hunt.players = value(raw),
            "week" => {
                hunt.week = match raw.parse::<u32>() {
                    Ok(week) if (1..=999).contains(&week) => Some(week),
                    _ => return Err(Refusal::BadWeek),
                }
            }
            _ => return Err(Refusal::BadLine(index + 1)),
        }
    }
    if hunt.map.is_empty() {
        return Err(Refusal::NoMap);
    }
    Ok(hunt)
}

/// The spare's own launch with the hunt's map, Behemoth, hunt id and roster; its key, port and address stay.
pub fn launch_for(spare: &Launch, hunt: &Hunt) -> Launch {
    Launch {
        map_path: hunt.map.clone(),
        behemoth: hunt.behemoth.clone(),
        matchmaker_hunt_id: hunt.hunt_id.clone(),
        expected_players: hunt.players.clone(),
        ..spare.clone()
    }
}

/// The held line.
pub fn held_line(call: u32, port: u16) -> String {
    format!(
        "[MitRuntime] [Spare] holding before the first map (SRV-1 call {call}); waiting for {}",
        file_name(port)
    )
}

/// The line when a hunt arrives: the map only, never the roster or hunt ids (as `[MapUrl]`).
pub fn arrived_line(hunt: &Hunt, held_ms: u64) -> String {
    format!(
        "[MitRuntime] [Spare] hunt arrived after {held_ms} ms held: map={}",
        String::from_utf16_lossy(&hunt.map)
    )
}

/// The line for a refused file.
pub fn refused_line(refusal: Refusal) -> String {
    let why = match refusal {
        Refusal::NoMap => "no map= line".to_owned(),
        Refusal::BadLine(line) => format!("line {line} is not a known key=value"),
        Refusal::BadWeek => "week= is not 1 to 999".to_owned(),
    };
    format!("[MitRuntime] [Spare] command file refused ({why}); still waiting")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::auth::AuthMode;
    use crate::config::launch::parse_launch;

    #[test]
    fn parses_a_hunt() {
        let hunt = parse("# from the Director\nmap=/Game/Maps/a?GauntletLevel=1\nbehemoth=/Game/B.B_C\nhunt=H\n\nplayers=u:p,v:p\nweek=12\n")
            .unwrap();
        assert_eq!(hunt.map, wide("/Game/Maps/a?GauntletLevel=1"));
        assert_eq!(hunt.behemoth, Some(wide("/Game/B.B_C")));
        assert_eq!(hunt.hunt_id, Some(wide("H")));
        assert_eq!(hunt.players, Some(wide("u:p,v:p")));
        assert_eq!(hunt.week, Some(12));
    }

    #[test]
    fn empty_values_are_absent_and_the_last_key_wins() {
        let hunt = parse("map=/a\nmap=/b\nbehemoth=\n").unwrap();
        assert_eq!(hunt.map, wide("/b"));
        assert_eq!(hunt.behemoth, None);
    }

    #[test]
    fn refusals() {
        assert_eq!(parse(""), Err(Refusal::NoMap));
        assert_eq!(parse("hunt=H"), Err(Refusal::NoMap));
        assert_eq!(parse("map=/a\nnonsense"), Err(Refusal::BadLine(2)));
        assert_eq!(parse("map=/a\ncolor=red"), Err(Refusal::BadLine(2)));
        assert_eq!(parse("map=/a\u{1}"), Err(Refusal::BadLine(1)));
        assert_eq!(parse("map=/a\nweek=0"), Err(Refusal::BadWeek));
        assert_eq!(parse("map=/a\nweek=x"), Err(Refusal::BadWeek));
    }

    #[test]
    fn the_override_keeps_the_spares_key_and_port() {
        let args: Vec<Vec<u16>> = [
            "game.exe",
            "KEY",
            "8770",
            "/Game/Spare",
            "NO_BEHEMOTH",
            "NO_MM_HUNTID",
            "NO_EXPECTED_PLAYERS",
            "1.2.3.4:8770",
            "-server",
        ]
        .iter()
        .map(|a| wide(a))
        .collect();
        let spare = parse_launch(&args, &wide("game.exe -server"), AuthMode::GlobalGameserver).unwrap();
        let hunt = parse("map=/Game/Maps/h\nhunt=H\nplayers=u:p").unwrap();
        let launch = launch_for(&spare, &hunt);
        assert_eq!(launch.port, spare.port);
        assert_eq!(launch.api_key, spare.api_key);
        assert_eq!(launch.own_address, spare.own_address);
        assert_eq!(launch.map_path, wide("/Game/Maps/h"));
        assert_eq!(launch.behemoth, None);
        assert_eq!(launch.expected_players, Some(wide("u:p")));
    }

    #[test]
    fn lines_name_the_map_only() {
        let hunt = parse("map=/Game/Maps/h\nplayers=secret:p").unwrap();
        let line = arrived_line(&hunt, 1500);
        assert!(line.contains("/Game/Maps/h") && line.contains("1500 ms"));
        assert!(!line.contains("secret"));
        assert_eq!(file_name(8770), "rustic_isles_spare_port8770.txt");
    }
}
