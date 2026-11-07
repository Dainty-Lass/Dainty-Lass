// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Reading `netstat -ano -p tcp`: the pid that owns one loopback connection's client end.

use std::net::SocketAddrV4;

/// What the table says about one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// No row for it yet: the table may predate the connection.
    NotFound,
    /// Exactly one row, owned by this pid.
    Pid(u32),
    /// Several rows, or a pid that is not a number.
    Refused,
}

/// The owner of the row whose local end is `client` and remote end is `server`. Rows are five fields split on
/// white space: `TCP`, the two ends, a state label (localised, so never read) and the pid.
#[must_use]
pub fn owner(table: &str, client: SocketAddrV4, server: SocketAddrV4) -> Owner {
    let (client, server) = (client.to_string(), server.to_string());
    let mut found = Owner::NotFound;
    for line in table.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [proto, local, remote, _state, pid] = fields[..] else {
            continue;
        };
        if proto != "TCP" || local != client || remote != server {
            continue;
        }
        if found != Owner::NotFound {
            return Owner::Refused;
        }
        found = if pid.bytes().all(|b| b.is_ascii_digit()) {
            pid.parse().map_or(Owner::Refused, Owner::Pid)
        } else {
            Owner::Refused
        };
    }
    found
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    const CLIENT: &str = "127.0.0.1:52000";
    const SERVER: &str = "127.0.0.1:61100";

    fn at(table: &str) -> Owner {
        owner(table, CLIENT.parse().unwrap(), SERVER.parse().unwrap())
    }

    const TABLE: &str = "\r\nActive Connections\r\n\r\n  Proto  Local Address          Foreign Address        State           PID\r\n  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1100\r\n  TCP    127.0.0.1:61100        0.0.0.0:0              LISTENING       4000\r\n  TCP    127.0.0.1:52000        127.0.0.1:61100        ESTABLISHED     7777\r\n  TCP    127.0.0.1:61100        127.0.0.1:52000        ESTABLISHED     4000\r\n  TCP    [::1]:52000            [::1]:61100            ESTABLISHED     9999\r\n";

    #[test]
    fn finds_the_client_row_not_the_server_row() {
        assert_eq!(at(TABLE), Owner::Pid(7777));
        assert_eq!(
            owner(TABLE, SERVER.parse().unwrap(), CLIENT.parse().unwrap()),
            Owner::Pid(4000)
        );
    }

    #[test]
    fn a_localised_state_is_ignored() {
        assert_eq!(
            at("  TCP 127.0.0.1:52000 127.0.0.1:61100 HERGESTELLT 12\n"),
            Owner::Pid(12)
        );
        assert_eq!(
            at("TCP\t127.0.0.1:52000\t127.0.0.1:61100\tESTABELECIDA\t12"),
            Owner::Pid(12)
        );
    }

    #[test]
    fn missing_rows_are_not_yet() {
        assert_eq!(at(""), Owner::NotFound);
        assert_eq!(
            at("  TCP 127.0.0.1:52001 127.0.0.1:61100 ESTABLISHED 12\n"),
            Owner::NotFound
        );
        // Wrong protocol, field count or order
        assert_eq!(at("  UDP 127.0.0.1:52000 127.0.0.1:61100 X 12\n"), Owner::NotFound);
        assert_eq!(at("  tcp 127.0.0.1:52000 127.0.0.1:61100 X 12\n"), Owner::NotFound);
        assert_eq!(at("  TCP 127.0.0.1:52000 127.0.0.1:61100 12\n"), Owner::NotFound);
        assert_eq!(
            at("  TCP 127.0.0.1:52000 127.0.0.1:61100 TIME WAIT 12\n"),
            Owner::NotFound
        );
        assert_eq!(
            at("  TCP 127.0.0.1:61100 127.0.0.1:52000 ESTABLISHED 12\n"),
            Owner::NotFound
        );
    }

    #[test]
    fn duplicates_and_bad_pids_are_refused() {
        let twice = "TCP 127.0.0.1:52000 127.0.0.1:61100 A 12\nTCP 127.0.0.1:52000 127.0.0.1:61100 B 12\n";
        assert_eq!(at(twice), Owner::Refused);
        assert_eq!(at("TCP 127.0.0.1:52000 127.0.0.1:61100 A 1x\n"), Owner::Refused);
        assert_eq!(at("TCP 127.0.0.1:52000 127.0.0.1:61100 A -1\n"), Owner::Refused);
        assert_eq!(
            at("TCP 127.0.0.1:52000 127.0.0.1:61100 A 99999999999\n"),
            Owner::Refused
        );
    }
}
