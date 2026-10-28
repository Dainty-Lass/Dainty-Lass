//! What the host reads from an external module's stdout, and the module from the host's (DF2, DD5, DX2):
//! length-prefixed frames of either direction, the hello the host checks before anything else, and the
//! names inside them. A frame over its limit is refused before its body is read, nothing panics, and
//! whatever is accepted writes back as the same message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use module_api::chat::Jid;
use module_api::frame::{self, MAX_FRAME, MAX_HELLO_FRAME, ModuleAnswer, ToHost, ToModule, check_hello};
use module_api::ids::{ModuleId, Sha256Hex};
use module_api::stub::StubModule;
use module_api::{EventName, GameModule, GameVersion, GlobalId};
use serde::Serialize;
use serde::de::DeserializeOwned;

const LENGTH_BYTES: usize = 4;

/// Decodes every frame in `bytes` as `T`, checking each accepted one round-trips.
fn frames<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    bytes: &[u8],
    max: usize,
    mut each: impl FnMut(&T),
) {
    let mut at = 0;
    while let Ok(Some((message, used))) = frame::decode::<T>(&bytes[at..], max) {
        assert!(used > LENGTH_BYTES && used - LENGTH_BYTES <= max);
        // Escapes can make the written body longer than the one read, so the bound is relaxed for it
        let written = frame::encode(&message, max * 8).expect("an accepted message writes back");
        let (again, _) = frame::decode::<T>(&written, max * 8)
            .expect("and reads back")
            .expect("whole");
        assert_eq!(again, message);
        each(&message);
        at += used;
    }
}

fn canonical<T: std::str::FromStr + ToString>(text: &str) {
    if let Ok(value) = text.parse::<T>() {
        assert_eq!(value.to_string(), text);
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, bytes)) = data.split_first() else {
        return;
    };
    let max = if mode & 1 == 0 { MAX_FRAME } else { MAX_HELLO_FRAME };
    let stub = StubModule::new().manifest();
    frames::<ToHost>(bytes, max, |message| match message {
        ToHost::Hello(hello) => {
            if check_hello(hello, None).is_ok() {
                assert!(hello.manifest.validate().is_ok());
                assert!(!hello.manifest.display_name.is_empty());
            }
        }
        ToHost::Reply {
            result: Ok(ModuleAnswer::Registered(registration)),
            ..
        } => {
            _ = registration.check(&stub);
        }
        _ => {}
    });
    frames::<ToModule>(bytes, max, |_| {});

    // The length is checked before the body: a header over the limit is refused whatever follows
    if let Some(head) = bytes.first_chunk::<LENGTH_BYTES>() {
        let len = u32::from_be_bytes(*head) as usize;
        if len == 0 || len > max {
            assert!(frame::decode::<ToHost>(bytes, max).is_err());
        }
    }

    let text = String::from_utf8_lossy(bytes);
    canonical::<GameVersion>(&text);
    canonical::<ModuleId>(&text);
    canonical::<GlobalId>(&text);
    canonical::<Jid>(&text);
    canonical::<EventName>(&text);
    canonical::<Sha256Hex>(&text);
});
