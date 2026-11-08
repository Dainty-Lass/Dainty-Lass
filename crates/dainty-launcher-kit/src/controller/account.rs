// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The account on the current server (RD27 spec §2.4.6). The key goes only to its own server and slot, and the
//! key store registers it with the log's redaction before anything could print it.

use dainty_protocol::account::{RegisterRequest, UsernameCheck, check_username, extract_account_key};
use dainty_protocol::status::limited_view;
use serde_json::{Value, json};

use super::Controller;
use super::platform::TEXT_FILTER;
use super::stored::edit;
use crate::bridge::types::{ActionResult, ErrorCode, LauncherError, NoticeCode, RegisterOutcome};
use crate::net::hostapi;
use crate::platform::keystore::{Backup, KeyStoreError, backup_text, write_private};
use crate::platform::log;

/// The biggest key file `import_key_file` reads.
pub const MAX_KEY_FILE_BYTES: u64 = 8192;

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
        let (slot, pin, saved_key) = (server.slot(), server.pin().map(str::to_owned), key.clone());
        if self
            .with_keys(move |keys| keys.save(&slot, &saved_key, pin.as_deref()))
            .await
            .is_err()
        {
            let error = LauncherError::new(ErrorCode::KeyStorageUnavailable);
            self.finish(Err(error.clone()));
            return RegisterOutcome::Err(error);
        }
        let id = server.id();
        let name = username.to_owned();
        self.edit_settings(|map| {
            edit::server_field(map, "code", Value::Null);
            edit::keyed_value(map, "usernames", &id, Some(json!(name)));
            edit::keyed_value(map, "backupOffered", &id, None);
        })
        .await;
        self.key_present();
        log::info(format!("registered {username} on {}:{}", server.host, server.port));
        self.confirm_account().await;
        self.poll_once().await;
        self.finish(Ok(()));
        RegisterOutcome::Ok {
            username: username.to_owned(),
        }
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
            Some(key) => self.adopt_key(key.to_owned()).await,
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
        let Some(key) = text.as_deref().and_then(extract_account_key).map(str::to_owned) else {
            return ActionResult::err(ErrorCode::KeyInvalid);
        };
        if let Err(error) = self.begin_busy() {
            return ActionResult::Err(error);
        }
        let result = self.adopt_key(key).await;
        self.finish(result)
    }

    /// Takes a key the player already had: the server must know it before it is saved.
    async fn adopt_key(&self, key: String) -> Result<(), LauncherError> {
        let Some(server) = self.server() else {
            return Err(LauncherError::new(ErrorCode::ServerUnreachable));
        };
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
