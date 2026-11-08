// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Invite links from the OS: the link in the launcher's arguments, the invite schemes registered for this
//! executable, and the per-user channel a second launcher hands its arguments over (spec §2.7.8).

mod channel;
mod schemes;

use std::ffi::OsStr;

use dainty_protocol::invite::{INVITE_SCHEME, InviteError, REVIVED_INVITE_SCHEME, invite_schemes, parse_invite};
use dainty_protocol::text::js_trim;

pub use channel::{
    Channel, Claim, Holder, MAX_MESSAGE_ARGS, MAX_MESSAGE_BYTES, READ_TIMEOUT, channel_id, claim, decode_message,
    encode_message, pipe_name, socket_path,
};
pub use schemes::{
    DESKTOP_FILE, LEGACY_DESKTOP_FILE, Registered, RegistryValue, desktop_entry, exec_quote, reg_add_args,
    register_linux, register_schemes, register_windows, registry_values,
};

/// The client's base channel id; kept so a launcher already running from an older install still takes the link.
pub const CLIENT_ID: &str = "io.github.freezamotoqueiro.dauntlessreforgedrevival";
/// The manager's base channel id.
pub const MANAGER_ID: &str = "com.daintylass.server";

/// What the arguments say about an invite link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkArg {
    /// No argument carries a link.
    None,
    /// The first link argument, trimmed; it parses as an invite.
    Invite(String),
    /// The first link argument did not parse, or the only link was a refused scheme (RD5).
    Refused(InviteError),
}

/// Scans start-up or second-launcher arguments: the first one starting (any case) with an invite scheme and `:`.
#[must_use]
pub fn scan_args<S: AsRef<OsStr>>(args: &[S]) -> LinkArg {
    let schemes = invite_schemes();
    let texts = || args.iter().filter_map(|arg| arg.as_ref().to_str());
    if let Some(text) = texts().find(|text| schemes.iter().any(|scheme| has_scheme(text, scheme))) {
        let trimmed = js_trim(text);
        return match parse_invite(trimmed) {
            Ok(_) => LinkArg::Invite(trimmed.to_owned()),
            Err(error) => LinkArg::Refused(error),
        };
    }
    // Never kept: named only so the page and the log can say why the link did nothing.
    if texts().any(|text| has_scheme(text, REVIVED_INVITE_SCHEME)) {
        return LinkArg::Refused(InviteError::Revived);
    }
    LinkArg::None
}

/// The invite link the arguments carry, if one parses.
#[must_use]
pub fn invite_in_args<S: AsRef<OsStr>>(args: &[S]) -> Option<String> {
    match scan_args(args) {
        LinkArg::Invite(link) => Some(link),
        LinkArg::None | LinkArg::Refused(_) => None,
    }
}

fn has_scheme(text: &str, scheme: &str) -> bool {
    let (bytes, len) = (text.as_bytes(), scheme.len());
    bytes.len() > len && bytes[..len].eq_ignore_ascii_case(scheme.as_bytes()) && bytes[len] == b':'
}

/// The registry description each scheme gets.
#[must_use]
pub fn scheme_label(scheme: &str) -> &'static str {
    if scheme == INVITE_SCHEME {
        "URL:Dainty Lass invite"
    } else {
        "URL:Dauntless invite"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f";

    fn private(scheme: &str) -> String {
        format!("{scheme}://join?v=1&host=100.101.102.103&port=61000&code=ABCD-1234&name=Test")
    }

    fn public() -> String {
        format!("dauntless://join?v=2&mode=public&host=203.0.113.10&port=443&fp={FP}&code=ABCD-EFGH-JKLM&name=Test")
    }

    #[test]
    fn invites_valid_for_the_tables() {
        assert!(parse_invite(&private("dainty")).is_ok(), "fixture must parse");
        assert!(parse_invite(&public()).is_ok(), "fixture must parse");
    }

    #[test]
    fn finds_the_first_link_argument() {
        let link = private("dainty");
        assert_eq!(
            scan_args(&["launcher.exe", link.as_str()]),
            LinkArg::Invite(link.clone())
        );
        assert_eq!(
            scan_args(&["launcher.exe", "--flag", &public()]),
            LinkArg::Invite(public())
        );
        let upper = private("DAINTY");
        assert_eq!(invite_in_args(&["x", upper.as_str()]), Some(upper.clone()));
        let padded = format!("{link} \r\n");
        assert_eq!(invite_in_args(&[padded.as_str()]), Some(link.clone()));
        // Only the first link argument counts, even when a later one parses.
        let broken = "dauntless://join?v=9";
        assert_eq!(
            scan_args(&[broken, link.as_str()]),
            LinkArg::Refused(InviteError::Version)
        );
    }

    #[test]
    fn arguments_without_a_link() {
        assert_eq!(scan_args::<&str>(&[]), LinkArg::None);
        assert_eq!(scan_args(&["launcher.exe", "--user-data-dir", "C:/p"]), LinkArg::None);
        // Leading text before the scheme, or no colon, is not a link argument.
        assert_eq!(
            scan_args(&[format!(" {}", private("dauntless")).as_str()]),
            LinkArg::None
        );
        assert_eq!(
            scan_args(&["dauntless", "dainty", "dauntlessX://join?v=1"]),
            LinkArg::None
        );
        assert_eq!(
            scan_args(&["https://example.com/join?v=1", "file:///c:/x"]),
            LinkArg::None
        );
        assert_eq!(scan_args(&["dauntless:"]), LinkArg::Refused(InviteError::NotInvite));
        assert_eq!(invite_in_args(&["dauntless:"]), None);
    }

    #[test]
    fn refused_schemes() {
        let revived = private("dauntless-revived");
        assert_eq!(
            scan_args(&["x", revived.as_str()]),
            LinkArg::Refused(InviteError::Revived)
        );
        assert_eq!(
            scan_args(&["DAUNTLESS-REVIVED:whatever"]),
            LinkArg::Refused(InviteError::Revived)
        );
        assert_eq!(invite_in_args(&[revived.as_str()]), None);
        // A refused scheme does not hide a real link after it.
        let link = private("dauntless");
        assert_eq!(scan_args(&[revived.as_str(), link.as_str()]), LinkArg::Invite(link));
        for other in [
            "undaunted://join?v=1",
            "reforged://join?v=1",
            "dauntless-reforged://join?v=1",
        ] {
            assert_eq!(scan_args(&[other]), LinkArg::None, "{other}");
        }
    }

    #[test]
    fn refused_invites_are_typed() {
        let long = format!("{}&x={}", private("dauntless"), "a".repeat(3000));
        assert_eq!(scan_args(&[long.as_str()]), LinkArg::Refused(InviteError::TooLong));
        assert_eq!(
            scan_args(&["dauntless://other?v=1"]),
            LinkArg::Refused(InviteError::NotInvite)
        );
        let extra = format!("{}&evil=1", private("dainty"));
        assert_eq!(scan_args(&[extra.as_str()]), LinkArg::Refused(InviteError::Params));
    }

    #[test]
    fn non_utf8_arguments_are_skipped() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            let bad = OsStr::from_bytes(b"dauntless://\xff");
            let link = private("dauntless");
            assert_eq!(scan_args(&[bad, OsStr::new(&link)]), LinkArg::Invite(link));
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt as _;
            let bad = std::ffi::OsString::from_wide(&[0xD800, b'd'.into()]);
            let link = private("dauntless");
            assert_eq!(scan_args(&[bad.as_os_str(), OsStr::new(&link)]), LinkArg::Invite(link));
        }
    }

    #[test]
    fn labels() {
        assert_eq!(scheme_label("dainty"), "URL:Dainty Lass invite");
        assert_eq!(scheme_label("dauntless"), "URL:Dauntless invite");
    }
}
