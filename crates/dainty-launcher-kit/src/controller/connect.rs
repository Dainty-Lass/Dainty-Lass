// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Reaching the server and keeping up with it (RD27 spec §2.4.5): the check, the status poll, the account
//! check, news and art.

use std::time::Instant;

use dainty_protocol::invite::{ServerMode, is_loopback_host};
use dainty_protocol::status::limited_view;
use serde_json::json;

use super::stored::edit;
use super::{ART_CACHE_DIR, Controller, NEWS_INTERVAL, UNREACHABLE_MAX_SKIP, background, news_items, now_iso};
use crate::bridge::types::{ActionResult, Branding, ConnectProblem, ErrorCode};
use crate::core::pins::sha256_hex;
use crate::net::hostapi::{self, StatusOutcome};
use crate::platform::log;

impl Controller {
    /// `connect` and `retry_connect`.
    pub async fn connect(&self) -> ActionResult {
        if self.server().is_none() {
            return ActionResult::err(ErrorCode::ServerUnreachable);
        }
        {
            let mut state = self.lock();
            if state.checking {
                return ActionResult::Ok;
            }
            state.checking = true;
        }
        self.publish();
        self.check().await;
        self.lock().checking = false;
        self.publish();
        ActionResult::Ok
    }

    /// The key the status request carries: only while one is stored and the server has not refused it.
    async fn status_key(&self) -> Option<String> {
        let usable = {
            let state = self.lock();
            state.has_key && !state.key_refused
        };
        if usable { self.load_key().await } else { None }
    }

    /// One full check of the current server.
    pub(crate) async fn check(&self) {
        let Some(server) = self.server() else { return };
        let private = server.mode == ServerMode::Private;
        let tailscale = private && self.platform().find_tailscale().is_some();
        let generation = {
            let mut state = self.lock();
            if private {
                state.tailscale_installed = tailscale;
            }
            state.generation
        };
        let key = self.status_key().await;
        let outcome = hostapi::fetch_status(&server.endpoint(), key.as_deref(), None).await;
        let mut rename = None;
        let (reached, first_failure) = {
            let mut state = self.lock();
            state.last_checked_at = Some(now_iso());
            let before = (state.reachable, state.problem);
            let reached = match outcome {
                StatusOutcome::Status(_) | StatusOutcome::Unsupported => {
                    state.reachable = Some(true);
                    state.problem = None;
                    state.failed_polls = 0;
                    state.failed_checks = 0;
                    state.skip_ticks = 0;
                    state.status_unsupported = matches!(outcome, StatusOutcome::Unsupported);
                    if let StatusOutcome::Status(status) = outcome {
                        if state.generation == generation {
                            if status.name != server.name {
                                rename = Some(status.name.clone());
                            }
                            state.status = Some(*status);
                        } else {
                            state.status = Some(limited_view(*status));
                        }
                    }
                    true
                }
                other => {
                    let problem = match other {
                        StatusOutcome::BadAnswer => ConnectProblem::BadAnswer,
                        StatusOutcome::CertMismatch { .. } => ConnectProblem::CertMismatch,
                        _ if private && !tailscale && !is_loopback_host(&server.host) => {
                            ConnectProblem::TailscaleMissing
                        }
                        _ => ConnectProblem::Unreachable,
                    };
                    state.reachable = Some(false);
                    state.problem = Some(problem);
                    state.failed_checks = state.failed_checks.saturating_add(1);
                    state.skip_ticks = unreachable_skip(state.failed_checks);
                    false
                }
            };
            (reached, before != (state.reachable, state.problem))
        };
        if !reached {
            let problem = self.lock().problem;
            if problem == Some(ConnectProblem::CertMismatch) {
                log::error(format!(
                    "{}:{} answered with another certificate",
                    server.host, server.port
                ));
            } else if first_failure {
                // Logged once per change: a saved invite whose host moved its port would warn every 15 s forever
                log::warn(format!(
                    "could not reach {}:{} ({problem:?}); checking again less and less often",
                    server.host, server.port
                ));
            }
            return;
        }
        if let Some(name) = rename {
            self.edit_settings(|map| edit::server_field(map, "name", json!(name)))
                .await;
        }
        self.confirm_account().await;
        self.refresh_offer().await;
        self.load_news_in_background(true);
    }

    /// Asks the server who the stored key belongs to (§2.4.5).
    pub(crate) async fn confirm_account(&self) {
        let Some(server) = self.server() else { return };
        let Some(key) = self.load_key().await else {
            self.lock().has_key = false;
            return;
        };
        match hostapi::fetch_user_info(&server.endpoint(), &key).await {
            Ok(info) => {
                self.lock().key_refused = false;
                let id = server.id();
                if self.stored().username(&id) != Some(info.username.as_str()) {
                    self.edit_settings(|map| edit::keyed_value(map, "usernames", &id, Some(json!(info.username))))
                        .await;
                }
            }
            Err(error) if matches!(error.code, ErrorCode::AccountBanned | ErrorCode::KeyRejected) => {
                if error.code == ErrorCode::KeyRejected {
                    log::warn(format!("{}:{} refused the stored key", server.host, server.port));
                }
                let mut state = self.lock();
                state.key_refused = true;
                state.last_error = Some(error);
            }
            Err(_) => {}
        }
    }

    /// `set_status_polling`: polls now and then every interval while on.
    pub fn set_status_polling(&self, on: bool) {
        let mut state = self.lock();
        if let Some(poll) = state.poll.take() {
            poll.abort();
        }
        if on && !state.shutting_down {
            let me = self.weak();
            let every = self.config().poll_interval;
            state.poll = Some(tokio::spawn(async move {
                let mut ticks = tokio::time::interval(every);
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut first = true;
                loop {
                    ticks.tick().await;
                    let Some(me) = Controller::from_weak(&me) else { return };
                    if !std::mem::take(&mut first) && me.skip_tick() {
                        continue;
                    }
                    me.poll_once().await;
                }
            }));
        }
    }

    /// Whether a scheduled poll passes over an unreachable server, so its checks back off.
    fn skip_tick(&self) -> bool {
        let mut state = self.lock();
        if state.reachable == Some(true) || state.skip_ticks == 0 {
            return false;
        }
        state.skip_ticks -= 1;
        true
    }

    /// `refresh_status`: one poll.
    pub async fn refresh_status(&self) {
        self.poll_once().await;
    }

    /// One status poll (§2.4.5).
    pub(crate) async fn poll_once(&self) {
        let Some(server) = self.server() else { return };
        let (reachable, problem, generation) = {
            let state = self.lock();
            if state.checking {
                return;
            }
            (state.reachable, state.problem, state.generation)
        };
        if reachable != Some(true) {
            if problem != Some(ConnectProblem::CertMismatch) {
                self.check().await;
                self.publish();
            }
            return;
        }
        let key = self.status_key().await;
        let outcome = hostapi::fetch_status(&server.endpoint(), key.as_deref(), None).await;
        let running = self.shared.game.is_running();
        let mut follow_up = false;
        {
            let mut state = self.lock();
            if state.generation != generation {
                return;
            }
            match outcome {
                StatusOutcome::Status(status) => {
                    state.status = Some(*status);
                    state.failed_polls = 0;
                    follow_up = true;
                }
                StatusOutcome::Unsupported => state.status_unsupported = true,
                StatusOutcome::CertMismatch { .. } => {
                    state.reachable = Some(false);
                    state.problem = Some(ConnectProblem::CertMismatch);
                    if let Some(status) = state.status.as_mut() {
                        status.online = false;
                    }
                    log::error(format!(
                        "{}:{} answered with another certificate",
                        server.host, server.port
                    ));
                }
                _ => {
                    state.failed_polls += 1;
                    if let Some(status) = state.status.as_mut() {
                        status.online = false;
                    }
                    if state.failed_polls >= 2 && !running {
                        if state.reachable == Some(true) {
                            log::warn(format!("lost {}:{}", server.host, server.port));
                        }
                        state.reachable = Some(false);
                        state.problem = Some(ConnectProblem::Unreachable);
                    }
                }
            }
        }
        if follow_up {
            if !running {
                self.confirm_account().await;
            }
            let needs_art = self.lock().branding.is_none();
            self.load_news_in_background(needs_art);
            let refresh = {
                let state = self.lock();
                !running
                    && state.task.is_none()
                    && (state.runtime.state == crate::bridge::types::RuntimeState::Unavailable
                        || state.manifest.is_none())
            };
            if refresh {
                self.refresh_offer().await;
            }
        }
        self.publish();
    }

    fn load_news_in_background(&self, with_art: bool) {
        let me = self.clone();
        self.spawn(async move {
            me.load_news().await;
            if with_art {
                me.load_art().await;
            }
        });
    }

    /// News, at most once a minute (§2.4.5).
    async fn load_news(&self) {
        let Some(server) = self.server() else { return };
        let (generation, fallback) = {
            let mut state = self.lock();
            if state.news_loading || state.news_at.is_some_and(|at| at.elapsed() < NEWS_INTERVAL) {
                return;
            }
            state.news_loading = true;
            let fallback: Vec<hostapi::NewsItem> = state
                .news
                .iter()
                .map(|item| hostapi::NewsItem {
                    date: item.date.clone(),
                    title: item.title.clone(),
                    body: item.body.clone(),
                })
                .collect();
            (state.generation, fallback)
        };
        let items = hostapi::fetch_news(&server.endpoint(), &fallback).await;
        let mut state = self.lock();
        state.news_loading = false;
        if state.generation == generation {
            state.news = news_items(items);
            state.news_at = Some(Instant::now());
        }
    }

    /// The server's backgrounds, cached by their SHA-256 and served as `rr-art://bg/<id>` (§2.4.5).
    async fn load_art(&self) {
        let Some(server) = self.server() else { return };
        let generation = self.lock().generation;
        let endpoint = server.endpoint();
        let index = hostapi::fetch_branding(&endpoint).await;
        let cache = self.user_data().join(ART_CACHE_DIR);
        if let Err(error) = std::fs::create_dir_all(&cache) {
            log::warn(format!("could not make the art cache: {error}"));
            return;
        }
        let mut backgrounds = Vec::new();
        for entry in index.backgrounds {
            let Some(image) = hostapi::fetch_branding_image(&endpoint, &entry.url).await else {
                continue;
            };
            let id = sha256_hex(&image.bytes);
            let file = cache.join(&id);
            if let Err(error) = std::fs::write(&file, &image.bytes) {
                log::warn(format!("could not write a background: {error}"));
                break;
            }
            self.platform().register_art(&id, &file, image.kind);
            backgrounds.push(background(format!("rr-art://bg/{id}"), entry.credit));
        }
        {
            let mut state = self.lock();
            if state.generation != generation {
                return;
            }
            state.branding = Some(Branding {
                backgrounds,
                accent: index.accent,
            });
        }
        self.publish();
    }

    pub(crate) fn unreachable_by_pin(&self) {
        let mut state = self.lock();
        state.reachable = Some(false);
        state.problem = Some(ConnectProblem::CertMismatch);
    }

    pub(crate) fn mark_refused(&self) {
        self.lock().key_refused = true;
    }
}

/// Poll ticks skipped after the `failures`-th failed check in a row: 0, 1, 3, 7, 15, then [`UNREACHABLE_MAX_SKIP`].
#[must_use]
pub fn unreachable_skip(failures: u32) -> u32 {
    let doubled = 1u32.checked_shl(failures.saturating_sub(1)).unwrap_or(u32::MAX);
    (doubled - 1).min(UNREACHABLE_MAX_SKIP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreachable_checks_back_off_to_five_minutes() {
        let skips: Vec<u32> = (1..=8).map(unreachable_skip).collect();
        assert_eq!(skips, [0, 1, 3, 7, 15, 19, 19, 19]);
        assert_eq!(unreachable_skip(0), 0);
        assert_eq!(unreachable_skip(u32::MAX), UNREACHABLE_MAX_SKIP);
    }
}
