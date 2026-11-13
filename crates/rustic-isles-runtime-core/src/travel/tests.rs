use super::*;
use crate::memory::BufferMemory;
use crate::offsets::{self, FoundBy};
use std::collections::HashMap;

const BASE: usize = 0x100_0000;
const OBJECTS: usize = BASE;
const TABLE: usize = BASE + 0x100;
const CHUNK0: usize = BASE + 0x1000;
const CHUNK1: usize = CHUNK0 + layout::OBJECTS_PER_CHUNK * layout::OBJECT_ITEM_SIZE;
const OBJ: usize = CHUNK1 + 0x1_0000;
const SIZE: usize = OBJ + 0x2_0000 - BASE;
const OUTSIDE: usize = 0x10;

const PK: u32 = layout::ITEM_PENDING_KILL;
const ASYNC: u32 = layout::ITEM_ASYNC;

/// A synthetic object array: chunk 0 full, chunk 1 holding the first items from index 65 536.
struct Fixture {
    bytes: Vec<u8>,
    next_item: usize,
    next_object: usize,
    names: HashMap<usize, String>,
    world_class: usize,
}

impl Fixture {
    fn new() -> Self {
        let mut fixture = Self {
            bytes: vec![0u8; SIZE],
            next_item: 0,
            next_object: OBJ,
            names: HashMap::new(),
            world_class: 0,
        };
        fixture.put_ptr(OBJECTS + layout::OBJECTS_CHUNKS, TABLE);
        fixture.put_i32(OBJECTS + layout::OBJECTS_CHUNK_COUNT, 2);
        fixture.put_ptr(TABLE, CHUNK0);
        fixture.put_ptr(TABLE + 8, CHUNK1);
        fixture.world_class = fixture.object(0, 0, 0, 0, "World");
        fixture
    }

    fn put(&mut self, at: usize, value: &[u8]) {
        let at = at - BASE;
        self.bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn put_ptr(&mut self, at: usize, value: usize) {
        self.put(at, &(value as u64).to_le_bytes());
    }

    fn put_i32(&mut self, at: usize, value: i32) {
        self.put(at, &value.to_le_bytes());
    }

    fn set_count(&mut self, count: i32) {
        self.put_i32(OBJECTS + layout::OBJECTS_COUNT, count);
    }

    fn item_address(index: usize) -> usize {
        let chunk = if index < layout::OBJECTS_PER_CHUNK {
            CHUNK0
        } else {
            CHUNK1
        };
        chunk + (index % layout::OBJECTS_PER_CHUNK) * layout::OBJECT_ITEM_SIZE
    }

    /// A new object with an item (flags `item_flags`), its class, outer, object flags and short name.
    fn object(&mut self, item_flags: u32, class: usize, outer: usize, object_flags: u32, name: &str) -> usize {
        let at = self.next_object;
        self.next_object += 0x80;
        self.put(at + layout::UOBJECT_FLAGS, &object_flags.to_le_bytes());
        self.put_ptr(at + layout::UOBJECT_CLASS, class);
        self.put_ptr(at + layout::UOBJECT_OUTER, outer);
        self.names.insert(at, name.to_owned());
        let index = self.next_item;
        self.next_item += 1;
        let item = Self::item_address(index);
        self.put_ptr(item, at);
        self.put(item + layout::OBJECT_ITEM_FLAGS, &item_flags.to_le_bytes());
        self.set_count(i32::try_from(self.next_item).unwrap());
        at
    }

    fn class(&mut self, parent: usize, name: &str) -> usize {
        let class = self.object(0, 0, 0, 0, name);
        self.put_ptr(class + layout::USTRUCT_SUPER, parent);
        class
    }

    /// Later objects land in chunk 1.
    fn skip_to_chunk1(&mut self) {
        self.next_item = layout::OBJECTS_PER_CHUNK;
    }

    fn flags_of(&self, object: usize) -> Option<u32> {
        (0..self.next_item).find_map(|index| {
            let item = Self::item_address(index) - BASE;
            let pointer = u64::from_le_bytes(self.bytes[item..item + 8].try_into().unwrap());
            (pointer as usize == object).then(|| {
                let at = item + layout::OBJECT_ITEM_FLAGS;
                u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap())
            })
        })
    }

    fn memory(&self) -> BufferMemory<'_> {
        BufferMemory {
            base: BASE,
            bytes: &self.bytes,
        }
    }

    fn names(&self) -> FakeNames {
        FakeNames {
            names: self.names.clone(),
            world_class: self.world_class,
        }
    }
}

struct FakeNames {
    names: HashMap<usize, String>,
    world_class: usize,
}

impl Names for FakeNames {
    fn short_name(&self, object: usize) -> Option<String> {
        self.names.get(&object).cloned()
    }

    fn full_name(&self, object: usize) -> Option<String> {
        if object == self.world_class {
            Some(WORLD_CLASS.to_owned())
        } else {
            self.names.get(&object).map(|name| format!("Class Other.{name}"))
        }
    }
}

#[test]
fn leftovers() {
    let mut f = Fixture::new();
    let world = f.world_class;
    let sub = f.class(world, "SubWorld");
    let other = f.class(0, "Actor");
    let mut deep = world;
    let mut chain = Vec::new();
    for depth in 1..=17 {
        deep = f.class(deep, &format!("Deep{depth}"));
        chain.push(deep);
    }
    let leftover = f.object(PK, world, 0, 0, "ramsgate_01_persistent");
    let current = f.object(PK, world, 0, 0, "current");
    let live = f.object(0, world, 0, 0, "live");
    let default = f.object(PK, world, 0, layout::RF_CLASS_DEFAULT_OBJECT, "Default__World");
    let actor = f.object(PK, other, 0, 0, "actor");
    let deep16 = f.object(PK, chain[15], 0, 0, "deep16");
    let deep17 = f.object(PK, chain[16], 0, 0, "deep17");
    f.skip_to_chunk1();
    let subworld = f.object(PK, sub, 0, 0, "training_grounds");
    let memory = f.memory();
    assert_eq!(find_world_class(&memory, OBJECTS, &f.names()), Some(world));
    let found = find_leftovers(&memory, OBJECTS, world, current);
    assert_eq!(found, [leftover, deep16, subworld]);
    for not in [current, live, default, actor, deep17] {
        assert!(!found.contains(&not));
    }
}

#[test]
fn leftovers_stop_at_64() {
    let mut f = Fixture::new();
    let world = f.world_class;
    let worlds: Vec<usize> = (0..70).map(|n| f.object(PK, world, 0, 0, &format!("w{n}"))).collect();
    let memory = f.memory();
    let found = find_leftovers(&memory, OBJECTS, world, 0);
    assert_eq!(found.len(), MAX_LEFTOVERS);
    assert_eq!(found, worlds[..64]);
}

#[test]
fn eligibility_bits() {
    assert!(is_eligible(PK | ASYNC));
    assert!(is_eligible(PK | ASYNC | 0x0000_00FF | layout::RF_CLASS_DEFAULT_OBJECT));
    assert!(!is_eligible(ASYNC), "pending-kill needed");
    assert!(!is_eligible(PK), "Async needed");
    assert!(!is_eligible(PK | ASYNC | layout::ITEM_ROOT_SET), "rooted");
    assert!(!is_eligible(PK | ASYNC | layout::ITEM_ASYNC_LOADING), "still loading");
    assert!(is_pending_kill(PK));
    assert!(!is_pending_kill(ASYNC | layout::ITEM_ROOT_SET));
}

#[test]
fn eligible_only_inside_leftovers() {
    let mut f = Fixture::new();
    let world = f.world_class;
    let leftover = f.object(PK, world, 0, 0, "left");
    let kept = f.object(0, world, 0, 0, "kept");
    let mut outer = leftover;
    let mut chain = Vec::new();
    for depth in 1..=17 {
        outer = f.object(PK | ASYNC, 0, outer, 0, &format!("o{depth}"));
        chain.push(outer);
    }
    let in_kept = f.object(PK | ASYNC, 0, kept, 0, "in_kept");
    let rooted = f.object(PK | ASYNC | layout::ITEM_ROOT_SET, 0, leftover, 0, "rooted");
    let memory = f.memory();
    let leftovers = [leftover];
    assert_eq!(world_holding(&memory, leftover, &leftovers), Some(leftover));
    assert_eq!(
        world_holding(&memory, chain[15], &leftovers),
        Some(leftover),
        "16 outers deep"
    );
    assert_eq!(world_holding(&memory, chain[16], &leftovers), None, "17 outers deep");
    let eligible: Vec<usize> = find_eligible(&memory, OBJECTS, &leftovers)
        .iter()
        .map(|found| found.item.object)
        .collect();
    assert_eq!(eligible, chain[..16]);
    assert!(!eligible.contains(&in_kept));
    assert!(!eligible.contains(&rooted));
    assert!(find_eligible(&memory, OBJECTS, &[]).is_empty());
}

fn count_items(memory: &dyn Memory) -> usize {
    let mut seen = 0;
    walk(memory, OBJECTS, &mut |item| {
        if item.object != 0 {
            seen += 1;
        }
        ControlFlow::Continue(())
    });
    seen
}

#[test]
fn walk_skips_bad_chunks() {
    let mut f = Fixture::new();
    f.object(0, 0, 0, 0, "a");
    f.skip_to_chunk1();
    f.object(0, 0, 0, 0, "b");
    assert_eq!(count_items(&f.memory()), 3, "both chunks, the world class included");
    f.put_ptr(TABLE, OUTSIDE);
    assert_eq!(count_items(&f.memory()), 1, "an unreadable chunk 0 skipped");
    f.put_ptr(TABLE, 0);
    assert_eq!(count_items(&f.memory()), 1, "a null chunk skipped");
    f.put_ptr(TABLE, CHUNK0);
    f.put_i32(OBJECTS + layout::OBJECTS_CHUNK_COUNT, 1);
    assert_eq!(count_items(&f.memory()), 2, "chunks past the chunk count unread");
    f.put_i32(OBJECTS + layout::OBJECTS_CHUNK_COUNT, 2);
    f.set_count(4_000_001);
    assert!(!walk(&f.memory(), OBJECTS, &mut |_| ControlFlow::Continue(())));
    assert_eq!(count_items(&f.memory()), 0);
    f.set_count(-1);
    assert_eq!(count_items(&f.memory()), 0);
}

/// A leftover Ramsgate holding the input component the crash needs: component → controller → level → world.
fn ramsgate() -> (Fixture, usize, usize) {
    let mut f = Fixture::new();
    let world = f.world_class;
    let input = f.class(0, "default_input_bpc_C");
    let leftover = f.object(PK, world, 0, 0, "ramsgate_01_persistent");
    let level = f.object(PK, 0, leftover, 0, "PersistentLevel");
    let controller = f.object(PK, 0, level, 0, "BP_PlayerController_C_0");
    let component = f.object(PK | ASYNC, input, controller, 0, "default_input_bpc_C_0");
    (f, leftover, component)
}

#[test]
fn pass_releases_and_rings() {
    let (f, leftover, component) = ramsgate();
    let memory = f.memory();
    let names = f.names();
    let mut travel = Travel::new();
    let mut reads = 0;
    let outcome = travel.pass(&memory, OBJECTS, &names, 0, Reason::LoadMapEnter, &mut || {
        reads += 1;
        false
    });
    assert_eq!(outcome.leftovers, [leftover]);
    assert_eq!(outcome.clear.len(), 1);
    assert_eq!(f.flags_of(component), memory.u32_at(outcome.clear[0]));
    let flags = memory.u32_at(outcome.clear[0]).unwrap();
    assert_eq!(flags, PK | ASYNC);
    assert_eq!(
        outcome.lines,
        [
            "[Travel] leftover-world release on".to_owned(),
            "[Travel] loadmap-enter: released the stuck Async keep flag on 1 object(s) in 1 leftover world(s): \
             default_input_bpc_C_0(default_input_bpc_C) in ramsgate_01_persistent"
                .to_owned(),
        ]
    );
    assert!(travel.released().contains(leftover));
    // The mode is read and said once
    let again = travel.pass(&memory, OBJECTS, &names, 0, Reason::Arrival10s, &mut || {
        reads += 1;
        false
    });
    assert_eq!(again.lines.len(), 1);
    assert!(again.lines[0].starts_with("[Travel] arrival+10s: released"));
    // Released before and still there: one survivor warning, on world-change only
    let warned = travel.pass(&memory, OBJECTS, &names, 0, Reason::WorldChange, &mut || true);
    assert_eq!(
        warned.lines[0],
        format!(
            "[Travel] leftover world {leftover:#x} ramsgate_01_persistent survived the purge after its keep flag was \
             released: something else holds it"
        )
    );
    let twice = travel.pass(&memory, OBJECTS, &names, 0, Reason::WorldChange, &mut || true);
    assert!(twice.lines.iter().all(|line| !line.contains("survived")));
    assert_eq!(reads, 1);
}

#[test]
fn survivor_warning_only_on_world_change() {
    let (f, leftover, _) = ramsgate();
    let memory = f.memory();
    let names = f.names();
    let mut travel = Travel::new();
    travel.pass(&memory, OBJECTS, &names, 0, Reason::LoadMapEnter, &mut || false);
    for reason in [Reason::LoadMapEnter, Reason::Arrival10s, Reason::Arrival60s] {
        let outcome = travel.pass(&memory, OBJECTS, &names, 0, reason, &mut || false);
        assert!(
            outcome.lines.iter().all(|line| !line.contains("survived")),
            "{reason:?}"
        );
    }
    let outcome = travel.pass(&memory, OBJECTS, &names, 0, Reason::WorldChange, &mut || false);
    assert!(outcome.lines[0].contains(&format!("{leftover:#x}")));
}

#[test]
fn report_only_clears_nothing() {
    let (f, leftover, _) = ramsgate();
    let memory = f.memory();
    let names = f.names();
    let mut travel = Travel::new();
    let outcome = travel.pass(&memory, OBJECTS, &names, 0, Reason::WorldChange, &mut || true);
    assert_eq!(outcome.leftovers, [leftover]);
    assert!(outcome.clear.is_empty());
    assert_eq!(
        outcome.lines,
        [
            "[Travel] leftover-world release off (KEEP_STUCK_ASYNC_FLAGS.flag; report only)".to_owned(),
            "[Travel] world-change: found (not released) the stuck Async keep flag on 1 object(s) in 1 leftover \
             world(s): default_input_bpc_C_0(default_input_bpc_C) in ramsgate_01_persistent"
                .to_owned(),
        ]
    );
    assert!(!travel.released().contains(leftover));
}

#[test]
fn pass_without_leftovers_says_nothing() {
    let (f, leftover, _) = ramsgate();
    let memory = f.memory();
    let mut travel = Travel::new();
    let outcome = travel.pass(&memory, OBJECTS, &f.names(), leftover, Reason::WorldChange, &mut || {
        panic!("the flag is read only once a leftover is found")
    });
    assert_eq!(outcome, Outcome::default());
    let no_class = FakeNames {
        names: HashMap::new(),
        world_class: 0,
    };
    let outcome = Travel::new().pass(&memory, OBJECTS, &no_class, 0, Reason::WorldChange, &mut || false);
    assert_eq!(outcome, Outcome::default(), "no world class: nothing this time");
}

#[test]
fn rings() {
    let mut ring = Ring::new();
    for world in 1..=16 {
        assert!(ring.insert_once(world * 0x100));
    }
    assert!(!ring.insert_once(0x100), "held once");
    assert!(ring.insert_once(0x1700));
    assert!(!ring.contains(0x100), "the 17th overwrote the first");
    assert!(ring.contains(0x200) && ring.contains(0x1700));
    assert!(!ring.insert_once(0));
    assert!(!ring.contains(0));
}

#[test]
fn watcher() {
    let mut watcher = Watcher::new();
    let mut world = 0usize;
    let mut reads = 0;
    let mut poll = |watcher: &mut Watcher, now: u64, world: usize| {
        watcher.poll(now, &mut || {
            reads += 1;
            world
        })
    };
    assert_eq!(
        poll(&mut watcher, 1_000, world),
        None,
        "first call reads; null is no change"
    );
    assert_eq!(poll(&mut watcher, 1_249, world), None);
    world = 0xA000;
    assert_eq!(poll(&mut watcher, 1_100, world), None, "inside the 250 ms slot");
    assert_eq!(
        poll(&mut watcher, 1_250, world),
        Some(Reason::WorldChange),
        "null to a world"
    );
    assert_eq!(poll(&mut watcher, 11_000, world), None);
    assert_eq!(poll(&mut watcher, 11_250, world), Some(Reason::Arrival10s));
    assert_eq!(poll(&mut watcher, 11_500, world), None, "once");
    assert_eq!(poll(&mut watcher, 61_250, world), Some(Reason::Arrival60s));
    assert_eq!(poll(&mut watcher, 200_000, world), None, "then idle");
    world = 0;
    assert_eq!(
        poll(&mut watcher, 200_250, world),
        Some(Reason::WorldChange),
        "a world to null"
    );
    world = 0xB000;
    assert_eq!(poll(&mut watcher, 200_500, world), Some(Reason::WorldChange));
    assert_eq!(poll(&mut watcher, 210_500, world), Some(Reason::Arrival10s));
    world = 0xC000;
    assert_eq!(
        poll(&mut watcher, 211_000, world),
        Some(Reason::WorldChange),
        "a change during +60 s pending"
    );
    assert_eq!(poll(&mut watcher, 220_750, world), None);
    assert_eq!(
        poll(&mut watcher, 221_000, world),
        Some(Reason::Arrival10s),
        "restarted at +10 s"
    );
    drop(poll);
    assert_eq!(reads, 13, "GWorld read only on the calls the slot let through");
}

#[test]
fn watcher_first_call_runs() {
    let mut watcher = Watcher::new();
    assert_eq!(watcher.poll(0, &mut || 0x1000), Some(Reason::WorldChange));
}

fn fstring(units: &[u16], num: i32) -> Option<String> {
    let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let memory = BufferMemory {
        base: BASE,
        bytes: &bytes,
    };
    fstring_text(&memory, BASE, num)
}

fn path(text: &str) -> Option<String> {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    let num = i32::try_from(units.len()).unwrap();
    fstring(&units, num)
}

#[test]
fn map_short_names() {
    let one = path("/Game/Maps/ramsgate/ramsgate_01_persistent").unwrap();
    assert_eq!(map_short_name(&one), "ramsgate_01_persistent");
    let two = path("/Game/Maps/ramsgate/ramsgate_01_persistent.ramsgate_01_persistent").unwrap();
    assert_eq!(map_short_name(&two), "ramsgate_01_persistent");
    assert_eq!(map_short_name("plain.map"), "plain");
    assert_eq!(map_short_name("/Game/Maps/nodot"), "nodot");
    assert_eq!(map_short_name(""), "");
    assert_eq!(path(""), None, "empty");
    assert_eq!(fstring(&[0x41, 0], 0), None, "num 0");
    assert_eq!(fstring(&[0], 1), None, "num 1");
    let long = vec![0x41u16; 1025];
    assert_eq!(fstring(&long, 1025), None, "num 1025");
    assert_eq!(fstring(&long, 1024).map(|text| text.len()), Some(1024));
    assert_eq!(path("/Game/Maps/caf\u{e9}").as_deref(), Some("/Game/Maps/caf?"));
    assert_eq!(fstring(&[0x41, 0x42], 3), None, "unreadable");
}

#[test]
fn map_path_from_url() {
    let mut bytes = vec![0u8; 0x100];
    let text: Vec<u8> = "/Game/a\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
    bytes[0x80..0x80 + text.len()].copy_from_slice(&text);
    bytes[layout::FURL_MAP..layout::FURL_MAP + 8].copy_from_slice(&((BASE + 0x80) as u64).to_le_bytes());
    bytes[layout::FURL_MAP + 8..layout::FURL_MAP + 12].copy_from_slice(&8i32.to_le_bytes());
    let memory = BufferMemory {
        base: BASE,
        bytes: &bytes,
    };
    assert_eq!(read_map_path(&memory, BASE).as_deref(), Some("/Game/a"));
    assert_eq!(read_map_path(&memory, BASE + 0xD0), None, "FURL not readable");
    assert_eq!(context_world(&memory, BASE), 0, "context not readable");
}

#[test]
fn guard() {
    let (f, leftover, _) = ramsgate();
    let names = f.names();
    let ramsgate = "/Game/Maps/ramsgate/ramsgate_01_persistent".to_owned();
    let decide = |travel: &mut Travel, path: &str, context: usize| {
        let path = path.to_owned();
        travel.guard(&mut || true, &mut || Some(path.clone()), &[leftover], context, &names)
    };
    let mut travel = Travel::new();
    let first = decide(&mut travel, &ramsgate, 0);
    assert!(first.refused);
    assert_eq!(
        first.lines,
        [
            "[Travel] leftover-world guard on (ENABLE_ZOMBIE_LOADMAP_GUARD.flag)".to_owned(),
            format!(
                "[Travel] REFUSED map={ramsgate}: its previous world {leftover:#x} is a pending-kill leftover; failing \
                 this travel instead of reusing it (ENABLE_ZOMBIE_LOADMAP_GUARD.flag)"
            ),
        ]
    );
    assert_eq!(
        decide(&mut travel, &ramsgate, 0).lines.len(),
        1,
        "the mode is said once"
    );
    assert!(
        !decide(&mut travel, &ramsgate, leftover).refused,
        "the context's own world"
    );
    assert!(
        !decide(&mut travel, "/Game/Maps/hunt/hunt_01", 0).refused,
        "a different name"
    );
    assert!(!decide(&mut travel, "", 0).refused, "an empty path");
    travel.released.insert_once(leftover);
    assert!(
        !decide(&mut travel, &ramsgate, 0).refused,
        "released: left to LoadMap's purge"
    );

    let mut off = Travel::new();
    let mut path_reads = 0;
    let guard = off.guard(
        &mut || false,
        &mut || {
            path_reads += 1;
            Some(ramsgate.clone())
        },
        &[leftover],
        0,
        &names,
    );
    assert!(!guard.refused);
    assert_eq!(guard.lines, ["[Travel] leftover-world guard off".to_owned()]);
    assert_eq!(path_reads, 0);
}

#[test]
fn release_line_lists_eight() {
    let listed: Vec<Listed> = (0..10)
        .map(|n| Listed {
            object: format!("o{n}"),
            class: "C".into(),
            world: "w".into(),
        })
        .collect();
    let line = release_line(Reason::Arrival60s, false, 10, 2, &listed);
    assert!(line.starts_with(
        "[Travel] arrival+60s: released the stuck Async keep flag on 10 object(s) in 2 leftover world(s): o0(C) in w, "
    ));
    assert!(line.ends_with("o7(C) in w"));
    assert_eq!(line.matches(" in w").count(), 8);
}

#[test]
fn flag_names_and_offsets() {
    assert_eq!(flag_names::KEEP_STUCK_ASYNC_FLAGS, "KEEP_STUCK_ASYNC_FLAGS");
    assert_eq!(flag_names::ENABLE_ZOMBIE_LOADMAP_GUARD, "ENABLE_ZOMBIE_LOADMAP_GUARD");
    assert_eq!(flag_names::NO_LOADMAP_PROBE, "NO_LOADMAP_PROBE");
    let lines: Vec<_> = offsets::TARGETS.iter().filter(|target| target.id == "TR-1").collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].rva, 0x0427_1370);
    assert!(matches!(lines[0].found_by, FoundBy::RvaAndBytes(prologue) if prologue.len() == 31));
    assert_eq!(ELIGIBLE_MASK, 0x6C00_0000);
    assert_eq!(ELIGIBLE, 0x2400_0000);
}
