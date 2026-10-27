//! `security/runtimeAuthorization.ts`: the runtime channel an account may play on, and the signed release manifest
//! (`<update root>/runtime/client/<channel>/windows-x86_64/latest.json`) the launcher's reported files must match.
//! The module's `TARGET_CHANGELIST` is a parameter: the manifest must target the build this module serves. The
//! launcher update feeds (the Reforged API's) publish the same manifests.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::errors::LauncherApiError;
use crate::repo::auth::RuntimeChannel;
use crate::settings::Settings;

const RUNTIME_TARGET: &str = "client";
const RUNTIME_PLATFORM: &str = "windows-x86_64";

/// Every launcher installs and reports these: the runtime and the winmm proxy that loads it.
const CORE_INSTALLED_ARTIFACTS: [&str; 2] = ["MysticParadox.dll", "winmm.dll"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeArtifactHash {
    pub name: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedRuntimeAttestation {
    pub version: String,
    pub runtime_sha256: String,
    pub artifact_set_sha256: String,
    pub artifacts: Vec<RuntimeArtifactHash>,
}

fn validation(message: &str) -> LauncherApiError {
    LauncherApiError::new("AUTH_VALIDATION_FAILED", message)
}

fn unsupported(message: impl Into<String>) -> LauncherApiError {
    LauncherApiError::new("RUNTIME_BUILD_UNSUPPORTED", message)
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_safe_artifact_name(name: &str) -> bool {
    (1..=96).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/`
fn is_safe_version(text: &str) -> bool {
    let core_end = text.find(['-', '+']).unwrap_or(text.len());
    let (core, rest) = text.split_at(core_end);
    let parts: Vec<&str> = core.split('.').collect();
    let core_ok = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let rest_ok = rest.is_empty()
        || (rest.len() > 1
            && rest[1..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'));
    core_ok && rest_ok
}

/// `ParseRuntimeChannel`.
pub fn parse_runtime_channel(value: Option<&Value>) -> Result<RuntimeChannel, LauncherApiError> {
    value
        .and_then(Value::as_str)
        .and_then(RuntimeChannel::parse)
        .ok_or_else(|| validation("runtimeChannel must be stable, beta, or dev."))
}

/// `NormalizeRuntimeSha256`.
pub fn normalize_runtime_sha256(value: Option<&Value>) -> Result<String, LauncherApiError> {
    let normalized = value
        .and_then(Value::as_str)
        .map(|text| service_kit::js::trim(text).to_lowercase())
        .unwrap_or_default();
    if is_sha256_hex(&normalized) {
        Ok(normalized)
    } else {
        Err(validation("runtimeSha256 must be a SHA-256 hash."))
    }
}

fn normalize_published_sha256(value: Option<&Value>) -> Result<String, LauncherApiError> {
    normalize_runtime_sha256(value)
        .map_err(|_| unsupported("The published runtime manifest has an invalid SHA-256 hash."))
}

/// The channel the current launcher policy returns for an account: stable for every account (the tester role no
/// longer moves a launcher to another channel, `security/testerFeatures.ts`).
pub fn expected_runtime_channel(_roles: &[String]) -> RuntimeChannel {
    RuntimeChannel::Stable
}

/// `AssertRuntimeChannelMatchesAccount`: the server-derived channel, not the one a launcher declares.
pub fn assert_runtime_channel_matches_account(
    requested: RuntimeChannel,
    roles: &[String],
) -> Result<(), LauncherApiError> {
    let expected = expected_runtime_channel(roles);
    if requested == expected {
        return Ok(());
    }
    Err(LauncherApiError::new(
        "AUTH_CHANNEL_MISMATCH",
        format!(
            "Your account currently requires the {} runtime channel.",
            expected.as_str()
        ),
    ))
}

/// `RuntimeUpdateRoot`: `LAUNCHER_UPDATE_ROOT`, else `updates` under the working directory.
pub fn runtime_update_root(settings: &Settings) -> PathBuf {
    let root = settings
        .raw("LAUNCHER_UPDATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("updates"));
    std::path::absolute(&root).unwrap_or(root)
}

pub fn runtime_manifest_path(channel: RuntimeChannel, update_root: &Path) -> PathBuf {
    update_root
        .join("runtime")
        .join(RUNTIME_TARGET)
        .join(channel.as_str())
        .join(RUNTIME_PLATFORM)
        .join("latest.json")
}

fn read_manifest(channel: RuntimeChannel, update_root: &Path) -> Result<Value, LauncherApiError> {
    std::fs::read_to_string(runtime_manifest_path(channel, update_root))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| {
            unsupported(format!(
                "No approved {} runtime is currently published.",
                channel.as_str()
            ))
        })
}

fn targets(manifest: &Value, target_changelist: u32) -> bool {
    service_kit::js::number(manifest.get("targetChangelist")) == f64::from(target_changelist)
}

/// `GetApprovedRuntimeSha256`.
pub fn approved_runtime_sha256(
    channel: RuntimeChannel,
    update_root: &Path,
    target_changelist: u32,
) -> Result<String, LauncherApiError> {
    let manifest = read_manifest(channel, update_root)?;
    if !targets(&manifest, target_changelist) {
        return Err(unsupported(
            "The published runtime does not match the supported game build.",
        ));
    }
    let hash = manifest
        .get("sha256")
        .and_then(Value::as_str)
        .map(|text| service_kit::js::trim(text).to_lowercase())
        .unwrap_or_default();
    if !is_sha256_hex(&hash) {
        return Err(unsupported(
            "The published runtime manifest has an invalid SHA-256 hash.",
        ));
    }
    Ok(hash)
}

fn has_all(seen: &BTreeSet<String>, names: &[&str]) -> bool {
    names.iter().all(|name| seen.contains(&name.to_lowercase()))
}

fn sorted(mut artifacts: Vec<RuntimeArtifactHash>) -> Vec<RuntimeArtifactHash> {
    artifacts.sort_by(|a, b| service_kit::collate::locale_compare(&a.name, &b.name));
    artifacts
}

fn artifact_set_digest(artifacts: &[RuntimeArtifactHash]) -> String {
    let text = artifacts
        .iter()
        .map(|entry| format!("{}:{}", entry.name.to_lowercase(), entry.sha256))
        .collect::<Vec<_>>()
        .join("\n");
    super::crypto::hex(&Sha256::digest(text.as_bytes()))
}

/// `GetApprovedRuntimeAttestation`: the published manifest as the set of files a launcher must report.
pub fn approved_runtime_attestation(
    channel: RuntimeChannel,
    update_root: &Path,
    target_changelist: u32,
) -> Result<ApprovedRuntimeAttestation, LauncherApiError> {
    let manifest = read_manifest(channel, update_root)?;
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .map(|text| service_kit::js::trim(text).to_owned())
        .unwrap_or_default();
    if !targets(&manifest, target_changelist) || !is_safe_version(&version) {
        return Err(unsupported(
            "The published runtime manifest is invalid or targets another game build.",
        ));
    }
    let main_hash = normalize_published_sha256(manifest.get("sha256"))?;
    let Some(extra_files) = manifest.get("extraFiles").and_then(Value::as_array) else {
        return Err(unsupported("The published runtime manifest is incomplete."));
    };
    let mut artifacts = vec![RuntimeArtifactHash {
        name: "MysticParadox.dll".into(),
        sha256: main_hash.clone(),
    }];
    let mut seen = BTreeSet::from(["mysticparadox.dll".to_owned()]);
    for raw in extra_files {
        let name = raw.get("name").and_then(Value::as_str).unwrap_or_default();
        if !is_safe_artifact_name(name) || seen.contains(&name.to_lowercase()) {
            return Err(unsupported(
                "The published runtime manifest has a duplicate or unsafe artifact.",
            ));
        }
        seen.insert(name.to_lowercase());
        artifacts.push(RuntimeArtifactHash {
            name: name.to_owned(),
            sha256: normalize_published_sha256(raw.get("sha256"))?,
        });
    }
    let artifacts = sorted(artifacts);
    if !has_all(&seen, &CORE_INSTALLED_ARTIFACTS) {
        return Err(unsupported(
            "The published runtime manifest does not contain the complete required artifact set.",
        ));
    }
    Ok(ApprovedRuntimeAttestation {
        version,
        runtime_sha256: main_hash,
        artifact_set_sha256: artifact_set_digest(&artifacts),
        artifacts,
    })
}

/// A launcher reports either the core set or the core plus all of the optional files; a name the published manifest
/// does not list makes the set malformed.
fn normalize_artifact_set(
    value: Option<&Value>,
    optional: &[String],
) -> Result<Vec<RuntimeArtifactHash>, LauncherApiError> {
    let known: BTreeSet<String> = CORE_INSTALLED_ARTIFACTS
        .iter()
        .map(|name| (*name).to_owned())
        .chain(optional.iter().cloned())
        .map(|name| name.to_lowercase())
        .collect();
    let Some(items) = value
        .and_then(Value::as_array)
        .filter(|items| items.len() <= known.len())
    else {
        return Err(validation("The complete runtime artifact set is required."));
    };
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for raw in items {
        let name = raw.get("name").and_then(Value::as_str).unwrap_or_default();
        if !is_safe_artifact_name(name) || seen.contains(&name.to_lowercase()) || !known.contains(&name.to_lowercase())
        {
            return Err(validation("The runtime artifact set is malformed."));
        }
        seen.insert(name.to_lowercase());
        normalized.push(RuntimeArtifactHash {
            name: name.to_owned(),
            sha256: normalize_runtime_sha256(raw.get("sha256"))?,
        });
    }
    let optional_refs: Vec<&str> = optional.iter().map(String::as_str).collect();
    let reports_optional = optional.iter().any(|name| seen.contains(&name.to_lowercase()));
    if !has_all(&seen, &CORE_INSTALLED_ARTIFACTS) || (reports_optional && !has_all(&seen, &optional_refs)) {
        return Err(validation("The complete runtime artifact set is required."));
    }
    Ok(sorted(normalized))
}

/// `AssertRuntimeArtifactSetApproved`: every reported file must match the manifest's hash for that name. The digest
/// covers the set actually verified.
pub fn assert_runtime_artifact_set_approved(
    channel: RuntimeChannel,
    requested_version: Option<&Value>,
    requested_artifacts: Option<&Value>,
    update_root: &Path,
    target_changelist: u32,
) -> Result<ApprovedRuntimeAttestation, LauncherApiError> {
    let version = requested_version
        .and_then(Value::as_str)
        .map(|text| service_kit::js::trim(text).to_owned())
        .unwrap_or_default();
    if !is_safe_version(&version) {
        return Err(validation("runtimeManifestVersion is invalid."));
    }
    let approved = approved_runtime_attestation(channel, update_root, target_changelist)?;
    let optional: Vec<String> = approved
        .artifacts
        .iter()
        .map(|entry| entry.name.clone())
        .filter(|name| {
            !CORE_INSTALLED_ARTIFACTS
                .iter()
                .any(|core| core.eq_ignore_ascii_case(name))
        })
        .collect();
    let actual = normalize_artifact_set(requested_artifacts, &optional)?;
    let mismatch = version != approved.version
        || actual.iter().any(|entry| {
            approved
                .artifacts
                .iter()
                .find(|known| known.name.eq_ignore_ascii_case(&entry.name))
                .is_none_or(|known| known.sha256 != entry.sha256)
        });
    if mismatch {
        return Err(unsupported(format!(
            "The installed {} runtime set is not the approved release.",
            channel.as_str()
        )));
    }
    Ok(ApprovedRuntimeAttestation {
        artifact_set_sha256: artifact_set_digest(&actual),
        artifacts: actual,
        ..approved
    })
}

/// `AssertRuntimeHashApproved`.
pub fn assert_runtime_hash_approved(
    channel: RuntimeChannel,
    runtime_sha256: Option<&Value>,
    update_root: &Path,
    target_changelist: u32,
) -> Result<String, LauncherApiError> {
    let actual = normalize_runtime_sha256(runtime_sha256)?;
    let approved = approved_runtime_sha256(channel, update_root, target_changelist)?;
    if actual != approved {
        return Err(unsupported(format!(
            "The installed {} runtime is not the approved release.",
            channel.as_str()
        )));
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const CHANGELIST: u32 = 392_819;

    fn hash(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn artifacts(entries: &[(&str, char)]) -> Value {
        Value::Array(
            entries
                .iter()
                .map(|(name, c)| json!({"name": name, "sha256": hash(*c)}))
                .collect(),
        )
    }

    fn core() -> Value {
        artifacts(&[("MysticParadox.dll", 'a'), ("winmm.dll", 'b')])
    }

    fn manifest(channel: RuntimeChannel, extra: Value, changelist: u32) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let path = runtime_manifest_path(channel, root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body =
            json!({"version": "0.1.78", "targetChangelist": changelist, "sha256": hash('a'), "extraFiles": extra});
        std::fs::write(path, body.to_string()).unwrap();
        root
    }

    fn code<T: std::fmt::Debug>(result: Result<T, LauncherApiError>) -> String {
        result.unwrap_err().code
    }

    #[test]
    fn the_channel_parser_accepts_only_the_three_named_channels() {
        assert_eq!(
            parse_runtime_channel(Some(&json!("stable"))).unwrap(),
            RuntimeChannel::Stable
        );
        assert_eq!(
            parse_runtime_channel(Some(&json!("beta"))).unwrap(),
            RuntimeChannel::Beta
        );
        assert_eq!(parse_runtime_channel(Some(&json!("dev"))).unwrap(), RuntimeChannel::Dev);
        assert_eq!(
            code(parse_runtime_channel(Some(&json!("preview")))),
            "AUTH_VALIDATION_FAILED"
        );
        assert_eq!(code(parse_runtime_channel(None)), "AUTH_VALIDATION_FAILED");
    }

    #[test]
    fn every_account_is_on_stable_and_beta_and_dev_are_denied() {
        for roles in [vec!["player"], vec!["player", "tester"], vec!["player", "admin"]] {
            let roles: Vec<String> = roles.into_iter().map(str::to_owned).collect();
            assert_eq!(expected_runtime_channel(&roles), RuntimeChannel::Stable);
            assert!(assert_runtime_channel_matches_account(RuntimeChannel::Stable, &roles).is_ok());
            assert_eq!(
                code(assert_runtime_channel_matches_account(RuntimeChannel::Beta, &roles)),
                "AUTH_CHANNEL_MISMATCH"
            );
            assert_eq!(
                code(assert_runtime_channel_matches_account(RuntimeChannel::Dev, &roles)),
                "AUTH_CHANNEL_MISMATCH"
            );
        }
    }

    #[test]
    fn an_approved_runtime_hash_is_accepted_from_the_channel_manifest() {
        let root = manifest(RuntimeChannel::Beta, artifacts(&[("winmm.dll", 'b')]), CHANGELIST);
        let upper = json!(hash('a').to_uppercase());
        assert_eq!(
            assert_runtime_hash_approved(RuntimeChannel::Beta, Some(&upper), root.path(), CHANGELIST).unwrap(),
            hash('a')
        );
        let other = json!(hash('b'));
        assert_eq!(
            code(assert_runtime_hash_approved(
                RuntimeChannel::Beta,
                Some(&other),
                root.path(),
                CHANGELIST
            )),
            "RUNTIME_BUILD_UNSUPPORTED"
        );
    }

    #[test]
    fn a_missing_or_wrong_build_manifest_fails_closed() {
        let empty = tempfile::tempdir().unwrap();
        let a = json!(hash('a'));
        assert_eq!(
            code(assert_runtime_hash_approved(
                RuntimeChannel::Stable,
                Some(&a),
                empty.path(),
                CHANGELIST
            )),
            "RUNTIME_BUILD_UNSUPPORTED"
        );
        let wrong = manifest(RuntimeChannel::Stable, artifacts(&[("winmm.dll", 'b')]), 1);
        assert_eq!(
            code(assert_runtime_hash_approved(
                RuntimeChannel::Stable,
                Some(&a),
                wrong.path(),
                CHANGELIST
            )),
            "RUNTIME_BUILD_UNSUPPORTED"
        );
    }

    #[test]
    fn the_complete_signed_manifest_set_is_accepted() {
        let root = manifest(RuntimeChannel::Stable, artifacts(&[("winmm.dll", 'b')]), CHANGELIST);
        let upper = Value::Array(
            core()
                .as_array()
                .unwrap()
                .iter()
                .map(|e| json!({"name": e["name"], "sha256": e["sha256"].as_str().unwrap().to_uppercase()}))
                .collect(),
        );
        let approved = assert_runtime_artifact_set_approved(
            RuntimeChannel::Stable,
            Some(&json!("0.1.78")),
            Some(&upper),
            root.path(),
            CHANGELIST,
        )
        .unwrap();
        assert_eq!(approved.version, "0.1.78");
        assert_eq!(approved.runtime_sha256, hash('a'));
        assert_eq!(approved.artifact_set_sha256.len(), 64);
        let mut names: Vec<_> = approved.artifacts.iter().map(|e| e.name.clone()).collect();
        names.sort();
        assert_eq!(names, ["MysticParadox.dll", "winmm.dll"]);
    }

    fn check(root: &Path, version: &str, reported: &Value) -> Result<ApprovedRuntimeAttestation, LauncherApiError> {
        assert_runtime_artifact_set_approved(
            RuntimeChannel::Stable,
            Some(&json!(version)),
            Some(reported),
            root,
            CHANGELIST,
        )
    }

    #[test]
    fn every_mutation_omission_or_unknown_file_is_rejected() {
        let root = manifest(RuntimeChannel::Stable, artifacts(&[("winmm.dll", 'b')]), CHANGELIST);
        let bad_proxy = artifacts(&[("MysticParadox.dll", 'a'), ("winmm.dll", 'f')]);
        assert_eq!(
            code(check(root.path(), "0.1.78", &bad_proxy)),
            "RUNTIME_BUILD_UNSUPPORTED"
        );
        let omitted = artifacts(&[("MysticParadox.dll", 'a')]);
        assert_eq!(code(check(root.path(), "0.1.78", &omitted)), "AUTH_VALIDATION_FAILED");
        let extra = artifacts(&[("MysticParadox.dll", 'a'), ("winmm.dll", 'b'), ("inject.dll", 'c')]);
        assert_eq!(code(check(root.path(), "0.1.78", &extra)), "AUTH_VALIDATION_FAILED");
        assert_eq!(code(check(root.path(), "0.1.77", &core())), "RUNTIME_BUILD_UNSUPPORTED");
        assert_eq!(
            code(check(root.path(), "not a version", &core())),
            "AUTH_VALIDATION_FAILED"
        );
    }

    #[test]
    fn an_incomplete_published_manifest_fails_closed() {
        let root = manifest(RuntimeChannel::Stable, json!([]), CHANGELIST);
        assert_eq!(code(check(root.path(), "0.1.78", &core())), "RUNTIME_BUILD_UNSUPPORTED");
    }

    const OPTIONAL: [(&str, char); 5] = [
        ("MysticParadox.dll", 'a'),
        ("winmm.dll", 'b'),
        ("coop-transport.dll", 'c'),
        ("coop-runtime.dll", 'd'),
        ("coop-admission.key", 'e'),
    ];

    #[test]
    fn the_optional_set_is_all_or_none_and_has_its_own_digest() {
        let root = manifest(RuntimeChannel::Stable, artifacts(&OPTIONAL[1..]), CHANGELIST);
        let full = check(root.path(), "0.1.78", &artifacts(&OPTIONAL)).unwrap();
        assert_eq!(full.runtime_sha256, hash('a'));
        let core_only = check(root.path(), "0.1.78", &artifacts(&OPTIONAL[..2])).unwrap();
        assert_ne!(core_only.artifact_set_sha256, full.artifact_set_sha256);
        for index in 1..OPTIONAL.len() {
            let mut mutated = OPTIONAL.to_vec();
            mutated[index].1 = 'f';
            assert_eq!(
                code(check(root.path(), "0.1.78", &artifacts(&mutated))),
                "RUNTIME_BUILD_UNSUPPORTED",
                "{index}"
            );
        }
        assert_eq!(
            code(check(root.path(), "0.1.78", &artifacts(&OPTIONAL[..4]))),
            "AUTH_VALIDATION_FAILED"
        );
    }

    #[test]
    fn a_core_only_manifest_refuses_unpublished_files() {
        let root = manifest(RuntimeChannel::Stable, artifacts(&OPTIONAL[1..2]), CHANGELIST);
        assert!(check(root.path(), "0.1.78", &artifacts(&OPTIONAL[..2])).is_ok());
        assert_eq!(
            code(check(root.path(), "0.1.78", &artifacts(&OPTIONAL))),
            "AUTH_VALIDATION_FAILED"
        );
    }

    #[test]
    fn files_the_manifest_lists_beyond_the_core_are_not_required_of_a_launcher() {
        let root = manifest(
            RuntimeChannel::Stable,
            json!([{"name": "winmm.dll", "sha256": hash('b')}, {"name": "optional-module.bin", "sha256": hash('c')}]),
            CHANGELIST,
        );
        assert!(check(root.path(), "0.1.78", &core()).is_ok());
    }
}
