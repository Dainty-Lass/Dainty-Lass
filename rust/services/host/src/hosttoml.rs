//! `host.toml`: the server's settings as the host reads and edits them (docs/config-toml-plan.md,
//! phase 1). It holds the same fields as the Node kit's `host.json`, with snake_case keys and no
//! `null`: a field that would be null is left out.
//!
//! Reading turns the file into the host.json document, so [`validate`](crate::hostconfig::validate)
//! checks both files the same way; [`message`] makes its errors name the TOML keys. Writing patches
//! the file as read: only a value that changed is rewritten, so the host's comments, order and
//! spacing stay. A new file (setup's first run, or a host.json being moved over) is written from a
//! template that says what each key does.
//!
//! `[modules]` (DF3) is the one part with lists and tables of its own (`hostconfig/modules.rs`): it is
//! read as it is and, when it changed, written back whole.

use serde_json::{Map, Number, Value};
use toml_edit::{DocumentMut, Item, Table, TableLike};

use crate::error::HostError;

/// A key: its name in host.json, its name in host.toml, and whether it may be null (left out).
type Field = (&'static str, &'static str, bool);

const TOP: &[Field] = &[
    ("version", "version", false),
    ("mode", "mode", false),
    ("edition", "edition", false),
    ("host", "host", false),
    ("gameAddress", "game_address", false),
    ("name", "name", false),
    ("gameDir", "game_dir", true),
    ("legacyGameDir", "legacy_game_dir", true),
    ("registration", "registration", false),
    ("shareUrl", "share_url", true),
    ("mongo", "mongo", false),
    ("maxHuntServers", "max_hunt_servers", false),
    ("multipliers", "multipliers", false),
    ("publicPort", "public_port", false),
    ("legacyPublicPort", "legacy_public_port", false),
    ("performance", "performance", false),
    ("linux", "linux", false),
    ("database", "database", false),
    ("guard", "guard", false),
    ("history", "history", false),
    ("switches", "switches", false),
    ("network", "network", false),
    ("modules", "modules", false),
];

/// The top-level key whose tables and lists are read as they are (`hostconfig/modules.rs` checks them).
const FREE_FORM: &str = "modules";

/// `network.nodes` is a list of tables (`[[network.nodes]]`), read as it is like `[modules]`
const NETWORK: &[Field] = &[
    ("node", "node", false),
    ("region", "region", false),
    ("leader", "leader", false),
    ("nodes", "nodes", false),
];

fn is_peer_list(prefix: &str, json: &str) -> bool {
    prefix == "network." && json == "nodes"
}

const MONGO: &[Field] = &[
    ("uri", "uri", false),
    ("manage", "manage", false),
    ("mongod", "mongod", true),
];

const MULTIPLIERS: &[Field] = &[("xp", "xp", false), ("rewards", "rewards", false)];

const PERFORMANCE: &[Field] = &[
    ("serverFps", "server_fps", false),
    ("idleFps", "idle_fps", false),
    ("trainingOnDemand", "training_on_demand", false),
    ("trainingIdleMinutes", "training_idle_minutes", false),
    ("verboseGameLogs", "verbose_game_logs", false),
    ("memoryGuard", "memory_guard", false),
    ("minFreeMemoryMb", "min_free_memory_mb", false),
    ("huntMemoryMb", "hunt_memory_mb", false),
];

const LINUX: &[Field] = &[
    ("wine", "wine", true),
    ("proton", "proton", true),
    ("xvfb", "xvfb", false),
    ("firewallInterface", "firewall_interface", true),
];

const DATABASE: &[Field] = &[
    ("engine", "engine", false),
    ("postgresHost", "postgres_host", false),
    ("postgresPort", "postgres_port", false),
    ("postgresUser", "postgres_user", false),
    ("postgresDatabase", "postgres_database", false),
    ("allowRemote", "allow_remote", false),
];

const GUARD: &[Field] = &[
    ("banLimit", "ban_limit", false),
    ("banWindowMinutes", "ban_window_minutes", false),
    ("banGuard", "ban_guard", false),
    ("revertLimit", "revert_limit", false),
    ("revertGuard", "revert_guard", false),
    ("revokeGuard", "revoke_guard", false),
    ("delayMinutes", "delay_minutes", false),
    ("expireHours", "expire_hours", false),
];

const HISTORY: &[Field] = &[("shipToPeer", "ship_to_peer", false)];

const SWITCHES: &[Field] = &[
    ("reforge", "reforge", false),
    ("masteryPersistence", "mastery_persistence", false),
    ("bountyPersistence", "bounty_persistence", false),
    ("cooldownPersistence", "cooldown_persistence", false),
    ("accountErasure", "account_erasure", false),
    ("logPurge", "log_purge", false),
    ("rateLimit", "rate_limit", false),
    ("rateLimitPerMinute", "rate_limit_per_minute", false),
    (
        "gameserverRateLimitPerMinute",
        "gameserver_rate_limit_per_minute",
        false,
    ),
    ("hostRateLimitPerMinute", "host_rate_limit_per_minute", false),
];

/// The keys of a table at the top of the file; `None` for a single value.
fn table_fields(json_key: &str) -> Option<&'static [Field]> {
    match json_key {
        "mongo" => Some(MONGO),
        "multipliers" => Some(MULTIPLIERS),
        "performance" => Some(PERFORMANCE),
        "linux" => Some(LINUX),
        "database" => Some(DATABASE),
        "guard" => Some(GUARD),
        "history" => Some(HISTORY),
        "switches" => Some(SWITCHES),
        "network" => Some(NETWORK),
        _ => None,
    }
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

/// `maxHuntServers` → `max_hunt_servers`.
fn snake_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

// ------------------------------------------------------------------ reading

/// The host.json document a host.toml stands for. Unknown keys are refused, naming the right one when
/// it is a host.json key (`maxHuntServers`: "did you mean max_hunt_servers?").
pub fn document(text: &str) -> Result<Value, HostError> {
    let doc: DocumentMut = text
        .parse()
        .map_err(|e| bad(format!("host.toml is not valid TOML: {e}")))?;
    Ok(Value::Object(object(doc.as_table(), TOP, "")?))
}

fn object(table: &dyn TableLike, fields: &[Field], prefix: &str) -> Result<Map<String, Value>, HostError> {
    for (key, _) in table.iter() {
        if !fields.iter().any(|f| f.1 == key) {
            return Err(unknown(key, fields, prefix));
        }
    }
    let mut out = Map::new();
    for &(json, toml, nullable) in fields {
        let path = format!("{prefix}{toml}");
        match table.get(toml) {
            Some(item) if prefix.is_empty() && json == FREE_FORM => {
                out.insert(json.into(), free_form(item, &path)?);
            }
            Some(item) if is_peer_list(prefix, json) => {
                out.insert(json.into(), free_form(item, &path)?);
            }
            Some(item) => {
                let sub = if prefix.is_empty() { table_fields(json) } else { None };
                out.insert(json.into(), value(item, sub, &path)?);
            }
            None if nullable => {
                out.insert(json.into(), Value::Null);
            }
            None => {}
        }
    }
    Ok(out)
}

fn unknown(key: &str, fields: &[Field], prefix: &str) -> HostError {
    let snake = snake_case(key);
    if let Some(f) = fields.iter().find(|f| f.1 == snake) {
        return bad(format!(
            "host.toml has no setting {prefix}{key}: did you mean {prefix}{}?",
            f.1
        ));
    }
    let known: Vec<String> = fields.iter().map(|f| format!("{prefix}{}", f.1)).collect();
    bad(format!(
        "host.toml has no setting {prefix}{key} (the settings there are {})",
        known.join(", ")
    ))
}

fn value(item: &Item, sub: Option<&[Field]>, path: &str) -> Result<Value, HostError> {
    if let Some(fields) = sub {
        let table = item
            .as_table_like()
            .ok_or_else(|| bad(format!("{path} must be a table ([{path}])")))?;
        return Ok(Value::Object(object(table, fields, &format!("{path}."))?));
    }
    let single = || bad(format!("{path} must be a single value, not a table or a list"));
    match item.as_value().ok_or_else(single)? {
        toml_edit::Value::String(s) => Ok(Value::String(s.value().clone())),
        toml_edit::Value::Integer(n) => Ok(Value::Number(Number::from(*n.value()))),
        toml_edit::Value::Float(n) => Number::from_f64(*n.value())
            .map(Value::Number)
            .ok_or_else(|| bad(format!("{path} must be a finite number"))),
        toml_edit::Value::Boolean(b) => Ok(Value::Bool(*b.value())),
        toml_edit::Value::Datetime(_) => Err(bad(format!("{path} must not be a date"))),
        toml_edit::Value::Array(_) | toml_edit::Value::InlineTable(_) => Err(single()),
    }
}

/// A part read as it is: tables, lists of tables and lists, with the scalars [`value`] takes.
fn free_form(item: &Item, path: &str) -> Result<Value, HostError> {
    match item {
        Item::Table(table) => table_object(table, path),
        Item::ArrayOfTables(tables) => tables
            .iter()
            .map(|table| table_object(table, path))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Item::Value(value) => free_value(value, path),
        Item::None => Ok(Value::Null),
    }
}

fn table_object(table: &dyn TableLike, path: &str) -> Result<Value, HostError> {
    let mut out = Map::new();
    for (key, item) in table.iter() {
        out.insert(key.to_owned(), free_form(item, &format!("{path}.{key}"))?);
    }
    Ok(Value::Object(out))
}

fn free_value(value: &toml_edit::Value, path: &str) -> Result<Value, HostError> {
    match value {
        toml_edit::Value::Array(items) => items
            .iter()
            .map(|item| free_value(item, path))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        toml_edit::Value::InlineTable(table) => table_object(table, path),
        scalar => self::value(&Item::Value(scalar.clone()), None, path),
    }
}

/// [`free_form`]'s part as TOML: an object of objects as `[a.b]` tables, a list of objects as `[[a.b]]`.
fn free_item(value: &Value, path: &str) -> Result<Item, HostError> {
    match value {
        Value::Object(map) => {
            let mut table = Table::new();
            for (key, inner) in map {
                table.insert(key, free_item(inner, &format!("{path}.{key}"))?);
            }
            // A table that holds only tables prints no header of its own
            if !table.is_empty()
                && table
                    .iter()
                    .all(|(_, item)| item.is_table() || item.is_array_of_tables())
            {
                table.set_implicit(true);
            }
            Ok(Item::Table(table))
        }
        Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_object) => {
            let mut tables = toml_edit::ArrayOfTables::new();
            for item in items {
                if let Item::Table(table) = free_item(item, path)? {
                    tables.push(table);
                }
            }
            Ok(Item::ArrayOfTables(tables))
        }
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(scalar(item, path)?);
            }
            Ok(Item::Value(toml_edit::Value::Array(array)))
        }
        other => Ok(Item::Value(scalar(other, path)?)),
    }
}

/// A message of [`validate`](crate::hostconfig::validate), with host.json's key names replaced by
/// host.toml's (`performance.serverFps` → `performance.server_fps`).
pub fn message(text: &str) -> String {
    let mut out = text.to_owned();
    for fields in [
        TOP,
        MONGO,
        MULTIPLIERS,
        PERFORMANCE,
        LINUX,
        DATABASE,
        GUARD,
        HISTORY,
        SWITCHES,
        NETWORK,
    ] {
        for &(json, toml, _) in fields {
            if json != toml {
                out = out.replace(json, toml);
            }
        }
    }
    out.replace("host.json", "host.toml")
}

// ------------------------------------------------------------------ writing

/// The text to write for `fields` (in [`HostConfig`](crate::hostconfig::HostConfig)'s host.json
/// form): `source`, the file as read, with only the values that differ from `read` replaced; or a new
/// file when there is no source.
pub fn text(fields: &Map<String, Value>, read: &Map<String, Value>, source: Option<&str>) -> Result<String, HostError> {
    let Some(source) = source else {
        return fresh(fields);
    };
    let mut doc: DocumentMut = source
        .parse()
        .map_err(|e| bad(format!("host.toml is not valid TOML: {e}")))?;
    patch(doc.as_table_mut(), fields, read, TOP, true)?;
    Ok(doc.to_string())
}

fn patch(
    table: &mut Table,
    new: &Map<String, Value>,
    old: &Map<String, Value>,
    fields: &[Field],
    top: bool,
) -> Result<(), HostError> {
    let empty = Map::new();
    for &(json, toml, _) in fields {
        let (n, o) = (new.get(json), old.get(json));
        if n == o {
            continue;
        }
        let sub = if top { table_fields(json) } else { None };
        match (n, sub) {
            (None | Some(Value::Null), _) => {
                table.remove(toml);
            }
            (Some(v), None) if top && json == FREE_FORM => {
                table.remove(toml);
                table.insert(toml, free_item(v, toml)?);
            }
            (Some(v), None) if !top && is_peer_list("network.", json) && toml == "nodes" => {
                table.remove(toml);
                table.insert(toml, free_item(v, "network.nodes")?);
            }
            (Some(Value::Object(values)), Some(sub)) => {
                let old_values = o.and_then(Value::as_object).unwrap_or(&empty);
                if let Some(Item::Table(existing)) = table.get_mut(toml) {
                    patch(existing, values, old_values, sub, false)?;
                } else {
                    // New, or written inline by hand: a [table] of its own from now on.
                    let mut fresh = Table::new();
                    patch(&mut fresh, values, &empty, sub, false)?;
                    table.insert(toml, Item::Table(fresh));
                }
            }
            (Some(v), _) => {
                let mut replacement = scalar(v, toml)?;
                if let Some(existing) = table.get_mut(toml).and_then(Item::as_value_mut) {
                    // In place, keeping the key, its comments and a comment after the value.
                    *replacement.decor_mut() = existing.decor().clone();
                    *existing = replacement;
                } else {
                    table.insert(toml, Item::Value(replacement));
                }
            }
        }
    }
    Ok(())
}

fn scalar(value: &Value, key: &str) -> Result<toml_edit::Value, HostError> {
    match value {
        Value::String(s) => Ok(s.as_str().into()),
        Value::Bool(b) => Ok((*b).into()),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Ok(i.into()),
            (None, Some(f)) => Ok(f.into()),
            (None, None) => Err(bad(format!("{key}: {n} is not a number host.toml can hold"))),
        },
        _ => Err(bad(format!("{key} must be a single value"))),
    }
}

const HEADER: &str = "\
# This server's settings (reforged-host). Edit them here, then run `reforged-host setup` again to
# apply them; setup keeps your comments and every value it does not change. A setting that is left
# out takes its default. The secrets are not here: they are in secrets.env, next to this file.
";

/// What each key does, written above it in a new file.
fn comment(path: &str) -> &'static str {
    match path {
        "version" => "The file's format: leave it at 1.",
        "mode" => "\"private\" (players reach it over Tailscale) or \"public\" (over the internet).",
        "edition" => {
            "\"reborn\" (Dauntless 1.14.7 on Mystic Paradox), \"legacy\" (Dauntless Revived's 1.4.4) or \"both\"
             (the two side by side, each on its own ports). Left out: reborn."
        }
        "host" => {
            "Private: this PC's Tailscale address or MagicDNS name. Public: the public IPv4 address or DNS name\n\
             players connect to. It goes into every invite."
        }
        "game_address" => "The IPv4 address the game servers announce for players' game traffic (UDP).",
        "name" => "The server's name, as players see it (up to 48 characters).",
        "game_dir" => {
            "The Dauntless folder the game servers run from (the one with Archon): 1.14.7, or 1.4.4 for legacy."
        }
        "registration" => {
            "Who may make an account: \"INVITECODE\" (with an invite code), \"OPEN\" (anyone) or \"NONE\" (nobody)."
        }
        "share_url" => "Private: the Tailscale share link that goes into invites.",
        "max_hunt_servers" => "How many hunts may run at once, 1 to 10: each one is a game server.",
        "public_port" => "Public: the gateway's TCP port when 443 is taken. It goes into every invite.",
        "legacy_game_dir" => "Both games: the Dauntless 1.4.4 folder its game servers run from (game_dir is 1.14.7's).",
        "legacy_public_port" => {
            "Both games, public: the 1.4.4 gateway's TCP port (4443 when left out). It goes into 1.4.4 invites."
        }
        "mongo" => "The database.",
        "mongo.manage" => "true: reforged-host starts mongod itself.",
        "mongo.mongod" => "The mongod program when reforged-host starts it.",
        "multipliers" => {
            "Private test servers only: multiply the XP and the loot the game grants, 1 to 100 (1 is off)."
        }
        "performance" => "How much CPU and RAM the game servers may use.",
        "performance.server_fps" => {
            "Frames per second of each game server while a player is connected: 10 to 240, or 0 for no cap."
        }
        "performance.idle_fps" => {
            "Frames per second after 15 s without a player: 1 to 240, or 0 for the same as server_fps."
        }
        "performance.training_on_demand" => {
            "true: the Training Grounds start the first time someone goes there, not at boot."
        }
        "performance.training_idle_minutes" => {
            "On demand: they stop again after this many minutes without a player (0 keeps them running)."
        }
        "performance.verbose_game_logs" => "true: upstream's diagnostic log levels; false: warnings and errors only.",
        "performance.memory_guard" => "true: a new hunt starts only when the machine has the memory for it.",
        "performance.min_free_memory_mb" => "MB left free after a new hunt starts.",
        "performance.hunt_memory_mb" => "MB one hunt needs (at least 256).",
        "linux" => "Linux only: how the Windows game servers run, and what the allowlist filters.",
        "linux.wine" => "The wine program. Left out: the first wine on PATH.",
        "linux.proton" => "A Proton (or GE-Proton) script to use instead of Wine.",
        "linux.xvfb" => "true: a virtual X display (xvfb-run) for the game servers when there is no DISPLAY.",
        "linux.firewall_interface" => {
            "Public: the network interface the allowlist filters, like eth0. Left out: every interface."
        }
        "database" => "The database of the Rust services (the 1.4.4 metagame keeps its own SQLite file).",
        "database.engine" => {
            "\"sqlite\" (one file in this folder, nothing to install) or \"postgres\" (a server you run). Left out: sqlite."
        }
        "database.postgres_host" => {
            "Postgres: the server's address (127.0.0.1 when left out). Connections are not encrypted, so it must\n\
             be this machine unless allow_remote is true."
        }
        "database.postgres_port" => "Postgres: its TCP port (5432 when left out).",
        "database.postgres_user" => {
            "Postgres: the role to log in as (dainty when left out). Its password is REFORGED_DB_PASSWORD in secrets.env."
        }
        "database.postgres_database" => "Postgres: the database (dainty when left out).",
        "guard" => {
            "Destructive and bulk admin actions wait before they run (DH6). A rule is \"delay\" (wait delay_minutes),
             \"approval\" (a second operator approves) or \"off\". Left out: the defaults shown below."
        }
        "guard.ban_limit" => "Bans and timeouts one admin key may make in the window before the next is held (10).",
        "guard.ban_window_minutes" => "The window, in minutes (60).",
        "guard.ban_guard" => "What a ban or timeout over the limit waits for (delay).",
        "guard.revert_limit" => "A history revert of more changes than this is held (100).",
        "guard.revert_guard" => "What a bigger revert waits for (delay).",
        "guard.revoke_guard" => "What revoking an admin or owner key waits for (delay). Other keys go at once.",
        "guard.delay_minutes" => "How long a delay waits, in minutes (10).",
        "guard.expire_hours" => "How long an action nobody approved stays before it expires (48).",
        "history" => "Off-server change log and snapshot shipping (DD23, DH7). Left out: off.",
        "history.ship_to_peer" => {
            "A peer node id in [[network.nodes]] that receives this node's change log and snapshots over Service::LogShip (DH7)."
        }
        "switches" => "Backend persistence and rule switches (left out: each version's own defaults).",
        "switches.reforge" => "Weapon reforge rules (MYSTICPARADOX_REFORGE): \"on\", \"log\" or \"off\".",
        "switches.mastery_persistence" => {
            "Whether a game server's mastery report is saved (MYSTICPARADOX_MASTERY_PERSISTENCE): \"on\" or \"off\"."
        }
        "switches.bounty_persistence" => {
            "Whether bounty saves are stored (MYSTICPARADOX_BOUNTY_PERSISTENCE): \"on\" or \"off\" (\"off\" is 1.12.0's stub)."
        }
        "switches.cooldown_persistence" => {
            "Whether cooldown saves are stored (MYSTICPARADOX_COOLDOWN_PERSISTENCE): \"on\" or \"off\" (\"off\" is 1.12.0's stub)."
        }
        "switches.account_erasure" => {
            "Whether the background sweep erases accounts past their deletion grace period (MYSTICPARADOX_ACCOUNT_ERASURE): \"on\" or \"off\"."
        }
        "switches.log_purge" => {
            "Whether the background sweep purges expired verification codes and activity logs (MYSTICPARADOX_LOG_PURGE): \"on\" or \"off\"."
        }
        "switches.rate_limit" => {
            "Whether the 1.14.7 per-minute HTTP request budget is active (MYSTICPARADOX_RATE_LIMIT): \"on\" or \"off\"."
        }
        "switches.rate_limit_per_minute" => {
            "Per-minute HTTP request budget for a client address (MYSTICPARADOX_RATE_LIMIT_PER_MINUTE, 600 when left out)."
        }
        "switches.gameserver_rate_limit_per_minute" => {
            "Per-minute HTTP request budget for a dedicated game server address (MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE, 6000 when left out)."
        }
        "switches.host_rate_limit_per_minute" => {
            "Per-minute HTTP request budget for a player-hosted hunt capability (MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE, 6000 when left out)."
        }
        "network" => "The network this server is a node of (DD16). Left out: none.",
        "network.node" => "This node's name in the network, such as oregon-1.",
        "network.region" => "The region it serves, such as oregon.",
        "network.leader" => {
            "The directory leader node id (DN4). Left out: the alphabetically first node id across this node and [[network.nodes]]."
        }
        "network.nodes" => {
            "The other nodes of the network, one [[network.nodes]] each (DN3): id, region, host, port and optionally
\n             fingerprint. Only these nodes may call this one. Left out: no peers."
        }
        "modules" => {
            "The game modules and plugins to load (DF3). compiled lists the compiled-in ones (left out: all of them);\n\
             each [[modules.external]] is an executable with its SHA-256 (id, path, sha256, args, autoload); a\n\
             module's own settings go in [modules.<id>.<section>]. Left out: every compiled-in module, no other."
        }
        "database.allow_remote" => {
            "true: accept a Postgres server on another machine, over a network you trust (private, not the internet)."
        }
        _ => "",
    }
}

fn push_comment(out: &mut String, path: &str) {
    for line in comment(path).lines() {
        out.push_str("# ");
        out.push_str(line.trim_start());
        out.push('\n');
    }
}

/// A new host.toml: the header, then each value under what it does. The single values come first and
/// the tables after them, as TOML needs.
fn fresh(fields: &Map<String, Value>) -> Result<String, HostError> {
    let mut out = String::from(HEADER);
    for &(json, toml, _) in TOP {
        if table_fields(json).is_some() || json == FREE_FORM {
            continue;
        }
        match fields.get(json) {
            None | Some(Value::Null) => {}
            Some(v) => {
                out.push('\n');
                push_comment(&mut out, toml);
                out.push_str(&format!("{toml} = {}\n", scalar(v, toml)?.to_string().trim()));
            }
        }
    }
    for &(json, toml, _) in TOP {
        let (Some(sub), Some(Value::Object(values))) = (table_fields(json), fields.get(json)) else {
            continue;
        };
        out.push('\n');
        push_comment(&mut out, toml);
        out.push_str(&format!("[{toml}]\n"));
        for &(sub_json, sub_toml, _) in sub {
            if is_peer_list(&format!("{toml}."), sub_json) {
                continue;
            }
            match values.get(sub_json) {
                None | Some(Value::Null) => {}
                Some(v) => {
                    push_comment(&mut out, &format!("{toml}.{sub_toml}"));
                    out.push_str(&format!("{sub_toml} = {}\n", scalar(v, sub_toml)?.to_string().trim()));
                }
            }
        }
        if let Some(nodes) = values.get("nodes").filter(|v| !v.is_null()) {
            let mut doc = DocumentMut::new();
            let mut parent = Table::new();
            parent.insert("nodes", free_item(nodes, "network.nodes")?);
            parent.set_implicit(true);
            doc.insert(toml, Item::Table(parent));
            out.push('\n');
            push_comment(&mut out, &format!("{toml}.nodes"));
            out.push_str(&doc.to_string());
        }
    }
    if let Some(modules) = fields.get(FREE_FORM).filter(|v| !v.is_null()) {
        let mut doc = DocumentMut::new();
        doc.insert(FREE_FORM, free_item(modules, FREE_FORM)?);
        out.push('\n');
        push_comment(&mut out, FREE_FORM);
        out.push_str(&doc.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn snake_case_of_the_host_json_keys() {
        assert_eq!(snake_case("maxHuntServers"), "max_hunt_servers");
        assert_eq!(snake_case("version"), "version");
        for fields in [
            TOP,
            MONGO,
            MULTIPLIERS,
            PERFORMANCE,
            LINUX,
            DATABASE,
            GUARD,
            HISTORY,
            SWITCHES,
            NETWORK,
        ] {
            for &(json, toml, _) in fields {
                assert_eq!(snake_case(json), toml);
            }
        }
    }

    #[test]
    fn a_left_out_nullable_key_reads_as_null() {
        let doc = document("mode = \"private\"\n[mongo]\nuri = \"mongodb://x\"\n[linux]\nxvfb = true\n").unwrap();
        assert_eq!(doc["gameDir"], Value::Null);
        assert_eq!(doc["shareUrl"], Value::Null);
        assert_eq!(doc["mongo"]["mongod"], Value::Null);
        assert_eq!(doc["linux"]["wine"], Value::Null);
        assert_eq!(doc["linux"]["firewallInterface"], Value::Null);
        assert!(doc.get("publicPort").is_none(), "not nullable: stays absent");
    }

    #[test]
    fn unknown_keys_are_refused_with_the_right_name() {
        let e = document("maxHuntServers = 3\n").unwrap_err().to_string();
        assert!(e.contains("did you mean max_hunt_servers?"), "{e}");
        let e = document("[performance]\nserverFps = 30\n").unwrap_err().to_string();
        assert!(e.contains("did you mean performance.server_fps?"), "{e}");
        let e = document("colour = \"red\"\n").unwrap_err().to_string();
        assert!(e.contains("no setting colour") && e.contains("max_hunt_servers"), "{e}");
    }

    #[test]
    fn wrong_shapes_are_refused() {
        assert!(
            document("mongo = 3\n")
                .unwrap_err()
                .to_string()
                .contains("mongo must be a table")
        );
        assert!(
            document("name = [1]\n")
                .unwrap_err()
                .to_string()
                .contains("single value")
        );
        assert!(
            document("name = 1979-05-27\n")
                .unwrap_err()
                .to_string()
                .contains("date")
        );
        assert!(
            document("name = \n")
                .unwrap_err()
                .to_string()
                .contains("not valid TOML")
        );
    }

    #[test]
    fn validation_messages_name_the_toml_keys() {
        assert_eq!(
            message("performance.idleFps must be 0 (same as serverFps), in host.json"),
            "performance.idle_fps must be 0 (same as server_fps), in host.toml"
        );
    }

    #[test]
    fn a_patch_changes_only_what_changed() {
        let source = "# mine\nname = \"Friday\" # the name\n\n# hunts\nmax_hunt_servers = 3\n";
        let read = json!({ "name": "Friday", "maxHuntServers": 3 });
        let new = json!({ "name": "Friday", "maxHuntServers": 5, "publicPort": 4443 });
        let out = text(new.as_object().unwrap(), read.as_object().unwrap(), Some(source)).unwrap();
        assert_eq!(
            out,
            "# mine\nname = \"Friday\" # the name\n\n# hunts\nmax_hunt_servers = 5\npublic_port = 4443\n"
        );
    }
}
