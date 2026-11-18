use super::*;
use crate::memory::BufferMemory;
use std::cell::RefCell;

const BASE: usize = 0x10_0000;
const TABLE: usize = BASE + 0x100;
const CHUNK0: usize = BASE + 0x1000;
const CHUNK1: usize = CHUNK0 + layout::OBJECTS_PER_CHUNK * layout::OBJECT_ITEM_SIZE;
const HEAP: usize = CHUNK1 + 0x1000;
const BAD: usize = 0x7FFF_0000_0000;

struct Scene(Vec<u8>);

impl Scene {
    fn new() -> Self {
        Self(vec![0; HEAP - BASE + 0x10_000])
    }
    fn put(&mut self, address: usize, value: u64) {
        let at = address - BASE;
        self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn put_i32(&mut self, address: usize, value: i32) {
        let at = address - BASE;
        self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn item(&mut self, chunk: usize, index: usize, object: usize) {
        self.put(chunk + index * layout::OBJECT_ITEM_SIZE, object as u64);
    }
    fn object(&mut self, object: usize, class: usize, outer: usize) {
        self.put(object + layout::UOBJECT_CLASS, class as u64);
        self.put(object + layout::UOBJECT_OUTER, outer as u64);
    }
}

struct Counting<'a> {
    inner: BufferMemory<'a>,
    reads: RefCell<Vec<usize>>,
}

impl Memory for Counting<'_> {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        self.inner.readable(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(out.len());
        self.inner.read(addr, out)
    }
}

fn heap(slot: usize) -> usize {
    HEAP + slot * 0x40
}

#[test]
fn walk_two_chunks() {
    let mut scene = Scene::new();
    let count = layout::OBJECTS_PER_CHUNK + 3;
    scene.put(BASE + layout::OBJECTS_CHUNKS, TABLE as u64);
    scene.put_i32(BASE + layout::OBJECTS_COUNT, i32::try_from(count).unwrap());
    scene.put_i32(BASE + layout::OBJECTS_CHUNK_COUNT, 2);
    scene.put(TABLE, CHUNK0 as u64);
    scene.put(TABLE + 8, CHUNK1 as u64);
    let (class_pkg, class_one, class_two) = (heap(100), heap(101), heap(102));
    let (package, x, y, z, cycle1, cycle2) = (heap(0), heap(1), heap(2), heap(3), heap(4), heap(5));
    scene.object(package, class_pkg, 0);
    scene.object(x, class_one, package);
    scene.object(y, class_one, x);
    scene.object(z, 0, package);
    scene.object(cycle1, class_two, cycle2);
    scene.object(cycle2, class_two, cycle1);
    for (index, object) in [package, x, y, BAD, z].into_iter().enumerate() {
        scene.item(CHUNK0, index, object);
    }
    scene.item(CHUNK0, 10, 0);
    scene.item(CHUNK1, 0, cycle1);
    scene.item(CHUNK1, 1, cycle2);
    let memory = Counting {
        inner: BufferMemory {
            base: BASE,
            bytes: &scene.0,
        },
        reads: RefCell::new(Vec::new()),
    };
    let walk = walk(&memory, BASE).expect("walk");
    assert_eq!(walk.objects, 5);
    assert_eq!(walk.unreadable, 2, "the unmapped header and the null class");
    assert_eq!(
        walk.classes,
        HashMap::from([(class_pkg, 1), (class_one, 2), (class_two, 2)])
    );
    assert_eq!(
        walk.outermost,
        HashMap::from([(package, 3), (cycle1, 2)]),
        "a cycle ends at its last object"
    );
    let chunk_reads = memory
        .reads
        .borrow()
        .iter()
        .filter(|&&len| len > layout::UOBJECT_SIZE)
        .count();
    assert_eq!(chunk_reads, 2, "one read per chunk");
}

#[test]
fn walk_refuses_a_bad_array() {
    let mut scene = Scene::new();
    scene.put_i32(BASE + layout::OBJECTS_COUNT, -1);
    let memory = BufferMemory {
        base: BASE,
        bytes: &scene.0,
    };
    assert_eq!(walk(&memory, BASE), None);
    assert_eq!(walk(&memory, BAD), None);
}

#[test]
fn walk_counts_a_missing_chunk() {
    let mut scene = Scene::new();
    scene.put(BASE + layout::OBJECTS_CHUNKS, TABLE as u64);
    scene.put_i32(BASE + layout::OBJECTS_COUNT, 4);
    scene.put_i32(BASE + layout::OBJECTS_CHUNK_COUNT, 1);
    let memory = BufferMemory {
        base: BASE,
        bytes: &scene.0,
    };
    let walk = walk(&memory, BASE).expect("walk");
    assert_eq!((walk.objects, walk.unreadable), (0, 4), "a null chunk pointer");
}

#[test]
fn outer_chain_cap() {
    let mut scene = Scene::new();
    for link in 0..70 {
        scene.object(heap(link), heap(200), heap(link + 1));
    }
    let memory = BufferMemory {
        base: BASE,
        bytes: &scene.0,
    };
    let mut memo = HashMap::new();
    assert_eq!(outermost(&memory, heap(0), &mut memo), heap(MAX_OUTERS - 1));
    assert_eq!(memo.len(), MAX_OUTERS);
    assert_eq!(outermost(&memory, heap(5), &mut memo), heap(MAX_OUTERS - 1), "memoized");
}

#[test]
fn schedule() {
    assert!(!due(29));
    assert!(due(30));
    assert!(!due(31));
    assert!(due(180));
    assert!(due(330));
    assert!(!due(150));
}

#[test]
fn size_checks() {
    assert_eq!(size_check(Some(("Object", Some(40)))), Ok(()));
    assert_eq!(size_check(None), Err("no class".to_owned()));
    assert_eq!(
        size_check(Some(("Object", Some(48)))),
        Err("root Object has 48 bytes, expected Object with 40".to_owned())
    );
    assert_eq!(
        size_check(Some(("Field", None))),
        Err("root Field has ? bytes, expected Object with 40".to_owned())
    );
    assert_eq!(class_size(Some(40)), Some(40));
    assert_eq!(class_size(Some(39)), None);
    assert_eq!(class_size(Some(-1)), None);
    assert_eq!(class_size(Some(16 * 1024 * 1024 + 1)), None);
    assert_eq!(class_size(None), None);
}

#[test]
fn folders_and_sizes() {
    assert_eq!(folder_of("/Game/Characters/Hero/Mesh"), "/Game/Characters");
    assert_eq!(folder_of("/Script/Engine"), "/Script/Engine");
    assert_eq!(folder_of("/Game"), "/Game");
    assert_eq!(folder_of("Transient"), "Transient");
    assert_eq!(size_text(2048), "2 KiB");
    assert_eq!(size_text(1_048_575), "1023 KiB");
    assert_eq!(size_text(3 * 1_048_576 / 2), "1.5 MiB");
}

fn row(name: &str, count: u64, size: Option<u32>, actor: bool) -> ClassRow {
    ClassRow {
        name: name.to_owned(),
        count,
        size,
        actor,
    }
}

fn report(classes: Vec<ClassRow>, sizes: Result<(), String>) -> Report {
    Report {
        classes,
        packages: vec![
            ("/Game/Maps/Ramsgate".to_owned(), 30),
            ("/Game/Maps/Ramsgate_Audio".to_owned(), 5),
            ("/Script/Engine".to_owned(), 40),
        ],
        objects: 75,
        unreadable: 1,
        sizes,
        ms: 12,
        connections: 2,
        stale: false,
        role: HubRole::Hub,
    }
}

#[test]
fn first_census_lines() {
    let classes = vec![
        row("Texture2D", 10, Some(400), false),
        row("PlayerPawn_C", 2, Some(4000), true),
        row("Function", 60, Some(176), false),
        row("Function", 3, Some(176), false),
    ];
    let (lines, next) = lines(&report(classes, Ok(())), None);
    assert_eq!(
        lines,
        [
            "[MitRuntime] [Census] 75 objects in 4 classes (1 unreadable), shallow 22 KiB, actors 2, connections 2, \
             role hub, took 12 ms",
            "[MitRuntime] [Census] class 1/3 Function: 63, 10 KiB shallow, 176 B each",
            "[MitRuntime] [Census] class 2/3 PlayerPawn_C: 2, 7 KiB shallow, 4000 B each, actor",
            "[MitRuntime] [Census] class 3/3 Texture2D: 10, 3 KiB shallow, 400 B each",
            "[MitRuntime] [Census] actor 1/1 PlayerPawn_C: 2",
            "[MitRuntime] [Census] folder 1/2 /Script/Engine: 40 objects",
            "[MitRuntime] [Census] folder 2/2 /Game/Maps: 35 objects",
            "[MitRuntime] [Census] package 1/3 /Script/Engine: 40 objects",
            "[MitRuntime] [Census] package 2/3 /Game/Maps/Ramsgate: 30 objects",
            "[MitRuntime] [Census] package 3/3 /Game/Maps/Ramsgate_Audio: 5 objects",
        ]
    );
    assert_eq!(next.classes.get("Function"), Some(&63));
    assert_eq!(next.folders.get("/Game/Maps"), Some(&35));
}

#[test]
fn deltas_and_sizes_off() {
    let previous = Previous {
        classes: HashMap::from([("A".to_owned(), 5), ("B".to_owned(), 5), ("C".to_owned(), 1)]),
        folders: HashMap::from([("/Game/Maps".to_owned(), 35)]),
    };
    let classes = vec![
        row("A", 7, None, true),
        row("B", 3, None, false),
        row("C", 1, None, false),
        row("D", 3, None, false),
    ];
    let mut report = report(classes, Err("no class".to_owned()));
    report.stale = true;
    let (lines, _) = lines(&report, Some(&previous));
    assert!(
        lines[0].ends_with("shallow 0 KiB, actors 7, connections 2 (stale), role hub, took 12 ms, sizes off: no class")
    );
    assert_eq!(
        lines[1],
        "[MitRuntime] [Census] class 1/4 A: 7 (+2), ? shallow, ? B each, actor"
    );
    assert_eq!(
        lines[2],
        "[MitRuntime] [Census] class 2/4 B: 3 (-2), ? shallow, ? B each"
    );
    assert_eq!(
        lines[3], "[MitRuntime] [Census] class 3/4 D: 3 (+3), ? shallow, ? B each",
        "ties by name"
    );
    assert_eq!(
        lines[4],
        "[MitRuntime] [Census] class 4/4 C: 1 (=), ? shallow, ? B each"
    );
    assert!(lines.contains(&"[MitRuntime] [Census] folder 2/2 /Game/Maps: 35 objects (=)".to_owned()));
    assert!(lines.contains(&"[MitRuntime] [Census] folder 1/2 /Script/Engine: 40 objects (+40)".to_owned()));
}

#[test]
fn caps_and_disagreeing_sizes() {
    let classes: Vec<ClassRow> = (0..40).map(|i| row(&format!("K{i:02}"), 1, Some(40), true)).collect();
    let (lines, _) = lines(&report(classes, Ok(())), None);
    assert_eq!(
        lines.iter().filter(|line| line.contains("] class ")).count(),
        TOP_CLASSES
    );
    assert_eq!(
        lines.iter().filter(|line| line.contains("] actor ")).count(),
        TOP_ACTORS
    );
    assert!(lines[1].starts_with("[MitRuntime] [Census] class 1/25 K00: 1"));
    let (lines, _) = super::lines(
        &report(
            vec![row("Same", 1, Some(40), false), row("Same", 1, Some(80), false)],
            Ok(()),
        ),
        None,
    );
    assert_eq!(
        lines[1],
        "[MitRuntime] [Census] class 1/1 Same: 2, 0 KiB shallow, ? B each"
    );
}
