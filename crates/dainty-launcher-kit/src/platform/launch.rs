// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Starting the game: its command line, its environment, the one game process, and the checks PLAY makes first
//! (spec §2.7.5).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use dainty_protocol::status::{MAX_CELL_FUSION_MINUTES, StatusEconomy};
use futures_util::future::BoxFuture;

use crate::bridge::types::RuntimeProfile;
use crate::core::game_language;
use crate::core::pins::{EXE_NAME, VC_RUNTIME_FILES};
use crate::platform::inject::{self, InjectDll};
use crate::platform::linux_runtime::PreparedLaunch;
use crate::platform::system;

/// What a masked `-AUTH_PASSWORD=` argument shows instead of the key.
pub const MASKED_PASSWORD: &str = "-AUTH_PASSWORD=<hidden>";
/// Arguments added for a windowed start.
pub const WINDOWED_ARGS: [&str; 3] = ["-windowed", "-ResX=1280", "-ResY=720"];
/// Variables with these (upper-cased) prefixes never reach the game from the launcher's own environment.
// REFORGED_: the names before the 2026-11-06 rename, which a runtime built before it still reads
pub const STRIPPED_PREFIXES: [&str; 5] = [
    "DAINTY_",
    "REFORGED_",
    "RUSTIC_ISLES_",
    "RUSTIC_RIFTS_",
    "MYSTICPARADOX_",
];
/// The variables naming the local backend: the runtime reads `RUSTIC_ISLES_BACKEND_URL` first, and one built
/// before the 2026-11-06 rename knows no `DAINTY_` name.
pub const BACKEND_URL_VARS: [&str; 2] = ["DAINTY_BACKEND_URL", "RUSTIC_ISLES_BACKEND_URL"];
/// The variable naming the runtime build (RT-8 dropped the `MYSTICPARADOX_` copy; the prefix is still stripped).
pub const PROFILE_VARS: [&str; 1] = ["RUSTIC_ISLES_RUNTIME_PROFILE"];
/// The runtime's names for the server's supply caps and fusion time (`rustic-isles-runtime-core`, R15).
pub const KEEP_SUPPLY_STACK_CAPS_VAR: &str = "DAINTY_KEEP_SUPPLY_STACK_CAPS";
/// See [`KEEP_SUPPLY_STACK_CAPS_VAR`].
pub const CELL_FUSION_MINUTES_VAR: &str = "DAINTY_CELL_FUSION_MINUTES";
/// The DirectX files the game needs in System32.
pub const DIRECTX_FILES: [&str; 3] = ["XINPUT1_3.dll", "X3DAudio1_7.dll", "XAPOFX1_5.dll"];
/// The exit code of a game whose console was closed or sent Ctrl+C: not a failure.
pub const CONSOLE_CLOSED_EXIT: u32 = 0xC000_013A;

const PASSWORD_PREFIX: &str = "-AUTH_PASSWORD=";
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: u64 = 1024 * 1024;

/// Why the game was not started.
#[derive(Debug)]
pub enum LaunchError {
    /// The account key is not plausible.
    InvalidKey,
    /// The user id is not 1..128 of `A-Z a-z 0-9 _ -`.
    InvalidUserId,
    /// The username is not 1..32 of `A-Z a-z 0-9 _`.
    InvalidUsername,
    /// The backend URL is not `http://127.0.0.1:<port>`.
    InvalidBackendUrl,
    /// A game started by this launcher is still running.
    AlreadyRunning,
    /// The process could not be started.
    Spawn(std::io::Error),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey => f.write_str("the account key is not valid"),
            Self::InvalidUserId => f.write_str("the user id is not valid"),
            Self::InvalidUsername => f.write_str("the username is not valid"),
            Self::InvalidBackendUrl => f.write_str("the backend URL is not a loopback HTTP URL"),
            Self::AlreadyRunning => f.write_str("the game is already running"),
            Self::Spawn(e) => write!(f, "the game could not be started: {e}"),
        }
    }
}

impl std::error::Error for LaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(e) => Some(e),
            _ => None,
        }
    }
}

/// What the game's command line is built from.
#[derive(Debug, Clone, Copy)]
pub struct LaunchArgs<'a> {
    /// The account key, sent as the exchange code.
    pub key: &'a str,
    /// The account's user id.
    pub user_id: &'a str,
    /// The account's username.
    pub username: &'a str,
    /// Start in a 1280×720 window.
    pub windowed: bool,
    /// The game's locale (§2.8.1); an unlisted one becomes `en-US`.
    pub locale: &'a str,
}

fn is_user_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn is_username(value: &str) -> bool {
    (1..=32).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The game's arguments, in the order the game's Epic login expects them.
pub fn launch_args(input: &LaunchArgs<'_>) -> Result<Vec<String>, LaunchError> {
    if !dainty_protocol::account::is_plausible_account_key(input.key) {
        return Err(LaunchError::InvalidKey);
    }
    if !is_user_id(input.user_id) {
        return Err(LaunchError::InvalidUserId);
    }
    if !is_username(input.username) {
        return Err(LaunchError::InvalidUsername);
    }
    let id = input.user_id;
    let mut args = vec![
        "-DisableEOS".to_owned(),
        "-EpicPortal".to_owned(),
        "-NoEAC".to_owned(),
        "-AUTH_TYPE=exchangecode".to_owned(),
        format!("-AUTH_LOGIN={id}"),
        format!("{PASSWORD_PREFIX}{}", input.key),
        "-epicapp=Archon".to_owned(),
        "-epicenv=Prod".to_owned(),
        format!("-epicusername={}", input.username),
        format!("-epicuserid={id}"),
        format!("-epicaccountid={id}"),
        "-epicsandboxid=jackal".to_owned(),
        "-epicdeploymentid=53565ba467df4edbb6f5a3d939a8b4f2".to_owned(),
        format!("-epiclocale={}", game_language::launch_locale(input.locale)),
    ];
    if input.windowed {
        args.extend(WINDOWED_ARGS.iter().map(|a| (*a).to_owned()));
    }
    Ok(args)
}

fn is_password_arg(arg: &str) -> bool {
    arg.get(..PASSWORD_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(PASSWORD_PREFIX))
}

/// `args` with the key hidden, for the log.
#[must_use]
pub fn mask_args<S: AsRef<str>>(args: &[S]) -> Vec<String> {
    args.iter()
        .map(|a| {
            let a = a.as_ref();
            if is_password_arg(a) {
                MASKED_PASSWORD.to_owned()
            } else {
                a.to_owned()
            }
        })
        .collect()
}

/// `<exe> <masked args>`, the command line as the log shows it.
#[must_use]
pub fn describe<S: AsRef<str>>(exe: &Path, args: &[S]) -> String {
    let mut line = exe.display().to_string();
    for arg in mask_args(args) {
        line.push(' ');
        line.push_str(&arg);
    }
    line
}

/// `url` is `http://127.0.0.1:<port>`, written as the relay prints it.
#[must_use]
pub fn is_backend_url(url: &str) -> bool {
    url.strip_prefix("http://127.0.0.1:").is_some_and(|port| {
        (1..=5).contains(&port.len()) && !port.starts_with('0') && port.bytes().all(|b| b.is_ascii_digit())
    })
}

/// The profile's name as the runtime reads it.
#[must_use]
pub const fn profile_name(profile: RuntimeProfile) -> &'static str {
    match profile {
        RuntimeProfile::Production => "production",
        RuntimeProfile::Development => "development",
    }
}

fn is_stripped(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    STRIPPED_PREFIXES.iter().any(|p| upper.starts_with(p))
}

/// The game's environment: `base` without the server's own variables, plus the backend URL and the profile.
///
/// Stripping by upper-cased name means a stale server setting in the shell, in any case, never reaches the game,
/// and Windows' case-insensitive names can't end up doubled.
pub fn launch_env<K, V>(
    base: impl IntoIterator<Item = (K, V)>,
    backend_url: &str,
    profile: RuntimeProfile,
) -> Result<BTreeMap<String, String>, LaunchError>
where
    K: Into<String>,
    V: Into<String>,
{
    if !is_backend_url(backend_url) {
        return Err(LaunchError::InvalidBackendUrl);
    }
    let mut env: BTreeMap<String, String> = base
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .filter(|(k, _)| !is_stripped(k))
        .collect();
    for var in BACKEND_URL_VARS {
        env.insert(var.to_owned(), backend_url.to_owned());
    }
    for var in PROFILE_VARS {
        env.insert(var.to_owned(), profile_name(profile).to_owned());
    }
    Ok(env)
}

/// Adds the server's economy settings (spec-runtime-r15 Q-1): only what it sent and the protocol accepted, so a
/// server sending none leaves the game on its defaults, the launcher's own copies being stripped already.
pub fn add_economy_env(env: &mut BTreeMap<String, String>, economy: Option<&StatusEconomy>) {
    let Some(economy) = economy else { return };
    if let Some(keep) = economy.keep_supply_stack_caps {
        env.insert(KEEP_SUPPLY_STACK_CAPS_VAR.to_owned(), keep.to_string());
    }
    if let Some(minutes) = economy.cell_fusion_minutes.filter(|&m| m <= MAX_CELL_FUSION_MINUTES) {
        env.insert(CELL_FUSION_MINUTES_VAR.to_owned(), minutes.to_string());
    }
}

/// A process ready to start: what runs, where, and with exactly which environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The game, or the Linux runner.
    pub program: PathBuf,
    /// Everything after the program: the runner's own arguments and the game when there is a runner.
    pub args: Vec<OsString>,
    /// The Win64 folder, which the game resolves its files from.
    pub working_dir: PathBuf,
    /// The whole environment; the process inherits nothing else.
    pub env: BTreeMap<String, String>,
    /// The DLLs loaded into the game before it runs, in order (RD9, 2026-11-12); empty under Proton.
    pub inject: Vec<InjectDll>,
}

impl LaunchPlan {
    /// The plan for `args` and `env` from [`launch_args`] and [`launch_env`], natively or through `runtime`.
    #[must_use]
    pub fn new(
        win64: &Path,
        runtime: Option<&PreparedLaunch>,
        args: &[String],
        mut env: BTreeMap<String, String>,
    ) -> Self {
        let exe = win64.join(EXE_NAME);
        let game_args = args.iter().map(OsString::from);
        let (program, args) = match runtime {
            None => (exe, game_args.collect()),
            Some(runtime) => {
                env.extend(runtime.env.iter().map(|(k, v)| (k.clone(), v.clone())));
                let mut all = runtime.args_before_exe.clone();
                all.push(exe.into_os_string());
                all.extend(game_args);
                (runtime.program.clone(), all)
            }
        };
        Self {
            program,
            args,
            working_dir: win64.to_path_buf(),
            env,
            inject: Vec::new(),
        }
    }

    /// Under Proton with the inject helper: the runner runs `helper` with `helper_args`
    /// ([`inject::helper_line`]), which starts the game and loads the DLLs into it inside Wine (RD9, 2026-11-12).
    #[must_use]
    pub fn through_helper(
        win64: &Path,
        runtime: &PreparedLaunch,
        helper: &Path,
        helper_args: Vec<OsString>,
        mut env: BTreeMap<String, String>,
    ) -> Self {
        env.extend(runtime.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        env.insert(inject::INJECTED_VAR.to_owned(), "1".to_owned());
        let mut args = runtime.args_before_exe.clone();
        args.push(helper.as_os_str().to_owned());
        args.extend(helper_args);
        Self {
            program: runtime.program.clone(),
            args,
            working_dir: win64.to_path_buf(),
            env,
            inject: Vec::new(),
        }
    }

    /// The plan with `dlls` loaded into the game before it runs, and any runtime loader beside it told to stand
    /// down ([`inject::INJECTED_VAR`]).
    #[must_use]
    pub fn with_inject(mut self, dlls: Vec<InjectDll>) -> Self {
        if !dlls.is_empty() {
            self.env.insert(inject::INJECTED_VAR.to_owned(), "1".to_owned());
        }
        self.inject = dlls;
        self
    }

    /// The whole command line with the key hidden, for the log.
    #[must_use]
    pub fn describe(&self) -> String {
        let args: Vec<String> = self.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        describe(&self.program, &args)
    }
}

/// What the game process's listener hears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameEvent {
    /// The game started, with its pid when the OS gave one.
    Started(Option<u32>),
    /// The game exited, with its code when it had one (a signal on Linux has none).
    Exited(Option<i32>),
}

/// A started game, as far as [`GameProcess`] needs it.
pub trait GameChild: Send {
    /// Its pid.
    fn id(&self) -> Option<u32>;
    /// Resolves to its exit code once it exits.
    fn wait(self: Box<Self>) -> BoxFuture<'static, std::io::Result<Option<i32>>>;
}

/// Starts the game; tests stand in their own.
pub trait Spawner: Send + Sync {
    /// Starts `plan`.
    fn spawn(&self, plan: &LaunchPlan) -> std::io::Result<Box<dyn GameChild>>;
}

/// The OS's spawner: standard streams null and the environment exactly the plan's.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsSpawner;

struct OsChild(tokio::process::Child);

impl GameChild for OsChild {
    fn id(&self) -> Option<u32> {
        self.0.id()
    }

    fn wait(mut self: Box<Self>) -> BoxFuture<'static, std::io::Result<Option<i32>>> {
        Box::pin(async move { self.0.wait().await.map(|status| status.code()) })
    }
}

impl Spawner for OsSpawner {
    fn spawn(&self, plan: &LaunchPlan) -> std::io::Result<Box<dyn GameChild>> {
        if !plan.inject.is_empty() && !inject::INJECTS {
            return Err(inject::InjectError::Unsupported.into());
        }
        // Not run_hidden: the game's console is its own, and the Game log reads it (§2.7.16).
        let mut command = tokio::process::Command::new(&plan.program);
        command
            .args(&plan.args)
            .current_dir(&plan.working_dir)
            .env_clear()
            .envs(&plan.env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        if !plan.inject.is_empty() {
            // The DLLs go in before the game's entry point, as the runtime loader's did
            const CREATE_SUSPENDED: u32 = 0x4;
            command.creation_flags(CREATE_SUSPENDED);
        }
        let mut child = command.spawn()?;
        if !plan.inject.is_empty()
            && let Err(error) = inject::load_and_resume(&child, &plan.inject)
        {
            // Never resumed: a game without its runtime must not reach the menu
            let _ = child.start_kill();
            return Err(error.into());
        }
        Ok(Box::new(OsChild(child)))
    }
}

#[derive(Debug, Default)]
struct Running {
    active: bool,
    pid: Option<u32>,
}

/// The one game this launcher runs at a time.
#[derive(Clone)]
pub struct GameProcess {
    spawner: Arc<dyn Spawner>,
    state: Arc<Mutex<Running>>,
}

impl Default for GameProcess {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for GameProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameProcess")
            .field("running", &self.is_running())
            .field("pid", &self.pid())
            .finish_non_exhaustive()
    }
}

impl GameProcess {
    /// Starts games with [`OsSpawner`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_spawner(Arc::new(OsSpawner))
    }

    /// Starts games with `spawner`.
    #[must_use]
    pub fn with_spawner(spawner: Arc<dyn Spawner>) -> Self {
        Self {
            spawner,
            state: Arc::new(Mutex::new(Running::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Running> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A game started here has not exited yet.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.lock().active
    }

    /// The running game's pid.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.lock().pid
    }

    /// Starts `plan` and watches it on the current tokio runtime; `listener` hears it start and exit.
    pub fn start(
        &self,
        plan: &LaunchPlan,
        listener: impl Fn(GameEvent) + Send + Sync + 'static,
    ) -> Result<Option<u32>, LaunchError> {
        let child = {
            let mut state = self.lock();
            if state.active {
                return Err(LaunchError::AlreadyRunning);
            }
            let child = self.spawner.spawn(plan).map_err(LaunchError::Spawn)?;
            state.active = true;
            state.pid = child.id();
            child
        };
        let pid = child.id();
        listener(GameEvent::Started(pid));
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let code = child.wait().await.ok().flatten();
            {
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                state.active = false;
                state.pid = None;
            }
            listener(GameEvent::Exited(code));
        });
        Ok(pid)
    }
}

/// An exit code as the launcher reports it: `0x` and eight upper-case hex digits.
#[must_use]
pub fn exit_code_hex(code: i32) -> String {
    format!("0x{:08X}", code.cast_unsigned())
}

/// The numeric lines of a process listing.
#[must_use]
pub fn parse_pids(output: &str) -> Vec<u32> {
    output.lines().filter_map(|l| l.trim().parse().ok()).collect()
}

/// `tasklist`'s CSV lists the game.
#[must_use]
pub fn tasklist_lists_game(output: &str) -> bool {
    output.to_ascii_lowercase().contains(&EXE_NAME.to_ascii_lowercase())
}

// Only ids leave PowerShell, so no command line (which holds the key) is ever read back.
const CLIENTS_QUERY: &str = "Get-CimInstance Win32_Process -Filter \"Name='Dauntless-Win64-Shipping.exe'\" \
     | Where-Object { $_.CommandLine -notmatch '(^|\\s)-server(\\s|$)' } \
     | ForEach-Object { $_.ProcessId }";

/// The pids of game clients running on this PC, from any launcher; a dedicated server (`-server`) is not one.
pub async fn running_clients() -> Vec<u32> {
    if cfg!(windows) {
        let mut list = system::run_hidden("tasklist");
        list.args(["/FI", &format!("IMAGENAME eq {EXE_NAME}"), "/FO", "CSV", "/NH"]);
        let listed = system::capture_hidden(list, LIST_TIMEOUT, OUTPUT_LIMIT).await;
        if !listed.as_deref().is_some_and(tasklist_lists_game) {
            return Vec::new();
        }
        let mut query = system::run_hidden("powershell");
        query.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            CLIENTS_QUERY,
        ]);
        system::capture_hidden(query, QUERY_TIMEOUT, OUTPUT_LIMIT)
            .await
            .map(|out| parse_pids(&out))
            .unwrap_or_default()
    } else if cfg!(target_os = "linux") {
        let mut pgrep = system::run_hidden("pgrep");
        pgrep.args(["-f", EXE_NAME]);
        system::capture_hidden(pgrep, LIST_TIMEOUT, OUTPUT_LIMIT)
            .await
            .map(|out| parse_pids(&out))
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// The system files the game needs that `system32` lacks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MissingFiles {
    /// From the Visual C++ redistributable.
    pub visual_cpp: Vec<&'static str>,
    /// From the DirectX end-user runtime.
    pub directx: Vec<&'static str>,
}

impl MissingFiles {
    /// Nothing is missing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.visual_cpp.is_empty() && self.directx.is_empty()
    }
}

/// The files missing from `system32`.
#[must_use]
pub fn missing_files_in(system32: &Path) -> MissingFiles {
    let missing = |names: &[&'static str]| -> Vec<&'static str> {
        names.iter().copied().filter(|n| !system32.join(n).is_file()).collect()
    };
    MissingFiles {
        visual_cpp: missing(&VC_RUNTIME_FILES),
        directx: missing(&DIRECTX_FILES),
    }
}

/// The files missing from this PC's System32; nothing elsewhere, where Wine or Proton supplies them.
#[must_use]
pub fn missing_files() -> MissingFiles {
    if cfg!(windows) {
        missing_files_in(&system::system_root().join("System32"))
    } else {
        MissingFiles::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const KEY: &str = "DLK_0123456789abcdefSECRETvalue";

    fn input() -> LaunchArgs<'static> {
        LaunchArgs {
            key: KEY,
            user_id: "user-01_ab",
            username: "Slayer_9",
            windowed: false,
            locale: "pt-BR",
        }
    }

    #[test]
    fn arguments_in_order() {
        assert_eq!(
            launch_args(&input()).unwrap(),
            [
                "-DisableEOS",
                "-EpicPortal",
                "-NoEAC",
                "-AUTH_TYPE=exchangecode",
                "-AUTH_LOGIN=user-01_ab",
                &format!("-AUTH_PASSWORD={KEY}"),
                "-epicapp=Archon",
                "-epicenv=Prod",
                "-epicusername=Slayer_9",
                "-epicuserid=user-01_ab",
                "-epicaccountid=user-01_ab",
                "-epicsandboxid=jackal",
                "-epicdeploymentid=53565ba467df4edbb6f5a3d939a8b4f2",
                "-epiclocale=pt-BR",
            ]
        );
    }

    #[test]
    fn windowed_appends_the_window() {
        let args = launch_args(&LaunchArgs {
            windowed: true,
            ..input()
        })
        .unwrap();
        assert_eq!(&args[14..], ["-windowed", "-ResX=1280", "-ResY=720"]);
        assert_eq!(args.len(), 17);
    }

    #[test]
    fn unknown_locale_is_english() {
        for locale in ["", "pt-PT", "auto", "PT-BR"] {
            let args = launch_args(&LaunchArgs { locale, ..input() }).unwrap();
            assert_eq!(args[13], "-epiclocale=en-US", "{locale:?}");
        }
    }

    #[test]
    fn refuses_bad_identity() {
        let bad_ids = ["", "a b", "a/b", "é", &"a".repeat(129)];
        for user_id in bad_ids {
            assert!(matches!(
                launch_args(&LaunchArgs { user_id, ..input() }),
                Err(LaunchError::InvalidUserId)
            ));
        }
        assert!(
            launch_args(&LaunchArgs {
                user_id: &"a".repeat(128),
                ..input()
            })
            .is_ok()
        );
        for username in ["", "a-b", "a b", &"a".repeat(33)] {
            assert!(matches!(
                launch_args(&LaunchArgs { username, ..input() }),
                Err(LaunchError::InvalidUsername)
            ));
        }
        for key in ["", "short", "DLK_has space", "DLK_with=equals"] {
            assert!(matches!(
                launch_args(&LaunchArgs { key, ..input() }),
                Err(LaunchError::InvalidKey)
            ));
        }
    }

    #[test]
    fn masking_hides_the_key_in_any_case() {
        let masked = mask_args(&[
            "-auth_password=abc",
            "-Auth_Password=",
            "-AUTH_PASSWORDX=1",
            "-AUTH_LOGIN=x",
        ]);
        assert_eq!(
            masked,
            [
                "-AUTH_PASSWORD=<hidden>",
                "-AUTH_PASSWORD=<hidden>",
                "-AUTH_PASSWORDX=1",
                "-AUTH_LOGIN=x"
            ]
        );
        assert_eq!(mask_args(&["-AUTH_PASS", "é", ""]), ["-AUTH_PASS", "é", ""]);
    }

    #[test]
    fn the_described_command_line_never_holds_the_key() {
        for windowed in [false, true] {
            let args = launch_args(&LaunchArgs { windowed, ..input() }).unwrap();
            let line = describe(Path::new("C:/Game/Dauntless-Win64-Shipping.exe"), &args);
            assert!(!line.contains(KEY), "{line}");
            assert!(!line.contains("SECRET"), "{line}");
            assert!(line.starts_with("C:/Game/Dauntless-Win64-Shipping.exe -DisableEOS -EpicPortal "));
            assert!(line.contains(" -AUTH_PASSWORD=<hidden> -epicapp=Archon "));
        }
        let args = launch_args(&input()).unwrap();
        let runner = PreparedLaunch {
            program: PathBuf::from("/usr/bin/proton"),
            args_before_exe: vec!["run".into()],
            env: BTreeMap::new(),
            kind: crate::platform::linux_runtime::RuntimeKind::Proton,
            prefix: PathBuf::from("/p"),
            label: "Proton".to_owned(),
            config_dir: PathBuf::from("/c"),
        };
        let plan = LaunchPlan::new(Path::new("/g/Win64"), Some(&runner), &args, BTreeMap::new());
        let line = plan.describe();
        assert!(!line.contains(KEY), "{line}");
        assert!(line.starts_with("/usr/bin/proton run "), "{line}");
    }

    #[test]
    fn backend_url_rule() {
        for ok in ["http://127.0.0.1:1", "http://127.0.0.1:61000", "http://127.0.0.1:99999"] {
            assert!(is_backend_url(ok), "{ok}");
        }
        for bad in [
            "http://127.0.0.1:0",
            "http://127.0.0.1:080",
            "http://127.0.0.1:",
            "http://127.0.0.1:100000",
            "http://127.0.0.1:80/",
            "https://127.0.0.1:80",
            "http://localhost:80",
            "http://127.0.0.2:80",
            "http://127.0.0.1:8a",
            " http://127.0.0.1:80",
        ] {
            assert!(!is_backend_url(bad), "{bad}");
        }
        assert!(matches!(
            launch_env(
                Vec::<(String, String)>::new(),
                "http://localhost:1",
                RuntimeProfile::Production
            ),
            Err(LaunchError::InvalidBackendUrl)
        ));
    }

    #[test]
    fn environment_strips_every_prefix_in_any_case() {
        let base = [
            ("PATH", "/bin"),
            ("SystemRoot", "C:\\Windows"),
            ("DAINTY_BACKEND_URL", "http://evil"),
            ("dainty_backend_url", "http://evil"),
            ("REFORGED_BACKEND_URL", "http://evil"),
            ("Reforged_Anything", "x"),
            ("RUSTIC_ISLES_RUNTIME_PROFILE", "development"),
            ("rustic_isles_x", "x"),
            ("RUSTIC_RIFTS_TOKEN", "x"),
            ("Rustic_Rifts_y", "x"),
            ("MYSTICPARADOX_RUNTIME_PROFILE", "development"),
            ("mysticparadox_z", "x"),
            ("REFORGEDX", "kept"),
            ("XREFORGED_Y", "kept"),
        ];
        let env = launch_env(base, "http://127.0.0.1:61000", RuntimeProfile::Production).unwrap();
        let expected: BTreeMap<String, String> = [
            ("PATH", "/bin"),
            ("DAINTY_BACKEND_URL", "http://127.0.0.1:61000"),
            ("REFORGEDX", "kept"),
            ("RUSTIC_ISLES_BACKEND_URL", "http://127.0.0.1:61000"),
            ("RUSTIC_ISLES_RUNTIME_PROFILE", "production"),
            ("SystemRoot", "C:\\Windows"),
            ("XREFORGED_Y", "kept"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        assert_eq!(env, expected);
        for name in env.keys() {
            let upper = name.to_ascii_uppercase();
            let same = env.keys().filter(|k| k.to_ascii_uppercase() == upper).count();
            assert_eq!(same, 1, "{name} doubled");
        }
    }

    #[test]
    fn economy_reaches_the_game_only_as_the_server_sent_it() {
        let base = [
            ("PATH", "/bin"),
            ("DAINTY_CELL_FUSION_MINUTES", "1"),
            ("dainty_keep_supply_stack_caps", "true"),
        ];
        let fresh = || launch_env(base, "http://127.0.0.1:5", RuntimeProfile::Production).unwrap();
        let economy = |keep, minutes, refused: &[&str]| StatusEconomy {
            keep_supply_stack_caps: keep,
            cell_fusion_minutes: minutes,
            refused: refused.iter().map(|r| (*r).to_owned()).collect(),
        };
        let pick = |env: &BTreeMap<String, String>| {
            env.iter()
                .filter(|(k, _)| k.to_ascii_uppercase().contains("SUPPLY") || k.to_ascii_uppercase().contains("FUSION"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>()
        };
        let owned = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<Vec<_>>()
        };

        let mut env = fresh();
        add_economy_env(&mut env, Some(&economy(Some(true), Some(45), &[])));
        assert_eq!(
            pick(&env),
            owned(&[(CELL_FUSION_MINUTES_VAR, "45"), (KEEP_SUPPLY_STACK_CAPS_VAR, "true")])
        );

        let mut env = fresh();
        add_economy_env(&mut env, Some(&economy(Some(false), Some(0), &[])));
        assert_eq!(
            pick(&env),
            owned(&[(CELL_FUSION_MINUTES_VAR, "0"), (KEEP_SUPPLY_STACK_CAPS_VAR, "false")])
        );

        // Absent, refused, or past the ceiling however it got here: nothing, not the launcher's own copies
        for absent in [
            None,
            Some(economy(None, None, &[])),
            Some(economy(None, None, &["keepSupplyStackCaps", "cellFusionMinutes"])),
            Some(economy(None, Some(MAX_CELL_FUSION_MINUTES + 1), &[])),
        ] {
            let mut env = fresh();
            add_economy_env(&mut env, absent.as_ref());
            assert_eq!(pick(&env), Vec::new(), "{absent:?}");
        }

        let mut env = fresh();
        add_economy_env(
            &mut env,
            Some(&economy(None, Some(MAX_CELL_FUSION_MINUTES), &["keepSupplyStackCaps"])),
        );
        assert_eq!(pick(&env), owned(&[(CELL_FUSION_MINUTES_VAR, "10080")]));
    }

    #[test]
    fn development_profile_reaches_the_runtime() {
        let env = launch_env([("A", "b")], "http://127.0.0.1:5", RuntimeProfile::Development).unwrap();
        assert_eq!(env["RUSTIC_ISLES_RUNTIME_PROFILE"], "development");
        assert!(!env.contains_key("MYSTICPARADOX_RUNTIME_PROFILE"));
    }

    #[test]
    fn native_and_runner_plans() {
        let env: BTreeMap<String, String> = [("A".to_owned(), "1".to_owned())].into();
        let args = vec!["-x".to_owned()];
        let win64 = Path::new("G/Archon/Binaries/Win64");
        let native = LaunchPlan::new(win64, None, &args, env.clone());
        assert_eq!(native.program, win64.join(EXE_NAME));
        assert_eq!(native.args, [OsString::from("-x")]);
        assert_eq!(native.working_dir, win64);
        assert_eq!(native.env, env);

        let runner = PreparedLaunch {
            program: PathBuf::from("/wine64"),
            args_before_exe: vec!["a".into(), "b".into()],
            env: [
                ("WINEPREFIX".to_owned(), "/p".to_owned()),
                ("A".to_owned(), "2".to_owned()),
            ]
            .into(),
            kind: crate::platform::linux_runtime::RuntimeKind::Wine,
            prefix: PathBuf::from("/p"),
            label: "Wine".to_owned(),
            config_dir: PathBuf::from("/c"),
        };
        let wine = LaunchPlan::new(win64, Some(&runner), &args, env);
        assert_eq!(wine.program, PathBuf::from("/wine64"));
        assert_eq!(
            wine.args,
            [
                OsString::from("a"),
                "b".into(),
                win64.join(EXE_NAME).into_os_string(),
                "-x".into()
            ]
        );
        assert_eq!(wine.working_dir, win64);
        assert_eq!(wine.env["WINEPREFIX"], "/p");
        assert_eq!(wine.env["A"], "2");

        // The inject helper in the game's place, the game and its arguments after the helper's own
        let dlls = [InjectDll {
            path: PathBuf::from("/g/Win64/rustic-isles-runtime.dll"),
            sha256: "a".repeat(64),
        }];
        let game_args = [OsString::from("-AUTH_PASSWORD=DLK_secret")];
        let line = inject::helper_line(Path::new("/g/Win64/Dauntless-Win64-Shipping.exe"), &dlls, &game_args).unwrap();
        let helped = LaunchPlan::through_helper(
            win64,
            &runner,
            Path::new("/res/dainty-inject.exe"),
            line,
            BTreeMap::new(),
        );
        assert_eq!(helped.program, PathBuf::from("/wine64"));
        assert_eq!(
            helped.args[..3],
            [OsString::from("a"), "b".into(), "/res/dainty-inject.exe".into()]
        );
        assert_eq!(helped.args[3], "--dll");
        assert_eq!(helped.args.last().unwrap(), "-AUTH_PASSWORD=DLK_secret");
        assert!(helped.inject.is_empty(), "Wine loads them, not the launcher");
        assert_eq!(helped.env["WINEPREFIX"], "/p");
        assert!(!helped.describe().contains("DLK_secret"));
    }

    struct FakeChild {
        exit: tokio::sync::oneshot::Receiver<Option<i32>>,
    }

    impl GameChild for FakeChild {
        fn id(&self) -> Option<u32> {
            Some(4242)
        }

        fn wait(self: Box<Self>) -> BoxFuture<'static, std::io::Result<Option<i32>>> {
            Box::pin(async move { Ok(self.exit.await.ok().flatten()) })
        }
    }

    struct FakeSpawner {
        spawned: AtomicUsize,
        exits: Mutex<Vec<tokio::sync::oneshot::Receiver<Option<i32>>>>,
        fail: bool,
    }

    impl Spawner for FakeSpawner {
        fn spawn(&self, _plan: &LaunchPlan) -> std::io::Result<Box<dyn GameChild>> {
            if self.fail {
                return Err(std::io::Error::other("no"));
            }
            self.spawned.fetch_add(1, Ordering::SeqCst);
            let exit = self.exits.lock().unwrap().remove(0);
            Ok(Box::new(FakeChild { exit }))
        }
    }

    fn plan() -> LaunchPlan {
        LaunchPlan::new(Path::new("w"), None, &[], BTreeMap::new())
    }

    #[tokio::test]
    async fn one_game_at_a_time_and_the_exit_is_heard() {
        let (tx1, rx1) = tokio::sync::oneshot::channel();
        let (tx2, rx2) = tokio::sync::oneshot::channel();
        let spawner = Arc::new(FakeSpawner {
            spawned: AtomicUsize::new(0),
            exits: Mutex::new(vec![rx1, rx2]),
            fail: false,
        });
        let game = GameProcess::with_spawner(spawner.clone());
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let listener = move |e| {
            let _ = events_tx.send(e);
        };
        assert_eq!(game.start(&plan(), listener.clone()).unwrap(), Some(4242));
        assert_eq!(events.recv().await, Some(GameEvent::Started(Some(4242))));
        assert!(game.is_running());
        assert_eq!(game.pid(), Some(4242));
        assert!(matches!(
            game.start(&plan(), listener.clone()),
            Err(LaunchError::AlreadyRunning)
        ));
        assert_eq!(spawner.spawned.load(Ordering::SeqCst), 1);

        tx1.send(Some(CONSOLE_CLOSED_EXIT.cast_signed())).unwrap();
        assert_eq!(events.recv().await, Some(GameEvent::Exited(Some(-1_073_741_510))));
        assert!(!game.is_running());
        assert_eq!(game.pid(), None);

        game.start(&plan(), listener).unwrap();
        assert_eq!(events.recv().await, Some(GameEvent::Started(Some(4242))));
        tx2.send(Some(1)).unwrap();
        assert_eq!(events.recv().await, Some(GameEvent::Exited(Some(1))));
    }

    #[tokio::test]
    async fn a_failed_start_leaves_nothing_running() {
        let game = GameProcess::with_spawner(Arc::new(FakeSpawner {
            spawned: AtomicUsize::new(0),
            exits: Mutex::new(Vec::new()),
            fail: true,
        }));
        assert!(matches!(game.start(&plan(), |_| {}), Err(LaunchError::Spawn(_))));
        assert!(!game.is_running());
    }

    #[test]
    fn exit_codes_as_hex() {
        assert_eq!(exit_code_hex(CONSOLE_CLOSED_EXIT.cast_signed()), "0xC000013A");
        assert_eq!(exit_code_hex(1), "0x00000001");
        assert_eq!(exit_code_hex(-1), "0xFFFFFFFF");
    }

    #[test]
    fn process_listings() {
        assert_eq!(parse_pids("12\n x \n 34 \r\n\n-5\n"), [12, 34]);
        assert!(tasklist_lists_game(
            "\"dauntless-win64-shipping.exe\",\"1234\",\"Console\""
        ));
        assert!(!tasklist_lists_game(
            "INFO: No tasks are running which match the specified criteria."
        ));
    }

    #[test]
    fn missing_files_lists_what_system32_lacks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MSVCP140.dll"), b"").unwrap();
        std::fs::write(dir.path().join("X3DAudio1_7.dll"), b"").unwrap();
        let missing = missing_files_in(dir.path());
        assert_eq!(missing.visual_cpp, ["VCRUNTIME140_1.dll"]);
        assert_eq!(missing.directx, ["XINPUT1_3.dll", "XAPOFX1_5.dll"]);
        assert!(!missing.is_empty());
    }

    #[tokio::test]
    #[cfg_attr(not(target_os = "linux"), ignore = "pgrep is Linux's")]
    async fn running_clients_finds_none_here() {
        assert!(running_clients().await.is_empty());
    }

    /// A plan that runs `cmd /c exit 7` with `inject` loaded first.
    #[cfg(windows)]
    fn cmd_plan(inject: Vec<InjectDll>) -> LaunchPlan {
        let system32 = system::system_root().join("System32");
        let env: BTreeMap<String, String> = std::env::vars().collect();
        LaunchPlan {
            program: system32.join("cmd.exe"),
            args: ["/c", "exit", "7"].iter().map(OsString::from).collect(),
            working_dir: system32,
            env,
            inject,
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_dll_is_loaded_before_the_process_runs_then_it_resumes() {
        let dll = system::system_root().join("System32").join("dbghelp.dll");
        let plan = cmd_plan(vec![InjectDll::as_found(&dll).unwrap()]);
        let child = OsSpawner.spawn(&plan).unwrap();
        assert_eq!(child.wait().await.unwrap(), Some(7));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_dll_that_is_not_the_checked_one_is_never_loaded() {
        let dll = system::system_root().join("System32").join("dbghelp.dll");
        let changed = InjectDll {
            path: dll,
            sha256: "0".repeat(64),
        };
        let refused = OsSpawner.spawn(&cmd_plan(vec![changed])).err().unwrap();
        assert!(
            refused.to_string().contains("not the file that was checked"),
            "{refused}"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn nothing_is_injected_off_windows() {
        let mut plan = plan();
        plan.inject.push(InjectDll {
            path: PathBuf::from("/x.dll"),
            sha256: String::new(),
        });
        let refused = OsSpawner.spawn(&plan).err().unwrap();
        assert_eq!(refused.kind(), std::io::ErrorKind::Unsupported);
    }
}
