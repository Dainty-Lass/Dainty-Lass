//! The fleet a remote controller manages (MS6, the module-sharing plan's A6, D1): `installs` reads each
//! enrolled install's last report, and a moderation mutation given `node` is queued as a
//! `RemoteCommand::AdminAction` for that install's next sync. The controller hands its remote state in
//! through [`ModuleControl`](crate::module_control::ModuleControl); any other server says it has no fleet.

use async_graphql::{Context, ErrorExtensions, SimpleObject};
use dainty_module_api::network::NodeId;

use crate::game::{Availability, GameServer, GameServerKind};
use crate::input;
use crate::keys::{Caller, Role};
use crate::module_control::control;
use crate::modules::NODE_GAME;
use crate::schema::{Admin, Code, allow, fail};

/// One enrolled install, as the controller hands it over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetInstall {
    /// Its node id.
    pub node: String,
    /// Its region.
    pub region: String,
    /// Display label.
    pub label: String,
    /// online, degraded, offline or revoked
    pub state: String,
    /// When its last report arrived, in Unix milliseconds.
    pub last_report_ms: Option<i64>,
    /// Server version, from the last report.
    pub version: Option<String>,
    /// Players online, from the last report.
    pub players_online: Option<u32>,
    /// Its game servers, from the last report.
    pub game_servers: Vec<dainty_module_api::GameServer>,
    /// Whether players can get in, from the last report.
    pub availability: Option<dainty_module_api::Availability>,
}

/// What a queued moderation action does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetAction {
    /// Disconnect the player.
    Kick,
    /// Ban the player for `duration_secs`.
    Timeout,
    /// End the player's timeout.
    LiftTimeout,
    /// Ban the player.
    Ban,
    /// Lift the player's ban.
    Unban,
}

/// A moderation action for one install, with the key that asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetModeration {
    /// What to do.
    pub action: FleetAction,
    /// The player.
    pub target: String,
    /// Why, as the player reads it.
    pub reason: Option<String>,
    /// How long, for a timeout.
    pub duration_secs: Option<u64>,
    /// The admin key's id, never the key.
    pub key: String,
    /// The key's role.
    pub role: Role,
}

/// Why the controller did not queue an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetRefusal {
    /// This server is not a remote controller.
    NoFleet,
    /// No enrolled install has that node id, or it was revoked.
    UnknownNode(String),
    /// The remote role check or validation refused it.
    Refused(String),
}

/// One install in the fleet.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct Install {
    /// Its node id; `node` on a moderation mutation.
    pub node: String,
    /// Its region.
    pub region: String,
    /// Display label.
    pub label: String,
    /// online, degraded, offline or revoked
    pub state: String,
    /// When its last report arrived (RFC 3339); null before the first.
    pub last_report_at: Option<String>,
    /// Server version, from the last report.
    pub version: Option<String>,
    /// Players online, from the last report.
    pub players_online: Option<u32>,
    /// Whether players can get in, from the last report; null for an install older than MS6.
    pub availability: Option<Availability>,
    /// Its game servers, from the last report.
    pub game_servers: Vec<GameServer>,
}

fn server(from: dainty_module_api::GameServer) -> GameServer {
    use dainty_module_api::GameServerKind as Kind;
    GameServer {
        // A node serves one game (2026-10-31), which the report doesn't repeat
        game: NODE_GAME.to_owned(),
        id: from.id,
        kind: match from.kind {
            Kind::Hub => GameServerKind::Hub,
            Kind::Hunt => GameServerKind::Hunt,
            Kind::Dojo => GameServerKind::Dojo,
            Kind::Tutorial => GameServerKind::Tutorial,
        },
        title: from.title,
        map: from.map,
        host: from.host,
        port: from.port,
        players: from.players,
        max_players: from.max_players,
        ready: from.ready,
        started_at: from.started_at,
    }
}

fn install(from: FleetInstall) -> Install {
    Install {
        node: from.node,
        region: from.region,
        label: from.label,
        state: from.state,
        last_report_at: from.last_report_ms.map(input::iso),
        version: from.version,
        players_online: from.players_online,
        availability: from.availability.map(|a| Availability {
            game: NODE_GAME.to_owned(),
            up: a.up,
            reason: a.reason.map(|r| r.code().to_owned()),
            hub: a.hub.map(server),
            free_hunts: a.free_hunts,
        }),
        game_servers: from.game_servers.into_iter().map(server).collect(),
    }
}

fn no_fleet() -> async_graphql::Error {
    async_graphql::Error::new("this server is not a remote controller, so it has no fleet")
        .extend_with(|_, e| e.set("code", "NO_FLEET"))
}

/// The `installs` query.
pub fn installs(ctx: &Context<'_>) -> async_graphql::Result<Vec<Install>> {
    let installs = control(ctx)?.installs().ok_or_else(no_fleet)?;
    Ok(installs.into_iter().map(install).collect())
}

/// A moderation mutation given `node`: queued for that install's next sync, answered with the code QUEUED
/// and the command id, as a held action is answered with GUARDED. The install's own admin API runs it.
pub async fn queue<T>(
    ctx: &Context<'_>,
    admin: &Admin,
    caller: &Caller,
    node: &str,
    mut moderation: FleetModeration,
) -> async_graphql::Result<T> {
    let (needs, name) = match moderation.action {
        FleetAction::Kick => (Role::Moderator, "kick"),
        FleetAction::Timeout => (Role::Moderator, "timeout"),
        FleetAction::Ban => (Role::Moderator, "ban"),
        FleetAction::Unban => (Role::Admin, "unban"),
        FleetAction::LiftTimeout => (Role::Admin, "liftTimeout"),
    };
    allow(caller, needs, name)?;
    let node: NodeId = node
        .trim()
        .parse()
        .map_err(|_| fail(Code::BadInput, "node is a node id, as installs lists it"))?;
    let target = input::username(&moderation.target)
        .ok_or_else(|| {
            fail(
                Code::BadInput,
                "username must be 3 to 16 letters, digits or underscores",
            )
        })?
        .to_owned();
    moderation.target.clone_from(&target);
    moderation.key.clone_from(&caller.key);
    moderation.role = caller.role;
    let detail_reason = moderation.reason.clone();
    let id = match control(ctx)?.queue_moderation(node.as_str(), moderation) {
        Ok(id) => id,
        Err(FleetRefusal::NoFleet) => return Err(no_fleet()),
        Err(FleetRefusal::UnknownNode(why)) => return Err(fail(Code::NotFound, why)),
        Err(FleetRefusal::Refused(why)) => return Err(fail(Code::BadInput, why)),
    };
    let detail = match detail_reason {
        Some(reason) => format!("queued for node {node} as {id}: {reason}"),
        None => format!("queued for node {node} as {id}"),
    };
    admin.audit(caller, name, Some(&target), None, Some(&detail)).await;
    Err(
        async_graphql::Error::new(format!("queued for node {node} as {id}")).extend_with(|_, e| {
            e.set("code", "QUEUED");
            e.set("id", id.clone());
            e.set("node", node.as_str());
        }),
    )
}

/// A timeout's length in seconds from `minutes` or `until`, as the local mutation reads them.
pub fn timeout_secs(minutes: Option<i64>, until: Option<&str>, now: i64) -> async_graphql::Result<u64> {
    let end = input::timeout_end(minutes, until, now).map_err(|why| fail(Code::BadInput, why))?;
    let secs = end.saturating_sub(now).div_euclid(1000).max(1);
    Ok(u64::try_from(secs).unwrap_or(1))
}
