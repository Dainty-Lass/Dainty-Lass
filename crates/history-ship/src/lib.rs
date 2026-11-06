//! Off-server history (DD23 layer 2, DH4): `store`'s change log streamed and the database snapshotted on a
//! schedule into an S3-compatible bucket with object lock (S3, Backblaze B2, MinIO, Cloudflare R2), and a
//! restore that replays a snapshot and the log up to a chosen moment, into either engine.
//!
//! The server holds write-only credentials: it PUTs, and never lists, reads or deletes. Object names carry
//! everything a restore needs ([`layout`]), and the server keeps its progress in a local state file
//! ([`ShipState`]), so nothing it does depends on reading the bucket back. The owner's own credentials,
//! which can list and read, are the only way back in ([`restore_to`]). Object lock keeps every version the
//! server wrote, so a partner with root on it can stop the shipping but not take back what already left.
//!
//! [`lag()`] says how far shipping has fallen behind, for the status page and the admin API.

pub mod bucket;
pub mod lag;
pub mod layout;
pub mod restore;
pub mod s3;
pub mod ship;

pub use bucket::{Bucket, BucketRead, MemoryBucket, ObjectInfo};
pub use lag::{Lag, LagAlert, LagLimits, assess, lag};
pub use restore::{RestoreOptions, RestoreOutcome, restore_to};
pub use s3::{S3Bucket, S3Config};
pub use ship::{ShipSettings, ShipState, Shipper};

/// What shipping and restoring return.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The local database failed.
    #[error(transparent)]
    Store(#[from] store::Error),
    /// The bucket refused or couldn't be reached. Never carries a credential or a signature.
    #[error("bucket: {0}")]
    Bucket(String),
    /// An object a restore read doesn't hold what its name says: damaged, or written by someone else.
    #[error("bucket object {key}: {reason}")]
    Object {
        /// The object's key.
        key: String,
        /// What did not match.
        reason: String,
    },
    /// The local log doesn't chain (DH5): it was edited, truncated or reordered since it was last shipped.
    #[error("the local change log is broken: {0}")]
    Chain(String),
    /// The shipper's state file could not be read, parsed or written.
    #[error("history state file {path}: {reason}")]
    State {
        /// The state file.
        path: String,
        /// The I/O or JSON error.
        reason: String,
    },
    /// A restore could not be carried out: no snapshot before the moment, a gap in the segments, or a conflict.
    #[error("restore: {0}")]
    Restore(String),
    /// A setting was refused: the prefix, the endpoint or bucket name, or a zero interval.
    #[error("bucket settings: {0}")]
    Config(String),
}
