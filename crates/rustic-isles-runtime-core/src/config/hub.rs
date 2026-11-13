//! Hub cap and AFK timeouts (R1 10, RT-11): the player host's override, then the `DAINTY_` variables, then the
//! number files next to the executable, then the default.

use crate::config::numbers::{stream_i32, stream_i64, strtol};

/// The player host's cap override (R10 validates it).
pub const LAUNCHER_MAX_PLAYERS: &str = "RUSTIC_ISLES_MAX_PLAYERS";
/// The player host's hub timeout override.
pub const LAUNCHER_HUB_AFK: &str = "RUSTIC_ISLES_HUB_AFK_SECONDS";
/// The hub cap (RT-11 name, R1 call 3).
pub const ENV_HUB_MAX_PLAYERS: &str = "DAINTY_HUB_MAX_PLAYERS";
/// The hub AFK timeout.
pub const ENV_HUB_AFK: &str = "DAINTY_HUB_AFK_SECONDS";
/// The hunt AFK timeout.
pub const ENV_HUNT_AFK: &str = "DAINTY_HUNT_AFK_SECONDS";
/// The environment reads' `max`.
pub const ENV_MAX: usize = 16;
/// The hub cap file.
pub const FILE_HUB_MAX_PLAYERS: &str = "HUB_MAX_PLAYERS.txt";
/// The hub AFK file.
pub const FILE_HUB_AFK: &str = "HUB_AFK_TIMEOUT_SECONDS.txt";
/// The hunt AFK file.
pub const FILE_HUNT_AFK: &str = "HUNT_AFK_TIMEOUT_SECONDS.txt";
/// Bytes read from a number file (R1 call 4).
pub const FILE_MAX_BYTES: usize = 64;
/// A Dauntless party.
pub const DEFAULT_HUB_CAP: i32 = 4;
/// The hub's timeout when nothing sets one.
pub const DEFAULT_HUB_AFK: i64 = 1200;
/// The longest timeout.
pub const MAX_AFK: i64 = 604_800;

/// The inputs for one value: the launcher's override (player host only), the environment (`env_value`'s answer)
/// and the file's first bytes (`None` when absent).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources {
    /// Source 1, already validated by R10.
    pub launcher: Option<i64>,
    /// Source 2.
    pub env: Option<String>,
    /// Source 3.
    pub file: Option<Vec<u8>>,
}

/// The hub cap and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubCap {
    /// 1 to 64.
    pub value: i32,
    /// `launcher role`, `env`, `file` or `default`.
    pub source: &'static str,
}

/// The hub cap: 1 to 64 from the first source that gives one, else 4.
pub fn hub_cap(sources: &Sources) -> HubCap {
    let valid = |value: i32| (1..=64).contains(&value);
    if let Some(value) = sources
        .launcher
        .and_then(|value| i32::try_from(value).ok())
        .filter(|&value| valid(value))
    {
        return HubCap {
            value,
            source: "launcher role",
        };
    }
    if let Some(value) = sources.env.as_deref().and_then(strtol).filter(|&value| valid(value)) {
        return HubCap { value, source: "env" };
    }
    if let Some(value) = sources
        .file
        .as_deref()
        .and_then(stream_i32)
        .filter(|&value| valid(value))
    {
        return HubCap { value, source: "file" };
    }
    HubCap {
        value: DEFAULT_HUB_CAP,
        source: "default",
    }
}

/// An AFK timeout: `None` seconds is the game's native behaviour (no override).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AfkSetting {
    /// The timeout, 0 = never kick.
    pub seconds: Option<i64>,
    /// The source text R8 logs.
    pub source: &'static str,
}

fn afk_rule(value: i64, texts: [&'static str; 5]) -> AfkSetting {
    let [plain, negative, never, sixty, most] = texts;
    match value {
        ..0 => AfkSetting {
            seconds: None,
            source: negative,
        },
        0 => AfkSetting {
            seconds: Some(0),
            source: never,
        },
        1..=59 => AfkSetting {
            seconds: Some(60),
            source: sixty,
        },
        _ if value > MAX_AFK => AfkSetting {
            seconds: Some(MAX_AFK),
            source: most,
        },
        _ => AfkSetting {
            seconds: Some(value),
            source: plain,
        },
    }
}

const ENV_TEXTS: [&str; 5] = [
    "env",
    "env (negative = native)",
    "env (0 = never kick)",
    "env (clamped to 60)",
    "env (clamped to 604800)",
];
const FILE_TEXTS: [&str; 5] = [
    "file",
    "file (negative = native)",
    "file (0 = never kick)",
    "file (clamped to 60)",
    "file (clamped to 604800)",
];

fn afk(sources: &Sources, default: Option<i64>) -> AfkSetting {
    if let Some(value) = sources.launcher {
        // The launcher's range is 0 to 7200 (R10), so only these three cases occur
        return match value {
            0 => AfkSetting {
                seconds: Some(0),
                source: "launcher role (0 = never kick)",
            },
            1..=59 => AfkSetting {
                seconds: Some(60),
                source: "launcher role (clamped to 60)",
            },
            _ => AfkSetting {
                seconds: Some(value),
                source: "launcher role",
            },
        };
    }
    let mut unreadable = None;
    if let Some(text) = &sources.env {
        match strtol(text) {
            Some(value) => return afk_rule(i64::from(value), ENV_TEXTS),
            None => unreadable = Some("env unreadable -> default"),
        }
    }
    if let Some(bytes) = &sources.file {
        match stream_i64(bytes) {
            Some(value) => return afk_rule(value, FILE_TEXTS),
            None => unreadable = Some("file unreadable -> default"),
        }
    }
    AfkSetting {
        seconds: default,
        source: unreadable.unwrap_or("default"),
    }
}

/// The hub timeout; the launcher override counts only for a player host (the caller passes none otherwise).
pub fn hub_afk(sources: &Sources) -> AfkSetting {
    afk(sources, Some(DEFAULT_HUB_AFK))
}

/// The hunt timeout: native by default. There is no launcher override for hunts.
pub fn hunt_afk(sources: &Sources) -> AfkSetting {
    afk(
        &Sources {
            launcher: None,
            ..sources.clone()
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(text: &str) -> Sources {
        Sources {
            env: Some(text.into()),
            ..Sources::default()
        }
    }

    fn file(bytes: &[u8]) -> Sources {
        Sources {
            file: Some(bytes.to_vec()),
            ..Sources::default()
        }
    }

    fn setting(seconds: Option<i64>, source: &'static str) -> AfkSetting {
        AfkSetting { seconds, source }
    }

    #[test]
    fn cap_sources_in_order() {
        assert_eq!(
            hub_cap(&Sources::default()),
            HubCap {
                value: 4,
                source: "default"
            }
        );
        let all = Sources {
            launcher: Some(8),
            env: Some("12".into()),
            file: Some(b"16".to_vec()),
        };
        assert_eq!(
            hub_cap(&all),
            HubCap {
                value: 8,
                source: "launcher role"
            }
        );
        let no_launcher = Sources {
            launcher: None,
            ..all.clone()
        };
        assert_eq!(
            hub_cap(&no_launcher),
            HubCap {
                value: 12,
                source: "env"
            }
        );
        let bad_env = Sources {
            env: Some("12x".into()),
            ..no_launcher.clone()
        };
        assert_eq!(
            hub_cap(&bad_env),
            HubCap {
                value: 16,
                source: "file"
            }
        );
        assert_eq!(hub_cap(&file(b" 12\nx")).value, 12);
        assert_eq!(hub_cap(&file(b"12x")).value, 12);
        assert_eq!(hub_cap(&env("12x")).source, "default");
        assert_eq!(hub_cap(&file(b"x12")).source, "default");
        assert_eq!(hub_cap(&file(b"")).source, "default");
        assert_eq!(hub_cap(&file(b"\xEF\xBB\xBF12")).source, "default");
        assert_eq!(hub_cap(&file(b"99999999999")).source, "default");
        assert_eq!(hub_cap(&file(b"65")).source, "default");
        assert_eq!(hub_cap(&file(b"0")).source, "default");
        assert_eq!(hub_cap(&file(b"64")).value, 64);
        assert_eq!(hub_cap(&env("1")).value, 1);
    }

    #[test]
    fn afk_rules() {
        assert_eq!(hub_afk(&Sources::default()), setting(Some(1200), "default"));
        assert_eq!(hunt_afk(&Sources::default()), setting(None, "default"));
        assert_eq!(hub_afk(&file(b"-1")), setting(None, "file (negative = native)"));
        assert_eq!(hub_afk(&file(b"0")), setting(Some(0), "file (0 = never kick)"));
        assert_eq!(hub_afk(&file(b"1")), setting(Some(60), "file (clamped to 60)"));
        assert_eq!(hub_afk(&file(b"59")), setting(Some(60), "file (clamped to 60)"));
        assert_eq!(hub_afk(&file(b"60")), setting(Some(60), "file"));
        assert_eq!(hub_afk(&file(b"604800")), setting(Some(604_800), "file"));
        assert_eq!(
            hub_afk(&file(b"604801")),
            setting(Some(604_800), "file (clamped to 604800)")
        );
        assert_eq!(
            hub_afk(&file(b"99999999999")),
            setting(Some(604_800), "file (clamped to 604800)")
        );
        assert_eq!(hub_afk(&file(b"x")), setting(Some(1200), "file unreadable -> default"));
        assert_eq!(hunt_afk(&file(b"x")), setting(None, "file unreadable -> default"));
        assert_eq!(hunt_afk(&env("-5")), setting(None, "env (negative = native)"));
        assert_eq!(hunt_afk(&env("0")), setting(Some(0), "env (0 = never kick)"));
        assert_eq!(hunt_afk(&env("30")), setting(Some(60), "env (clamped to 60)"));
        assert_eq!(
            hunt_afk(&env("700000")),
            setting(Some(604_800), "env (clamped to 604800)")
        );
        assert_eq!(hunt_afk(&env("300")), setting(Some(300), "env"));
        assert_eq!(hunt_afk(&env("12x")), setting(None, "env unreadable -> default"));
    }

    #[test]
    fn afk_falls_through() {
        let both = Sources {
            launcher: None,
            env: Some("x".into()),
            file: Some(b"300".to_vec()),
        };
        assert_eq!(hub_afk(&both), setting(Some(300), "file"));
        let env_wins = Sources {
            launcher: None,
            env: Some("120".into()),
            file: Some(b"300".to_vec()),
        };
        assert_eq!(hub_afk(&env_wins), setting(Some(120), "env"));
    }

    #[test]
    fn launcher_override() {
        let launcher = |value| Sources {
            launcher: Some(value),
            env: Some("300".into()),
            file: None,
        };
        assert_eq!(
            hub_afk(&launcher(0)),
            setting(Some(0), "launcher role (0 = never kick)")
        );
        assert_eq!(
            hub_afk(&launcher(30)),
            setting(Some(60), "launcher role (clamped to 60)")
        );
        assert_eq!(hub_afk(&launcher(7200)), setting(Some(7200), "launcher role"));
        assert_eq!(hunt_afk(&launcher(30)), setting(Some(300), "env"));
    }
}
