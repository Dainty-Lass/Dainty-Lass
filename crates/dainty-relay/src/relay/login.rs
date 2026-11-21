// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The game's login tickets across one session. A ticket logs in once, but the game keeps the one it was started
//! with and sends it again when it signs in from its menu. Each login's answer carries the next ticket
//! ([`REFRESH_HEADER`]); the relay keeps it from the game and puts it in place of the spent one on the next login.

use std::collections::VecDeque;

/// The login route's path, after any `/__origin/<host>` the runtime's redirect put before it.
pub const LOGIN_PATH: &str = "/account/api/oauth/token";
/// The answer's next ticket; never passed to the game.
pub const REFRESH_HEADER: &str = "x-dainty-refresh-ticket";
/// A login body longer than this is not read for a ticket; the game's is a few hundred bytes.
pub const MAX_LOGIN_BODY: usize = 16 * 1024;

const PREFIX: &[u8] = b"DLT_";
const TICKET_LEN: usize = 4 + 48;
/// Spent tickets remembered: the game's own and the few before a relay restart would matter.
const MAX_SPENT: usize = 8;

/// Whether `target` (path and query) is the game's login.
#[must_use]
pub fn is_login(target: &str) -> bool {
    target.split('?').next().is_some_and(|path| path.ends_with(LOGIN_PATH))
}

/// Whether `text` is a ticket: `DLT_` and 48 lowercase hex digits.
#[must_use]
pub fn is_ticket(text: &[u8]) -> bool {
    text.len() == TICKET_LEN
        && text.starts_with(PREFIX)
        && text[PREFIX.len()..]
            .iter()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Where the first ticket in `body` starts.
fn find_ticket(body: &[u8]) -> Option<usize> {
    body.windows(TICKET_LEN).position(is_ticket)
}

/// One session's tickets.
#[derive(Debug, Default)]
pub struct LoginTickets {
    spent: VecDeque<Vec<u8>>,
    next: Option<Vec<u8>>,
}

impl LoginTickets {
    /// The login body to send: a spent ticket swapped for the next one when there is one. `true` when swapped.
    pub fn outgoing(&mut self, body: &mut [u8]) -> bool {
        let Some(at) = find_ticket(body) else {
            return false;
        };
        let sent = &mut body[at..at + TICKET_LEN];
        if self.spent.iter().any(|spent| spent == sent) {
            if let Some(next) = &self.next {
                sent.copy_from_slice(next);
                return true;
            }
            return false;
        }
        if self.spent.len() == MAX_SPENT {
            self.spent.pop_front();
        }
        self.spent.push_back(sent.to_vec());
        false
    }

    /// The login's answer: its next ticket kept; a refused swap forgets the one it tried, which the server no
    /// longer holds. A failure on the way keeps it for the game's retry.
    pub fn incoming(&mut self, status: u16, refresh: Option<&[u8]>, swapped: bool) {
        match refresh.filter(|value| is_ticket(value)) {
            Some(next) => self.next = Some(next.to_vec()),
            None if swapped && (400..500).contains(&status) => self.next = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(n: u8) -> Vec<u8> {
        format!("DLT_{}", format!("{n:02x}").repeat(24)).into_bytes()
    }

    fn body(ticket: &[u8]) -> Vec<u8> {
        [
            &b"grant_type=exchange_code&exchange_code="[..],
            ticket,
            b"&token_type=eg1",
        ]
        .concat()
    }

    #[test]
    fn logins_by_path() {
        assert!(is_login("/account/api/oauth/token"));
        assert!(is_login(
            "/__origin/account-public-service-prod.ol.epicgames.com/account/api/oauth/token?x=1"
        ));
        assert!(!is_login("/account/api/oauth/token/x"));
        assert!(!is_login("/account/api/oauth/verify"));
    }

    #[test]
    fn the_spent_ticket_is_swapped_for_each_next_one() {
        let mut tickets = LoginTickets::default();
        let (first, second, third) = (ticket(1), ticket(2), ticket(3));
        let mut sent = body(&first);
        assert!(!tickets.outgoing(&mut sent));
        assert_eq!(sent, body(&first));
        tickets.incoming(200, Some(&second), false);

        // Back at the menu the game sends its first ticket again
        let mut sent = body(&first);
        assert!(tickets.outgoing(&mut sent));
        assert_eq!(sent, body(&second));
        tickets.incoming(200, Some(&third), true);
        let mut sent = body(&first);
        assert!(tickets.outgoing(&mut sent));
        assert_eq!(sent, body(&third));
    }

    #[test]
    fn a_failed_swap_is_kept_for_a_retry_and_a_refused_one_forgotten() {
        let mut tickets = LoginTickets::default();
        let (first, second) = (ticket(1), ticket(2));
        tickets.outgoing(&mut body(&first));
        tickets.incoming(200, Some(&second), false);
        let mut sent = body(&first);
        assert!(tickets.outgoing(&mut sent));
        tickets.incoming(502, None, true);
        let mut sent = body(&first);
        assert!(tickets.outgoing(&mut sent));
        assert_eq!(sent, body(&second));
        tickets.incoming(400, None, true);
        let mut sent = body(&first);
        assert!(!tickets.outgoing(&mut sent));
        assert_eq!(sent, body(&first));
    }

    #[test]
    fn nothing_else_is_touched() {
        let mut tickets = LoginTickets::default();
        tickets.outgoing(&mut body(&ticket(1)));
        tickets.incoming(200, Some(b"DLT_short"), false);
        let mut sent = body(&ticket(1));
        assert!(!tickets.outgoing(&mut sent), "a malformed next ticket is never kept");
        let mut key = b"exchange_code=DLK_0123456789abcdef0123456789abcdef0123456789abcdef".to_vec();
        let before = key.clone();
        assert!(!tickets.outgoing(&mut key));
        assert_eq!(key, before);
        let mut upper = body(&ticket(0xab).to_ascii_uppercase());
        assert!(!tickets.outgoing(&mut upper));
    }
}
