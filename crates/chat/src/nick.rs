// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Room nicknames: the client joins as `UrlEncode(DisplayName):AccountId:OwnResource`, and a nick is only
//! admitted when it names its own sender.

/// The display name the client falls back to when it has none
pub const FALLBACK_NAME: &str = "InvalidMCPUser";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NickReason {
    /// Names another account, or none
    Account,
    /// The right account with another resource
    Resource,
    /// The name part doesn't decode
    Format,
    /// The name isn't the player's
    Name,
}

impl NickReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Account => "nick-account",
            Self::Resource => "nick-resource",
            Self::Format => "nick-format",
            Self::Name => "nick-name",
        }
    }
}

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
}

/// The client's encoder: unreserved bytes kept, NUL dropped, everything else `%XX` in upper case.
pub fn url_encode_like_client(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b == 0 {
            continue;
        }
        if is_unreserved(b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn decode_name_part(part: &str) -> Option<String> {
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let high = char::from(hex[0]).to_digit(16)?;
            let low = char::from(hex[1]).to_digit(16)?;
            out.push(u8::try_from(high * 16 + low).ok()?);
            i += 3;
        } else if is_unreserved(b) {
            out.push(b);
            i += 1;
        } else {
            return None;
        }
    }
    String::from_utf8(out).ok()
}

/// The decoded display name before the first `:`.
pub fn name_part_of(nick: &str) -> Option<String> {
    let part = nick.split_once(':').map_or(nick, |(name, _)| name);
    decode_name_part(part)
}

/// The decoded display name when `nick` is this player's, or why not.
pub fn check_nickname(nick: &str, uid: &str, resource: &str, names: &[Option<&str>]) -> Result<String, NickReason> {
    let Some((name_part, rest)) = nick.split_once(':') else {
        return Err(NickReason::Account);
    };
    if rest != format!("{uid}:{resource}") {
        let first = rest.split(':').next().unwrap_or_default();
        return Err(if first == uid {
            NickReason::Resource
        } else {
            NickReason::Account
        });
    }
    let others = if uid.is_empty() {
        nick.to_owned()
    } else {
        nick.replace(uid, "")
    };
    if others.contains("UID-") {
        return Err(NickReason::Account);
    }
    let name = decode_name_part(name_part).ok_or(NickReason::Format)?;
    if name == FALLBACK_NAME || names.iter().flatten().any(|known| *known == name) {
        Ok(name)
    } else {
        Err(NickReason::Name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nicknames_are_checked_like_node() {
        let resource = "V2:app:WIN::0123";
        let uid = "UID-a";
        let name = Some("Ada Lovelace");
        let good = format!("{}:{uid}:{resource}", url_encode_like_client("Ada Lovelace"));
        assert_eq!(good, format!("Ada%20Lovelace:{uid}:{resource}"));
        assert_eq!(check_nickname(&good, uid, resource, &[name]), Ok("Ada Lovelace".into()));
        let fallback = format!("{FALLBACK_NAME}:{uid}:{resource}");
        assert_eq!(
            check_nickname(&fallback, uid, resource, &[None]),
            Ok(FALLBACK_NAME.into())
        );
        let bo = format!("Bo:{uid}:{resource}");
        assert_eq!(
            check_nickname(&bo, uid, resource, &[Some("Ada")]),
            Err(NickReason::Name)
        );
        assert_eq!(check_nickname("Ada", uid, resource, &[name]), Err(NickReason::Account));
        assert_eq!(
            check_nickname("Ada:UID-b:x", uid, resource, &[name]),
            Err(NickReason::Account)
        );
        assert_eq!(
            check_nickname("Ada:UID-a:other", uid, resource, &[name]),
            Err(NickReason::Resource)
        );
        for bad in ["A%2", "%FF"] {
            let nick = format!("{bad}:{uid}:{resource}");
            assert_eq!(check_nickname(&nick, uid, resource, &[name]), Err(NickReason::Format));
        }
        let other = format!("UID-b:{uid}:{resource}");
        assert_eq!(check_nickname(&other, uid, resource, &[name]), Err(NickReason::Account));
        assert_eq!(NickReason::Format.as_str(), "nick-format");
    }

    #[test]
    fn names_encode_and_decode_like_the_client() {
        assert_eq!(url_encode_like_client("é a"), "%C3%A9%20a");
        assert_eq!(url_encode_like_client("a\0b~"), "ab~");
        assert_eq!(decode_name_part("%c3%A9").as_deref(), Some("é"));
        assert_eq!(decode_name_part("%FF"), None);
        assert_eq!(decode_name_part("A%2"), None);
        assert_eq!(decode_name_part("a b"), None);
        assert_eq!(name_part_of("Ada%20L:UID-a:r").as_deref(), Some("Ada L"));
        assert_eq!(name_part_of("Ada").as_deref(), Some("Ada"));
    }
}
