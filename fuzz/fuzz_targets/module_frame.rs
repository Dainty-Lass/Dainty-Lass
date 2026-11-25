//! What a module child writes to the core and the core to it (docs/security/module-host.md): length-prefixed
//! JSON frames. None of it may panic; a frame is never read past its length or over the cap for its stage, and a
//! message decoded and written again decodes to the same message.
#![no_main]

use dainty_module_api::frame::{Hello, MAX_FRAME, MAX_HELLO_FRAME, ToHost, ToModule, check_hello, decode, encode};
use libfuzzer_sys::fuzz_target;
use serde::Serialize;
use serde::de::DeserializeOwned;

fn round_trip<T: DeserializeOwned + Serialize + PartialEq + Clone + std::fmt::Debug>(
    data: &[u8],
    max: usize,
) -> Option<T> {
    let Ok(Some((message, used))) = decode::<T>(data, max) else {
        return None;
    };
    // The prefix is 4 bytes and the body is within the cap
    assert!(used >= 4 && used <= data.len() && used - 4 <= max);
    let frame = encode(&message, max).ok()?;
    let again = decode::<T>(&frame, max)
        .ok()
        .flatten()
        .map(|(m, n)| (m, n == frame.len()));
    assert_eq!(again, Some((message.clone(), true)));
    Some(message)
}

fuzz_target!(|data: &[u8]| {
    // A child's first frame has the smaller cap: the core reads it before it knows the child
    if let Some(hello) = round_trip::<Hello>(data, MAX_HELLO_FRAME) {
        let _ = check_hello(&hello, None);
    }
    round_trip::<ToHost>(data, MAX_FRAME);
    round_trip::<ToModule>(data, MAX_FRAME);
    // Several frames back to back: each starts where the last one ended
    let mut rest = data;
    while let Ok(Some((_, used))) = decode::<ToHost>(rest, MAX_FRAME) {
        rest = &rest[used..];
    }
});
