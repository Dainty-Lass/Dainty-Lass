//! The shipper: on every tick, the log entries after the last one shipped go up as one or more segments,
//! and a snapshot when one is due. Progress lives in a local state file, never in the bucket, which the
//! server can't read. The state is saved after every object, so a crash ships at most one segment twice;
//! a lost state file ships the whole log again, under new names a restore reconciles (`restore`).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use store::history::{Chain, Head};
use store::{ChangeEntry, Database};

use crate::Error;
use crate::bucket::Bucket;
use crate::layout::{SnapshotName, check_prefix, encode_segment, gzip, sha256_hex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShipSettings {
    /// Keys start with this (`` or `server-name/`), so one bucket can hold several databases.
    pub prefix: String,
    pub state_file: PathBuf,
    /// How often the log is shipped: the most a stopped server loses.
    pub interval: Duration,
    /// How often a snapshot is taken: what a restore replays the log from.
    pub snapshot_every: Duration,
    /// A segment holds at most this many entries...
    pub segment_entries: usize,
    /// ...and at most about this many bytes of JSON.
    pub segment_bytes: usize,
}

impl ShipSettings {
    /// Every 10 seconds, a snapshot a day.
    pub fn new(prefix: impl Into<String>, state_file: impl Into<PathBuf>) -> Self {
        Self {
            prefix: prefix.into(),
            state_file: state_file.into(),
            interval: Duration::from_secs(10),
            snapshot_every: Duration::from_secs(24 * 60 * 60),
            segment_entries: 10_000,
            segment_bytes: 8 << 20,
        }
    }
}

/// What the shipper has done, as its state file holds it. Times are unix milliseconds by the database's
/// clock. Read by [`crate::lag`] in whatever process shows the status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShipState {
    /// The newest entry in the bucket: the next segment starts after it.
    pub last_shipped_id: i64,
    /// Its hash (DH5): the head shipped. The next segment must chain onto it, or the local log was rewritten.
    pub last_shipped_hash: Option<String>,
    pub last_shipped_entry_at: Option<i64>,
    /// When the last segment went up.
    pub last_upload_at: Option<i64>,
    /// The last snapshot uploaded: when it was taken and the log it holds.
    pub last_snapshot_at: Option<i64>,
    pub last_snapshot_id: Option<i64>,
    /// Every tick, whatever it managed: a stopped shipper stops this.
    pub heartbeat_at: Option<i64>,
    /// The last tick's failure, cleared by a tick that succeeds.
    pub last_error: Option<String>,
    pub last_error_at: Option<i64>,
    /// Set while the local log fails the hash chain, whatever else the tick did.
    pub chain_broken: Option<String>,
}

impl ShipState {
    /// The state in `path`; a missing file is a shipper that never ran.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let error = |reason: String| Error::State {
            path: path.display().to_string(),
            reason,
        };
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| error(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(error(e.to_string())),
        }
    }

    /// Written beside and renamed over, so a reader never sees half a file.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let error = |reason: String| Error::State {
            path: path.display().to_string(),
            reason,
        };
        let mut temp = path.as_os_str().to_owned();
        temp.push(".tmp");
        let temp = PathBuf::from(temp);
        let json = serde_json::to_vec_pretty(self).map_err(|e| error(e.to_string()))?;
        std::fs::write(&temp, json).map_err(|e| error(e.to_string()))?;
        std::fs::rename(&temp, path).map_err(|e| error(e.to_string()))
    }
}

/// Entries read from the database per query.
const PAGE: i64 = 1000;
const CONTENT_TYPE: &str = "application/gzip";

pub struct Shipper<B> {
    db: Database,
    bucket: B,
    settings: ShipSettings,
    state: ShipState,
}

impl<B: Bucket> Shipper<B> {
    pub fn new(db: Database, bucket: B, settings: ShipSettings) -> Result<Self, Error> {
        check_prefix(&settings.prefix)?;
        if settings.segment_entries == 0 || settings.interval.is_zero() || settings.snapshot_every.is_zero() {
            return Err(Error::Config(
                "the shipping intervals and segment size must be above zero".into(),
            ));
        }
        let state = ShipState::load(&settings.state_file)?;
        Ok(Self {
            db,
            bucket,
            settings,
            state,
        })
    }

    pub fn state(&self) -> &ShipState {
        &self.state
    }

    fn save(&self) -> Result<(), Error> {
        self.state.save(&self.settings.state_file)
    }

    /// Ships every entry not shipped yet. Returns how many went.
    pub async fn ship_log(&mut self) -> Result<usize, Error> {
        self.ship_through(i64::MAX).await
    }

    /// Ships the entries up to `through`, in segments. Each entry is checked against the hash chain from the
    /// last shipped head (genesis for a first shipment) before it leaves, so a log edited, cut or reordered
    /// since is not shipped on and the failure names the entry (DH5).
    async fn ship_through(&mut self, through: i64) -> Result<usize, Error> {
        let mut shipped = 0;
        loop {
            let after = self.state.last_shipped_id;
            let mut chain = match (&self.state.last_shipped_hash, after) {
                (Some(hash), id) if id > 0 => Some(Chain::after(&Head { id, hash: hash.clone() })),
                (_, 0) => Some(Chain::genesis()),
                // A state file from before the head was kept: the first entry names what it follows.
                _ => None,
            };
            let mut batch: Vec<ChangeEntry> = Vec::new();
            let mut bytes = 0usize;
            let mut cursor = after;
            'fill: while batch.len() < self.settings.segment_entries && bytes < self.settings.segment_bytes {
                let page = self.db.changes(cursor, PAGE).await?;
                let full = i64::try_from(page.len()).unwrap_or(i64::MAX) == PAGE;
                for entry in page {
                    if entry.id > through {
                        break 'fill;
                    }
                    let chain = chain.get_or_insert_with(|| {
                        Chain::after(&Head {
                            id: after,
                            hash: entry.prev_hash.clone().unwrap_or_default(),
                        })
                    });
                    if let Err(broken) = chain.check(&entry) {
                        self.state.chain_broken = Some(broken.to_string());
                        self.save()?;
                        return Err(Error::Chain(broken.to_string()));
                    }
                    cursor = entry.id;
                    bytes += entry.before.as_ref().map_or(0, String::len)
                        + entry.after.as_ref().map_or(0, String::len)
                        + entry.row_key.len()
                        + 64;
                    batch.push(entry);
                    if batch.len() >= self.settings.segment_entries || bytes >= self.settings.segment_bytes {
                        break 'fill;
                    }
                }
                if !full {
                    break;
                }
            }
            let Some(last) = batch.last() else {
                return Ok(shipped);
            };
            let (last_id, last_at, last_hash) = (last.id, last.at, last.hash.clone());
            let (name, body) = encode_segment(after, &batch)?;
            self.bucket
                .put(&name.key(&self.settings.prefix), body, CONTENT_TYPE)
                .await?;
            self.state.last_shipped_id = last_id;
            self.state.last_shipped_hash = last_hash;
            self.state.last_shipped_entry_at = Some(last_at);
            self.state.last_upload_at = Some(self.db.now());
            self.save()?;
            shipped += batch.len();
        }
    }

    /// Takes a snapshot and uploads it, after the log up to it, so a segment starts where it ends.
    /// Returns its key.
    pub async fn snapshot(&mut self) -> Result<String, Error> {
        let snapshot = self.db.snapshot().await?;
        self.ship_through(snapshot.last_id).await?;
        let body = gzip(&snapshot.data)?;
        let name = SnapshotName {
            taken_at: snapshot.taken_at,
            last_id: snapshot.last_id,
            kind: snapshot.kind,
            sha256: sha256_hex(&body),
        };
        let key = name.key(&self.settings.prefix);
        self.bucket.put(&key, body, CONTENT_TYPE).await?;
        self.state.last_snapshot_at = Some(snapshot.taken_at);
        self.state.last_snapshot_id = Some(snapshot.last_id);
        self.save()?;
        Ok(key)
    }

    /// One round: a snapshot if one is due (the first round always takes one), then the log. Records the
    /// heartbeat and any failure in the state file whatever happens.
    pub async fn tick(&mut self) -> Result<(), Error> {
        let now = self.db.now();
        let every = i64::try_from(self.settings.snapshot_every.as_millis()).unwrap_or(i64::MAX);
        let due = self
            .state
            .last_snapshot_at
            .is_none_or(|at| now.saturating_sub(at) >= every);
        let result = async {
            if due {
                self.snapshot().await?;
            }
            self.ship_log().await.map(|_| ())
        }
        .await;
        self.state.heartbeat_at = Some(self.db.now());
        match &result {
            Ok(()) => {
                self.state.last_error = None;
                self.state.last_error_at = None;
                self.state.chain_broken = None;
            }
            Err(e) => {
                self.state.last_error = Some(e.to_string());
                self.state.last_error_at = Some(self.db.now());
            }
        }
        self.save()?;
        result
    }

    /// Ticks every interval until `shutdown` resolves, then ships what is left once more. Failures are in
    /// the state file for the status page; `on_error` hears each one (for the service's log).
    pub async fn run(mut self, shutdown: impl Future<Output = ()>, on_error: impl Fn(&Error)) {
        tokio::pin!(shutdown);
        loop {
            if let Err(e) = self.tick().await {
                on_error(&e);
            }
            tokio::select! {
                () = &mut shutdown => break,
                () = tokio::time::sleep(self.settings.interval) => {}
            }
        }
        if let Err(e) = self.ship_log().await {
            on_error(&e);
        }
    }
}
