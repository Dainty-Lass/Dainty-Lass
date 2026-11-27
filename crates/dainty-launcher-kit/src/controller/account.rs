// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The account on the current server (RD27 spec §2.4.6). The key goes only to its own server and slot, and the
//! key store registers it with the log's redaction before anything could print it.

use dainty_protocol::account::{RegisterRequest, UsernameCheck, check_username, extract_account_key};
use dainty_protocol::status::limited_view;
use serde_json::{Value, json};

use tokio_util::sync::CancellationToken;

use super::Controller;
use super::platform::TEXT_FILTER;
use super::stored::StoredServer;
use super::stored::edit;
use crate::bridge::types::{ActionResult, ErrorCode, LauncherError, NoticeCode, RegisterOutcome};
use crate::net::hostapi::{self, OAuthFinishRequest};
use crate::net::oauth_redirect::{self, RedirectError};
use crate::platform::keystore::{Backup, KeyStoreError, backup_is_for, backup_text, write_private};
use crate::platform::links;
use crate::platform::log;

/// The biggest key file `import_key_file` reads.
pub const MAX_KEY_FILE_BYTES: u64 = 8192;
/// How long `sign_in` waits for the browser to come back; the server forgets the sign-in after five minutes.
const SIGN_IN_WAIT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

fn storage_error(error: &KeyStoreError) -> LauncherError {
    LauncherError::new(match error {
        KeyStoreError::InvalidKey => ErrorCode::KeyInvalid,
        _ => ErrorCode::KeyStorageUnavailable,
    })
}

impl Controller {
    /// `register`: a new account with the stored invite code.
    pub async fn register(&self, username: &str) -> RegisterOutcome {
        let check = check_username(username);
        if check != UsernameCheck::Ok {
            return RegisterOutcome::Err(LauncherError::with_detail(ErrorCode::UsernameInvalid, check.as_str()));
        }
        let Some(server) = self.server() else {
            return RegisterOutcome::Err(LauncherError::new(ErrorCode::ServerUnreachable));
        };
        if let Err(error) = self.begin_busy() {
            return RegisterOutcome::Err(error);
        }
        let request = RegisterRequest {
            username: username.to_owned(),
            invite_code: server.code.clone().unwrap_or_default(),
        };
        let key = match hostapi::register(&server.endpoint(), &request).await {
            Ok(key) => key,
            Err(error) => {
                self.finish(Err(error.clone()));
                return RegisterOutcome::Err(error);
            }
        };
        if let Err(error) = self.keep_new_key(&server, key, username).await {
            self.finish(Err(error.clone()));
            return RegisterOutcome::Err(error);
        }
        log::info(format!("registered {username} on {}:{}", server.host, server.port));
        self.confirm_account().await;
        self.poll_once().await;
        self.finish(Ok(()));
        RegisterOutcome::Ok {
            username: username.to_owned(),
        }
    }

    /// Saves a key the server just made for `username`, in this server's slot.
    async fn keep_new_key(&self, server: &StoredServer, key: String, username: &str) -> Result<(), LauncherError> {
        let (slot, pin) = (server.slot(), server.pin().map(str::to_owned));
        self.with_keys(move |keys| keys.save(&slot, &key, pin.as_deref()))
            .await
            .map_err(|_| LauncherError::new(ErrorCode::KeyStorageUnavailable))?;
        let id = server.id();
        let name = username.to_owned();
        self.edit_settings(|map| {
            edit::server_field(map, "code", Value::Null);
            edit::keyed_value(map, "usernames", &id, Some(json!(name)));
            edit::keyed_value(map, "backupOffered", &id, None);
        })
        .await;
        self.key_present();
        Ok(())
    }

    /// `sign_in`: the server's OAuth sign-in (`docs/oauth.md`). The provider's page opens in the browser, which comes
    /// back to this launcher on loopback; the server trades the code and answers the account key, of the account
    /// linked to that sign-in or a new one named `username` (empty: the provider's name). The player cancels with
    /// `cancel_task`.
    pub async fn sign_in(&self, username: &str) -> RegisterOutcome {
        let username = username.trim();
        if !username.is_empty() {
            let check = check_username(username);
            if check != UsernameCheck::Ok {
                return RegisterOutcome::Err(LauncherError::with_detail(ErrorCode::UsernameInvalid, check.as_str()));
            }
        }
        let Some(server) = self.server() else {
            return RegisterOutcome::Err(LauncherError::new(ErrorCode::ServerUnreachable));
        };
        if let Err(error) = self.begin_busy() {
            return RegisterOutcome::Err(error);
        }
        let cancel = CancellationToken::new();
        self.lock().sign_in_cancel = Some(cancel.clone());
        let result = self.sign_in_steps(&server, username, &cancel).await;
        self.lock().sign_in_cancel = None;
        let name = match result {
            Ok(name) => name,
            Err(error) => {
                self.finish(Err(error.clone()));
                return RegisterOutcome::Err(error);
            }
        };
        log::info(format!("signed in as {name} on {}:{}", server.host, server.port));
        self.confirm_account().await;
        self.poll_once().await;
        self.finish(Ok(()));
        RegisterOutcome::Ok { username: name }
    }

    /// `link_sign_in`: links the server's OAuth sign-in to the account the stored key holds, so signing in later
    /// gets it back on another PC. The key stays as it is.
    pub async fn link_sign_in(&self) -> ActionResult {
        let Some(server) = self.server() else {
            return ActionResult::err(ErrorCode::ServerUnreachable);
        };
        let Some(key) = self.load_key().await else {
            return ActionResult::err(ErrorCode::KeyInvalid);
        };
        if let Err(error) = self.begin_busy() {
            return ActionResult::Err(error);
        }
        let cancel = CancellationToken::new();
        self.lock().sign_in_cancel = Some(cancel.clone());
        let result = match self.browser_round(&server, &cancel).await {
            Ok(request) => hostapi::oauth_link(&server.endpoint(), &key, &request).await,
            Err(error) => Err(error),
        };
        self.lock().sign_in_cancel = None;
        if result.is_ok() {
            log::info(format!("linked the sign-in on {}:{}", server.host, server.port));
            let mut state = self.lock();
            state.oauth_linked = Some(true);
            state.notice = Some(NoticeCode::OauthLinked);
        }
        self.finish(result)
    }

    async fn sign_in_steps(
        &self,
        server: &StoredServer,
        username: &str,
        cancel: &CancellationToken,
    ) -> Result<String, LauncherError> {
        let mut request = self.browser_round(server, cancel).await?;
        request.username = (!username.is_empty()).then(|| username.to_owned());
        request.invite_code = server.code.clone().unwrap_or_default();
        let (key, name) = hostapi::oauth_finish(&server.endpoint(), &request).await?;
        self.keep_new_key(server, key, &name).await?;
        Ok(name)
    }

    /// Starts a sign-in, opens the provider's page and waits for the browser: the request to finish it with.
    async fn browser_round(
        &self,
        server: &StoredServer,
        cancel: &CancellationToken,
    ) -> Result<OAuthFinishRequest, LauncherError> {
        let endpoint = server.endpoint();
        let start = hostapi::oauth_start(&endpoint).await?;
        let Some(url) = links::check_oauth(&start.authorize_url, &start.provider) else {
            log::warn(format!(
                "{}:{} sent a sign-in address it may not",
                server.host, server.port
            ));
            return Err(LauncherError::new(ErrorCode::OauthBadUrl));
        };
        let listener = oauth_redirect::bind()
            .await
            .map_err(|_| LauncherError::new(ErrorCode::OauthPortBusy))?;
        self.platform()
            .open_url(&url)
            .map_err(|_| LauncherError::new(ErrorCode::OauthFailed))?;
        let code = oauth_redirect::wait(listener, &start.state, self.language(), cancel, SIGN_IN_WAIT)
            .await
            .map_err(|error| {
                LauncherError::new(match error {
                    RedirectError::PortBusy => ErrorCode::OauthPortBusy,
                    RedirectError::Denied => ErrorCode::OauthDenied,
                    RedirectError::TimedOut => ErrorCode::OauthExpired,
                    RedirectError::Cancelled => ErrorCode::Cancelled,
                })
            })?;
        Ok(OAuthFinishRequest {
            state: start.state,
            code,
            ticket: start.ticket,
            username: None,
            invite_code: String::new(),
        })
    }

    fn key_present(&self) {
        let mut state = self.lock();
        state.has_key = true;
        state.key_refused = false;
        state.generation += 1;
    }

    /// `use_existing_key`: a key pasted, or found anywhere in pasted text.
    pub async fn use_existing_key(&self, text: &str) -> ActionResult {
        if let Err(error) = self.begin_busy() {
            return ActionResult::Err(error);
        }
        let result = match extract_account_key(text) {
            Some(key) => self.adopt_key(key.to_owned(), text).await,
            None => Err(LauncherError::new(ErrorCode::KeyInvalid)),
        };
        self.finish(result)
    }

    /// `import_key_file`: a key from a backup file.
    pub async fn import_key_file(&self) -> ActionResult {
        if self.lock().busy {
            return ActionResult::err(ErrorCode::Busy);
        }
        let language = self.language();
        let Some(file) = self.platform().choose_open_file(language, TEXT_FILTER).await else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        let small = std::fs::metadata(&file).is_ok_and(|meta| meta.len() <= MAX_KEY_FILE_BYTES);
        let text = if small {
            std::fs::read(&file)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .ok()
        } else {
            None
        };
        let Some(text) = text else {
            return ActionResult::err(ErrorCode::KeyInvalid);
        };
        let Some(key) = extract_account_key(&text).map(str::to_owned) else {
            return ActionResult::err(ErrorCode::KeyInvalid);
        };
        if let Err(error) = self.begin_busy() {
            return ActionResult::Err(error);
        }
        let result = self.adopt_key(key, &text).await;
        self.finish(result)
    }

    /// Takes a key the player already had: the server must know it before it is saved. `text` is what it came in,
    /// so a backup made for another server never reaches this one.
    async fn adopt_key(&self, key: String, text: &str) -> Result<(), LauncherError> {
        let Some(server) = self.server() else {
            return Err(LauncherError::new(ErrorCode::ServerUnreachable));
        };
        if !backup_is_for(text, &server.host, server.port, server.pin()) {
            log::info(format!(
                "refused a key backup made for another server than {}:{}",
                server.host, server.port
            ));
            return Err(LauncherError::new(ErrorCode::KeyInvalid));
        }
        let info = hostapi::fetch_user_info(&server.endpoint(), &key).await?;
        let (slot, pin) = (server.slot(), server.pin().map(str::to_owned));
        self.with_keys(move |keys| keys.save(&slot, &key, pin.as_deref()))
            .await
            .map_err(|error| storage_error(&error))?;
        let id = server.id();
        self.edit_settings(|map| {
            edit::keyed_value(map, "usernames", &id, Some(json!(info.username)));
            edit::keyed_value(map, "backupOffered", &id, Some(json!(true)));
            edit::server_field(map, "code", Value::Null);
        })
        .await;
        self.key_present();
        log::info(format!("using an existing key on {}:{}", server.host, server.port));
        self.poll_once().await;
        Ok(())
    }

    /// `save_key_backup`: writes the backup file where the player says.
    pub async fn save_key_backup(&self) -> ActionResult {
        let Some(server) = self.server() else {
            return ActionResult::err(ErrorCode::ServerUnreachable);
        };
        let Some(key) = self.load_key().await else {
            return ActionResult::err(ErrorCode::KeyInvalid);
        };
        let id = server.id();
        let username = self.stored().username(&id).map(str::to_owned);
        let default_name = match &username {
            Some(name) => format!("Dainty Lass key - {name}.txt"),
            None => "Dainty Lass key.txt".to_owned(),
        };
        let language = self.language();
        let Some(file) = self
            .platform()
            .choose_save_file(language, default_name, TEXT_FILTER)
            .await
        else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        let text = backup_text(&Backup {
            server_name: &server.name,
            host: &server.host,
            port: server.port,
            fingerprint: server.pin(),
            username: username.as_deref(),
            saved: chrono::Local::now().date_naive(),
            key: &key,
        });
        if let Err(error) = write_private(&file, text.as_bytes()) {
            log::warn(format!("could not write the key backup: {error}"));
            return ActionResult::err(ErrorCode::Unknown);
        }
        self.edit_settings(|map| edit::keyed_value(map, "backupOffered", &id, Some(json!(true))))
            .await;
        self.set_notice(NoticeCode::BackupSaved);
        ActionResult::Ok
    }

    /// `dismiss_backup_offer`.
    pub async fn dismiss_backup_offer(&self) -> ActionResult {
        if let Some(server) = self.server() {
            let id = server.id();
            self.edit_settings(|map| edit::keyed_value(map, "backupOffered", &id, Some(json!(true))))
                .await;
        }
        self.publish();
        ActionResult::Ok
    }

    /// `logout`: forgets the key on this PC, every copy.
    pub async fn logout(&self) -> ActionResult {
        if self.occupied() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let Some(server) = self.server() else {
            return ActionResult::Ok;
        };
        self.realms_shutdown().await;
        let slot = server.slot();
        if let Err(error) = self.with_keys(move |keys| keys.remove(&slot)).await {
            log::warn(format!("could not remove every copy of the key: {error}"));
        }
        let id = server.id();
        self.edit_settings(|map| {
            edit::keyed_value(map, "usernames", &id, None);
            edit::keyed_value(map, "backupOffered", &id, None);
        })
        .await;
        {
            let mut state = self.lock();
            state.has_key = false;
            state.key_refused = false;
            state.generation += 1;
            state.status = state.status.take().map(limited_view);
        }
        log::info(format!("signed out of {}:{}", server.host, server.port));
        self.publish();
        ActionResult::Ok
    }
}
