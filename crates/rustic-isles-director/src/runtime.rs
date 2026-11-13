// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The runtime every game server is started with (RD9 as of 2026-11-13, `docs/director-injection.md`): the host's
//! published `Rustic-Isles.dll` in `GAMESERVER_RUNTIME_DIR`, with the SHA-256 its `runtime.json` lists. Read at
//! each start, so a `publish-runtime` reaches the next server without a restart.

use std::path::Path;

use dainty_inject::InjectDll;
use serde_json::Value;

/// The runtime's file name in the published folder.
pub const RUNTIME_DLL: &str = "Rustic-Isles.dll";
/// What `publish-runtime` writes beside it.
pub const MANIFEST: &str = "runtime.json";
/// A `runtime.json` bigger than this is not one the host kit wrote.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The wrapper's environment on Linux: the DLL, as a host path, and its SHA-256 (`dainty-host gameserver`).
pub const ENV_INJECT_DLL: &str = "DAINTY_INJECT_DLL";
/// See [`ENV_INJECT_DLL`].
pub const ENV_INJECT_SHA256: &str = "DAINTY_INJECT_SHA256";
/// Set on every server the runtime is injected into: a loader that honours it loads nothing (the launcher kit's
/// `platform::inject::INJECTED_VAR`, `native/runtime-loader`'s stand-down).
pub const ENV_INJECTED: &str = "DAINTY_RUNTIME_INJECTED";
/// The bytes such a loader carries.
pub const STANDS_DOWN_MARKER: &[u8] = b"dainty-loader-stands-down:DAINTY_RUNTIME_INJECTED=1";

/// Whether the loader at `path` stands down for an injected runtime.
pub fn loader_stands_down(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| {
        bytes
            .windows(STANDS_DOWN_MARKER.len())
            .any(|window| window == STANDS_DOWN_MARKER)
    })
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The runtime's SHA-256 in a `runtime.json` text: exactly one `Rustic-Isles.dll` entry with 64 lowercase hex.
pub fn manifest_sha256(text: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("{MANIFEST} is not JSON: {e}"))?;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{MANIFEST} has no files list"))?;
    let mut found = files
        .iter()
        .filter(|f| f.get("name").and_then(Value::as_str) == Some(RUNTIME_DLL));
    let entry = found
        .next()
        .ok_or_else(|| format!("{MANIFEST} does not list {RUNTIME_DLL}"))?;
    if found.next().is_some() {
        return Err(format!("{MANIFEST} lists {RUNTIME_DLL} twice"));
    }
    entry
        .get("sha256")
        .and_then(Value::as_str)
        .filter(|s| is_sha256(s))
        .map(str::to_owned)
        .ok_or_else(|| format!("{MANIFEST} has no valid sha256 for {RUNTIME_DLL}"))
}

/// The DLL to inject from the published folder `dir`. The hash is the manifest's; the injector checks the file
/// against it through a held handle, so a file changed after publishing is refused there.
pub fn published(dir: &Path) -> Result<InjectDll, String> {
    if !dir.is_absolute() {
        return Err(format!(
            "GAMESERVER_RUNTIME_DIR must be an absolute path: {}",
            dir.display()
        ));
    }
    let manifest = dir.join(MANIFEST);
    let read = || -> std::io::Result<String> {
        let meta = std::fs::metadata(&manifest)?;
        if meta.len() > MAX_MANIFEST_BYTES {
            return Err(std::io::Error::other("too large"));
        }
        std::fs::read_to_string(&manifest)
    };
    let text = read().map_err(|e| format!("{}: {e}", manifest.display()))?;
    let sha256 = manifest_sha256(&text).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(InjectDll {
        path: dir.join(RUNTIME_DLL),
        sha256,
    })
}

/// Whether the director injects into `program` itself; otherwise it is a wrapper (Wine) that gets the DLL in its
/// environment and runs the `dainty-inject.exe` helper.
pub fn injects_directly(program: &str) -> bool {
    cfg!(windows)
        && Path::new(program)
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
}

/// What one start of `program` loads: the DLLs the director injects itself, or none with the wrapper's
/// [`ENV_INJECT_DLL`] set in `env`. No `dir` loads nothing (a test's fake server, or a kit from before
/// `GAMESERVER_RUNTIME_DIR`); a `dir` whose runtime can't be read refuses the start.
pub fn for_start(dir: Option<&Path>, program: &str, env: &mut Vec<(String, String)>) -> Result<Vec<InjectDll>, String> {
    // Only the director names the DLL: never a stale value from the shell or the env file
    env.retain(|(k, _)| {
        ![ENV_INJECT_DLL, ENV_INJECT_SHA256, ENV_INJECTED]
            .iter()
            .any(|name| k.eq_ignore_ascii_case(name))
    });
    let Some(dir) = dir else {
        return Ok(Vec::new());
    };
    let dll = published(dir)?;
    env.push((ENV_INJECTED.to_owned(), "1".to_owned()));
    if injects_directly(program) {
        return Ok(vec![dll]);
    }
    let path = dll
        .path
        .to_str()
        .ok_or_else(|| format!("{} is not a Unicode path", dll.path.display()))?
        .to_owned();
    env.push((ENV_INJECT_DLL.to_owned(), path));
    env.push((ENV_INJECT_SHA256.to_owned(), dll.sha256));
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn manifest(files: &str) -> String {
        format!(r#"{{"version":"20261113-abcd0123","build":{{"changelist":647472}},"files":{files}}}"#)
    }

    #[test]
    fn the_runtime_hash_is_read_strictly() {
        let good = manifest(&format!(
            r#"[{{"name":"Rustic-Isles.dll","size":3,"sha256":"{SHA}"}},{{"name":"winmm.dll","size":1,"sha256":"{SHA}"}}]"#
        ));
        assert_eq!(manifest_sha256(&good).unwrap(), SHA);
        let without_loader = manifest(&format!(r#"[{{"name":"Rustic-Isles.dll","size":3,"sha256":"{SHA}"}}]"#));
        assert_eq!(manifest_sha256(&without_loader).unwrap(), SHA);
        for bad in [
            "not json".to_owned(),
            "{}".to_owned(),
            manifest("[]"),
            manifest(&format!(r#"[{{"name":"winmm.dll","sha256":"{SHA}"}}]"#)),
            manifest(r#"[{"name":"Rustic-Isles.dll","sha256":"abc"}]"#),
            manifest(&format!(
                r#"[{{"name":"Rustic-Isles.dll","sha256":"{}"}}]"#,
                SHA.to_uppercase()
            )),
            manifest(r#"[{"name":"Rustic-Isles.dll"}]"#),
            manifest(&format!(
                r#"[{{"name":"Rustic-Isles.dll","sha256":"{SHA}"}},{{"name":"Rustic-Isles.dll","sha256":"{SHA}"}}]"#
            )),
        ] {
            assert!(manifest_sha256(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_published_folder_names_the_dll_by_full_path() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(published(tmp.path()).unwrap_err().contains(MANIFEST));
        let text = manifest(&format!(r#"[{{"name":"Rustic-Isles.dll","size":3,"sha256":"{SHA}"}}]"#));
        std::fs::write(tmp.path().join(MANIFEST), text).unwrap();
        let dll = published(tmp.path()).unwrap();
        assert_eq!(dll.path, tmp.path().join(RUNTIME_DLL));
        assert_eq!(dll.sha256, SHA);
        std::fs::write(tmp.path().join(MANIFEST), vec![b' '; 70 * 1024]).unwrap();
        assert!(published(tmp.path()).is_err());
    }

    #[test]
    fn a_start_injects_or_hands_the_wrapper_the_dll() {
        let tmp = tempfile::tempdir().unwrap();
        let text = manifest(&format!(r#"[{{"name":"Rustic-Isles.dll","size":3,"sha256":"{SHA}"}}]"#));
        std::fs::write(tmp.path().join(MANIFEST), text).unwrap();
        let stale = || {
            vec![
                (ENV_INJECT_DLL.to_owned(), "/evil.dll".to_owned()),
                ("A".to_owned(), "1".to_owned()),
            ]
        };

        let mut env = stale();
        env.push((ENV_INJECTED.to_owned(), "1".to_owned()));
        assert!(for_start(None, "game.exe", &mut env).unwrap().is_empty());
        assert_eq!(env, [("A".to_owned(), "1".to_owned())]);

        let mut env = stale();
        let wrapper = for_start(Some(tmp.path()), "/srv/.workspace/bin/dainty-gameserver", &mut env).unwrap();
        assert!(wrapper.is_empty());
        let dll = tmp.path().join(RUNTIME_DLL).to_string_lossy().into_owned();
        assert_eq!(
            env[1..],
            [
                (ENV_INJECTED.to_owned(), "1".to_owned()),
                (ENV_INJECT_DLL.to_owned(), dll),
                (ENV_INJECT_SHA256.to_owned(), SHA.to_owned())
            ]
        );

        if cfg!(windows) {
            let mut env = stale();
            let direct = for_start(Some(tmp.path()), r"C:\g\Dauntless-Win64-Shipping.exe", &mut env).unwrap();
            assert_eq!(direct.len(), 1);
            assert_eq!(env[1..], [(ENV_INJECTED.to_owned(), "1".to_owned())]);
        }

        std::fs::remove_file(tmp.path().join(MANIFEST)).unwrap();
        assert!(for_start(Some(tmp.path()), "game.exe", &mut stale()).is_err());
    }

    #[test]
    fn only_a_windows_exe_is_injected_directly() {
        assert_eq!(
            injects_directly(r"C:\g\Win64\Dauntless-Win64-Shipping.exe"),
            cfg!(windows)
        );
        assert!(!injects_directly("/srv/.workspace/bin/dainty-gameserver"));
    }
}
