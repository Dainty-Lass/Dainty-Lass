// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! A typed view of the settings map (RD27 spec §2.4.11). Reads come from a sanitised map, so a field is either
//! valid or absent; edits are made on the map, and the store sanitises and writes it.

use dainty_protocol::invite::ServerMode;
use serde_json::{Map, Value, json};

use crate::bridge::types::{GameLanguage, GraphicsPreset, RuntimeProfile, ServerGame};
use crate::core::i18n::Language;
use crate::net::http::Endpoint;
use crate::platform::keystore::{Slot, slot_id};

/// One server entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredServer {
    /// The game it runs.
    pub game: ServerGame,
    /// Private or public.
    pub mode: ServerMode,
    /// Its address.
    pub host: String,
    /// Its port.
    pub port: u16,
    /// Its name.
    pub name: String,
    /// Its Tailscale share link (private only).
    pub share: Option<String>,
    /// Its certificate's fingerprint (public only).
    pub fp: Option<String>,
    /// The invite code, kept until this PC has a key there.
    pub code: Option<String>,
}

impl StoredServer {
    /// Reads a sanitised entry.
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        let entry = value.as_object()?;
        let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            game: if text("game").as_deref() == Some("revived") {
                ServerGame::Revived
            } else {
                ServerGame::Reforged
            },
            mode: if text("mode").as_deref() == Some("public") {
                ServerMode::Public
            } else {
                ServerMode::Private
            },
            host: text("host")?,
            port: u16::try_from(entry.get("port")?.as_u64()?).ok()?,
            name: text("name")?,
            share: text("share"),
            fp: text("fp"),
            code: text("code"),
        })
    }

    /// The entry as the settings file writes it.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "game": match self.game { ServerGame::Reforged => "reforged", ServerGame::Revived => "revived" },
            "mode": mode_name(self.mode),
            "host": self.host,
            "port": self.port,
            "name": self.name,
            "share": self.share,
            "fp": self.fp,
            "code": self.code,
        })
    }

    /// The key store's id, which also keys `usernames`, `backupOffered` and `trustedRuntimes`.
    #[must_use]
    pub fn id(&self) -> String {
        slot_id(self.mode, &self.host, self.port)
    }

    /// Where requests go: the fingerprint pins it in public mode only.
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        Endpoint {
            host: self.host.clone(),
            port: self.port,
            pin: self.pin().map(str::to_owned),
        }
    }

    /// The fingerprint the key is bound to, in public mode.
    #[must_use]
    pub fn pin(&self) -> Option<&str> {
        match self.mode {
            ServerMode::Public => self.fp.as_deref(),
            ServerMode::Private => None,
        }
    }

    /// The key store slot.
    #[must_use]
    pub fn slot(&self) -> Slot {
        Slot::new(self.mode, &self.host, self.port)
    }

    /// Same mode, host and port: the same slot.
    #[must_use]
    pub fn same_place(&self, other: &Self) -> bool {
        self.mode == other.mode && self.host == other.host && self.port == other.port
    }
}

/// `private` or `public`.
#[must_use]
pub const fn mode_name(mode: ServerMode) -> &'static str {
    match mode {
        ServerMode::Private => "private",
        ServerMode::Public => "public",
    }
}

/// The settings map, read through.
#[derive(Debug, Clone, Default)]
pub struct Stored(pub Map<String, Value>);

impl Stored {
    fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    fn flag(&self, key: &str) -> bool {
        self.0.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    fn keyed(&self, map: &str, id: &str) -> Option<&Value> {
        self.0.get(map).and_then(Value::as_object).and_then(|m| m.get(id))
    }

    /// The current server.
    #[must_use]
    pub fn server(&self) -> Option<StoredServer> {
        self.0.get("server").and_then(StoredServer::from_value)
    }

    /// The saved servers, the current one included.
    #[must_use]
    pub fn saved_servers(&self) -> Vec<StoredServer> {
        self.0
            .get("savedServers")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(StoredServer::from_value).collect())
            .unwrap_or_default()
    }

    /// The chosen game folder.
    #[must_use]
    pub fn game_root(&self) -> Option<&str> {
        self.text("gameRoot")
    }

    /// The forced graphics preset.
    #[must_use]
    pub fn graphics(&self) -> GraphicsPreset {
        self.0
            .get("graphics")
            .and_then(Value::as_i64)
            .and_then(GraphicsPreset::new)
            .unwrap_or_default()
    }

    /// Windowed rather than full screen.
    #[must_use]
    pub fn windowed(&self) -> bool {
        self.flag("windowed")
    }

    /// Streamer mode.
    #[must_use]
    pub fn streamer_mode(&self) -> bool {
        self.flag("streamerMode")
    }

    /// The launcher's language.
    #[must_use]
    pub fn language(&self) -> Language {
        self.text("language")
            .and_then(Language::from_code)
            .unwrap_or(Language::En)
    }

    /// The game's language setting.
    #[must_use]
    pub fn game_language(&self) -> GameLanguage {
        self.0
            .get("gameLanguage")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// The runtime profile.
    #[must_use]
    pub fn runtime_profile(&self) -> RuntimeProfile {
        self.0
            .get("runtimeProfile")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// The Host page is shown, always in a debug build: only a developer runs one, from a checkout.
    #[must_use]
    pub fn host_tools(&self) -> bool {
        cfg!(debug_assertions) || self.chose_host_tools()
    }

    /// The player turned the Host page on in Preferences.
    #[must_use]
    pub fn chose_host_tools(&self) -> bool {
        self.flag("hostTools")
    }

    /// Minimising hides to the tray.
    #[must_use]
    pub fn minimize_to_tray(&self) -> bool {
        self.flag("minimizeToTray")
    }

    /// Closing hides to the tray.
    #[must_use]
    pub fn close_to_tray(&self) -> bool {
        self.flag("closeToTray")
    }

    /// The game's console window stays visible.
    #[must_use]
    pub fn show_game_console(&self) -> bool {
        self.flag("showGameConsole")
    }

    /// Under Proton, the `dainty-inject.exe` helper loads the runtime rather than the runtime loader.
    #[must_use]
    pub fn inject_helper(&self) -> bool {
        self.flag("injectHelper")
    }

    /// The account name on server `id`.
    #[must_use]
    pub fn username(&self, id: &str) -> Option<&str> {
        self.keyed("usernames", id).and_then(Value::as_str)
    }

    /// A key backup was offered for server `id`.
    #[must_use]
    pub fn backup_offered(&self, id: &str) -> bool {
        self.keyed("backupOffered", id)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// The runtime sets trusted on server `id`.
    #[must_use]
    pub fn trusted_runtimes(&self, id: &str) -> Vec<String> {
        self.keyed("trustedRuntimes", id)
            .and_then(Value::as_array)
            .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// The local runtime folder.
    #[must_use]
    pub fn local_runtime_dir(&self) -> Option<&str> {
        self.text("localRuntimeDir")
    }

    /// The Host workspace.
    #[must_use]
    pub fn host_workspace(&self) -> Option<&str> {
        self.text("hostWorkspace")
    }

    /// The Dumper-7 DLL.
    #[must_use]
    pub fn dumper_dll(&self) -> Option<&str> {
        self.text("dumperDll")
    }
}

/// Edits on the raw map; the store sanitises after each.
pub mod edit {
    use super::{Map, StoredServer, Value};

    fn keyed<'a>(map: &'a mut Map<String, Value>, name: &str) -> Option<&'a mut Map<String, Value>> {
        let entry = map.entry(name.to_owned()).or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        entry.as_object_mut()
    }

    /// Makes `server` current (or none).
    pub fn set_server(map: &mut Map<String, Value>, server: Option<&StoredServer>) {
        map.insert("server".into(), server.map_or(Value::Null, StoredServer::to_value));
    }

    /// One field of the current server.
    pub fn server_field(map: &mut Map<String, Value>, field: &str, value: Value) {
        if let Some(Value::Object(server)) = map.get_mut("server") {
            server.insert(field.to_owned(), value);
        }
    }

    /// Sets or removes a value under `name` for server `id`.
    pub fn keyed_value(map: &mut Map<String, Value>, name: &str, id: &str, value: Option<Value>) {
        let Some(inner) = keyed(map, name) else { return };
        match value {
            Some(value) => {
                inner.insert(id.to_owned(), value);
            }
            None => {
                inner.remove(id);
            }
        }
    }

    /// A top-level field, `null` to clear it.
    pub fn field(map: &mut Map<String, Value>, name: &str, value: Value) {
        map.insert(name.to_owned(), value);
    }

    /// Drops the saved entry with key-store id `id`.
    pub fn remove_saved(map: &mut Map<String, Value>, id: &str) {
        if let Some(Value::Array(list)) = map.get_mut("savedServers") {
            list.retain(|entry| StoredServer::from_value(entry).is_none_or(|s| s.id() != id));
        }
    }
}
