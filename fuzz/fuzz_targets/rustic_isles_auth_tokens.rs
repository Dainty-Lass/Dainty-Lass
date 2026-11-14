//! Tokens and runtime reports Rustic Isles' sign-in area checks (spec-rustic-isles-auth §13): JWT validation of
//! arbitrary text accepts nothing the test key didn't sign, and the artifact-set check of arbitrary JSON against a
//! fixed manifest accepts only sets that match it.
#![no_main]

use std::sync::{Arc, OnceLock};

use libfuzzer_sys::fuzz_target;
use rustic_isles_auth::test_support::{MapSettings, MemoryStore, throwaway_signing_keys};
use rustic_isles_auth::{Auth, AuthConfig, Channel, Seams, runtime};
use serde_json::{Value, json};

fn auth() -> &'static Arc<Auth<MemoryStore>> {
    static AUTH: OnceLock<Arc<Auth<MemoryStore>>> = OnceLock::new();
    AUTH.get_or_init(|| {
        let (private, public) = throwaway_signing_keys();
        let settings = MapSettings::new(&[("AUTH_SIGNING_PRIVKEY_B64", &private), ("AUTH_SIGNING_PUBKEY_B64", &public)]);
        let config = AuthConfig::for_1_14_7(dainty_module_api::NodeIdentity::standalone());
        Auth::new(config, Arc::new(settings), None, Seams::default())
    })
}

fn manifest() -> Value {
    json!({"version": "0.1.78", "targetChangelist": 647472, "sha256": "b".repeat(64),
        "extraFiles": [{"name": "winmm.dll", "sha256": "c".repeat(64)}, {"name": "a.pak", "sha256": "d".repeat(64)}]})
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let auth = auth();
    // Nothing random is signed by the test key
    assert!(auth.validate_game_token(&text).is_err());
    let _ = auth.validate_launcher_token(&text);
    let _ = auth.validate_store_token(&text);

    if let Ok(report) = serde_json::from_str::<Value>(&text) {
        let manifest = manifest();
        let checked = runtime::assert_artifact_set(
            &manifest,
            647_472,
            Channel::Stable,
            report.get("runtimeManifestVersion"),
            report.get("runtimeArtifacts"),
        );
        if let Ok(approved) = checked {
            assert_eq!(approved.manifest_version, "0.1.78");
            let wanted = [("rustic-isles-runtime.dll", 'b'), ("winmm.dll", 'c'), ("a.pak", 'd')];
            for (name, hash) in &approved.artifacts {
                let (_, c) = wanted.iter().find(|(n, _)| n == name).copied().unwrap_or(("", '?'));
                assert_eq!(*hash, c.to_string().repeat(64));
            }
            assert!(approved.artifacts.len() == 2 || approved.artifacts.len() == 3);
        }
    }
});
