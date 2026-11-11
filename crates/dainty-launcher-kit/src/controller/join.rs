// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Joining a server (RD27 spec §2.4.4): checking and using an invite, switching between and dropping saved servers.

use dainty_protocol::invite::{Invite, ServerMode, parse_invite};
use dainty_protocol::text::js_trim;
use serde_json::Value;

use super::stored::{StoredServer, edit};
use super::{Controller, Stored};
use crate::bridge::types::{ActionResult, ErrorCode, InviteCheck, LauncherError, ServerGame};
use crate::platform::keystore::{KeyStore, StoredCertificate};
use crate::platform::log;

fn invalid(detail: &str) -> LauncherError {
    LauncherError::with_detail(ErrorCode::InviteInvalidFormat, detail)
}

fn server_of(invite: Invite) -> StoredServer {
    StoredServer {
        game: ServerGame::Reforged,
        mode: invite.mode,
        host: invite.host,
        port: invite.port,
        name: invite.name,
        share: invite.share,
        fp: invite.fp,
        code: Some(invite.code),
    }
}

fn has_key(keys: &KeyStore, server: &StoredServer) -> bool {
    !matches!(
        keys.certificate(&server.slot()),
        Ok(StoredCertificate::Nothing) | Err(_)
    )
}

/// The fingerprint stored for a public server when it is not `server`'s: `Some(None)` for a bare key.
fn changed_certificate(keys: &KeyStore, server: &StoredServer) -> Option<Option<String>> {
    if server.mode != ServerMode::Public {
        return None;
    }
    match keys.certificate(&server.slot()) {
        Ok(StoredCertificate::Fingerprint(fp)) if Some(fp.as_str()) != server.fp.as_deref() => Some(Some(fp)),
        Ok(StoredCertificate::Bare) => Some(None),
        _ => None,
    }
}

/// A saved private server this one replaces at a new address: same game, port and trimmed name, a key there and
/// none here.
fn moved_from(keys: &KeyStore, stored: &Stored, server: &StoredServer) -> Option<StoredServer> {
    if server.mode != ServerMode::Private || has_key(keys, server) {
        return None;
    }
    stored.saved_servers().into_iter().find(|saved| {
        saved.game == server.game
            && saved.mode == ServerMode::Private
            && saved.port == server.port
            && saved.host != server.host
            && js_trim(&saved.name) == js_trim(&server.name)
            && has_key(keys, saved)
    })
}

impl Controller {
    /// `check_invite`: whether the invite can be used, and what it would change.
    #[must_use]
    pub fn check_invite(&self, text: &str) -> InviteCheck {
        let invite = match parse_invite(text) {
            Ok(invite) => invite,
            Err(error) => return InviteCheck::Err(invalid(error.as_str())),
        };
        let server = server_of(invite);
        let keys = &self.shared.keys;
        let changed = changed_certificate(keys, &server);
        let moved = moved_from(keys, &self.stored(), &server);
        InviteCheck::Ok {
            certificate_changed: changed.is_some(),
            previous_fingerprint: changed.flatten(),
            moved_from: moved.map(|saved| saved.host),
        }
    }

    fn join_refusal(&self) -> Option<LauncherError> {
        let state = self.lock();
        if state.busy || state.task.is_some() {
            Some(LauncherError::new(ErrorCode::Busy))
        } else if self.shared.game.is_running() {
            Some(LauncherError::new(ErrorCode::AlreadyRunning))
        } else {
            None
        }
    }

    /// `submit_invite`: joins the invite's server.
    pub async fn submit_invite(&self, text: &str, accept_new_certificate: bool, carry_account: bool) -> ActionResult {
        if let Some(refusal) = self.join_refusal() {
            return ActionResult::Err(refusal);
        }
        match parse_invite(text) {
            Ok(invite) => {
                self.join(server_of(invite), accept_new_certificate, carry_account)
                    .await
            }
            Err(error) => ActionResult::Err(invalid(error.as_str())),
        }
    }

    /// `switch_server`: joins a saved server again, keeping its stored code.
    pub async fn switch_server(&self, id: &str) -> ActionResult {
        if let Some(refusal) = self.join_refusal() {
            return ActionResult::Err(refusal);
        }
        let saved = self.stored().saved_servers().into_iter().find(|saved| saved.id() == id);
        match saved {
            Some(saved) if saved.game == ServerGame::Revived => ActionResult::Err(invalid("revived")),
            Some(saved) => self.join(saved, false, false).await,
            None => ActionResult::err(ErrorCode::InviteInvalidFormat),
        }
    }

    /// `remove_saved_server`: drops a saved server other than the current one.
    pub async fn remove_saved_server(&self, id: &str) -> ActionResult {
        if self.server().is_some_and(|current| current.id() == id) {
            return ActionResult::err(ErrorCode::Busy);
        }
        self.edit_settings(|map| edit::remove_saved(map, id)).await;
        self.publish();
        ActionResult::Ok
    }

    /// `forget_server`: leaves the current server.
    pub async fn forget_server(&self) -> ActionResult {
        if self.occupied() {
            return ActionResult::err(ErrorCode::Busy);
        }
        self.realms_shutdown().await;
        self.edit_settings(|map| edit::set_server(map, None)).await;
        {
            let mut state = self.lock();
            state.reset_server();
            state.has_key = false;
        }
        self.look_at_runtime().await;
        self.publish();
        ActionResult::Ok
    }

    async fn join(&self, server: StoredServer, accept: bool, carry: bool) -> ActionResult {
        let keys = std::sync::Arc::clone(&self.shared.keys);
        let stored = self.stored();
        let probe = server.clone();
        let checked = tokio::task::spawn_blocking(move || {
            let changed = changed_certificate(&keys, &probe);
            let moved = if carry {
                moved_from(&keys, &stored, &probe)
            } else {
                None
            };
            (changed, moved)
        })
        .await;
        let Ok((changed, moved)) = checked else {
            return ActionResult::err(ErrorCode::Unknown);
        };

        if let Some(old) = changed {
            if !accept {
                return ActionResult::Err(LauncherError::with_detail(
                    ErrorCode::CertChanged,
                    old.as_deref().unwrap_or(""),
                ));
            }
            let (slot, fp) = (server.slot(), server.fp.clone().unwrap_or_default());
            let new = fp.clone();
            match self.with_keys(move |keys| keys.rebind(&slot, &fp)).await {
                Ok(_) => log::info(format!(
                    "certificate of {}:{} changed from {} to {new}",
                    server.host,
                    server.port,
                    old.as_deref().unwrap_or("-")
                )),
                Err(error) => log::warn(format!("could not move the key to the new certificate: {error}")),
            }
        }

        if let Some(old) = moved {
            self.carry_account(&old, &server).await;
        }

        // The invite's code stays even beside a stored key: the server may have forgotten that key, and only the
        // code registers again. `confirm_account` drops it once the server takes the key.
        let previous = self.server();
        let differs = previous.is_none_or(|previous| !previous.same_place(&server) || previous.fp != server.fp);
        self.edit_settings(|map| edit::set_server(map, Some(&server))).await;
        if differs {
            self.realms_shutdown().await;
            self.lock().reset_server();
        }
        self.refresh_key_flag().await;
        self.lock().last_error = None;
        self.connect().await
    }

    /// Copies the account of a server that moved to `server`'s address.
    async fn carry_account(&self, old: &StoredServer, server: &StoredServer) {
        let (from, to) = (old.slot(), server.slot());
        let copied = self
            .with_keys(move |keys| match keys.load(&from, None)? {
                Some(key) => keys.save(&to, &key, None).map(|()| true),
                None => Ok(false),
            })
            .await;
        if !matches!(copied, Ok(true)) {
            log::warn(format!("could not carry the account from {}", old.host));
            return;
        }
        let (old_id, new_id) = (old.id(), server.id());
        let username = self.stored().username(&old_id).map(str::to_owned);
        let keep_name = self.stored().username(&new_id).is_none();
        self.edit_settings(|map| {
            if keep_name && let Some(name) = username {
                edit::keyed_value(map, "usernames", &new_id, Some(Value::String(name)));
            }
            edit::remove_saved(map, &old_id);
        })
        .await;
        log::info(format!(
            "carried the account from {}:{} to {}:{}",
            old.host, old.port, server.host, server.port
        ));
    }

    /// Whether a usable key is stored for the current server.
    pub(crate) async fn refresh_key_flag(&self) {
        let Some(server) = self.server() else {
            self.lock().has_key = false;
            return;
        };
        let (slot, pin) = (server.slot(), server.pin().map(str::to_owned));
        let found = self.with_keys(move |keys| keys.load(&slot, pin.as_deref())).await;
        self.lock().has_key = matches!(found, Ok(Some(_)));
    }

    /// The stored key of the current server, read fresh.
    pub(crate) async fn load_key(&self) -> Option<String> {
        let server = self.server()?;
        let (slot, pin) = (server.slot(), server.pin().map(str::to_owned));
        self.with_keys(move |keys| keys.load(&slot, pin.as_deref()))
            .await
            .ok()
            .flatten()
    }
}
