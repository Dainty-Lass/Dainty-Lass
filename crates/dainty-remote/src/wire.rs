// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Wire messages for enrollment (`POST /__node/enroll`) and mTLS remote management
//! (`POST /__node/remote`, `docs/remote-management.md` §§2–6, `docs/security/remote-management.md`).

use std::collections::BTreeMap;

use dainty_module_api::network::{NodeId, Region};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RemoteError;
use crate::enroll::{is_valid_sha256_fp, validate_controller_url, validate_enrollment_token, validate_label};

/// Maximum wire payload size (`1 MiB`, matching `dainty_network::link::MAX_MESSAGE`).
pub const MAX_REMOTE_WIRE: usize = 1 << 20;

/// Maximum log entries returned in a single page.
pub const MAX_LOG_ENTRIES: usize = 500;

/// Maximum length of a public key / certificate PEM carried during enrollment.
pub const MAX_PEM_LEN: usize = 16 * 1024;

/// Secret patterns that must never cross the remote-management wire in either direction
/// (`docs/remote-management.md` §6).
pub const FORBIDDEN_SECRET_PATTERNS: &[&str] = &[
    "REFORGED_ADMIN_API_KEY",
    "JWT_SECRET",
    "SESSION_SECRET",
    "TURN_STATIC_AUTH_SECRET",
    "AUTH_SIGNING_PRIVKEY_B64",
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN EC PRIVATE KEY-----",
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "SQLite format 3",
    "$2a$",
    "$2b$",
    "$2y$",
    "$argon2",
];

fn is_key_body_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Returns `true` if `text` contains a token shaped like a player account key (`RRK_...`) or an
/// admin secret key (`RRA_...`).
pub fn contains_prefixed_key_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 8 {
        return false;
    }
    for i in 0..=bytes.len().saturating_sub(8) {
        let p = &bytes[i..i + 4];
        let is_rrk = p[0].eq_ignore_ascii_case(&b'r')
            && p[1].eq_ignore_ascii_case(&b'r')
            && (p[2].eq_ignore_ascii_case(&b'k') || p[2].eq_ignore_ascii_case(&b'a'))
            && p[3] == b'_';
        if !is_rrk {
            continue;
        }
        if i > 0 && is_key_body_char(bytes[i - 1]) {
            continue;
        }
        let body_len = bytes[i + 4..].iter().take_while(|&&b| is_key_body_char(b)).count();
        if body_len >= 4 {
            return true;
        }
    }
    false
}

/// Returns `true` if `text` contains any secret material that is forbidden on the remote wire
/// (`secrets.env` keys, `RRK_` account keys, `RRA_` admin keys, PEM private keys, password hashes,
/// or SQLite database headers).
pub fn contains_forbidden_secret_str(text: &str) -> bool {
    if contains_prefixed_key_token(text) {
        return true;
    }
    let upper = text.to_ascii_uppercase();
    for pat in FORBIDDEN_SECRET_PATTERNS {
        if upper.contains(&pat.to_ascii_uppercase()) {
            return true;
        }
    }
    false
}

/// Recursively inspect a `serde_json::Value` for any forbidden secret string in keys or values.
pub fn contains_forbidden_secret_value(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
        Value::String(s) => contains_forbidden_secret_str(s),
        Value::Array(items) => items.iter().any(contains_forbidden_secret_value),
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| contains_forbidden_secret_str(k) || contains_forbidden_secret_value(v)),
    }
}

/// Inspect raw wire bytes for SQLite magic headers or forbidden secret substrings.
pub fn contains_forbidden_secret_bytes(bytes: &[u8]) -> bool {
    if bytes.windows(15).any(|w| w == b"SQLite format 3") {
        return true;
    }
    let lossy = String::from_utf8_lossy(bytes);
    contains_forbidden_secret_str(&lossy)
}

/// Request sent by a managed install to `POST /__node/enroll` on the controller's gateway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollRequest {
    /// The one-time token from the invite.
    pub token: String,
    /// Node id the invite was minted for.
    pub node: NodeId,
    /// Region the invite was minted for.
    pub region: Region,
    /// Display label; must match the invite's when both are set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The install's new public key, PEM, for the controller to sign a node certificate for.
    pub public_key_pem: String,
    /// The install's gateway URL, if it has a public one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
    /// SHA-256 of the install's gateway certificate, 64 lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_cert_fp: Option<String>,
}

impl EnrollRequest {
    /// Checks the token, label, key and gateway fields; refuses secrets.
    pub fn validate(&self) -> Result<(), RemoteError> {
        let _ = validate_enrollment_token(&self.token)?;
        if let Some(label) = &self.label {
            let checked = validate_label(label)?;
            if &checked != label {
                return Err(RemoteError::InvalidInput(
                    "label must not have leading or trailing whitespace".to_string(),
                ));
            }
        }
        if self.public_key_pem.is_empty() || self.public_key_pem.len() > MAX_PEM_LEN {
            return Err(RemoteError::InvalidInput(
                "public_key_pem is empty or exceeds 16 KiB".to_string(),
            ));
        }
        if contains_forbidden_secret_str(&self.public_key_pem) {
            return Err(RemoteError::SecretLeakRefused(
                "enrollment request must never carry a private key or secret".to_string(),
            ));
        }
        if !self.public_key_pem.contains("-----BEGIN PUBLIC KEY-----") {
            return Err(RemoteError::InvalidInput(
                "public_key_pem must be a SubjectPublicKeyInfo PEM block".to_string(),
            ));
        }
        if let Some(url) = &self.gateway_url {
            let _ = validate_controller_url(url)?;
        }
        if let Some(fp) = &self.gateway_cert_fp
            && !is_valid_sha256_fp(fp)
        {
            return Err(RemoteError::InvalidInput(
                "gateway_cert_fp must be 64 lowercase hex characters".to_string(),
            ));
        }
        Ok(())
    }
}

/// Answer returned by the controller from `POST /__node/enroll`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollAnswer {
    /// The controller's node id.
    pub controller_node: NodeId,
    /// The controller's region.
    pub controller_region: Region,
    /// SHA-256 of the controller's certificate.
    pub controller_cert_fp: String,
    /// The network CA certificate, PEM.
    pub ca_cert_pem: String,
    /// The install's signed node certificate, PEM.
    pub node_cert_pem: String,
    /// Config revision to start from.
    pub initial_revision: u64,
}

impl EnrollAnswer {
    /// Checks the fingerprint and both certificates; refuses secrets.
    pub fn validate(&self) -> Result<(), RemoteError> {
        if !is_valid_sha256_fp(&self.controller_cert_fp) {
            return Err(RemoteError::InvalidInput(
                "controller_cert_fp must be 64 lowercase hex characters".to_string(),
            ));
        }
        if self.ca_cert_pem.is_empty()
            || self.ca_cert_pem.len() > MAX_PEM_LEN
            || !self.ca_cert_pem.contains("-----BEGIN CERTIFICATE-----")
        {
            return Err(RemoteError::InvalidInput(
                "ca_cert_pem must be a valid certificate PEM".to_string(),
            ));
        }
        if self.node_cert_pem.is_empty()
            || self.node_cert_pem.len() > MAX_PEM_LEN
            || !self.node_cert_pem.contains("-----BEGIN CERTIFICATE-----")
        {
            return Err(RemoteError::InvalidInput(
                "node_cert_pem must be a valid certificate PEM".to_string(),
            ));
        }
        if contains_forbidden_secret_str(&self.ca_cert_pem) || contains_forbidden_secret_str(&self.node_cert_pem) {
            return Err(RemoteError::SecretLeakRefused(
                "enrollment answer must never carry a private key or secret".to_string(),
            ));
        }
        Ok(())
    }
}

/// Role of the operator issuing a remote command (`DH2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteRole {
    /// Reads status, history and doctor results.
    Viewer,
    /// Moderates players as well.
    Moderator,
    /// Runs lifecycle, module, backup and database commands as well.
    Admin,
    /// Everything, including updates, fresh starts, real reverts and detaching.
    Owner,
}

impl RemoteRole {
    /// Order of authority, 1 for viewer to 4 for owner.
    pub fn rank(self) -> u8 {
        match self {
            Self::Viewer => 1,
            Self::Moderator => 2,
            Self::Admin => 3,
            Self::Owner => 4,
        }
    }

    /// Whether this role may do what `required` may.
    pub fn allows(self, required: RemoteRole) -> bool {
        self.rank() >= required.rank()
    }
}

/// Authenticated operator identity attached to every remote command (`DD23`, `DH2`).
/// Never carries the raw admin key secret (`RRA_...`); carries only the key's identifier (`key_id`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorEnvelope {
    /// The controller the operator acts through.
    pub controller_node: NodeId,
    /// Admin key's id, never its secret.
    pub key_id: String,
    /// The operator's role.
    pub role: RemoteRole,
    /// When the envelope was made, in Unix milliseconds.
    pub issued_at_ms: i64,
}

impl OperatorEnvelope {
    /// Checks the key id is a plain id, not `system`, and holds no secret.
    pub fn validate(&self) -> Result<(), RemoteError> {
        let trimmed = self.key_id.trim();
        if trimmed.is_empty() || trimmed.len() > 64 || trimmed != self.key_id {
            return Err(RemoteError::InvalidInput(
                "operator key_id must be 1..=64 trimmed characters".to_string(),
            ));
        }
        if trimmed.eq_ignore_ascii_case("system") {
            return Err(RemoteError::InvalidInput(
                "operator key_id cannot be 'system' (DD23 requires a named operator)".to_string(),
            ));
        }
        if contains_forbidden_secret_str(&self.key_id)
            || !self
                .key_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        {
            return Err(RemoteError::SecretLeakRefused(
                "operator key_id must be an identifier, never a secret key".to_string(),
            ));
        }
        Ok(())
    }

    /// Canonical `store::history` actor string (`DD23`): `"remote:<controller_node>:<key_id>"`.
    pub fn actor_string(&self) -> String {
        format!("remote:{}:{}", self.controller_node, self.key_id)
    }
}

/// What a lifecycle command does to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAction {
    /// Start it.
    Start,
    /// Stop it.
    Stop,
    /// Stop and start it.
    Restart,
}

/// A moderation command's action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminModerationKind {
    /// Disconnect the player.
    Kick,
    /// Mute the player for a while.
    Timeout,
    /// End the player's timeout.
    LiftTimeout,
    /// Ban the player.
    Ban,
    /// Lift the player's ban.
    Unban,
    /// Read the player's moderation record.
    Lookup,
}

/// What a module command does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModuleControlKind {
    /// Load the module.
    Load,
    /// Unload the module.
    Unload,
    /// Replace the module's settings.
    SetSettings {
        /// The new settings by name.
        settings: BTreeMap<String, Value>,
    },
}

/// A database command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbSubcommand {
    /// Apply pending migrations.
    Migrate,
    /// Check the database.
    Verify,
    /// Export the store as Mongo collections.
    ExportMongo,
}

/// Operator command dispatched from the controller to one or more managed installs (`docs/remote-management.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteCommand {
    /// Start, stop or restart the server.
    Lifecycle {
        /// Start, stop or restart.
        action: LifecycleAction,
    },
    /// Run `setup` again.
    Setup {
        /// Install the example files.
        examples: bool,
        /// Discard the existing data and start over (guarded).
        fresh_start: bool,
    },
    /// A moderation action on a player.
    AdminAction {
        /// What to do.
        action: AdminModerationKind,
        /// The player, at most 128 characters.
        target: String,
        /// Why, at most 512 characters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// How long, for a timeout or ban.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_secs: Option<u64>,
    },
    /// Load, unload or configure a module.
    ModuleControl {
        /// Id of the module.
        module_id: String,
        /// What to do.
        action: ModuleControlKind,
    },
    /// Read the change history.
    HistoryQuery {
        /// Only changes by this actor.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
        /// Only changes of this kind.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        /// Most entries returned.
        limit: u32,
    },
    /// Verify the change log's hash chain.
    HistoryVerify,
    /// Revert the history back to an entry.
    HistoryRevert {
        /// Change-log entry to revert to.
        target_seq: i64,
        /// Plan only; a real revert is guarded.
        dry_run: bool,
        /// Why, for the log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Back up the database.
    Backup,
    /// Update the server.
    Update {
        /// Only report whether an update is available.
        check_only: bool,
        /// Version to install; the latest when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// A database command.
    Db {
        /// The database command to run.
        subcommand: DbSubcommand,
    },
    /// Run `doctor`.
    Doctor,
    /// Detach the install from the controller for good.
    Detached {
        /// Why, for the log.
        reason: String,
    },
}

impl RemoteCommand {
    /// Minimum `DH2` role required to execute this command.
    pub fn required_role(&self) -> RemoteRole {
        match self {
            Self::HistoryQuery { .. } | Self::HistoryVerify | Self::Doctor => RemoteRole::Viewer,
            Self::AdminAction {
                action: AdminModerationKind::Lookup,
                ..
            } => RemoteRole::Viewer,
            Self::AdminAction { .. } => RemoteRole::Moderator,
            Self::Lifecycle { .. }
            | Self::Setup { fresh_start: false, .. }
            | Self::ModuleControl { .. }
            | Self::HistoryRevert { dry_run: true, .. }
            | Self::Backup
            | Self::Db { .. } => RemoteRole::Admin,
            Self::Setup { fresh_start: true, .. }
            | Self::HistoryRevert { dry_run: false, .. }
            | Self::Update { .. }
            | Self::Detached { .. } => RemoteRole::Owner,
        }
    }

    /// Returns `true` if this command is a destructive or bulk action subject to `DH6` guarded
    /// two-person approval and cooling-off delay (`docs/remote-management.md` §4.1).
    pub fn is_guarded(&self) -> bool {
        matches!(
            self,
            Self::Setup { fresh_start: true, .. } | Self::HistoryRevert { dry_run: false, .. }
        )
    }

    /// Checks targets, reasons and arguments are in range and hold no secrets.
    pub fn validate(&self) -> Result<(), RemoteError> {
        match self {
            Self::AdminAction {
                target,
                reason,
                duration_secs,
                ..
            } => {
                if target.trim().is_empty() || target.len() > 128 {
                    return Err(RemoteError::InvalidInput(
                        "admin target must be 1..=128 characters".to_string(),
                    ));
                }
                if contains_forbidden_secret_str(target) {
                    return Err(RemoteError::SecretLeakRefused(
                        "admin target must not contain a secret key".to_string(),
                    ));
                }
                if let Some(r) = reason {
                    if r.len() > 512 {
                        return Err(RemoteError::InvalidInput(
                            "admin reason exceeds 512 characters".to_string(),
                        ));
                    }
                    if contains_forbidden_secret_str(r) {
                        return Err(RemoteError::SecretLeakRefused(
                            "admin reason must not contain a secret".to_string(),
                        ));
                    }
                }
                if let Some(d) = duration_secs
                    && (*d == 0 || *d > 365 * 86_400)
                {
                    return Err(RemoteError::InvalidInput(
                        "timeout duration must be between 1s and 365 days".to_string(),
                    ));
                }
            }
            Self::ModuleControl { module_id, action } => {
                if module_id.trim().is_empty() || module_id.len() > 64 {
                    return Err(RemoteError::InvalidInput(
                        "module_id must be 1..=64 characters".to_string(),
                    ));
                }
                if let ModuleControlKind::SetSettings { settings } = action {
                    for (k, v) in settings {
                        if contains_forbidden_secret_str(k) || contains_forbidden_secret_value(v) {
                            return Err(RemoteError::SecretLeakRefused(
                                "module settings must not contain forbidden secrets".to_string(),
                            ));
                        }
                    }
                }
            }
            Self::HistoryQuery { actor, kind, limit } => {
                if *limit == 0 || *limit > 500 {
                    return Err(RemoteError::InvalidInput(
                        "history query limit must be 1..=500".to_string(),
                    ));
                }
                if let Some(a) = actor
                    && (a.len() > 128 || contains_forbidden_secret_str(a))
                {
                    return Err(RemoteError::InvalidInput("invalid history actor filter".to_string()));
                }
                if let Some(k) = kind
                    && (k.len() > 64 || contains_forbidden_secret_str(k))
                {
                    return Err(RemoteError::InvalidInput("invalid history kind filter".to_string()));
                }
            }
            Self::HistoryRevert { target_seq, reason, .. } => {
                if *target_seq < 0 {
                    return Err(RemoteError::InvalidInput(
                        "history revert target_seq must be non-negative".to_string(),
                    ));
                }
                if let Some(r) = reason
                    && (r.len() > 512 || contains_forbidden_secret_str(r))
                {
                    return Err(RemoteError::InvalidInput("invalid revert reason".to_string()));
                }
            }
            Self::Update { version, .. } => {
                if let Some(v) = version
                    && (v.trim().is_empty() || v.len() > 64 || contains_forbidden_secret_str(v))
                {
                    return Err(RemoteError::InvalidInput("invalid update version".to_string()));
                }
            }
            Self::Detached { reason } => {
                if reason.len() > 512 || contains_forbidden_secret_str(reason) {
                    return Err(RemoteError::InvalidInput("invalid detached reason".to_string()));
                }
            }
            Self::Lifecycle { .. }
            | Self::Setup { .. }
            | Self::HistoryVerify
            | Self::Backup
            | Self::Db { .. }
            | Self::Doctor => {}
        }
        Ok(())
    }
}

/// Command envelope queued or pushed to a managed install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedRemoteCommand {
    /// Command id, 1 to 64 letters, digits, `-` or `_`.
    pub command_id: String,
    /// Who asked for it.
    pub operator: OperatorEnvelope,
    /// What to run.
    pub command: RemoteCommand,
    /// The second operator, for a guarded command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<OperatorEnvelope>,
}

impl QueuedRemoteCommand {
    /// Checks the id, the operator's role, and the approval a guarded command needs.
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.command_id.trim().is_empty()
            || self.command_id.len() > 64
            || !self
                .command_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(RemoteError::InvalidInput(
                "command_id must be 1..=64 ASCII alphanumeric, '-', or '_' characters".to_string(),
            ));
        }
        self.operator.validate()?;
        self.command.validate()?;
        if !self.operator.role.allows(self.command.required_role()) {
            return Err(RemoteError::PermissionDenied(format!(
                "role {:?} is not allowed to run command requiring {:?}",
                self.operator.role,
                self.command.required_role()
            )));
        }
        if let Some(approver) = &self.approved_by {
            approver.validate()?;
            if approver.key_id == self.operator.key_id {
                return Err(RemoteError::GuardedApprovalRequired(
                    "second approver must have a distinct key_id from the requester (DH6)".to_string(),
                ));
            }
            if !approver.role.allows(RemoteRole::Admin) {
                return Err(RemoteError::PermissionDenied(
                    "second approver must hold at least Admin role (DH6)".to_string(),
                ));
            }
        } else if self.command.is_guarded() {
            return Err(RemoteError::GuardedApprovalRequired(
                "guarded remote command requires a second operator approval before execution (DH6)".to_string(),
            ));
        }
        Ok(())
    }
}

/// Execution result reported back by a managed install for a `QueuedRemoteCommand`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome {
    /// The command it answers.
    pub command_id: String,
    /// Whether it succeeded.
    pub ok: bool,
    /// Machine-readable result code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// What happened, for the operator.
    pub message: String,
    /// Command-specific result data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl CommandOutcome {
    /// Checks the fields are in range and hold no secrets.
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.command_id.trim().is_empty() || self.command_id.len() > 64 {
            return Err(RemoteError::InvalidInput("invalid command_id in outcome".to_string()));
        }
        if self.message.len() > 4096 || contains_forbidden_secret_str(&self.message) {
            return Err(RemoteError::SecretLeakRefused(
                "command outcome message exceeds limit or contains a forbidden secret".to_string(),
            ));
        }
        if let Some(data) = &self.data
            && contains_forbidden_secret_value(data)
        {
            return Err(RemoteError::SecretLeakRefused(
                "command outcome data must never contain secrets, account keys, or raw database bytes".to_string(),
            ));
        }
        Ok(())
    }
}

/// How the server is running, as the install reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerRunState {
    /// Running normally.
    Running,
    /// Starting up.
    Starting,
    /// Shutting down.
    Stopping,
    /// Not running.
    Stopped,
    /// Running with a component down.
    Degraded,
}

/// One component's health in a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentHealth {
    /// Component name.
    pub id: String,
    /// Its phase, as text.
    pub state: String,
    /// Port it listens on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Status text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One doctor check's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoctorCheckEntry {
    /// Check id.
    pub id: String,
    /// `pass`, `warn` or `fail`.
    pub status: String,
    /// What it found.
    pub summary: String,
    /// More detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The install's last doctor run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoctorSummary {
    /// Whether nothing failed.
    pub ok: bool,
    /// Checks that passed.
    pub pass_count: u32,
    /// Checks that warned.
    pub warn_count: u32,
    /// Checks that failed.
    pub fail_count: u32,
    /// When it ran, in Unix milliseconds.
    pub checked_at_ms: i64,
    /// Each check, at most 128.
    #[serde(default)]
    pub checks: Vec<DoctorCheckEntry>,
}

/// Whether the install's legacy Mongo data was brought over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MongoMigrationState {
    /// There was none to bring over.
    NotNeeded,
    /// There is data still to migrate.
    Unmigrated,
    /// It was migrated.
    Migrated,
    /// The host chose a fresh start instead.
    Skipped,
}

/// The install's change log, for the dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistorySummary {
    /// Id of the newest entry.
    pub head_seq: i64,
    /// Hash of the newest entry.
    pub head_hash: String,
    /// Whether the hash chain last verified.
    pub chain_verified: bool,
    /// When the newest change was made, in Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change_at_ms: Option<i64>,
    /// Who made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_actor: Option<String>,
    /// Guarded actions waiting on the install.
    pub pending_guarded_count: u32,
    /// How far behind log shipping is, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship_lag_secs: Option<u64>,
}

/// Telemetry and status report sent by a managed install to its controller (`docs/remote-management.md` §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallReport {
    /// The install's node id.
    pub node: NodeId,
    /// Its region.
    pub region: Region,
    /// Display label.
    pub label: String,
    /// Server version.
    pub version: String,
    /// Operating system.
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Config revision applied.
    pub applied_revision: u64,
    /// How the server is running.
    pub server_state: ServerRunState,
    /// Seconds since the server started.
    pub uptime_secs: u64,
    /// Players online.
    pub players_online: u32,
    /// Hunts running.
    pub active_hunts: u32,
    /// Hubs running.
    pub active_hubs: u32,
    /// Realms running.
    pub active_realms: u32,
    /// Each component's health.
    #[serde(default)]
    pub components: Vec<ComponentHealth>,
    /// The last doctor run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doctor: Option<DoctorSummary>,
    /// Legacy Mongo data state.
    pub mongo_migration: MongoMigrationState,
    /// Change-log summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<HistorySummary>,
}

impl InstallReport {
    /// Checks the label and sizes, and that nothing in it is a secret.
    pub fn validate(&self) -> Result<(), RemoteError> {
        let _ = validate_label(&self.label)?;
        if self.components.len() > 64 {
            return Err(RemoteError::InvalidInput(
                "too many component health entries".to_string(),
            ));
        }
        if let Some(doc) = &self.doctor
            && doc.checks.len() > 128
        {
            return Err(RemoteError::InvalidInput("too many doctor check entries".to_string()));
        }
        let val =
            serde_json::to_value(self).map_err(|e| RemoteError::InvalidInput(format!("serialization error: {e}")))?;
        if contains_forbidden_secret_value(&val) {
            return Err(RemoteError::SecretLeakRefused(
                "install report must never contain secrets or account keys".to_string(),
            ));
        }
        Ok(())
    }
}

/// An invite code every managed install accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedInvite {
    /// The invite code.
    pub code: String,
    /// Account it is reserved for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub for_user: Option<String>,
    /// Registrations left; unlimited when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses_remaining: Option<u32>,
    /// When it stops working, in Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    /// Whether it was withdrawn.
    pub revoked: bool,
}

/// Authoritative shared settings and artifact hashes synced from the controller to managed installs
/// (`docs/remote-management.md` §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedConfigBundle {
    /// Revision number, rising with every change.
    pub revision: u64,
    /// The `host.toml` settings every install shares.
    pub shared_host_toml: String,
    /// Extra `host.toml` text per node id.
    #[serde(default)]
    pub node_overrides: BTreeMap<String, String>,
    /// Invite codes every install accepts.
    #[serde(default)]
    pub invites: Vec<SharedInvite>,
    /// Server presets by name, as JSON.
    #[serde(default)]
    pub presets: BTreeMap<String, String>,
    /// SHA-256 of the runtime manifest installs must serve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_manifest_sha256: Option<String>,
    /// SHA-256 of the game data pack installs must use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamedata_pack_sha256: Option<String>,
    /// Server name to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
}

impl SharedConfigBundle {
    /// Checks every part is in range, the hashes are SHA-256 and nothing is a secret.
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.shared_host_toml.len() > 256 * 1024 {
            return Err(RemoteError::InvalidInput(
                "shared_host_toml exceeds 256 KiB".to_string(),
            ));
        }
        if contains_forbidden_secret_str(&self.shared_host_toml) {
            return Err(RemoteError::SecretLeakRefused(
                "shared_host_toml must never contain secrets".to_string(),
            ));
        }
        for (node_key, toml_text) in &self.node_overrides {
            if contains_forbidden_secret_str(node_key) || contains_forbidden_secret_str(toml_text) {
                return Err(RemoteError::SecretLeakRefused(
                    "node_overrides must never contain secrets".to_string(),
                ));
            }
        }
        for (preset_name, preset_json) in &self.presets {
            if contains_forbidden_secret_str(preset_name) || contains_forbidden_secret_str(preset_json) {
                return Err(RemoteError::SecretLeakRefused(
                    "presets must never contain secrets".to_string(),
                ));
            }
        }
        if let Some(fp) = &self.runtime_manifest_sha256
            && !is_valid_sha256_fp(fp)
        {
            return Err(RemoteError::InvalidInput(
                "runtime_manifest_sha256 must be 64 lowercase hex characters".to_string(),
            ));
        }
        if let Some(fp) = &self.gamedata_pack_sha256
            && !is_valid_sha256_fp(fp)
        {
            return Err(RemoteError::InvalidInput(
                "gamedata_pack_sha256 must be 64 lowercase hex characters".to_string(),
            ));
        }
        Ok(())
    }
}

/// One log line fetched from a managed install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteLogEntry {
    /// Sequence number, for paging.
    pub seq: u64,
    /// When it was logged, in Unix milliseconds.
    pub timestamp_ms: i64,
    /// Level name, as text.
    pub level: String,
    /// Module that wrote it.
    pub target: String,
    /// The message, redacted.
    pub message: String,
}

/// Request sent over `Service::Remote` (`POST /__node/remote`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RemoteRequest {
    /// Outbound heartbeat pull sent by a managed install to its controller.
    Sync {
        /// The install's node id; must match its certificate.
        node: NodeId,
        /// Config revision it has applied.
        applied_revision: u64,
        /// Its status report.
        report: Box<InstallReport>,
        /// Outcomes of commands run since the last sync.
        #[serde(default)]
        completed_commands: Vec<CommandOutcome>,
    },
    /// Direct command push sent by the controller to a reachable managed install.
    Execute {
        /// The command to run now.
        command: QueuedRemoteCommand,
    },
    /// Direct config revision push sent by the controller to a reachable managed install.
    PushRevision {
        /// The config to apply.
        bundle: SharedConfigBundle,
    },
    /// Log page request sent by the controller to a reachable managed install.
    FetchLogs {
        /// Return entries after this sequence number.
        since_seq: u64,
        /// Most entries, 1 to [`MAX_LOG_ENTRIES`].
        limit: u32,
    },
}

impl RemoteRequest {
    /// Checks every part; a `FetchLogs` limit must be 1 to [`MAX_LOG_ENTRIES`].
    pub fn validate(&self) -> Result<(), RemoteError> {
        match self {
            Self::Sync {
                report,
                completed_commands,
                node,
                ..
            } => {
                if &report.node != node {
                    return Err(RemoteError::InvalidInput(
                        "Sync node must match InstallReport.node".to_string(),
                    ));
                }
                report.validate()?;
                if completed_commands.len() > 64 {
                    return Err(RemoteError::InvalidInput(
                        "too many completed_commands in one Sync".to_string(),
                    ));
                }
                for outcome in completed_commands {
                    outcome.validate()?;
                }
            }
            Self::Execute { command } => command.validate()?,
            Self::PushRevision { bundle } => bundle.validate()?,
            Self::FetchLogs { limit, .. } => {
                if *limit == 0 || (*limit as usize) > MAX_LOG_ENTRIES {
                    return Err(RemoteError::InvalidInput(format!(
                        "FetchLogs limit must be 1..={MAX_LOG_ENTRIES}"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Response returned over `Service::Remote` (`POST /__node/remote`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RemoteResponse {
    /// The controller's answer to a sync.
    SyncAck {
        /// The controller's current config revision.
        controller_revision: u64,
        /// The new config, when the install is behind.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle: Option<SharedConfigBundle>,
        /// Commands queued for the install.
        #[serde(default)]
        pending_commands: Vec<QueuedRemoteCommand>,
    },
    /// The outcome of an `Execute`.
    Executed {
        /// How it went.
        outcome: CommandOutcome,
    },
    /// A pushed revision was applied.
    RevisionApplied {
        /// The revision now applied.
        applied_revision: u64,
    },
    /// A page of log lines.
    Logs {
        /// Sequence number to ask from next.
        next_seq: u64,
        /// The lines, at most [`MAX_LOG_ENTRIES`].
        entries: Vec<RemoteLogEntry>,
    },
    /// The request was refused or failed.
    Error {
        /// Machine-readable code, at most 64 bytes.
        code: String,
        /// What went wrong, at most 2048 bytes.
        message: String,
    },
}

impl RemoteResponse {
    /// Checks every part; an error's code and message are bounded and hold no secrets.
    pub fn validate(&self) -> Result<(), RemoteError> {
        match self {
            Self::SyncAck {
                bundle,
                pending_commands,
                ..
            } => {
                if let Some(b) = bundle {
                    b.validate()?;
                }
                if pending_commands.len() > 64 {
                    return Err(RemoteError::InvalidInput(
                        "too many pending_commands in SyncAck".to_string(),
                    ));
                }
                for cmd in pending_commands {
                    cmd.validate()?;
                }
            }
            Self::Executed { outcome } => outcome.validate()?,
            Self::RevisionApplied { .. } => {}
            Self::Logs { entries, .. } => {
                if entries.len() > MAX_LOG_ENTRIES {
                    return Err(RemoteError::InvalidInput(format!(
                        "Logs response exceeds {MAX_LOG_ENTRIES} entries"
                    )));
                }
                for entry in entries {
                    if contains_forbidden_secret_str(&entry.message) || contains_forbidden_secret_str(&entry.target) {
                        return Err(RemoteError::SecretLeakRefused(
                            "log entry contains a forbidden secret".to_string(),
                        ));
                    }
                }
            }
            Self::Error { code, message } => {
                if code.trim().is_empty()
                    || code.len() > 64
                    || message.len() > 2048
                    || contains_forbidden_secret_str(code)
                    || contains_forbidden_secret_str(message)
                {
                    return Err(RemoteError::InvalidInput("invalid Error response".to_string()));
                }
            }
        }
        Ok(())
    }
}

fn check_wire_slice(bytes: &[u8]) -> Result<(), RemoteError> {
    if bytes.len() > MAX_REMOTE_WIRE {
        return Err(RemoteError::TooLarge(bytes.len()));
    }
    if contains_forbidden_secret_bytes(bytes) {
        return Err(RemoteError::SecretLeakRefused(
            "wire payload contains a forbidden secret, account key, or database header".to_string(),
        ));
    }
    Ok(())
}

/// Validates and serializes an enrollment request; refuses one over [`MAX_REMOTE_WIRE`] or holding a secret.
pub fn encode_enroll_request(req: &EnrollRequest) -> Result<Vec<u8>, RemoteError> {
    req.validate()?;
    let bytes = serde_json::to_vec(req).map_err(|e| RemoteError::InvalidInput(format!("encode EnrollRequest: {e}")))?;
    check_wire_slice(&bytes)?;
    Ok(bytes)
}

/// Parses and validates an enrollment request, with the same size and secret checks.
pub fn decode_enroll_request(bytes: &[u8]) -> Result<EnrollRequest, RemoteError> {
    check_wire_slice(bytes)?;
    let req: EnrollRequest =
        serde_json::from_slice(bytes).map_err(|e| RemoteError::InvalidInput(format!("decode EnrollRequest: {e}")))?;
    req.validate()?;
    Ok(req)
}

/// Validates and serializes an enrollment answer, with the size and secret checks.
pub fn encode_enroll_answer(ans: &EnrollAnswer) -> Result<Vec<u8>, RemoteError> {
    ans.validate()?;
    let bytes = serde_json::to_vec(ans).map_err(|e| RemoteError::InvalidInput(format!("encode EnrollAnswer: {e}")))?;
    check_wire_slice(&bytes)?;
    Ok(bytes)
}

/// Parses and validates an enrollment answer, with the size and secret checks.
pub fn decode_enroll_answer(bytes: &[u8]) -> Result<EnrollAnswer, RemoteError> {
    check_wire_slice(bytes)?;
    let ans: EnrollAnswer =
        serde_json::from_slice(bytes).map_err(|e| RemoteError::InvalidInput(format!("decode EnrollAnswer: {e}")))?;
    ans.validate()?;
    Ok(ans)
}

/// Validates and serializes a remote request, with the size and secret checks.
pub fn encode_request(req: &RemoteRequest) -> Result<Vec<u8>, RemoteError> {
    req.validate()?;
    let bytes = serde_json::to_vec(req).map_err(|e| RemoteError::InvalidInput(format!("encode RemoteRequest: {e}")))?;
    check_wire_slice(&bytes)?;
    Ok(bytes)
}

/// Parses and validates a remote request, with the size and secret checks.
pub fn decode_request(bytes: &[u8]) -> Result<RemoteRequest, RemoteError> {
    check_wire_slice(bytes)?;
    let req: RemoteRequest =
        serde_json::from_slice(bytes).map_err(|e| RemoteError::InvalidInput(format!("decode RemoteRequest: {e}")))?;
    req.validate()?;
    Ok(req)
}

/// Validates and serializes a remote response, with the size and secret checks.
pub fn encode_response(resp: &RemoteResponse) -> Result<Vec<u8>, RemoteError> {
    resp.validate()?;
    let bytes =
        serde_json::to_vec(resp).map_err(|e| RemoteError::InvalidInput(format!("encode RemoteResponse: {e}")))?;
    check_wire_slice(&bytes)?;
    Ok(bytes)
}

/// Parses and validates a remote response, with the size and secret checks.
pub fn decode_response(bytes: &[u8]) -> Result<RemoteResponse, RemoteError> {
    check_wire_slice(bytes)?;
    let resp: RemoteResponse =
        serde_json::from_slice(bytes).map_err(|e| RemoteError::InvalidInput(format!("decode RemoteResponse: {e}")))?;
    resp.validate()?;
    Ok(resp)
}
