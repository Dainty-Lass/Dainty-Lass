// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Clone, Parser)]
#[command(name = "reforged-host", disable_version_flag = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    #[must_use]
    pub fn parse_from<I, T>(itr: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        <Self as Parser>::parse_from(itr)
    }
}

#[derive(Debug, Clone, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    Setup(SetupArgs),
    Start,
    Serve(ServeArgs),
    Invite(InviteArgs),
    Doctor(DoctorArgs),
    Gamedata(GamedataArgs),
    PublishRuntime(PublishRuntimeArgs),
    InstallGame(InstallGameArgs),
    Preset(PresetArgs),
    Multipliers(MultipliersArgs),
    Update(UpdateArgs),
    Backup,
    Db(DbArgs),
    Network(NetworkArgs),
    History(HistoryArgs),
    Revert(RevertArgs),
    #[command(hide = true, disable_help_flag = true)]
    Gateway {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(hide = true, disable_help_flag = true)]
    Allowlist {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(hide = true, disable_help_flag = true)]
    Content {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(hide = true, disable_help_flag = true)]
    Admin {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(hide = true, disable_help_flag = true)]
    Backend {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(hide = true, disable_help_flag = true)]
    Director {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(hide = true, disable_help_flag = true)]
    Gameserver(GameserverArgs),
}

impl Command {
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
            Self::History(_) => "history",
            Self::Revert(_) => "revert",
            Self::Gateway { .. } => "gateway",
            Self::Allowlist { .. } => "allowlist",
            Self::Content { .. } => "content",
            Self::Admin { .. } => "admin",
            Self::Backend { .. } => "backend",
            Self::Director { .. } => "director",
            Self::Gameserver(_) => "gameserver",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct SetupArgs {
    #[arg(long)]
    pub mode: Option<String>,
    #[arg(long)]
    pub edition: Option<String>,
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long = "game-address")]
    pub game_address: Option<String>,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub game: Option<String>,
    #[arg(long = "legacy-game")]
    pub legacy_game: Option<String>,
    #[arg(long)]
    pub registration: Option<String>,
    #[arg(long)]
    pub share: Option<String>,
    #[arg(
        long = "mongo-uri",
        value_name = "uri",
        help = "MongoDB connection string to migrate pre-cutover player data from"
    )]
    pub mongo_uri: Option<String>,
    #[arg(long = "manage-mongo", hide = !cfg!(feature = "paradox"))]
    pub manage_mongo: bool,
    #[arg(long = "no-manage-mongo", hide = !cfg!(feature = "paradox"))]
    pub no_manage_mongo: bool,
    #[arg(long, hide = !cfg!(feature = "paradox"))]
    pub mongod: Option<String>,
    #[arg(
        long = "force-upstream",
        hide = !cfg!(feature = "paradox"),
        help = "Download and patch upstream Mystic Paradox again"
    )]
    pub force_upstream: bool,
    #[arg(long = "max-hunts")]
    pub max_hunts: Option<String>,
    #[arg(long = "xp-multiplier", alias = "xp")]
    pub xp_multiplier: Option<String>,
    #[arg(long = "reward-multiplier", alias = "rewards", alias = "rewards-multiplier")]
    pub reward_multiplier: Option<String>,
    #[arg(long = "no-multipliers", alias = "off")]
    pub no_multipliers: bool,
    #[arg(long = "public-port")]
    pub public_port: Option<String>,
    #[arg(long = "legacy-public-port")]
    pub legacy_public_port: Option<String>,
    #[arg(long = "server-fps")]
    pub server_fps: Option<String>,
    #[arg(long = "idle-fps")]
    pub idle_fps: Option<String>,
    #[arg(long = "training-on-demand")]
    pub training_on_demand: bool,
    #[arg(long = "training-at-boot")]
    pub training_at_boot: bool,
    #[arg(long = "training-idle-minutes")]
    pub training_idle_minutes: Option<String>,
    #[arg(long = "verbose-game-logs")]
    pub verbose_game_logs: bool,
    #[arg(long = "quiet-game-logs")]
    pub quiet_game_logs: bool,
    #[arg(long = "memory-guard")]
    pub memory_guard: bool,
    #[arg(long = "no-memory-guard")]
    pub no_memory_guard: bool,
    #[arg(long = "min-free-ram")]
    pub min_free_ram: Option<String>,
    #[arg(long = "hunt-ram")]
    pub hunt_ram: Option<String>,
    #[arg(long)]
    pub wine: Option<String>,
    #[arg(long)]
    pub proton: Option<String>,
    #[arg(long = "no-proton")]
    pub no_proton: bool,
    #[arg(long)]
    pub xvfb: bool,
    #[arg(long = "no-xvfb")]
    pub no_xvfb: bool,
    #[arg(long = "firewall-interface")]
    pub firewall_interface: Option<String>,
    #[arg(long = "no-firewall-interface")]
    pub no_firewall_interface: bool,
    #[arg(long = "fresh-start")]
    pub fresh_start: bool,
    #[arg(long = "skip-install")]
    pub skip_install: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub unexpected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ServeArgs {
    #[arg(short = 'c', long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub tui: bool,
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

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InviteArgs {
    #[arg(long = "for")]
    pub for_whom: Option<String>,
    #[arg(long, default_value = "1")]
    pub uses: String,
    #[arg(long)]
    pub unlimited: bool,
    #[arg(long)]
    pub list: bool,
    #[arg(long)]
    pub json: bool,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Args)]
pub struct DoctorArgs {
    #[arg(long = "no-hash")]
    pub no_hash: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct GamedataArgs {
    #[arg(long)]
    pub examples: bool,
    #[arg(long = "example-sources")]
    pub example_sources: Option<String>,
    #[arg(long)]
    pub progression: Option<String>,
    #[arg(long)]
    pub export: Option<String>,
    #[arg(long)]
    pub import: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct PublishRuntimeArgs {
    #[arg(long)]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct InstallGameArgs {
    #[arg(long)]
    pub dir: Option<String>,
    #[arg(long)]
    pub version: Option<String>,
    #[arg(long = "code-file")]
    pub code_file: Option<String>,
    #[arg(long = "sign-out")]
    pub sign_out: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct PresetArgs {
    #[arg(long)]
    pub create: Option<String>,
    #[arg(long)]
    pub generate: bool,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub apply: Option<String>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct MultipliersArgs {
    #[arg(long, alias = "xp-multiplier")]
    pub xp: Option<String>,
    #[arg(long, alias = "reward-multiplier", alias = "rewards-multiplier")]
    pub rewards: Option<String>,
    #[arg(long)]
    pub off: bool,
    #[arg(long = "no-multipliers", hide = true)]
    pub no_multipliers: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct UpdateArgs {
    #[arg(long)]
    pub check: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long = "no-setup")]
    pub no_setup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum DbCommand {
    Migrate(DbMigrateArgs),
    Verify(DbVerifyArgs),
    #[command(name = "export-mongo")]
    ExportMongo(DbExportMongoArgs),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct DbMigrateArgs {
    #[arg(long = "from", visible_alias = "from-mongo")]
    pub from: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long)]
    pub node: Option<String>,
    #[arg(long)]
    pub area: Vec<String>,
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    #[arg(long = "skip-invalid")]
    pub skip_invalid: bool,
    #[arg(long = "game-data-dir")]
    pub game_data_dir: Option<String>,
    #[arg(long = "allow-remote-postgres")]
    pub allow_remote_postgres: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct DbVerifyArgs {
    #[arg(long = "against", visible_aliases = ["from", "from-mongo"])]
    pub against: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long)]
    pub node: Option<String>,
    #[arg(long)]
    pub area: Vec<String>,
    #[arg(long = "skip-invalid")]
    pub skip_invalid: bool,
    #[arg(long = "game-data-dir")]
    pub game_data_dir: Option<String>,
    #[arg(long = "allow-remote-postgres")]
    pub allow_remote_postgres: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct DbExportMongoArgs {
    #[arg(long, visible_alias = "from-mongo")]
    pub from: Option<String>,
    #[arg(long, visible_alias = "out")]
    pub to: Option<String>,
    #[arg(long)]
    pub area: Vec<String>,
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    #[arg(long = "allow-remote-postgres")]
    pub allow_remote_postgres: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct NetworkArgs {
    #[command(subcommand)]
    pub command: NetworkCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum NetworkCommand {
    #[command(name = "ca-init")]
    CaInit(CaInitArgs),
    #[command(name = "issue-node-cert")]
    IssueNodeCert(IssueNodeCertArgs),
    List(ListArgs),
    Status(StatusArgs),
    Join(JoinArgs),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct CaInitArgs {
    #[arg(long)]
    pub out: Option<String>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct IssueNodeCertArgs {
    #[arg(long)]
    pub ca: String,
    #[arg(long)]
    pub node: String,
    #[arg(long)]
    pub region: String,
    #[arg(long, required = true)]
    pub host: Vec<String>,
    #[arg(long)]
    pub days: Option<String>,
    #[arg(long)]
    pub out: Option<String>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct ListArgs {
    #[arg(long)]
    pub ca: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct StatusArgs {
    #[arg(long = "env-file")]
    pub env_file: Option<String>,
    #[arg(long, default_value_t = 3)]
    pub count: u32,
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

pub type NetworkStatusArgs = StatusArgs;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Args)]
pub struct JoinArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct HistoryFilterArgs {
    #[arg(long)]
    pub account: Option<String>,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct HistoryArgs {
    #[command(flatten)]
    pub filter: HistoryFilterArgs,
    #[arg(long, default_value = "50")]
    pub limit: String,
    #[arg(long)]
    pub cursor: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum HistoryCommand {
    Verify,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct RevertArgs {
    #[command(flatten)]
    pub filter: HistoryFilterArgs,
    #[arg(long = "dry-run", conflicts_with = "apply")]
    pub dry_run: bool,
    #[arg(long)]
    pub apply: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct GameserverArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn parses_core_commands_and_flags() {
        let cli = Cli::parse_from(["reforged-host", "setup", "--mode", "private", "--fresh-start"]);
        assert_eq!(cli.command.name(), "setup");
        if let Command::Setup(s) = cli.command {
            assert_eq!(s.mode.as_deref(), Some("private"));
            assert!(s.fresh_start);
        } else {
            panic!("expected Setup");
        }

        let cli = Cli::parse_from(["reforged-host", "gateway", "serve", "--env-file", "g.env"]);
        assert_eq!(cli.command.name(), "gateway");
        if let Command::Gateway { args } = cli.command {
            assert_eq!(args, vec!["serve", "--env-file", "g.env"]);
        } else {
            panic!("expected Gateway");
        }
    }
}
