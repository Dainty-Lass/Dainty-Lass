//! R12's core tests (16.1) over a fake game: blocks of bytes at made-up addresses.

use super::*;
use crate::net::Admit;
use crate::offsets::{r9, repgraph as at};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Default)]
struct Fake {
    blocks: RefCell<BTreeMap<usize, Vec<u8>>>,
    read_only: RefCell<HashSet<usize>>,
    live: RefCell<HashSet<usize>>,
    classes: RefCell<HashMap<&'static str, usize>>,
    names: RefCell<HashMap<usize, String>>,
    world: Cell<usize>,
    now: Cell<u64>,
    image: Cell<(usize, usize)>,
    entries: RefCell<HashSet<u32>>,
    lines: RefCell<Vec<String>>,
    class_reads: Cell<u32>,
}

const CLASS_PC: usize = 0x1_0000;
const CLASS_ARCHON_PC: usize = 0x1_1000;
const CLASS_CONN: usize = 0x1_2000;
const CLASS_CHAN: usize = 0x1_3000;
const CLASS_OTHER: usize = 0x1_4000;
const CLASS_ROOT: usize = 0x1_5000;

impl Fake {
    fn block_of(&self, addr: usize, size: usize) -> Option<(usize, usize)> {
        let blocks = self.blocks.borrow();
        let (&base, bytes) = blocks.range(..=addr).next_back()?;
        let end = addr.checked_add(size)?;
        (end <= base + bytes.len()).then_some((base, addr - base))
    }

    fn alloc(&self, base: usize, size: usize) {
        self.blocks.borrow_mut().insert(base, vec![0; size]);
    }

    fn put(&self, addr: usize, value: usize) {
        let (base, offset) = self.block_of(addr, 8).expect("inside a block");
        self.blocks.borrow_mut().get_mut(&base).expect("block")[offset..offset + 8]
            .copy_from_slice(&(value as u64).to_le_bytes());
    }

    fn put_i32(&self, addr: usize, value: i32) {
        let (base, offset) = self.block_of(addr, 4).expect("inside a block");
        self.blocks.borrow_mut().get_mut(&base).expect("block")[offset..offset + 4]
            .copy_from_slice(&value.to_le_bytes());
    }

    fn get(&self, addr: usize) -> usize {
        self.ptr(addr)
    }

    fn add_class(&self, class: usize, sup: usize, name: &'static str) {
        self.alloc(class, 0x50);
        self.put(class + at::STRUCT_SUPER, sup);
        self.classes.borrow_mut().insert(name, class);
    }

    fn object(&self, base: usize, size: usize, class: usize) -> usize {
        self.alloc(base, size);
        self.put(base + 0x10, class);
        self.live.borrow_mut().insert(base);
        base
    }

    fn new() -> Self {
        let fake = Self::default();
        fake.add_class(CLASS_ROOT, 0, "Object");
        fake.add_class(CLASS_PC, CLASS_ROOT, "PlayerController");
        fake.add_class(CLASS_ARCHON_PC, CLASS_PC, "ArchonPlayerController");
        fake.add_class(CLASS_CONN, CLASS_ROOT, "NetConnection");
        fake.add_class(CLASS_CHAN, CLASS_ROOT, "ActorChannel");
        fake.add_class(CLASS_OTHER, CLASS_ROOT, "Pawn");
        fake.world.set(0x9000);
        fake
    }

    fn pc(&self, base: usize, conn: usize) -> usize {
        self.object(base, 0x420, CLASS_ARCHON_PC);
        self.put(base + at::PLAYER_CONTROLLER_NET_CONNECTION, conn);
        base
    }

    fn conn(&self, base: usize, pc: usize, owner: usize) -> usize {
        self.object(base, 0xA0, CLASS_CONN);
        self.put(base + at::CONNECTION_PLAYER_CONTROLLER, pc);
        self.put(base + at::CONNECTION_OWNING_ACTOR, owner);
        base
    }

    fn chan(&self, base: usize, conn: usize, actor: usize) -> usize {
        self.object(base, 0x78, CLASS_CHAN);
        self.put(base + at::CHANNEL_CONNECTION, conn);
        self.put(base + r9::CHANNEL_ACTOR, actor);
        base
    }

    fn take_lines(&self) -> Vec<String> {
        std::mem::take(&mut *self.lines.borrow_mut())
    }
}

impl Game for Fake {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        if (CLASS_PC..=CLASS_ROOT).contains(&ptr) {
            self.class_reads.set(self.class_reads.get() + 1);
        }
        ptr != 0 && ptr.is_multiple_of(8) && self.block_of(ptr, size).is_some()
    }
    fn readable_this_frame(&self, ptr: usize, size: usize) -> bool {
        self.readable(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        let Some((base, offset)) = self.block_of(addr, out.len()) else {
            return false;
        };
        out.copy_from_slice(&self.blocks.borrow()[&base][offset..offset + out.len()]);
        true
    }
    fn write_ptr(&self, addr: usize, value: usize) -> bool {
        if self.read_only.borrow().contains(&addr) || !self.readable(addr, 8) {
            return false;
        }
        self.put(addr, value);
        true
    }
    fn is_live_object(&self, ptr: usize) -> bool {
        self.live.borrow().contains(&ptr)
    }
    fn class(&self, class: Class) -> Option<usize> {
        self.classes.borrow().get(class.name()).copied()
    }
    fn world(&self) -> usize {
        self.world.get()
    }
    fn now_ms(&self) -> u64 {
        self.now.get()
    }
    fn name(&self, object: usize) -> Option<String> {
        self.names.borrow().get(&object).cloned()
    }
    fn class_name(&self, object: usize) -> Option<String> {
        self.name(self.ptr(object + 0x10))
    }
    fn image_rva(&self, address: usize) -> Option<u32> {
        let (base, size) = self.image.get();
        (address >= base && address < base + size).then(|| (address - base) as u32)
    }
    fn function_entry(&self, rva: u32) -> Option<u32> {
        self.entries.borrow().contains(&rva).then_some(rva)
    }
    fn log(&self, line: &str) {
        self.lines.borrow_mut().push(line.to_owned());
    }
}

/// Two players: controllers A and B on connections CA and CB.
struct Hub {
    fake: Fake,
    a: usize,
    b: usize,
    ca: usize,
    cb: usize,
}

const A: usize = 0x10_0000;
const B: usize = 0x11_0000;
const CA: usize = 0x20_0000;
const CB: usize = 0x21_0000;

fn hub() -> Hub {
    let fake = Fake::new();
    fake.pc(A, CA);
    fake.pc(B, CB);
    fake.conn(CA, A, A);
    fake.conn(CB, B, B);
    Hub {
        fake,
        a: A,
        b: B,
        ca: CA,
        cb: CB,
    }
}

#[test]
fn remote_rows() {
    let (actor, own, other) = (0xA0, 0xC0, 0xD0);
    assert_eq!(remote(actor, own, true, own, true, 0, 0), None, "own connection");
    assert_eq!(
        remote(actor, own, true, other, true, 0, 0),
        Some("actor-netconnection-mismatch")
    );
    assert_eq!(
        remote(actor, 0, false, other, true, actor, 0),
        None,
        "target's controller"
    );
    assert_eq!(remote(actor, 0, false, other, true, 0, actor), None, "target's owner");
    assert_eq!(
        remote(actor, 0, false, other, true, 0xE0, 0xE8),
        Some("target-identity-mismatch")
    );
    assert_eq!(remote(actor, 0, false, other, true, 0, 0), None, "no identity");
    assert_eq!(remote(actor, own, true, other, false, 0, 0), None, "unproven target");
}

#[test]
fn identity_tests() {
    let hub = hub();
    let fake = &hub.fake;
    assert!(pc(fake, hub.a));
    assert!(!pc(fake, hub.ca));
    assert!(conn(fake, hub.ca));
    assert!(!conn(fake, hub.a), "a controller is no connection");
    fake.live.borrow_mut().remove(&hub.cb);
    assert!(!conn(fake, hub.cb), "not live");
    assert_eq!(live_or_null(fake, hub.ca + at::CONNECTION_PLAYER_CONTROLLER), hub.a);
    fake.live.borrow_mut().remove(&hub.a);
    assert_eq!(live_or_null(fake, hub.ca + at::CONNECTION_PLAYER_CONTROLLER), 0);
    fake.classes.borrow_mut().remove("PlayerController");
    assert!(!pc(fake, hub.b), "class not found yet: false");
}

#[test]
fn quarantine_table() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let mut table = Quarantine::new();
    assert!(!table.contains(0x500));
    let queued = |channel| Queued {
        channel,
        actor: 0xA0,
        wrong: 0xC0,
        correct: 0,
        source: Source::SetChannelActor,
        caller: 0x3D8_FB20,
    };
    assert!(!table.queue(&fake, &budgets, queued(0)), "null channel");
    fake.now.set(5);
    assert!(table.queue(&fake, &budgets, queued(0x500)));
    assert_eq!(
        fake.take_lines(),
        [
            "[MitRepGraph] quarantine: queued channel=0x500 actor=0xA0 wrong=0xC0 correct=null source=set-channel-actor \
          caller=+0x3D8FB20"
        ]
    );
    assert!(table.contains(0x500));
    assert_eq!(table.entry(0x500).map(|entry| entry.queued_ms), Some(5));
    let mut again = queued(0x500);
    again.actor = 0xFF;
    again.correct = 0xC8;
    assert!(table.queue(&fake, &budgets, again));
    assert!(fake.take_lines().is_empty(), "a second queue does not log");
    let entry = *table.entry(0x500).expect("entry");
    assert_eq!(
        (entry.actor, entry.correct_connection),
        (0xA0, 0xC8),
        "only zero fields filled"
    );
    for n in 1..TABLE {
        assert!(table.queue(&fake, &budgets, queued(0x500 + n * 8)));
    }
    fake.take_lines();
    assert!(!table.queue(&fake, &budgets, queued(0x9000)), "65th channel");
    assert_eq!(
        fake.take_lines(),
        ["[MitRepGraph] quarantine: table full channel=0x9000 actor=0xA0 wrong=0xC0 correct=null"]
    );
    fake.now.set(900);
    assert!(!table.queue(&fake, &budgets, queued(0x9008)));
    assert!(fake.take_lines().is_empty(), "not twice within a second");
    fake.now.set(1005);
    assert!(!table.queue(&fake, &budgets, queued(0x9010)));
    assert_eq!(fake.take_lines().len(), 1);
}

#[test]
fn stale_entries() {
    let hub = hub();
    let fake = &hub.fake;
    let budgets = Budgets::new();
    let mut table = Quarantine::new();
    let wrong = fake.chan(0x30_0000, hub.cb, hub.a);
    let moved = fake.chan(0x31_0000, hub.ca, 0);
    let rebound = fake.chan(0x32_0000, hub.cb, hub.b);
    let dead = 0x33_0000;
    for channel in [wrong, moved, rebound, dead] {
        table.queue(
            fake,
            &budgets,
            Queued {
                channel,
                actor: hub.a,
                wrong: hub.cb,
                correct: hub.ca,
                source: Source::ReplicateActorFinal,
                caller: 0,
            },
        );
    }
    fake.take_lines();
    table.sweep(fake, &budgets);
    assert_eq!(table.len(), 1);
    assert!(table.contains(wrong), "still wrong: kept");
    let lines = fake.take_lines();
    assert_eq!(lines.len(), 3);
    let has = |line: &str| lines.iter().any(|written| written == line);
    assert!(has(
        "[MitRepGraph] quarantine: dropped stale channel=0x310000 expectedActor=0x100000 currentActor=null \
         expectedConnection=0x210000 currentConnection=0x200000"
    ));
    assert!(has(
        "[MitRepGraph] quarantine: dropped stale channel=0x330000 expectedActor=0x100000 currentActor=null \
         expectedConnection=0x210000 currentConnection=null"
    ));
}

#[test]
fn class_memo() {
    let hub = hub();
    let fake = &hub.fake;
    let mut memo = ClassMemo::new();
    let pawn = fake.object(0x40_0000, 0x420, CLASS_OTHER);
    assert!(is_controller(fake, &mut memo, hub.a));
    assert_eq!(memo.len(), 1, "filled on the first lookup");
    assert!(!is_controller(fake, &mut memo, pawn));
    assert_eq!(memo.len(), 2);
    fake.class_reads.set(0);
    assert!(!is_controller(fake, &mut memo, pawn));
    assert_eq!(fake.class_reads.get(), 0, "the second lookup walks no chain");
    assert!(is_controller(fake, &mut memo, hub.b));
    assert_eq!(memo.len(), 2, "the same class");
    fake.world.set(0x9100);
    assert!(is_controller(fake, &mut memo, hub.a));
    assert_eq!(memo.len(), 1, "emptied on a world change");
    // A full table answers without remembering
    let mut full = ClassMemo::new();
    for n in 0..MEMO_SLOTS {
        let class = 0x80_0000 + n * 0x100;
        fake.add_class(class, CLASS_ROOT, "Filler");
        assert!(!full.is_controller_class(fake, class));
    }
    assert_eq!(full.len(), MEMO_SLOTS);
    assert!(full.is_controller_class(fake, CLASS_ARCHON_PC));
    assert_eq!(full.len(), MEMO_SLOTS);
}

fn info(fake: &Fake, base: usize, channel: usize) -> usize {
    fake.alloc(base, 8);
    fake.put(base, channel);
    base
}

fn manager(fake: &Fake, base: usize, connection: usize) -> usize {
    fake.object(base, 0x30, CLASS_ROOT);
    fake.put(base + at::CONN_MANAGER_CONNECTION, connection);
    base
}

#[test]
fn pre_channel_non_controller_passes() {
    let hub = hub();
    let fake = &hub.fake;
    let pawn = fake.object(0x40_0000, 0x420, CLASS_OTHER);
    let mut state = State::new();
    let call = PreChannel {
        actor: pawn,
        conn_actor_info: 0,
        conn_manager: 0,
        frame: 1,
    };
    assert_eq!(pre_channel(fake, &mut state, &Budgets::new(), call), Verdict::Next);
    assert!(fake.take_lines().is_empty());
}

#[test]
fn pre_channel_own_connection_passes() {
    let hub = hub();
    let fake = &hub.fake;
    let own = fake.chan(0x30_0000, hub.ca, hub.a);
    let slot = info(fake, 0x50_0000, own);
    let manager = manager(fake, 0x60_0000, hub.ca);
    let mut state = State::new();
    let call = PreChannel {
        actor: hub.a,
        conn_actor_info: slot,
        conn_manager: manager,
        frame: 7,
    };
    assert_eq!(pre_channel(fake, &mut state, &Budgets::new(), call), Verdict::Next);
    assert_eq!(fake.get(slot), own);
    assert!(fake.take_lines().is_empty());
}

#[test]
fn pre_channel_cached_reasons() {
    let hub = hub();
    let fake = &hub.fake;
    let budgets = Budgets::new();
    let manager = manager(fake, 0x60_0000, hub.ca);
    // A cached channel on B's connection, still null: queued and cleared
    let mut state = State::new();
    let other = fake.chan(0x30_0000, hub.cb, 0);
    let slot = info(fake, 0x50_0000, other);
    let call = PreChannel {
        actor: hub.a,
        conn_actor_info: slot,
        conn_manager: manager,
        frame: 7,
    };
    assert_eq!(pre_channel(fake, &mut state, &budgets, call), Verdict::Next);
    assert_eq!(fake.get(slot), 0, "cleared");
    let entry = *state.quarantine.entry(other).expect("queued");
    assert_eq!(entry.source, Source::ConnActorInfoMismatch);
    assert_eq!((entry.wrong_connection, entry.correct_connection), (hub.cb, hub.ca));
    let lines = fake.take_lines();
    assert_eq!(
        lines[1],
        "[MitRepGraph] pre-channel: reset cached channel actor=0x100000 actorConnection=0x200000 target=0x200000 \
         channel=0x300000 channelConnection=0x210000 channelActor=null cleared=1 \
         reason=cached-channel-connection-mismatch"
    );
    // Another player's connection carrying another actor: reset, not queued
    let mut state = State::new();
    let carried = fake.chan(0x31_0000, hub.cb, hub.b);
    fake.put(slot, carried);
    pre_channel(fake, &mut state, &budgets, call);
    assert!(state.quarantine.is_empty());
    assert!(fake.take_lines()[0].ends_with("cleared=1 reason=cached-channel-connection-mismatch"));
    // Own connection, another actor
    let foreign = fake.chan(0x32_0000, hub.ca, hub.b);
    fake.put(slot, foreign);
    pre_channel(fake, &mut state, &budgets, call);
    assert!(fake.take_lines()[0].ends_with("reason=cached-channel-actor-mismatch"));
    // Not a live channel; the slot is read-only, so the write's check fails
    fake.alloc(0x34_0000, 0x78);
    fake.put(slot, 0x34_0000);
    fake.read_only.borrow_mut().insert(slot);
    pre_channel(fake, &mut state, &budgets, call);
    assert!(fake.take_lines()[0].ends_with("cleared=-1 reason=cached-channel-unusable"));
    assert!(state.quarantine.is_empty());
}

#[test]
fn pre_channel_slot_cleared_only_while_it_matches() {
    struct Swapping<'a>(&'a Fake, usize, Cell<u32>);
    impl Game for Swapping<'_> {
        fn readable(&self, ptr: usize, size: usize) -> bool {
            self.0.readable(ptr, size)
        }
        fn readable_this_frame(&self, ptr: usize, size: usize) -> bool {
            self.0.readable(ptr, size)
        }
        fn read(&self, addr: usize, out: &mut [u8]) -> bool {
            // The slot changes under the guard after its first read
            if addr == self.1 {
                self.2.set(self.2.get() + 1);
                if self.2.get() == 2 {
                    self.0.put(self.1, 0x77_0000);
                }
            }
            self.0.read(addr, out)
        }
        fn write_ptr(&self, addr: usize, value: usize) -> bool {
            self.0.write_ptr(addr, value)
        }
        fn is_live_object(&self, ptr: usize) -> bool {
            self.0.is_live_object(ptr)
        }
        fn class(&self, class: Class) -> Option<usize> {
            Game::class(self.0, class)
        }
        fn world(&self) -> usize {
            self.0.world()
        }
        fn now_ms(&self) -> u64 {
            0
        }
        fn name(&self, _: usize) -> Option<String> {
            None
        }
        fn class_name(&self, _: usize) -> Option<String> {
            None
        }
        fn image_rva(&self, _: usize) -> Option<u32> {
            None
        }
        fn function_entry(&self, _: u32) -> Option<u32> {
            None
        }
        fn log(&self, line: &str) {
            self.0.log(line);
        }
    }
    let hub = hub();
    let fake = &hub.fake;
    let manager = manager(fake, 0x60_0000, hub.ca);
    let foreign = fake.chan(0x32_0000, hub.ca, hub.b);
    let slot = info(fake, 0x50_0000, foreign);
    let game = Swapping(fake, slot, Cell::new(0));
    let call = PreChannel {
        actor: hub.a,
        conn_actor_info: slot,
        conn_manager: manager,
        frame: 7,
    };
    pre_channel(&game, &mut State::new(), &Budgets::new(), call);
    assert_eq!(fake.get(slot), 0x77_0000, "the new value is left alone");
    assert!(fake.take_lines()[0].contains("cleared=0 "));
}

#[test]
fn pre_channel_unproven_target_fails_open() {
    let hub = hub();
    let fake = &hub.fake;
    let budgets = Budgets::new();
    let mut state = State::new();
    let manager = manager(fake, 0x60_0000, 0x70_0000);
    let call = PreChannel {
        actor: hub.a,
        conn_actor_info: 0,
        conn_manager: manager,
        frame: 3,
    };
    fake.now.set(10_000);
    assert_eq!(pre_channel(fake, &mut state, &budgets, call), Verdict::Next);
    assert_eq!(
        fake.take_lines(),
        [
            "[MitRepGraph] pre-channel: target connection unproven, allowed actor=0x100000 manager=0x600000 \
          target=0x700000 frame=3"
        ]
    );
    fake.now.set(10_500);
    assert_eq!(pre_channel(fake, &mut state, &budgets, call), Verdict::Next);
    assert!(fake.take_lines().is_empty(), "once a second");
    fake.now.set(11_000);
    pre_channel(fake, &mut state, &budgets, call);
    assert_eq!(fake.take_lines().len(), 1);
}

#[test]
fn pre_channel_remote_controller_absorbed() {
    let hub = hub();
    let fake = &hub.fake;
    let budgets = Budgets::new();
    let mut state = State::new();
    let manager = manager(fake, 0x60_0000, hub.cb);
    let before = fake.chan(0x30_0000, hub.cb, 0);
    let slot = info(fake, 0x50_0000, before);
    let call = PreChannel {
        actor: hub.a,
        conn_actor_info: slot,
        conn_manager: manager,
        frame: 9,
    };
    assert_eq!(pre_channel(fake, &mut state, &budgets, call), Verdict::Absorb);
    assert_eq!(fake.get(slot), 0, "cleared");
    let entry = *state.quarantine.entry(before).expect("queued");
    assert_eq!(entry.source, Source::RemoteControllerPreChannel);
    assert_eq!((entry.wrong_connection, entry.correct_connection), (hub.cb, hub.ca));
    let lines = fake.take_lines();
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[1],
        "[MitRepGraph] pre-channel: blocked actor=0x100000 actorConnection=0x200000 target=0x210000 \
         targetController=0x110000 targetOwner=0x110000 channelBefore=0x300000 frame=9 \
         reason=actor-netconnection-mismatch"
    );
    assert_eq!(pre_channel(fake, &mut state, &budgets, call), Verdict::Absorb);
    assert!(fake.take_lines().is_empty(), "once per pair");
    // The 65th pair falls back to the budget
    for n in 0..TABLE {
        state.pre_pairs.note((n + 1, 1));
    }
    assert_eq!(state.pre_pairs.note((hub.b, hub.ca)), Pair::Full);
    let other = PreChannel {
        actor: hub.b,
        conn_manager: manager_for(fake, hub.ca),
        ..call
    };
    pre_channel(fake, &mut state, &budgets, other);
    assert_eq!(fake.take_lines().len(), 1, "the budget's first line");
}

fn manager_for(fake: &Fake, connection: usize) -> usize {
    manager(fake, 0x61_0000, connection)
}

#[test]
fn bind_decisions() {
    let hub = hub();
    let fake = &hub.fake;
    let budgets = Budgets::new();
    let mut state = State::new();
    // B's channel, A's controller: wrong by connection
    let theirs = fake.chan(0x30_0000, hub.cb, 0);
    assert_eq!(
        bind(fake, &mut state, &budgets, theirs, hub.a, 0x2, || 0x00F2_7D40),
        Verdict::Absorb
    );
    assert!(state.quarantine.contains(theirs));
    let lines = fake.take_lines();
    assert_eq!(
        lines[1],
        "[MitRepGraph] bind: refused actor=0x100000 actorConnection=0x200000 channel=0x300000 \
         channelConnection=0x210000 channelController=0x110000 channelOwner=0x110000 previousActor=null flags=0x2 \
         caller=+0xF27D40 reason=actor-netconnection-mismatch queued=1"
    );
    // A later bind on the same channel is refused as quarantined, with its caller
    assert_eq!(
        bind(fake, &mut state, &budgets, theirs, hub.b, 0, || 0x1234),
        Verdict::Absorb
    );
    assert_eq!(
        fake.take_lines(),
        ["[MitRepGraph] bind: refused quarantined channel=0x300000 actor=0x110000 caller=+0x1234"]
    );
    // By identity: the controller's own connection unknown
    let stray = fake.pc(0x12_0000, 0);
    let channel = fake.chan(0x31_0000, hub.cb, 0);
    assert_eq!(
        bind(fake, &mut state, &budgets, channel, stray, 0, || 0),
        Verdict::Absorb
    );
    assert!(fake.take_lines()[1].ends_with("reason=target-identity-mismatch queued=1"));
    // Passes: own channel, a non-controller, an unproven channel connection
    let own = fake.chan(0x32_0000, hub.ca, 0);
    assert_eq!(bind(fake, &mut state, &budgets, own, hub.a, 0, || 0), Verdict::Next);
    let pawn = fake.object(0x40_0000, 0x420, CLASS_OTHER);
    let other = fake.chan(0x33_0000, hub.cb, 0);
    assert_eq!(bind(fake, &mut state, &budgets, other, pawn, 0, || 0), Verdict::Next);
    let loose = fake.chan(0x34_0000, 0x70_0000, 0);
    assert_eq!(bind(fake, &mut state, &budgets, loose, hub.a, 0, || 0), Verdict::Next);
    assert!(fake.take_lines().is_empty());
}

#[test]
fn send_decisions() {
    let hub = hub();
    let fake = &hub.fake;
    let budgets = Budgets::new();
    let mut state = State::new();
    // Quarantined: silent
    state.quarantine.queue(
        fake,
        &budgets,
        Queued {
            channel: 0x39_0000,
            actor: 0,
            wrong: 0,
            correct: 0,
            source: Source::SetChannelActor,
            caller: 0,
        },
    );
    fake.take_lines();
    assert_eq!(send(fake, &mut state, &budgets, 0x39_0000), Verdict::Absorb);
    assert!(fake.take_lines().is_empty());
    // Remote: queued and blocked once per pair
    let wrong = fake.chan(0x30_0000, hub.cb, hub.a);
    fake.put_i32(hub.a + at::ACTOR_FLAGS_58, 0x14);
    assert_eq!(send(fake, &mut state, &budgets, wrong), Verdict::Absorb);
    assert!(state.quarantine.contains(wrong));
    let lines = fake.take_lines();
    assert_eq!(
        lines[1],
        "[MitRepGraph] send: blocked actor=0x100000 actorConnection=0x200000 target=0x210000 \
         targetController=0x110000 targetOwner=0x110000 flags=0x14 reason=actor-netconnection-mismatch"
    );
    // By identity with an unproven target: its controller and owner read as 0, so nothing to compare
    let stray = fake.pc(0x12_0000, 0);
    let loose = fake.chan(0x31_0000, 0x70_0000, stray);
    assert_eq!(send(fake, &mut state, &budgets, loose), Verdict::Next);
    // By identity with a proven target
    let identity = fake.chan(0x32_0000, hub.cb, stray);
    assert_eq!(send(fake, &mut state, &budgets, identity), Verdict::Absorb);
    assert!(fake.take_lines()[1].ends_with("reason=target-identity-mismatch"));
    // Own channel passes
    let own = fake.chan(0x33_0000, hub.ca, hub.a);
    assert_eq!(send(fake, &mut state, &budgets, own), Verdict::Next);
}

#[test]
fn rpc_decisions() {
    let hub = hub();
    let fake = &hub.fake;
    let budgets = Budgets::new();
    let mut state = State::new();
    let unbound = fake.chan(0x30_0000, hub.cb, 0);
    let call = |channel, is_server| Rpc {
        channel,
        target_obj: hub.a,
        connection: hub.cb,
        function: 0x88_0000,
        is_server,
    };
    assert_eq!(
        rpc(fake, &state, &budgets, call(unbound, 0)),
        Verdict::Next,
        "client call"
    );
    assert_eq!(rpc(fake, &state, &budgets, call(0, 1)), Verdict::Next, "null channel");
    fake.names
        .borrow_mut()
        .insert(0x88_0000, "ClientSelectRelic".to_owned());
    assert_eq!(rpc(fake, &state, &budgets, call(unbound, 1)), Verdict::Absorb);
    assert_eq!(
        fake.take_lines(),
        [
            "[MitRepGraph] rpc: dropped fn=ClientSelectRelic target=unreadable channel=0x300000 connection=0x210000 \
          reason=channel-without-actor count=1"
        ]
    );
    assert_eq!(
        rpc(fake, &state, &budgets, call(0x7F_0000, 1)),
        Verdict::Absorb,
        "unreadable actor"
    );
    let bound = fake.chan(0x31_0000, hub.ca, hub.a);
    assert_eq!(rpc(fake, &state, &budgets, call(bound, 1)), Verdict::Next);
    state.quarantine.queue(
        fake,
        &budgets,
        Queued {
            channel: bound,
            actor: hub.a,
            wrong: hub.cb,
            correct: 0,
            source: Source::SetChannelActor,
            caller: 0,
        },
    );
    fake.take_lines();
    assert_eq!(rpc(fake, &state, &budgets, call(bound, 1)), Verdict::Absorb);
    assert!(fake.take_lines()[0].contains("reason=quarantined-channel count=3"));
    // The schedule: 1–64, then every 256th
    let schedule = Budgets::new();
    let logged: Vec<u32> = (0..1024)
        .filter_map(|_| Some(schedule.rpc_drop()).filter(|d| d.1).map(|d| d.0))
        .collect();
    assert_eq!(logged.len(), 64 + 4);
    assert_eq!(logged[63..], [64, 256, 512, 768, 1024]);
}

#[test]
fn rpc_drop_schedule_names() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    fake.alloc(0x30_0000, 0x78);
    let call = Rpc {
        channel: 0x30_0000,
        target_obj: 0,
        connection: 0,
        function: 0,
        is_server: 1,
    };
    rpc(&fake, &State::new(), &budgets, call);
    assert_eq!(
        fake.take_lines(),
        [
            "[MitRepGraph] rpc: dropped fn=null target=null channel=0x300000 connection=null \
          reason=channel-without-actor count=1"
        ]
    );
}

#[test]
fn drive_checks() {
    let fake = Fake::new();
    let mut checked = Checked::new();
    let (driver, graph, vtable) = (0x100_0000, 0x110_0000, 0x120_0000);
    assert_eq!(drive_target(&fake, &mut checked, None), None, "no driver");
    assert_eq!(
        drive_target(&fake, &mut checked, Some(driver)),
        None,
        "unreadable driver"
    );
    fake.alloc(driver, 0x6F0);
    assert_eq!(drive_target(&fake, &mut checked, Some(driver)), None, "null graph");
    fake.put(driver + at::NETDRIVER_REPLICATION_DRIVER, graph);
    fake.alloc(graph, 0x4C8);
    fake.put(graph + at::GRAPH_NET_DRIVER, 0x130_0000);
    assert_eq!(drive_target(&fake, &mut checked, Some(driver)), None, "foreign graph");
    fake.put(graph + at::GRAPH_NET_DRIVER, driver);
    fake.put(graph, vtable);
    assert_eq!(drive_target(&fake, &mut checked, Some(driver)), None, "unreadable slot");
    fake.alloc(vtable, 0x300);
    let image = 0x1_4000_0000;
    fake.image.set((image, 0x700_0000));
    let outside = 0x2_0000_0000;
    fake.put(vtable + 0x2F8, outside);
    assert_eq!(drive_target(&fake, &mut checked, Some(driver)), None);
    assert_eq!(
        fake.take_lines(),
        ["[MitRepGraph] drive: slot +0x2F8 of graph=0x1100000 is 0x200000000, not a function of the game; not called"]
    );
    assert_eq!(drive_target(&fake, &mut checked, Some(driver)), None);
    assert!(fake.take_lines().is_empty(), "one line per distinct value");
    let inside = image + 0x01C0_34C1;
    fake.put(vtable + 0x2F8, inside);
    assert_eq!(
        drive_target(&fake, &mut checked, Some(driver)),
        None,
        "not a .pdata entry"
    );
    assert_eq!(fake.take_lines().len(), 1);
    let entry = image + 0x01C0_34C0;
    fake.entries.borrow_mut().insert(0x01C0_34C0);
    fake.put(vtable + 0x2F8, entry);
    assert_eq!(drive_target(&fake, &mut checked, Some(driver)), Some((graph, entry)));
    assert!(fake.take_lines().is_empty());
    assert_eq!(graph_of(&fake, driver), Some(graph));
    // The first six drives log
    let budgets = Budgets::new();
    fake.put(graph + 0x10, 0x150_0000);
    fake.names
        .borrow_mut()
        .insert(0x150_0000, "ArchonReplicationGraph".to_owned());
    fake.put_i32(graph + at::GRAPH_CONNECTIONS_NUM, 1);
    fake.put_i32(graph + at::GRAPH_DEFERRED_NUM, 0);
    for _ in 0..8 {
        drive_line(&fake, &budgets, graph, entry, 42);
    }
    let lines = fake.take_lines();
    assert_eq!(lines.len(), 6);
    assert_eq!(
        lines[0],
        "[MitRepGraph] drive: graph=0x1100000 class=ArchonReplicationGraph fn=+0x1C034C0 -> 42 connections=1 deferred=0"
    );
}

#[test]
fn drive_reentry() {
    let flag = Cell::new(false);
    let outer = crate::net::Reentry::enter(&flag);
    assert!(outer.is_some());
    assert!(crate::net::Reentry::enter(&flag).is_none(), "a nested drive returns");
    drop(outer);
    assert!(crate::net::Reentry::enter(&flag).is_some());
}

#[test]
fn budgets_and_texts() {
    let fake = Fake::new();
    let budget = crate::net::LineBudget::new(BUDGET);
    for _ in 0..70 {
        budgeted(&fake, &budget, kinds::BIND_REFUSED, || "line".to_owned());
    }
    let lines = fake.take_lines();
    assert_eq!(lines.len(), 65);
    assert_eq!(lines[64], "[MitRepGraph] bind refused: further lines suppressed");
    assert_eq!(budget.admit(), Admit::Drop);
    assert_eq!(addr(0), "null");
    assert_eq!(addr(0xABC), "0xABC");
    assert_eq!(rva(0), "+0x0");
    assert_eq!(rpc_guard(true), "[MitRepGraph] rpc guard on");
    assert_eq!(
        rpc_guard(false),
        "[MitRepGraph] rpc guard off (DISABLE_RPC_UNBOUND_CHANNEL_GUARD.flag)"
    );
    assert_eq!(DRIVE_ORDER, 500);
    const { assert!(DRIVE_ORDER < crate::net::MANUAL_TICK_ORDER) };
    assert_eq!(flags::DROPPED.len(), 5);
    assert!(!flags::DROPPED.contains(&flags::DISABLE_RPC_UNBOUND_CHANNEL_GUARD));
    let mut last = None;
    assert!(once_a_second(&mut last, 0));
    assert!(!once_a_second(&mut last, 999));
    assert!(once_a_second(&mut last, 1000));
}
