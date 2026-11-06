// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Controller and managed-install state machines, certificate issuance from public keys,
//! `DH6` guarded two-person approval, and offline backoff (`docs/remote-management.md` §§3–7).

use std::collections::BTreeMap;
use std::path::Path;

use module_api::network::{NodeId, Region};
use network::cluster::{ClusterCa, DEFAULT_NODE_DAYS};
use rand::Rng;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, SanType, SerialNumber, SubjectPublicKeyInfo,
};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use url::Url;

use crate::RemoteError;
use crate::enroll::{
    DEFAULT_ENROLL_TTL_SECS, EnrollmentInvite, MAX_ENROLL_TTL_SECS, PendingEnrollment, generate_enrollment_token,
    sha256_hex, validate_controller_url, validate_enrollment_token, validate_label,
};
use crate::merge::merge_managed_host_toml;
use crate::wire::{
    CommandOutcome, EnrollAnswer, EnrollRequest, InstallReport, OperatorEnvelope, QueuedRemoteCommand, RemoteCommand,
    RemoteLogEntry, RemoteRequest, RemoteResponse, RemoteRole, SharedConfigBundle,
};

const NODE_URI_PREFIX: &str = "urn:dainty-lass:node:";
const REGION_URI_PREFIX: &str = "urn:dainty-lass:region:";

/// Generate a fresh P-256 keypair on a managed install.
/// Returns `(private_key_pem, public_key_spki_pem)`. The private key stays on the managed install
/// (`0600`) and never crosses the wire.
pub fn generate_node_keypair_and_spki_pem() -> Result<(String, String), RemoteError> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .map_err(|e| RemoteError::Crypto(format!("generate P-256 keypair: {e}")))?;
    let priv_pem = key.serialize_pem().replace("\r\n", "\n");
    let pub_pem = key.public_key_pem().replace("\r\n", "\n");
    Ok((priv_pem, pub_pem))
}

fn extract_private_key_block(ca_pem: &str) -> Option<&str> {
    let begin = "-----BEGIN PRIVATE KEY-----";
    let end = "-----END PRIVATE KEY-----";
    let from = ca_pem.find(begin)?;
    let to = from + ca_pem[from..].find(end)? + end.len();
    Some(&ca_pem[from..to])
}

/// Sign a `DaintyLass node` X.509 certificate for a managed install's `SubjectPublicKeyInfo` PEM
/// using the controller's `ClusterCa`, without the managed install's private key ever leaving the
/// managed install.
/// Returns `(node_cert_pem, cert_sha256_fp)`.
pub fn sign_enrolled_node_cert(
    ca: &ClusterCa,
    node: &NodeId,
    region: &Region,
    hosts: &[String],
    public_key_pem: &str,
    days: u32,
) -> Result<(String, String), RemoteError> {
    let spki = SubjectPublicKeyInfo::from_pem(public_key_pem)
        .map_err(|e| RemoteError::InvalidInput(format!("invalid public_key_pem: {e}")))?;
    let ca_full_pem = ca.to_pem();
    let ca_key_pem = extract_private_key_block(&ca_full_pem)
        .ok_or_else(|| RemoteError::Crypto("cluster CA is missing private key".to_string()))?;
    let ca_key =
        KeyPair::from_pem(ca_key_pem).map_err(|e| RemoteError::Crypto(format!("parse cluster CA key: {e}")))?;
    let issuer = Issuer::from_ca_cert_pem(ca.certificate_pem(), ca_key)
        .map_err(|e| RemoteError::Crypto(format!("build CA issuer: {e}")))?;

    let mut names: Vec<SanType> = Vec::new();
    let mut host_list: Vec<String> = hosts.to_vec();
    if host_list.is_empty() {
        host_list.push("127.0.0.1".to_string());
        host_list.push("localhost".to_string());
    }
    for h in &host_list {
        if let Ok(ip) = h.parse::<std::net::IpAddr>() {
            names.push(SanType::IpAddress(ip));
        } else if let Ok(dns) = h.as_str().try_into() {
            names.push(SanType::DnsName(dns));
        }
    }
    for (prefix, val) in [(NODE_URI_PREFIX, node.as_str()), (REGION_URI_PREFIX, region.as_str())] {
        let uri = format!("{prefix}{val}")
            .try_into()
            .map_err(|e| RemoteError::Crypto(format!("build SAN URI: {e}")))?;
        names.push(SanType::URI(uri));
    }

    let mut serial_bytes = [0u8; 20];
    rand::rng().fill_bytes(&mut serial_bytes);
    serial_bytes[0] = (serial_bytes[0] & 0x3f) | 0x40;

    let mut params = CertificateParams::default();
    params.serial_number = Some(SerialNumber::from_slice(&serial_bytes));
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, node.as_str());
    dn.push(DnType::OrganizationName, "DaintyLass node");
    params.distinguished_name = dn;
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth, ExtendedKeyUsagePurpose::ClientAuth];
    params.subject_alt_names = names;
    let _ = days;

    let cert = params
        .signed_by(&spki, &issuer)
        .map_err(|e| RemoteError::Crypto(format!("sign node certificate: {e}")))?;
    let fp = sha256_hex(cert.der());
    let pem = cert.pem().replace("\r\n", "\n");
    Ok((pem, fp))
}

/// Compute the lowercase 64-hex SHA-256 fingerprint of the first certificate in a PEM string.
pub fn cert_pem_fingerprint(cert_pem: &str) -> Result<String, RemoteError> {
    let der = CertificateDer::from_pem_slice(cert_pem.as_bytes())
        .map_err(|e| RemoteError::InvalidInput(format!("invalid certificate PEM: {e}")))?;
    Ok(sha256_hex(&der))
}

/// Live reachability/enrollment state of a managed install on the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrolledInstallState {
    /// Seen within the offline threshold and running normally.
    Online,
    /// Seen recently, but its last report says degraded or stopped.
    Degraded,
    /// Not seen within the offline threshold, or never.
    Offline,
    /// Revoked by the controller.
    Revoked,
}

/// Controller-side record for an enrolled install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrolledInstallRecord {
    /// Its node id.
    pub node: NodeId,
    /// Its region.
    pub region: Region,
    /// Display label.
    pub label: String,
    /// When it enrolled, in Unix milliseconds.
    pub enrolled_at_ms: i64,
    /// When it last synced, in Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_ms: Option<i64>,
    /// SHA-256 of the node certificate the controller signed for it.
    pub cert_fp: String,
    /// Its gateway's public URL, if it reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
    /// SHA-256 of its gateway certificate, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_cert_fp: Option<String>,
    /// Whether it was revoked.
    pub revoked: bool,
    /// Config revision it last applied.
    pub applied_revision: u64,
    /// Its last status report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<InstallReport>,
    /// Commands waiting for its next sync.
    #[serde(default)]
    pub queued_commands: Vec<QueuedRemoteCommand>,
    /// Outcomes of commands it ran, the last 100.
    #[serde(default)]
    pub completed_outcomes: Vec<CommandOutcome>,
}

impl EnrolledInstallRecord {
    /// Its state at `now_ms`; unseen for longer than `offline_threshold_ms` is offline.
    pub fn state(&self, now_ms: i64, offline_threshold_ms: i64) -> EnrolledInstallState {
        if self.revoked {
            return EnrolledInstallState::Revoked;
        }
        let Some(last_seen) = self.last_seen_ms else {
            return EnrolledInstallState::Offline;
        };
        if now_ms.saturating_sub(last_seen) > offline_threshold_ms {
            return EnrolledInstallState::Offline;
        }
        if let Some(report) = &self.last_report
            && matches!(
                report.server_state,
                crate::wire::ServerRunState::Degraded | crate::wire::ServerRunState::Stopped
            )
        {
            return EnrolledInstallState::Degraded;
        }
        EnrolledInstallState::Online
    }
}

/// Pending guarded remote action awaiting a second operator's approval (`DH6`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingGuardedRemoteAction {
    /// Command id.
    pub id: String,
    /// Installs it runs on.
    pub target_nodes: Vec<NodeId>,
    /// What it does.
    pub command: RemoteCommand,
    /// Who asked for it.
    pub requested_by: OperatorEnvelope,
    /// When it was asked for, in Unix milliseconds.
    pub created_at_ms: i64,
    /// Earliest time it may run, in Unix milliseconds.
    pub execute_after_ms: i64,
    /// Whether a second operator must approve it.
    pub requires_second_approver: bool,
    /// Who approved it, once approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<OperatorEnvelope>,
    /// Whether it was cancelled.
    pub canceled: bool,
    /// Whether it was approved and queued to the installs.
    pub executed: bool,
}

/// What submitting a command did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubmitCommandResult {
    /// Queued for the target installs.
    Queued {
        /// Command id.
        command_id: String,
        /// Installs it was queued for.
        target_nodes: Vec<NodeId>,
    },
    /// Held as a guarded action until another operator approves it after its delay.
    PendingGuarded {
        /// The held action.
        guarded: Box<PendingGuardedRemoteAction>,
    },
}

/// Persistent controller state (`<workspace>/.workspace/remote/controller-state.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteControllerState {
    /// The controller's node id.
    pub controller_node: NodeId,
    /// The controller's region.
    pub controller_region: Region,
    /// The controller's URL, as invites carry it.
    pub controller_url: String,
    /// SHA-256 of the controller's certificate.
    pub controller_cert_fp: String,
    /// Configuration every managed install applies.
    pub config_bundle: SharedConfigBundle,
    /// Enrollment invites not yet redeemed.
    #[serde(default)]
    pub pending_enrollments: Vec<PendingEnrollment>,
    /// Enrolled installs by node id.
    #[serde(default)]
    pub installs: BTreeMap<String, EnrolledInstallRecord>,
    /// Guarded actions not yet run or cancelled.
    #[serde(default)]
    pub pending_guarded: Vec<PendingGuardedRemoteAction>,
    /// Sequence number the next command id takes.
    #[serde(default)]
    pub next_command_seq: u64,
}

impl RemoteControllerState {
    /// A controller with revision 1 of `initial_shared_host_toml`, no installs and no invites.
    pub fn new(
        controller_node: NodeId,
        controller_region: Region,
        controller_url: String,
        controller_cert_fp: String,
        initial_shared_host_toml: String,
    ) -> Result<Self, RemoteError> {
        let canonical_url = validate_controller_url(&controller_url)?;
        let bundle = SharedConfigBundle {
            revision: 1,
            shared_host_toml: initial_shared_host_toml,
            node_overrides: BTreeMap::new(),
            invites: Vec::new(),
            presets: BTreeMap::new(),
            runtime_manifest_sha256: None,
            gamedata_pack_sha256: None,
            server_name: None,
        };
        bundle.validate()?;
        Ok(Self {
            controller_node,
            controller_region,
            controller_url: canonical_url,
            controller_cert_fp,
            config_bundle: bundle,
            pending_enrollments: Vec::new(),
            installs: BTreeMap::new(),
            pending_guarded: Vec::new(),
            next_command_seq: 1,
        })
    }

    /// Reads the state file.
    pub fn load(path: &Path) -> Result<Self, RemoteError> {
        let text = std::fs::read_to_string(path).map_err(|e| RemoteError::Io(format!("{}: {e}", path.display())))?;
        serde_json::from_str(&text).map_err(|e| RemoteError::InvalidInput(format!("{}: {e}", path.display())))
    }

    /// Writes the state file through a temporary file and a rename, creating its folder.
    pub fn save(&self, path: &Path) -> Result<(), RemoteError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| RemoteError::Io(format!("{}: {e}", parent.display())))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| RemoteError::InvalidInput(format!("serialize controller state: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| RemoteError::Io(format!("{}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path).map_err(|e| RemoteError::Io(format!("{}: {e}", path.display())))?;
        Ok(())
    }

    /// Mint a one-time enrollment invite URI and record only `sha256(token)` on the controller.
    pub fn mint_enrollment_invite(
        &mut self,
        node: NodeId,
        region: Region,
        label: Option<String>,
        now_ms: i64,
        ttl_secs: Option<i64>,
    ) -> Result<(String, EnrollmentInvite), RemoteError> {
        if node == self.controller_node {
            return Err(RemoteError::InvalidInput(
                "managed install node_id cannot be the controller's own node_id".to_string(),
            ));
        }
        let ttl = ttl_secs.unwrap_or(DEFAULT_ENROLL_TTL_SECS);
        if !(60..=MAX_ENROLL_TTL_SECS).contains(&ttl) {
            return Err(RemoteError::InvalidInput(format!(
                "enrollment TTL must be between 60 and {MAX_ENROLL_TTL_SECS} seconds"
            )));
        }
        let checked_label = match label {
            Some(l) => Some(validate_label(&l)?),
            None => None,
        };
        let token = generate_enrollment_token();
        let token_bytes = validate_enrollment_token(&token)?;
        let token_sha256_hex = sha256_hex(&token_bytes);

        let invite = EnrollmentInvite {
            url: self.controller_url.clone(),
            controller_fp: self.controller_cert_fp.clone(),
            node: node.clone(),
            region: region.clone(),
            token,
            label: checked_label.clone(),
        };
        let uri = invite.to_uri()?;

        self.pending_enrollments.push(PendingEnrollment {
            token_sha256_hex,
            node,
            region,
            label: checked_label,
            created_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(ttl.saturating_mul(1000)),
            used: false,
        });
        Ok((uri, invite))
    }

    /// Redeem a one-time enrollment request (`POST /__node/enroll`), signing the managed install's
    /// public key with `ca` and registering the install.
    pub fn redeem_enrollment(
        &mut self,
        ca: &ClusterCa,
        req: &EnrollRequest,
        now_ms: i64,
    ) -> Result<EnrollAnswer, RemoteError> {
        req.validate()?;
        let token_bytes = validate_enrollment_token(&req.token)?;
        let incoming_hash = sha256_hex(&token_bytes);

        let mut matched_idx: Option<usize> = None;
        for (idx, entry) in self.pending_enrollments.iter().enumerate() {
            if bool::from(entry.token_sha256_hex.as_bytes().ct_eq(incoming_hash.as_bytes())) {
                matched_idx = Some(idx);
                break;
            }
        }

        let idx = matched_idx.ok_or(RemoteError::InvalidEnrollmentToken)?;
        let entry = &self.pending_enrollments[idx];
        if entry.used {
            return Err(RemoteError::EnrollmentTokenUsed);
        }
        if now_ms > entry.expires_at_ms {
            return Err(RemoteError::EnrollmentTokenExpired);
        }
        if entry.node != req.node || entry.region != req.region {
            return Err(RemoteError::InvalidEnrollmentToken);
        }

        let mut hosts = vec!["127.0.0.1".to_string(), "localhost".to_string()];
        if let Some(gw_url) = &req.gateway_url
            && let Ok(parsed) = Url::parse(gw_url)
            && let Some(h) = parsed.host_str()
            && !hosts.iter().any(|existing| existing == h)
        {
            hosts.push(h.to_string());
        }

        let (node_cert_pem, cert_fp) = sign_enrolled_node_cert(
            ca,
            &req.node,
            &req.region,
            &hosts,
            &req.public_key_pem,
            DEFAULT_NODE_DAYS,
        )?;

        self.pending_enrollments[idx].used = true;
        let label = req
            .label
            .clone()
            .or_else(|| self.pending_enrollments[idx].label.clone())
            .unwrap_or_else(|| req.node.as_str().to_string());

        let record = EnrolledInstallRecord {
            node: req.node.clone(),
            region: req.region.clone(),
            label,
            enrolled_at_ms: now_ms,
            last_seen_ms: Some(now_ms),
            cert_fp,
            gateway_url: req.gateway_url.clone(),
            gateway_cert_fp: req.gateway_cert_fp.clone(),
            revoked: false,
            applied_revision: 0,
            last_report: None,
            queued_commands: Vec::new(),
            completed_outcomes: Vec::new(),
        };
        self.installs.insert(req.node.as_str().to_string(), record);

        let answer = EnrollAnswer {
            controller_node: self.controller_node.clone(),
            controller_region: self.controller_region.clone(),
            controller_cert_fp: self.controller_cert_fp.clone(),
            ca_cert_pem: ca.certificate_pem().to_string(),
            node_cert_pem,
            initial_revision: self.config_bundle.revision,
        };
        answer.validate()?;
        Ok(answer)
    }

    /// Returns `true` if `node` is an enrolled, non-revoked managed install.
    pub fn is_authorized_peer(&self, node: &NodeId) -> bool {
        self.installs.get(node.as_str()).is_some_and(|rec| !rec.revoked)
    }

    /// Revoke an enrolled managed install (`docs/remote-management.md` §5.3).
    pub fn revoke_install(&mut self, node: &NodeId) -> Result<(), RemoteError> {
        let rec = self
            .installs
            .get_mut(node.as_str())
            .ok_or_else(|| RemoteError::UnknownInstall(node.as_str().to_string()))?;
        rec.revoked = true;
        rec.queued_commands.clear();
        Ok(())
    }

    /// Update the shared `host.toml` baseline and increment `config_bundle.revision`.
    pub fn update_shared_config(&mut self, shared_host_toml: String) -> Result<u64, RemoteError> {
        let mut next = self.config_bundle.clone();
        next.revision = next.revision.saturating_add(1);
        next.shared_host_toml = shared_host_toml;
        next.validate()?;
        self.config_bundle = next;
        Ok(self.config_bundle.revision)
    }

    /// Set a per-install `host.toml` override on the controller and increment `config_bundle.revision`.
    pub fn set_node_override(&mut self, node: &NodeId, override_toml: String) -> Result<u64, RemoteError> {
        if !self.installs.contains_key(node.as_str()) {
            return Err(RemoteError::UnknownInstall(node.as_str().to_string()));
        }
        let mut next = self.config_bundle.clone();
        next.revision = next.revision.saturating_add(1);
        if override_toml.trim().is_empty() {
            next.node_overrides.remove(node.as_str());
        } else {
            next.node_overrides.insert(node.as_str().to_string(), override_toml);
        }
        next.validate()?;
        self.config_bundle = next;
        Ok(self.config_bundle.revision)
    }

    /// Submit a remote operator command targeting one or more enrolled installs.
    /// Enforces `DH2` role checks and `DH6` guarded action rules.
    pub fn submit_command(
        &mut self,
        target_nodes: Vec<NodeId>,
        operator: OperatorEnvelope,
        command: RemoteCommand,
        now_ms: i64,
        guarded_delay_secs: u64,
        require_second_approver: bool,
    ) -> Result<SubmitCommandResult, RemoteError> {
        if target_nodes.is_empty() {
            return Err(RemoteError::InvalidInput(
                "at least one target install must be specified".to_string(),
            ));
        }
        operator.validate()?;
        command.validate()?;
        if !operator.role.allows(command.required_role()) {
            return Err(RemoteError::PermissionDenied(format!(
                "role {:?} cannot run command requiring {:?}",
                operator.role,
                command.required_role()
            )));
        }
        for node in &target_nodes {
            let rec = self
                .installs
                .get(node.as_str())
                .ok_or_else(|| RemoteError::UnknownInstall(node.as_str().to_string()))?;
            if rec.revoked {
                return Err(RemoteError::RevokedPeer(node.as_str().to_string()));
            }
        }

        let seq = self.next_command_seq;
        self.next_command_seq = self.next_command_seq.saturating_add(1);

        if command.is_guarded() && (require_second_approver || guarded_delay_secs > 0) {
            let guarded_id = format!("guard-{seq}");
            let pending = PendingGuardedRemoteAction {
                id: guarded_id,
                target_nodes,
                command,
                requested_by: operator,
                created_at_ms: now_ms,
                execute_after_ms: now_ms.saturating_add((guarded_delay_secs as i64).saturating_mul(1000)),
                requires_second_approver: require_second_approver,
                approved_by: None,
                canceled: false,
                executed: false,
            };
            self.pending_guarded.push(pending.clone());
            return Ok(SubmitCommandResult::PendingGuarded {
                guarded: Box::new(pending),
            });
        }

        let command_id = format!("cmd-{seq}");
        for node in &target_nodes {
            if let Some(rec) = self.installs.get_mut(node.as_str()) {
                rec.queued_commands.push(QueuedRemoteCommand {
                    command_id: command_id.clone(),
                    operator: operator.clone(),
                    command: command.clone(),
                    approved_by: None,
                });
            }
        }
        Ok(SubmitCommandResult::Queued {
            command_id,
            target_nodes,
        })
    }

    /// Approve a pending `DH6` guarded action as a second operator with a distinct `key_id`.
    pub fn approve_guarded(
        &mut self,
        guarded_id: &str,
        approver: OperatorEnvelope,
        now_ms: i64,
    ) -> Result<PendingGuardedRemoteAction, RemoteError> {
        approver.validate()?;
        if !approver.role.allows(RemoteRole::Admin) {
            return Err(RemoteError::PermissionDenied(
                "approver must hold Admin or Owner role (DH6)".to_string(),
            ));
        }
        let item = self
            .pending_guarded
            .iter_mut()
            .find(|g| g.id == guarded_id)
            .ok_or_else(|| RemoteError::InvalidInput(format!("unknown guarded action: {guarded_id}")))?;
        if item.canceled || item.executed {
            return Err(RemoteError::InvalidInput(
                "guarded action is already canceled or executed".to_string(),
            ));
        }
        if item.requested_by.key_id == approver.key_id {
            return Err(RemoteError::GuardedApprovalRequired(
                "second approver must have a distinct key_id from requested_by (DH6)".to_string(),
            ));
        }
        if now_ms < item.execute_after_ms {
            return Err(RemoteError::GuardedApprovalRequired(
                "guarded action cooling-off delay has not elapsed yet (DH6)".to_string(),
            ));
        }
        item.approved_by = Some(approver.clone());
        item.executed = true;
        let cloned = item.clone();

        for node in &cloned.target_nodes {
            if let Some(rec) = self.installs.get_mut(node.as_str())
                && !rec.revoked
            {
                rec.queued_commands.push(QueuedRemoteCommand {
                    command_id: cloned.id.clone(),
                    operator: cloned.requested_by.clone(),
                    command: cloned.command.clone(),
                    approved_by: Some(approver.clone()),
                });
            }
        }
        Ok(cloned)
    }

    /// Cancel a pending `DH6` guarded action while it waits.
    pub fn cancel_guarded(
        &mut self,
        guarded_id: &str,
        actor: &OperatorEnvelope,
    ) -> Result<PendingGuardedRemoteAction, RemoteError> {
        actor.validate()?;
        if !actor.role.allows(RemoteRole::Admin) {
            return Err(RemoteError::PermissionDenied(
                "canceling a guarded action requires Admin or Owner role".to_string(),
            ));
        }
        let item = self
            .pending_guarded
            .iter_mut()
            .find(|g| g.id == guarded_id)
            .ok_or_else(|| RemoteError::InvalidInput(format!("unknown guarded action: {guarded_id}")))?;
        if item.executed {
            return Err(RemoteError::InvalidInput(
                "guarded action has already been executed".to_string(),
            ));
        }
        item.canceled = true;
        Ok(item.clone())
    }

    /// Handle an incoming `RemoteRequest` on the controller from an mTLS-authenticated `peer`.
    /// The caller identity comes from the mTLS peer certificate (`from`), never solely from the JSON body.
    pub fn handle_remote_request(
        &mut self,
        authenticated_peer: &NodeId,
        req: RemoteRequest,
        now_ms: i64,
    ) -> Result<RemoteResponse, RemoteError> {
        req.validate()?;
        let rec = self
            .installs
            .get_mut(authenticated_peer.as_str())
            .ok_or_else(|| RemoteError::UnknownInstall(authenticated_peer.as_str().to_string()))?;
        if rec.revoked {
            return Err(RemoteError::RevokedPeer(authenticated_peer.as_str().to_string()));
        }

        match req {
            RemoteRequest::Sync {
                node,
                applied_revision,
                report,
                completed_commands,
            } => {
                if &node != authenticated_peer {
                    return Err(RemoteError::PeerIdentityMismatch {
                        cert_node: authenticated_peer.as_str().to_string(),
                        body_node: node.as_str().to_string(),
                    });
                }
                rec.last_seen_ms = Some(now_ms);
                rec.applied_revision = applied_revision;
                rec.label = report.label.clone();
                rec.last_report = Some(*report);

                for outcome in completed_commands {
                    rec.queued_commands.retain(|q| q.command_id != outcome.command_id);
                    rec.completed_outcomes.push(outcome);
                    if rec.completed_outcomes.len() > 100 {
                        rec.completed_outcomes.remove(0);
                    }
                }

                let bundle = if applied_revision < self.config_bundle.revision {
                    Some(self.config_bundle.clone())
                } else {
                    None
                };
                let pending_commands = rec.queued_commands.clone();
                Ok(RemoteResponse::SyncAck {
                    controller_revision: self.config_bundle.revision,
                    bundle,
                    pending_commands,
                })
            }
            _ => Err(RemoteError::InvalidInput(
                "controller only accepts Sync requests from managed installs".to_string(),
            )),
        }
    }
}

/// Trait implemented by the host runtime (`crates/host` / `crates/hostkit` / `launcher`) so
/// `crates/remote` can execute commands and read local status without depending on AGPL crates.
pub trait RemoteHostHandler: Send + Sync {
    /// This install's status report at `applied_revision`.
    fn build_report(&self, applied_revision: u64) -> Result<InstallReport, RemoteError>;
    /// This install's `host.toml`.
    fn read_local_host_toml(&self) -> Result<String, RemoteError>;
    /// Writes the merged `host.toml` as `revision`.
    fn write_merged_host_toml(&self, merged_toml: &str, revision: u64) -> Result<(), RemoteError>;
    /// Runs one command and reports how it went.
    fn execute_command(&self, cmd: &QueuedRemoteCommand) -> Result<CommandOutcome, RemoteError>;
    /// Log entries after `since_seq`, at most `limit`, and the sequence number to ask from next.
    fn fetch_logs(&self, since_seq: u64, limit: u32) -> Result<(u64, Vec<RemoteLogEntry>), RemoteError>;
}

/// Compute exponential backoff delay in milliseconds when the controller is unreachable
/// (`docs/remote-management.md` §7: 5s -> 10s -> 20s -> 40s -> max 60s).
pub fn offline_backoff_delay_ms(consecutive_failures: u32) -> u64 {
    if consecutive_failures == 0 {
        return 15_000;
    }
    let shift = consecutive_failures.saturating_sub(1).min(4);
    let secs = (5u64.saturating_mul(1u64 << shift)).min(60);
    secs * 1000
}

/// Managed-install agent state (`docs/remote-management.md` §§2, 4, 7).
pub struct RemoteManagedAgent {
    /// This install's node id.
    pub node: NodeId,
    /// The controller's node id; only it may call.
    pub controller_node: NodeId,
    /// Config revision last applied.
    pub applied_revision: u64,
    /// `host.toml` keys this install keeps over the shared config.
    pub local_overrides: Vec<String>,
    /// Set once the install detached or was revoked; it then refuses the controller.
    pub detached: bool,
    /// Syncs in a row that could not reach the controller, for the backoff.
    pub consecutive_unreachable: u32,
}

impl RemoteManagedAgent {
    /// An attached agent at `applied_revision`.
    pub fn new(node: NodeId, controller_node: NodeId, applied_revision: u64, local_overrides: Vec<String>) -> Self {
        Self {
            node,
            controller_node,
            applied_revision,
            local_overrides,
            detached: false,
            consecutive_unreachable: 0,
        }
    }

    /// Apply a `SharedConfigBundle` onto the managed install via `handler`, preserving per-host
    /// identity fields and `local_overrides`.
    pub fn apply_bundle(
        &mut self,
        handler: &dyn RemoteHostHandler,
        bundle: &SharedConfigBundle,
    ) -> Result<u64, RemoteError> {
        bundle.validate()?;
        let local_toml = handler.read_local_host_toml()?;
        let per_node = bundle.node_overrides.get(self.node.as_str()).map(String::as_str);
        let merged = merge_managed_host_toml(&local_toml, &bundle.shared_host_toml, per_node, &self.local_overrides)?;
        handler.write_merged_host_toml(&merged, bundle.revision)?;
        self.applied_revision = bundle.revision;
        Ok(self.applied_revision)
    }

    /// Handle a direct push request (`Execute`, `PushRevision`, `FetchLogs`) from the controller.
    /// Verifies that `authenticated_caller == self.controller_node`.
    pub fn handle_controller_push(
        &mut self,
        authenticated_caller: &NodeId,
        handler: &dyn RemoteHostHandler,
        req: RemoteRequest,
    ) -> Result<RemoteResponse, RemoteError> {
        if self.detached {
            return Err(RemoteError::RevokedPeer(self.node.as_str().to_string()));
        }
        if authenticated_caller != &self.controller_node {
            return Err(RemoteError::PeerIdentityMismatch {
                cert_node: authenticated_caller.as_str().to_string(),
                body_node: self.controller_node.as_str().to_string(),
            });
        }
        req.validate()?;
        match req {
            RemoteRequest::Execute { command } => {
                if matches!(command.command, RemoteCommand::Detached { .. }) {
                    self.detached = true;
                }
                let outcome = handler.execute_command(&command)?;
                outcome.validate()?;
                Ok(RemoteResponse::Executed { outcome })
            }
            RemoteRequest::PushRevision { bundle } => {
                let rev = self.apply_bundle(handler, &bundle)?;
                Ok(RemoteResponse::RevisionApplied { applied_revision: rev })
            }
            RemoteRequest::FetchLogs { since_seq, limit } => {
                let (next_seq, entries) = handler.fetch_logs(since_seq, limit)?;
                let resp = RemoteResponse::Logs { next_seq, entries };
                resp.validate()?;
                Ok(resp)
            }
            RemoteRequest::Sync { .. } => Err(RemoteError::InvalidInput(
                "managed install does not accept Sync requests".to_string(),
            )),
        }
    }

    /// Process a `SyncAck` returned by the controller on a heartbeat pull, applying any newer
    /// `SharedConfigBundle` and executing any `pending_commands`.
    pub fn process_sync_ack(
        &mut self,
        handler: &dyn RemoteHostHandler,
        ack: RemoteResponse,
    ) -> Result<Vec<CommandOutcome>, RemoteError> {
        ack.validate()?;
        let RemoteResponse::SyncAck {
            bundle,
            pending_commands,
            ..
        } = ack
        else {
            return Err(RemoteError::InvalidInput(
                "expected SyncAck response from controller".to_string(),
            ));
        };
        self.consecutive_unreachable = 0;
        if let Some(b) = bundle
            && b.revision > self.applied_revision
        {
            let _ = self.apply_bundle(handler, &b)?;
        }
        let mut outcomes = Vec::with_capacity(pending_commands.len());
        for cmd in pending_commands {
            cmd.validate()?;
            if matches!(cmd.command, RemoteCommand::Detached { .. }) {
                self.detached = true;
            }
            let outcome = handler.execute_command(&cmd)?;
            outcome.validate()?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    /// Record that the controller was unreachable during a heartbeat pull.
    /// Never stops the server or wipes local state; returns the next backoff delay in ms.
    pub fn record_controller_unreachable(&mut self) -> u64 {
        self.consecutive_unreachable = self.consecutive_unreachable.saturating_add(1);
        offline_backoff_delay_ms(self.consecutive_unreachable)
    }
}
