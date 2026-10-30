//! Host tools (`RA12`), re-exported from `launcher_kit`.

pub use launcher_kit::platform::hostkit::*;

/// The Host page's test stand-in (`fixtures/host`): built once per test run with this toolchain.
#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    fn out_dir() -> &'static Path {
        static BUILT: OnceLock<PathBuf> = OnceLock::new();
        BUILT.get_or_init(|| {
            let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
            let target = manifest.join("..").join("..").join("target").join("host-fixtures");
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "--quiet", "--locked", "--manifest-path"])
                .arg(manifest.join("fixtures").join("host").join("Cargo.toml"))
                .arg("--target-dir")
                .arg(&target)
                .status()
                .expect("cargo runs");
            assert!(status.success(), "the host fixtures build");
            target.join("debug")
        })
    }

    /// `fake-host-kit`.
    pub fn exe(name: &str) -> PathBuf {
        out_dir().join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }
}
