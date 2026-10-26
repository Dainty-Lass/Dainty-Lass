//! Settings from the environment (`src/config.ts`), read once at start; a bad one stops the start.
//! The names and defaults are Node's, so the `.env` files the host kit writes keep working.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use protocol::game::Game;
use serde_json::json;
use service_kit::env::Env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `CONTENT_GAME`: the 1.14.7 runtime, or Dauntless Revived's 1.4.4 game files (legacy)
    pub game: Game,
    pub port: u16,
    pub bind_hosts: Vec<String>,
    /// The Mystic Paradox backend, or DR's metagame on a legacy server: the account-key lookup
    pub backend_url: String,
    /// `CONTENT_RUNTIME_DIR`: unset means no runtime is offered, which is not a start error
    pub runtime_dir: Option<PathBuf>,
    /// `CONTENT_GAME_DIR`, the folder that contains `Archon\`: required on a legacy server only
    pub game_dir: Option<PathBuf>,
    /// `CONTENT_MANIFEST`, or `None` for DR's 1.4.4 manifest built into the binary (legacy only)
    pub manifest_path: Option<PathBuf>,
    pub branding_dir: Option<PathBuf>,
    pub news_file: Option<PathBuf>,
    pub news_url: Option<String>,
    pub max_streams_per_account: usize,
    pub max_streams_total: usize,
    pub auth_cache_seconds: u64,
}

fn int(env: &Env, name: &str, default: i64, min: i64, max: i64) -> Result<i64, String> {
    let Some(raw) = env.raw(name).filter(|r| !r.trim().is_empty()) else {
        return Ok(default);
    };
    let value = service_kit::js::number(Some(&json!(raw)));
    if value.fract() != 0.0 || value < min as f64 || value > max as f64 {
        return Err(format!("{name} must be a whole number from {min} to {max}"));
    }
    Ok(value as i64)
}

fn optional(env: &Env, name: &str) -> Option<String> {
    env.raw(name)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// `path.resolve`.
fn resolve(path: &str) -> PathBuf {
    std::path::absolute(Path::new(path)).unwrap_or_else(|_| PathBuf::from(path))
}

/// `IsLoopbackBindHost`: `localhost`, any 127.x IPv4 address, or `::1`. The content server sits
/// behind the gateway, which is the only listener players reach.
pub fn is_loopback_bind_host(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    // net.isIPv4: four decimal parts without leading zeros
    let is_ipv4 = host.parse::<std::net::Ipv4Addr>().is_ok();
    if is_ipv4 {
        return host.starts_with("127.");
    }
    host == "::1"
}

/// `CONTENT_GAME`: `reborn` (the default) or `legacy`, the latter only in a build that carries it.
pub fn parse_game(raw: Option<&str>) -> Result<Game, String> {
    let game = match raw.map(|r| r.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") | Some("reborn") => Game::Reborn,
        Some("legacy") => Game::Legacy,
        Some(_) => {
            return Err(format!(
                "CONTENT_GAME must be reborn or legacy (got {})",
                service_kit::jsjson::stringify(&json!(raw.unwrap_or_default()))
            ));
        }
    };
    if !game.supported() {
        return Err("CONTENT_GAME=legacy (Dauntless Revived's 1.4.4) needs a content server built with the `legacy` feature; this one serves reborn only".into());
    }
    Ok(game)
}

impl Config {
    pub fn from_env(env: &Env) -> Result<Self, String> {
        let game = parse_game(env.raw("CONTENT_GAME"))?;
        let port = int(env, "PORT", 61002, 1, 65535)? as u16;
        let bind_hosts: Vec<String> = optional(env, "BIND_HOST")
            .unwrap_or_else(|| "127.0.0.1".into())
            .split(',')
            .map(|h| service_kit::js::trim(h).to_owned())
            .filter(|h| !h.is_empty())
            .collect();
        if bind_hosts.is_empty() {
            return Err("BIND_HOST is empty".into());
        }
        for host in &bind_hosts {
            if !is_loopback_bind_host(host) {
                return Err(format!(
                    "BIND_HOST must be a loopback address (got {}): players reach the content server through the gateway",
                    service_kit::jsjson::stringify(&json!(host))
                ));
            }
        }
        let backend_url = optional(env, "BACKEND_URL")
            .unwrap_or_else(|| "http://127.0.0.1:61000".into())
            .trim_end_matches('/')
            .to_owned();
        let parsed = url::Url::parse(&backend_url).map_err(|_| "BACKEND_URL is not a URL".to_owned())?;
        let host = parsed.host_str().unwrap_or("");
        let host = host.strip_prefix('[').unwrap_or(host);
        let host = host.strip_suffix(']').unwrap_or(host);
        if parsed.scheme() != "http" || !is_loopback_bind_host(host) {
            return Err("BACKEND_URL must be http:// on this machine (the account keys travel in it)".into());
        }
        // Validated here rather than in the news module's constructor (as Node does): the start
        // fails the same way, with a `Configuration:` error
        let news_url = crate::news::github_news_url(optional(env, "CONTENT_NEWS_URL").as_deref())?;
        // Read only on a legacy server, where the game folder is DR's only required setting
        let (game_dir, manifest_path) = match game {
            Game::Reborn => (None, None),
            Game::Legacy => {
                let Some(game_dir) = optional(env, "CONTENT_GAME_DIR") else {
                    return Err("CONTENT_GAME_DIR is not set (the folder that contains Archon\\)".into());
                };
                (
                    Some(resolve(&game_dir)),
                    optional(env, "CONTENT_MANIFEST").map(|p| resolve(&p)),
                )
            }
        };
        Ok(Self {
            game,
            port,
            bind_hosts,
            backend_url,
            runtime_dir: optional(env, "CONTENT_RUNTIME_DIR").map(|p| resolve(&p)),
            game_dir,
            manifest_path,
            branding_dir: optional(env, "CONTENT_BRANDING_DIR").map(|p| resolve(&p)),
            news_file: optional(env, "CONTENT_NEWS_FILE").map(|p| resolve(&p)),
            news_url,
            max_streams_per_account: int(env, "CONTENT_MAX_STREAMS_PER_ACCOUNT", 4, 1, 64)? as usize,
            max_streams_total: int(env, "CONTENT_MAX_STREAMS_TOTAL", 48, 1, 1024)? as usize,
            auth_cache_seconds: int(env, "CONTENT_AUTH_CACHE_SECONDS", 300, 0, 3600)? as u64,
        })
    }

    /// The address a bind host listens on (`localhost` is 127.0.0.1, as Node resolves it first).
    pub fn bind_address(host: &str) -> Option<IpAddr> {
        if host == "localhost" {
            return Some(IpAddr::from([127, 0, 0, 1]));
        }
        host.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    //! test/config.test.ts, and the rest of config.ts
    use super::*;

    fn load(pairs: &[(&str, &str)]) -> Result<Config, String> {
        Config::from_env(&Env::from_pairs(pairs.iter().copied()))
    }

    #[test]
    fn has_the_documented_defaults() {
        let c = load(&[]).unwrap();
        assert_eq!(c.port, 61002);
        assert_eq!(c.bind_hosts, ["127.0.0.1"]);
        assert_eq!(c.backend_url, "http://127.0.0.1:61000");
        assert_eq!(c.runtime_dir, None);
        assert_eq!(c.max_streams_per_account, 4);
        assert_eq!(c.max_streams_total, 48);
        assert_eq!(c.auth_cache_seconds, 300);
        assert_eq!((c.branding_dir, c.news_file, c.news_url), (None, None, None));
    }

    #[test]
    fn listens_on_loopback_only_and_checks_keys_with_the_backend_on_this_machine_only() {
        for bind in ["0.0.0.0", "100.101.102.103", "203.0.113.7", "::", "my-host.example"] {
            assert!(load(&[("BIND_HOST", bind)]).unwrap_err().contains("loopback"), "{bind}");
        }
        assert_eq!(
            load(&[("BIND_HOST", "127.0.0.1,::1")]).unwrap().bind_hosts,
            ["127.0.0.1", "::1"]
        );
        assert_eq!(
            load(&[("BIND_HOST", " localhost , 127.0.0.2 ")]).unwrap().bind_hosts,
            ["localhost", "127.0.0.2"]
        );
        assert!(
            load(&[("BACKEND_URL", "http://203.0.113.7:61000")])
                .unwrap_err()
                .contains("this machine")
        );
        assert!(
            load(&[("BACKEND_URL", "https://127.0.0.1:61000")])
                .unwrap_err()
                .contains("this machine")
        );
        assert!(load(&[("PORT", "0")]).unwrap_err().contains("PORT"));
    }

    #[test]
    fn the_backend_url_is_trimmed_and_may_name_any_loopback_host() {
        for (given, kept) in [
            ("http://127.0.0.1:61000///", "http://127.0.0.1:61000"),
            ("http://localhost:61000", "http://localhost:61000"),
            ("http://[::1]:61000/", "http://[::1]:61000"),
            ("http://127.5.6.7:1", "http://127.5.6.7:1"),
        ] {
            assert_eq!(load(&[("BACKEND_URL", given)]).unwrap().backend_url, kept, "{given}");
        }
        assert!(load(&[("BACKEND_URL", "not a url")]).unwrap_err().contains("not a URL"));
        assert!(load(&[("BACKEND_URL", "ftp://127.0.0.1")]).is_err());
    }

    #[test]
    fn the_game_is_reborn_unless_a_legacy_build_is_told_otherwise() {
        let c = load(&[("CONTENT_GAME_DIR", "game"), ("CONTENT_MANIFEST", "m.json")]).unwrap();
        // A reborn server never reads the game folder
        assert_eq!((c.game, c.game_dir, c.manifest_path), (Game::Reborn, None, None));
        assert_eq!(load(&[("CONTENT_GAME", " REBORN ")]).unwrap().game, Game::Reborn);
        assert!(
            load(&[("CONTENT_GAME", "1.4.4")])
                .unwrap_err()
                .contains("reborn or legacy")
        );
        let legacy = load(&[("CONTENT_GAME", "legacy"), ("CONTENT_GAME_DIR", "game")]);
        if cfg!(feature = "legacy") {
            let legacy = legacy.unwrap();
            assert_eq!(legacy.game, Game::Legacy);
            assert!(legacy.game_dir.unwrap().is_absolute());
            assert_eq!(legacy.manifest_path, None);
            assert!(
                load(&[("CONTENT_GAME", "legacy")])
                    .unwrap_err()
                    .contains("CONTENT_GAME_DIR is not set")
            );
            let manifest = load(&[
                ("CONTENT_GAME", "legacy"),
                ("CONTENT_GAME_DIR", "game"),
                ("CONTENT_MANIFEST", " m.json "),
            ])
            .unwrap()
            .manifest_path
            .unwrap();
            assert!(manifest.is_absolute() && manifest.ends_with("m.json"));
        } else {
            assert!(legacy.unwrap_err().contains("`legacy` feature"));
        }
    }

    #[test]
    fn numbers_folders_and_the_news_url_are_checked() {
        assert!(
            load(&[("CONTENT_MAX_STREAMS_PER_ACCOUNT", "65")])
                .unwrap_err()
                .contains("CONTENT_MAX_STREAMS_PER_ACCOUNT must be a whole number from 1 to 64")
        );
        assert!(load(&[("CONTENT_MAX_STREAMS_TOTAL", "1.5")]).is_err());
        assert_eq!(
            load(&[("CONTENT_AUTH_CACHE_SECONDS", " 0 ")])
                .unwrap()
                .auth_cache_seconds,
            0
        );
        let c = load(&[("CONTENT_RUNTIME_DIR", " runtime ")]).unwrap();
        assert!(c.runtime_dir.as_ref().unwrap().is_absolute());
        assert!(c.runtime_dir.unwrap().ends_with("runtime"));
        assert!(load(&[("CONTENT_NEWS_URL", "https://example.com/feed")]).is_err());
        assert_eq!(
            load(&[(
                "CONTENT_NEWS_URL",
                "https://raw.githubusercontent.com/o/r/main/news.json"
            )])
            .unwrap()
            .news_url
            .as_deref(),
            Some("https://raw.githubusercontent.com/o/r/main/news.json")
        );
    }
}
