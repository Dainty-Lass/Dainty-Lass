// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The login queue: with a cap on players online (settings.toml's `backend.max_online_players`), PLAY waits here
//! first come, first served. A waiting launcher polls; one that stops polling loses its place. An admitted one
//! holds a seat until its game shows up in chat's presence or the hold runs out. No cap, no queue.

use std::collections::{HashMap, HashSet, VecDeque};

use axum::response::Response;
use http::{HeaderMap, StatusCode};
use rustic_isles::{Call, Shared};
use rustic_isles_store::IslesStore;
use serde_json::Value;

use crate::launcher_api::key_holder;
use crate::state::LauncherApiState;
use crate::util::{self, json, now_ms, object, store_of};

/// The backend env variable the host kit writes the cap to.
pub const MAX_ONLINE_ENV: &str = "DAINTY_MAX_ONLINE_PLAYERS";
/// A waiting launcher not heard from for this long has left; the launcher polls every 5 s.
pub const STALE_MS: i64 = 30_000;
/// How long an admitted player's seat waits for their game to come online.
pub const HOLD_MS: i64 = 180_000;

/// Where a player stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Go ahead and play.
    Admitted,
    /// Waiting, 1 being next.
    Waiting(usize),
}

/// The waiting line and the seats held for admitted players.
#[derive(Debug, Default)]
pub struct LoginQueue {
    /// User ids in arrival order, with when each last polled.
    waiting: VecDeque<(String, i64)>,
    /// Admitted user ids not yet online, with when each was admitted.
    held: HashMap<String, i64>,
}

impl LoginQueue {
    /// Joins or polls for `user`. `online` is chat's presence, which counts toward `capacity` with the held seats.
    pub fn poll(&mut self, user: &str, capacity: Option<usize>, online: &HashSet<String>, now: i64) -> Place {
        let Some(capacity) = capacity else {
            self.waiting.clear();
            self.held.clear();
            return Place::Admitted;
        };
        self.expire(online, now);
        if online.contains(user) {
            self.leave(user);
            return Place::Admitted;
        }
        // The hold keeps its first time, so polling again does not stretch it
        if self.held.contains_key(user) {
            return Place::Admitted;
        }
        let index = match self.waiting.iter().position(|(id, _)| id == user) {
            Some(index) => {
                self.waiting[index].1 = now;
                index
            }
            None => {
                self.waiting.push_back((user.to_owned(), now));
                self.waiting.len() - 1
            }
        };
        let free = capacity.saturating_sub(online.len() + self.held.len());
        if index < free {
            self.waiting.remove(index);
            self.held.insert(user.to_owned(), now);
            Place::Admitted
        } else {
            Place::Waiting(index - free + 1)
        }
    }

    /// Gives up `user`'s place or held seat.
    pub fn leave(&mut self, user: &str) {
        self.waiting.retain(|(id, _)| id != user);
        self.held.remove(user);
    }

    /// Players waiting.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Seats held for admitted players.
    pub fn held(&self) -> usize {
        self.held.len()
    }

    fn expire(&mut self, online: &HashSet<String>, now: i64) {
        self.waiting
            .retain(|(id, polled)| !online.contains(id) && now - polled < STALE_MS && now >= *polled);
        self.held
            .retain(|id, admitted| !online.contains(id) && now - *admitted < HOLD_MS && now >= *admitted);
    }
}

/// The host's cap: a whole number above 0, else none.
pub fn capacity(settings: &rustic_isles::Settings) -> Option<usize> {
    settings
        .raw(MAX_ONLINE_ENV)
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|&cap| cap > 0)
}

/// The eligible key holder's userId, or the refusal.
async fn caller(shared: &Shared, store: &IslesStore, headers: &HeaderMap) -> Result<String, Response> {
    let Some((holder, _)) = key_holder(shared, store, headers).await else {
        let body = object(vec![("error", Value::from("unauthorized"))]);
        return Err(json(StatusCode::UNAUTHORIZED, &body));
    };
    if !holder.eligible() {
        let body = object(vec![("error", Value::from("account_disabled"))]);
        return Err(json(StatusCode::FORBIDDEN, &body));
    }
    Ok(holder.user_id)
}

/// `POST /dainty/api/LoginQueue`: joins the line or polls it; `{"admitted":true}` or the place in it.
pub async fn join(call: Call) -> Response {
    let Some(store) = store_of(&call.shared) else {
        return util::no_store(call.route, &call.request_id);
    };
    let user = match caller(&call.shared, &store, call.request.headers()).await {
        Ok(user) => user,
        Err(refused) => return refused,
    };
    let cap = capacity(call.shared.settings());
    let online: HashSet<String> = call.shared.presence().online_user_ids().into_iter().collect();
    let place = LauncherApiState::of(&call.shared)
        .login_queue()
        .poll(&user, cap, &online, now_ms());
    let body = match place {
        Place::Admitted => object(vec![("admitted", Value::Bool(true))]),
        Place::Waiting(position) => object(vec![
            ("admitted", Value::Bool(false)),
            ("position", Value::from(position)),
        ]),
    };
    let mut response = util::ok(&body);
    util::set_header(&mut response, "cache-control", "no-store");
    response
}

/// `DELETE /dainty/api/LoginQueue`: leaves the line, or gives back a held seat.
pub async fn leave(call: Call) -> Response {
    let Some(store) = store_of(&call.shared) else {
        return util::no_store(call.route, &call.request_id);
    };
    match caller(&call.shared, &store, call.request.headers()).await {
        Ok(user) => {
            LauncherApiState::of(&call.shared).login_queue().leave(&user);
            util::empty(StatusCode::NO_CONTENT)
        }
        Err(refused) => refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn online(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn no_cap_is_no_queue() {
        let mut q = LoginQueue::default();
        let full = online(&["a", "b", "c"]);
        assert_eq!(q.poll("d", None, &full, 0), Place::Admitted);
        assert_eq!((q.waiting(), q.held()), (0, 0));
    }

    #[test]
    fn first_come_first_served() {
        let mut q = LoginQueue::default();
        let full = online(&["a", "b"]);
        assert_eq!(q.poll("c", Some(2), &full, 0), Place::Waiting(1));
        assert_eq!(q.poll("d", Some(2), &full, 1), Place::Waiting(2));
        assert_eq!(q.poll("e", Some(2), &full, 2), Place::Waiting(3));
        // A seat opens: the later "d" polls first but "c" is still ahead
        let one_left = online(&["a"]);
        assert_eq!(q.poll("d", Some(2), &one_left, 3), Place::Waiting(1));
        assert_eq!(q.poll("c", Some(2), &one_left, 4), Place::Admitted);
        assert_eq!(q.held(), 1);
        // "c"'s held seat fills the server until "c" is online
        assert_eq!(q.poll("d", Some(2), &one_left, 5), Place::Waiting(1));
        assert_eq!(q.poll("e", Some(2), &one_left, 5), Place::Waiting(2));
        let c_in = online(&["a", "c"]);
        assert_eq!(q.poll("d", Some(2), &c_in, 6), Place::Waiting(1));
        assert_eq!(q.held(), 0);
    }

    #[test]
    fn room_admits_at_once_and_online_players_skip_the_line() {
        let mut q = LoginQueue::default();
        assert_eq!(q.poll("a", Some(2), &online(&[]), 0), Place::Admitted);
        assert_eq!(q.poll("a", Some(2), &online(&[]), 1), Place::Admitted);
        assert_eq!(q.poll("b", Some(2), &online(&[]), 1), Place::Admitted);
        assert_eq!(q.poll("c", Some(2), &online(&[]), 2), Place::Waiting(1));
        let full = online(&["x", "y", "z"]);
        assert_eq!(q.poll("x", Some(2), &full, 3), Place::Admitted);
    }

    #[test]
    fn leaving_moves_the_line_up() {
        let mut q = LoginQueue::default();
        let full = online(&["a"]);
        q.poll("b", Some(1), &full, 0);
        q.poll("c", Some(1), &full, 0);
        q.leave("b");
        assert_eq!(q.poll("c", Some(1), &full, 1), Place::Waiting(1));
        q.leave("c");
        assert_eq!(q.waiting(), 0);
        // Giving back a held seat lets the next one in
        assert_eq!(q.poll("d", Some(1), &online(&[]), 2), Place::Admitted);
        assert_eq!(q.poll("e", Some(1), &online(&[]), 2), Place::Waiting(1));
        q.leave("d");
        assert_eq!(q.poll("e", Some(1), &online(&[]), 3), Place::Admitted);
    }

    #[test]
    fn stale_polls_and_old_holds_expire() {
        let mut q = LoginQueue::default();
        let full = online(&["a"]);
        q.poll("b", Some(1), &full, 0);
        q.poll("c", Some(1), &full, 20_000);
        // "b" went quiet: "c" moves up and "b" rejoins at the back
        assert_eq!(q.poll("c", Some(1), &full, STALE_MS), Place::Waiting(1));
        assert_eq!(q.poll("b", Some(1), &full, STALE_MS + 1), Place::Waiting(2));
        // A held seat whose game never came online is freed
        let mut q = LoginQueue::default();
        assert_eq!(q.poll("d", Some(1), &online(&[]), 0), Place::Admitted);
        assert_eq!(q.poll("e", Some(1), &online(&[]), HOLD_MS - 1), Place::Waiting(1));
        assert_eq!(q.poll("e", Some(1), &online(&[]), HOLD_MS), Place::Admitted);
        assert_eq!(q.poll("d", Some(1), &online(&[]), HOLD_MS + 1), Place::Waiting(1));
    }

    #[test]
    fn the_cap_from_settings() {
        let read = |value: Option<&str>| {
            let pairs: Vec<(&str, &str)> = value.map(|v| (MAX_ONLINE_ENV, v)).into_iter().collect();
            capacity(&rustic_isles::Settings::from_pairs(pairs))
        };
        assert_eq!(read(None), None);
        assert_eq!(read(Some("0")), None);
        assert_eq!(read(Some("abc")), None);
        assert_eq!(read(Some(" 40 ")), Some(40));
    }
}
