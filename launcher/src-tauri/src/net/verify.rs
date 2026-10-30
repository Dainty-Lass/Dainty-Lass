//! SHA-256 of files on disk (`L24`), re-exported from `launcher_kit` with the 1.4.4 manifest verifier
//! in `install.rs` (`legacy` feature).

#[cfg(feature = "legacy")]
pub mod install;

pub use launcher_kit::net::verify::*;

#[cfg(all(feature = "legacy", test))]
pub use install::FileProblem;
#[cfg(feature = "legacy")]
pub use install::{Trusted, VerifiedCache, VerifyOptions, remove_part_files, verify_install};
