// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The `dainty-host` command line, parsed with clap. Doc comments here are the commands' `--help` text.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Runs, sets up and looks after a Dainty Lass server.
#[derive(Debug, Clone, Parser)]
#[command(name = "dainty-host", disable_version_flag = true)]
pub struct Cli {
    /// What to run.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Parses `itr` (argv, program name first); prints help or the error and exits on a bad command line.
    #[must_use]
    pub fn parse_from<I, T>(itr: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        <Self as Parser>::parse_from(itr)
    }
}

/// A `dainty-host` command.
#[derive(Debug, Clone, Subcommand)]
#[allow(
    clippy::large_enum_variant,
    reason = "built once per run; boxing the arguments would only clutter clap's derive"
)]
pub enum Command {
    /// Writes settings.toml, secrets.env and the services' env files, and installs what the server runs.
    Setup(SetupArgs),
    /// Runs the server in the background without a console.
    Start,
    /// Runs the server with its console on this terminal.
    Serve(ServeArgs),
    /// Makes an invite link for the launcher, or lists invite codes.
    Invite(InviteArgs),
    /// Checks the tools, configuration, game data and running services.
    Doctor(DoctorArgs),
    /// Generates, seeds, exports or imports the game data.
    Gamedata(GamedataArgs),
    /// Publishes the game runtime's DLLs for the game servers.
    PublishRuntime(PublishRuntimeArgs),
    /// Installs Dauntless from the host's Epic library for the game servers.
    InstallGame(InstallGameArgs),
    /// Creates, shows or applies the checkout's game-data preset.
    Preset(PresetArgs),
    /// Sets the XP and reward multipliers of a private test server.
    Multipliers(MultipliersArgs),
    /// Updates the server to a newer release.
    Update(UpdateArgs),
    /// Backs up the server's database into `.workspace/backups/`.
    Backup,
    /// Migrates, verifies or exports player data, or replays captured traffic.
    Db(DbArgs),
    /// Sets up node certificates and checks node links between servers.
    Network(NetworkArgs),
    /// Manages installs from a controller over node links.
    Remote(RemoteArgs),
    /// Shows the change log (DD23).
    History(HistoryArgs),
    /// Undoes logged changes, after a dry run that shows what would change.
    Revert(RevertArgs),
    /// Imports an old install with dainty-legacy: a Revived 1.4.4 server or offline save, or a Reforged Node kit
    Legacy(LegacyArgs),
    /// Runs the gateway with the command line of its own binary.
    #[command(hide = true, disable_help_flag = true)]
    Gateway {
        /// The service's own arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Runs the allowlist with the command line of its own binary.
    #[command(hide = true, disable_help_flag = true)]
    Allowlist {
        /// The service's own arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Runs the content server with the command line of its own binary.
    #[command(hide = true, disable_help_flag = true)]
    Content {
        /// The service's own arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Runs the admin service with the command line of its own binary.
    #[command(hide = true, disable_help_flag = true)]
    Admin {
        /// The service's own arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Runs the backend with the command line of its own binary.
    #[command(hide = true, disable_help_flag = true)]
    Backend {
        /// The service's own arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Runs the Director with the command line of its own binary.
    #[command(hide = true, disable_help_flag = true)]
    Director {
        /// The service's own arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Runs the remote management agent with the command line of its own binary.
    #[command(name = "remote-agent", hide = true, disable_help_flag = true)]
    RemoteAgent {
        /// The service's own arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Runs a game server under Wine or Proton (Linux).
    #[command(hide = true, disable_help_flag = true)]
    Gameserver(GameserverArgs),
}

impl Command {
    /// The command's name as typed: the service name a hidden service command runs.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Setup(_) => "setup",
            Self::Start => "start",
            Self::Serve(_) => "serve",
            Self::Invite(_) => "invite",
            Self::Doctor(_) => "doctor",
            Self::Gamedata(_) => "gamedata",
            Self::PublishRuntime(_) => "publish-runtime",
            Self::InstallGame(_) => "install-game",
            Self::Preset(_) => "preset",
            Self::Multipliers(_) => "multipliers",
            Self::Update(_) => "update",
            Self::Backup => "backup",
            Self::Db(_) => "db",
            Self::Network(_) => "network",
            Self::Remote(_) => "remote",
            Self::History(_) => "history",
            Self::Revert(_) => "revert",
            Self::Legacy(_) => "legacy",
            Self::Gateway { .. } => "gateway",
            Self::Allowlist { .. } => "allowlist",
            Self::Content { .. } => "content",
            Self::Admin { .. } => "admin",
            Self::Backend { .. } => "backend",
            Self::Director { .. } => "director",
            Self::RemoteAgent { .. } => "remote-agent",
            Self::Gameserver(_) => "gameserver",
        }
    }
}

/// Options of `setup`; each one left out keeps settings.toml's value, or the default on a first setup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct SetupArgs {
    /// The kind of server: private (friends over Tailscale) or public (a server on the internet).
    #[arg(long)]
    pub mode: Option<String>,
    /// The address players reach: this PC's Tailscale address or .ts.net name, or the public IPv4 or domain name.
    #[arg(long)]
    pub host: Option<String>,
    /// The IPv4 address the game servers announce, when the host is a DNS name.
    #[arg(long = "game-address")]
    pub game_address: Option<String>,
    /// The server's name, as the launcher shows it.
    #[arg(long)]
    pub name: Option<String>,
    /// The Dauntless 1.14.7 folder the game servers run from.
    #[arg(long)]
    pub game: Option<String>,
    /// Who may register: INVITECODE, OPEN or NONE.
    #[arg(long)]
    pub registration: Option<String>,
    /// A Tailscale share link from login.tailscale.com, for a private server's invites.
    #[arg(long)]
    pub share: Option<String>,
    /// The MongoDB connection string to migrate pre-cutover player data from.
    #[arg(
        long = "mongo-uri",
        value_name = "uri",
        help = "MongoDB connection string to migrate pre-cutover player data from"
    )]
    pub mongo_uri: Option<String>,
    /// Has the server start its own mongod.
    #[arg(long = "manage-mongo", hide = !cfg!(feature = "paradox"))]
    pub manage_mongo: bool,
    /// Uses a MongoDB the host runs instead of starting one.
    #[arg(long = "no-manage-mongo", hide = !cfg!(feature = "paradox"))]
    pub no_manage_mongo: bool,
    /// The mongod executable to start; found on PATH or under Program Files when not given.
    #[arg(long, hide = !cfg!(feature = "paradox"))]
    pub mongod: Option<String>,
    /// Downloads and patches upstream Mystic Paradox again.
    #[arg(
        long = "force-upstream",
        hide = !cfg!(feature = "paradox"),
        help = "Download and patch upstream Mystic Paradox again"
    )]
    pub force_upstream: bool,
    /// The most hunts that run at once, 1 to 10.
    #[arg(long = "max-hunts")]
    pub max_hunts: Option<String>,
    /// The XP multiplier of a private test server.
    #[arg(long = "xp-multiplier", alias = "xp")]
    pub xp_multiplier: Option<String>,
    /// The reward multiplier of a private test server.
    #[arg(long = "reward-multiplier", alias = "rewards", alias = "rewards-multiplier")]
    pub reward_multiplier: Option<String>,
    /// Turns the test multipliers off.
    #[arg(long = "no-multipliers", alias = "off")]
    pub no_multipliers: bool,
    /// The TCP port a public server's gateway listens on (443 when not given).
    #[arg(long = "public-port")]
    pub public_port: Option<String>,
    /// The game servers' frame cap: 0 for none, or 10 to 240.
    #[arg(long = "server-fps")]
    pub server_fps: Option<String>,
    /// The frame cap after 15 s without players: 0 for the same as --server-fps, or 1 to 240.
    #[arg(long = "idle-fps")]
    pub idle_fps: Option<String>,
    /// Starts the Training Grounds when a player asks for them.
    #[arg(long = "training-on-demand")]
    pub training_on_demand: bool,
    /// Starts the Training Grounds when the server starts.
    #[arg(long = "training-at-boot")]
    pub training_at_boot: bool,
    /// Minutes without players before an on-demand Training Grounds stops: 0 keeps it running, or 1 to 1440.
    #[arg(long = "training-idle-minutes")]
    pub training_idle_minutes: Option<String>,
    /// Keeps the game servers' full logs.
    #[arg(long = "verbose-game-logs")]
    pub verbose_game_logs: bool,
    /// Keeps the game servers' logs short.
    #[arg(long = "quiet-game-logs")]
    pub quiet_game_logs: bool,
    /// Starts a new hunt only when enough memory is free.
    #[arg(long = "memory-guard")]
    pub memory_guard: bool,
    /// Starts hunts whatever memory is free.
    #[arg(long = "no-memory-guard")]
    pub no_memory_guard: bool,
    /// Memory to keep free for the system, in MB.
    #[arg(long = "min-free-ram")]
    pub min_free_ram: Option<String>,
    /// Memory one hunt needs, in MB (at least 256).
    #[arg(long = "hunt-ram")]
    pub hunt_ram: Option<String>,
    /// The Wine executable, a path or a name on PATH (Linux); empty to find it.
    #[arg(long)]
    pub wine: Option<String>,
    /// The Proton executable to run the game servers with instead of Wine (Linux).
    #[arg(long)]
    pub proton: Option<String>,
    /// Runs the game servers with Wine, not Proton (Linux).
    #[arg(long = "no-proton")]
    pub no_proton: bool,
    /// Runs the game servers under xvfb-run when there is no display (Linux).
    #[arg(long)]
    pub xvfb: bool,
    /// Runs the game servers without xvfb-run (Linux).
    #[arg(long = "no-xvfb")]
    pub no_xvfb: bool,
    /// The network interface the firewall script guards, like eth0 (Linux).
    #[arg(long = "firewall-interface")]
    pub firewall_interface: Option<String>,
    /// Has the firewall script guard every interface (Linux).
    #[arg(long = "no-firewall-interface")]
    pub no_firewall_interface: bool,
    /// Starts with an empty database instead of migrating the MongoDB data.
    #[arg(long = "fresh-start")]
    pub fresh_start: bool,
    /// Skips installing and building the services.
    #[arg(long = "skip-install")]
    pub skip_install: bool,
    /// Reads the Discord bot's token from stdin into secrets.env and turns the bot on (settings.toml's discord table).
    #[arg(long = "discord-bot-token")]
    pub discord_bot_token: bool,
    /// Turns the Discord bot off; its token stays in secrets.env.
    #[arg(long = "no-discord")]
    pub no_discord: bool,
    /// A Discord server (guild id) the bot answers in; repeat for more. Replaces the list settings.toml has.
    #[arg(long = "discord-guild")]
    pub discord_guilds: Vec<String>,
    /// The Discord bot's application id, checked against its token's.
    #[arg(long = "discord-application-id")]
    pub discord_application_id: Option<String>,
    /// dainty-dash, which the bot asks whether signups are on (left out: settings.toml's dashboard_url).
    #[arg(long = "discord-dashboard-url")]
    pub discord_dashboard_url: Option<String>,
    /// Players sign in to the launcher with Discord: the Discord application's client id (settings.toml's oauth table).
    #[arg(long = "oauth-discord", value_name = "CLIENT_ID")]
    pub oauth_discord: Option<String>,
    /// Reads the OAuth application's client secret from stdin into secrets.env.
    #[arg(long = "oauth-client-secret")]
    pub oauth_client_secret: bool,
    /// A Discord server (guild id) a player must be in to sign in; repeat for more. Replaces settings.toml's list.
    #[arg(long = "oauth-guild")]
    pub oauth_guilds: Vec<String>,
    /// New accounts are made only by signing in.
    #[arg(long = "oauth-required")]
    pub oauth_required: bool,
    /// Signing in is one way to make an account; a name and invite code still make one too.
    #[arg(long = "oauth-optional")]
    pub oauth_optional: bool,
    /// Turns OAuth sign-in off (removes settings.toml's oauth table); the client secret stays in secrets.env.
    #[arg(long = "no-oauth")]
    pub no_oauth: bool,
    /// Words setup did not expect, refused with a hint to quote values with spaces.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub unexpected: Vec<String>,
}

/// Options of `serve`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ServeArgs {
    /// The settings.toml (or .yaml, .yml, .json) to run; the workspace's settings.toml when not given.
    #[arg(short = 'c', long)]
    pub config: Option<PathBuf>,
    /// Shows the dashboard instead of the line console.
    #[arg(long)]
    pub tui: bool,
    /// The log filter, as `RUST_LOG` takes it.
    #[arg(long = "log-level", default_value = "info")]
    pub log_level: String,
}

impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            config: None,
            tui: false,
            log_level: "info".to_owned(),
        }
    }
}

/// Options of `invite`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InviteArgs {
    /// Who the invite is for, kept as the code's note.
    #[arg(long = "for")]
    pub for_whom: Option<String>,
    /// How many accounts the code registers, 1 to 10000.
    #[arg(long, default_value = "1")]
    pub uses: String,
    /// Makes a code with no limit on its uses.
    #[arg(long)]
    pub unlimited: bool,
    /// Lists the invite codes instead of making one.
    #[arg(long)]
    pub list: bool,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
    /// The game version to invite to; 1.14.7 when not given.
    #[arg(long)]
    pub game: Option<String>,
}

impl Default for InviteArgs {
    fn default() -> Self {
        Self {
            for_whom: None,
            uses: "1".to_owned(),
            unlimited: false,
            list: false,
            json: false,
            game: None,
        }
    }
}

/// Options of `doctor`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Args)]
pub struct DoctorArgs {
    /// Skips hashing the game executable.
    #[arg(long = "no-hash")]
    pub no_hash: bool,
}

/// Options of `gamedata`; with none, generates the game data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct GamedataArgs {
    /// Seeds the example game data.
    #[arg(long)]
    pub examples: bool,
    /// Copies the supplied source files from this folder into the raw game data.
    #[arg(long = "example-sources")]
    pub example_sources: Option<String>,
    /// Builds the progression source from the exports in this folder (the workspace's exports when empty).
    #[arg(long)]
    pub progression: Option<String>,
    /// Exports the game data as a pack to this file or folder.
    #[arg(long)]
    pub export: Option<String>,
    /// Imports a game-data pack.
    #[arg(long)]
    pub import: Option<String>,
}

/// Options of `publish-runtime`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct PublishRuntimeArgs {
    /// The folder holding the runtime's DLLs; the default build outputs when not given.
    #[arg(long)]
    pub from: Option<String>,
}

/// Options of `install-game`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct InstallGameArgs {
    /// The folder to install into, as a full path.
    #[arg(long)]
    pub dir: Option<String>,
    /// The game version to install.
    #[arg(long)]
    pub version: Option<String>,
    /// A file holding the Epic authorization code, instead of pasting it.
    #[arg(long = "code-file")]
    pub code_file: Option<String>,
    /// Signs the host's Epic account out and installs nothing.
    #[arg(long = "sign-out")]
    pub sign_out: bool,
}

/// Options of `preset`; with none, shows the preset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct PresetArgs {
    /// Creates the preset from this server's game data, under this name.
    #[arg(long)]
    pub create: Option<String>,
    /// Generates the game data before creating the preset.
    #[arg(long)]
    pub generate: bool,
    /// The new preset's name.
    #[arg(long)]
    pub name: Option<String>,
    /// Applies the preset in this folder (the checkout's when `default`).
    #[arg(long)]
    pub apply: Option<String>,
    /// Replaces game data that differs from the preset, keeping a backup of each file.
    #[arg(long)]
    pub force: bool,
}

/// Options of `multipliers`; with none, shows them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct MultipliersArgs {
    /// The XP multiplier.
    #[arg(long, alias = "xp-multiplier")]
    pub xp: Option<String>,
    /// The reward multiplier.
    #[arg(long, alias = "reward-multiplier", alias = "rewards-multiplier")]
    pub rewards: Option<String>,
    /// Turns the multipliers off.
    #[arg(long)]
    pub off: bool,
    /// Same as --off.
    #[arg(long = "no-multipliers", hide = true)]
    pub no_multipliers: bool,
}

/// Options of `update`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct UpdateArgs {
    /// Only says whether a newer release is out.
    #[arg(long)]
    pub check: bool,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
    /// Doesn't run setup after installing.
    #[arg(long = "no-setup")]
    pub no_setup: bool,
}

/// Options of `db`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct DbArgs {
    /// What to do.
    #[command(subcommand)]
    pub command: DbCommand,
}

/// A `db` command.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
#[allow(
    clippy::large_enum_variant,
    reason = "built once per run; boxing the arguments would only clutter clap's derive"
)]
pub enum DbCommand {
    /// Migrates the MongoDB data into the store.
    Migrate(DbMigrateArgs),
    /// Compares the store with the MongoDB data.
    Verify(DbVerifyArgs),
    /// Exports the store as MongoDB collections.
    #[command(name = "export-mongo")]
    ExportMongo(DbExportMongoArgs),
    /// Replays a capture corpus against recorded or live servers and reports the differences.
    Replay(DbReplayArgs),
}

/// Options of `db migrate`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct DbMigrateArgs {
    /// The MongoDB URI to read; backend.env's or settings.toml's when not given.
    #[arg(long = "from", visible_alias = "from-mongo")]
    pub from: Option<String>,
    /// The store to write: a SQLite file or a postgres:// URL; settings.toml's when not given.
    #[arg(long)]
    pub to: Option<String>,
    /// The node id derived ids are made for (DD18); `local` when not given.
    #[arg(long)]
    pub node: Option<String>,
    /// The areas to migrate, comma-separated or repeated; all when not given.
    #[arg(long)]
    pub area: Vec<String>,
    /// Reads and checks without writing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Skips documents that don't convert instead of stopping.
    #[arg(long = "skip-invalid")]
    pub skip_invalid: bool,
    /// The folder holding the inventory storage policy.
    #[arg(long = "game-data-dir")]
    pub game_data_dir: Option<String>,
    /// Allows a Postgres store off this machine.
    #[arg(long = "allow-remote-postgres")]
    pub allow_remote_postgres: bool,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `db verify`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct DbVerifyArgs {
    /// The MongoDB URI to compare with; backend.env's or settings.toml's when not given.
    #[arg(long = "against", visible_aliases = ["from", "from-mongo"])]
    pub against: Option<String>,
    /// The store to check: a SQLite file or a postgres:// URL; settings.toml's when not given.
    #[arg(long)]
    pub to: Option<String>,
    /// The node id derived ids were made for (DD18); `local` when not given.
    #[arg(long)]
    pub node: Option<String>,
    /// The areas to compare, comma-separated or repeated; all when not given.
    #[arg(long)]
    pub area: Vec<String>,
    /// Skips documents that don't convert instead of stopping.
    #[arg(long = "skip-invalid")]
    pub skip_invalid: bool,
    /// The folder holding the inventory storage policy.
    #[arg(long = "game-data-dir")]
    pub game_data_dir: Option<String>,
    /// Allows a Postgres store off this machine.
    #[arg(long = "allow-remote-postgres")]
    pub allow_remote_postgres: bool,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `db export-mongo`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct DbExportMongoArgs {
    /// The store to read: a SQLite file or a postgres:// URL.
    #[arg(long, visible_alias = "from-mongo")]
    pub from: Option<String>,
    /// Where the collections go; the current folder when not given.
    #[arg(long, visible_alias = "out")]
    pub to: Option<String>,
    /// The areas to export, comma-separated or repeated; all when not given.
    #[arg(long)]
    pub area: Vec<String>,
    /// Reads without writing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Allows a Postgres store off this machine.
    #[arg(long = "allow-remote-postgres")]
    pub allow_remote_postgres: bool,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `db replay`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct DbReplayArgs {
    /// The folder of captured gateway traffic (JSONL) to replay.
    #[arg(long)]
    pub corpus: String,
    /// The baseline: `recorded` for the captured answers, or a backend URL.
    #[arg(long, default_value = "recorded")]
    pub left: String,
    /// The backend URL compared with the baseline.
    #[arg(long, default_value = "http://127.0.0.1:41000")]
    pub right: String,
    /// The baseline's Director URL.
    #[arg(long = "left-director")]
    pub left_director: Option<String>,
    /// The compared side's Director URL.
    #[arg(long = "right-director")]
    pub right_director: Option<String>,
    /// The baseline's chat URL.
    #[arg(long = "left-chat")]
    pub left_chat: Option<String>,
    /// The compared side's chat URL.
    #[arg(long = "right-chat")]
    pub right_chat: Option<String>,
    /// The JWT secret of both sides; secrets.env's when not given.
    #[arg(long = "jwt-secret")]
    pub jwt_secret: Option<String>,
    /// The baseline's JWT secret.
    #[arg(long = "left-jwt-secret")]
    pub left_jwt_secret: Option<String>,
    /// The compared side's JWT secret.
    #[arg(long = "right-jwt-secret")]
    pub right_jwt_secret: Option<String>,
    /// The signing key of both sides; secrets.env's when not given.
    #[arg(long = "signing-key")]
    pub signing_key: Option<String>,
    /// The baseline's signing key.
    #[arg(long = "left-signing-key")]
    pub left_signing_key: Option<String>,
    /// The compared side's signing key.
    #[arg(long = "right-signing-key")]
    pub right_signing_key: Option<String>,
    /// The game server key of both sides; secrets.env's when not given.
    #[arg(long = "gs-key")]
    pub gs_key: Option<String>,
    /// The baseline's game server key.
    #[arg(long = "left-gs-key")]
    pub left_gs_key: Option<String>,
    /// The compared side's game server key.
    #[arg(long = "right-gs-key")]
    pub right_gs_key: Option<String>,
    /// The gateway secret; secrets.env's when not given.
    #[arg(long = "gateway-secret")]
    pub gateway_secret: Option<String>,
    /// A JSON file mapping the recorded keys to the replayed ones.
    #[arg(long = "key-map")]
    pub key_map: Option<String>,
    /// Pins both sides' clocks to the captured times (the default).
    #[arg(long = "pin-clock")]
    pub pin_clock: bool,
    /// Lets both sides use their own clocks.
    #[arg(long = "no-pin-clock")]
    pub no_pin_clock: bool,
    /// Replays only the requests this matches.
    #[arg(long)]
    pub filter: Option<String>,
    /// Writes the report to this file.
    #[arg(long)]
    pub out: Option<String>,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `network`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct NetworkArgs {
    /// What to do.
    #[command(subcommand)]
    pub command: NetworkCommand,
}

/// A `network` command.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum NetworkCommand {
    /// Creates the cluster CA that signs every node's certificate.
    #[command(name = "ca-init")]
    CaInit(CaInitArgs),
    /// Issues a node's certificate and key from the cluster CA.
    #[command(name = "issue-node-cert")]
    IssueNodeCert(IssueNodeCertArgs),
    /// Lists the certificates a CA issued, or this server's node and peers.
    List(ListArgs),
    /// Probes this server's node links to its peers.
    Status(StatusArgs),
    /// Joins this server's database to its network's directory.
    Join(JoinArgs),
}

/// Options of `network ca-init`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct CaInitArgs {
    /// The CA file to write; cluster-ca.pem when not given.
    #[arg(long)]
    pub out: Option<String>,
    /// Replaces a file that exists.
    #[arg(long)]
    pub force: bool,
}

/// Options of `network issue-node-cert`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct IssueNodeCertArgs {
    /// The cluster CA file.
    #[arg(long)]
    pub ca: String,
    /// The node's id.
    #[arg(long)]
    pub node: String,
    /// The node's region.
    #[arg(long)]
    pub region: String,
    /// A name or address the node is reached at; repeat for each.
    #[arg(long, required = true)]
    pub host: Vec<String>,
    /// How many days the certificate is valid.
    #[arg(long)]
    pub days: Option<String>,
    /// The folder to write into; one named after the node when not given.
    #[arg(long)]
    pub out: Option<String>,
    /// Replaces files that exist.
    #[arg(long)]
    pub force: bool,
}

/// Options of `network list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct ListArgs {
    /// The CA file whose issued certificates to list; this server's node and peers when not given.
    #[arg(long)]
    pub ca: Option<String>,
}

/// Options of `network status`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct StatusArgs {
    /// The gateway.env to read the node links from; this server's when not given.
    #[arg(long = "env-file")]
    pub env_file: Option<String>,
    /// How many pings to time for each peer, 1 to 100.
    #[arg(long, default_value_t = 3)]
    pub count: u32,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

impl Default for StatusArgs {
    fn default() -> Self {
        Self {
            env_file: None,
            count: 3,
            json: false,
        }
    }
}

/// `network status`'s options under the name the network module uses.
pub type NetworkStatusArgs = StatusArgs;

/// Options of `network join`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Args)]
pub struct JoinArgs {
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Which logged changes `history` and `revert` take.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct HistoryFilterArgs {
    /// Only changes to this account.
    #[arg(long)]
    pub account: Option<String>,
    /// Only changes made by this actor.
    #[arg(long)]
    pub actor: Option<String>,
    /// Only changes from this time on.
    #[arg(long)]
    pub since: Option<String>,
    /// Only changes before this time.
    #[arg(long)]
    pub until: Option<String>,
}

/// Options of `history`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct HistoryArgs {
    /// Which changes.
    #[command(flatten)]
    pub filter: HistoryFilterArgs,
    /// How many changes a page shows.
    #[arg(long, default_value = "50")]
    pub limit: String,
    /// Where the page starts: the cursor the previous page printed.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Checks the log instead of listing it.
    #[command(subcommand)]
    pub command: Option<HistoryCommand>,
}

impl Default for HistoryArgs {
    fn default() -> Self {
        Self {
            filter: HistoryFilterArgs::default(),
            limit: "50".to_owned(),
            cursor: None,
            command: None,
        }
    }
}

/// A `history` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum HistoryCommand {
    /// Checks that each database's change chain is unbroken.
    Verify,
}

/// Options of `revert`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RevertArgs {
    /// Which changes to undo.
    #[command(flatten)]
    pub filter: HistoryFilterArgs,
    /// Shows what would change and keeps nothing (the default).
    #[arg(long = "dry-run", conflicts_with = "apply")]
    pub dry_run: bool,
    /// Applies the dry run with this plan id, if the changes still plan the same.
    #[arg(long)]
    pub apply: Option<String>,
}

/// Options of `legacy`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct LegacyArgs {
    /// What to do.
    #[command(subcommand)]
    pub command: LegacyCommand,
}

/// A `legacy` command.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum LegacyCommand {
    /// Imports an old install into this server.
    Import(LegacyImportArgs),
}

/// Options of `legacy import`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct LegacyImportArgs {
    /// revived, offline-save or reforged
    pub kind: String,
    /// dainty-legacy's executable, when settings.toml pins none (a Node kit before its first setup)
    #[arg(long)]
    pub program: Option<std::path::PathBuf>,
    /// Passed on to `dainty-legacy import <kind>`
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

/// Options of `remote`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct RemoteArgs {
    /// What to do.
    #[command(subcommand)]
    pub command: RemoteCommand,
}

/// A `remote` command.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum RemoteCommand {
    /// Makes this server the controller of its installs.
    #[command(name = "init-controller", visible_alias = "init")]
    InitController(RemoteInitControllerArgs),
    /// Makes a single-use enrollment invite for an install.
    Invite(RemoteInviteArgs),
    /// Enrolls this install with a controller's invite.
    Enroll(RemoteEnrollArgs),
    /// Shows this install's remote management state.
    Status(RemoteStatusArgs),
    /// Lists the controller's enrolled installs.
    List(RemoteStatusArgs),
    /// Revokes an enrolled install.
    Revoke(RemoteRevokeArgs),
    /// Leaves remote management; the install runs standalone.
    Leave(RemoteLeaveArgs),
    /// Publishes the shared configuration to the enrolled installs.
    #[command(name = "set-config")]
    SetConfig(RemoteSetConfigArgs),
    /// Runs a command on enrolled installs.
    Exec(RemoteExecArgs),
    /// Approves a guarded command.
    Approve(RemoteApproveArgs),
    /// Cancels a guarded command.
    Cancel(RemoteCancelArgs),
    /// Syncs with the controller now, or in a loop.
    Sync(RemoteSyncArgs),
}

/// Options of `remote init-controller`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteInitControllerArgs {
    /// The controller's node id; settings.toml's when not given.
    #[arg(long)]
    pub node: Option<String>,
    /// The controller's region; settings.toml's when not given.
    #[arg(long)]
    pub region: Option<String>,
    /// The URL installs reach the controller at; the gateway's when not given.
    #[arg(long)]
    pub url: Option<String>,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote invite`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteInviteArgs {
    /// The node id the install will have.
    #[arg(long)]
    pub node: String,
    /// The install's region; the controller's when not given.
    #[arg(long)]
    pub region: Option<String>,
    /// A label for the install.
    #[arg(long)]
    pub label: Option<String>,
    /// How long the invite is valid, in seconds.
    #[arg(long = "ttl-secs")]
    pub ttl_secs: Option<i64>,
    /// The URL installs reach the controller at.
    #[arg(long)]
    pub url: Option<String>,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote enroll`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteEnrollArgs {
    /// The enrollment invite URI from `remote invite`.
    pub uri: String,
    /// A label for this install; the invite's when not given.
    #[arg(long)]
    pub label: Option<String>,
    /// The URL the controller reaches this install's gateway at; a public server's own when not given.
    #[arg(long = "gateway-url")]
    pub gateway_url: Option<String>,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote status` and `remote list`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Args)]
pub struct RemoteStatusArgs {
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote revoke`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteRevokeArgs {
    /// The node id of the install to revoke.
    pub node: String,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote leave`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Args)]
pub struct RemoteLeaveArgs {
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote set-config`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteSetConfigArgs {
    /// Takes the shared configuration from settings.toml; the command always does, so the flag changes nothing.
    #[arg(long = "from-host-toml")]
    pub from_settings_toml: bool,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote exec`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteExecArgs {
    /// The node id to run it on, or `all`.
    #[arg(long)]
    pub node: String,
    /// The action: start, stop, restart, setup, setup-fresh-start, doctor, backup, ban, unban or kick.
    #[arg(long)]
    pub action: String,
    /// The account to ban, unban or kick.
    #[arg(long)]
    pub account: Option<String>,
    /// Why, for ban, unban and kick.
    #[arg(long)]
    pub reason: Option<String>,
    /// Accepted and ignored.
    #[arg(long = "for")]
    pub for_whom: Option<String>,
    /// The role the command runs as: viewer, moderator, admin or owner.
    #[arg(long, default_value = "admin")]
    pub role: String,
    /// Who runs it, as the log shows.
    #[arg(long, default_value = "cli-operator")]
    pub actor: String,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote approve`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteApproveArgs {
    /// The guarded command's id.
    pub id: String,
    /// The role the approval is made as.
    #[arg(long, default_value = "owner")]
    pub role: String,
    /// Who approves, as the log shows.
    #[arg(long, default_value = "cli-approver")]
    pub actor: String,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote cancel`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteCancelArgs {
    /// The guarded command's id.
    pub id: String,
    /// The role the cancel is made as.
    #[arg(long, default_value = "admin")]
    pub role: String,
    /// Who cancels, as the log shows.
    #[arg(long, default_value = "cli-operator")]
    pub actor: String,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of `remote sync`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RemoteSyncArgs {
    /// Keeps syncing until stopped.
    #[arg(long = "loop")]
    pub continuous: bool,
    /// Seconds between syncs; settings.toml's, or 5, when not given.
    #[arg(long = "interval-secs")]
    pub interval_secs: Option<u64>,
    /// Prints JSON.
    #[arg(long)]
    pub json: bool,
}

/// Options of the hidden `gameserver` command.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct GameserverArgs {
    /// The game server's arguments.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn parses_core_commands_and_flags() {
        let cli = Cli::parse_from(["dainty-host", "setup", "--mode", "private", "--fresh-start"]);
        assert_eq!(cli.command.name(), "setup");
        if let Command::Setup(s) = cli.command {
            assert_eq!(s.mode.as_deref(), Some("private"));
            assert!(s.fresh_start);
        } else {
            panic!("expected Setup");
        }

        let cli = Cli::parse_from(["dainty-host", "gateway", "serve", "--env-file", "g.env"]);
        assert_eq!(cli.command.name(), "gateway");
        if let Command::Gateway { args } = cli.command {
            assert_eq!(args, vec!["serve", "--env-file", "g.env"]);
        } else {
            panic!("expected Gateway");
        }

        let cli = Cli::parse_from([
            "dainty-host",
            "db",
            "replay",
            "--corpus",
            ".workspace/captures",
            "--right",
            "http://127.0.0.1:61100",
            "--key-map",
            "keys.json",
            "--json",
        ]);
        if let Command::Db(DbArgs {
            command: DbCommand::Replay(r),
        }) = cli.command
        {
            assert_eq!(r.corpus, ".workspace/captures");
            assert_eq!(r.left, "recorded");
            assert_eq!(r.right, "http://127.0.0.1:61100");
            assert_eq!(r.key_map.as_deref(), Some("keys.json"));
            assert!(r.json);
        } else {
            panic!("expected Db::Replay");
        }
    }
}
