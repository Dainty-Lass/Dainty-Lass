//! What a module library hands the core (RD27, docs/security/plugin-host.md): bytes in any chunking, read through
//! the same stream and frame reader a child's stdout goes through, are a hello and frames within their limits or
//! refused, never a panic, and the chunking never changes what is read.
#![no_main]

use libfuzzer_sys::fuzz_target;
use module_api::frame::{MAX_FRAME, MAX_HELLO_FRAME, ToHost, read_frame};
use module_api::plugin::ChannelReader;
use tokio::io::BufReader;

/// Every frame read from `chunks` sent one by one, until the stream ends or a frame is refused.
fn frames(chunks: Vec<Vec<u8>>) -> Vec<Result<ToHost, String>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    for chunk in chunks {
        _ = tx.send(chunk);
    }
    drop(tx);
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
        return Vec::new();
    };
    runtime.block_on(async {
        let mut reader = BufReader::new(ChannelReader::new(rx));
        let mut out = Vec::new();
        let mut limit = MAX_HELLO_FRAME;
        loop {
            match read_frame::<ToHost, _>(&mut reader, limit).await {
                Ok(Some(frame)) => out.push(Ok(frame)),
                Ok(None) => break,
                Err(error) => {
                    out.push(Err(error.to_string()));
                    break;
                }
            }
            limit = MAX_FRAME;
        }
        out
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&cut, bytes)) = data.split_first() else {
        return;
    };
    let whole = frames(vec![bytes.to_vec()]);
    let step = usize::from(cut).max(1);
    let chunked = frames(bytes.chunks(step).map(<[u8]>::to_vec).collect());
    assert_eq!(format!("{whole:?}"), format!("{chunked:?}"));
});
