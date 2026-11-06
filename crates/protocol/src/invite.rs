// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Invite links (`dauntless://join?...`): parsing, formatting and the game version aliases they carry (RD5).

use std::fmt;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::GameVersion;
use crate::text::{collapse_js_whitespace, has_control_or_format, js_length, js_trim};

/// Scheme of current invites, which may name any played version.
pub const UNIFIED_INVITE_SCHEME: &str = "dauntless";
/// DaintyLass's own scheme, pinned to 1.14.7: what `dainty-host invite` prints for it.
pub const INVITE_SCHEME: &str = "dainty";
/// Dauntless Revived's scheme, recognised only to refuse it (RD5).
pub const REVIVED_INVITE_SCHEME: &str = "dauntless-revived";
/// Port a public invite implies when it names none.
pub const DEFAULT_PUBLIC_PORT: u16 = 443;
/// Longest invite accepted, in UTF-16 code units.
pub const MAX_INVITE_LENGTH: usize = 2048;

const MAX_INVITE_UTF16_LEN: usize = MAX_INVITE_LENGTH;
const MAX_NAME_CHARS: usize = 64;
const MAX_SHARE_UTF16_LEN: usize = 512;
const TAILSCALE_SHARE_PREFIX: &str = "https://login.tailscale.com/";

/// Returns the default Reforged game version (`1.14.7`).
#[must_use]
pub fn reforged_version() -> GameVersion {
    GameVersion::release(1, 14, 7)
}

/// Dauntless Revived's 1.4.4: never played, only refused with its own error (RD5).
fn revived_version() -> GameVersion {
    GameVersion::release(1, 4, 4)
}

/// Whether `version` is Dauntless Revived's 1.4.4, which is recognised only to be refused (RD5).
#[must_use]
pub fn is_revived(version: &GameVersion) -> bool {
    *version == revived_version()
}

/// Returns the game versions this build plays.
#[must_use]
pub fn played_versions() -> Vec<GameVersion> {
    vec![reforged_version()]
}

/// Resolves a version string or known alias (`"1.14.7"`, `"reforged"`, `"1.4.4"`, `"revived"`) to a [`GameVersion`].
#[must_use]
pub fn named_version(text: &str) -> Option<GameVersion> {
    match text {
        "1.14.7" | "reforged" => Some(reforged_version()),
        "1.4.4" | "revived" => Some(revived_version()),
        _ => None,
    }
}

/// Returns the short version alias (`"reforged"`) for `version`, if one exists.
#[must_use]
pub fn version_alias(version: &GameVersion) -> Option<&'static str> {
    (*version == reforged_version()).then_some("reforged")
}

/// Returns the URL schemes that `parse_invite` accepts.
#[must_use]
pub fn invite_schemes() -> Vec<&'static str> {
    vec![UNIFIED_INVITE_SCHEME, INVITE_SCHEME]
}

/// How the launcher reaches the server named in an invite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ServerMode {
    /// Loopback or Tailscale address, invite format v1, no certificate pin.
    Private,
    /// Public host behind the gateway, invite format v2, pinned by certificate fingerprint.
    Public,
}

/// A parsed invite, every field already validated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Invite {
    /// Game version the server plays, 1.14.7 when the invite names none.
    #[serde(default = "reforged_version")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub version: GameVersion,
    /// Private or public, from the invite format.
    pub mode: ServerMode,
    /// Host name or IP address, lowercased.
    pub host: String,
    /// TCP port of the server (the gateway's in public mode).
    pub port: u16,
    /// Invite code to register with.
    pub code: String,
    /// Server name for display, cleaned of control characters.
    pub name: String,
    /// Tailscale share link (`https://login.tailscale.com/...`), private invites only.
    pub share: Option<String>,
    /// SHA-256 of the gateway certificate, 64 lowercase hex digits; public invites only.
    pub fp: Option<String>,
}

/// Why [`parse_invite`] refused a link, serialized as the snake_case code the launcher translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteError {
    /// The text is empty or only whitespace.
    Empty,
    /// Longer than [`MAX_INVITE_LENGTH`].
    TooLong,
    /// Not a `join` link in a recognised scheme.
    NotInvite,
    /// A Dauntless Revived invite, which this build does not play (RD5).
    Revived,
    /// The `v` format parameter is missing or unknown.
    Version,
    /// The invite names a game version this build does not play.
    GameVersion,
    /// `mode` is missing or not `public` in a v2 invite.
    Mode,
    /// `fp` is missing or not a SHA-256 fingerprint.
    Fp,
    /// `host` is missing or not allowed for the invite's mode.
    Host,
    /// `port` is missing or out of range.
    Port,
    /// `code` is missing or malformed.
    Code,
    /// `name` is missing or empty after cleaning.
    Name,
    /// `share` is not a Tailscale share link.
    Share,
    /// The query is malformed, repeats a key, or carries one the format does not allow.
    Params,
}

impl InviteError {
    /// The wire code, as serialized.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooLong => "too_long",
            Self::NotInvite => "not_invite",
            Self::Revived => "revived",
            Self::Version => "version",
            Self::GameVersion => "game_version",
            Self::Mode => "mode",
            Self::Fp => "fp",
            Self::Host => "host",
            Self::Port => "port",
            Self::Code => "code",
            Self::Name => "name",
            Self::Share => "share",
            Self::Params => "params",
        }
    }
}

/// Why [`format_invite`] could not write an [`Invite`]; names the offending field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// The host is not allowed for the invite's mode.
    Host,
    /// The port is zero.
    Port,
    /// The invite code is malformed.
    Code,
    /// The name is empty after cleaning.
    Name,
    /// The share link is not a Tailscale share link, or is set on a public invite.
    Share,
    /// The fingerprint is missing or malformed in public mode.
    Fp,
    /// This build cannot write an invite for the game version.
    Version,
}

impl FormatError {
    /// The field's name, as used in messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Port => "port",
            Self::Code => "code",
            Self::Name => "name",
            Self::Share => "share",
            Self::Fp => "fp",
            Self::Version => "version",
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for FormatError {}

fn parse_ipv4_dotted(host: &str) -> Option<[u8; 4]> {
    let mut parts = host.split('.');
    let a = parse_octet(parts.next()?)?;
    let b = parse_octet(parts.next()?)?;
    let c = parse_octet(parts.next()?)?;
    let d = parse_octet(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some([a, b, c, d])
}

fn parse_octet(s: &str) -> Option<u8> {
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u8>().ok()
}

/// Returns `true` if `text` is a canonical dotted-quad IPv4 address.
#[must_use]
pub fn is_valid_ipv4(text: &str) -> bool {
    parse_ipv4_dotted(text).is_some()
}

fn looks_numeric_or_ipv4_candidate(host: &str) -> bool {
    host.split('.').next_back().is_some_and(|last| {
        !last.is_empty()
            && (last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x") || last.starts_with("0X"))
    })
}

/// Returns `true` if `host` is a valid lowercase DNS hostname or canonical dotted-quad IPv4 address.
#[must_use]
pub fn is_valid_host(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if is_valid_ipv4(host) {
        return true;
    }
    if looks_numeric_or_ipv4_candidate(host) {
        return false;
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return false;
        }
    }
    true
}

/// Returns `true` if `host` is a canonical dotted-quad IPv4 address in the Tailscale CGNAT range (`100.64.0.0/10`).
#[must_use]
pub fn is_tailscale_ipv4(host: &str) -> bool {
    matches!(parse_ipv4_dotted(host), Some([100, b, _, _]) if (64..=127).contains(&b))
}

/// Returns `true` if `host` is `"localhost"` or a canonical `127.0.0.0/8` IPv4 address.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || matches!(parse_ipv4_dotted(host), Some([127, _, _, _]))
}

/// Returns `true` if `host` is a valid Tailscale MagicDNS hostname (`*.ts.net`).
#[must_use]
pub fn is_magic_dns_name(host: &str) -> bool {
    is_valid_host(host) && host.strip_suffix(".ts.net").is_some_and(|prefix| !prefix.is_empty())
}

/// Returns `true` if `host` is an allowed private-mode (Tailscale / loopback) host.
#[must_use]
pub fn is_private_mode_host(host: &str) -> bool {
    if !is_valid_host(host) {
        return false;
    }
    is_loopback_host(host) || is_tailscale_ipv4(host) || is_magic_dns_name(host)
}

/// Returns `true` if `fp` is a valid 64-character lowercase hexadecimal SHA-256 fingerprint.
#[must_use]
pub fn is_valid_fingerprint(fp: &str) -> bool {
    fp.len() == 64 && fp.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Returns `true` if `code` is a valid invite code (`^[A-Za-z0-9-]{4,64}$`).
#[must_use]
pub fn is_valid_invite_code(code: &str) -> bool {
    (4..=64).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Returns `true` if `text` is a valid Tailscale share URL (`https://login.tailscale.com/...`).
#[must_use]
pub fn is_valid_share_url(text: &str) -> bool {
    if text.is_empty() || js_length(text) > MAX_SHARE_UTF16_LEN || has_control_or_format(text) {
        return false;
    }
    if text.len() <= TAILSCALE_SHARE_PREFIX.len()
        || !text[..TAILSCALE_SHARE_PREFIX.len()].eq_ignore_ascii_case(TAILSCALE_SHARE_PREFIX)
    {
        return false;
    }
    let Ok(url) = Url::parse(text) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("login.tailscale.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().len() > 1
}

/// Validates and normalizes an invite server name, returning `None` if it contains control/bidi
/// characters, is empty after collapsing whitespace, or exceeds 64 characters.
#[must_use]
pub fn clean_server_name(raw: &str) -> Option<String> {
    if has_control_or_format(raw) {
        return None;
    }
    let name = collapse_js_whitespace(raw);
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return None;
    }
    Some(name)
}

fn parse_port(raw: &str) -> Result<u16, InviteError> {
    if raw.is_empty() || (raw.len() > 1 && raw.starts_with('0')) || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(InviteError::Port);
    }
    let port = raw.parse::<u16>().map_err(|_| InviteError::Port)?;
    if port == 0 {
        return Err(InviteError::Port);
    }
    Ok(port)
}

/// Returns `true` if `text` is a canonical decimal port number in `1..=65535` without leading zeros.
#[must_use]
pub fn is_valid_port(text: &str) -> bool {
    parse_port(text).is_ok()
}

fn decode_form_component(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hi = *bytes.get(i + 1)?;
                let lo = *bytes.get(i + 2)?;
                let h = (hi as char).to_digit(16)?;
                let l = (lo as char).to_digit(16)?;
                out.push(((h << 4) | l) as u8);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn resolve_game_version_token(raw: &str, supported: &[GameVersion]) -> Option<GameVersion> {
    if let Some(v) = named_version(raw) {
        return Some(v);
    }
    let parsed = raw.parse::<GameVersion>().ok()?;
    supported.contains(&parsed).then_some(parsed)
}

/// Parses an invite URL using the crate's compiled-in supported game versions.
pub fn parse_invite(raw: &str) -> Result<Invite, InviteError> {
    parse_invite_for(raw, &played_versions())
}

/// Parses an invite URL against the caller's `supported` game versions.
pub fn parse_invite_for(raw: &str, supported: &[GameVersion]) -> Result<Invite, InviteError> {
    let trimmed = js_trim(raw);
    if trimmed.is_empty() {
        return Err(InviteError::Empty);
    }
    if js_length(trimmed) > MAX_INVITE_UTF16_LEN {
        return Err(InviteError::TooLong);
    }

    let Some((scheme_raw, after_scheme)) = trimmed.split_once("://") else {
        return Err(InviteError::NotInvite);
    };
    let scheme = scheme_raw.to_ascii_lowercase();
    let scheme_pinned_version = match scheme.as_str() {
        UNIFIED_INVITE_SCHEME => None,
        INVITE_SCHEME => Some(reforged_version()),
        REVIVED_INVITE_SCHEME => Some(revived_version()),
        _ => return Err(InviteError::NotInvite),
    };

    let Some((path_raw, query_raw)) = after_scheme.split_once('?') else {
        return Err(InviteError::NotInvite);
    };
    let path = path_raw.strip_suffix('/').unwrap_or(path_raw);
    if !path.eq_ignore_ascii_case("join") {
        return Err(InviteError::NotInvite);
    }

    if let Some(ref pinned) = scheme_pinned_version
        && *pinned == revived_version()
        && !supported.contains(pinned)
    {
        return Err(InviteError::Revived);
    }

    if query_raw.is_empty() || query_raw.contains('#') {
        return Err(InviteError::Params);
    }

    let mut p_v: Option<String> = None;
    let mut p_mode: Option<String> = None;
    let mut p_host: Option<String> = None;
    let mut p_port: Option<String> = None;
    let mut p_code: Option<String> = None;
    let mut p_name: Option<String> = None;
    let mut p_share: Option<String> = None;
    let mut p_fp: Option<String> = None;
    let mut p_version: Option<String> = None;

    for pair in query_raw.split('&') {
        if pair.is_empty() {
            return Err(InviteError::Params);
        }
        let Some((k_raw, v_raw)) = pair.split_once('=') else {
            return Err(InviteError::Params);
        };
        if k_raw.is_empty() || k_raw.contains('%') || k_raw.contains('+') {
            return Err(InviteError::Params);
        }
        let val = decode_form_component(v_raw).ok_or(InviteError::Params)?;
        let slot = match k_raw {
            "v" => &mut p_v,
            "mode" => &mut p_mode,
            "host" => &mut p_host,
            "port" => &mut p_port,
            "code" => &mut p_code,
            "name" => &mut p_name,
            "share" => &mut p_share,
            "fp" => &mut p_fp,
            "version" => &mut p_version,
            _ => return Err(InviteError::Params),
        };
        if slot.is_some() {
            return Err(InviteError::Params);
        }
        *slot = Some(val);
    }

    let version = match (scheme_pinned_version, p_version) {
        (Some(pinned), Some(explicit_raw)) => {
            let Some(explicit_ver) = resolve_game_version_token(&explicit_raw, supported) else {
                return Err(InviteError::Params);
            };
            if explicit_ver != pinned {
                return Err(InviteError::Params);
            }
            pinned
        }
        (Some(pinned), None) => pinned,
        (None, Some(explicit_raw)) => {
            let Some(explicit_ver) = resolve_game_version_token(&explicit_raw, supported) else {
                return Err(InviteError::Params);
            };
            if explicit_ver == revived_version() && !supported.contains(&explicit_ver) {
                return Err(InviteError::Revived);
            }
            explicit_ver
        }
        (None, None) => reforged_version(),
    };

    if !supported.contains(&version) {
        if version == revived_version() {
            return Err(InviteError::Revived);
        }
        return Err(InviteError::Params);
    }

    let v_str = p_v.ok_or(InviteError::Version)?;
    match v_str.as_str() {
        "1" => {
            if p_mode.is_some() || p_fp.is_some() {
                return Err(InviteError::Params);
            }
            let host_raw = p_host.ok_or(InviteError::Host)?;
            let host = js_trim(&host_raw).to_lowercase();
            if !is_private_mode_host(&host) {
                return Err(InviteError::Host);
            }
            let port_raw = p_port.ok_or(InviteError::Port)?;
            let port = parse_port(js_trim(&port_raw))?;
            let code_raw = p_code.ok_or(InviteError::Code)?;
            let code = js_trim(&code_raw).to_string();
            if !is_valid_invite_code(&code) {
                return Err(InviteError::Code);
            }
            let name_raw = p_name.ok_or(InviteError::Name)?;
            let name = clean_server_name(&name_raw).ok_or(InviteError::Name)?;
            let share = match p_share {
                Some(s_raw) => {
                    let s = js_trim(&s_raw).to_string();
                    if !is_valid_share_url(&s) {
                        return Err(InviteError::Share);
                    }
                    Some(s)
                }
                None => None,
            };

            Ok(Invite {
                version,
                mode: ServerMode::Private,
                host,
                port,
                code,
                name,
                share,
                fp: None,
            })
        }
        "2" => {
            if p_share.is_some() {
                return Err(InviteError::Params);
            }
            let mode_raw = p_mode.ok_or(InviteError::Mode)?;
            if mode_raw != "public" {
                return Err(InviteError::Mode);
            }
            let fp_raw = p_fp.ok_or(InviteError::Fp)?;
            let fp = js_trim(&fp_raw).to_string();
            if !is_valid_fingerprint(&fp) {
                return Err(InviteError::Fp);
            }
            let host_raw = p_host.ok_or(InviteError::Host)?;
            let host = js_trim(&host_raw).to_lowercase();
            if !is_valid_host(&host) {
                return Err(InviteError::Host);
            }
            let port = match p_port {
                Some(port_raw) => parse_port(js_trim(&port_raw))?,
                None => 443,
            };
            let code_raw = p_code.ok_or(InviteError::Code)?;
            let code = js_trim(&code_raw).to_string();
            if !is_valid_invite_code(&code) {
                return Err(InviteError::Code);
            }
            let name_raw = p_name.ok_or(InviteError::Name)?;
            let name = clean_server_name(&name_raw).ok_or(InviteError::Name)?;

            Ok(Invite {
                version,
                mode: ServerMode::Public,
                host,
                port,
                code,
                name,
                share: None,
                fp: Some(fp),
            })
        }
        _ => Err(InviteError::Version),
    }
}

const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn encode_query_val(s: &str) -> String {
    utf8_percent_encode(s, QUERY_ENCODE_SET).to_string()
}

/// Formats a validated [`Invite`] into its canonical invite URL.
pub fn format_invite(invite: &Invite) -> Result<String, FormatError> {
    if invite.port == 0 {
        return Err(FormatError::Port);
    }
    let code = js_trim(&invite.code);
    if !is_valid_invite_code(code) {
        return Err(FormatError::Code);
    }
    let name = clean_server_name(&invite.name).ok_or(FormatError::Name)?;

    if !played_versions().contains(&invite.version) {
        return Err(FormatError::Version);
    }

    let (scheme, include_version) = if invite.version == reforged_version() {
        (INVITE_SCHEME, false)
    } else {
        (UNIFIED_INVITE_SCHEME, true)
    };

    let mut params = Vec::with_capacity(8);
    match invite.mode {
        ServerMode::Private => {
            if invite.fp.is_some() {
                return Err(FormatError::Fp);
            }
            let host = js_trim(&invite.host).to_lowercase();
            if !is_private_mode_host(&host) {
                return Err(FormatError::Host);
            }
            params.push("v=1".to_string());
            if include_version {
                params.push(format!("version={}", encode_query_val(&invite.version.to_string())));
            }
            params.push(format!("host={}", encode_query_val(&host)));
            params.push(format!("port={}", invite.port));
            params.push(format!("code={}", encode_query_val(code)));
            params.push(format!("name={}", encode_query_val(&name)));
            if let Some(ref s_raw) = invite.share {
                let s = js_trim(s_raw);
                if !is_valid_share_url(s) {
                    return Err(FormatError::Share);
                }
                params.push(format!("share={}", encode_query_val(s)));
            }
        }
        ServerMode::Public => {
            if invite.share.is_some() {
                return Err(FormatError::Share);
            }
            let fp_raw = invite.fp.as_deref().ok_or(FormatError::Fp)?;
            let fp = js_trim(fp_raw);
            if !is_valid_fingerprint(fp) {
                return Err(FormatError::Fp);
            }
            let host = js_trim(&invite.host).to_lowercase();
            if !is_valid_host(&host) {
                return Err(FormatError::Host);
            }
            params.push("v=2".to_string());
            if include_version {
                params.push(format!("version={}", encode_query_val(&invite.version.to_string())));
            }
            params.push("mode=public".to_string());
            params.push(format!("host={}", encode_query_val(&host)));
            params.push(format!("port={}", invite.port));
            params.push(format!("fp={}", encode_query_val(fp)));
            params.push(format!("code={}", encode_query_val(code)));
            params.push(format!("name={}", encode_query_val(&name)));
        }
    }

    let out = format!("{scheme}://join?{}", params.join("&"));
    if js_length(&out) > MAX_INVITE_UTF16_LEN {
        return Err(FormatError::Name);
    }
    Ok(out)
}
