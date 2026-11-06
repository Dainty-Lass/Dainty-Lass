//! Checks on the player's PC (row L33) over launcher-kit's `platform::system` (RL3): Tailscale, free disk
//! space and the client-reported device signals (neither the SMBIOS UUID nor the TPM key hash is attestation).

#[cfg(test)]
use std::path::{Path, PathBuf};

use launcher_kit::platform::system as kit;
use serde::Serialize;

pub use kit::{find_tailscale, free_bytes};

/// This process's candidates, from its own environment.
#[cfg(test)]
fn tailscale_candidates() -> Vec<PathBuf> {
    kit::tailscale_candidates(&|name| std::env::var_os(name))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSignals {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smbios_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpm_ek_sha256: Option<String>,
}

impl From<(Option<String>, Option<String>)> for DeviceSignals {
    fn from((smbios_uuid, tpm_ek_sha256): (Option<String>, Option<String>)) -> Self {
        Self {
            smbios_uuid,
            tpm_ek_sha256,
        }
    }
}

/// Known formats only, without firmware placeholders.
pub fn normalize_device_signals(uuid: &str, tpm: &str) -> DeviceSignals {
    kit::normalize_device_signals(Some(uuid), Some(tpm)).into()
}

/// Two fixed, hidden, bounded PowerShell queries; a missing, denied or slow TPM never drops the UUID.
pub async fn collect_device_signals() -> DeviceSignals {
    kit::read_device_signals().await.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_signals_keep_known_formats_and_drop_placeholders() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let tpm = "ab".repeat(32);
        assert_eq!(
            normalize_device_signals(&format!(" {}\r\n", uuid.to_uppercase()), &tpm.to_uppercase()),
            DeviceSignals {
                smbios_uuid: Some(uuid.into()),
                tpm_ek_sha256: Some(tpm.clone())
            }
        );
        for bad in [
            "00000000-0000-0000-0000-000000000000",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
            "03000200-0400-0500-0006-000700080009",
            "Default string",
            "unknown",
            "not-a-uuid",
        ] {
            assert_eq!(normalize_device_signals(bad, "").smbios_uuid, None, "{bad}");
        }
        for bad in [
            "0".repeat(64),
            "f".repeat(64),
            "g".repeat(64),
            "ab".into(),
            format!("{tpm}0"),
        ] {
            assert_eq!(normalize_device_signals("", &bad).tpm_ek_sha256, None, "{bad}");
        }
        assert_eq!(
            serde_json::to_value(normalize_device_signals(uuid, "")).unwrap(),
            serde_json::json!({ "smbiosUuid": uuid })
        );
    }

    #[test]
    fn free_space_of_a_missing_folder_is_its_parents() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_bytes(&dir.path().join("not").join("yet")).is_some_and(|free| free > 0));
    }

    #[test]
    fn tailscale_is_looked_for_in_the_usual_places() {
        let candidates = tailscale_candidates();
        if cfg!(windows) {
            assert!(
                candidates
                    .iter()
                    .any(|c| c.ends_with(Path::new("Tailscale").join("tailscale.exe")))
            );
        } else {
            assert_eq!(candidates[0], PathBuf::from("/usr/bin/tailscale"));
        }
    }
}
