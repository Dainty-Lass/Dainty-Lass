//! Launcher Guard (`controllers/launcherGuard.ts`, `routes/launcherGuard.ts`; isles keeps it, RF1 decision 1, and
//! rustic-rifts reads its sessions through `IslesCore`): a launcher opens a session against the signed Guard manifest,
//! then sends signed heartbeats about the game process; the server scores them and keeps the verdict.

use axum::response::Response;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use module_api::{Actor, ActorContext};
use serde_json::{Map, Value, json};

use super::guard_manifest::{Payload, is_version};
use super::launcher_auth::{LauncherCaller, LauncherFailure, has_launcher_auth};
use super::updates::published_guard_manifest;
use super::util::{self, BodyKind, Req, base64_decode, date_parse, iso, now_ms, random_bytes};
use crate::db::IslesStore;
use crate::http::{Call, Handlers, Shared, not_implemented};
use crate::ids::AccountId;
use crate::repo::reforged::{GuardHeartbeat, GuardRole, GuardSighting, GuardStatus, LauncherGuardSessionRecord};
use crate::routes::{Method, Service};

const SESSION_LIFETIME_MS: i64 = 5 * 60_000;
const MAX_CLOCK_SKEW_MS: i64 = 60_000;
const UNEXPECTED_MODULE_LIMIT: usize = 32;
const MODULE_FILE_NAME_LIMIT: usize = 260;
const MODULE_SIGNER_LIMIT: usize = 256;
const MODULE_SIZE_LIMIT: i64 = 1 << 40;
const BODY_LIMIT: usize = 100 * 1024;

type Failure = LauncherFailure;

fn report_invalid() -> Failure {
    Failure::new(400, "GUARD_REPORT_INVALID", "The Launcher Guard report is invalid.")
}

fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn opaque(bytes: usize) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(bytes))
}

/// `ParseLauncherGuardRole`.
pub fn parse_role(value: Option<&Value>, fallback: Option<GuardRole>) -> Option<GuardRole> {
    match value {
        Some(Value::String(s)) if s == "client" => Some(GuardRole::Client),
        Some(Value::String(s)) if s == "host" => Some(GuardRole::Host),
        None | Some(Value::Null) => fallback,
        _ => None,
    }
}

/// `VersionAtLeast` (`security/launcherVersion.ts`): semver order, a prerelease below its release.
pub fn version_at_least(actual: &str, minimum: &str) -> bool {
    struct Parsed {
        core: [u128; 3],
        prerelease: Option<Vec<String>>,
    }
    fn parse(text: &str) -> Option<Parsed> {
        let text = service_kit::js::trim(text);
        let (rest, _build) = text.split_once('+').map_or((text, None), |(a, b)| (a, Some(b)));
        let (core, pre) = rest.split_once('-').map_or((rest, None), |(a, b)| (a, Some(b)));
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        let valid = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if pre.is_some_and(|p| !valid(p)) || _build.is_some_and(|b| !valid(b)) {
            return None;
        }
        Some(Parsed {
            core: [parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?],
            prerelease: pre.map(|p| p.split('.').map(str::to_owned).collect()),
        })
    }
    let (Some(left), Some(right)) = (parse(actual), parse(minimum)) else {
        return false;
    };
    for index in 0..3 {
        if left.core[index] != right.core[index] {
            return left.core[index] > right.core[index];
        }
    }
    let Some(left_pre) = left.prerelease else {
        return true;
    };
    let Some(right_pre) = right.prerelease else {
        return false;
    };
    for index in 0..left_pre.len().max(right_pre.len()) {
        let (Some(a), b) = (left_pre.get(index), right_pre.get(index)) else {
            return true;
        };
        let Some(b) = b else {
            return false;
        };
        if a == b {
            continue;
        }
        let numeric = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
        return match (numeric(a), numeric(b)) {
            (true, true) => a.parse::<u128>().unwrap_or(0) > b.parse::<u128>().unwrap_or(0),
            (x, y) if x != y => !x,
            _ => a > b,
        };
    }
    true
}

/// `IsGuardManifestCurrent`.
pub fn is_manifest_current(manifest: Option<&Payload>, expected_id: &str, expected_sequence: i64, now: i64) -> bool {
    let Some(manifest) = manifest else {
        return false;
    };
    manifest.manifest_id == expected_id
        && manifest.sequence == expected_sequence
        && date_parse(&manifest.expires_at).is_some_and(|expiry| expiry > now)
}

// ---------------------------------------------------------------- the report

#[derive(Debug, Clone, PartialEq)]
pub struct UnexpectedModule {
    pub file_name: String,
    pub signer: String,
    pub sha256: String,
    pub size: i64,
}

impl UnexpectedModule {
    fn to_json(&self) -> Value {
        json!({ "fileName": self.file_name, "signer": self.signer, "sha256": self.sha256, "size": self.size })
    }
}

/// `ParseUnexpectedModules`.
pub fn parse_unexpected_modules(value: Option<&Value>) -> Result<Option<Vec<UnexpectedModule>>, Failure> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let Value::Array(entries) = value else {
        return Err(report_invalid());
    };
    if entries.len() > UNEXPECTED_MODULE_LIMIT {
        return Err(report_invalid());
    }
    entries
        .iter()
        .map(|entry| {
            let Value::Object(input) = entry else {
                return Err(report_invalid());
            };
            let file_name = input.get("fileName").and_then(Value::as_str).unwrap_or_default();
            let units = file_name.encode_utf16().count();
            if units == 0
                || units > MODULE_FILE_NAME_LIMIT
                || file_name == "."
                || file_name == ".."
                || file_name.contains(['\\', '/'])
                || file_name
                    .chars()
                    .any(|c| c <= '\u{1f}' || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
                || !input.get("fileName").is_some_and(Value::is_string)
            {
                return Err(report_invalid());
            }
            let signer = input.get("signer").and_then(Value::as_str).unwrap_or_default();
            let signer_units = signer.encode_utf16().count();
            if signer_units == 0 || signer_units > MODULE_SIGNER_LIMIT || signer.chars().any(|c| c <= '\u{1f}') {
                return Err(report_invalid());
            }
            let sha256 = input
                .get("sha256")
                .and_then(Value::as_str)
                .filter(|s| is_hex64(s))
                .ok_or_else(report_invalid)?;
            let size = service_kit::js::safe_integer(input.get("size"))
                .filter(|s| (0..=MODULE_SIZE_LIMIT).contains(s))
                .filter(|_| input.get("size").is_some_and(Value::is_number))
                .ok_or_else(report_invalid)?;
            Ok(UnexpectedModule {
                file_name: file_name.to_owned(),
                signer: signer.to_owned(),
                sha256: sha256.to_lowercase(),
                size,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// `LauncherGuardSignals`: the report as stored (every key the launcher sent, the module list normalised) and
/// the numbers the score reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Signals {
    pub raw: Map<String, Value>,
    pub process_id: Value,
    pub started_at: String,
    pub module_set_digest: String,
    pub private_executable_bytes: i64,
    pub writable_executable_bytes: i64,
    pub unexpected_module_count: i64,
    pub protected_page_mismatch_count: i64,
    pub debugger_present: bool,
    pub modules: Vec<UnexpectedModule>,
}

impl Signals {
    /// `requireSignals`.
    pub fn parse(value: Option<&Value>) -> Result<Self, Failure> {
        let Some(Value::Object(input)) = value else {
            return Err(report_invalid());
        };
        let int = |name: &str| service_kit::js::safe_integer(input.get(name)).filter(|n| *n >= 0);
        let (Some(_), Some(_), Some(private), Some(writable), Some(unexpected), Some(mismatch)) = (
            int("gameProcessId"),
            int("moduleCount"),
            int("privateExecutableBytes"),
            int("writableExecutableBytes"),
            int("unexpectedModuleCount"),
            int("protectedPageMismatchCount"),
        ) else {
            return Err(report_invalid());
        };
        let started_at = input.get("gameStartedAt").and_then(Value::as_str);
        let digest = input.get("moduleSetDigest").and_then(Value::as_str);
        let debugger = input.get("debuggerPresent").and_then(Value::as_bool);
        let (Some(started_at), Some(digest), Some(debugger)) = (started_at, digest, debugger) else {
            return Err(report_invalid());
        };
        if date_parse(started_at).is_none() || !is_hex64(digest) {
            return Err(report_invalid());
        }
        let modules = parse_unexpected_modules(input.get("unexpectedModules"))?;
        let mut raw = input.clone();
        raw.remove("unexpectedModules");
        if let Some(modules) = &modules {
            raw.insert(
                "unexpectedModules".into(),
                Value::Array(modules.iter().map(UnexpectedModule::to_json).collect()),
            );
        }
        Ok(Self {
            process_id: input.get("gameProcessId").cloned().unwrap_or(Value::Null),
            raw,
            started_at: started_at.to_owned(),
            module_set_digest: digest.to_owned(),
            private_executable_bytes: private,
            writable_executable_bytes: writable,
            unexpected_module_count: unexpected,
            protected_page_mismatch_count: mismatch,
            debugger_present: debugger,
            modules: modules.unwrap_or_default(),
        })
    }

    /// The same report read back from what was stored.
    pub fn from_stored(value: &Value) -> Option<Self> {
        Self::parse(Some(value)).ok()
    }

    fn number_text(&self, name: &str) -> String {
        service_kit::js::string(self.raw.get(name))
    }
}

/// `LauncherGuardSignatureMaterial`: ordered, versioned and deterministic.
#[allow(clippy::too_many_arguments)]
pub fn signature_material(
    role: GuardRole,
    guard_session_id: &str,
    sequence: i64,
    challenge: &str,
    manifest_id: &str,
    manifest_sequence: u64,
    observed_at: &str,
    signals: &Signals,
) -> String {
    [
        "MYSTIC-GUARD-2".to_owned(),
        role.as_str().to_owned(),
        guard_session_id.to_owned(),
        sequence.to_string(),
        challenge.to_owned(),
        manifest_id.to_owned(),
        manifest_sequence.to_string(),
        observed_at.to_owned(),
        signals.number_text("gameProcessId"),
        signals.started_at.clone(),
        signals.number_text("moduleCount"),
        signals.module_set_digest.to_lowercase(),
        signals.number_text("privateExecutableBytes"),
        signals.number_text("writableExecutableBytes"),
        signals.number_text("unexpectedModuleCount"),
        signals.number_text("protectedPageMismatchCount"),
        u8::from(signals.debugger_present).to_string(),
    ]
    .join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub score: i64,
    pub reasons: Vec<String>,
}

/// `SplitUnexpectedModules`: `(untrusted, signed)`.
fn split_unexpected(signals: &Signals) -> (i64, i64) {
    let count = signals.unexpected_module_count;
    let listed = signals
        .modules
        .iter()
        .filter(|m| {
            let signer = service_kit::js::trim(&m.signer);
            !(signer.eq_ignore_ascii_case("unsigned") || signer.eq_ignore_ascii_case("invalid"))
        })
        .count();
    let signed = i64::try_from(listed).unwrap_or(i64::MAX).min(count);
    (count - signed, signed)
}

/// `explainSignals`.
pub fn explain_signals(signals: &Signals, private_limit: i64, writable_limit: i64) -> Explanation {
    let mut score = 0;
    let mut reasons = Vec::new();
    if signals.debugger_present {
        score += 50;
        reasons.push("DEBUGGER".to_owned());
    }
    // Untrusted modules weigh +15 each (a hard violation); signed ones are a low-weight finding only
    let (untrusted, signed) = split_unexpected(signals);
    if untrusted > 0 {
        score += 50.min(untrusted * 15);
        reasons.push(format!("UNEXPECTED_UNSIGNED:{untrusted}"));
    }
    if signed > 0 {
        score += 10.min(signed * 2);
        reasons.push(format!("UNEXPECTED_SIGNED:{signed}"));
    }
    if signals.protected_page_mismatch_count > 0 {
        score += 60.min(signals.protected_page_mismatch_count * 30);
        reasons.push(format!("PAGE_MISMATCH:{}", signals.protected_page_mismatch_count));
    }
    if signals.writable_executable_bytes > writable_limit {
        score += 60;
        reasons.push("WRITABLE_EXEC_OVER".to_owned());
    }
    if signals.private_executable_bytes > private_limit {
        score += 40;
        reasons.push("PRIVATE_EXEC_OVER".to_owned());
    }
    Explanation {
        score: score.min(100),
        reasons,
    }
}

/// `hasHardIntegrityViolation`.
pub fn hard_violation(signals: &Signals, private_limit: i64, writable_limit: i64) -> bool {
    signals.debugger_present
        || split_unexpected(signals).0 > 0
        || signals.protected_page_mismatch_count > 0
        || signals.private_executable_bytes > private_limit
        || signals.writable_executable_bytes > writable_limit
}

/// `guardStatus`.
pub fn guard_status(signals: &Signals, private_limit: i64, writable_limit: i64, risk: i64) -> GuardStatus {
    if !hard_violation(signals, private_limit, writable_limit) && risk < 50 {
        GuardStatus::Healthy
    } else {
        GuardStatus::Degraded
    }
}

/// `explainContinuity`: a changed process is 100; a changed module set once warm (after the first two reports) 50.
pub fn explain_continuity(previous: Option<&Signals>, current: &Signals, previous_sequence: u64) -> Explanation {
    let Some(previous) = previous else {
        return Explanation {
            score: 0,
            reasons: vec![],
        };
    };
    if previous.process_id != current.process_id || previous.started_at != current.started_at {
        return Explanation {
            score: 100,
            reasons: vec!["PROCESS_CHANGED".into()],
        };
    }
    if previous_sequence >= 2 && previous.module_set_digest != current.module_set_digest {
        return Explanation {
            score: 50,
            reasons: vec!["MODULE_SET_CHANGED".into()],
        };
    }
    Explanation {
        score: 0,
        reasons: vec![],
    }
}

// ---------------------------------------------------------------- the routes

fn internal() -> Failure {
    Failure::new(500, "INTERNAL", "Launcher Guard failed.")
}

fn db_failure(error: impl std::fmt::Display) -> Failure {
    tracing::error!("Unhandled Launcher Guard error: {error}");
    internal()
}

fn actor(account: &AccountId) -> ActorContext {
    ActorContext::new(Actor::Player {
        account: account.global().clone(),
    })
}

/// `StartLauncherGuard`.
pub async fn start(
    shared: &Shared,
    store: &IslesStore,
    account: &AccountId,
    body: &Map<String, Value>,
) -> Result<Value, Failure> {
    let role = parse_role(body.get("role"), Some(GuardRole::Client))
        .ok_or_else(|| Failure::new(400, "GUARD_REPORT_INVALID", "The Launcher Guard role is invalid."))?;
    let channel = body
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or("stable")
        .to_owned();
    let launcher_version = body
        .get("launcherVersion")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let now = now_ms();
    let manifest = published_guard_manifest(shared, &channel, "windows-x86_64")
        .filter(|m| date_parse(&m.expires_at).is_some_and(|expiry| expiry > now))
        .ok_or_else(|| {
            Failure::new(
                503,
                "GUARD_MANIFEST_UNAVAILABLE",
                "No current signed Guard manifest is available.",
            )
        })?;
    let mismatch = body.get("manifestId").and_then(Value::as_str) != Some(manifest.manifest_id.as_str())
        || service_kit::js::safe_integer(body.get("manifestSequence")) != Some(manifest.sequence)
        || manifest.channel != channel
        || !is_version(&launcher_version)
        || !version_at_least(&launcher_version, &manifest.minimum_launcher_version);
    if mismatch {
        return Err(Failure::new(
            409,
            "GUARD_MANIFEST_MISMATCH",
            "The launcher runtime manifest is not current.",
        ));
    }
    let public_key = body
        .get("publicKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if base64_decode(&public_key).len() != 32 {
        return Err(Failure::new(
            400,
            "GUARD_REPORT_INVALID",
            "The ephemeral Guard public key is invalid.",
        ));
    }
    let guard_session_id = uuid::Uuid::new_v4().to_string();
    let challenge = opaque(24);
    let created = iso(now);
    let expires = iso(now + SESSION_LIFETIME_MS);
    let record = LauncherGuardSessionRecord {
        guard_session_id: guard_session_id.clone(),
        account: account.clone(),
        role,
        public_key_base64: public_key,
        manifest_id: manifest.manifest_id.clone(),
        manifest_sequence: u64::try_from(manifest.sequence).unwrap_or(0),
        channel,
        launcher_version,
        challenge: challenge.clone(),
        last_sequence: 0,
        status: GuardStatus::Challenged,
        risk_score: 0.0,
        created_at: created.clone(),
        expires_at: expires.clone(),
        last_heartbeat_at: None,
        last_signals: None,
        reasons: vec![],
        history: vec![],
    };
    let target = account.clone();
    store
        .account_write(account, &actor(account), move |tx, repos| {
            repos.launcher_guard.revoke_for_account(tx, &target, role, &created)?;
            repos.launcher_guard.create(tx, &record)
        })
        .await
        .map_err(db_failure)?;
    Ok(json!({
        "schema": 1, "role": role.as_str(), "guardSessionId": guard_session_id, "challenge": challenge,
        "manifestId": manifest.manifest_id, "manifestSequence": manifest.sequence, "expiresAt": expires,
        "heartbeatIntervalMs": 15_000,
    }))
}

/// `SubmitLauncherGuardHeartbeat`.
pub async fn heartbeat(
    shared: &Shared,
    store: &IslesStore,
    account: &AccountId,
    body: &Map<String, Value>,
) -> Result<Value, Failure> {
    let session_id = body
        .get("guardSessionId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let sequence_value = service_kit::js::number(body.get("sequence"));
    let observed_at = body
        .get("observedAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let signature_text = body
        .get("signature")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let invalid = || Failure::new(400, "GUARD_REPORT_INVALID", "The Launcher Guard heartbeat is invalid.");
    let sequence =
        (sequence_value.fract() == 0.0 && sequence_value.abs() <= 9_007_199_254_740_991.0 && sequence_value >= 1.0)
            .then_some(sequence_value as i64);
    let observed = date_parse(&observed_at);
    let signature_units = signature_text.encode_utf16().count();
    let uuid_shape = session_id.len() == 36 && session_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
    let (true, Some(sequence), Some(observed), true) =
        (uuid_shape, sequence, observed, (80..=100).contains(&signature_units))
    else {
        return Err(invalid());
    };
    if (now_ms() - observed).abs() > MAX_CLOCK_SKEW_MS {
        return Err(invalid());
    }
    let signals = Signals::parse(body.get("signals"))?;
    let lookup = session_id.clone();
    let current = store
        .read(move |conn, repos| repos.launcher_guard.find(conn, &lookup))
        .await
        .map_err(db_failure)?;
    let session_invalid = |message: &str| Failure::new(401, "GUARD_SESSION_INVALID", message);
    let Some(current) = current.filter(|c| {
        &c.account == account
            && i64::try_from(c.last_sequence).unwrap_or(i64::MAX) < sequence
            && !matches!(c.status, GuardStatus::Revoked | GuardStatus::Expired)
            && date_parse(&c.expires_at).is_some_and(|expiry| expiry > now_ms())
    }) else {
        return Err(session_invalid("The Launcher Guard session is invalid or expired."));
    };
    if let Some(requested) = parse_role(body.get("role"), None)
        && requested != current.role
    {
        return Err(session_invalid("The Launcher Guard role does not match the session."));
    }
    let material = signature_material(
        current.role,
        &session_id,
        sequence,
        &current.challenge,
        &current.manifest_id,
        current.manifest_sequence,
        &observed_at,
        &signals,
    );
    let public = base64_decode(&current.public_key_base64);
    let valid = <[u8; 32]>::try_from(public.as_slice())
        .ok()
        .and_then(|raw| VerifyingKey::from_bytes(&raw).ok())
        .zip(Signature::from_slice(&base64_decode(&signature_text)).ok())
        .is_some_and(|(key, signature)| key.verify(material.as_bytes(), &signature).is_ok());
    if !valid {
        return Err(Failure::new(
            401,
            "GUARD_SIGNATURE_INVALID",
            "The Launcher Guard heartbeat could not be authenticated.",
        ));
    }
    let manifest = published_guard_manifest(shared, &current.channel, "windows-x86_64");
    if !is_manifest_current(
        manifest.as_ref(),
        &current.manifest_id,
        i64::try_from(current.manifest_sequence).unwrap_or(i64::MAX),
        now_ms(),
    ) {
        return Err(Failure::new(
            409,
            "GUARD_MANIFEST_MISMATCH",
            "The Launcher Guard manifest changed; restart the game.",
        ));
    }
    let Some(manifest) = manifest else {
        return Err(internal());
    };
    let explained = explain_signals(
        &signals,
        manifest.maximum_private_executable_bytes,
        manifest.maximum_writable_executable_bytes,
    );
    let previous = current.last_signals.as_ref().and_then(Signals::from_stored);
    let continuity = explain_continuity(previous.as_ref(), &signals, current.last_sequence);
    let risk = (explained.score + continuity.score).min(100);
    let reasons: Vec<String> = explained.reasons.into_iter().chain(continuity.reasons).collect();
    let status = guard_status(
        &signals,
        manifest.maximum_private_executable_bytes,
        manifest.maximum_writable_executable_bytes,
        risk,
    );
    let now = now_ms();
    let heartbeat_at = iso(now);
    let expires_at = iso(now + SESSION_LIFETIME_MS);
    let next_challenge = opaque(24);
    let stored = Value::Object(signals.raw.clone());
    let step = GuardHeartbeat {
        guard_session_id: session_id,
        account: account.clone(),
        expected_challenge: current.challenge.clone(),
        sequence: u64::try_from(sequence).unwrap_or(0),
        next_challenge: next_challenge.clone(),
        status,
        risk_score: risk as f64,
        heartbeat_at: heartbeat_at.clone(),
        expires_at: expires_at.clone(),
        signals: stored,
        reasons: reasons.clone(),
        history_entry: json!({ "at": heartbeat_at, "status": status.as_str(), "riskScore": risk, "reasons": reasons }),
    };
    let updated = store
        .account_write(account, &actor(account), move |tx, repos| {
            repos.launcher_guard.advance(tx, &step)
        })
        .await
        .map_err(db_failure)?;
    if updated.is_none() {
        return Err(Failure::new(
            409,
            "GUARD_SEQUENCE_CONFLICT",
            "The Launcher Guard heartbeat was superseded.",
        ));
    }
    record_findings(
        store,
        account,
        &signals.modules,
        previous.as_ref().map(|p| &p.modules[..]),
        &heartbeat_at,
    )
    .await;
    Ok(json!({
        "schema": 1, "accepted": true, "role": current.role.as_str(), "status": status.as_str(), "riskScore": risk,
        "reasons": reasons, "nextChallenge": next_challenge, "expiresAt": expires_at,
    }))
}

/// `RecordGuardModuleFindings`: one sighting per distinct module, a failure logged and never raised.
pub async fn record_findings(
    store: &IslesStore,
    account: &AccountId,
    modules: &[UnexpectedModule],
    previous: Option<&[UnexpectedModule]>,
    seen_at: &str,
) -> usize {
    if modules.is_empty() {
        return 0;
    }
    let key = |m: &UnexpectedModule| format!("{}|{}", m.file_name, m.sha256);
    let before: std::collections::HashSet<String> = previous.unwrap_or_default().iter().map(key).collect();
    let mut seen = std::collections::HashSet::new();
    let mut recorded = 0;
    for module in modules {
        if !seen.insert(key(module)) {
            continue;
        }
        let sighting = GuardSighting {
            file_name: module.file_name.clone(),
            sha256: module.sha256.clone(),
            signer: module.signer.clone(),
            size: u64::try_from(module.size).unwrap_or(0),
            account: account.clone(),
            seen_at: seen_at.to_owned(),
            new_in_session: !before.contains(&key(module)),
        };
        let result = store
            .account_write(account, &actor(account), move |tx, repos| {
                repos.launcher_guard.record_finding_sighting(tx, &sighting)
            })
            .await;
        match result {
            Ok(()) => recorded += 1,
            Err(error) => tracing::warn!(
                "[GuardFindings] could not record {} for account={account}: {error}",
                module.file_name
            ),
        }
    }
    recorded
}

/// `GetLauncherGuardStatus`.
pub async fn status(store: &IslesStore, account: &AccountId, guard_session_id: &str) -> Result<Value, Failure> {
    let lookup = guard_session_id.to_owned();
    let session = store
        .read(move |conn, repos| repos.launcher_guard.find(conn, &lookup))
        .await
        .map_err(db_failure)?;
    let Some(session) = session.filter(|s| &s.account == account) else {
        return Err(Failure::new(
            401,
            "GUARD_SESSION_INVALID",
            "Launcher Guard session not found.",
        ));
    };
    let mut out = Map::new();
    out.insert("schema".into(), json!(1));
    out.insert("role".into(), json!(session.role.as_str()));
    out.insert("guardSessionId".into(), json!(guard_session_id));
    out.insert("status".into(), json!(session.status.as_str()));
    out.insert("riskScore".into(), json!(session.risk_score));
    out.insert("manifestId".into(), json!(session.manifest_id));
    out.insert("manifestSequence".into(), json!(session.manifest_sequence));
    // `lastHeartbeatAt` is undefined until the first heartbeat, and JSON.stringify leaves it out
    if let Some(at) = &session.last_heartbeat_at {
        out.insert("lastHeartbeatAt".into(), json!(at));
    }
    out.insert("expiresAt".into(), json!(session.expires_at));
    Ok(Value::Object(out))
}

#[derive(Clone, Copy)]
enum Action {
    Start,
    Heartbeat,
    Status,
}

async fn handle(call: Call, action: Action) -> Response {
    let (shared, params) = (call.shared.clone(), call.params.clone());
    let route = (call.route, call.request_id.clone());
    // The app-wide body parser comes before the route's auth
    let (mut req, body) = Req::head(call.request);
    if let Err(response) = req.load(body, BodyKind::Json(BODY_LIMIT)).await {
        return response;
    }
    let caller: LauncherCaller = match has_launcher_auth(&shared, req.header("authorization")).await {
        Ok(caller) => caller,
        Err(failure) => return failure.response(),
    };
    let Some(store) = shared.store.clone() else {
        return not_implemented(route.0, &route.1);
    };
    let fields = req.fields();
    let result = match action {
        Action::Start => start(&shared, &store, &caller.account, &fields).await,
        Action::Heartbeat => heartbeat(&shared, &store, &caller.account, &fields).await,
        Action::Status => {
            status(
                &store,
                &caller.account,
                params.get("guardSessionId").unwrap_or_default(),
            )
            .await
        }
    };
    match result {
        Ok(body) => util::json(200, &body),
        Err(failure) => failure.response(),
    }
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Post, "/launcher/v1/guard/sessions", |call| {
        Box::pin(handle(call, Action::Start))
    });
    handlers.on(b, Method::Post, "/launcher/v1/guard/heartbeat", |call| {
        Box::pin(handle(call, Action::Heartbeat))
    });
    handlers.on(b, Method::Get, "/launcher/v1/guard/sessions/:guardSessionId", |call| {
        Box::pin(handle(call, Action::Status))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> Signals {
        Signals::parse(Some(&json!({
            "gameProcessId": 4242, "gameStartedAt": "2026-10-14T10:00:00.000Z", "moduleCount": 188,
            "moduleSetDigest": "d".repeat(64), "privateExecutableBytes": 0, "writableExecutableBytes": 0,
            "unexpectedModuleCount": 0, "protectedPageMismatchCount": 0, "debuggerPresent": false
        })))
        .unwrap()
    }

    fn with(changes: Value) -> Signals {
        let mut raw = clean().raw;
        for (k, v) in changes.as_object().unwrap() {
            raw.insert(k.clone(), v.clone());
        }
        Signals::parse(Some(&Value::Object(raw))).unwrap()
    }

    fn explained(signals: &Signals, private: i64) -> (i64, Vec<String>) {
        let e = explain_signals(signals, private, 0);
        (e.score, e.reasons)
    }

    // security/guardProtocols.test.ts
    #[test]
    fn the_signature_material_is_ordered_versioned_and_deterministic() {
        let signals = Signals::parse(Some(&json!({
            "gameProcessId": 42, "gameStartedAt": "2026-08-12T00:00:01.000Z", "moduleCount": 5,
            "moduleSetDigest": "A".repeat(64), "privateExecutableBytes": 0, "writableExecutableBytes": 0,
            "unexpectedModuleCount": 0, "protectedPageMismatchCount": 0, "debuggerPresent": false
        })))
        .unwrap();
        let material = |role| {
            signature_material(
                role,
                "11111111-1111-4111-8111-111111111111",
                7,
                "challenge",
                "22222222-2222-4222-8222-222222222222",
                3,
                "2026-08-12T00:00:00.000Z",
                &signals,
            )
        };
        let lines: Vec<String> = material(GuardRole::Client).split('\n').map(str::to_owned).collect();
        assert_eq!(lines.len(), 17);
        assert_eq!((lines[0].as_str(), lines[1].as_str()), ("MYSTIC-GUARD-2", "client"));
        assert_eq!(lines[11], "a".repeat(64));
        assert_eq!(lines[3], "7");
        assert_eq!(lines[16], "0");
        assert_eq!(material(GuardRole::Client), material(GuardRole::Client));
        assert_ne!(material(GuardRole::Client), material(GuardRole::Host));
    }

    #[test]
    fn the_role_parser_defaults_only_legacy_sessions_to_client() {
        assert_eq!(parse_role(None, Some(GuardRole::Client)), Some(GuardRole::Client));
        assert_eq!(parse_role(Some(&json!("host")), None), Some(GuardRole::Host));
        assert_eq!(parse_role(Some(&json!("worker")), None), None);
        assert_eq!(parse_role(Some(&json!("worker")), Some(GuardRole::Client)), None);
    }

    #[test]
    fn a_changed_process_or_module_set_is_noticed_once_warm() {
        let base = clean();
        let moved = with(json!({ "moduleSetDigest": "e".repeat(64) }));
        assert_eq!(explain_continuity(Some(&base), &moved, 1).score, 0);
        assert_eq!(explain_continuity(Some(&base), &moved, 2).score, 50);
        let other = with(json!({ "gameProcessId": 4243 }));
        assert_eq!(explain_continuity(Some(&base), &other, 2).score, 100);
        assert_eq!(
            explain_continuity(None, &base, 0),
            Explanation {
                score: 0,
                reasons: vec![]
            }
        );
        assert_eq!(explain_continuity(Some(&base), &other, 5).reasons, ["PROCESS_CHANGED"]);
        assert_eq!(
            explain_continuity(Some(&base), &moved, 2).reasons,
            ["MODULE_SET_CHANGED"]
        );
    }

    #[test]
    fn a_manifest_past_its_expiry_is_not_current() {
        let manifest = Payload {
            manifest_id: "manifest-a".into(),
            sequence: 4,
            channel: "stable".into(),
            platform: "windows-x86_64".into(),
            target_changelist: 1,
            minimum_launcher_version: "0.1.0".into(),
            expires_at: "2026-09-06T00:00:10.000Z".into(),
            maximum_private_executable_bytes: 0,
            maximum_writable_executable_bytes: 0,
            artifacts: vec![],
        };
        let now = date_parse("2026-09-06T00:00:09.000Z").unwrap();
        assert!(is_manifest_current(Some(&manifest), "manifest-a", 4, now));
        assert!(!is_manifest_current(Some(&manifest), "manifest-a", 4, now + 1_000));
        assert!(!is_manifest_current(Some(&manifest), "manifest-b", 4, now));
        assert!(!is_manifest_current(Some(&manifest), "manifest-a", 5, now));
        assert!(!is_manifest_current(None, "manifest-a", 4, now));
    }

    #[test]
    fn executable_memory_thresholds_tolerate_bounded_trampolines_and_flag_excess() {
        let base = with(json!({ "privateExecutableBytes": 512 * 1024, "writableExecutableBytes": 32 * 1024 }));
        let (private, writable) = (1024 * 1024, 64 * 1024);
        let score = |s: &Signals| explain_signals(s, private, writable).score;
        assert_eq!(score(&base), 0);
        assert_eq!(guard_status(&base, private, writable, 0), GuardStatus::Healthy);
        let cases = [
            (
                json!({ "privateExecutableBytes": 512 * 1024, "writableExecutableBytes": 64 * 1024 + 1 }),
                60,
            ),
            (
                json!({ "privateExecutableBytes": 1024 * 1024 + 1, "writableExecutableBytes": 32 * 1024 }),
                40,
            ),
            (
                json!({ "privateExecutableBytes": 512 * 1024, "writableExecutableBytes": 32 * 1024, "protectedPageMismatchCount": 1 }),
                30,
            ),
            (
                json!({ "privateExecutableBytes": 512 * 1024, "writableExecutableBytes": 32 * 1024, "unexpectedModuleCount": 1 }),
                15,
            ),
        ];
        for (changes, expected) in cases {
            let signals = with(changes);
            assert_eq!(score(&signals), expected);
            assert_eq!(
                guard_status(&signals, private, writable, expected),
                GuardStatus::Degraded
            );
        }
    }

    // security/guardReportValidation.test.ts
    #[test]
    fn each_cause_names_itself() {
        assert_eq!(explained(&clean(), 1000), (0, vec![]));
        assert_eq!(
            explained(&with(json!({ "debuggerPresent": true })), 1000),
            (50, vec!["DEBUGGER".into()])
        );
        assert_eq!(
            explained(&with(json!({ "unexpectedModuleCount": 2 })), 1000),
            (30, vec!["UNEXPECTED_UNSIGNED:2".into()])
        );
        assert_eq!(explained(&with(json!({ "unexpectedModuleCount": 5 })), 1000).0, 50);
        assert_eq!(
            explained(&with(json!({ "protectedPageMismatchCount": 3 })), 1000),
            (60, vec!["PAGE_MISMATCH:3".into()])
        );
        assert_eq!(
            explained(&with(json!({ "writableExecutableBytes": 1 })), 1000),
            (60, vec!["WRITABLE_EXEC_OVER".into()])
        );
        assert_eq!(
            explained(&with(json!({ "privateExecutableBytes": 2000 })), 1000),
            (40, vec!["PRIVATE_EXEC_OVER".into()])
        );
        let everything = with(json!({
            "debuggerPresent": true, "unexpectedModuleCount": 5, "protectedPageMismatchCount": 3,
            "writableExecutableBytes": 1, "privateExecutableBytes": 2000
        }));
        let (score, reasons) = explained(&everything, 1000);
        assert_eq!(score, 100);
        assert_eq!(
            reasons,
            [
                "DEBUGGER",
                "UNEXPECTED_UNSIGNED:5",
                "PAGE_MISMATCH:3",
                "WRITABLE_EXEC_OVER",
                "PRIVATE_EXEC_OVER"
            ]
        );
    }

    #[test]
    fn a_signed_module_is_a_low_weight_finding_and_an_unsigned_one_a_hard_violation() {
        let module =
            |signer: &str| json!({ "fileName": "m.dll", "signer": signer, "sha256": "3".repeat(64), "size": 1 });
        let signed = with(json!({ "unexpectedModuleCount": 1, "unexpectedModules": [module("Valve Corp.")] }));
        assert_eq!(explained(&signed, 1000), (2, vec!["UNEXPECTED_SIGNED:1".into()]));
        assert!(!hard_violation(&signed, 1000, 0));
        assert_eq!(guard_status(&signed, 1000, 0, 2), GuardStatus::Healthy);
        for unsigned in ["unsigned", " Invalid ", "UNSIGNED"] {
            let s = with(json!({ "unexpectedModuleCount": 1, "unexpectedModules": [module(unsigned)] }));
            assert!(hard_violation(&s, 1000, 0), "{unsigned}");
        }
        // A listed module the count doesn't cover can't lift the score
        let over = with(json!({ "unexpectedModuleCount": 0, "unexpectedModules": [module("Valve Corp.")] }));
        assert_eq!(explained(&over, 1000).0, 0);
        let many = with(
            json!({ "unexpectedModuleCount": 9, "unexpectedModules": [module("A"), module("B"), module("C"), module("D"), module("E"), module("F")] }),
        );
        assert_eq!(
            explained(&many, 1000),
            (
                10 + 45,
                vec!["UNEXPECTED_UNSIGNED:3".into(), "UNEXPECTED_SIGNED:6".into()]
            )
        );
    }

    #[test]
    fn the_report_is_checked_field_by_field() {
        let good =
            json!({ "fileName": "Overlay (x64).dll", "signer": "unsigned", "sha256": "A".repeat(64), "size": 10 });
        let parsed = parse_unexpected_modules(Some(&json!([good.clone()]))).unwrap().unwrap();
        assert_eq!(parsed[0].sha256, "a".repeat(64));
        assert_eq!(parse_unexpected_modules(None).unwrap(), None);
        assert_eq!(parse_unexpected_modules(Some(&Value::Null)).unwrap(), None);
        let bad = |changes: Value| {
            let mut entry = good.as_object().unwrap().clone();
            for (k, v) in changes.as_object().unwrap() {
                if v.is_null() {
                    entry.remove(k);
                } else {
                    entry.insert(k.clone(), v.clone());
                }
            }
            parse_unexpected_modules(Some(&json!([Value::Object(entry)]))).unwrap_err()
        };
        for changes in [
            json!({ "fileName": "" }),
            json!({ "fileName": ".." }),
            json!({ "fileName": "a/b.dll" }),
            json!({ "fileName": "a\\b.dll" }),
            json!({ "fileName": "a:b.dll" }),
            json!({ "fileName": "a\u{1}b" }),
            json!({ "fileName": "a".repeat(261) }),
            json!({ "fileName": 5 }),
            json!({ "signer": "" }),
            json!({ "signer": "x\ny" }),
            json!({ "signer": "x".repeat(257) }),
            json!({ "sha256": "z".repeat(64) }),
            json!({ "sha256": "a".repeat(63) }),
            json!({ "size": -1 }),
            json!({ "size": 1.5 }),
            json!({ "size": "1" }),
            json!({ "size": (1u64 << 40) + 1 }),
            json!({ "size": null }),
        ] {
            assert_eq!(bad(changes.clone()).code, "GUARD_REPORT_INVALID", "{changes}");
        }
        assert!(parse_unexpected_modules(Some(&json!({ "a": 1 }))).is_err());
        assert!(parse_unexpected_modules(Some(&Value::Array(vec![good.clone(); 33]))).is_err());
        assert!(parse_unexpected_modules(Some(&Value::Array(vec![good; 32]))).is_ok());
        assert!(parse_unexpected_modules(Some(&json!(["x"]))).is_err());
        for broken in [json!(null), json!([]), json!({ "gameProcessId": -1 })] {
            assert!(Signals::parse(Some(&broken)).is_err());
        }
        let mut raw = clean().raw;
        raw.insert("extra".into(), json!(1));
        raw.insert("gameStartedAt".into(), json!("soon"));
        assert!(Signals::parse(Some(&Value::Object(raw.clone()))).is_err());
        raw.insert("gameStartedAt".into(), json!("2026-10-14T10:00:00.000Z"));
        assert_eq!(
            Signals::parse(Some(&Value::Object(raw))).unwrap().raw.get("extra"),
            Some(&json!(1))
        );
    }

    #[test]
    fn versions_compare_as_semver() {
        for (actual, minimum, expected) in [
            ("0.1.65", "0.1.65", true),
            ("0.1.66", "0.1.65", true),
            ("0.1.64", "0.1.65", false),
            ("1.0.0", "0.9.9", true),
            ("0.1.65-beta.1", "0.1.65", false),
            ("0.1.65", "0.1.65-beta.1", true),
            ("0.1.65-beta.2", "0.1.65-beta.1", true),
            ("0.1.65-beta.1", "0.1.65-beta.1", true),
            ("0.1.65-alpha", "0.1.65-1", true),
            ("0.1.65-1", "0.1.65-alpha", false),
            // As Node's code answers it: the side with the shorter prerelease list is the greater
            ("0.1.65-rc.1", "0.1.65-rc", false),
            ("0.1.65-rc", "0.1.65-rc.1", true),
            ("0.1.65+build", "0.1.65", true),
            ("x", "0.1.65", false),
            ("0.1.65", "y", false),
            ("0.1", "0.1.0", false),
        ] {
            assert_eq!(version_at_least(actual, minimum), expected, "{actual} >= {minimum}");
        }
    }
}

#[cfg(test)]
mod flow {
    //! A session from the first request to the findings, against a store, with a manifest signed by keys of the test's own.
    use std::sync::Arc;

    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use ed25519_dalek::{Signer, SigningKey};
    use module_api::NodeIdentity;
    use module_api::network::StandaloneHome;
    use store::{DatabaseConfig, DatabaseOptions};

    use super::*;
    use crate::areas::reforged_api::guard_manifest::Trusted;
    use crate::areas::reforged_api::updates::{Ctx, TEST_TRUSTED, publish_guard};
    use crate::areas::reforged_api::util::{Req, base64_encode};
    use crate::hooks::*;
    use crate::http::Shared;
    use crate::routes::{Params, RouteSet};
    use crate::tables::{StarterManifest, VersionTables};
    use crate::{IslesVersion, Settings, VersionHooks};

    struct Hooks;

    impl VersionHooks for Hooks {
        fn no_player_policy(&self) -> NoPlayerPolicy {
            NoPlayerPolicy::Off
        }
        fn game_session_payload(&self, _: &str) -> Value {
            Value::Null
        }
        fn network_encryption_keys(&self) -> bool {
            false
        }
        fn weapons_at_base_level(&self, _: &Settings) -> bool {
            false
        }
        fn reforge_rules(&self) -> bool {
            false
        }
        fn mastery_persistence(&self) -> bool {
            false
        }
        fn huntpass_payload(&self, _: &str) -> Value {
            Value::Null
        }
        fn objective_row(&self, _: &str, _: &ObjectiveRow) -> Value {
            Value::Null
        }
        fn progression_config(&self) -> ProgressionConfigRule {
            ProgressionConfigRule::AsIs
        }
        fn survey_config(&self) -> StubReply {
            StubReply {
                status: 400,
                body: None,
            }
        }
        fn entitlements_payload(&self, _: &[Entitlement]) -> Value {
            Value::Null
        }
        fn candidate_destination(&self, hunt_id: &str, _: bool) -> Destination {
            Destination {
                game_mode: GameMode::Island,
                hunt_id: Some(hunt_id.to_owned()),
            }
        }
    }

    fn shared(root: &std::path::Path, store: IslesStore) -> Shared {
        let mut shared = Shared::bare(IslesVersion {
            hooks: Arc::new(Hooks),
            tables: VersionTables {
                target_changelist: 647_472,
                game_version: "1.14.7",
                active_huntpass_id: "",
                recent_huntpass_seasons: &[],
                patchnotes_release_version: "",
                bounty_item_grant_data: &[],
                store_sections: &[],
                seasonal_coin: None,
                masked_journey_nodes: &[],
                starter_manifest: StarterManifest {
                    bootstrap_version: "",
                    stacked: &[],
                    quick_emojis: None,
                },
                switch_defaults: &[],
            },
            routes: RouteSet::V1147,
        });
        shared.settings = Settings::from_pairs([
            ("LAUNCHER_UPDATE_ROOT", root.to_str().unwrap_or_default()),
            ("UPDATE_PUBLISHER_ALLOWED_IPS", "127.0.0.1"),
            ("UPDATE_PUBLISHER_API_KEY", "publisher-key"),
        ]);
        shared.store = Some(store);
        shared
    }

    fn publish(shared: &Shared, guard: &SigningKey) {
        let now = now_ms();
        let payload = json!({
            "schema": 1, "manifestId": "11111111-2222-3333-4444-555555555555", "sequence": 3, "channel": "stable",
            "platform": "windows-x86_64", "targetChangelist": 647_472, "minimumLauncherVersion": "0.1.65",
            "issuedAt": iso(now - 3_600_000), "expiresAt": iso(now + 86_400_000),
            "artifacts": [{"name": "Game.exe", "role": "GAME", "size": 1, "sha256": "ab".repeat(32), "required": true}],
            "executableMemoryPolicy": {"maximumPrivateExecutableBytes": 1000, "maximumWritableExecutableBytes": 100, "allowlistedModuleHashes": []}
        })
        .to_string()
        .into_bytes();
        let envelope = json!({
            "schema": 1, "keyId": "guard-2026-10", "payloadBase64": base64_encode(&payload),
            "signature": base64_encode(&guard.sign(&payload).to_bytes()),
        });
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-update-api-key"),
            HeaderValue::from_static("publisher-key"),
        );
        let req = Req {
            method: "POST".into(),
            path: "/".into(),
            headers,
            query: vec![],
            peer: Some(service_kit::serve::ConnectionInfo {
                peer: "127.0.0.1:5".parse().unwrap(),
                local: "127.0.0.1:6".parse().unwrap(),
            }),
            body: Some(envelope),
            raw: None,
        };
        let params = Params(vec![
            ("channel".into(), "stable".into()),
            ("platform".into(), "windows-x86_64".into()),
        ]);
        let response = publish_guard(&Ctx::new(shared).unwrap(), &req, &params);
        assert_eq!(response.status(), 201);
    }

    fn body(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    fn signals(extra: Value) -> Value {
        let mut base = json!({
            "gameProcessId": 4242, "gameStartedAt": "2026-10-14T10:00:00.000Z", "moduleCount": 188,
            "moduleSetDigest": "d".repeat(64), "privateExecutableBytes": 0, "writableExecutableBytes": 0,
            "unexpectedModuleCount": 0, "protectedPageMismatchCount": 0, "debuggerPresent": false
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    }

    /// A heartbeat body signed over the material the session expects.
    fn signed(session: &Value, key: &SigningKey, sequence: i64, signals: &Value) -> Map<String, Value> {
        let observed = iso(now_ms());
        let parsed = Signals::parse(Some(signals)).unwrap();
        let material = signature_material(
            GuardRole::Client,
            session["guardSessionId"].as_str().unwrap(),
            sequence,
            session["challenge"].as_str().unwrap(),
            session["manifestId"].as_str().unwrap(),
            session["manifestSequence"].as_u64().unwrap(),
            &observed,
            &parsed,
        );
        body(json!({
            "guardSessionId": session["guardSessionId"], "sequence": sequence, "observedAt": observed,
            "signature": base64_encode(&key.sign(material.as_bytes()).to_bytes()), "signals": signals,
        }))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_session_is_opened_scored_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let node = NodeIdentity::standalone();
        let config = DatabaseConfig::Sqlite {
            file: dir.path().join("isles.db"),
        };
        let home = Arc::new(StandaloneHome { identity: node.clone() });
        let store = IslesStore::open(&config, &DatabaseOptions::default(), home, &node).unwrap();
        let shared = shared(dir.path(), store.clone());
        let (runtime, guard) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]));
        *TEST_TRUSTED.write().unwrap() = Some(Trusted::with_keys(runtime.verifying_key(), guard.verifying_key()));
        let account = AccountId::new(&node.node);
        let launcher = SigningKey::from_bytes(&[3; 32]);
        let public = base64_encode(launcher.verifying_key().as_bytes());
        let start_body = |extra: Value| {
            let mut base = json!({
                "role": "client", "channel": "stable", "launcherVersion": "0.1.70",
                "manifestId": "11111111-2222-3333-4444-555555555555", "manifestSequence": 3, "publicKey": public,
            });
            for (k, v) in extra.as_object().unwrap() {
                base[k] = v.clone();
            }
            body(base)
        };

        // No manifest published yet
        let failure = start(&shared, &store, &account, &start_body(json!({})))
            .await
            .unwrap_err();
        assert_eq!(
            (failure.status, failure.code.as_str()),
            (503, "GUARD_MANIFEST_UNAVAILABLE")
        );
        publish(&shared, &guard);

        for (extra, status, code) in [
            (json!({ "role": "worker" }), 400, "GUARD_REPORT_INVALID"),
            (json!({ "manifestSequence": 4 }), 409, "GUARD_MANIFEST_MISMATCH"),
            (json!({ "manifestId": "other" }), 409, "GUARD_MANIFEST_MISMATCH"),
            (json!({ "launcherVersion": "0.1.64" }), 409, "GUARD_MANIFEST_MISMATCH"),
            (json!({ "launcherVersion": "nope" }), 409, "GUARD_MANIFEST_MISMATCH"),
            (json!({ "channel": "beta" }), 503, "GUARD_MANIFEST_UNAVAILABLE"),
            (json!({ "publicKey": "AAAA" }), 400, "GUARD_REPORT_INVALID"),
        ] {
            let failure = start(&shared, &store, &account, &start_body(extra.clone()))
                .await
                .unwrap_err();
            assert_eq!((failure.status, failure.code.as_str()), (status, code), "{extra}");
        }

        let first = start(&shared, &store, &account, &start_body(json!({}))).await.unwrap();
        assert_eq!(
            first.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "schema",
                "role",
                "guardSessionId",
                "challenge",
                "manifestId",
                "manifestSequence",
                "expiresAt",
                "heartbeatIntervalMs"
            ]
        );
        assert_eq!(
            (first["heartbeatIntervalMs"].clone(), first["role"].clone()),
            (json!(15_000), json!("client"))
        );
        let waiting = status(&store, &account, first["guardSessionId"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(
            waiting.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "schema",
                "role",
                "guardSessionId",
                "status",
                "riskScore",
                "manifestId",
                "manifestSequence",
                "expiresAt"
            ]
        );
        assert_eq!(waiting["status"], "CHALLENGED");

        // A clean heartbeat is healthy, and the chain moves on with a new challenge
        let report = signals(json!({}));
        let accepted = heartbeat(&shared, &store, &account, &signed(&first, &launcher, 1, &report))
            .await
            .unwrap();
        assert_eq!(accepted["status"], "HEALTHY");
        assert_eq!(
            (accepted["riskScore"].clone(), accepted["reasons"].clone()),
            (json!(0), json!([]))
        );
        assert_eq!(accepted["accepted"], true);
        let next = accepted["nextChallenge"].as_str().unwrap();
        assert_ne!(next, first["challenge"].as_str().unwrap());
        // The same sequence again is a replay
        let replay = heartbeat(&shared, &store, &account, &signed(&first, &launcher, 1, &report))
            .await
            .unwrap_err();
        assert_eq!((replay.status, replay.code.as_str()), (401, "GUARD_SESSION_INVALID"));
        // The old challenge no longer signs
        let stale = heartbeat(&shared, &store, &account, &signed(&first, &launcher, 2, &report))
            .await
            .unwrap_err();
        assert_eq!((stale.status, stale.code.as_str()), (401, "GUARD_SIGNATURE_INVALID"));
        let mut session = first.clone();
        session["challenge"] = json!(next);

        // Another account's key, a role that isn't the session's, a clock that is off, a report that is not one
        let other = AccountId::new(&node.node);
        let stranger = heartbeat(&shared, &store, &other, &signed(&session, &launcher, 2, &report))
            .await
            .unwrap_err();
        assert_eq!(stranger.code, "GUARD_SESSION_INVALID");
        let mut wrong_role = signed(&session, &launcher, 2, &report);
        wrong_role.insert("role".into(), json!("host"));
        assert_eq!(
            heartbeat(&shared, &store, &account, &wrong_role)
                .await
                .unwrap_err()
                .code,
            "GUARD_SESSION_INVALID"
        );
        let mut skewed = signed(&session, &launcher, 2, &report);
        skewed.insert("observedAt".into(), json!(iso(now_ms() - 120_000)));
        assert_eq!(
            heartbeat(&shared, &store, &account, &skewed).await.unwrap_err().code,
            "GUARD_REPORT_INVALID"
        );
        let mut no_signals = signed(&session, &launcher, 2, &report);
        no_signals.remove("signals");
        assert_eq!(
            heartbeat(&shared, &store, &account, &no_signals)
                .await
                .unwrap_err()
                .code,
            "GUARD_REPORT_INVALID"
        );
        let forged = SigningKey::from_bytes(&[9; 32]);
        assert_eq!(
            heartbeat(&shared, &store, &account, &signed(&session, &forged, 2, &report))
                .await
                .unwrap_err()
                .code,
            "GUARD_SIGNATURE_INVALID"
        );

        // A debugger and an unsigned module are a degraded verdict with their reasons; the module is a finding
        let module = json!({ "fileName": "overlay.dll", "signer": "unsigned", "sha256": "A".repeat(64), "size": 10 });
        let bad =
            signals(json!({ "debuggerPresent": true, "unexpectedModuleCount": 1, "unexpectedModules": [module] }));
        let degraded = heartbeat(&shared, &store, &account, &signed(&session, &launcher, 2, &bad))
            .await
            .unwrap();
        assert_eq!(degraded["status"], "DEGRADED");
        assert_eq!(degraded["riskScore"], 65);
        assert_eq!(degraded["reasons"], json!(["DEBUGGER", "UNEXPECTED_UNSIGNED:1"]));
        let (findings, total) = store
            .read(|conn, repos| repos.launcher_guard.list_findings(conn, Some("UNSIG"), 0, 10))
            .await
            .unwrap();
        assert_eq!((total, findings.len()), (1, 1));
        assert_eq!(
            (
                findings[0].session_count,
                findings[0].account_count,
                findings[0].sha256.as_str()
            ),
            (1, 1, "a".repeat(64).as_str())
        );
        assert_eq!(
            store
                .read(|conn, repos| repos.launcher_guard.list_findings(conn, Some("nobody"), 0, 10))
                .await
                .unwrap()
                .1,
            0
        );

        // A session in a new process is a changed process, whatever else it reports
        let mut session = session.clone();
        session["challenge"] = json!(degraded["nextChallenge"]);
        let moved = signals(json!({ "gameProcessId": 4243 }));
        let verdict = heartbeat(&shared, &store, &account, &signed(&session, &launcher, 3, &moved))
            .await
            .unwrap();
        assert_eq!(
            (verdict["status"].clone(), verdict["riskScore"].clone()),
            (json!("DEGRADED"), json!(100))
        );
        assert_eq!(verdict["reasons"], json!(["PROCESS_CHANGED"]));

        // The listing and the session record, as the dashboard reads them
        let id = first["guardSessionId"].as_str().unwrap().to_owned();
        let record = store
            .read(move |conn, repos| repos.launcher_guard.find(conn, &id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((record.last_sequence, record.status), (3, GuardStatus::Degraded));
        assert_eq!(record.history.len(), 3);
        assert_eq!(record.history[0]["riskScore"], 100);
        assert_eq!(record.launcher_version, "0.1.70");
        let (sessions, total) = store
            .read(|conn, repos| {
                repos
                    .launcher_guard
                    .list_sessions(conn, Some(GuardStatus::Degraded), Some(90.0), 0, 10)
            })
            .await
            .unwrap();
        assert_eq!((total, sessions.len(), sessions[0].history.len()), (1, 1, 0));
        assert_eq!(
            store
                .read(|conn, repos| repos.launcher_guard.list_sessions(conn, None, None, 0, 10))
                .await
                .unwrap()
                .1,
            1
        );

        // A new session of the same role revokes the old one, and the healthy lookup needs a recent heartbeat
        let second = start(&shared, &store, &account, &start_body(json!({}))).await.unwrap();
        let old = first["guardSessionId"].as_str().unwrap().to_owned();
        let revoked = store
            .read(move |conn, repos| repos.launcher_guard.find(conn, &old))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(revoked.status, GuardStatus::Revoked);
        assert!(
            heartbeat(&shared, &store, &account, &signed(&first, &launcher, 9, &report))
                .await
                .is_err()
        );
        let healthy_id = second["guardSessionId"].as_str().unwrap().to_owned();
        let (lookup, who) = (healthy_id.clone(), account.clone());
        let now = iso(now_ms());
        let found = store
            .read(move |conn, repos| {
                repos
                    .launcher_guard
                    .find_healthy_for_account(conn, &who, &lookup, GuardRole::Client, &now)
            })
            .await
            .unwrap();
        assert!(found.is_none(), "still challenged");
        let healthy = heartbeat(&shared, &store, &account, &signed(&second, &launcher, 1, &report))
            .await
            .unwrap();
        assert_eq!(healthy["status"], "HEALTHY");
        let (lookup, who) = (healthy_id.clone(), account.clone());
        let now = iso(now_ms());
        let found = store
            .read(move |conn, repos| {
                repos
                    .launcher_guard
                    .find_healthy_for_account(conn, &who, &lookup, GuardRole::Client, &now)
            })
            .await
            .unwrap();
        assert!(found.is_some());
        let (lookup, who) = (healthy_id, account.clone());
        let later = iso(now_ms() + 60_000);
        let found = store
            .read(move |conn, repos| {
                repos
                    .launcher_guard
                    .find_healthy_for_account(conn, &who, &lookup, GuardRole::Client, &later)
            })
            .await
            .unwrap();
        assert!(found.is_none(), "a heartbeat older than 45 seconds is not healthy");
        let missing = status(&store, &account, "nothing").await.unwrap_err();
        assert_eq!((missing.status, missing.code.as_str()), (401, "GUARD_SESSION_INVALID"));
        *TEST_TRUSTED.write().unwrap() = None;
    }
}
