//! What a MongoDB server answers the database tool (spec-rustic-isles-db §4.3, §15): arbitrary bytes as the
//! replies to a `find` and its `getMore`s, over an in-memory stream. Nothing panics, nothing larger than a
//! reply's declared length (at most 64 MiB) is allocated, and the answer is documents or an error.
#![no_main]

use std::io::{Cursor, Read, Write};

use libfuzzer_sys::fuzz_target;
use rustic_isles_db::mongo::{MongoClient, read_reply};

/// Reads come from the fuzz bytes; writes (the requests) go nowhere.
struct Replay(Cursor<Vec<u8>>);

impl Read for Replay {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Replay {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let _ = read_reply(&mut &data[..]);
    let mut client = MongoClient::over_stream(Box::new(Replay(Cursor::new(data.to_vec()))), "dauntless");
    if let Ok(documents) = client.find_all("accounts") {
        // Every document came out of the bytes, so there can't be more of them than bytes
        assert!(documents.len() <= data.len());
    }
    let mut probe = MongoClient::over_stream(Box::new(Replay(Cursor::new(data.to_vec()))), "dauntless");
    let _ = probe.collection_has_documents("accounts");
});
