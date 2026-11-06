// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Launcher Guard (public spec §11): a session against the signed manifest, signed heartbeats about the game
//! process, the score and verdict, the findings, and the dashboard's read-only views.

use std::cmp::Ordering;

use axum::response::Response;
use http::StatusCode;
use module_api::{Actor, ActorContext};
use rustic_isles::Call;
use rustic_isles_store::repo::reforged::{
    GuardHeartbeat, GuardRole, GuardSighting, GuardStatus, LauncherGuardSessionRecord,
};
use rustic_isles_store::{AccountId, RepoError};
use serde_json::{Map, Value};
use service_kit::js::safe_integer;

use crate::admin::{AdminContext, admin_error, limit_of, page_of, skip_of, user_id_of};
use crate::guard_manifest::{GuardPayload, PLATFORM, ed25519_verify, is_version};
use crate::launcher_auth::{LauncherCaller, launcher_sign_in};
use crate::util::{self, MAX_SAFE, iso, lenient_base64, now_ms, object};

const SESSION_TTL_MS: i64 = 5 * 60_000;
const MAX_MODULES: usize = 32;
const MAX_MODULE_SIZE: i64 = 1 << 40;

// ------------------------------------------------------------------ the report (§11.4)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnexpectedModule {
    pub file_name: String,
    pub signer: String,
    /// Lower-cased
    pub sha256: String,
    pub size: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signals {
    pub game_process_id: i64,
    pub module_count: i64,
    pub private_executable_bytes: i64,
    pub writable_executable_bytes: i64,
    pub unexpected_module_count: i64,
    pub protected_page_mismatch_count: i64,
    pub game_started_at: String,
    pub module_set_digest: String,
    pub debugger_present: bool,
    pub modules: Option<Vec<UnexpectedModule>>,
    /// Every key as sent, the module list normalized in its place
    pub stored: Value,
    raw: Map<String, Value>,
}

fn is_file_name(text: &str) -> bool {
    (1..=260).contains(&util::utf16_len(text))
        && text != "."
        && text != ".."
        && !text
            .chars()
            .any(|c| c <= '\u{1f}' || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'))
}

fn is_signer(text: &str) -> bool {
    (1..=256).contains(&util::utf16_len(text)) && !text.chars().any(|c| c <= '\u{1f}')
}

fn module(value: &Value) -> Option<UnexpectedModule> {
    let map = value.as_object()?;
    let file_name = map.get("fileName")?.as_str().filter(|n| is_file_name(n))?;
    let signer = map.get("signer")?.as_str().filter(|s| is_signer(s))?;
    let sha256 = map.get("sha256")?.as_str().filter(|s| util::is_hex(s, 64))?;
    let size_value = map.get("size")?;
    if !size_value.is_number() {
        return None;
    }
    let size = safe_integer(Some(size_value)).filter(|n| (0..=MAX_MODULE_SIZE).contains(n))?;
    Some(UnexpectedModule {
        file_name: file_name.to_owned(),
        signer: signer.to_owned(),
        sha256: sha256.to_lowercase(),
        size,
    })
}

impl UnexpectedModule {
    fn to_json(&self) -> Value {
        object(vec![
            ("fileName", Value::from(self.file_name.clone())),
            ("signer", Value::from(self.signer.clone())),
            ("sha256", Value::from(self.sha256.clone())),
            ("size", Value::from(self.size)),
        ])
    }
}

impl Signals {
    /// A report as §11.4 checks it; anything else is invalid.
    pub fn parse(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        let count = |name: &str| safe_integer(map.get(name)).filter(|n| *n >= 0);
        let started = map
            .get("gameStartedAt")?
            .as_str()
            .filter(|t| service_kit::js::date_parse(t).is_some())?;
        let digest = map.get("moduleSetDigest")?.as_str().filter(|d| util::is_hex(d, 64))?;
        let debugger = map.get("debuggerPresent")?.as_bool()?;
        let modules = match map.get("unexpectedModules") {
            None | Some(Value::Null) => None,
            Some(Value::Array(list)) if list.len() <= MAX_MODULES => {
                Some(list.iter().map(module).collect::<Option<Vec<_>>>()?)
            }
            Some(_) => return None,
        };
        let mut stored = map.clone();
        if let Some(list) = &modules {
            stored.insert(
                "unexpectedModules".into(),
                Value::Array(list.iter().map(UnexpectedModule::to_json).collect()),
            );
        }
        Some(Self {
            game_process_id: count("gameProcessId")?,
            module_count: count("moduleCount")?,
            private_executable_bytes: count("privateExecutableBytes")?,
            writable_executable_bytes: count("writableExecutableBytes")?,
            unexpected_module_count: count("unexpectedModuleCount")?,
            protected_page_mismatch_count: count("protectedPageMismatchCount")?,
            game_started_at: started.to_owned(),
            module_set_digest: digest.to_owned(),
            debugger_present: debugger,
            modules,
            stored: Value::Object(stored),
            raw: map.clone(),
        })
    }

    fn js(&self, name: &str) -> String {
        service_kit::js::string(self.raw.get(name))
    }
}

/// The 17 lines a heartbeat signs.
#[allow(clippy::too_many_arguments)]
pub fn signature_material(
    role: &str,
    session_id: &str,
    sequence: u64,
    challenge: &str,
    manifest_id: &str,
    manifest_sequence: u64,
    observed_at: &str,
    signals: &Signals,
) -> String {
    [
        "MYSTIC-GUARD-2".to_owned(),
        role.to_owned(),
        session_id.to_owned(),
        sequence.to_string(),
        challenge.to_owned(),
        manifest_id.to_owned(),
        manifest_sequence.to_string(),
        observed_at.to_owned(),
        signals.js("gameProcessId"),
        signals.game_started_at.clone(),
        signals.js("moduleCount"),
        signals.module_set_digest.to_lowercase(),
        signals.js("privateExecutableBytes"),
        signals.js("writableExecutableBytes"),
        signals.js("unexpectedModuleCount"),
        signals.js("protectedPageMismatchCount"),
        if signals.debugger_present { "1" } else { "0" }.to_owned(),
    ]
    .join("\n")
}

// ------------------------------------------------------------------ scoring (§11.5)

fn module_counts(signals: &Signals) -> (i64, i64) {
    let listed_signed = signals.modules.as_ref().map_or(0, |list| {
        list.iter()
            .filter(|m| {
                let signer = m.signer.trim().to_ascii_lowercase();
                signer != "unsigned" && signer != "invalid"
            })
            .count()
    });
    let signed = i64::try_from(listed_signed)
        .unwrap_or(i64::MAX)
        .min(signals.unexpected_module_count);
    (signals.unexpected_module_count - signed, signed)
}

/// The signals' score and reasons against the manifest's memory limits.
pub fn explain_signals(signals: &Signals, max_private: i64, max_writable: i64) -> (i64, Vec<String>) {
    let mut score = 0i64;
    let mut reasons = Vec::new();
    let (untrusted, signed) = module_counts(signals);
    if signals.debugger_present {
        score += 50;
        reasons.push("DEBUGGER".to_owned());
    }
    if untrusted > 0 {
        score += (untrusted.saturating_mul(15)).min(50);
        reasons.push(format!("UNEXPECTED_UNSIGNED:{untrusted}"));
    }
    if signed > 0 {
        score += (signed.saturating_mul(2)).min(10);
        reasons.push(format!("UNEXPECTED_SIGNED:{signed}"));
    }
    let pages = signals.protected_page_mismatch_count;
    if pages > 0 {
        score += (pages.saturating_mul(30)).min(60);
        reasons.push(format!("PAGE_MISMATCH:{pages}"));
    }
    if signals.writable_executable_bytes > max_writable {
        score += 60;
        reasons.push("WRITABLE_EXEC_OVER".to_owned());
    }
    if signals.private_executable_bytes > max_private {
        score += 40;
        reasons.push("PRIVATE_EXEC_OVER".to_owned());
    }
    (score.min(100), reasons)
}

pub fn hard_violation(signals: &Signals, max_private: i64, max_writable: i64) -> bool {
    let (untrusted, _) = module_counts(signals);
    signals.debugger_present
        || untrusted > 0
        || signals.protected_page_mismatch_count > 0
        || signals.writable_executable_bytes > max_writable
        || signals.private_executable_bytes > max_private
}

/// Continuity against the previous report and the session's last sequence.
pub fn explain_continuity(previous: Option<&Signals>, current: &Signals, last_sequence: u64) -> (i64, Vec<String>) {
    let Some(previous) = previous else {
        return (0, Vec::new());
    };
    let same_process = previous.raw.get("gameProcessId") == current.raw.get("gameProcessId")
        && previous.game_started_at == current.game_started_at;
    if !same_process {
        return (100, vec!["PROCESS_CHANGED".to_owned()]);
    }
    if last_sequence >= 2 && previous.module_set_digest != current.module_set_digest {
        return (50, vec!["MODULE_SET_CHANGED".to_owned()]);
    }
    (0, Vec::new())
}

pub fn guard_status(hard: bool, risk: i64) -> GuardStatus {
    if !hard && risk < 50 {
        GuardStatus::Healthy
    } else {
        GuardStatus::Degraded
    }
}

struct Semver {
    core: [u64; 3],
    pre: Option<Vec<String>>,
}

fn is_ident_text(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

fn parse_semver(text: &str) -> Option<Semver> {
    let text = util::trim(text);
    let (rest, build) = match text.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (text, None),
    };
    if build.is_some_and(|b| !is_ident_text(b)) {
        return None;
    }
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    if pre.is_some_and(|p| !is_ident_text(p)) {
        return None;
    }
    let groups: Vec<&str> = core.split('.').collect();
    if groups.len() != 3 {
        return None;
    }
    let mut out = [0u64; 3];
    for (i, g) in groups.iter().enumerate() {
        if g.is_empty() || !g.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out[i] = g.parse().ok()?;
    }
    Some(Semver {
        core: out,
        pre: pre.map(|p| p.split('.').map(str::to_owned).collect()),
    })
}

fn numeric(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn compare_numeric(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// The launcher's own version order (not auth's semver): a prerelease list that ends first is the greater.
pub fn version_at_least(actual: &str, minimum: &str) -> bool {
    let (Some(a), Some(m)) = (parse_semver(actual), parse_semver(minimum)) else {
        return false;
    };
    match a.core.cmp(&m.core) {
        Ordering::Greater => return true,
        Ordering::Less => return false,
        Ordering::Equal => {}
    }
    let (a_pre, m_pre) = match (a.pre, m.pre) {
        (None, _) => return true,
        (Some(_), None) => return false,
        (Some(a), Some(m)) => (a, m),
    };
    let mut i = 0;
    loop {
        match (a_pre.get(i), m_pre.get(i)) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(x), Some(y)) => {
                let order = match (numeric(x), numeric(y)) {
                    (true, true) => compare_numeric(x, y),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => x.as_bytes().cmp(y.as_bytes()),
                };
                match order {
                    Ordering::Greater => return true,
                    Ordering::Less => return false,
                    Ordering::Equal => i += 1,
                }
            }
        }
    }
}

// ------------------------------------------------------------------ routes (§11.1 to §11.6)

fn refused(code: &str, message: &str) -> Response {
    util::launcher_error(code, message)
}

fn failed(error: impl std::fmt::Display) -> Response {
    tracing::error!("Unhandled Launcher Guard error: {error}");
    refused("INTERNAL", "Launcher Guard failed.")
}

struct Entered {
    caller: LauncherCaller,
    account: AccountId,
    body: Option<Value>,
    store: std::sync::Arc<rustic_isles_store::IslesStore>,
}

async fn enter(call: &mut Call) -> Result<Entered, Response> {
    let body = util::read_json(&mut call.request).await?;
    let caller = launcher_sign_in(&call.shared, call.request.headers()).await?;
    let Some(store) = util::store_of(&call.shared) else {
        return Err(util::no_store(call.route, &call.request_id));
    };
    let account = caller
        .account_id
        .parse::<AccountId>()
        .map_err(|e| failed(format!("account id {}: {e}", caller.account_id)))?;
    Ok(Entered {
        caller,
        account,
        body,
        store,
    })
}

fn player(account: &AccountId) -> ActorContext {
    ActorContext::new(Actor::Player {
        account: account.global().clone(),
    })
}

fn current_manifest(shared: &rustic_isles::Shared, channel: &str) -> Option<GuardPayload> {
    let (_, payload) = crate::updates::published_guard_manifest(shared, channel, PLATFORM)?;
    let expires = service_kit::js::date_parse(&payload.expires_at)?;
    (expires > now_ms()).then_some(payload)
}

fn parse_role(value: Option<&Value>) -> Option<GuardRole> {
    match value {
        None | Some(Value::Null) => Some(GuardRole::Client),
        Some(Value::String(text)) if text == "client" => Some(GuardRole::Client),
        Some(Value::String(text)) if text == "host" => Some(GuardRole::Host),
        _ => None,
    }
}

pub async fn start(mut call: Call) -> Response {
    let entered = match enter(&mut call).await {
        Ok(entered) => entered,
        Err(answer) => return answer,
    };
    let body = entered.body.as_ref();
    let Some(role) = parse_role(util::field(body, "role")) else {
        return refused("GUARD_REPORT_INVALID", "The Launcher Guard role is invalid.");
    };
    let channel = util::str_field(body, "channel").unwrap_or("stable").to_owned();
    let version = util::str_field(body, "launcherVersion").unwrap_or("").to_owned();
    let Some(manifest) = current_manifest(&call.shared, &channel) else {
        return refused(
            "GUARD_MANIFEST_UNAVAILABLE",
            "No current signed Guard manifest is available.",
        );
    };
    let current = util::str_field(body, "manifestId") == Some(manifest.manifest_id.as_str())
        && safe_integer(util::field(body, "manifestSequence")) == Some(manifest.sequence)
        && is_version(&version)
        && version_at_least(&version, &manifest.minimum_launcher_version);
    if !current {
        return refused(
            "GUARD_MANIFEST_MISMATCH",
            "The launcher runtime manifest is not current.",
        );
    }
    let public_key = util::str_field(body, "publicKey").unwrap_or("");
    if util::field(body, "publicKey").is_none_or(|v| !v.is_string()) || lenient_base64(public_key).len() != 32 {
        return refused("GUARD_REPORT_INVALID", "The ephemeral Guard public key is invalid.");
    }
    let now = now_ms();
    let session_id = util::random_uuid();
    let challenge = util::random_base64url::<24>();
    let record = LauncherGuardSessionRecord {
        guard_session_id: session_id.clone(),
        account: entered.account.clone(),
        role,
        public_key_base64: public_key.to_owned(),
        manifest_id: manifest.manifest_id.clone(),
        manifest_sequence: u64::try_from(manifest.sequence).unwrap_or(0),
        channel,
        launcher_version: version,
        challenge: challenge.clone(),
        last_sequence: 0,
        status: GuardStatus::Challenged,
        risk_score: 0.0,
        created_at: iso(now),
        expires_at: iso(now + SESSION_TTL_MS),
        last_heartbeat_at: None,
        last_signals: None,
        reasons: Vec::new(),
        history: Vec::new(),
    };
    let account = entered.account.clone();
    let expires = record.expires_at.clone();
    let written = entered
        .store
        .account_write(&entered.account, &player(&entered.account), move |tx, repos| {
            repos
                .launcher_guard
                .revoke_for_account(tx, &account, role, &record.created_at)?;
            repos.launcher_guard.create(tx, &record)
        })
        .await;
    if let Err(e) = written {
        return failed(e);
    }
    util::ok(&object(vec![
        ("schema", Value::from(1)),
        ("role", Value::from(role.as_str())),
        ("guardSessionId", Value::from(session_id)),
        ("challenge", Value::from(challenge)),
        ("manifestId", Value::from(manifest.manifest_id)),
        ("manifestSequence", Value::from(manifest.sequence)),
        ("expiresAt", Value::from(expires)),
        ("heartbeatIntervalMs", Value::from(15_000)),
    ]))
}

fn whole_sequence(value: Option<&Value>) -> Option<u64> {
    let n = service_kit::js::number(value);
    (n.is_finite() && n.fract() == 0.0 && (1.0..=MAX_SAFE).contains(&n)).then(|| {
        // Whole and inside 2^53
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = n as u64;
        v
    })
}

fn is_session_id(text: &str) -> bool {
    text.len() == 36 && text.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

pub async fn heartbeat(mut call: Call) -> Response {
    let entered = match enter(&mut call).await {
        Ok(entered) => entered,
        Err(answer) => return answer,
    };
    let body = entered.body.as_ref();
    let invalid = || refused("GUARD_REPORT_INVALID", "The Launcher Guard heartbeat is invalid.");
    let session_id = util::str_field(body, "guardSessionId").unwrap_or("").to_owned();
    let sequence = whole_sequence(util::field(body, "sequence"));
    let observed = util::str_field(body, "observedAt").map(str::to_owned);
    let observed_ms = observed.as_deref().and_then(service_kit::js::date_parse);
    let signature = util::str_field(body, "signature").unwrap_or("").to_owned();
    let (Some(sequence), Some(observed), Some(observed_ms)) = (sequence, observed, observed_ms) else {
        return invalid();
    };
    if !is_session_id(&session_id) || !(80..=100).contains(&util::utf16_len(&signature)) {
        return invalid();
    }
    let now = now_ms();
    if (observed_ms - now).abs() > 60_000 {
        return invalid();
    }
    let Some(signals) = util::field(body, "signals").and_then(Signals::parse) else {
        return refused("GUARD_REPORT_INVALID", "The Launcher Guard report is invalid.");
    };
    let id = session_id.clone();
    let session = match entered
        .store
        .read(move |conn, repos| repos.launcher_guard.find(conn, &id))
        .await
    {
        Ok(session) => session,
        Err(e) => return failed(e),
    };
    let session_invalid = || {
        refused(
            "GUARD_SESSION_INVALID",
            "The Launcher Guard session is invalid or expired.",
        )
    };
    let Some(session) = session else {
        return session_invalid();
    };
    let live = session.account == entered.account
        && session.last_sequence < sequence
        && !matches!(session.status, GuardStatus::Revoked | GuardStatus::Expired)
        && service_kit::js::date_parse(&session.expires_at).is_some_and(|at| at > now);
    if !live {
        return session_invalid();
    }
    if let Some(sent) =
        parse_role(util::field(body, "role")).filter(|_| util::field(body, "role").is_some_and(|r| !r.is_null()))
        && sent != session.role
    {
        return refused(
            "GUARD_SESSION_INVALID",
            "The Launcher Guard role does not match the session.",
        );
    }
    let material = signature_material(
        session.role.as_str(),
        &session.guard_session_id,
        sequence,
        &session.challenge,
        &session.manifest_id,
        session.manifest_sequence,
        &observed,
        &signals,
    );
    let key = lenient_base64(&session.public_key_base64);
    let sig = lenient_base64(&signature);
    if key.len() != 32 || !ed25519_verify(&key, material.as_bytes(), &sig) {
        return refused(
            "GUARD_SIGNATURE_INVALID",
            "The Launcher Guard heartbeat could not be authenticated.",
        );
    }
    let manifest = current_manifest(&call.shared, &session.channel).filter(|m| {
        m.manifest_id == session.manifest_id && u64::try_from(m.sequence).ok() == Some(session.manifest_sequence)
    });
    let Some(manifest) = manifest else {
        return refused(
            "GUARD_MANIFEST_MISMATCH",
            "The Launcher Guard manifest changed; restart the game.",
        );
    };
    let (max_private, max_writable) = (
        manifest.max_private_executable_bytes,
        manifest.max_writable_executable_bytes,
    );
    let previous = session.last_signals.as_ref().and_then(Signals::parse);
    let (signal_score, mut reasons) = explain_signals(&signals, max_private, max_writable);
    let (continuity, more) = explain_continuity(previous.as_ref(), &signals, session.last_sequence);
    reasons.extend(more);
    let risk = (signal_score + continuity).min(100);
    let status = guard_status(hard_violation(&signals, max_private, max_writable), risk);
    let at = iso(now_ms());
    let next = util::random_base64url::<24>();
    let expires = iso(now_ms() + SESSION_TTL_MS);
    #[allow(clippy::cast_precision_loss)]
    let risk_f = risk as f64;
    let beat = GuardHeartbeat {
        guard_session_id: session.guard_session_id.clone(),
        account: entered.account.clone(),
        expected_challenge: session.challenge.clone(),
        sequence,
        next_challenge: next.clone(),
        status,
        risk_score: risk_f,
        heartbeat_at: at.clone(),
        expires_at: expires.clone(),
        signals: signals.stored.clone(),
        reasons: reasons.clone(),
        history_entry: object(vec![
            ("at", Value::from(at.clone())),
            ("status", Value::from(status.as_str())),
            ("riskScore", Value::from(risk)),
            (
                "reasons",
                Value::Array(reasons.iter().cloned().map(Value::from).collect()),
            ),
        ]),
    };
    let advanced = entered
        .store
        .account_write(&entered.account, &player(&entered.account), move |tx, repos| {
            repos.launcher_guard.advance(tx, &beat)
        })
        .await;
    match advanced {
        Ok(Some(_)) => {}
        Ok(None) => {
            return refused(
                "GUARD_SEQUENCE_CONFLICT",
                "The Launcher Guard heartbeat was superseded.",
            );
        }
        Err(e) => return failed(e),
    }
    record_findings(&entered, &signals, previous.as_ref(), &at).await;
    util::ok(&object(vec![
        ("schema", Value::from(1)),
        ("accepted", Value::Bool(true)),
        ("role", Value::from(session.role.as_str())),
        ("status", Value::from(status.as_str())),
        ("riskScore", Value::from(risk)),
        ("reasons", Value::Array(reasons.into_iter().map(Value::from).collect())),
        ("nextChallenge", Value::from(next)),
        ("expiresAt", Value::from(expires)),
    ]))
}

async fn record_findings(entered: &Entered, signals: &Signals, previous: Option<&Signals>, at: &str) {
    let Some(modules) = &signals.modules else {
        return;
    };
    let before: Vec<(String, String)> = previous
        .and_then(|p| p.modules.as_ref())
        .map(|l| l.iter().map(|m| (m.file_name.clone(), m.sha256.clone())).collect())
        .unwrap_or_default();
    let mut seen: Vec<(String, String)> = Vec::new();
    for m in modules {
        let pair = (m.file_name.clone(), m.sha256.clone());
        if seen.contains(&pair) {
            continue;
        }
        seen.push(pair.clone());
        let sighting = GuardSighting {
            file_name: m.file_name.clone(),
            sha256: m.sha256.clone(),
            signer: m.signer.clone(),
            size: u64::try_from(m.size).unwrap_or(0),
            account: entered.account.clone(),
            seen_at: at.to_owned(),
            new_in_session: !before.contains(&pair),
        };
        let result: Result<(), RepoError> = entered
            .store
            .account_write(&entered.account, &player(&entered.account), move |tx, repos| {
                repos.launcher_guard.record_finding_sighting(tx, &sighting)
            })
            .await;
        if let Err(e) = result {
            tracing::warn!(
                "[GuardFindings] could not record {} for account={}: {e}",
                m.file_name,
                entered.caller.account_id
            );
        }
    }
}

pub async fn status(mut call: Call) -> Response {
    let entered = match enter(&mut call).await {
        Ok(entered) => entered,
        Err(answer) => return answer,
    };
    let id = call.params.get("guardSessionId").unwrap_or("").to_owned();
    let lookup = id.clone();
    let session = match entered
        .store
        .read(move |conn, repos| repos.launcher_guard.find(conn, &lookup))
        .await
    {
        Ok(session) => session,
        Err(e) => return failed(e),
    };
    let Some(session) = session.filter(|s| s.account == entered.account) else {
        return refused("GUARD_SESSION_INVALID", "Launcher Guard session not found.");
    };
    let mut pairs = vec![
        ("schema", Value::from(1)),
        ("role", Value::from(session.role.as_str())),
        ("guardSessionId", Value::from(id)),
        ("status", Value::from(session.status.as_str())),
        ("riskScore", util::number(session.risk_score)),
        ("manifestId", Value::from(session.manifest_id.clone())),
        ("manifestSequence", Value::from(session.manifest_sequence)),
    ];
    if let Some(at) = &session.last_heartbeat_at {
        pairs.push(("lastHeartbeatAt", Value::from(at.clone())));
    }
    pairs.push(("expiresAt", Value::from(session.expires_at.clone())));
    util::ok(&object(pairs))
}

// ------------------------------------------------------------------ the dashboard's views (§11.7)

fn dashboard_session(
    conn: &mut rustic_isles_store::AnyConnection,
    repos: &rustic_isles_store::Repositories,
    s: &LauncherGuardSessionRecord,
) -> Value {
    object(vec![
        ("guardSessionId", Value::from(s.guard_session_id.clone())),
        ("accountId", Value::from(user_id_of(conn, repos, &s.account))),
        ("role", Value::from(s.role.as_str())),
        ("status", Value::from(s.status.as_str())),
        ("riskScore", util::number(s.risk_score)),
        (
            "reasons",
            Value::Array(s.reasons.iter().cloned().map(Value::from).collect()),
        ),
        ("channel", Value::from(s.channel.clone())),
        ("launcherVersion", Value::from(s.launcher_version.clone())),
        ("manifestId", Value::from(s.manifest_id.clone())),
        ("manifestSequence", Value::from(s.manifest_sequence)),
        ("lastSequence", Value::from(s.last_sequence)),
        ("createdAt", Value::from(s.created_at.clone())),
        (
            "lastHeartbeatAt",
            s.last_heartbeat_at.clone().map_or(Value::Null, Value::from),
        ),
        ("expiresAt", Value::from(s.expires_at.clone())),
        ("lastSignals", s.last_signals.clone().unwrap_or(Value::Null)),
    ])
}

fn query_invalid(message: &str) -> Response {
    admin_error(StatusCode::BAD_REQUEST, "GUARD_QUERY_INVALID", message)
}

pub async fn sessions(ctx: AdminContext) -> Response {
    let query = &ctx.request.query;
    let status_text = util::single(query, "status")
        .map(|s| util::trim(s).to_owned())
        .unwrap_or_default();
    let status = if status_text.is_empty() {
        None
    } else {
        match GuardStatus::parse(&status_text) {
            Some(status) => Some(status),
            None => {
                return query_invalid("status must be one of CHALLENGED, HEALTHY, DEGRADED, REVOKED, EXPIRED");
            }
        }
    };
    let risk_text = util::single(query, "minRisk")
        .map(|s| util::trim(s).to_owned())
        .unwrap_or_default();
    let min_risk = if risk_text.is_empty() {
        None
    } else {
        let n = service_kit::js::number(Some(&Value::String(risk_text)));
        if !(n.is_finite() && n.fract() == 0.0 && (0.0..=100.0).contains(&n)) {
            return query_invalid("minRisk must be a whole number from 0 to 100");
        }
        Some(n)
    };
    let page = page_of(&ctx.request);
    let limit = limit_of(&ctx.request, 25, 100);
    let skip = skip_of(page, limit);
    let read = ctx
        .store
        .read(move |conn, repos| {
            let (list, total) = repos
                .launcher_guard
                .list_sessions(conn, status, min_risk, skip, limit)?;
            let shown: Vec<Value> = list.iter().map(|s| dashboard_session(conn, repos, s)).collect();
            Ok((shown, total))
        })
        .await;
    match read {
        Ok((list, total)) => util::ok(&object(vec![
            ("sessions", Value::Array(list)),
            ("total", Value::from(total)),
            ("page", Value::from(page)),
            ("limit", Value::from(limit)),
        ])),
        Err(e) => {
            tracing::error!("[ADMIN] guard sessions: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn session(ctx: AdminContext) -> Response {
    let id = ctx.param("guardSessionId").to_owned();
    let not_found = || {
        admin_error(
            StatusCode::NOT_FOUND,
            "GUARD_SESSION_NOT_FOUND",
            "No Guard session has that id.",
        )
    };
    if id.is_empty() {
        return not_found();
    }
    let read = ctx
        .store
        .read(move |conn, repos| {
            let Some(s) = repos.launcher_guard.find(conn, &id)? else {
                return Ok(None);
            };
            Ok(Some((dashboard_session(conn, repos, &s), s.history)))
        })
        .await;
    match read {
        Ok(Some((session, history))) => {
            util::ok(&object(vec![("session", session), ("history", Value::Array(history))]))
        }
        Ok(None) => not_found(),
        Err(e) => {
            tracing::error!("[ADMIN] guard session: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn findings(ctx: AdminContext) -> Response {
    let signer = util::single(&ctx.request.query, "signer")
        .map(|s| util::take_chars(util::trim(s), 100))
        .filter(|s| !s.is_empty());
    let page = page_of(&ctx.request);
    let limit = limit_of(&ctx.request, 25, 100);
    let skip = skip_of(page, limit);
    let read = ctx
        .store
        .read(move |conn, repos| repos.launcher_guard.list_findings(conn, signer.as_deref(), skip, limit))
        .await;
    match read {
        Ok((list, total)) => {
            let shown: Vec<Value> = list
                .into_iter()
                .map(|f| {
                    object(vec![
                        ("fileName", Value::from(f.file_name)),
                        ("sha256", Value::from(f.sha256)),
                        ("signer", Value::from(f.signer)),
                        ("size", Value::from(f.size)),
                        ("firstSeenAt", Value::from(f.first_seen_at)),
                        ("lastSeenAt", Value::from(f.last_seen_at)),
                        ("sessionCount", Value::from(f.session_count)),
                        ("accountCount", Value::from(f.account_count)),
                    ])
                })
                .collect();
            util::ok(&object(vec![
                ("findings", Value::Array(shown)),
                ("total", Value::from(total)),
                ("page", Value::from(page)),
                ("limit", Value::from(limit)),
            ]))
        }
        Err(e) => {
            tracing::error!("[ADMIN] guard findings: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn report(extra: &[(&str, Value)]) -> Value {
        let mut base = json!({
            "gameProcessId": 42, "gameStartedAt": "2026-10-20T00:00:00Z", "moduleCount": 10,
            "moduleSetDigest": "A".repeat(64), "privateExecutableBytes": 512 * 1024,
            "writableExecutableBytes": 32 * 1024, "unexpectedModuleCount": 0,
            "protectedPageMismatchCount": 0, "debuggerPresent": false
        });
        for (k, v) in extra {
            base[*k] = v.clone();
        }
        base
    }

    fn signals(extra: &[(&str, Value)]) -> Signals {
        Signals::parse(&report(extra)).unwrap()
    }

    const MIB: i64 = 1024 * 1024;

    #[test]
    fn material_and_roles() {
        let s = signals(&[]);
        let text = signature_material("client", "sid", 4, "ch", "mid", 2, "obs", &s);
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines.len(), 17);
        assert_eq!((lines[0], lines[1], lines[3]), ("MYSTIC-GUARD-2", "client", "4"));
        assert_eq!(lines[11], "a".repeat(64));
        assert_eq!(lines[16], "0");
        assert_ne!(text, signature_material("host", "sid", 4, "ch", "mid", 2, "obs", &s));
        assert_eq!(parse_role(None), Some(GuardRole::Client));
        assert_eq!(parse_role(Some(&json!("host"))), Some(GuardRole::Host));
        assert_eq!(parse_role(Some(&json!("worker"))), None);
    }

    #[test]
    fn scoring() {
        let clean = signals(&[]);
        assert_eq!(explain_signals(&clean, MIB, 64 * 1024), (0, vec![]));
        assert!(!hard_violation(&clean, MIB, 64 * 1024));
        let writable = signals(&[("writableExecutableBytes", json!(64 * 1024 + 1))]);
        assert_eq!(explain_signals(&writable, MIB, 64 * 1024).0, 60);
        let private = signals(&[("privateExecutableBytes", json!(MIB + 1))]);
        assert_eq!(explain_signals(&private, MIB, 64 * 1024).0, 40);
        let page = signals(&[("protectedPageMismatchCount", json!(1))]);
        assert_eq!(explain_signals(&page, MIB, 64 * 1024).0, 30);
        let one = signals(&[("unexpectedModuleCount", json!(1))]);
        assert_eq!(explain_signals(&one, MIB, 64 * 1024).0, 15);
        assert_eq!(
            guard_status(hard_violation(&one, MIB, 64 * 1024), 15),
            GuardStatus::Degraded
        );
        let five = signals(&[("unexpectedModuleCount", json!(5))]);
        assert_eq!(explain_signals(&five, MIB, 64 * 1024).0, 50);
        let everything = signals(&[
            ("debuggerPresent", json!(true)),
            ("unexpectedModuleCount", json!(2)),
            ("protectedPageMismatchCount", json!(3)),
            ("writableExecutableBytes", json!(MIB)),
            ("privateExecutableBytes", json!(2 * MIB)),
        ]);
        let (score, reasons) = explain_signals(&everything, MIB, 64 * 1024);
        assert_eq!(score, 100);
        assert_eq!(
            reasons,
            [
                "DEBUGGER",
                "UNEXPECTED_UNSIGNED:2",
                "PAGE_MISMATCH:3",
                "WRITABLE_EXEC_OVER",
                "PRIVATE_EXEC_OVER"
            ]
        );
        let m = |signer: &str| json!({"fileName":"a.dll","signer":signer,"sha256":"b".repeat(64),"size":1});
        let signed = signals(&[
            ("unexpectedModuleCount", json!(1)),
            ("unexpectedModules", json!([m("Microsoft")])),
        ]);
        assert_eq!(
            explain_signals(&signed, MIB, 64 * 1024),
            (2, vec!["UNEXPECTED_SIGNED:1".to_owned()])
        );
        assert!(!hard_violation(&signed, MIB, 64 * 1024));
        for unsigned in ["unsigned", " Invalid ", "UNSIGNED"] {
            let s = signals(&[
                ("unexpectedModuleCount", json!(1)),
                ("unexpectedModules", json!([m(unsigned)])),
            ]);
            assert!(hard_violation(&s, MIB, 64 * 1024), "{unsigned}");
        }
        let beyond = signals(&[("unexpectedModules", json!([m("unsigned")]))]);
        assert_eq!(explain_signals(&beyond, MIB, 64 * 1024).0, 0);
        let six: Vec<Value> = (0..6).map(|_| m("Vendor")).collect();
        let nine = signals(&[("unexpectedModuleCount", json!(9)), ("unexpectedModules", json!(six))]);
        assert_eq!(
            explain_signals(&nine, MIB, 64 * 1024),
            (
                55,
                vec!["UNEXPECTED_UNSIGNED:3".to_owned(), "UNEXPECTED_SIGNED:6".to_owned()]
            )
        );
    }

    #[test]
    fn continuity() {
        let a = signals(&[]);
        let digest = signals(&[("moduleSetDigest", json!("b".repeat(64)))]);
        assert_eq!(explain_continuity(Some(&a), &digest, 1).0, 0);
        assert_eq!(
            explain_continuity(Some(&a), &digest, 2),
            (50, vec!["MODULE_SET_CHANGED".to_owned()])
        );
        let process = signals(&[("gameProcessId", json!(43))]);
        assert_eq!(
            explain_continuity(Some(&a), &process, 5),
            (100, vec!["PROCESS_CHANGED".to_owned()])
        );
        assert_eq!(explain_continuity(None, &a, 5).0, 0);
    }

    #[test]
    fn reports() {
        let m = json!({"fileName":"a.dll","signer":"s","sha256":"B".repeat(64),"size":1});
        let mut bad_names = vec![
            json!(""),
            json!(".."),
            json!("a/b.dll"),
            json!("a\\b.dll"),
            json!("a:b.dll"),
        ];
        bad_names.push(json!("a\u{1}b"));
        bad_names.push(json!("x".repeat(261)));
        bad_names.push(json!(5));
        for name in bad_names {
            let mut entry = m.clone();
            entry["fileName"] = name;
            assert!(Signals::parse(&report(&[("unexpectedModules", json!([entry]))])).is_none());
        }
        for signer in [json!(""), json!("a\nb"), json!("s".repeat(257))] {
            let mut entry = m.clone();
            entry["signer"] = signer;
            assert!(Signals::parse(&report(&[("unexpectedModules", json!([entry]))])).is_none());
        }
        for sha in [json!("z".repeat(64)), json!("a".repeat(63))] {
            let mut entry = m.clone();
            entry["sha256"] = sha;
            assert!(Signals::parse(&report(&[("unexpectedModules", json!([entry]))])).is_none());
        }
        for size in [json!(-1), json!(1.5), json!("1"), json!((1i64 << 40) + 1)] {
            let mut entry = m.clone();
            entry["size"] = size;
            assert!(Signals::parse(&report(&[("unexpectedModules", json!([entry]))])).is_none());
        }
        let mut no_size = m.clone();
        no_size.as_object_mut().unwrap().remove("size");
        assert!(Signals::parse(&report(&[("unexpectedModules", json!([no_size]))])).is_none());
        let many: Vec<Value> = (0..33).map(|_| m.clone()).collect();
        assert!(Signals::parse(&report(&[("unexpectedModules", json!(many))])).is_none());
        let ok: Vec<Value> = (0..32).map(|_| m.clone()).collect();
        let parsed = Signals::parse(&report(&[("unexpectedModules", json!(ok)), ("extra", json!(1))])).unwrap();
        assert_eq!(parsed.stored["extra"], 1);
        assert_eq!(parsed.stored["unexpectedModules"][0]["sha256"], "b".repeat(64));
    }

    #[test]
    fn versions() {
        let yes = [
            ("0.1.65", "0.1.65"),
            ("0.1.66", "0.1.65"),
            ("1.0.0", "0.9.9"),
            ("0.1.65", "0.1.65-beta.1"),
            ("0.1.65-beta.2", "0.1.65-beta.1"),
            ("0.1.65-beta.1", "0.1.65-beta.1"),
            ("0.1.65-alpha", "0.1.65-1"),
            ("0.1.65-rc", "0.1.65-rc.1"),
            ("0.1.65+build", "0.1.65"),
        ];
        for (a, m) in yes {
            assert!(version_at_least(a, m), "{a} >= {m}");
        }
        let no = [
            ("0.1.64", "0.1.65"),
            ("0.1.65-beta.1", "0.1.65"),
            ("0.1.65-1", "0.1.65-alpha"),
            ("0.1.65-rc.1", "0.1.65-rc"),
            ("x", "0.1.0"),
            ("0.1.0", "y"),
            ("0.1", "0.1.0"),
        ];
        for (a, m) in no {
            assert!(!version_at_least(a, m), "{a} >= {m}");
        }
    }
}
