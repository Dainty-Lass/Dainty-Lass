//! The in-game relay's target: the launcher names the server and its certificate's SHA-256, and the game's own
//! backend address (a loopback `http` URL) says which port to listen on. The relay runs in the game, so the game
//! keeps its connection to the server after the launcher closes. Client role only; fail closed.

use crate::config::backend::{BackendUrl, is_dns_name, is_dotted_quad};
use crate::env::Raw;

/// The server's host, as the invite gives it.
pub const HOST_VAR: &str = "DAINTY_SERVER_HOST";
/// The server's port.
pub const PORT_VAR: &str = "DAINTY_SERVER_PORT";
/// The server certificate's SHA-256, 64 hex digits; unset for a private-mode server.
pub const PIN_VAR: &str = "DAINTY_SERVER_FP";
/// The raw reads' cap, in UTF-16 units.
pub const READ_CAP: usize = 255;
/// What the runtime puts in its start line, and the launcher looks for in the runtime's bytes before it trusts the
/// game to carry its own connection.
pub const CAPABILITY_MARKER: &str = "dainty-lass:in-game-relay:v1";

/// Where the relay listens and what it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayTarget {
    /// The server's host, lower-cased.
    pub host: String,
    /// The server's port.
    pub port: u16,
    /// The pin, 64 lowercase hex digits, for a public server.
    pub pin: Option<String>,
    /// The loopback port the game is pointed at.
    pub listen: u16,
}

/// Why the relay is not started. The texts never echo a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The game's backend address is not `http` on loopback, so the relay would not be in its path.
    NotLoopback,
    /// The host is missing, too long or not a host name or IPv4 address.
    BadHost,
    /// The port is not 1 to 65535.
    BadPort,
    /// The pin is not 64 hex digits.
    BadPin,
}

impl Refusal {
    /// The fixed text.
    pub fn text(self) -> &'static str {
        match self {
            Refusal::NotLoopback => "the backend address is not http on 127.0.0.1",
            Refusal::BadHost => "the server host is not valid",
            Refusal::BadPort => "the server port must be 1-65535",
            Refusal::BadPin => "the certificate fingerprint must be 64 hex digits",
        }
    }
}

/// `None` when the launcher named no server (it runs the relay itself, or none is wanted).
pub fn resolve(backend: &BackendUrl, host: &Raw, port: &Raw, pin: &Raw) -> Option<Result<RelayTarget, Refusal>> {
    let host = match host {
        Raw::Unset => return None,
        Raw::TooLong(_) => return Some(Err(Refusal::BadHost)),
        Raw::Value(value) => value.to_ascii_lowercase(),
    };
    Some(target(backend, host, port, pin))
}

fn target(backend: &BackendUrl, host: String, port: &Raw, pin: &Raw) -> Result<RelayTarget, Refusal> {
    if backend.secure || !matches!(backend.host.as_str(), "127.0.0.1" | "localhost") {
        return Err(Refusal::NotLoopback);
    }
    if !is_dotted_quad(&host) && !is_dns_name(&host) {
        return Err(Refusal::BadHost);
    }
    let port = match port {
        Raw::Value(value) if (1..=5).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit()) => value
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(Refusal::BadPort)?,
        _ => return Err(Refusal::BadPort),
    };
    let pin = match pin {
        Raw::Unset => None,
        Raw::Value(value) if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Some(value.to_ascii_lowercase())
        }
        _ => return Err(Refusal::BadPin),
    };
    Ok(RelayTarget {
        host,
        port,
        pin,
        listen: backend.port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::backend::parse_backend_url;

    fn raw(value: Option<&str>) -> Raw {
        Raw::from_text(value, READ_CAP)
    }

    type Resolved = Option<Result<RelayTarget, Refusal>>;

    fn at(backend: &str, host: Option<&str>, port: Option<&str>, pin: Option<&str>) -> Resolved {
        let backend = parse_backend_url(backend).unwrap_or_else(|_| BackendUrl::built_in());
        resolve(&backend, &raw(host), &raw(port), &raw(pin))
    }

    const LOCAL: &str = "http://127.0.0.1:61100";

    #[test]
    fn no_host_is_no_relay() {
        assert_eq!(at(LOCAL, None, Some("443"), None), None);
        assert_eq!(at(LOCAL, Some(""), None, None), None);
    }

    #[test]
    fn a_public_server() {
        let pin = "AB".repeat(32);
        assert_eq!(
            at(LOCAL, Some("Play.Example.org"), Some("443"), Some(&pin)),
            Some(Ok(RelayTarget {
                host: "play.example.org".to_owned(),
                port: 443,
                pin: Some("ab".repeat(32)),
                listen: 61100,
            }))
        );
    }

    #[test]
    fn a_private_server_listening_elsewhere() {
        assert_eq!(
            at("http://localhost:41101", Some("100.64.0.7"), Some("61000"), None),
            Some(Ok(RelayTarget {
                host: "100.64.0.7".to_owned(),
                port: 61000,
                pin: None,
                listen: 41101,
            }))
        );
    }

    #[test]
    fn refusals() {
        let pin = "ab".repeat(32);
        type Case<'a> = (&'a str, Option<&'a str>, Option<&'a str>, Option<&'a str>, Refusal);
        let cases: [Case<'_>; 9] = [
            (
                "https://127.0.0.1:61100",
                Some("a.org"),
                Some("443"),
                Some(&pin),
                Refusal::NotLoopback,
            ),
            (
                "http://10.0.0.1:61100",
                Some("a.org"),
                Some("443"),
                Some(&pin),
                Refusal::NotLoopback,
            ),
            (LOCAL, Some("a b.org"), Some("443"), Some(&pin), Refusal::BadHost),
            (LOCAL, Some("user@a.org"), Some("443"), Some(&pin), Refusal::BadHost),
            (LOCAL, Some("a.org"), None, Some(&pin), Refusal::BadPort),
            (LOCAL, Some("a.org"), Some("0"), Some(&pin), Refusal::BadPort),
            (LOCAL, Some("a.org"), Some("65536"), Some(&pin), Refusal::BadPort),
            (LOCAL, Some("a.org"), Some("443"), Some("abcd"), Refusal::BadPin),
            (
                LOCAL,
                Some("a.org"),
                Some("443"),
                Some(&"zz".repeat(32)),
                Refusal::BadPin,
            ),
        ];
        for (backend, host, port, fp, refusal) in cases {
            assert_eq!(
                at(backend, host, port, fp),
                Some(Err(refusal)),
                "{backend} {host:?} {port:?} {fp:?}"
            );
        }
        let long = "a".repeat(READ_CAP + 1);
        assert_eq!(at(LOCAL, Some(&long), Some("443"), None), Some(Err(Refusal::BadHost)));
    }
}
