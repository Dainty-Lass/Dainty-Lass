// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Remote management (`docs/remote-management.md`, `docs/security/remote-management.md`, DD27, RM1–RM4):
//! one controller (`Dainty Lass Manager` or `dainty-host`) manages enrolled Dainty Lass server installs
//! over `crates/dainty-network` mTLS node links (`POST /__node/remote`) and one-time enrollment codes
//! (`POST /__node/enroll`).

pub mod enroll;
pub mod merge;
pub mod state;
pub mod transport;
pub mod wire;

pub use enroll::{
    DEFAULT_ENROLL_TTL_SECS, ENROLL_HOST, ENROLL_SCHEME, ENROLL_VERSION, EnrollmentInvite, MAX_ENROLL_TTL_SECS,
    MAX_ENROLL_URI_LEN, MAX_LABEL_LEN, PendingEnrollment, generate_enrollment_token, is_valid_sha256_fp,
    parse_enrollment_uri, sha256_hex, validate_controller_url, validate_enrollment_token, validate_label,
    verify_cert_fingerprint,
};
pub use merge::{PRESERVED_LOCAL_KEYS, PRESERVED_LOCAL_TABLES, is_preserved_local_key, merge_managed_settings_toml};
pub use state::{
    DEFAULT_OFFLINE_THRESHOLD_MS, EnrolledInstallRecord, EnrolledInstallState, InstallSummary,
    PendingGuardedRemoteAction, RemoteControllerState, RemoteHostHandler, RemoteManagedAgent, SubmitCommandResult,
    cert_pem_fingerprint, generate_node_keypair_and_spki_pem, offline_backoff_delay_ms, sign_enrolled_node_cert,
};
pub use transport::{
    ENROLL_PATH, GameFleet, GameFleetSource, RemoteControllerService, RemoteManagedService, enroll_with_controller,
    sync_once_with_controller, sync_once_with_games,
};
pub use wire::{
    AdminModerationKind, CommandOutcome, ComponentHealth, DbSubcommand, DoctorCheckEntry, DoctorSummary, EnrollAnswer,
    EnrollRequest, FORBIDDEN_SECRET_PATTERNS, HistorySummary, InstallReport, LifecycleAction, MAX_LOG_ENTRIES,
    MAX_PEM_LEN, MAX_REMOTE_WIRE, MAX_REPORT_GAME_SERVERS, ModuleControlKind, MongoMigrationState, OperatorEnvelope,
    QueuedRemoteCommand, RemoteCommand, RemoteLogEntry, RemoteRequest, RemoteResponse, RemoteRole, ServerRunState,
    SharedConfigBundle, SharedInvite, contains_forbidden_secret_bytes, contains_forbidden_secret_str,
    contains_forbidden_secret_value, contains_prefixed_key_token, decode_enroll_answer, decode_enroll_request,
    decode_request, decode_response, encode_enroll_answer, encode_enroll_request, encode_request, encode_response,
};

/// Why a remote-management step was refused or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoteError {
    /// A field, URL, label or message failed validation; says which.
    #[error("invalid remote-management input: {0}")]
    InvalidInput(String),
    /// A wire message over [`MAX_REMOTE_WIRE`] bytes.
    #[error("remote wire message is {0} bytes (maximum is {MAX_REMOTE_WIRE})")]
    TooLarge(usize),
    /// A message carried a secret or raw database material; names the pattern.
    #[error("refused secret or raw database material on remote wire: {0}")]
    SecretLeakRefused(String),
    /// The controller's certificate is not the one the invite pins.
    #[error("controller certificate fingerprint mismatch: expected {expected}, got {actual}")]
    CertificateMismatch {
        /// The invite's fingerprint.
        expected: String,
        /// The certificate's fingerprint.
        actual: String,
    },
    /// The enrollment token is malformed or unknown.
    #[error("invalid enrollment token")]
    InvalidEnrollmentToken,
    /// The enrollment token was already redeemed.
    #[error("enrollment token has already been used")]
    EnrollmentTokenUsed,
    /// The enrollment token is past its expiry.
    #[error("enrollment token has expired")]
    EnrollmentTokenExpired,
    /// No enrolled install has this node id.
    #[error("unknown managed install: {0}")]
    UnknownInstall(String),
    /// The install was revoked or detached.
    #[error("managed install {0} has been revoked")]
    RevokedPeer(String),
    /// The node a request names is not the one its mTLS certificate names.
    #[error("mTLS peer certificate node {cert_node} does not match request node {body_node}")]
    PeerIdentityMismatch {
        /// Node id the certificate names.
        cert_node: String,
        /// Node id the request names.
        body_node: String,
    },
    /// The caller may not do this; says why.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// A guarded command needs its delay to pass or a second operator's approval.
    #[error("guarded action approval required: {0}")]
    GuardedApprovalRequired(String),
    /// Key or certificate work failed.
    #[error("cryptographic error: {0}")]
    Crypto(String),
    /// Reading or writing the state file failed.
    #[error("I/O error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use dainty_network::cluster::{CA_DAYS, ClusterCa};

    use super::*;

    fn sample_fp() -> String {
        "a".repeat(64)
    }

    fn sample_report(node: &str, region: &str, rev: u64) -> InstallReport {
        InstallReport {
            node: node.parse().expect("valid node"),
            region: region.parse().expect("valid region"),
            label: format!("Node {node}"),
            version: "2.0.0-alpha.0".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            applied_revision: rev,
            server_state: ServerRunState::Running,
            uptime_secs: 120,
            players_online: 4,
            active_hunts: 1,
            active_hubs: 1,
            active_realms: 0,
            components: vec![ComponentHealth {
                id: "gateway".to_string(),
                state: "ready".to_string(),
                port: Some(61000),
                detail: None,
            }],
            doctor: Some(DoctorSummary {
                ok: true,
                pass_count: 8,
                warn_count: 0,
                fail_count: 0,
                checked_at_ms: 1_700_000_000_000,
                checks: vec![DoctorCheckEntry {
                    id: "tls".to_string(),
                    status: "pass".to_string(),
                    summary: "Gateway certificate valid".to_string(),
                    detail: None,
                }],
            }),
            mongo_migration: MongoMigrationState::NotNeeded,
            history: Some(HistorySummary {
                head_seq: 12,
                head_hash: "b".repeat(64),
                chain_verified: true,
                last_change_at_ms: Some(1_700_000_000_000),
                last_actor: Some("remote:ctrl-1:owner-1".to_string()),
                pending_guarded_count: 0,
                ship_lag_secs: Some(0),
            }),
            game_servers: Vec::new(),
            availability: None,
        }
    }

    #[test]
    fn enrollment_uri_round_trips_and_rejects_bad_schemes_or_tokens() {
        let token = generate_enrollment_token();
        let invite = EnrollmentInvite {
            url: "https://ctrl.example.org:61000".to_string(),
            controller_fp: sample_fp(),
            node: "eu-de-1".parse().expect("node"),
            region: "eu-central".parse().expect("region"),
            token: token.clone(),
            label: Some("Frankfurt 1".to_string()),
        };
        let uri = invite.to_uri().expect("uri");
        let parsed = parse_enrollment_uri(&uri).expect("parse uri");
        assert_eq!(parsed, invite);

        assert!(parse_enrollment_uri("http://enroll?v=1").is_err());
        assert!(parse_enrollment_uri(&format!("{uri}&v=1")).is_err());
        assert!(
            parse_enrollment_uri(
                "dainty-lass-remote://enroll?v=1&url=http%3A%2F%2Fevil.example.org%3A61000&fp=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&node=eu-de-1&region=eu&token=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            )
            .is_err()
        );
    }

    #[test]
    fn certificate_fingerprint_pinning_refuses_mismatch() {
        let cert_bytes = b"simulated-der-certificate-bytes";
        let actual_fp = sha256_hex(cert_bytes);
        verify_cert_fingerprint(cert_bytes, &actual_fp).expect("matches exact fingerprint");

        let wrong_fp = "0".repeat(64);
        let err = verify_cert_fingerprint(cert_bytes, &wrong_fp).expect_err("must refuse mismatch");
        assert!(matches!(err, RemoteError::CertificateMismatch { .. }));
    }

    #[test]
    fn single_use_enrollment_redeems_once_and_refuses_replay_or_expired_token() {
        let ca = ClusterCa::create(CA_DAYS).expect("ca");
        let mut ctrl = RemoteControllerState::new(
            "ctrl-1".parse().expect("node"),
            "us-east".parse().expect("region"),
            "https://ctrl.example.org:61000".to_string(),
            sample_fp(),
            "[server]\nregistration = \"invite\"\n".to_string(),
        )
        .expect("controller state");

        let (_uri, invite) = ctrl
            .mint_enrollment_invite(
                "eu-de-1".parse().expect("node"),
                "eu-central".parse().expect("region"),
                Some("Frankfurt".to_string()),
                1_000_000,
                Some(900),
            )
            .expect("mint");

        let (_priv_pem, pub_pem) = generate_node_keypair_and_spki_pem().expect("keypair");
        let req = EnrollRequest {
            token: invite.token.clone(),
            node: invite.node.clone(),
            region: invite.region.clone(),
            label: Some("Frankfurt".to_string()),
            public_key_pem: pub_pem,
            gateway_url: Some("https://eu-de-1.example.org:61000".to_string()),
            gateway_cert_fp: Some("c".repeat(64)),
        };

        let encoded = encode_enroll_request(&req).expect("encode req");
        let decoded = decode_enroll_request(&encoded).expect("decode req");
        assert_eq!(decoded, req);

        let answer = ctrl
            .redeem_enrollment(&ca, &decoded, 1_005_000)
            .expect("first redeem succeeds");
        let ans_bytes = encode_enroll_answer(&answer).expect("encode answer");
        let ans_decoded = decode_enroll_answer(&ans_bytes).expect("decode answer");
        assert_eq!(ans_decoded, answer);
        assert!(ctrl.is_authorized_peer(&invite.node));

        let replay = ctrl
            .redeem_enrollment(&ca, &decoded, 1_006_000)
            .expect_err("replay must fail");
        assert_eq!(replay, RemoteError::EnrollmentTokenUsed);

        let (_uri2, invite2) = ctrl
            .mint_enrollment_invite(
                "us-west-1".parse().expect("node"),
                "us-west".parse().expect("region"),
                None,
                1_000_000,
                Some(60),
            )
            .expect("mint 2");
        let (_priv2, pub2) = generate_node_keypair_and_spki_pem().expect("keypair 2");
        let req2 = EnrollRequest {
            token: invite2.token,
            node: invite2.node,
            region: invite2.region,
            label: None,
            public_key_pem: pub2,
            gateway_url: None,
            gateway_cert_fp: None,
        };
        let expired = ctrl
            .redeem_enrollment(&ca, &req2, 1_070_000)
            .expect_err("expired token must fail");
        assert_eq!(expired, RemoteError::EnrollmentTokenExpired);
    }

    #[test]
    fn config_merge_applies_shared_and_node_overrides_while_preserving_local_identity_and_overrides() {
        let local_toml = r#"# Local host configuration
[server]
name = "Local Name"
host = "203.0.113.10"
game_address = "203.0.113.10"
mode = "public"
registration = "open"

[ports]
gateway = 61000
backend = 61001

[paths]
gameserver_binary = "/srv/dauntless/ArchonServer.exe"

[database]
engine = "sqlite"
url = "sqlite:///srv/dauntless/.workspace/config/dainty-lass.db"

[network]
node_id = "eu-de-1"
region = "eu-central"

[paradox]
max_hunts = 4
idle_timeout_secs = 120

[remote]
role = "managed"
controller_node = "ctrl-1"
local_overrides = ["paradox.idle_timeout_secs"]
"#;

        let shared_toml = r#"[server]
name = "Fleet Shared Name"
host = "198.51.100.1"
game_address = "198.51.100.1"
mode = "private"
registration = "invite"

[ports]
gateway = 443

[database]
engine = "postgres"
url = "postgres://shared/db"

[paradox]
max_hunts = 8
idle_timeout_secs = 600

[switches]
rate_limit_enabled = true
"#;

        let node_override_toml = r#"[server]
name = "Fleet Frankfurt"

[paradox]
max_hunts = 12
idle_timeout_secs = 900
"#;

        let merged = merge_managed_settings_toml(local_toml, shared_toml, Some(node_override_toml), &[])
            .expect("merge succeeds");

        assert!(merged.contains("name = \"Fleet Frankfurt\""));
        assert!(merged.contains("registration = \"invite\""));
        assert!(merged.contains("max_hunts = 12"));
        assert!(merged.contains("rate_limit_enabled = true"));
        assert!(merged.contains("host = \"203.0.113.10\""));
        assert!(merged.contains("game_address = \"203.0.113.10\""));
        assert!(merged.contains("mode = \"public\""));
        assert!(merged.contains("gateway = 61000"));
        assert!(merged.contains("engine = \"sqlite\""));
        assert!(merged.contains("node_id = \"eu-de-1\""));
        assert!(merged.contains("idle_timeout_secs = 120"));
    }

    #[test]
    fn secret_leak_check_refuses_secrets_env_account_keys_private_keys_and_sqlite_headers() {
        assert!(contains_forbidden_secret_str("DLK_1234567890abcdef"));
        assert!(contains_forbidden_secret_str("dla_abcdef1234567890"));
        assert!(contains_forbidden_secret_str("DAINTY_ADMIN_API_KEY=secret"));
        assert!(contains_forbidden_secret_str("JWT_SECRET=supersecret"));
        assert!(contains_forbidden_secret_str("-----BEGIN PRIVATE KEY-----"));
        assert!(contains_forbidden_secret_bytes(b"SQLite format 3\0raw db bytes"));

        let bad_outcome = CommandOutcome {
            command_id: "cmd-1".to_string(),
            ok: true,
            code: None,
            message: "player key is DLK_abcdef0123456789".to_string(),
            data: None,
        };
        assert!(matches!(bad_outcome.validate(), Err(RemoteError::SecretLeakRefused(_))));
    }

    #[test]
    fn guarded_actions_require_second_operator_with_distinct_key_id_and_delay() {
        let mut ctrl = RemoteControllerState::new(
            "ctrl-1".parse().expect("node"),
            "us-east".parse().expect("region"),
            "https://ctrl.example.org:61000".to_string(),
            sample_fp(),
            "[server]\nregistration = \"invite\"\n".to_string(),
        )
        .expect("controller state");

        let ca = ClusterCa::create(CA_DAYS).expect("ca");
        let (_uri, invite) = ctrl
            .mint_enrollment_invite(
                "eu-de-1".parse().expect("node"),
                "eu-central".parse().expect("region"),
                None,
                1_000,
                None,
            )
            .expect("mint");
        let (_priv_pem, pub_pem) = generate_node_keypair_and_spki_pem().expect("keypair");
        let _ = ctrl
            .redeem_enrollment(
                &ca,
                &EnrollRequest {
                    token: invite.token,
                    node: invite.node.clone(),
                    region: invite.region,
                    label: None,
                    public_key_pem: pub_pem,
                    gateway_url: None,
                    gateway_cert_fp: None,
                },
                2_000,
            )
            .expect("redeem");

        let owner_1 = OperatorEnvelope {
            controller_node: "ctrl-1".parse().expect("node"),
            key_id: "owner-alice".to_string(),
            role: RemoteRole::Owner,
            issued_at_ms: 3_000,
        };
        let revert_cmd = RemoteCommand::HistoryRevert {
            target_seq: 5,
            dry_run: false,
            reason: Some("rollback accidental wipe".to_string()),
        };

        let res = ctrl
            .submit_command(vec![invite.node.clone()], owner_1.clone(), revert_cmd, 3_000, 30, true)
            .expect("submit guarded");
        let SubmitCommandResult::PendingGuarded { guarded } = res else {
            panic!("expected PendingGuarded");
        };
        assert!(ctrl.installs["eu-de-1"].queued_commands.is_empty());

        let same_op_err = ctrl
            .approve_guarded(&guarded.id, owner_1, 40_000)
            .expect_err("same operator cannot self-approve");
        assert!(matches!(same_op_err, RemoteError::GuardedApprovalRequired(_)));

        let admin_bob = OperatorEnvelope {
            controller_node: "ctrl-1".parse().expect("node"),
            key_id: "admin-bob".to_string(),
            role: RemoteRole::Admin,
            issued_at_ms: 10_000,
        };
        let early_err = ctrl
            .approve_guarded(&guarded.id, admin_bob.clone(), 10_000)
            .expect_err("cannot approve before cooling-off delay");
        assert!(matches!(early_err, RemoteError::GuardedApprovalRequired(_)));

        let approved = ctrl
            .approve_guarded(&guarded.id, admin_bob, 35_000)
            .expect("second operator approves after delay");
        assert!(approved.executed);
        assert_eq!(ctrl.installs["eu-de-1"].queued_commands.len(), 1);
        assert_eq!(
            ctrl.installs["eu-de-1"].queued_commands[0].operator.actor_string(),
            "remote:ctrl-1:owner-alice"
        );
    }

    struct MockHostHandler {
        local_toml: Mutex<String>,
        executed: Mutex<Vec<String>>,
    }

    impl RemoteHostHandler for MockHostHandler {
        fn build_report(&self, applied_revision: u64) -> Result<InstallReport, RemoteError> {
            Ok(sample_report("eu-de-1", "eu-central", applied_revision))
        }

        fn read_local_settings_toml(&self) -> Result<String, RemoteError> {
            Ok(self
                .local_toml
                .lock()
                .map_err(|_| RemoteError::Io("lock".to_string()))?
                .clone())
        }

        fn write_merged_settings_toml(&self, merged_toml: &str, _revision: u64) -> Result<(), RemoteError> {
            *self
                .local_toml
                .lock()
                .map_err(|_| RemoteError::Io("lock".to_string()))? = merged_toml.to_string();
            Ok(())
        }

        fn execute_command(&self, cmd: &QueuedRemoteCommand) -> Result<CommandOutcome, RemoteError> {
            self.executed
                .lock()
                .map_err(|_| RemoteError::Io("lock".to_string()))?
                .push(cmd.command_id.clone());
            Ok(CommandOutcome {
                command_id: cmd.command_id.clone(),
                ok: true,
                code: None,
                message: format!("executed by {}", cmd.operator.actor_string()),
                data: None,
            })
        }

        fn fetch_logs(&self, since_seq: u64, _limit: u32) -> Result<(u64, Vec<RemoteLogEntry>), RemoteError> {
            Ok((
                since_seq + 1,
                vec![RemoteLogEntry {
                    seq: since_seq + 1,
                    timestamp_ms: 1_700_000_000_000,
                    level: "INFO".to_string(),
                    target: "gateway".to_string(),
                    message: "listening on 61000".to_string(),
                }],
            ))
        }
    }

    #[test]
    fn revoked_install_is_refused_and_unreachable_controller_keeps_running_with_backoff() {
        let ca = ClusterCa::create(CA_DAYS).expect("ca");
        let mut ctrl = RemoteControllerState::new(
            "ctrl-1".parse().expect("node"),
            "us-east".parse().expect("region"),
            "https://ctrl.example.org:61000".to_string(),
            sample_fp(),
            "[server]\nregistration = \"invite\"\n".to_string(),
        )
        .expect("controller state");

        let (_uri, invite) = ctrl
            .mint_enrollment_invite(
                "eu-de-1".parse().expect("node"),
                "eu-central".parse().expect("region"),
                None,
                1_000,
                None,
            )
            .expect("mint");
        let (_priv_pem, pub_pem) = generate_node_keypair_and_spki_pem().expect("keypair");
        let _ = ctrl
            .redeem_enrollment(
                &ca,
                &EnrollRequest {
                    token: invite.token,
                    node: invite.node.clone(),
                    region: invite.region,
                    label: None,
                    public_key_pem: pub_pem,
                    gateway_url: None,
                    gateway_cert_fp: None,
                },
                2_000,
            )
            .expect("redeem");

        let handler = MockHostHandler {
            local_toml: Mutex::new("[server]\nhost = \"203.0.113.10\"\nregistration = \"open\"\n".to_string()),
            executed: Mutex::new(Vec::new()),
        };
        let mut agent = RemoteManagedAgent::new(invite.node.clone(), "ctrl-1".parse().expect("node"), 0, Vec::new());

        let sync_req = RemoteRequest::Sync {
            node: invite.node.clone(),
            applied_revision: agent.applied_revision,
            report: Box::new(handler.build_report(agent.applied_revision).expect("report")),
            completed_commands: Vec::new(),
        };
        let req_bytes = encode_request(&sync_req).expect("encode sync");
        let req_decoded = decode_request(&req_bytes).expect("decode sync");

        let ack = ctrl
            .handle_remote_request(&invite.node, req_decoded, 3_000)
            .expect("sync ack");
        let ack_bytes = encode_response(&ack).expect("encode ack");
        let ack_decoded = decode_response(&ack_bytes).expect("decode ack");

        let outcomes = agent.process_sync_ack(&handler, ack_decoded).expect("process ack");
        assert!(outcomes.is_empty());
        assert_eq!(agent.applied_revision, 1);
        assert!(
            handler
                .read_local_settings_toml()
                .expect("toml")
                .contains("registration = \"invite\"")
        );

        assert_eq!(agent.record_controller_unreachable(), 5_000);
        assert_eq!(agent.record_controller_unreachable(), 10_000);
        assert_eq!(agent.record_controller_unreachable(), 20_000);
        assert_eq!(agent.record_controller_unreachable(), 40_000);
        assert_eq!(agent.record_controller_unreachable(), 60_000);
        assert_eq!(agent.applied_revision, 1);
        assert!(!agent.detached);

        ctrl.revoke_install(&invite.node).expect("revoke");
        assert!(!ctrl.is_authorized_peer(&invite.node));
        let refused = ctrl
            .handle_remote_request(&invite.node, sync_req, 4_000)
            .expect_err("revoked peer must be refused");
        assert_eq!(refused, RemoteError::RevokedPeer("eu-de-1".to_string()));
    }

    fn enrolled_controller() -> (RemoteControllerState, dainty_module_api::network::NodeId) {
        let ca = ClusterCa::create(CA_DAYS).expect("ca");
        let mut ctrl = RemoteControllerState::new(
            "ctrl-1".parse().expect("node"),
            "us-east".parse().expect("region"),
            "https://ctrl.example.org:61000".to_string(),
            sample_fp(),
            "[server]\nregistration = \"invite\"\n".to_string(),
        )
        .expect("controller state");
        let (_uri, invite) = ctrl
            .mint_enrollment_invite(
                "eu-de-1".parse().expect("node"),
                "eu-central".parse().expect("region"),
                None,
                1_000,
                None,
            )
            .expect("mint");
        let (_priv_pem, pub_pem) = generate_node_keypair_and_spki_pem().expect("keypair");
        let _ = ctrl
            .redeem_enrollment(
                &ca,
                &EnrollRequest {
                    token: invite.token,
                    node: invite.node.clone(),
                    region: invite.region,
                    label: None,
                    public_key_pem: pub_pem,
                    gateway_url: None,
                    gateway_cert_fp: None,
                },
                2_000,
            )
            .expect("redeem");
        (ctrl, invite.node)
    }

    fn hub() -> dainty_module_api::GameServer {
        dainty_module_api::GameServer {
            id: "city-1".to_string(),
            kind: dainty_module_api::GameServerKind::Hub,
            title: "Ramsgate".to_string(),
            map: "Ramsgate".to_string(),
            host: "203.0.113.10".to_string(),
            port: 7777,
            players: 3,
            max_players: 100,
            ready: true,
            started_at: None,
        }
    }

    fn up() -> dainty_module_api::Availability {
        dainty_module_api::Availability {
            up: true,
            reason: None,
            hub: Some(hub()),
            free_hunts: Some(4),
        }
    }

    fn sync_bytes(report: InstallReport) -> Vec<u8> {
        encode_request(&RemoteRequest::Sync {
            node: report.node.clone(),
            applied_revision: 1,
            report: Box::new(report),
            completed_commands: Vec::new(),
        })
        .expect("encode sync")
    }

    #[test]
    fn a_report_from_a_node_older_than_ms6_still_parses() {
        let old = serde_json::to_value(sample_report("eu-de-1", "eu-central", 1)).expect("json");
        let fields = old.as_object().expect("object");
        assert!(!fields.contains_key("game_servers") && !fields.contains_key("availability"));
        let request = serde_json::json!({
            "op": "sync", "node": "eu-de-1", "applied_revision": 1, "report": old, "completed_commands": []
        });
        let RemoteRequest::Sync { report, .. } =
            decode_request(&serde_json::to_vec(&request).expect("bytes")).expect("an old report parses")
        else {
            panic!("expected Sync");
        };
        assert!(report.game_servers.is_empty());
        assert_eq!(report.availability, None);
    }

    #[test]
    fn a_report_without_game_servers_is_what_an_older_controller_reads() {
        // Empty fields are left out, so a node upgraded first still reaches a controller that refuses unknown ones
        let text = String::from_utf8(sync_bytes(sample_report("eu-de-1", "eu-central", 1))).expect("utf-8");
        assert!(!text.contains("game_servers") && !text.contains("availability"));

        let mut report = sample_report("eu-de-1", "eu-central", 1);
        report.game_servers = vec![hub()];
        report.availability = Some(up());
        let bytes = sync_bytes(report.clone());
        let RemoteRequest::Sync { report: back, .. } = decode_request(&bytes).expect("decodes") else {
            panic!("expected Sync");
        };
        assert_eq!(*back, report);

        report.game_servers = vec![hub(); MAX_REPORT_GAME_SERVERS + 1];
        assert!(report.validate().is_err());
    }

    #[test]
    fn the_controller_keeps_each_nodes_last_report_for_the_fleet_view() {
        let (mut ctrl, node) = enrolled_controller();
        let before = ctrl.install_summaries(3_000, DEFAULT_OFFLINE_THRESHOLD_MS);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].last_report_ms, None);
        assert!(before[0].game_servers.is_empty());

        let mut report = sample_report("eu-de-1", "eu-central", 1);
        report.game_servers = vec![hub()];
        report.availability = Some(up());
        let request = decode_request(&sync_bytes(report)).expect("decode");
        ctrl.handle_remote_request(&node, request, 3_000).expect("sync");

        let after = ctrl.install_summaries(4_000, DEFAULT_OFFLINE_THRESHOLD_MS);
        assert_eq!(after[0].state, EnrolledInstallState::Online);
        assert_eq!(after[0].last_report_ms, Some(3_000));
        assert_eq!(after[0].version.as_deref(), Some("2.0.0-alpha.0"));
        assert_eq!(after[0].game_servers, vec![hub()]);
        assert_eq!(after[0].availability, Some(up()));
        let later = ctrl.install_summaries(3_000 + DEFAULT_OFFLINE_THRESHOLD_MS + 1, DEFAULT_OFFLINE_THRESHOLD_MS);
        assert_eq!(later[0].state, EnrolledInstallState::Offline);
        assert_eq!(
            later[0].game_servers,
            vec![hub()],
            "the last report stays while the node is offline"
        );
    }

    #[test]
    fn moderation_for_one_node_is_queued_for_its_next_sync() {
        let (mut ctrl, node) = enrolled_controller();
        let operator = |role| OperatorEnvelope {
            controller_node: "ctrl-1".parse().expect("node"),
            key_id: "mod-ann".to_string(),
            role,
            issued_at_ms: 10_000,
        };
        let ban = RemoteCommand::AdminAction {
            action: AdminModerationKind::Ban,
            target: "Slayer".to_string(),
            reason: Some("cheating".to_string()),
            duration_secs: None,
        };
        let refused = ctrl
            .queue_admin_action(&node, operator(RemoteRole::Viewer), ban.clone(), 10_000)
            .expect_err("a viewer does not ban");
        assert!(matches!(refused, RemoteError::PermissionDenied(_)));
        assert!(
            ctrl.queue_admin_action(&node, operator(RemoteRole::Admin), RemoteCommand::Backup, 10_000)
                .is_err()
        );
        let unknown = "nowhere-1".parse().expect("node");
        assert!(matches!(
            ctrl.queue_admin_action(&unknown, operator(RemoteRole::Moderator), ban.clone(), 10_000),
            Err(RemoteError::UnknownInstall(_))
        ));

        let id = ctrl
            .queue_admin_action(&node, operator(RemoteRole::Moderator), ban.clone(), 10_000)
            .expect("queued");
        let queued = &ctrl.installs["eu-de-1"].queued_commands;
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].command_id, id);
        assert_eq!(queued[0].command, ban);
    }

    struct ControllerLink(Mutex<RemoteControllerState>);

    impl dainty_network::link::PeerLink for ControllerLink {
        fn call<'a>(
            &'a self,
            _to: &'a dainty_module_api::network::NodeId,
            _service: dainty_network::link::Service,
            body: Vec<u8>,
        ) -> dainty_module_api::BoxFuture<'a, Result<Vec<u8>, dainty_network::link::LinkError>> {
            let answer = decode_request(&body)
                .and_then(|req| {
                    let mut ctrl = self.0.lock().expect("lock");
                    ctrl.handle_remote_request(&"eu-de-1".parse().expect("node"), req, 5_000)
                })
                .and_then(|resp| encode_response(&resp))
                .expect("controller answers");
            Box::pin(async move { Ok(answer) })
        }
    }

    struct Games;

    impl GameFleetSource for Games {
        fn game_fleet(&self) -> dainty_module_api::BoxFuture<'_, GameFleet> {
            Box::pin(async {
                GameFleet {
                    game_servers: vec![hub()],
                    availability: Some(up()),
                }
            })
        }
    }

    #[test]
    fn a_sync_with_a_game_source_carries_its_game_servers() {
        let (ctrl, node) = enrolled_controller();
        let link = ControllerLink(Mutex::new(ctrl));
        let agent = Mutex::new(RemoteManagedAgent::new(
            node,
            "ctrl-1".parse().expect("node"),
            0,
            Vec::new(),
        ));
        let handler = MockHostHandler {
            local_toml: Mutex::new("[server]\nhost = \"203.0.113.10\"\n".to_string()),
            executed: Mutex::new(Vec::new()),
        };
        let outcomes = Mutex::new(Vec::new());
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        runtime
            .block_on(sync_once_with_games(&link, &agent, &handler, &outcomes, Some(&Games)))
            .expect("sync");
        let ctrl = link.0.lock().expect("lock");
        let report = ctrl.installs["eu-de-1"].last_report.as_ref().expect("report kept");
        assert_eq!(report.game_servers, vec![hub()]);
        assert_eq!(report.availability, Some(up()));
    }
}
