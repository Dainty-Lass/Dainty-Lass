//! The fixed list of calls the page may make (rows L6, L7, L9; RA2): one entry per `LauncherApi` method
//! that the backend serves, with the shape of its arguments. Three layers read
//! it: the isolation hook in the page (`isolation/commands.js`, generated from this list), the capability
//! that grants exactly these commands (`build.rs`), and the commands themselves, which check the same
//! limits again. The `revived*` methods (1.4.4) are served and always refused (RD5); the window controls are Tauri's own. After `subscribe_progress` come Dainty Lass's own
//! commands (DF8), stubs until their rows fill them (`PLANNED`).
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "build.rs and the generator read the list; the commands use its limits"
    )
)]

/// One argument, as the page sends it (JSON, camelCase names). The generator writes each as the
/// hook's `{ kind, ... }` object.
#[derive(Debug, Clone, Copy)]
pub enum Arg {
    /// A string of at most `max` UTF-16 code units (JavaScript's `length`)
    Text {
        max: usize,
    },
    /// The same, or absent, or null
    OptionalText {
        max: usize,
    },
    Flag,
    /// An integer from 0 to `max`
    Index {
        max: u32,
    },
    /// One of a fixed list of names
    Name {
        names: &'static [&'static str],
    },
    /// A finite number from `min` to `max`
    Number {
        min: f64,
        max: f64,
    },
    /// An object with exactly these fields (each may be absent only if its kind allows it), or null when
    /// `nullable`
    Object {
        fields: &'static [(&'static str, Arg)],
        nullable: bool,
    },
    /// An object whose keys are settings names and whose values are strings, numbers or booleans
    SettingsPatch,
    /// A `Channel` the backend pushes values into
    Channel,
}

#[derive(Debug, Clone, Copy)]
pub struct Command {
    pub name: &'static str,
    pub args: &'static [(&'static str, Arg)],
}

const fn command(name: &'static str, args: &'static [(&'static str, Arg)]) -> Command {
    Command { name, args }
}

pub const INVITE_MAX: usize = 2048;
pub const USERNAME_MAX: usize = 32;
pub const KEY_TEXT_MAX: usize = 8192;
pub const GAME_PATH_MAX: usize = 260;
/// A saved server's id: 24 lower-case hex characters
pub const SERVER_ID_LEN: usize = 24;
/// `useDetectedGame`: an index into `game.detected`, below 16
pub const DETECTED_GAME_MAX: u32 = 15;
/// The Host page's setup form: the server's name and its public host (a DNS name or IPv4)
pub const SERVER_NAME_MAX: usize = 60;
pub const PUBLIC_HOST_MAX: usize = 253;
/// Who an invite is for
pub const INVITE_FOR_MAX: usize = 40;
/// `copyText`: an invite link, with room to spare
pub const COPY_TEXT_MAX: usize = 4096;

/// The settings `setSettings` may name.
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
];

/// `InstallMode`'s names (`revivedInstall`).
pub const INSTALL_MODES: &[&str] = &["install", "repair"];

/// `ExternalTarget`'s names (a test checks they are the enum's).
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

/// `HostAction`'s names.
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

/// The services of the server inside the launcher (`host_server_restart`): the host kit's module names.
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

/// `HostFolder`'s names.
pub const HOST_FOLDERS: &[&str] = &[
    "workspace",
    "runtime_source",
    "dumps",
    "game_data",
    "runtime_publish",
    "host_logs",
    "gamedata_packs",
];

/// `HostServerMode`'s names.
pub const SERVER_MODES: &[&str] = &["private", "public"];

/// The setup form: the hook checks the shape and the raw lengths; `commands::host_setup_request` trims
/// and checks the rest.
const HOST_SETUP: Arg = Arg::Object {
    fields: &[
        ("name", Arg::Text { max: SERVER_NAME_MAX }),
        ("mode", Arg::Name { names: SERVER_MODES }),
        ("publicHost", Arg::OptionalText { max: PUBLIC_HOST_MAX }),
    ],
    nullable: false,
};

const MULTIPLIER: Arg = Arg::Number {
    min: 1.0,
    max: super::types::MAX_MULTIPLIER,
};

/// The test multipliers, or null to turn them off.
const HOST_MULTIPLIERS: Arg = Arg::Object {
    fields: &[("xp", MULTIPLIER), ("rewards", MULTIPLIER)],
    nullable: true,
};

/// `epic_sign_in_code`: the pasted authorization code, or the JSON page Epic shows it in (DD22).
pub const EPIC_CODE_MAX: usize = 1024;
/// A game version as a client profile names it (DE4)
pub const GAME_VERSION_MAX: usize = 64;
/// A game module's manifest id, and a command name from its manifest (DD5, DD11)
pub const MODULE_ID_MAX: usize = 64;
pub const MODULE_COMMAND_MAX: usize = 64;
/// A module command's arguments or settings values, as JSON text: their shape is the manifest's, so the
/// backend checks it, not the hook.
pub const MODULE_JSON_MAX: usize = 8192;
/// What `realm_join` joins (RF7)
pub const REALM_ID_MAX: usize = 128;
/// A history filter's fields: an account, an actor (an admin key's id, the console, the host kit), RFC 3339
/// times and a page cursor (DD23)
pub const HISTORY_WHO_MAX: usize = 64;
pub const HISTORY_TIME_MAX: usize = 40;
pub const HISTORY_CURSOR_MAX: usize = 128;
/// A revert plan's id from `history_revert_dry_run`, or a guarded action's id (DH6)
pub const CHANGE_ID_MAX: usize = 64;

/// `RealmKind`'s names.
pub const REALM_KINDS: &[&str] = &["hub", "hunt"];

const HISTORY_FILTER: Arg = Arg::Object {
    fields: &[
        ("account", Arg::OptionalText { max: HISTORY_WHO_MAX }),
        ("actor", Arg::OptionalText { max: HISTORY_WHO_MAX }),
        ("since", Arg::OptionalText { max: HISTORY_TIME_MAX }),
        ("until", Arg::OptionalText { max: HISTORY_TIME_MAX }),
        (
            "cursor",
            Arg::OptionalText {
                max: HISTORY_CURSOR_MAX,
            },
        ),
    ],
    nullable: false,
};

/// What a revert undoes: the history filter without the cursor (`revert --actor <who> --since <time>`).
const REVERT_SELECTION: Arg = Arg::Object {
    fields: &[
        ("account", Arg::OptionalText { max: HISTORY_WHO_MAX }),
        ("actor", Arg::OptionalText { max: HISTORY_WHO_MAX }),
        ("since", Arg::OptionalText { max: HISTORY_TIME_MAX }),
        ("until", Arg::OptionalText { max: HISTORY_TIME_MAX }),
    ],
    nullable: false,
};

/// `AdminActionKind`'s names (`admin_action`, server GUI).
pub const ADMIN_ACTIONS: &[&str] = &["lookup", "kick", "timeout", "ban", "unban"];
pub const ADMIN_PLAYER_MAX: usize = 64;
pub const ADMIN_REASON_MAX: usize = 500;

const ADMIN_REQUEST: Arg = Arg::Object {
    fields: &[
        ("action", Arg::Name { names: ADMIN_ACTIONS }),
        ("player", Arg::Text { max: ADMIN_PLAYER_MAX }),
        ("reason", Arg::OptionalText { max: ADMIN_REASON_MAX }),
        ("until", Arg::OptionalText { max: HISTORY_TIME_MAX }),
        ("module", Arg::OptionalText { max: MODULE_ID_MAX }),
    ],
    nullable: false,
};

/// `host.toml` settings edited from the server GUI (`server_config_save`).
pub const REGISTRATION_MODES: &[&str] = &["OPEN", "INVITECODE", "NONE", "INVITE"];
pub const DATABASE_ENGINES: &[&str] = &["sqlite", "postgres"];

const SERVER_CONFIG_PATCH: Arg = Arg::Object {
    fields: &[
        (
            "registration",
            Arg::Name {
                names: REGISTRATION_MODES,
            },
        ),
        (
            "database",
            Arg::Name {
                names: DATABASE_ENGINES,
            },
        ),
        ("maxHubs", Arg::Index { max: 64 }),
        ("maxHunts", Arg::Index { max: 128 }),
        ("freshStart", Arg::Flag),
    ],
    nullable: false,
};

pub const REMOTE_LABEL_MAX: usize = 128;
pub const REMOTE_URL_MAX: usize = 512;
pub const REMOTE_URI_MAX: usize = 2048;
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

const REMOTE_INIT_REQUEST: Arg = Arg::Object {
    fields: &[
        ("node", Arg::OptionalText { max: MODULE_ID_MAX }),
        ("region", Arg::OptionalText { max: MODULE_ID_MAX }),
        ("url", Arg::OptionalText { max: REMOTE_URL_MAX }),
    ],
    nullable: false,
};

const REMOTE_INVITE_REQUEST: Arg = Arg::Object {
    fields: &[
        ("node", Arg::Text { max: MODULE_ID_MAX }),
        ("region", Arg::OptionalText { max: MODULE_ID_MAX }),
        ("label", Arg::OptionalText { max: REMOTE_LABEL_MAX }),
        ("url", Arg::OptionalText { max: REMOTE_URL_MAX }),
    ],
    nullable: false,
};

const REMOTE_ENROLL_REQUEST: Arg = Arg::Object {
    fields: &[
        ("uri", Arg::Text { max: REMOTE_URI_MAX }),
        ("label", Arg::OptionalText { max: REMOTE_LABEL_MAX }),
    ],
    nullable: false,
};

const REMOTE_EXEC_REQUEST: Arg = Arg::Object {
    fields: &[
        ("node", Arg::Text { max: MODULE_ID_MAX }),
        (
            "action",
            Arg::Name {
                names: REMOTE_EXEC_ACTIONS,
            },
        ),
        ("target", Arg::OptionalText { max: ADMIN_PLAYER_MAX }),
        ("reason", Arg::OptionalText { max: ADMIN_REASON_MAX }),
    ],
    nullable: false,
};

/// Which builds serve a command. Every build lists every command, so the bridge and its generated files
/// are the same in all; a build outside a command's own refuses it with a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    /// The player launcher and the server GUI
    Every,
    /// The Host page's `host_*` commands: builds with the `host` feature (`host_off.rs` refuses them)
    Host,
    /// The server GUI only (`server-gui` feature)
    ServerGui,
    /// Realm hosting and joining (`realms` feature, RF7)
    Realms,
}

/// The server GUI's own commands: module panels (DU3), history and revert (DH3b), guarded actions (DH6),
/// doctor, backup, config, MongoDB migration, player administration (B3) and remote management (DD27).
pub const SERVER_GUI_COMMANDS: &[&str] = &[
    "module_list",
    "module_get",
    "module_run_command",
    "module_set_settings",
    "module_load",
    "module_unload",
    "history_query",
    "history_revert_dry_run",
    "history_revert_apply",
    "guarded_list",
    "guarded_approve",
    "guarded_cancel",
    "server_doctor",
    "server_backup",
    "server_config_get",
    "server_config_save",
    "server_migrate_mongo",
    "server_confirm_fresh_start",
    "server_invite_list",
    "admin_action",
    "remote_status",
    "remote_init_controller",
    "remote_invite",
    "remote_enroll",
    "remote_revoke",
    "remote_leave",
    "remote_exec",
];

/// Realm hosting and joining commands (`realms` feature, RF7).
pub const REALM_COMMANDS: &[&str] = &["realm_host", "realm_join", "realm_stop", "realm_get"];

pub fn build(name: &str) -> Build {
    if SERVER_GUI_COMMANDS.contains(&name) {
        Build::ServerGui
    } else if REALM_COMMANDS.contains(&name) {
        Build::Realms
    } else if name.starts_with("host_") {
        Build::Host
    } else {
        Build::Every
    }
}

/// Whether this build serves a command.
pub fn serves(name: &str) -> bool {
    match build(name) {
        Build::Every => true,
        Build::Host => cfg!(feature = "host"),
        Build::ServerGui => cfg!(feature = "server-gui"),
        Build::Realms => cfg!(feature = "realms"),
    }
}

/// The commands registered ahead of the rows that fill them (DF8), each refused with `not_implemented`
/// (`commands::planned`). A row that fills one takes it off this list.
pub const PLANNED: &[&str] = &[];

pub const COMMANDS: &[Command] = &[
    command("get_snapshot", &[]),
    command("check_invite", &[("text", Arg::Text { max: INVITE_MAX })]),
    command(
        "submit_invite",
        &[
            ("text", Arg::Text { max: INVITE_MAX }),
            ("acceptNewCertificate", Arg::Flag),
            ("carryAccount", Arg::Flag),
        ],
    ),
    command("retry_connect", &[]),
    command("forget_server", &[]),
    command("switch_server", &[("id", Arg::Text { max: SERVER_ID_LEN })]),
    command("remove_saved_server", &[("id", Arg::Text { max: SERVER_ID_LEN })]),
    command("register", &[("username", Arg::Text { max: USERNAME_MAX })]),
    command("use_existing_key", &[("text", Arg::Text { max: KEY_TEXT_MAX })]),
    command("import_key_file", &[]),
    command("save_key_backup", &[]),
    command("dismiss_backup_offer", &[]),
    command("choose_game_folder", &[]),
    command("use_game_path", &[("gamePath", Arg::Text { max: GAME_PATH_MAX })]),
    command("use_detected_game", &[("index", Arg::Index { max: DETECTED_GAME_MAX })]),
    command("verify_game", &[]),
    command("revived_install", &[("mode", Arg::Name { names: INSTALL_MODES })]),
    command("revived_choose_folder", &[]),
    command(
        "revived_use_detected",
        &[("index", Arg::Index { max: DETECTED_GAME_MAX })],
    ),
    command("trust_runtime", &[]),
    command("install_runtime", &[]),
    command("choose_local_runtime", &[]),
    command("clear_local_runtime", &[]),
    command("cancel_task", &[]),
    command("play", &[]),
    command("set_settings", &[("patch", Arg::SettingsPatch)]),
    command("set_status_polling", &[("on", Arg::Flag)]),
    command("refresh_status", &[]),
    command("get_news", &[]),
    command("get_branding", &[]),
    command(
        "open_external",
        &[(
            "target",
            Arg::Name {
                names: EXTERNAL_TARGETS,
            },
        )],
    ),
    command("open_game_folder", &[]),
    command("open_logs_folder", &[]),
    command("logout", &[]),
    command("install_update", &[]),
    command("host_get", &[]),
    command("host_choose_workspace", &[]),
    command("host_choose_dumper", &[]),
    command("host_import_gamedata", &[]),
    command("host_import_sdk_folder", &[]),
    command("host_download_project", &[]),
    command("host_action", &[("action", Arg::Name { names: HOST_ACTIONS })]),
    command("host_open", &[("folder", Arg::Name { names: HOST_FOLDERS })]),
    command("host_server_get", &[]),
    command("host_server_start", &[]),
    command("host_server_stop", &[]),
    command(
        "host_server_restart",
        &[(
            "name",
            Arg::Name {
                names: HOST_SERVER_COMPONENTS,
            },
        )],
    ),
    command("host_setup", &[("request", HOST_SETUP)]),
    command(
        "host_create_invite",
        &[("forName", Arg::OptionalText { max: INVITE_FOR_MAX })],
    ),
    command("host_update", &[]),
    command("host_set_multipliers", &[("multipliers", HOST_MULTIPLIERS)]),
    command("copy_text", &[("text", Arg::Text { max: COPY_TEXT_MAX })]),
    command("take_invite_link", &[]),
    command("game_log_get", &[]),
    command("dismiss_notice", &[]),
    command("dismiss_error", &[]),
    command("subscribe_progress", &[("onProgress", Arg::Channel)]),
    // Epic sign-in and library (DE2, DE3; every build). The session stays in the keystore: no answer
    // carries it, as none carries the account key (DD22)
    command("epic_sign_in_start", &[]),
    command("epic_sign_in_code", &[("code", Arg::Text { max: EPIC_CODE_MAX })]),
    command("epic_get", &[]),
    command("epic_sign_out", &[]),
    command("epic_library_check", &[]),
    // Installable versions and their download (DE4, DE5; every build)
    command("game_version_list", &[]),
    command(
        "game_install_start",
        &[("version", Arg::Text { max: GAME_VERSION_MAX })],
    ),
    command(
        "game_install_pause",
        &[("version", Arg::Text { max: GAME_VERSION_MAX })],
    ),
    command(
        "game_install_resume",
        &[("version", Arg::Text { max: GAME_VERSION_MAX })],
    ),
    command(
        "game_install_cancel",
        &[("version", Arg::Text { max: GAME_VERSION_MAX })],
    ),
    command("game_install_get", &[("version", Arg::Text { max: GAME_VERSION_MAX })]),
    // Where versions install: a folder dialog, refused while an install runs or waits to continue
    command("game_install_choose_folder", &[]),
    // Round trip to every node, in order (DN7; every build)
    command("region_ping", &[]),
    // Module panels drawn from manifests, load and unload (DU3, DF4d; server GUI): by manifest id, never
    // a path (DD5)
    command("module_list", &[]),
    command("module_get", &[("module", Arg::Text { max: MODULE_ID_MAX })]),
    command(
        "module_run_command",
        &[
            ("module", Arg::Text { max: MODULE_ID_MAX }),
            (
                "command",
                Arg::Text {
                    max: MODULE_COMMAND_MAX,
                },
            ),
            ("input", Arg::Text { max: MODULE_JSON_MAX }),
        ],
    ),
    command(
        "module_set_settings",
        &[
            ("module", Arg::Text { max: MODULE_ID_MAX }),
            ("values", Arg::Text { max: MODULE_JSON_MAX }),
        ],
    ),
    command("module_load", &[("module", Arg::Text { max: MODULE_ID_MAX })]),
    command("module_unload", &[("module", Arg::Text { max: MODULE_ID_MAX })]),
    // Realm hosting and joining (DU5, RF7; every build, behind the realms feature once RF7 adds it)
    command("realm_host", &[("kind", Arg::Name { names: REALM_KINDS })]),
    command("realm_join", &[("realm", Arg::Text { max: REALM_ID_MAX })]),
    command("realm_stop", &[]),
    command("realm_get", &[]),
    // History and revert (DH3b; server GUI): apply takes the dry run's plan, so only what was shown runs
    command("history_query", &[("filter", HISTORY_FILTER)]),
    command("history_revert_dry_run", &[("selection", REVERT_SELECTION)]),
    command("history_revert_apply", &[("plan", Arg::Text { max: CHANGE_ID_MAX })]),
    // Guarded actions waiting out their delay or a second operator (DH6; server GUI)
    command("guarded_list", &[]),
    command("guarded_approve", &[("id", Arg::Text { max: CHANGE_ID_MAX })]),
    command("guarded_cancel", &[("id", Arg::Text { max: CHANGE_ID_MAX })]),
    // Server GUI doctor, backup, config, MongoDB migration, invites and player administration (B3)
    command("server_doctor", &[]),
    command("server_backup", &[]),
    command("server_config_get", &[]),
    command("server_config_save", &[("config", SERVER_CONFIG_PATCH)]),
    command("server_migrate_mongo", &[]),
    command("server_confirm_fresh_start", &[]),
    command("server_invite_list", &[]),
    command("admin_action", &[("request", ADMIN_REQUEST)]),
    // Remote management (DD27, RM4; server GUI)
    command("remote_status", &[]),
    command("remote_init_controller", &[("request", REMOTE_INIT_REQUEST)]),
    command("remote_invite", &[("request", REMOTE_INVITE_REQUEST)]),
    command("remote_enroll", &[("request", REMOTE_ENROLL_REQUEST)]),
    command("remote_revoke", &[("node", Arg::Text { max: MODULE_ID_MAX })]),
    command("remote_leave", &[]),
    command("remote_exec", &[("request", REMOTE_EXEC_REQUEST)]),
];

/// Whether a string fits a limit counted as JavaScript counts it, so the page, the hook and the
/// backend agree on every edge.
pub fn fits(text: &str, max: usize) -> bool {
    text.len() <= max || text.encode_utf16().count() <= max
}

pub fn is_server_id(id: &str) -> bool {
    id.len() == SERVER_ID_LEN && id.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use ts_rs::TS;

    use super::super::types::{
        AdminActionKind, ExternalTarget, HostAction, HostFolder, HostServerMode, InstallMode, RealmKind,
    };
    use super::*;

    #[test]
    fn limits_count_like_javascript() {
        assert!(fits("ab", 2));
        assert!(!fits("abc", 2));
        // One code point outside the BMP is two UTF-16 units, four UTF-8 bytes
        assert!(fits("\u{1F600}", 2));
        assert!(!fits("\u{1F600}a", 2));
        assert!(fits("ää", 2));
    }

    #[test]
    fn server_ids_are_24_hex() {
        assert!(is_server_id("0123456789abcdef01234567"));
        assert!(!is_server_id("0123456789ABCDEF01234567"));
        assert!(!is_server_id("0123456789abcdef0123456"));
    }

    /// The command names `generate_handler!` lists in lib.rs, in order.
    fn served() -> Vec<&'static str> {
        let lib = include_str!("../lib.rs");
        lib.split_once("generate_handler![")
            .map_or("", |(_, rest)| rest)
            .split_once("])")
            .map_or("", |(list, _)| list)
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|item| item.trim_start_matches("commands::"))
            .collect()
    }

    #[test]
    fn every_command_is_served_and_nothing_else() {
        let listed: Vec<&str> = COMMANDS.iter().map(|command| command.name).collect();
        assert_eq!(served(), listed);
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = COMMANDS.iter().map(|command| command.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COMMANDS.len());
    }

    /// A name list is exactly the enum's names: each one parses, and the TypeScript union ts-rs writes
    /// for the enum has the same names in the same order.
    fn same_names<T: TS + DeserializeOwned>(names: &[&str]) {
        for name in names {
            assert!(
                serde_json::from_value::<T>(serde_json::Value::from(*name)).is_ok(),
                "{name}"
            );
        }
        let union = T::inline(&ts_rs::Config::new());
        let declared: Vec<String> = union
            .split('|')
            .map(|item| item.trim().trim_matches('"').to_owned())
            .collect();
        assert_eq!(declared, names, "{}", T::name(&ts_rs::Config::new()));
    }

    #[test]
    fn name_lists_are_the_enums() {
        same_names::<ExternalTarget>(EXTERNAL_TARGETS);
        same_names::<HostAction>(HOST_ACTIONS);
        same_names::<HostFolder>(HOST_FOLDERS);
        same_names::<HostServerMode>(SERVER_MODES);
        same_names::<InstallMode>(INSTALL_MODES);
        same_names::<RealmKind>(REALM_KINDS);
        same_names::<AdminActionKind>(ADMIN_ACTIONS);
    }

    fn listed(name: &str) -> bool {
        COMMANDS.iter().any(|command| command.name == name)
    }

    #[test]
    fn planned_and_server_gui_commands_are_listed() {
        for name in PLANNED.iter().chain(SERVER_GUI_COMMANDS).chain(REALM_COMMANDS) {
            assert!(listed(name), "{name}");
        }
    }

    #[test]
    fn each_command_names_its_build() {
        assert_eq!(build("host_get"), Build::Host);
        assert_eq!(build("module_load"), Build::ServerGui);
        assert_eq!(build("history_revert_apply"), Build::ServerGui);
        assert_eq!(build("epic_sign_in_start"), Build::Every);
        assert_eq!(build("realm_host"), Build::Realms);
        for command in COMMANDS {
            if build(command.name) == Build::ServerGui {
                assert!(!command.name.starts_with("host_"), "{}", command.name);
            }
        }
        assert!(serves("play"));
        assert_eq!(serves("module_list"), cfg!(feature = "server-gui"));
        assert_eq!(serves("realm_host"), cfg!(feature = "realms"));
    }
}
