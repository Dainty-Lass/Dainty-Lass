//! `restore --to <time>`: the newest snapshot taken by then, and the log segments after it up to that moment,
//! read with the owner's credentials and replayed into a new database on either engine
//! (`dainty_store::recovery::restore`), which checks the snapshot's log and every segment against the hash chain (DH5)
//! and names the first entry that was edited, removed or reordered.
//!
//! Whoever holds the server's key can add objects, never change or remove one (object lock). So a restore
//! trusts names only as far as the bucket backs them: every body must match the hash in its name, an object
//! the bucket received more than [`RestoreOptions::trust_window`] after the moment restored to is not used,
//! overlapping segments must agree entry for entry, and a hole in the segments' ranges is an error, not a
//! silent stop.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use dainty_store::history::Head;
use dainty_store::recovery::{RestoreReport, RestoreRequest, Snapshot};
use dainty_store::{ChangeEntry, DatabaseConfig, DatabaseOptions, DatabaseSchema};
use serde::Serialize;

use crate::Error;
use crate::bucket::{BucketRead, ObjectInfo};
use crate::layout::{
    LOG_DIR, MAX_SNAPSHOT_BYTES, SNAPSHOT_DIR, SegmentName, SnapshotName, check_prefix, decode_segment, gunzip,
    sha256_hex,
};

/// What a [`restore_to`] may read and trust.
#[derive(Debug, Clone)]
pub struct RestoreOptions {
    /// A folder for a SQLite snapshot's file while it is read.
    pub scratch: PathBuf,
    /// Objects the bucket received this long after the moment restored to are not trusted: a snapshot or a
    /// segment written then claiming an earlier time is what someone with the server's key would plant.
    pub trust_window: Duration,
    /// Keys to leave out, such as a segment a conflict named as planted.
    pub skip: Vec<String>,
    /// A snapshot key to use instead of the newest before the moment.
    pub snapshot: Option<String>,
    /// A head the owner keeps off the server (DH5), such as the one on the status page: a log whose entry
    /// has another hash, or lacks it, fails the restore.
    pub trusted_head: Option<Head>,
}

impl RestoreOptions {
    /// A one-hour trust window.
    pub fn new(scratch: impl Into<PathBuf>) -> Self {
        Self {
            scratch: scratch.into(),
            trust_window: Duration::from_secs(60 * 60),
            skip: Vec::new(),
            snapshot: None,
            trusted_head: None,
        }
    }
}

/// What a restore read and what it wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestoreOutcome {
    /// The key of the snapshot replayed from.
    pub snapshot: String,
    /// The segments read, in order.
    pub segments: Vec<String>,
    /// What `store` reports for the replay.
    pub report: RestoreReport,
    /// The newest entry the segments read hold (id, time): when it is before the moment asked for, the
    /// shipped log ends there.
    pub log_ends: Option<(i64, i64)>,
    /// Objects left out: unknown names, skipped keys, or outside the trust window.
    pub ignored: Vec<String>,
}

fn millis(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// Restores the database under `prefix` in `bucket` as it was at `to` (unix milliseconds) into `target`,
/// which must be new (see `dainty_store::recovery::restore`).
pub async fn restore_to<B: BucketRead>(
    bucket: &B,
    prefix: &str,
    to: i64,
    target: &DatabaseConfig,
    schema: &DatabaseSchema,
    options: &DatabaseOptions,
    restore: &RestoreOptions,
) -> Result<RestoreOutcome, Error> {
    check_prefix(prefix)?;
    let trusted_until = to.saturating_add(millis(restore.trust_window));
    let mut ignored = Vec::new();

    let snapshots = bucket.list(&format!("{prefix}{SNAPSHOT_DIR}"), None).await?;
    let mut best: Option<(SnapshotName, ObjectInfo)> = None;
    for object in snapshots {
        let Some(name) = SnapshotName::parse(prefix, &object.key) else {
            ignored.push(object.key);
            continue;
        };
        let chosen = restore.snapshot.as_deref() == Some(object.key.as_str());
        if restore.snapshot.is_some() && !chosen {
            continue;
        }
        if restore.skip.contains(&object.key) || (!chosen && object.last_modified > trusted_until) {
            ignored.push(object.key);
            continue;
        }
        if name.taken_at > to {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(b, _)| (name.taken_at, name.last_id) > (b.taken_at, b.last_id))
        {
            best = Some((name, object));
        }
    }
    let (name, object) = best.ok_or_else(|| Error::Restore(format!("no snapshot taken by {to} under {prefix:?}")))?;
    let body = bucket.get(&object.key).await?;
    let object_error = |reason: String| Error::Object {
        key: object.key.clone(),
        reason,
    };
    if sha256_hex(&body) != name.sha256 {
        return Err(object_error("its body doesn't match the hash in its name".into()));
    }
    let snapshot = Snapshot {
        kind: name.kind,
        last_id: name.last_id,
        taken_at: name.taken_at,
        data: gunzip(&body, MAX_SNAPSHOT_BYTES).map_err(object_error)?,
    };
    drop(body);

    let (entries, segments, log_ends) =
        read_log(bucket, prefix, name.last_id, to, trusted_until, restore, &mut ignored).await?;

    let target = target.clone();
    let schema = DatabaseSchema {
        sqlite: schema.sqlite,
        postgres: schema.postgres,
        soft_delete: schema.soft_delete,
    };
    let options = options.clone();
    let request = RestoreRequest {
        until: to,
        scratch: restore.scratch.clone(),
        trusted_head: restore.trusted_head.clone(),
    };
    let report = tokio::task::spawn_blocking(move || {
        dainty_store::recovery::restore(&target, &schema, &options, &snapshot, &entries, &request)
    })
    .await
    .map_err(|_| Error::Restore("the restore task panicked".into()))??;
    Ok(RestoreOutcome {
        snapshot: object.key,
        segments,
        report,
        log_ends,
        ignored,
    })
}

type Log = (Vec<ChangeEntry>, Vec<String>, Option<(i64, i64)>);

/// The entries after `from` up to the first one past `to`, with the keys read.
async fn read_log<B: BucketRead>(
    bucket: &B,
    prefix: &str,
    from: i64,
    to: i64,
    trusted_until: i64,
    restore: &RestoreOptions,
    ignored: &mut Vec<String>,
) -> Result<Log, Error> {
    let dir = format!("{prefix}{LOG_DIR}");
    let parse = |objects: Vec<ObjectInfo>, ignored: &mut Vec<String>| {
        let mut out = Vec::new();
        for object in objects {
            match SegmentName::parse(prefix, &object.key) {
                Some(name) if !restore.skip.contains(&object.key) && object.last_modified <= trusted_until => {
                    out.push((name, object.key));
                }
                _ => ignored.push(object.key),
            }
        }
        out
    };
    // The shipper starts a segment where each snapshot ends, so this listing usually starts at `from`; one
    // that doesn't (a lost state file shipped other ranges) is read from the start.
    let mut left_out = Vec::new();
    let mut found = parse(
        bucket.list(&dir, Some(&SegmentName::start_after(prefix, from))).await?,
        &mut left_out,
    );
    if !found.iter().any(|(n, _)| n.after == from) {
        left_out.clear();
        found = parse(bucket.list(&dir, None).await?, &mut left_out);
    }
    ignored.extend(left_out);
    found.retain(|(n, _)| n.through > from);
    found.sort_by_key(|(n, _)| (n.after, n.through));

    let mut entries: Vec<ChangeEntry> = Vec::new();
    let mut keys = Vec::new();
    let mut cursor = from;
    loop {
        // The segments that hold `cursor + 1`, and every segment overlapping the range they cover together.
        let mut end = cursor;
        let mut batch: Vec<&(SegmentName, String)> = Vec::new();
        loop {
            let before = batch.len();
            batch = found
                .iter()
                .filter(|(n, _)| n.through > cursor && (n.after <= cursor || n.after < end))
                .collect();
            end = batch.iter().map(|(n, _)| n.through).max().unwrap_or(cursor);
            if batch.len() == before {
                break;
            }
        }
        if batch.is_empty() {
            if let Some((n, key)) = found.iter().find(|(n, _)| n.after > cursor) {
                return Err(Error::Restore(format!(
                    "the log has a gap: nothing holds the entries after {cursor}, and {key} starts after {}",
                    n.after
                )));
            }
            break;
        }
        if !batch.iter().any(|(n, _)| n.after <= cursor) {
            return Err(Error::Restore(format!("the log has a gap after entry {cursor}")));
        }
        let mut merged: BTreeMap<i64, (ChangeEntry, String)> = BTreeMap::new();
        for (name, key) in batch {
            let body = bucket.get(key).await?;
            let segment = decode_segment(name, &body).map_err(|reason| Error::Object {
                key: key.clone(),
                reason,
            })?;
            for entry in segment.into_iter().filter(|e| e.id > cursor) {
                match merged.get(&entry.id) {
                    Some((seen, other)) if *seen != entry => {
                        return Err(Error::Restore(format!(
                            "{key} and {other} disagree about entry {}: one of them was not written by this \
                             server's shipper (skip it to restore from the other)",
                            entry.id
                        )));
                    }
                    Some(_) => {}
                    None => {
                        merged.insert(entry.id, (entry, key.clone()));
                    }
                }
            }
            keys.push(key.clone());
        }
        let past = merged.values().any(|(e, _)| e.at > to);
        entries.extend(merged.into_values().map(|(e, _)| e));
        cursor = end;
        if past {
            break;
        }
    }
    let log_ends = entries.last().map(|e| (e.id, e.at));
    Ok((entries, keys, log_ends))
}
