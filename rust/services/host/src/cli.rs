//! The command line: the Node kit's commands with their options (`legacy/ReforgedHost/src/cli.ts`),
//! so an existing host's habits, scripts and the launcher's Host page keep working (section 6).
//!
//! Every option is kept as the Node kit has it. Values stay text here: each command checks its own
//! values with the Node kit's messages (`--max-hunts`, `--public-port`, `--xp x5` ...). Options
//! whose value the Node kit lets be left out (`gamedata --export [file]`, `--import`, `--from`)
//! take an optional value, so the command can answer as the Node kit does.
//!
//! RH1 owns this file; a row that needs another option goes through the integration pass.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "reforged-host",
    about = "DaintyLass host kit: sets up the server, runs it and makes invites",
    after_help = "See README.md and docs/hosting.md.",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

// Parsed once per run: boxing setup's options would only make the rows' code noisier.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Set up (or update) this server: Mystic Paradox, the services, host.toml and the env files
    Setup(SetupArgs),
    /// Run every service until Ctrl+C (or a "stop" line with REFORGED_CONTROL=stdin)
    Start,
    /// Run the server in this process with a console (--tui for the dashboard)
    Serve(ServeArgs),
    /// Make an invite link for a player
    Invite(InviteArgs),
    /// Check what is still missing or wrong
    Doctor(DoctorArgs),
    /// Generate the game data from your export, or use the examples, or move it as a pack
    Gamedata(GamedataArgs),
    /// Publish the game runtime (MysticParadox.dll and winmm.dll) for the players
    PublishRuntime(PublishRuntimeArgs),
    /// Install Dauntless from your own Epic library, for the game servers (sign-in with a pasted code)
    InstallGame(InstallGameArgs),
    /// Show, create or apply the project's server preset (preset/)
    Preset(PresetArgs),
    /// XP and reward multipliers (private test servers only)
    Multipliers(MultipliersArgs),
    /// Check for a new server release, install it and run setup again
    Update(UpdateArgs),
    /// Back up the server's database
    Backup,
    /// The network's cluster CA and node certificates (a server in a network, DD16)
    Network(NetworkArgs),
    /// What changed in the server's data, by whom and when (DD23); `history verify` checks the log's hash chain
    History(HistoryArgs),
    /// Undo what an actor, an account or a time range changed (DD23): a dry run first, then `--apply <plan id>`
    Revert(RevertArgs),
    /// Migrate, verify or export the server's database between MongoDB and store (RB4, RB13)
    Db(DbArgs),
    /// Linux: one game server through Wine or Proton, as the Director starts it (the Node kit's
    /// `dist/gameserver.js`, run by `.workspace/bin/reforged-gameserver`). Not for people.
    #[command(hide = true)]
    Gameserver {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// One service alone, as its own binary ran it: `gateway serve --env-file <file>`, `gateway make-cert`,
    /// `allowlist --env-file <file> [--close-ports]`, `content serve`, `admin serve|schema`. For the
    /// systemd units and the containers, which run each service with its own privileges (RD26).
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
}

impl Command {
    /// The command's name as typed.
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
            Self::Network(_) => "network",
            Self::History(_) => "history",
            Self::Revert(_) => "revert",
            Self::Db(_) => "db",
            Self::Gameserver { .. } => "gameserver",
            Self::Gateway { .. } => "gateway",
            Self::Allowlist { .. } => "allowlist",
            Self::Content { .. } => "content",
            Self::Admin { .. } => "admin",
            Self::Backend { .. } => "backend",
            Self::Director { .. } => "director",
        }
    }
}

#[derive(Debug, Clone, Default, Args)]
pub struct SetupArgs {
    /// private (friends over Tailscale) or public (a server on the internet)
    #[arg(long, value_name = "private|public")]
    pub mode: Option<String>,
    /// reborn (Dauntless 1.14.7 on Mystic Paradox, the default), legacy (Dauntless Revived's 1.4.4), or both
    #[arg(long, value_name = "reborn|legacy|both")]
    pub edition: Option<String>,
    /// Private: this PC's Tailscale IP (found when Tailscale runs). Public: the public IPv4 or DNS name
    #[arg(long, value_name = "address")]
    pub host: Option<String>,
    /// The server's name
    #[arg(long, value_name = "server name")]
    pub name: Option<String>,
    /// The Dauntless folder (the one with Archon), as a full path: 1.14.7, or 1.4.4 for --edition legacy
    #[arg(long, value_name = "Dauntless folder")]
    pub game: Option<String>,
    /// --edition both: the Dauntless 1.4.4 folder (--game is the 1.14.7 one)
    #[arg(long, value_name = "Dauntless 1.4.4 folder")]
    pub legacy_game: Option<String>,
    /// The IPv4 the game servers announce (a public server on a DNS name needs it)
    #[arg(long, value_name = "IPv4")]
    pub game_address: Option<String>,
    /// Who may register
    #[arg(long, value_name = "INVITECODE|OPEN|NONE")]
    pub registration: Option<String>,
    /// A Tailscale share link to put in invites (private servers)
    #[arg(long, value_name = "Tailscale share link")]
    pub share: Option<String>,
    /// The backend's MongoDB connection string
    #[arg(long, value_name = "uri")]
    pub mongo_uri: Option<String>,
    /// The host kit runs its own MongoDB (the default with the default URI)
    #[arg(long)]
    pub manage_mongo: bool,
    /// Use a MongoDB that runs on its own (with --mongo-uri)
    #[arg(long)]
    pub no_manage_mongo: bool,
    /// The mongod program, when it is not found
    #[arg(long, value_name = "path")]
    pub mongod: Option<String>,
    /// Hunts at once
    #[arg(long, value_name = "1-10")]
    pub max_hunts: Option<String>,
    /// Do not install and build the services
    #[arg(long)]
    pub skip_install: bool,
    /// Start on a new database without migrating existing MongoDB data (MongoDB data is kept intact)
    #[arg(long)]
    pub fresh_start: bool,
    /// Download and patch Mystic Paradox again even when it is in place
    #[arg(long)]
    pub force_upstream: bool,
    /// The gateway's port when 443 is taken (a hosting panel, for example); 443 goes back
    #[arg(long, value_name = "port", help_heading = "Public servers")]
    pub public_port: Option<String>,
    /// --edition both: the 1.4.4 gateway's port (4443 unless given)
    #[arg(long, value_name = "port", help_heading = "Public servers")]
    pub legacy_public_port: Option<String>,
    /// The wine program (a path, or a name on PATH)
    #[arg(long, value_name = "path", help_heading = "Linux")]
    pub wine: Option<String>,
    /// A Proton (or GE-Proton) script to run the game servers with instead of Wine
    #[arg(long, value_name = "script", help_heading = "Linux")]
    pub proton: Option<String>,
    /// Back to Wine
    #[arg(long, help_heading = "Linux")]
    pub no_proton: bool,
    /// A virtual display (xvfb-run) for the game servers (the default)
    #[arg(long, help_heading = "Linux")]
    pub xvfb: bool,
    /// No virtual display
    #[arg(long, help_heading = "Linux")]
    pub no_xvfb: bool,
    /// Public: the allowlist filters the game ports on this interface only
    #[arg(long, value_name = "eth0", help_heading = "Linux")]
    pub firewall_interface: Option<String>,
    /// The allowlist filters on every interface (the default)
    #[arg(long, help_heading = "Linux")]
    pub no_firewall_interface: bool,
    /// Each game server's frame rate with players on it; 0: no cap
    #[arg(long, value_name = "0|10-240", help_heading = "CPU and RAM")]
    pub server_fps: Option<String>,
    /// The frame rate after 15 s without players; 0: the same as --server-fps
    #[arg(long, value_name = "0|1-240", help_heading = "CPU and RAM")]
    pub idle_fps: Option<String>,
    /// The Training Grounds start when someone goes there (the default)
    #[arg(long, help_heading = "CPU and RAM")]
    pub training_on_demand: bool,
    /// The Training Grounds start with the server
    #[arg(long, help_heading = "CPU and RAM")]
    pub training_at_boot: bool,
    /// Warnings and errors only in the game servers' logs (the default)
    #[arg(long, help_heading = "CPU and RAM")]
    pub quiet_game_logs: bool,
    /// Upstream's diagnostic log levels in the game servers' logs
    #[arg(long, help_heading = "CPU and RAM")]
    pub verbose_game_logs: bool,
    /// A new hunt only when there is memory for it (the default)
    #[arg(long, help_heading = "CPU and RAM")]
    pub memory_guard: bool,
    /// Start hunts whatever the free memory
    #[arg(long, help_heading = "CPU and RAM")]
    pub no_memory_guard: bool,
    /// Memory left free after a new hunt
    #[arg(long, value_name = "MB", help_heading = "CPU and RAM")]
    pub min_free_ram: Option<String>,
    /// Memory one hunt needs
    #[arg(long, value_name = "MB", help_heading = "CPU and RAM")]
    pub hunt_ram: Option<String>,
    /// On demand: stop the Training Grounds after this long without players; 0: keep them running
    #[arg(long, value_name = "0-1440", help_heading = "CPU and RAM")]
    pub training_idle_minutes: Option<String>,
    /// Multiply the XP the game servers grant
    #[arg(long, value_name = "1-100", help_heading = "Test multipliers (private servers)")]
    pub xp_multiplier: Option<String>,
    /// Multiply the loot the game servers grant
    #[arg(long, value_name = "1-100", help_heading = "Test multipliers (private servers)")]
    pub reward_multiplier: Option<String>,
    /// Turns the multipliers off
    #[arg(long, help_heading = "Test multipliers (private servers)")]
    pub off: bool,
    /// Same as --off
    #[arg(long, help_heading = "Test multipliers (private servers)")]
    pub no_multipliers: bool,
    /// Words that are not options: a value with spaces that was not quoted. Setup refuses them and
    /// says to quote the value, as the Node kit does.
    #[arg(hide = true)]
    pub unexpected: Vec<String>,
}

#[derive(Debug, Clone, Default, Args)]
pub struct InviteArgs {
    /// How many players can register with it
    #[arg(long, value_name = "N", default_value = "1")]
    pub uses: String,
    /// Any number of players
    #[arg(long)]
    pub unlimited: bool,
    /// Who the invite is for (a note on the code)
    #[arg(long = "for", value_name = "who")]
    pub for_whom: Option<String>,
    /// List the invite codes instead
    #[arg(long)]
    pub list: bool,
    /// A server running both games: which one the invite is for (reforged or 1.14.7, revived or 1.4.4)
    #[arg(long, value_name = "game")]
    pub game: Option<String>,
    /// One JSON line, for the launcher's Host page
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct DoctorArgs {
    /// Do not hash the game executable
    #[arg(long)]
    pub no_hash: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct GamedataArgs {
    /// Copy the example data where data is missing (a start-up test only)
    #[arg(long)]
    pub examples: bool,
    /// Use the example sources for missing inputs
    #[arg(long)]
    pub example_sources: bool,
    /// Only the progression config, from the progression export
    #[arg(long)]
    pub progression: bool,
    /// Write a game data pack (to the file, or the default name)
    #[arg(long, value_name = "file", num_args = 0..=1)]
    pub export: Option<Option<String>>,
    /// Import a game data pack
    #[arg(long, value_name = "file", num_args = 0..=1)]
    pub import: Option<Option<String>>,
}

/// `install-game` (DE6). The Epic session is never an option: it is kept in `epic-session.env` (owner-only).
#[derive(Debug, Clone, Default, Args)]
pub struct InstallGameArgs {
    /// Which Dauntless to install: the one the game servers run (1.14.7, the default) or 1.12.0
    #[arg(long, value_name = "1.14.7|1.12.0")]
    pub version: Option<String>,
    /// The folder each version's own folder goes in, as a full path (<dir>/1.14.7); .workspace/game when left out
    #[arg(long, value_name = "folder")]
    pub dir: Option<String>,
    /// Read the Epic authorization code from this file instead of asking for it
    #[arg(long, value_name = "file")]
    pub code_file: Option<String>,
    /// End the Epic session this host holds and delete it, then stop
    #[arg(long)]
    pub sign_out: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct PublishRuntimeArgs {
    /// The folder with MysticParadox.dll and winmm.dll, instead of this checkout's builds
    #[arg(long, value_name = "folder", num_args = 0..=1)]
    pub from: Option<Option<String>>,
}

#[derive(Debug, Clone, Default, Args)]
pub struct PresetArgs {
    /// Make the preset from this server's game data
    #[arg(long)]
    pub create: bool,
    /// With --create: generate the game data first
    #[arg(long)]
    pub generate: bool,
    /// With --create: the preset's name
    #[arg(long, value_name = "name")]
    pub name: Option<String>,
    /// Apply the preset now
    #[arg(long)]
    pub apply: bool,
    /// With --apply: replace your own game data too
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct MultipliersArgs {
    /// Multiply the XP the game servers grant
    #[arg(long, value_name = "1-100")]
    pub xp: Option<String>,
    /// Multiply the loot the game servers grant
    #[arg(long, value_name = "1-100")]
    pub rewards: Option<String>,
    /// Same as --xp (setup's name)
    #[arg(long, value_name = "1-100")]
    pub xp_multiplier: Option<String>,
    /// Same as --rewards (setup's name)
    #[arg(long, value_name = "1-100")]
    pub reward_multiplier: Option<String>,
    /// Turns the multipliers off
    #[arg(long)]
    pub off: bool,
    /// Same as --off with --xp or --rewards (alone it changes nothing, as in the Node kit)
    #[arg(long)]
    pub no_multipliers: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct UpdateArgs {
    /// Only compare, change nothing
    #[arg(long)]
    pub check: bool,
    /// End with one JSON line (the launcher)
    #[arg(long)]
    pub json: bool,
    /// Do not run setup again after updating
    #[arg(long)]
    pub no_setup: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Command {
        let mut argv = vec!["reforged-host"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).unwrap().command
    }

    #[test]
    fn the_tree_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn has_the_node_kits_commands() {
        let names: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|c| c.get_name().to_owned())
            .collect();
        for name in [
            "setup",
            "start",
            "invite",
            "doctor",
            "gamedata",
            "publish-runtime",
            "install-game",
            "preset",
            "multipliers",
            "update",
            "backup",
        ] {
            assert!(names.iter().any(|n| n == name), "{name} in {names:?}");
        }
    }

    #[test]
    fn takes_the_node_kits_options() {
        let Command::Setup(s) = parse(&[
            "setup",
            "--mode",
            "public",
            "--host=203.0.113.7",
            "--name",
            "Friday Hunts",
            "--public-port",
            "4443",
            "--no-xvfb",
            "--server-fps",
            "45",
            "--xp-multiplier",
            "x5",
        ]) else {
            panic!("setup")
        };
        assert_eq!(s.mode.as_deref(), Some("public"));
        assert_eq!(s.host.as_deref(), Some("203.0.113.7"));
        assert_eq!(s.public_port.as_deref(), Some("4443"));
        assert!(s.no_xvfb && !s.xvfb);
        assert_eq!(s.server_fps.as_deref(), Some("45"));
        assert_eq!(s.xp_multiplier.as_deref(), Some("x5"));
        assert!(s.unexpected.is_empty());

        let Command::Setup(s) = parse(&["setup", "--game", "D:\\Games\\My", "Dauntless"]) else {
            panic!("setup")
        };
        assert_eq!(s.unexpected, ["Dauntless"], "left to setup, which names it");

        let Command::Invite(i) = parse(&["invite", "--for", "Alex"]) else {
            panic!("invite")
        };
        assert_eq!(i.uses, "1", "--uses defaults to 1");
        assert_eq!(i.for_whom.as_deref(), Some("Alex"));

        let Command::Gamedata(g) = parse(&["gamedata", "--export"]) else {
            panic!("gamedata")
        };
        assert_eq!(g.export, Some(None), "--export without a file: the default name");
        let Command::Gamedata(g) = parse(&["gamedata", "--export", "pack.zip"]) else {
            panic!("gamedata")
        };
        assert_eq!(g.export, Some(Some("pack.zip".into())));
        let Command::PublishRuntime(p) = parse(&["publish-runtime", "--from"]) else {
            panic!("publish-runtime")
        };
        assert_eq!(
            p.from,
            Some(None),
            "setup answers --from without a folder as the Node kit does"
        );

        let Command::Multipliers(m) = parse(&["multipliers", "--xp", "5", "--rewards", "3"]) else {
            panic!("multipliers")
        };
        assert_eq!((m.xp.as_deref(), m.rewards.as_deref()), (Some("5"), Some("3")));
        let Command::Update(u) = parse(&["update", "--check", "--json"]) else {
            panic!("update")
        };
        assert!(u.check && u.json && !u.no_setup);
        let Command::InstallGame(g) = parse(&[
            "install-game",
            "--version",
            "1.12.0",
            "--dir",
            "/game",
            "--code-file",
            "c",
        ]) else {
            panic!("install-game")
        };
        assert_eq!(
            (
                g.version.as_deref(),
                g.dir.as_deref(),
                g.code_file.as_deref(),
                g.sign_out
            ),
            (Some("1.12.0"), Some("/game"), Some("c"), false)
        );
        assert!(Cli::try_parse_from(["reforged-host", "install-game", "--token", "x"]).is_err());
        let Command::Doctor(d) = parse(&["doctor", "--no-hash"]) else {
            panic!("doctor")
        };
        assert!(d.no_hash);

        let Command::Db(DbArgs {
            command: DbCommand::Migrate(m),
        }) = parse(&["db", "migrate", "--from", "mongodb://127.0.0.1:27017/reforged"])
        else {
            panic!("db migrate")
        };
        assert_eq!(m.from.as_deref(), Some("mongodb://127.0.0.1:27017/reforged"));

        let Command::Db(DbArgs {
            command: DbCommand::Verify(v),
        }) = parse(&["db", "verify", "--against", "mongodb://127.0.0.1:27017/reforged"])
        else {
            panic!("db verify")
        };
        assert_eq!(v.against.as_deref(), Some("mongodb://127.0.0.1:27017/reforged"));

        let Command::Db(DbArgs {
            command: DbCommand::ExportMongo(e),
        }) = parse(&["db", "export-mongo", "--to", "mongodb://127.0.0.1:27017/reforged"])
        else {
            panic!("db export-mongo")
        };
        assert_eq!(e.to.as_deref(), Some("mongodb://127.0.0.1:27017/reforged"));
    }

    #[test]
    fn refuses_what_the_node_kit_never_had() {
        assert!(Cli::try_parse_from(["reforged-host", "frobnicate"]).is_err());
        assert!(Cli::try_parse_from(["reforged-host", "start", "--mode", "public"]).is_err());
        // Removed with `npm run upstream` (RH5, section 6): Mystic Paradox is in this project since RP4 (modules/rustic-isles since DS6).
        assert!(Cli::try_parse_from(["reforged-host", "upstream"]).is_err());
    }
}

/// `serve`: `start` with a console (RD26).
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// host.toml, host.yaml or host.json to run from (the workspace's otherwise)
    #[arg(long, short = 'c', value_name = "FILE")]
    pub config: Option<std::path::PathBuf>,
    /// The dashboard instead of the line console
    #[arg(long)]
    pub tui: bool,
    /// error, warn, info, debug or trace
    #[arg(long, value_name = "LEVEL", default_value = "info")]
    pub log_level: String,
}

#[derive(Debug, Clone, Args)]
pub struct NetworkArgs {
    #[command(subcommand)]
    pub command: NetworkCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum NetworkCommand {
    /// Make the cluster CA: the file that signs every node's certificate. Keep it off the nodes
    CaInit(CaInitArgs),
    /// Issue one node's certificate from the CA file, which is given each time and never stored on a node
    IssueNodeCert(IssueNodeCertArgs),
    /// What the CA has issued (--ca), or this server's node settings and certificate
    List(ListArgs),
    /// Ping every peer over the node link now, with this server's own certificate, and show what came back
    Status(StatusArgs),
    /// Join this existing server's accounts to the network directory, resolving username clashes (DN13)
    Join(JoinArgs),
}

#[derive(Debug, Clone, Default, Args)]
pub struct CaInitArgs {
    /// Where to write the CA file (cluster-ca.pem here). It holds the CA's key
    #[arg(long, value_name = "file")]
    pub out: Option<String>,
    /// Replace the file if it is there (every node certificate it issued stays valid)
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct IssueNodeCertArgs {
    /// The CA file `ca-init` made
    #[arg(long, value_name = "file")]
    pub ca: String,
    /// The node's name, as host.toml's [network] node gives it, such as oregon-1
    #[arg(long, value_name = "id")]
    pub node: String,
    /// Its region, such as oregon
    #[arg(long, value_name = "region")]
    pub region: String,
    /// An IP address or DNS name its gateway answers on, as its peers list it (repeat for more)
    #[arg(long, value_name = "address", required = true)]
    pub host: Vec<String>,
    /// Where to write the node's three files (a folder named after the node here)
    #[arg(long, value_name = "folder")]
    pub out: Option<String>,
    /// How long the certificate is valid (365)
    #[arg(long, value_name = "days")]
    pub days: Option<String>,
    /// Replace the files if they are there
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct ListArgs {
    /// List what this CA file has issued instead of this server's settings
    #[arg(long, value_name = "file")]
    pub ca: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct StatusArgs {
    /// The gateway.env setup wrote, which names the node certificate and the peers (this server's, by default)
    #[arg(long, value_name = "file")]
    pub env_file: Option<String>,
    /// Pings timed per peer after the first, which pays the handshake; the median is shown
    #[arg(long, value_name = "n", default_value_t = 5)]
    pub count: u32,
    /// The answer as JSON, one object for the node and its peers
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct JoinArgs {
    /// The directory leader's node id (defaults to the first peer in [[network.nodes]], or this node if none)
    #[arg(long, value_name = "node")]
    pub leader: Option<String>,
    /// The answer as JSON
    #[arg(long)]
    pub json: bool,
}

/// Which changes: every option given must match. A time is RFC 3339, a date (UTC), or how long ago (`6h`, `2d`).
#[derive(Debug, Clone, Default, Args)]
pub struct HistoryFilterArgs {
    /// Rows that belong to this account id
    #[arg(long, value_name = "id")]
    pub account: Option<String>,
    /// Who made them: admin:<key id>, host:console, player:<account>, system:<task> (a bare key id works)
    #[arg(long, value_name = "who")]
    pub actor: Option<String>,
    /// From this time on
    #[arg(long, value_name = "time")]
    pub since: Option<String>,
    /// Before this time
    #[arg(long, value_name = "time")]
    pub until: Option<String>,
}

#[derive(Debug, Clone, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct HistoryArgs {
    #[command(subcommand)]
    pub command: Option<HistoryCommand>,
    #[command(flatten)]
    pub filter: HistoryFilterArgs,
    /// Changes per page, newest first (1 to 1000)
    #[arg(long, value_name = "N", default_value = "50")]
    pub limit: String,
    /// Continue after a page: the cursor it printed
    #[arg(long, value_name = "cursor")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum HistoryCommand {
    /// Walk each log's hash chain from its start and name the first entry edited or removed
    Verify,
}

#[derive(Debug, Clone, Args)]
pub struct RevertArgs {
    #[command(flatten)]
    pub filter: HistoryFilterArgs,
    /// Show what would change and print the plan's id (also what no option does)
    #[arg(long, conflicts_with = "apply")]
    pub dry_run: bool,
    /// Apply the plan with this id, if it is still what the dry run showed; give the same selection
    #[arg(long, value_name = "plan id")]
    pub apply: Option<String>,
}

/// `db`: migrate, verify or export the server's database between MongoDB and `store` (RB4, RB13).
#[derive(Debug, Clone, Args)]
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DbCommand {
    /// Migrate existing MongoDB collections into the configured SQLite or Postgres store (RB4, RU7)
    Migrate(DbMigrateArgs),
    /// Verify the SQLite or Postgres store against a MongoDB database (RB4, RU7)
    Verify(DbVerifyArgs),
    /// Export the SQLite or Postgres store back to MongoDB for one-release rollback (RB13, RU9)
    ExportMongo(DbExportMongoArgs),
}

#[derive(Debug, Clone, Default, Args)]
pub struct DbMigrateArgs {
    /// Source MongoDB URI (defaults from host.toml [mongo] / backend.env)
    #[arg(long, value_name = "mongodb-uri")]
    pub from: Option<String>,
    /// Target SQLite file or postgres:// URL (defaults from host.toml [database] / secrets.env)
    #[arg(long, value_name = "sqlite-or-postgres")]
    pub to: Option<String>,
    /// Node id for DD18 global account and party ids (defaults from host.toml [network] or "local")
    #[arg(long, value_name = "node-id")]
    pub node: Option<String>,
    /// Migrate only selected area(s): auth, characters, inventory, progression, bounties, parties, reforged
    #[arg(long, value_name = "area", value_delimiter = ',')]
    pub area: Vec<String>,
    /// Read and transform documents without writing to the target store
    #[arg(long)]
    pub dry_run: bool,
    /// Skip malformed source documents with a warning instead of failing
    #[arg(long)]
    pub skip_invalid: bool,
    /// Folder holding inventory_storage_policy.json (defaults to the backend's game-data folder)
    #[arg(long, value_name = "dir")]
    pub game_data_dir: Option<String>,
    /// Allow a non-loopback Postgres target
    #[arg(long)]
    pub allow_remote_postgres: bool,
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct DbVerifyArgs {
    /// MongoDB URI to verify against (defaults from host.toml [mongo] / backend.env)
    #[arg(long, visible_alias = "from", value_name = "mongodb-uri")]
    pub against: Option<String>,
    /// Target SQLite file or postgres:// URL to verify (defaults from host.toml [database] / secrets.env)
    #[arg(long, visible_alias = "target", value_name = "sqlite-or-postgres")]
    pub to: Option<String>,
    /// Node id for DD18 global account and party ids (defaults from host.toml [network] or "local")
    #[arg(long, value_name = "node-id")]
    pub node: Option<String>,
    /// Verify only selected area(s): auth, characters, inventory, progression, bounties, parties, reforged
    #[arg(long, value_name = "area", value_delimiter = ',')]
    pub area: Vec<String>,
    /// Skip malformed source documents with a warning instead of failing
    #[arg(long)]
    pub skip_invalid: bool,
    /// Folder holding inventory_storage_policy.json (defaults to the backend's game-data folder)
    #[arg(long, value_name = "dir")]
    pub game_data_dir: Option<String>,
    /// Allow a non-loopback Postgres target
    #[arg(long)]
    pub allow_remote_postgres: bool,
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Default, Args)]
pub struct DbExportMongoArgs {
    /// Source SQLite file or postgres:// URL (defaults from host.toml [database] / secrets.env)
    #[arg(long, value_name = "sqlite-or-postgres")]
    pub from: Option<String>,
    /// Target MongoDB URI (defaults from host.toml [mongo] / backend.env)
    #[arg(long, value_name = "mongodb-uri")]
    pub to: Option<String>,
    /// Export only selected area(s): auth, characters, inventory, progression, bounties, parties, reforged
    #[arg(long, value_name = "area", value_delimiter = ',')]
    pub area: Vec<String>,
    /// Read and transform rows without writing to MongoDB
    #[arg(long)]
    pub dry_run: bool,
    /// Allow reading from a non-loopback Postgres source
    #[arg(long)]
    pub allow_remote_postgres: bool,
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
}
