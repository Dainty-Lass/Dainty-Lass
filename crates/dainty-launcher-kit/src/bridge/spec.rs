// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The bridge's command table (RD27 spec §2.3.1–2.3.3): every command the page may call, in the order the
//! generated files list them, with the shape of each argument and the build that serves it.

use crate::core::text::utf16_len;

/// What one argument must look like. The isolation hook checks the same shapes (`commands.js`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A string of at most `max` UTF-16 units.
    Text {
        /// Longest accepted length.
        max: usize,
    },
    /// Such a string, or absent, or null.
    OptionalText {
        /// Longest accepted length.
        max: usize,
    },
    /// A boolean.
    Flag,
    /// An integral number in `0..=max`.
    Index {
        /// Highest accepted value.
        max: u32,
    },
    /// One of a fixed list of strings.
    Name(&'static [&'static str]),
    /// A finite number in `min..=max`.
    Number {
        /// Lowest accepted value.
        min: i64,
        /// Highest accepted value.
        max: i64,
    },
    /// An object with exactly these fields, or null when `nullable`.
    Object {
        /// Null is accepted.
        nullable: bool,
        /// The fields, in the table's order.
        fields: &'static [Field],
    },
    /// Settings names mapped to strings, numbers or booleans.
    SettingsPatch,
    /// A Tauri channel id.
    Channel,
}

/// A named argument, or a named field of an object argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    /// The page's (camelCase) name.
    pub name: &'static str,
    /// What its value must look like.
    pub shape: Shape,
}

/// The builds that serve a command (§2.3.2). Every build registers every command; the others refuse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// Every build.
    Every,
    /// Builds with the Host page (`host` feature).
    Host,
    /// The server manager (`server-gui` feature).
    ServerGui,
    /// Builds with realms (`realms` feature).
    Realms,
}

/// One row of the command table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandSpec {
    /// The command's name, as the page invokes it.
    pub name: &'static str,
    /// Its arguments, in order; empty when it takes none.
    pub args: &'static [Field],
    /// Which builds serve it.
    pub group: Group,
    /// Answered on the calling thread rather than as a task (§2.3.4).
    pub sync: bool,
}

/// Longest invite text.
pub const INVITE_MAX: usize = 2048;
/// A server id's exact length (lower-case hex).
pub const SERVER_ID_LEN: usize = 24;
/// Longest username.
pub const USERNAME_MAX: usize = 32;
/// Longest pasted account key.
pub const KEY_TEXT_MAX: usize = 8192;
/// Longest game path.
pub const GAME_PATH_MAX: usize = 260;
/// Highest detected-game index.
pub const DETECTED_INDEX_MAX: u32 = 15;
/// Longest server name.
pub const SERVER_NAME_MAX: usize = 60;
/// Longest public host name.
pub const PUBLIC_HOST_MAX: usize = 253;
/// Longest "invite for" name.
pub const INVITE_FOR_MAX: usize = 40;
/// Longest text put on the clipboard.
pub const COPY_TEXT_MAX: usize = 4096;
/// Longest Epic authorisation code.
pub const EPIC_CODE_MAX: usize = 1024;
/// Longest game version.
pub const GAME_VERSION_MAX: usize = 64;
/// Longest module id.
pub const MODULE_ID_MAX: usize = 64;
/// Longest module command name.
pub const MODULE_COMMAND_MAX: usize = 64;
/// Longest module JSON (command input or settings).
pub const MODULE_JSON_MAX: usize = 8192;
/// Longest realm id.
pub const REALM_ID_MAX: usize = 128;
/// Longest account or actor in a history filter.
pub const HISTORY_WHO_MAX: usize = 64;
/// Longest time in a history filter.
pub const HISTORY_TIME_MAX: usize = 40;
/// Longest history page cursor.
pub const HISTORY_CURSOR_MAX: usize = 128;
/// Longest change id (revert plan, guarded action).
pub const CHANGE_ID_MAX: usize = 64;
/// A guarded action id's exact length (lower-case hex).
pub const GUARDED_ID_LEN: usize = 16;
/// Longest admin player name.
pub const ADMIN_PLAYER_MAX: usize = 64;
/// Longest admin reason.
pub const ADMIN_REASON_MAX: usize = 500;
/// Longest remote node id or region.
pub const REMOTE_NODE_MAX: usize = 64;
/// Longest remote label.
pub const REMOTE_LABEL_MAX: usize = 128;
/// Longest remote URL.
pub const REMOTE_URL_MAX: usize = 512;
/// Longest remote enrolment URI.
pub const REMOTE_URI_MAX: usize = 2048;
/// Highest hub count in the server config.
pub const MAX_HUBS: u32 = 64;
/// Highest hunt count in the server config.
pub const MAX_HUNTS: u32 = 128;
/// Lowest multiplier.
pub const MULTIPLIER_MIN: i64 = 1;
/// Highest multiplier ([`super::types::MAX_MULTIPLIER`]).
pub const MULTIPLIER_MAX: i64 = 100;
/// Longest string value in a settings patch, as the hook checks it.
pub const SETTINGS_TEXT_MAX: usize = 16;

/// Settings a patch may name ([`super::types::SettingsPatch`]'s fields, in order).
pub const SETTINGS_KEYS: &[&str] = &[
    "graphics",
    "windowed",
    "streamerMode",
    "hostTools",
    "language",
    "gameLanguage",
    "runtimeProfile",
    "gameEdition",
    "minimizeToTray",
    "closeToTray",
    "showGameConsole",
    "injectHelper",
];

/// [`super::types::InstallMode`]'s names.
pub const INSTALL_MODES: &[&str] = &["install", "repair"];

/// [`super::types::ExternalTarget`]'s names.
pub const EXTERNAL_TARGETS: &[&str] = &[
    "tailscale_download",
    "tailscale_share",
    "vc_redist",
    "server_source",
    "project_source",
    "project_license",
    "project_issues",
    "project_docs_players",
    "project_docs_hosting",
    "project_docs_sdk",
    "installing_guide",
    "legendary",
    "dumper7",
    "visual_studio",
    "rust",
    "nodejs",
    "mongodb",
    "git",
    "visual_studio_installer",
    "rust_installer",
    "dumper7_zip",
    "revived_source",
    "mystic_source",
    "undaunted_source",
];

/// [`super::types::HostAction`]'s names.
pub const HOST_ACTIONS: &[&str] = &[
    "inject_dumper",
    "import_sdk",
    "build_runtime",
    "build_loader",
    "build_exporter",
    "inject_exporter",
    "copy_exports",
    "synthetic_data",
    "publish_runtime",
    "generate_gamedata",
    "export_gamedata",
];

/// The server components `host_server_restart` may name.
pub const HOST_SERVER_COMPONENTS: &[&str] = &[
    "allowlist",
    "backend",
    "director",
    "content",
    "admin",
    "gateway",
    "metagame",
    "legacy-allowlist",
    "deploy",
    "legacy-content",
    "legacy-gateway",
];

/// [`super::types::HostFolder`]'s names.
pub const HOST_FOLDERS: &[&str] = &[
    "workspace",
    "runtime_source",
    "dumps",
    "game_data",
    "runtime_publish",
    "host_logs",
    "gamedata_packs",
];

/// [`super::types::HostServerMode`]'s names.
pub const SERVER_MODES: &[&str] = &["private", "public"];
/// [`super::types::RealmKind`]'s names.
pub const REALM_KINDS: &[&str] = &["hub", "hunt"];
/// [`super::types::AdminActionKind`]'s names.
pub const ADMIN_ACTIONS: &[&str] = &["lookup", "kick", "timeout", "ban", "unban"];
/// Registration modes the server config may choose.
pub const REGISTRATION_MODES: &[&str] = &["OPEN", "INVITECODE", "NONE", "INVITE"];
/// Database engines the server config may choose.
pub const DATABASE_ENGINES: &[&str] = &["sqlite", "postgres"];
/// What `remote_exec` may ask a managed install to do.
pub const REMOTE_EXEC_ACTIONS: &[&str] = &[
    "doctor",
    "backup",
    "invite",
    "ban",
    "unban",
    "kick",
    "announce",
    "guarded_approve",
    "guarded_cancel",
];

/// Commands registered ahead of their rows (none today, §2.3.2).
pub const PLANNED: &[&str] = &[];

/// Events Rust emits to the window `main` (§2.3.5).
pub const EVENTS: &[&str] = &[
    "snapshot",
    "invite-link",
    "host-kit",
    "host-server",
    "window-state",
    "game-log",
];

const fn text(name: &'static str, max: usize) -> Field {
    Field {
        name,
        shape: Shape::Text { max },
    }
}

const fn opt(name: &'static str, max: usize) -> Field {
    Field {
        name,
        shape: Shape::OptionalText { max },
    }
}

const fn flag(name: &'static str) -> Field {
    Field {
        name,
        shape: Shape::Flag,
    }
}

const fn index(name: &'static str, max: u32) -> Field {
    Field {
        name,
        shape: Shape::Index { max },
    }
}

const fn names(name: &'static str, list: &'static [&'static str]) -> Field {
    Field {
        name,
        shape: Shape::Name(list),
    }
}

const fn number(name: &'static str, min: i64, max: i64) -> Field {
    Field {
        name,
        shape: Shape::Number { min, max },
    }
}

const fn object(name: &'static str, nullable: bool, fields: &'static [Field]) -> Field {
    Field {
        name,
        shape: Shape::Object { nullable, fields },
    }
}

const fn cmd(name: &'static str, args: &'static [Field], group: Group, sync: bool) -> CommandSpec {
    CommandSpec {
        name,
        args,
        group,
        sync,
    }
}

use Group::{Every, Host, Realms, ServerGui};

const HISTORY_SELECTION: [Field; 4] = [
    opt("account", HISTORY_WHO_MAX),
    opt("actor", HISTORY_WHO_MAX),
    opt("since", HISTORY_TIME_MAX),
    opt("until", HISTORY_TIME_MAX),
];

const HISTORY_FILTER: [Field; 5] = [
    HISTORY_SELECTION[0],
    HISTORY_SELECTION[1],
    HISTORY_SELECTION[2],
    HISTORY_SELECTION[3],
    opt("cursor", HISTORY_CURSOR_MAX),
];

const VERSION: &[Field] = &[text("version", GAME_VERSION_MAX)];
const MODULE: &[Field] = &[text("module", MODULE_ID_MAX)];
const SERVER_ID: &[Field] = &[text("id", SERVER_ID_LEN)];
const DETECTED: &[Field] = &[index("index", DETECTED_INDEX_MAX)];
const GUARDED_ID: &[Field] = &[text("id", CHANGE_ID_MAX)];

/// Every command the page may call, in the generated files' order (§2.3.1).
pub const COMMANDS: &[CommandSpec] = &[
    cmd("get_snapshot", &[], Every, true),
    cmd("check_invite", &[text("text", INVITE_MAX)], Every, true),
    cmd(
        "submit_invite",
        &[
            text("text", INVITE_MAX),
            flag("acceptNewCertificate"),
            flag("carryAccount"),
        ],
        Every,
        false,
    ),
    cmd("retry_connect", &[], Every, false),
    cmd("forget_server", &[], Every, false),
    cmd("switch_server", SERVER_ID, Every, false),
    cmd("remove_saved_server", SERVER_ID, Every, false),
    cmd("register", &[text("username", USERNAME_MAX)], Every, false),
    cmd("use_existing_key", &[text("text", KEY_TEXT_MAX)], Every, false),
    cmd("import_key_file", &[], Every, false),
    cmd("save_key_backup", &[], Every, false),
    cmd("dismiss_backup_offer", &[], Every, false),
    cmd("choose_game_folder", &[], Every, false),
    cmd("use_game_path", &[text("gamePath", GAME_PATH_MAX)], Every, false),
    cmd("use_detected_game", DETECTED, Every, false),
    cmd("verify_game", &[], Every, false),
    cmd("revived_install", &[names("mode", INSTALL_MODES)], Every, false),
    cmd("revived_choose_folder", &[], Every, false),
    cmd("revived_use_detected", DETECTED, Every, false),
    cmd("trust_runtime", &[], Every, false),
    cmd("install_runtime", &[], Every, false),
    cmd("choose_local_runtime", &[], Every, false),
    cmd("clear_local_runtime", &[], Every, false),
    cmd("cancel_task", &[], Every, true),
    cmd("play", &[], Every, false),
    cmd(
        "set_settings",
        &[Field {
            name: "patch",
            shape: Shape::SettingsPatch,
        }],
        Every,
        false,
    ),
    cmd("set_status_polling", &[flag("on")], Every, true),
    cmd("refresh_status", &[], Every, false),
    cmd("get_news", &[], Every, true),
    cmd("get_branding", &[], Every, true),
    cmd("open_external", &[names("target", EXTERNAL_TARGETS)], Every, true),
    cmd("open_game_folder", &[], Every, true),
    cmd("open_logs_folder", &[], Every, true),
    cmd("logout", &[], Every, false),
    cmd("install_update", &[], Every, false),
    cmd("host_get", &[], Host, false),
    cmd("host_choose_workspace", &[], Host, false),
    cmd("host_choose_dumper", &[], Host, false),
    cmd("host_import_gamedata", &[], Host, false),
    cmd("host_import_sdk_folder", &[], Host, false),
    cmd("host_download_project", &[], Host, false),
    cmd("host_action", &[names("action", HOST_ACTIONS)], Host, false),
    cmd("host_open", &[names("folder", HOST_FOLDERS)], Host, true),
    cmd("host_server_get", &[], Host, true),
    cmd("host_server_start", &[], Host, false),
    cmd("host_server_stop", &[], Host, false),
    cmd(
        "host_server_restart",
        &[names("name", HOST_SERVER_COMPONENTS)],
        Host,
        false,
    ),
    cmd(
        "host_setup",
        &[object(
            "request",
            false,
            &[
                text("name", SERVER_NAME_MAX),
                names("mode", SERVER_MODES),
                opt("publicHost", PUBLIC_HOST_MAX),
            ],
        )],
        Host,
        false,
    ),
    cmd("host_create_invite", &[opt("forName", INVITE_FOR_MAX)], Host, false),
    cmd("host_update", &[], Host, false),
    cmd(
        "host_set_multipliers",
        &[object(
            "multipliers",
            true,
            &[
                number("xp", MULTIPLIER_MIN, MULTIPLIER_MAX),
                number("rewards", MULTIPLIER_MIN, MULTIPLIER_MAX),
            ],
        )],
        Host,
        false,
    ),
    cmd("copy_text", &[text("text", COPY_TEXT_MAX)], Every, true),
    cmd("take_invite_link", &[], Every, true),
    cmd("game_log_get", &[], Every, true),
    cmd("dismiss_notice", &[], Every, true),
    cmd("dismiss_error", &[], Every, true),
    cmd(
        "subscribe_progress",
        &[Field {
            name: "onProgress",
            shape: Shape::Channel,
        }],
        Every,
        true,
    ),
    cmd("epic_sign_in_start", &[], Every, false),
    cmd("epic_sign_in_code", &[text("code", EPIC_CODE_MAX)], Every, false),
    cmd("epic_get", &[], Every, true),
    cmd("epic_sign_out", &[], Every, false),
    cmd("epic_library_check", &[], Every, false),
    cmd("game_version_list", &[], Every, false),
    cmd("game_install_start", VERSION, Every, false),
    cmd("game_install_pause", VERSION, Every, true),
    cmd("game_install_resume", VERSION, Every, true),
    cmd("game_install_cancel", VERSION, Every, true),
    cmd("game_install_get", VERSION, Every, true),
    // Where versions install: a folder dialog, refused while an install runs or waits to continue
    cmd("game_install_choose_folder", &[], Every, false),
    // Cancels a download altogether, deleting what it downloaded
    cmd("game_install_discard", VERSION, Every, false),
    cmd("region_ping", &[], Every, false),
    cmd("module_list", &[], ServerGui, false),
    cmd("module_get", MODULE, ServerGui, false),
    cmd(
        "module_run_command",
        &[
            text("module", MODULE_ID_MAX),
            text("command", MODULE_COMMAND_MAX),
            text("input", MODULE_JSON_MAX),
        ],
        ServerGui,
        false,
    ),
    cmd(
        "module_set_settings",
        &[text("module", MODULE_ID_MAX), text("values", MODULE_JSON_MAX)],
        ServerGui,
        false,
    ),
    cmd("module_load", MODULE, ServerGui, false),
    cmd("module_unload", MODULE, ServerGui, false),
    cmd("realm_host", &[names("kind", REALM_KINDS)], Realms, false),
    cmd("realm_join", &[text("realm", REALM_ID_MAX)], Realms, false),
    cmd("realm_stop", &[], Realms, false),
    cmd("realm_get", &[], Realms, false),
    cmd(
        "history_query",
        &[object("filter", false, &HISTORY_FILTER)],
        ServerGui,
        false,
    ),
    cmd(
        "history_revert_dry_run",
        &[object("selection", false, &HISTORY_SELECTION)],
        ServerGui,
        false,
    ),
    cmd("history_revert_apply", &[text("plan", CHANGE_ID_MAX)], ServerGui, false),
    cmd("guarded_list", &[], ServerGui, false),
    cmd("guarded_approve", GUARDED_ID, ServerGui, false),
    cmd("guarded_cancel", GUARDED_ID, ServerGui, false),
    cmd("server_doctor", &[], ServerGui, false),
    cmd("server_backup", &[], ServerGui, false),
    cmd("server_config_get", &[], ServerGui, false),
    cmd(
        "server_config_save",
        &[object(
            "config",
            false,
            &[
                names("registration", REGISTRATION_MODES),
                names("database", DATABASE_ENGINES),
                index("maxHubs", MAX_HUBS),
                index("maxHunts", MAX_HUNTS),
                flag("freshStart"),
            ],
        )],
        ServerGui,
        false,
    ),
    cmd("server_migrate_mongo", &[], ServerGui, false),
    cmd("server_confirm_fresh_start", &[], ServerGui, false),
    cmd("server_invite_list", &[], ServerGui, false),
    cmd(
        "admin_action",
        &[object(
            "request",
            false,
            &[
                names("action", ADMIN_ACTIONS),
                text("player", ADMIN_PLAYER_MAX),
                opt("reason", ADMIN_REASON_MAX),
                opt("until", HISTORY_TIME_MAX),
                opt("module", MODULE_ID_MAX),
            ],
        )],
        ServerGui,
        false,
    ),
    cmd("remote_status", &[], ServerGui, false),
    cmd(
        "remote_init_controller",
        &[object(
            "request",
            false,
            &[
                opt("node", REMOTE_NODE_MAX),
                opt("region", REMOTE_NODE_MAX),
                opt("url", REMOTE_URL_MAX),
            ],
        )],
        ServerGui,
        false,
    ),
    cmd(
        "remote_invite",
        &[object(
            "request",
            false,
            &[
                text("node", REMOTE_NODE_MAX),
                opt("region", REMOTE_NODE_MAX),
                opt("label", REMOTE_LABEL_MAX),
                opt("url", REMOTE_URL_MAX),
            ],
        )],
        ServerGui,
        false,
    ),
    cmd(
        "remote_enroll",
        &[object(
            "request",
            false,
            &[text("uri", REMOTE_URI_MAX), opt("label", REMOTE_LABEL_MAX)],
        )],
        ServerGui,
        false,
    ),
    cmd("remote_revoke", &[text("node", REMOTE_NODE_MAX)], ServerGui, false),
    cmd("remote_leave", &[], ServerGui, false),
    cmd(
        "remote_exec",
        &[object(
            "request",
            false,
            &[
                text("node", REMOTE_NODE_MAX),
                names("action", REMOTE_EXEC_ACTIONS),
                opt("target", ADMIN_PLAYER_MAX),
                opt("reason", ADMIN_REASON_MAX),
            ],
        )],
        ServerGui,
        false,
    ),
    cmd("update_launcher", &[], Every, false),
    // What uninstalling a version deletes, then the uninstall: refused while the game runs
    cmd("game_uninstall_plan", VERSION, Every, false),
    cmd("game_uninstall", VERSION, Every, false),
    // The signed-in account's online seconds, for the account page's badges
    cmd("get_playtime", &[], Every, false),
    // PLAY waiting in the server's login queue gives up its place
    cmd("leave_queue", &[], Every, true),
];

/// The command named `name`.
pub fn command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|spec| spec.name == name)
}

/// The features a build was compiled with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Build {
    /// The Host page (`host`).
    pub host: bool,
    /// The server manager (`server-gui`).
    pub server_gui: bool,
    /// Realms (`realms`).
    pub realms: bool,
}

impl Build {
    /// The build this crate was compiled as.
    pub const fn current() -> Self {
        Self {
            host: cfg!(feature = "host"),
            server_gui: cfg!(feature = "server-gui"),
            realms: cfg!(feature = "realms"),
        }
    }

    /// Whether this build serves `group`'s commands.
    pub const fn serves(self, group: Group) -> bool {
        match group {
            Group::Every => true,
            Group::Host => self.host,
            Group::ServerGui => self.server_gui,
            Group::Realms => self.realms,
        }
    }
}

/// `text` is at most `max` long. JavaScript counts UTF-16 units, never more than UTF-8 bytes, so this is the
/// looser of the two counts (§2.3.3).
pub fn fits(text: &str, max: usize) -> bool {
    utf16_len(text) <= max
}

/// `text` is exactly `len` lower-case hex digits.
pub fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `text` is a saved server's id.
pub fn is_server_id(text: &str) -> bool {
    is_lower_hex(text, SERVER_ID_LEN)
}

/// `value` fits `shape` as the isolation hook judges it; `None` is an absent field (§2.3.7).
pub fn fits_shape(shape: &Shape, value: Option<&serde_json::Value>) -> bool {
    use serde_json::Value;
    match (shape, value) {
        (Shape::OptionalText { .. }, None | Some(Value::Null)) => true,
        (Shape::Text { max } | Shape::OptionalText { max }, Some(Value::String(s))) => fits(s, *max),
        (Shape::Flag, Some(Value::Bool(_))) => true,
        (Shape::Index { max }, Some(Value::Number(n))) => n
            .as_f64()
            .is_some_and(|f| f.fract() == 0.0 && (0.0..=f64::from(*max)).contains(&f)),
        (Shape::Name(list), Some(Value::String(s))) => list.contains(&s.as_str()),
        #[allow(clippy::cast_precision_loss, reason = "the bounds are small whole numbers")]
        (Shape::Number { min, max }, Some(Value::Number(n))) => n
            .as_f64()
            .is_some_and(|f| f.is_finite() && f >= *min as f64 && f <= *max as f64),
        (Shape::Object { nullable, .. }, Some(Value::Null)) => *nullable,
        (Shape::Object { fields, .. }, Some(v @ Value::Object(_))) => fits_fields(fields, v),
        (Shape::SettingsPatch, Some(Value::Object(map))) => map.iter().all(|(key, v)| {
            SETTINGS_KEYS.contains(&key.as_str())
                && match v {
                    Value::Number(_) | Value::Bool(_) => true,
                    Value::String(s) => fits(s, SETTINGS_TEXT_MAX),
                    _ => false,
                }
        }),
        (Shape::Channel, Some(Value::String(s))) => is_channel(s),
        _ => false,
    }
}

/// `value` is an object (or, for no fields, absent or null) holding exactly `fields`, each fitting its shape.
pub fn fits_fields(fields: &[Field], value: &serde_json::Value) -> bool {
    use serde_json::Value;
    let empty = serde_json::Map::new();
    let map = match value {
        Value::Object(map) => map,
        Value::Null => &empty,
        _ => return false,
    };
    map.keys().all(|key| fields.iter().any(|f| f.name == key))
        && fields.iter().all(|f| fits_shape(&f.shape, map.get(f.name)))
}

fn is_channel(text: &str) -> bool {
    text.strip_prefix("__CHANNEL__:")
        .is_some_and(|id| (1..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use std::collections::HashSet;

    use serde_json::json;

    use super::*;

    #[test]
    fn the_table_has_a_hundred_and_three_unique_commands() {
        assert_eq!(COMMANDS.len(), 103);
        let mut seen = HashSet::new();
        for spec in COMMANDS {
            assert!(seen.insert(spec.name), "{} twice", spec.name);
            assert_eq!(command(spec.name), Some(spec));
        }
        assert_eq!(command("nope"), None);
        assert!(PLANNED.iter().all(|name| command(name).is_some()));
    }

    #[test]
    fn groups_follow_the_names() {
        let count = |g| COMMANDS.iter().filter(|c| c.group == g).count();
        assert_eq!(count(Group::ServerGui), 27);
        assert_eq!(count(Group::Realms), 4);
        for spec in COMMANDS {
            let expected = if spec.name.starts_with("realm_") {
                Group::Realms
            } else if spec.name.starts_with("host_") {
                Group::Host
            } else {
                spec.group
            };
            assert_eq!(spec.group, expected, "{}", spec.name);
        }
        let player = Build::default();
        assert!(player.serves(Group::Every));
        assert!(!player.serves(Group::Host) && !player.serves(Group::ServerGui) && !player.serves(Group::Realms));
    }

    #[test]
    fn lengths_count_utf16_units() {
        assert!(fits("", 0));
        assert!(fits("a\u{1F600}", 3));
        assert!(!fits("a\u{1F600}", 2));
        assert!(fits("\u{e9}\u{e9}", 2));
    }

    #[test]
    fn ids_are_lower_hex() {
        assert!(is_server_id("0123456789abcdef01234567"));
        assert!(!is_server_id("0123456789ABCDEF01234567"));
        assert!(!is_server_id("0123456789abcdef0123456"));
        assert!(!is_server_id("0123456789abcdef0123456g"));
        assert!(is_lower_hex("00ff00ff00ff00ff", GUARDED_ID_LEN));
    }

    #[test]
    fn shapes_match_the_hook() {
        let index = Shape::Index { max: 15 };
        assert!(fits_shape(&index, Some(&json!(15))));
        assert!(fits_shape(&index, Some(&json!(3.0))));
        assert!(!fits_shape(&index, Some(&json!(16))));
        assert!(!fits_shape(&index, Some(&json!(1.5))));
        assert!(!fits_shape(&index, Some(&json!(-1))));
        assert!(!fits_shape(&index, Some(&json!("1"))));
        let opt = Shape::OptionalText { max: 2 };
        assert!(fits_shape(&opt, None) && fits_shape(&opt, Some(&json!(null))));
        assert!(!fits_shape(&opt, Some(&json!("abc"))));
        assert!(!fits_shape(&Shape::Text { max: 2 }, None));
        assert!(fits_shape(&Shape::Channel, Some(&json!("__CHANNEL__:12"))));
        assert!(!fits_shape(&Shape::Channel, Some(&json!("__CHANNEL__:"))));
        assert!(fits_shape(
            &Shape::SettingsPatch,
            Some(&json!({"graphics": 2, "language": "pt-BR"}))
        ));
        assert!(!fits_shape(&Shape::SettingsPatch, Some(&json!({"other": 1}))));
        assert!(!fits_shape(
            &Shape::SettingsPatch,
            Some(&json!({"language": "x".repeat(17)}))
        ));
    }

    #[test]
    fn objects_hold_exactly_their_fields() {
        let setup = command("host_setup").unwrap().args;
        assert!(fits_fields(
            setup,
            &json!({"request": {"name": "A", "mode": "private"}})
        ));
        assert!(!fits_fields(
            setup,
            &json!({"request": {"name": "A", "mode": "private", "x": 1}})
        ));
        assert!(!fits_fields(setup, &json!({"request": null})));
        assert!(!fits_fields(setup, &json!({})));
        let multipliers = command("host_set_multipliers").unwrap().args;
        assert!(fits_fields(multipliers, &json!({"multipliers": null})));
        assert!(fits_fields(
            multipliers,
            &json!({"multipliers": {"xp": 1.5, "rewards": 100}})
        ));
        assert!(!fits_fields(
            multipliers,
            &json!({"multipliers": {"xp": 0.5, "rewards": 1}})
        ));
        assert!(fits_fields(&[], &json!(null)) && fits_fields(&[], &json!({})));
        assert!(!fits_fields(&[], &json!({"x": 1})));
    }
}
