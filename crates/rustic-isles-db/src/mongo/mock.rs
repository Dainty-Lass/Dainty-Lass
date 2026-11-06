// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A MongoDB stand-in for tests (spec §16.1): OP_MSG on 127.0.0.1:0, one thread per connection, `hello`,
//! `find`, `getMore`, `delete` and `insert` over collections held in memory. No authentication.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use serde_json::{Map, Value, json};

use super::{OP_MSG, decode_document, encode_document};

type Collections = BTreeMap<String, Vec<Map<String, Value>>>;

/// An in-memory MongoDB server on loopback for tests; it stops accepting when dropped.
pub struct MockMongoServer {
    address: SocketAddr,
    collections: Arc<Mutex<Collections>>,
    stop: Arc<AtomicBool>,
}

fn lock(collections: &Mutex<Collections>) -> MutexGuard<'_, Collections> {
    collections.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl MockMongoServer {
    /// A server whose cursors return at most `max_batch` documents a batch (at least 1).
    pub fn start(max_batch: usize) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let collections = Arc::new(Mutex::new(Collections::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopping) = (collections.clone(), stop.clone());
        let max_batch = max_batch.max(1);
        thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let shared = shared.clone();
                thread::spawn(move || serve(stream, &shared, max_batch));
            }
        });
        Ok(Self {
            address,
            collections,
            stop,
        })
    }

    /// Where the server listens, for a `mongodb://` URI.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// `mongodb://127.0.0.1:<port>/<database>`.
    pub fn uri(&self, database: &str) -> String {
        format!("mongodb://{}/{database}", self.address)
    }

    /// Appends documents (JSON objects; anything else is skipped) to a collection.
    pub fn seed(&self, collection: &str, documents: Vec<Value>) {
        let mut all = lock(&self.collections);
        let docs = all.entry(collection.to_owned()).or_default();
        docs.extend(documents.into_iter().filter_map(|d| d.as_object().cloned()));
    }

    /// Every collection's documents as they stand now.
    pub fn snapshot(&self) -> BTreeMap<String, Vec<Map<String, Value>>> {
        lock(&self.collections).clone()
    }
}

impl Drop for MockMongoServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop so it sees the flag
        let _ = TcpStream::connect(self.address);
    }
}

fn read_message(stream: &mut TcpStream) -> Option<Map<String, Value>> {
    let mut header = [0u8; 16];
    stream.read_exact(&mut header).ok()?;
    let total = usize::try_from(i32::from_le_bytes([header[0], header[1], header[2], header[3]])).ok()?;
    let mut rest = vec![0u8; total.checked_sub(16).filter(|n| *n <= 64 << 20)?];
    stream.read_exact(&mut rest).ok()?;
    // Flags, then one kind-0 section
    (rest.get(4) == Some(&0)).then(|| decode_document(&rest[5..]).ok())?
}

fn write_reply(stream: &mut TcpStream, request: i32, body: &Value) -> Option<()> {
    let bytes = encode_document(body.as_object()?).ok()?;
    let total = i32::try_from(bytes.len() + 21).ok()?;
    let mut out = Vec::new();
    for n in [total, request, request, OP_MSG] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0, 0, 0, 0]);
    out.extend(bytes);
    stream.write_all(&out).ok()
}

struct Cursor {
    collection: String,
    left: Vec<Map<String, Value>>,
    batch: usize,
}

fn serve(mut stream: TcpStream, collections: &Mutex<Collections>, max_batch: usize) {
    let mut cursors: HashMap<i64, Cursor> = HashMap::new();
    let mut next_cursor = 1i64;
    let mut request = 0;
    while let Some(command) = read_message(&mut stream) {
        request += 1;
        let reply = answer(&command, collections, max_batch, &mut cursors, &mut next_cursor);
        if write_reply(&mut stream, request, &reply).is_none() {
            break;
        }
    }
}

fn batch_of(cursor: &mut Cursor) -> Vec<Value> {
    let n = cursor.batch.min(cursor.left.len());
    cursor.left.drain(..n).map(Value::Object).collect()
}

fn answer(
    command: &Map<String, Value>,
    collections: &Mutex<Collections>,
    max_batch: usize,
    cursors: &mut HashMap<i64, Cursor>,
    next_cursor: &mut i64,
) -> Value {
    let Some((name, target)) = command.iter().next() else {
        return json!({"ok": 0, "errmsg": "empty command"});
    };
    let collection = target.as_str().unwrap_or_default().to_owned();
    let number = |key: &str| command.get(key).and_then(Value::as_i64);
    match name.as_str() {
        "hello" | "ismaster" | "isMaster" => json!({"isWritablePrimary": true, "maxWireVersion": 21, "ok": 1}),
        "find" => {
            let mut left = lock(collections).get(&collection).cloned().unwrap_or_default();
            if let Some(limit) = number("limit").and_then(|n| usize::try_from(n).ok()).filter(|n| *n > 0) {
                left.truncate(limit);
            }
            let asked = number("batchSize")
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(max_batch);
            let mut cursor = Cursor {
                collection: collection.clone(),
                left,
                batch: asked.clamp(1, max_batch),
            };
            let first = batch_of(&mut cursor);
            let single = command.get("singleBatch") == Some(&Value::Bool(true));
            let id = if single || cursor.left.is_empty() {
                0
            } else {
                let id = *next_cursor;
                *next_cursor += 1;
                cursors.insert(id, cursor);
                id
            };
            json!({"cursor": {"firstBatch": first, "id": id, "ns": format!("dauntless.{collection}")}, "ok": 1})
        }
        "getMore" => {
            let id = target.as_i64().unwrap_or(0);
            let Some(mut cursor) = cursors.remove(&id) else {
                return json!({"ok": 0, "errmsg": "cursor not found"});
            };
            let next = batch_of(&mut cursor);
            let ns = format!("dauntless.{}", cursor.collection);
            let id = if cursor.left.is_empty() {
                0
            } else {
                cursors.insert(id, cursor);
                id
            };
            json!({"cursor": {"nextBatch": next, "id": id, "ns": ns}, "ok": 1})
        }
        "delete" => {
            let n = lock(collections).remove(&collection).map_or(0, |d| d.len());
            json!({"n": n, "ok": 1})
        }
        "insert" => {
            let docs: Vec<Map<String, Value>> = command
                .get("documents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|d| d.as_object().cloned())
                .collect();
            let n = docs.len();
            lock(collections).entry(collection).or_default().extend(docs);
            json!({"n": n, "ok": 1})
        }
        _ => json!({"ok": 0, "errmsg": "unsupported command"}),
    }
}
