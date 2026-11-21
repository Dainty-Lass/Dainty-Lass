// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Relay-only filtering of signal text, applied both ways: only relay candidates leave the transport or reach the
//! agent, so neither side's address is shown to the other.

pub const DESCRIPTION: &str = "DESCRIPTION";
pub const CANDIDATE: &str = "CANDIDATE";
pub const GATHERING_DONE: &str = "GATHERING_DONE";

fn is_candidate(line: &str) -> bool {
    line.starts_with("a=candidate:") || line.starts_with("candidate:")
}

fn is_relay(line: &str) -> bool {
    line.match_indices(" typ relay").any(|(at, m)| {
        let after = &line[at + m.len()..];
        after.is_empty() || after.starts_with(' ')
    })
}

fn kept_lines(data: &str) -> impl Iterator<Item = &str> {
    data.split('\n')
        .map(|line| line.trim_end_matches(['\r', ' ']).trim_start_matches(' '))
        .filter(|line| !line.is_empty())
}

/// The text a relay-only link sends or accepts for a signal of `kind`; empty means nothing is left.
pub fn relay_only(kind: &str, data: &str) -> String {
    match kind {
        CANDIDATE => {
            let kept: Vec<&str> = kept_lines(data).filter(|l| is_candidate(l) && is_relay(l)).collect();
            match kept.as_slice() {
                [one] => (*one).to_owned(),
                _ => String::new(),
            }
        }
        DESCRIPTION => {
            let kept: Vec<&str> = kept_lines(data)
                .filter(|l| {
                    if is_candidate(l) {
                        is_relay(l)
                    } else {
                        l.starts_with("a=ice-ufrag:")
                            || l.starts_with("a=ice-pwd:")
                            || l.starts_with("a=ice-options:")
                            || *l == "a=end-of-candidates"
                    }
                })
                .collect();
            if !kept.iter().any(|l| l.starts_with("a=ice-ufrag:")) {
                return String::new();
            }
            kept.iter().map(|l| format!("{l}\r\n")).collect()
        }
        GATHERING_DONE if data.is_empty() => "-".to_owned(),
        _ => data.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_keeps_relay() {
        let text = "a=ice-ufrag:abcd\r\na=ice-pwd:0123456789abcdef01234567\r\n\
                    a=candidate:1 1 UDP 2130706431 192.168.1.2 5000 typ host\r\n\
                    a=candidate:2 1 UDP 16777215 203.0.113.9 6000 typ relay raddr 0.0.0.0 rport 0\r\n\
                    a=extra:thing\r\n";
        let out = relay_only(DESCRIPTION, text);
        assert!(out.contains("a=ice-ufrag:abcd\r\n"));
        assert!(out.contains("typ relay raddr"));
        assert!(!out.contains("typ host"));
        assert!(!out.contains("a=extra"));
        assert_eq!(
            relay_only(
                CANDIDATE,
                "a=candidate:3 1 UDP 1694498815 203.0.113.1 7000 typ srflx raddr 10.0.0.1 rport 7000"
            ),
            ""
        );
        assert_eq!(
            relay_only(CANDIDATE, "candidate:3 1 UDP 1 203.0.113.1 7000 typ relay"),
            "candidate:3 1 UDP 1 203.0.113.1 7000 typ relay"
        );
        assert_eq!(
            relay_only(CANDIDATE, "candidate:3 1 UDP 1 203.0.113.1 7000 typ relayx"),
            ""
        );
        assert_eq!(relay_only(DESCRIPTION, "a=ice-pwd:x\r\n"), "");
        assert_eq!(relay_only(GATHERING_DONE, ""), "-");
    }
}
