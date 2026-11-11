//! What the catalog exporter (`native/rustic-isles-exporter`, RD27) reads besides the game's own objects: the flags file
//! the launcher or a host writes, the SDK headers `build.rs` extracts layouts from, and the readers run over memory
//! of any content (spec-catalog-exporter §3.2, §4.4, §9).
#![no_main]

use catalog_exporter::engine::Engine;
use catalog_exporter::facts;
use catalog_exporter::flags::{Flag, Flags};
use catalog_exporter::json;
use catalog_exporter::memory::{NoCalls, Synthetic};
use catalog_exporter::sdk_parse::Sdk;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The flags: the same bytes give the same flags, a later line wins, the status line stays ASCII
    let flags = Flags::parse(data);
    assert_eq!(flags, Flags::parse(data));
    assert!(flags.status_line().is_ascii());
    let mut later = data.to_vec();
    later.extend_from_slice(b"\nEXPORT_MASTERY=1\nEXPORT_CATALOG=0\n");
    let later = Flags::parse(&later);
    assert!(later.get(Flag::Mastery) && !later.get(Flag::Catalog));

    // The SDK extractor: any header resolves or names what is missing, never panics
    let mut sdk = Sdk::default();
    sdk.parse(&String::from_utf8_lossy(data));
    for (_, path) in facts::MEMBERS {
        let _ = sdk.resolve(path);
    }
    for (_, path, _) in facts::OPTIONAL_MEMBERS {
        let _ = sdk.resolve(path);
    }
    for (_, owner) in facts::BASE_OWNERS {
        for member in facts::BASE_MEMBERS {
            let _ = sdk.resolve(&format!("{owner}.{member}"));
        }
    }
    for (_, path) in facts::ENUMS {
        let _ = sdk.enum_value(path);
    }

    // Escaping leaves no raw control byte in a quoted value
    let mut quoted = Vec::new();
    json::escape_into(&mut quoted, data);
    assert!(quoted.iter().all(|&byte| byte >= 0x20));

    // The readers on arbitrary memory: names, strings, texts, arrays and maps read or come back empty
    let mut mem = Synthetic::new();
    let base = mem.put(data);
    let engine = Engine::new(&mem, &NoCalls, base, base);
    for at in (base..base + data.len() as u64).step_by(4).take(64) {
        let _ = engine.fname_raw(at);
        let _ = engine.fstring(at);
        let _ = engine.ftext(at);
        let _ = engine.weak_object(at);
        let (_, elements) = engine.map_elements(at, 0x18);
        assert!(elements.len() <= 1_000_000);
        let (_, count) = engine.array(at);
        assert!(count >= 0);
    }
});
