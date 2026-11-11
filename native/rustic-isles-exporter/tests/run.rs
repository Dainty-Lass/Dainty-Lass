//! The whole run on a synthetic image (spec §3.1, §6.15), and the readers' edge cases (spec §4.3, §4.4).
#![allow(clippy::unwrap_used, reason = "test helpers fail the test")]

mod common;

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Duration;

use catalog_exporter::clock::Utc;
use catalog_exporter::flags::Flags;
use catalog_exporter::memory::NoCalls;
use catalog_exporter::run::{self, Host, MANIFEST, Results};
use catalog_exporter::sdk;
use catalog_exporter::status::no_debug;
use common::{FakeCatalog, World, read, status_text, temp_dir};

const NOW: Utc = Utc {
    year: 2026,
    month: 11,
    day: 1,
    hour: 9,
    minute: 5,
    second: 7,
};

fn catalog_world() -> (World, FakeCatalog) {
    let mut world = World::new();
    world.class("ArchonCatalog", Some("Object"));
    let live = world.object("ArchonCatalog_0", "ArchonCatalog", 0x80);
    let function_class = world.class_ptr("Function");
    let owner = world.class_ptr("ArchonCatalog");
    let function = world.raw_object("GetAllItems", function_class, owner, 0x80);
    world.register(function);
    let data = world.mem.alloc(sdk::GET_ALL_ITEMS_OUT.stride as usize);
    world.set_text(data, &sdk::CAT_ITEM_ID, "WP_ONE");
    let fake = FakeCatalog {
        catalog: live,
        function,
        data,
        count: 1,
        calls: Cell::new(0),
    };
    (world, fake)
}

#[test]
fn runs_the_enabled_exports_and_writes_the_manifest() {
    let (world, fake) = catalog_world();
    let flags = Flags::parse(b"EXPORT_CATALOG=1\r\nEXPORT_HUNTS=1\r\nEXPORT_COMBAT=1\r\nEXPORT_CURRENCY_NAMES=1\r\n");
    let out = temp_dir("run");
    let dir = out.clone();
    let output = move || -> PathBuf { dir.clone() };
    let sleeps = Cell::new(0);
    let sleep = |_: Duration| sleeps.set(sleeps.get() + 1);
    let now = || NOW;
    let host = Host {
        out: &output,
        debug: no_debug,
        sleep: &sleep,
        now: &now,
    };
    let engine = world.engine(&fake);
    let results = run::run(&engine, &flags, &host).unwrap();
    assert_eq!(sleeps.get(), 0, "the catalog was there at once");
    assert_eq!(
        results,
        Results {
            catalog_general: 1,
            ..Results::default()
        }
    );
    assert_eq!(
        read(&out.join(MANIFEST)),
        "{\r\n  \"gameVersion\": \"1.14.7\",\r\n  \"changelist\": 647472,\r\n  \"engineVersion\": \"4.26.2\",\r\n  \
         \"exportedAt\": \"2026-11-01T09:05:07Z\",\r\n  \"flags\": {\r\n    \"EXPORT_CATALOG\": true,\r\n    \
         \"EXPORT_PROGRESSION\": false,\r\n    \"EXPORT_COMBAT\": true,\r\n    \"EXPORT_SKINS\": false,\r\n    \
         \"EXPORT_SKINS_RESOLVE_STRINGTABLE\": false,\r\n    \"EXPORT_SLAYERS_PATH\": false,\r\n    \
         \"EXPORT_WEAPON_SLOTS\": false,\r\n    \"EXPORT_HUNTS\": true,\r\n    \"EXPORT_TABLE_INVENTORY\": false,\r\n    \
         \"EXPORT_STORE_RUNTIME\": false,\r\n    \"EXPORT_MASTERY\": false\r\n  },\r\n  \"results\": {\r\n    \
         \"catalogGeneralItems\": 1,\r\n    \"catalogEquipmentItems\": 0,\r\n    \"liveWeaponSlotItems\": 0,\r\n    \
         \"progressionRows\": 0,\r\n    \"combatRows\": 0,\r\n    \"skinsRows\": 0,\r\n    \"slayersPathNodes\": 0,\r\n    \
         \"huntRows\": 0,\r\n    \"tableInventoryRows\": 0,\r\n    \"storeRuntimeRows\": 0,\r\n    \"masteryRows\": 0\r\n  \
         }\r\n}\r\n"
    );
    let log = status_text(&out);
    let lines: Vec<&str> = log.split("\r\n").collect();
    assert_eq!(
        lines[0],
        "thread start; flags: catalog=1 progression=0 combat=1 skins=0 skinsResolveStringTable=0 slayersPath=0 \
         weaponSlots=0 hunts=1 tableInventory=0 cells=0 dropTables=0 currencyNames=1 storeRuntime=0"
    );
    assert!(lines[1].starts_with("WARNING: EXPORT_CURRENCY_NAMES=1"));
    assert_eq!(lines[2], "waiting for GObjects + catalog...");
    let order: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| {
            line.contains(", skipping ") || line.contains("not supported") || line.starts_with("catalog DONE")
        })
        .collect();
    assert_eq!(
        order,
        [
            "catalog DONE general=1 equipment=0",
            "EXPORT_WEAPON_SLOTS=0, skipping live weapon-slot export",
            "EXPORT_CELLS=0, skipping live cell-slot export",
            "EXPORT_PROGRESSION=0, skipping progression export",
            "EXPORT_COMBAT=1 is not supported by this build, skipping",
            "EXPORT_SKINS=0, skipping weapon skins export",
            "EXPORT_SLAYERS_PATH=0, skipping Slayer's Path node export",
            "EXPORT_DROP_TABLES=0, skipping drop-table export",
            "EXPORT_CURRENCY_NAMES=1 is not supported by this build, skipping",
            "EXPORT_STORE_RUNTIME=0, skipping store runtime capture",
            "EXPORT_MASTERY=0, skipping mastery/achievement objective export",
            "EXPORT_TABLE_INVENTORY=0, skipping DataTable census",
        ]
    );
    assert!(lines.iter().any(|line| line.starts_with("ALL DONE catalog=1 ")));
    // The refused combat export leaves its folder alone.
    assert!(!out.join("combat_1_14_7").exists());
}

#[test]
fn times_out_when_the_catalog_never_appears() {
    let world = World::new();
    let out = temp_dir("timeout");
    let dir = out.clone();
    let output = move || -> PathBuf { dir.clone() };
    let sleeps = Cell::new(0);
    let sleep = |step: Duration| {
        assert_eq!(step, Duration::from_millis(500));
        sleeps.set(sleeps.get() + 1);
    };
    let now = || NOW;
    let host = Host {
        out: &output,
        debug: no_debug,
        sleep: &sleep,
        now: &now,
    };
    let engine = world.engine(&NoCalls);
    assert!(run::run(&engine, &Flags::defaults(), &host).is_none());
    assert_eq!(sleeps.get(), 239, "240 tries");
    let files: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(files, ["catalog_export_status.txt"]);
    assert!(
        status_text(&out)
            .ends_with("TIMEOUT: catalog never appeared. Reach the main menu/Ramsgate, then re-inject.\r\n")
    );
}

#[test]
fn reads_names_strings_and_containers_through_faults() {
    let mut world = World::new();
    let wide = world.intern_wide("Zoë/Ñame");
    let narrow = world.intern("/Game/Path/Thing.Thing");
    let at = world.mem.alloc(0x80);
    world.mem.write_i32(at, wide);
    world.mem.write_i32(at + 8, narrow);
    world.mem.write_i32(at + 12, 3);
    world.write_fstring(at + 0x10, "text\u{0}after");
    // An FString whose data is unmapped, and an array with a negative count.
    world.mem.write_u64(at + 0x20, 0xDEAD_0000);
    world.mem.write_i32(at + 0x28, 4);
    world.mem.write_i32(at + 0x38, -1);

    let engine = world.engine(&NoCalls);
    assert_eq!(engine.fname(at), "Ñame");
    assert_eq!(engine.fname_raw(at), "Zoë/Ñame");
    assert_eq!(
        engine.fname_raw(at + 8),
        "/Game/Path/Thing.Thing_2",
        "number n > 0 adds _<n-1>"
    );
    assert_eq!(engine.fname(at + 8), "Thing.Thing_2");
    assert_eq!(engine.fstring(at + 0x10), "text", "up to the first NUL");
    engine.take_fault();
    assert_eq!(engine.fstring(at + 0x20), "", "unreadable string data reads empty");
    assert!(!engine.take_fault(), "a string's fault does not skip the row");
    assert_eq!(engine.array(at + 0x30), (0, 0));
    assert!(engine.take_fault(), "a bad count does");
    assert_eq!(engine.ftext(at + 0x40), "", "a null text");
    assert_eq!(engine.fname_raw(0xDEAD_0000), "");
    assert_eq!(engine.name_text(0x7FFF_0000, 0), "", "a block past the pool's");
}

#[test]
fn pointers_at_the_top_of_memory_read_as_faults() {
    let mut world = World::new();
    let at = world.mem.alloc(0x40);
    world.mem.write_u64(at, u64::MAX - 2);
    world.mem.write_i32(at + 8, 3);
    let engine = world.engine(&NoCalls);
    assert_eq!(engine.fstring(at), "");
    assert_eq!(engine.ftext(at), "");
    assert_eq!(engine.fname_raw(u64::MAX - 3), "");
    assert_eq!(
        engine.elements(at, u32::MAX).len(),
        3,
        "addresses wrap instead of overflowing"
    );
    assert_eq!(engine.map_elements(u64::MAX - 0x10, 0x18), (0, Vec::new()));
}
