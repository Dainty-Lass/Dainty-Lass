//! The files generated from the bridge (rows L6, L8, L9; RA2), kept in the repository and checked here:
//!
//! - the TypeScript declarations of every type in `types.rs` and the command names, for dainty-lass-ui's
//!   bridge package (`ui/packages/bridge/src/generated/types.ts`, DD11);
//! - `isolation/commands.js`: the command list with each argument's shape, for the isolation hook;
//! - `src-tauri/permissions/app-commands.toml`: the permission set that grants exactly those commands;
//! - `src-tauri/gen/strings.json`: dainty-lass-ui's language catalogs (`ui/packages/i18n/catalogs`, DU2),
//!   from which build.rs makes `Language` and the backend's own text.
//!
//! The test fails when a file is stale; `RR_UPDATE_GENERATED=1 cargo test -p launcher generated`
//! rewrites them.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use protocol::invite::ServerMode;
use protocol::status::{
    GameDataKind, InstanceKind, PlayerWhere, RegistrationMode, ServerStatus, StatusGame, StatusInstance, StatusPlayer,
};
use ts_rs::{Config, TS};

use super::spec::{Arg, COMMANDS, SETTINGS_KEYS};
use super::types::*;

const HEADER: &str = "Generated from the launcher backend (launcher/src-tauri/src/bridge); do not edit.\n\
Regenerate with: RR_UPDATE_GENERATED=1 cargo test -p launcher generated";

fn declarations() -> String {
    let config = Config::new().with_large_int("number");
    let mut out = String::new();
    for line in HEADER.lines() {
        let _ = writeln!(out, "// {line}");
    }
    out.push('\n');
    let mut declare = |text: String| {
        out.push_str("export ");
        out.push_str(&text);
        out.push('\n');
    };
    macro_rules! each {
        ($($ty:ty),* $(,)?) => { $( declare(<$ty as TS>::decl(&config)); )* };
    }
    each!(
        ServerMode,
        PlayerWhere,
        InstanceKind,
        RegistrationMode,
        StatusPlayer,
        StatusInstance,
        GameDataKind,
        StatusGame,
        ServerStatus,
        Language,
        GameLanguage,
        RuntimeProfile,
        Phase,
        ConnectProblem,
        GameProblem,
        ServerGame,
        Game,
        RuntimeState,
        ErrorCode,
        NoticeCode,
        LauncherError,
        TaskKind,
        TaskProgress,
        ServerInfo,
        SavedServerInfo,
        GameSource,
        DetectedGame,
        GameInfo,
        RevivedInfo,
        InstallMode,
        RuntimeFileInfo,
        RuntimeInfo,
        Settings,
        SnapshotConnect,
        SnapshotAccount,
        SnapshotRunning,
        SnapshotApp,
        Snapshot,
        NewsItem,
        Background,
        Branding,
        GameDataState,
        HostUpstream,
        HostSdk,
        HostDump,
        HostBuilds,
        HostToolchain,
        HostGameData,
        HostExports,
        HostServerMode,
        HostServerConfig,
        HostKit,
        HostMultipliers,
        HostUpdateState,
        HostUpdateView,
        RequirementId,
        RequirementState,
        Requirement,
        HostServerState,
        HostInvite,
        HostServerView,
        HostServerComponent,
        HostComponentPhase,
        HostJobKind,
        HostJobState,
        HostJob,
        ExternalTarget,
        HostAction,
        HostFolder,
        HostSetupRequest,
        RealmKind,
        RegionProblem,
        RegionRtt,
        GameVersionInstall,
        GameVersionOrigin,
        EpicManifestState,
        EpicVersionInfo,
        GameVersionEntry,
        EpicView,
        InstallState,
        InstallInfo,
        GuardedState,
        GuardedMode,
        GuardedActionInfo,
        ModuleRole,
        ModuleOrigin,
        ModuleRunState,
        ModulePortView,
        ModuleArgView,
        ModuleCommandView,
        ModuleSettingValue,
        ModuleSettingView,
        ModuleSectionView,
        ModulePatchSetView,
        ModulePaletteEntry,
        ModuleThemeView,
        ModuleCapabilitiesView,
        ModuleView,
        HistoryEntryView,
        HistoryPageView,
        RevertRowView,
        RevertConflictView,
        RevertPlanView,
        RevertedInView,
        RevertDoneView,
        RealmState,
        RealmSessionView,
        RealmView,
        DoctorLevel,
        DoctorCheckView,
        DoctorReportView,
        MongoMigrationState,
        MongoMigrationView,
        ServerConfigView,
        ServerInviteEntry,
        AdminActionKind,
        AdminPlayerView,
        AdminResultView,
        ManagedInstallView,
        RemoteStatusView,
        RemoteInviteView,
        RemoteExecView,
    );
    // The types serde writes by hand (types.rs)
    declare("type GraphicsPreset = -1 | 0 | 1 | 2 | 3 | 4;".into());
    declare("type ActionResult = { ok: true } | { ok: false; error: LauncherError };".into());
    declare(
        "type InviteCheck = { ok: true; certificateChanged: boolean; previousFingerprint: string | null; movedFrom: string | null } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare("type RegisterOutcome = { ok: true; username: string } | { ok: false; error: LauncherError };".into());
    declare(
        "type RegionPing = { ok: true; regions: Array<RegionRtt>; reported: boolean } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare(
        "type EpicLibraryCheck = { ok: true; owned: boolean; checkedAt: string } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare(
        "type GameVersionList = { ok: true; versions: Array<GameVersionEntry> } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare("type InstallOutcome = { ok: true; install: InstallInfo } | { ok: false; error: LauncherError };".into());
    declare(
        "type GuardedList = { ok: true; actions: Array<GuardedActionInfo> } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare(
        "type GuardedOutcome = { ok: true; action: GuardedActionInfo } | { ok: false; error: LauncherError };".into(),
    );
    declare(
        "type ModuleListOutcome = { ok: true; modules: Array<ModuleView> } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare("type ModuleOutcome = { ok: true; module: ModuleView } | { ok: false; error: LauncherError };".into());
    declare(
        "type ModuleCommandOutcome = { ok: true; text: string; data: string | null } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare("type HistoryOutcome = { ok: true; page: HistoryPageView } | { ok: false; error: LauncherError };".into());
    declare(
        "type RevertPlanOutcome = { ok: true; plan: RevertPlanView } | { ok: false; error: LauncherError };".into(),
    );
    declare(
        "type RevertApplyOutcome = { ok: true; done: RevertDoneView } | { ok: false; error: LauncherError };".into(),
    );
    declare("type RealmOutcome = { ok: true; realm: RealmView } | { ok: false; error: LauncherError };".into());
    declare(
        "type ServerDoctorOutcome = { ok: true; report: DoctorReportView } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare(
        "type ServerConfigOutcome = { ok: true; config: ServerConfigView } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare(
        "type ServerInviteListOutcome = { ok: true; invites: Array<ServerInviteEntry> } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare("type AdminOutcome = { ok: true; result: AdminResultView } | { ok: false; error: LauncherError };".into());
    declare(
        "type RemoteStatusOutcome = { ok: true; status: RemoteStatusView } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare(
        "type RemoteInviteOutcome = { ok: true; invite: RemoteInviteView } | { ok: false; error: LauncherError };"
            .into(),
    );
    declare(
        "type RemoteExecOutcome = { ok: true; result: RemoteExecView } | { ok: false; error: LauncherError };".into(),
    );
    let names: Vec<String> = COMMANDS.iter().map(|command| format!("\"{}\"", command.name)).collect();
    declare(format!("type CommandName = {};", names.join(" | ")));
    out
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// A number as JavaScript writes it: `1`, not `1.0`.
fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

/// One argument's shape for the hook, written by hand so the order is the list's: a serde_json map's
/// order depends on whether another crate in the build turns on its `preserve_order` feature.
fn arg_js(arg: &Arg) -> String {
    match arg {
        Arg::Text { max } => format!("{{ kind: \"text\", max: {max} }}"),
        Arg::OptionalText { max } => format!("{{ kind: \"optionalText\", max: {max} }}"),
        Arg::Flag => "{ kind: \"flag\" }".into(),
        Arg::Index { max } => format!("{{ kind: \"index\", max: {max} }}"),
        Arg::Name { names } => {
            let names: Vec<String> = names.iter().map(|name| quote(name)).collect();
            format!("{{ kind: \"name\", names: [{}] }}", names.join(", "))
        }
        Arg::Number { min, max } => format!("{{ kind: \"number\", min: {}, max: {} }}", number(*min), number(*max)),
        Arg::Object { fields, nullable } => format!(
            "{{ kind: \"object\", nullable: {nullable}, fields: {} }}",
            args_js(fields)
        ),
        Arg::SettingsPatch => "{ kind: \"settingsPatch\" }".into(),
        Arg::Channel => "{ kind: \"channel\" }".into(),
    }
}

fn args_js(args: &[(&str, Arg)]) -> String {
    if args.is_empty() {
        return "{}".into();
    }
    let args: Vec<String> = args
        .iter()
        .map(|(name, arg)| format!("{}: {}", quote(name), arg_js(arg)))
        .collect();
    format!("{{ {} }}", args.join(", "))
}

fn hook_commands() -> String {
    let mut out = String::new();
    for line in HEADER.lines() {
        let _ = writeln!(out, "// {line}");
    }
    out.push_str("window.__RR_COMMANDS__ = {\n");
    for command in COMMANDS {
        let _ = writeln!(out, "  {}: {},", quote(command.name), args_js(command.args));
    }
    out.push_str("};\n");
    let keys: Vec<String> = SETTINGS_KEYS.iter().map(|key| quote(key)).collect();
    let _ = writeln!(out, "window.__RR_SETTINGS_KEYS__ = [{}];", keys.join(", "));
    out
}

fn permission_set() -> String {
    let mut out = String::new();
    for line in HEADER.lines() {
        let _ = writeln!(out, "# {line}");
    }
    out.push_str("\n[[set]]\nidentifier = \"app-commands\"\ndescription = \"Every command in src/bridge/spec.rs, and nothing else.\"\npermissions = [\n");
    for command in COMMANDS {
        let _ = writeln!(out, "  \"allow-{}\",", command.name.replace('_', "-"));
    }
    out.push_str("]\n");
    out
}

fn launcher_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The dainty-lass-ui submodule (DD11)
fn ui_dir() -> PathBuf {
    launcher_dir().join("../ui")
}

/// Every catalog in `ui/packages/i18n/catalogs`, `en` first (the fallback and `Language::ALL[0]`), each
/// table sorted by key for the backend's binary search.
fn strings_export() -> String {
    let dir = ui_dir().join("packages/i18n/catalogs");
    let mut languages: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let code = path.file_name()?.to_str()?.strip_suffix(".json")?.to_owned();
            Some((code, path))
        })
        .collect();
    languages.sort_by(|a, b| (a.0 != "en", &a.0).cmp(&(b.0 != "en", &b.0)));
    let mut out = serde_json::Map::new();
    for (code, path) in languages {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let table: BTreeMap<String, String> =
            serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let table = table.into_iter().map(|(key, text)| (key, text.into())).collect();
        out.insert(code, serde_json::Value::Object(table));
    }
    serde_json::to_string_pretty(&out).unwrap_or_default() + "\n"
}

#[test]
fn generated_files_are_current() {
    // Refused rather than skipped, and never written into an empty submodule folder
    assert!(
        ui_dir().join("package.json").is_file(),
        "{} is not checked out: run `git submodule update --init ui`",
        ui_dir().display()
    );
    let files = [
        (ui_dir().join("packages/bridge/src/generated/types.ts"), declarations()),
        (launcher_dir().join("isolation/commands.js"), hook_commands()),
        (
            launcher_dir().join("src-tauri/permissions/app-commands.toml"),
            permission_set(),
        ),
        (launcher_dir().join("src-tauri/gen/strings.json"), strings_export()),
    ];
    let update = std::env::var("RR_UPDATE_GENERATED").is_ok_and(|value| value == "1");
    let mut stale = Vec::new();
    for (path, content) in files {
        let current = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        if current == content {
            continue;
        }
        if update {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).unwrap();
            }
            std::fs::write(&path, content).unwrap();
        } else {
            stale.push(path.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "stale generated files (run RR_UPDATE_GENERATED=1 cargo test -p launcher generated): {stale:?}"
    );
}

#[test]
fn numbers_read_as_javascript_writes_them() {
    assert_eq!(number(1.0), "1");
    assert_eq!(number(100.0), "100");
    assert_eq!(number(1.5), "1.5");
}

#[test]
fn generated_output_is_feature_independent() {
    use super::spec::{REALM_COMMANDS, SERVER_GUI_COMMANDS};

    // Neither generate.rs nor spec.rs's command declarations may gate generated types or commands on Cargo features.
    let gen_src = include_str!("generate.rs");
    let before_tests = gen_src.split("#[test]").next().unwrap_or(gen_src);
    assert!(
        !before_tests.contains("cfg(feature") && !before_tests.contains("cfg!(feature"),
        "generate.rs must not branch on Cargo features"
    );

    let ts = declarations();
    let js = hook_commands();
    let perms = permission_set();

    for cmd in SERVER_GUI_COMMANDS.iter().chain(REALM_COMMANDS) {
        assert!(ts.contains(&format!("\"{cmd}\"")), "types.ts missing command {cmd}");
        assert!(
            js.contains(&format!("\"{cmd}\":")),
            "isolation/commands.js missing command {cmd}"
        );
        assert!(
            perms.contains(&format!("\"allow-{}\"", cmd.replace('_', "-"))),
            "permissions/app-commands.toml missing command {cmd}"
        );
    }

    for ty in [
        "ModuleView",
        "HistoryEntryView",
        "RevertPlanView",
        "GuardedActionInfo",
        "RealmView",
        "RevivedInfo",
        "DoctorReportView",
        "MongoMigrationView",
        "ServerConfigView",
        "ServerInviteEntry",
        "AdminResultView",
        "ManagedInstallView",
        "RemoteStatusView",
        "RemoteInviteView",
        "RemoteExecView",
    ] {
        assert!(
            ts.contains(&format!("export type {ty} =")),
            "types.ts missing type {ty}"
        );
    }
}
