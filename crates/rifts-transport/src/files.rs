// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The files the launcher writes for a session (`bridge.conf`, `transport.secret` and the one-value secret files):
//! `key=value` lines, read on the session thread.

use std::fs;
use std::io::Read as _;
use std::path::Path;

use crate::link::RelayServer;
use crate::seal::decode_pair_key;
use crate::text::{is_account, safe_str};
use crate::url::{ServerAddr, parse_stun, parse_turn_udp, split_list};

pub const CONFIG_LIMIT: usize = 16384;
pub const CAPABILITY_LIMIT: usize = 128;
pub const TICKET_LIMIT: usize = 4096;
const MAX_KEY: usize = 48;
const MAX_VALUE: usize = 1024;
const MAX_RELAYS: usize = 4;

/// The file's text with trailing CR and LF bytes removed.
pub fn read_input(path: &Path, limit: usize, allow_empty: bool) -> Result<String, String> {
    let regular = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());
    if !regular {
        return Err("input must be a regular non-symlink file".to_owned());
    }
    let mut file = fs::File::open(path).map_err(|_| "input must be a regular non-symlink file".to_owned())?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let bounded = usize::try_from(size).is_ok_and(|s| s <= limit && (allow_empty || s > 0));
    if !bounded {
        return Err("input file is empty or exceeds its size limit".to_owned());
    }
    let mut bytes = Vec::new();
    let read = file.read_to_end(&mut bytes);
    if read.is_err() || u64::try_from(bytes.len()).ok() != Some(size) {
        return Err("input file could not be read completely".to_owned());
    }
    while bytes.last().is_some_and(|b| *b == b'\r' || *b == b'\n') {
        bytes.pop();
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The fields of one file in order. Only `peer` may repeat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields(Vec<(String, String)>);

impl Fields {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut fields: Vec<(String, String)> = Vec::new();
        for line in text.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(at) = line.find('=').filter(|&at| at >= 1 && at + 1 < line.len()) else {
                return Err("configuration contains a malformed line".to_owned());
            };
            let (key, value) = (&line[..at], &line[at + 1..]);
            let repeated = key != "peer" && fields.iter().any(|(k, _)| k == key);
            if !safe_str(key, MAX_KEY, b"_-") || value.len() > MAX_VALUE || value.contains('\0') || repeated {
                return Err("configuration contains an unsafe or duplicate field".to_owned());
            }
            fields.push((key.to_owned(), value.to_owned()));
        }
        Ok(Self(fields))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.0.iter().filter(move |(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// Reads every field of a group and keeps the last failure, which is the one reported.
pub struct Reader<'a> {
    fields: &'a Fields,
    error: Option<String>,
}

impl<'a> Reader<'a> {
    pub fn new(fields: &'a Fields) -> Self {
        Self { fields, error: None }
    }

    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn text(&mut self, key: &str, max: usize, extra: &[u8]) -> String {
        match self.fields.get(key) {
            Some(value) if safe_str(value, max, extra) => value.to_owned(),
            _ => {
                self.fail(format!("missing or invalid configuration field: {key}"));
                String::new()
            }
        }
    }

    pub fn number(&mut self, key: &str, min: u64, max: u64) -> u64 {
        let Some(value) = self.fields.get(key) else {
            self.fail(format!("missing numeric configuration field: {key}"));
            return 0;
        };
        let parsed = if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
            value.parse::<u64>().ok().filter(|n| (min..=max).contains(n))
        } else {
            None
        };
        parsed.unwrap_or_else(|| {
            self.fail(format!("invalid numeric configuration field: {key}"));
            0
        })
    }

    pub fn finish(self) -> Result<(), String> {
        self.error.map_or(Ok(()), Err)
    }
}

pub fn boolean(fields: &Fields, key: &str) -> bool {
    fields.get(key) == Some("true")
}

/// What a session's config binds it to; the ticket must name the same values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings {
    pub key_id: String,
    pub session_id: String,
    pub party_id: String,
    pub hunt_definition_id: String,
    pub bridge_instance_id: String,
    pub party_revision: u64,
    pub host_peer_id: u64,
    pub generation: u32,
    pub build: u32,
    pub bridge_protocol: u16,
    pub virtual_port: u16,
}

// The ranges below bound each value to its type, so the narrowing casts cannot truncate.
#[allow(clippy::cast_possible_truncation)]
pub fn read_bindings(reader: &mut Reader<'_>) -> Bindings {
    Bindings {
        key_id: reader.text("key_id", 32, b"._-"),
        session_id: reader.text("session_id", 36, b"-"),
        party_id: reader.text("party_id", 192, b"._:+/=-"),
        hunt_definition_id: reader.text("hunt_definition_id", 128, b"._:/-"),
        bridge_instance_id: reader.text("bridge_instance_id", 96, b"._:-"),
        party_revision: reader.number("party_revision", 1, u64::MAX),
        host_peer_id: reader.number("host_peer_id", 1, u64::MAX),
        generation: reader.number("generation", 1, u64::from(u32::MAX)) as u32,
        build: reader.number("build", 1, u64::from(u32::MAX)) as u32,
        bridge_protocol: reader.number("bridge_protocol", 1, 65535) as u16,
        virtual_port: reader.number("virtual_port", 1, 999) as u16,
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PeerEntry {
    pub account: String,
    pub peer_id: u64,
    pub pair_key: [u8; 32],
}

impl std::fmt::Debug for PeerEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerEntry")
            .field("account", &self.account)
            .field("peer_id", &self.peer_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IceMaterial {
    pub stun: Option<ServerAddr>,
    pub relays: Vec<RelayServer>,
    pub peers: Vec<PeerEntry>,
}

fn credential_ok(text: &str) -> bool {
    (1..=512).contains(&text.len()) && text.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// The servers from the config and the relay credential and peers from the secrets.
pub fn ice_material(config: &Fields, secrets: &Fields) -> Result<IceMaterial, String> {
    let mut material = IceMaterial {
        stun: config
            .get("ice_stun_urls")
            .and_then(|list| split_list(list).into_iter().find_map(parse_stun)),
        ..IceMaterial::default()
    };
    if let (Some(urls), Some(username), Some(credential)) = (
        config.get("ice_turn_urls"),
        secrets.get("turn_username"),
        secrets.get("turn_credential"),
    ) {
        if !credential_ok(username) || !credential_ok(credential) {
            return Err("invalid relay credential".to_owned());
        }
        for server in split_list(urls).into_iter().filter_map(parse_turn_udp) {
            if material.relays.len() >= MAX_RELAYS {
                break;
            }
            if material.relays.iter().any(|r| r.server == server) {
                continue;
            }
            material.relays.push(RelayServer {
                server,
                username: username.to_owned(),
                credential: credential.to_owned(),
            });
        }
    }
    for value in secrets.all("peer") {
        let items = split_list(value);
        let entry = match items.as_slice() {
            [account, peer_id, key] if is_account(account) => {
                let peer_id = peer_id
                    .bytes()
                    .all(|b| b.is_ascii_digit())
                    .then(|| peer_id.parse::<u64>().ok())
                    .flatten()
                    .filter(|id| *id >= 1);
                match (peer_id, decode_pair_key(key)) {
                    (Some(peer_id), Some(pair_key)) => Some(PeerEntry {
                        account: (*account).to_owned(),
                        peer_id,
                        pair_key,
                    }),
                    _ => None,
                }
            }
            _ => None,
        };
        let Some(entry) = entry else {
            return Err("invalid peer entry in the transport secrets".to_owned());
        };
        if material
            .peers
            .iter()
            .any(|p| p.account == entry.account || p.peer_id == entry.peer_id)
        {
            return Err("duplicate peer entry in the transport secrets".to_owned());
        }
        material.peers.push(entry);
    }
    Ok(material)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn lines() {
        let fields = Fields::parse("# c\r\n\na=1\r\npeer=x\npeer=y\nb==2").unwrap();
        assert_eq!(fields.get("a"), Some("1"));
        assert_eq!(fields.get("b"), Some("=2"));
        assert_eq!(fields.all("peer").count(), 2);
        assert_eq!(
            Fields::parse("=1"),
            Err("configuration contains a malformed line".to_owned())
        );
        assert_eq!(
            Fields::parse("a="),
            Err("configuration contains a malformed line".to_owned())
        );
        assert_eq!(
            Fields::parse("a=1\na=2"),
            Err("configuration contains an unsafe or duplicate field".to_owned())
        );
        assert_eq!(
            Fields::parse("a b=1"),
            Err("configuration contains an unsafe or duplicate field".to_owned())
        );
    }

    #[test]
    fn last_error_wins() {
        let fields = Fields::parse("generation=0\nbuild=x").unwrap();
        let mut reader = Reader::new(&fields);
        let _ = reader.number("generation", 1, 9);
        let _ = reader.number("build", 1, 9);
        let _ = reader.number("party_revision", 1, 9);
        assert_eq!(
            reader.finish(),
            Err("missing numeric configuration field: party_revision".to_owned())
        );
    }

    #[test]
    fn material() {
        let text = format!(
            "ice_stun_urls=stun:stun.cloudflare.com:3478\n\
             ice_turn_urls=turn:turn.cloudflare.com:3478?transport=udp, turns:turn.cloudflare.com:5349?transport=tcp, \
             turn:turn.cloudflare.com:443?transport=udp\n\
             turn_username=user\nturn_credential=secret\n\
             peer=member-0,11,{KEY}\npeer=member-1,12,{KEY}\n"
        );
        let fields = Fields::parse(&text).unwrap();
        let material = ice_material(&fields, &fields).unwrap();
        assert_eq!(material.relays.len(), 2);
        assert_eq!(material.peers.len(), 2);
        assert_eq!(material.stun.unwrap().port, 3478);

        let short = Fields::parse("peer=member-0,11,AAAA").unwrap();
        assert_eq!(
            ice_material(&short, &short),
            Err("invalid peer entry in the transport secrets".to_owned())
        );
        let twice = Fields::parse(&format!("peer=member-0,11,{KEY}\npeer=member-0,12,{KEY}")).unwrap();
        assert_eq!(
            ice_material(&twice, &twice),
            Err("duplicate peer entry in the transport secrets".to_owned())
        );
        let bad = Fields::parse("ice_turn_urls=turn:a\nturn_username=u s\nturn_credential=c").unwrap();
        assert_eq!(ice_material(&bad, &bad), Err("invalid relay credential".to_owned()));
    }
}
