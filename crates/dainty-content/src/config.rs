// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `content.env`, as the host kit writes it. Settings are read in a fixed order, so the first bad one is the
//! error a host sees.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use dainty_module_api::GameVersion;
use dainty_service_kit::env::Env;
use dainty_service_kit::js;
use serde_json::Value;

/// Port served on when `PORT` is unset.
pub const DEFAULT_PORT: u16 = 41002;
/// Host bound when `BIND_HOST` is unset.
pub const DEFAULT_BIND: &str = "127.0.0.1";
/// Backend reached when `BACKEND_URL` is unset.
pub const DEFAULT_BACKEND: &str = "http://127.0.0.1:41000";

/// The content server's settings, from `content.env`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Game version served (`CONTENT_GAME`).
    pub version: GameVersion,
    /// TCP port (`PORT`).
    pub port: u16,
    /// Loopback hosts to bind (`BIND_HOST`).
    pub bind_hosts: Vec<String>,
    /// Backend base URL for account lookups (`BACKEND_URL`).
    pub backend_url: String,
    /// Folder holding the runtime and `runtime.json` (`CONTENT_RUNTIME_DIR`).
    pub runtime_dir: Option<PathBuf>,
    /// Art pack folder (`CONTENT_BRANDING_DIR`).
    pub branding_dir: Option<PathBuf>,
    /// News file (`CONTENT_NEWS_FILE`).
    pub news_file: Option<PathBuf>,
    /// GitHub raw URL of the news, which wins over the file once fetched (`CONTENT_NEWS_URL`).
    pub news_url: Option<String>,
    /// Concurrent downloads one account may hold, 1 to 64 (`CONTENT_MAX_STREAMS_PER_ACCOUNT`).
    pub max_streams_per_account: usize,
    /// Concurrent downloads in all, 1 to 1024 (`CONTENT_MAX_STREAMS_TOTAL`).
    pub max_streams_total: usize,
    /// Seconds an account lookup stays cached, 0 to 3600 (`CONTENT_AUTH_CACHE_SECONDS`).
    pub auth_cache_seconds: u64,
}

impl Config {
    /// Reads the settings; the error names the first bad one.
    pub fn from_env(env: &Env) -> Result<Config, String> {
        let version = game(env.raw("CONTENT_GAME"))?;
        let port = whole(env, "PORT", f64::from(DEFAULT_PORT), 1.0, 65535.0)? as u16;
        let bind_hosts = bind_hosts(env.raw("BIND_HOST"))?;
        let backend_url = backend_url(env.raw("BACKEND_URL"))?;
        let news_url = match env.raw("CONTENT_NEWS_URL").map(js::trim).filter(|v| !v.is_empty()) {
            None => None,
            Some(text) => {
                Some(news_url(text).ok_or("CONTENT_NEWS_URL must be a raw.githubusercontent.com HTTPS file URL")?)
            }
        };
        let runtime_dir = folder(env.raw("CONTENT_RUNTIME_DIR"));
        let branding_dir = folder(env.raw("CONTENT_BRANDING_DIR"));
        let news_file = folder(env.raw("CONTENT_NEWS_FILE"));
        let max_streams_per_account = whole(env, "CONTENT_MAX_STREAMS_PER_ACCOUNT", 4.0, 1.0, 64.0)? as usize;
        let max_streams_total = whole(env, "CONTENT_MAX_STREAMS_TOTAL", 48.0, 1.0, 1024.0)? as usize;
        let auth_cache_seconds = whole(env, "CONTENT_AUTH_CACHE_SECONDS", 300.0, 0.0, 3600.0)? as u64;
        Ok(Config {
            version,
            port,
            bind_hosts,
            backend_url,
            runtime_dir,
            branding_dir,
            news_file,
            news_url,
            max_streams_per_account,
            max_streams_total,
            auth_cache_seconds,
        })
    }
}

/// The address a bind host listens on: `localhost` is IPv4 loopback only, as Node bound it.
pub fn bind_address(host: &str) -> Option<IpAddr> {
    if host == "localhost" {
        return Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    host.parse().ok()
}

/// `localhost`, a strict dotted `127.x.y.z`, or `::1`; nothing else, not even `[::1]`.
pub fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host == "::1" || host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.octets()[0] == 127)
}

fn game(raw: Option<&str>) -> Result<GameVersion, String> {
    let value = raw.map(|r| js::trim(r).to_ascii_lowercase()).unwrap_or_default();
    match value.as_str() {
        "" | "reborn" => Ok(dainty_protocol::invite::dainty_version()),
        "legacy" => Err(
            "CONTENT_GAME=legacy (Dauntless Revived's 1.4.4) is no longer served; this content server serves reborn only"
                .to_owned(),
        ),
        _ => Err(format!(
            "CONTENT_GAME must be reborn (got {})",
            dainty_service_kit::jsjson::stringify(&Value::String(raw.unwrap_or_default().to_owned()))
        )),
    }
}

/// Unset or blank is the default; otherwise `Number(value)` must be whole and within the bounds.
fn whole(env: &Env, name: &str, default: f64, min: f64, max: f64) -> Result<f64, String> {
    let Some(raw) = env.raw(name).filter(|r| !js::trim(r).is_empty()) else {
        return Ok(default);
    };
    let n = js::number(Some(&Value::String(raw.to_owned())));
    if n.is_finite() && n.fract() == 0.0 && (min..=max).contains(&n) {
        Ok(n)
    } else {
        Err(format!("{name} must be a whole number from {min} to {max}"))
    }
}

fn bind_hosts(raw: Option<&str>) -> Result<Vec<String>, String> {
    let value = raw.map(js::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_BIND);
    let hosts: Vec<String> = value
        .split(',')
        .map(js::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_owned)
        .collect();
    if hosts.is_empty() {
        return Err("BIND_HOST is empty".to_owned());
    }
    if let Some(bad) = hosts.iter().find(|h| !is_loopback_host(h)) {
        return Err(format!(
            "BIND_HOST must be a loopback address (got {}): players reach the content server through the gateway",
            dainty_service_kit::jsjson::stringify(&Value::String(bad.clone()))
        ));
    }
    Ok(hosts)
}

fn backend_url(raw: Option<&str>) -> Result<String, String> {
    let value = raw.map(js::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_BACKEND);
    let text = value.trim_end_matches('/').to_owned();
    let url = url::Url::parse(&text).map_err(|_| "BACKEND_URL is not a URL".to_owned())?;
    let host = url.host_str().unwrap_or_default().replace(['[', ']'], "");
    if url.scheme() != "http" || !is_loopback_host(&host) {
        // The account keys travel to it in a header
        return Err("BACKEND_URL must be http:// on this machine (the account keys travel in it)".to_owned());
    }
    Ok(text)
}

/// A raw GitHub file: https, that host, no port, credentials, query or fragment, `/owner/repo/branch/file`.
fn news_url(text: &str) -> Option<String> {
    let url = url::Url::parse(text).ok()?;
    let ok = url.scheme() == "https"
        && url.host_str() == Some("raw.githubusercontent.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().split('/').count() >= 5;
    ok.then(|| url.to_string())
}

fn folder(raw: Option<&str>) -> Option<PathBuf> {
    let text = raw.map(js::trim).filter(|v| !v.is_empty())?;
    Some(std::path::absolute(text).unwrap_or_else(|_| PathBuf::from(text)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]
    use super::*;

    fn config(pairs: &[(&str, &str)]) -> Result<Config, String> {
        Config::from_env(&Env::from_pairs(pairs.iter().copied()))
    }

    #[test]
    fn c1_defaults() {
        let c = config(&[]).unwrap();
        assert_eq!(c.version, dainty_protocol::invite::dainty_version());
        assert_eq!(c.port, 41002);
        assert_eq!(c.bind_hosts, ["127.0.0.1"]);
        assert_eq!(c.backend_url, "http://127.0.0.1:41000");
        assert_eq!(
            (c.runtime_dir, c.branding_dir, c.news_file, c.news_url),
            (None, None, None, None)
        );
        assert_eq!(
            (c.max_streams_per_account, c.max_streams_total, c.auth_cache_seconds),
            (4, 48, 300)
        );
    }

    #[test]
    fn c2_bind_hosts_are_loopback_only() {
        for bad in [
            "0.0.0.0",
            "100.101.102.103",
            "203.0.113.7",
            "::",
            "my-host.example",
            "[::1]",
            "127.0.0.01",
        ] {
            let error = config(&[("BIND_HOST", bad)]).unwrap_err();
            assert!(error.contains("loopback"), "{bad}: {error}");
        }
        assert_eq!(
            config(&[("BIND_HOST", "0.0.0.0")]).unwrap_err(),
            "BIND_HOST must be a loopback address (got \"0.0.0.0\"): players reach the content server through the gateway"
        );
        assert_eq!(
            config(&[("BIND_HOST", "127.0.0.1,::1")]).unwrap().bind_hosts,
            ["127.0.0.1", "::1"]
        );
        assert_eq!(
            config(&[("BIND_HOST", " localhost , 127.0.0.2 ")]).unwrap().bind_hosts,
            ["localhost", "127.0.0.2"]
        );
        assert_eq!(config(&[("BIND_HOST", " , ")]).unwrap_err(), "BIND_HOST is empty");
        assert_eq!(config(&[("BIND_HOST", "  ")]).unwrap().bind_hosts, ["127.0.0.1"]);
        assert_eq!(bind_address("localhost"), Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert_eq!(bind_address("::1"), Some("::1".parse().unwrap()));
    }

    #[test]
    fn c3_backend_url_is_http_on_this_machine() {
        for bad in ["http://203.0.113.7:61000", "https://127.0.0.1:61000", "ftp://127.0.0.1"] {
            assert!(
                config(&[("BACKEND_URL", bad)]).unwrap_err().contains("this machine"),
                "{bad}"
            );
        }
        assert_eq!(
            config(&[("BACKEND_URL", "not a url")]).unwrap_err(),
            "BACKEND_URL is not a URL"
        );
        assert_eq!(
            config(&[("BACKEND_URL", "http://127.0.0.1:61000///")])
                .unwrap()
                .backend_url,
            "http://127.0.0.1:61000"
        );
        for good in ["http://[::1]:61000", "http://localhost:61000", "http://127.5.6.7"] {
            assert_eq!(config(&[("BACKEND_URL", good)]).unwrap().backend_url, good);
        }
    }

    #[test]
    fn c4_game() {
        assert!(config(&[("CONTENT_GAME", " REBORN ")]).is_ok());
        assert!(config(&[("CONTENT_GAME", "")]).is_ok());
        assert_eq!(
            config(&[("CONTENT_GAME", "1.4.4")]).unwrap_err(),
            "CONTENT_GAME must be reborn (got \"1.4.4\")"
        );
        assert!(
            config(&[("CONTENT_GAME", " Legacy")])
                .unwrap_err()
                .contains("no longer served")
        );
    }

    #[test]
    fn c5_numbers_and_folders() {
        assert_eq!(
            config(&[("PORT", "0")]).unwrap_err(),
            "PORT must be a whole number from 1 to 65535"
        );
        assert_eq!(
            config(&[("CONTENT_MAX_STREAMS_PER_ACCOUNT", "65")]).unwrap_err(),
            "CONTENT_MAX_STREAMS_PER_ACCOUNT must be a whole number from 1 to 64"
        );
        assert!(config(&[("CONTENT_MAX_STREAMS_TOTAL", "1.5")]).is_err());
        assert!(config(&[("PORT", "abc")]).is_err());
        assert_eq!(
            config(&[("CONTENT_AUTH_CACHE_SECONDS", " 0 ")])
                .unwrap()
                .auth_cache_seconds,
            0
        );
        assert_eq!(config(&[("PORT", "1e1")]).unwrap().port, 10);
        assert_eq!(config(&[("PORT", "0x10")]).unwrap().port, 16);
        assert_eq!(config(&[("PORT", " ")]).unwrap().port, 41002);
        let dir = config(&[("CONTENT_RUNTIME_DIR", " runtime ")])
            .unwrap()
            .runtime_dir
            .unwrap();
        assert!(dir.is_absolute() && dir.ends_with("runtime"), "{dir:?}");
        // The first bad setting in order is the one reported
        assert!(
            config(&[("PORT", "0"), ("BIND_HOST", "0.0.0.0")])
                .unwrap_err()
                .starts_with("PORT")
        );
    }

    #[test]
    fn c6_news_url() {
        let ok = config(&[(
            "CONTENT_NEWS_URL",
            "https://raw.githubusercontent.com/o/r/main/news.json",
        )])
        .unwrap();
        assert_eq!(
            ok.news_url.as_deref(),
            Some("https://raw.githubusercontent.com/o/r/main/news.json")
        );
        for bad in [
            "https://example.com/feed",
            "http://raw.githubusercontent.com/o/r/main/news.json",
            "https://raw.githubusercontent.com@127.0.0.1/a/b/c/d",
            "https://raw.githubusercontent.com/o/r/main/news.json?token=1",
            "https://raw.githubusercontent.com/o/r/main/news.json?",
            "https://raw.githubusercontent.com/o/r/main",
            "https://raw.githubusercontent.com:444/o/r/main/news.json",
            "https://raw.githubusercontent.com/o/r/main/news.json#x",
        ] {
            assert_eq!(
                config(&[("CONTENT_NEWS_URL", bad)]).unwrap_err(),
                "CONTENT_NEWS_URL must be a raw.githubusercontent.com HTTPS file URL",
                "{bad}"
            );
        }
        // The parser drops the default port before it is looked at (kept for parity)
        assert!(
            config(&[(
                "CONTENT_NEWS_URL",
                "https://raw.githubusercontent.com:443/o/r/main/n.json"
            )])
            .is_ok()
        );
    }

    #[test]
    fn dr_settings_are_ignored() {
        let c = config(&[
            ("CONTENT_GAME_DIR", "x"),
            ("CONTENT_MANIFEST", "y"),
            ("CONTENT_DOWNLOAD_BASE_URL", "z"),
            ("CONTENT_ALLOW_ANY_BIND", "1"),
        ]);
        assert!(c.is_ok());
    }
}
