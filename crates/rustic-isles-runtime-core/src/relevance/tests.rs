//! R13's core tests (15.1) over a fake game: blocks of bytes at made-up addresses.

use super::*;
use crate::offsets::relevance as at;
use crate::offsets::repgraph as rg;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};

const CLASS_ROOT: usize = 0x1_0000;
const CLASS_PC: usize = 0x1_0100;
const CLASS_PS: usize = 0x1_0200;
const CLASS_ROLE: usize = 0x1_0300;
const CLASS_WEAPON: usize = 0x1_0400;
const CLASS_CHARACTER: usize = 0x1_0500;
const CLASS_BP_PLAYER: usize = 0x1_0600;
const CLASS_OTHER: usize = 0x1_0700;
const CLASS_CREATURE: usize = 0x1_0800;
const GRAPH: usize = 0x10_0000;

struct Fake {
    blocks: RefCell<BTreeMap<usize, Vec<u8>>>,
    read_only: RefCell<HashSet<usize>>,
    live: RefCell<HashSet<usize>>,
    dying: RefCell<HashSet<usize>>,
    classes: RefCell<HashMap<&'static str, usize>>,
    names: RefCell<HashMap<usize, String>>,
    now: Cell<u64>,
    lines: RefCell<Vec<String>>,
    next: Cell<usize>,
    index: Cell<i32>,
    wake_answer: Cell<WakeCall>,
    wakes: RefCell<Vec<usize>>,
    writes: Cell<u32>,
}

impl Fake {
    fn block_of(&self, addr: usize, size: usize) -> Option<(usize, usize)> {
        let blocks = self.blocks.borrow();
        let (&base, bytes) = blocks.range(..=addr).next_back()?;
        let end = addr.checked_add(size)?;
        (end <= base + bytes.len()).then_some((base, addr - base))
    }

    fn alloc(&self, base: usize, size: usize) -> usize {
        self.blocks.borrow_mut().insert(base, vec![0; size]);
        base
    }

    /// A fresh block, 0x100-aligned, past every other.
    fn fresh(&self, size: usize) -> usize {
        let base = self.next.get();
        self.next.set(base + size.div_ceil(0x100) * 0x100 + 0x100);
        self.alloc(base, size)
    }

    fn put_bytes(&self, addr: usize, bytes: &[u8]) {
        let (base, offset) = self.block_of(addr, bytes.len()).expect("inside a block");
        self.blocks.borrow_mut().get_mut(&base).expect("block")[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    fn put(&self, addr: usize, value: usize) {
        self.put_bytes(addr, &(value as u64).to_le_bytes());
    }
    fn put_i32(&self, addr: usize, value: i32) {
        self.put_bytes(addr, &value.to_le_bytes());
    }
    fn put_u8(&self, addr: usize, value: u8) {
        self.put_bytes(addr, &[value]);
    }
    fn put_u16(&self, addr: usize, value: u16) {
        self.put_bytes(addr, &value.to_le_bytes());
    }
    fn put_f32(&self, addr: usize, value: f32) {
        self.put_bytes(addr, &value.to_le_bytes());
    }

    fn add_class(&self, class: usize, sup: usize, name: &'static str) {
        self.alloc(class, 0x50);
        self.put(class + rg::STRUCT_SUPER, sup);
        self.classes.borrow_mut().insert(name, class);
    }

    fn object(&self, size: usize, class: usize, name: &str) -> usize {
        let base = self.fresh(size);
        self.put(base + 0x10, class);
        self.index.set(self.index.get() + 1);
        self.put_i32(base + 0x0C, self.index.get());
        self.live.borrow_mut().insert(base);
        self.names.borrow_mut().insert(base, name.to_owned());
        base
    }

    fn new() -> Self {
        let fake = Self {
            blocks: RefCell::default(),
            read_only: RefCell::default(),
            live: RefCell::default(),
            dying: RefCell::default(),
            classes: RefCell::default(),
            names: RefCell::default(),
            now: Cell::new(100_000),
            lines: RefCell::default(),
            next: Cell::new(0x100_0000),
            index: Cell::new(0),
            wake_answer: Cell::new(WakeCall::Made),
            wakes: RefCell::default(),
            writes: Cell::new(0),
        };
        fake.add_class(CLASS_ROOT, 0, "Object");
        fake.add_class(CLASS_PC, CLASS_ROOT, "PlayerController");
        fake.add_class(CLASS_PS, CLASS_ROOT, "PlayerState");
        fake.add_class(CLASS_ROLE, CLASS_ROOT, "ArchonPlayerRole");
        fake.add_class(CLASS_WEAPON, CLASS_ROOT, "ArchonWeapon");
        fake.add_class(CLASS_CHARACTER, CLASS_ROOT, "ArchonCharacter");
        fake.add_class(CLASS_BP_PLAYER, CLASS_CHARACTER, "BP_PlayerCharacter_C");
        fake.add_class(CLASS_OTHER, CLASS_ROOT, "Pawn");
        fake.add_class(CLASS_CREATURE, CLASS_ROOT, "ArchonCreatureBase");
        fake.alloc(GRAPH, rg::GRAPH_SPAN);
        fake
    }

    /// A map at `header` (a `TMap` laid out as 5.1 says) holding `entries`, every bit set.
    fn map(&self, header: usize, entries: &[(usize, usize)], stride: usize) -> usize {
        let data = self.fresh((entries.len().max(1)) * stride);
        for (i, &(key, value)) in entries.iter().enumerate() {
            self.put(data + i * stride, key);
            self.put(data + i * stride + 8, value);
        }
        let num = i32::try_from(entries.len()).expect("small");
        self.put(header + at::MAP_DATA, data);
        self.put_i32(header + at::MAP_NUM, num);
        self.put_i32(header + at::MAP_MAX, num);
        self.put_i32(header + at::MAP_NUM_BITS, num);
        self.put(header + at::MAP_SECONDARY_BITS, 0);
        for word in 0..4 {
            self.put_bytes(header + at::MAP_INLINE_BITS + word * 4, &u32::MAX.to_le_bytes());
        }
        data
    }

    fn info(&self, cull: f32, cull_sq: f32, grace: u16) -> usize {
        let info = self.fresh(at::GLOBAL_INFO_SIZE);
        self.put_f32(info + at::GLOBAL_CULL_DISTANCE, cull);
        self.put_f32(info + at::GLOBAL_CULL_DISTANCE_SQ, cull_sq);
        self.put_u16(info + at::GLOBAL_CHANNEL_TIMEOUT, grace);
        info
    }

    fn set_connections(&self, managers: &[usize]) {
        let data = self.fresh(managers.len().max(1) * 8);
        for (i, &manager) in managers.iter().enumerate() {
            self.put(data + i * 8, manager);
        }
        let num = i32::try_from(managers.len()).expect("small");
        self.put(GRAPH + rg::GRAPH_CONNECTIONS, data);
        self.put_i32(GRAPH + rg::GRAPH_CONNECTIONS_NUM, num);
        self.put_i32(GRAPH + rg::GRAPH_CONNECTIONS_MAX, num);
    }

    fn u8(&self, addr: usize) -> u8 {
        self.u8_at(addr).expect("readable")
    }
    fn f32(&self, addr: usize) -> f32 {
        self.f32_at(addr).expect("readable")
    }
    fn u16(&self, addr: usize) -> u16 {
        self.u16_at(addr).expect("readable")
    }

    fn take_lines(&self) -> Vec<String> {
        std::mem::take(&mut *self.lines.borrow_mut())
    }
}

impl Game for Fake {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        ptr != 0 && ptr.is_multiple_of(8) && self.block_of(ptr, size).is_some()
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        let Some((base, offset)) = self.block_of(addr, out.len()) else {
            return false;
        };
        out.copy_from_slice(&self.blocks.borrow()[&base][offset..offset + out.len()]);
        true
    }
    fn write(&self, addr: usize, bytes: &[u8]) -> bool {
        let Some((base, _)) = self.block_of(addr, bytes.len()) else {
            return false;
        };
        if self.read_only.borrow().contains(&base) {
            return false;
        }
        self.writes.set(self.writes.get() + 1);
        self.put_bytes(addr, bytes);
        true
    }
    fn is_live_object(&self, ptr: usize) -> bool {
        self.live.borrow().contains(&ptr)
    }
    fn is_alive_for_replication(&self, ptr: usize) -> bool {
        self.readable(ptr, 0x60) && self.is_live_object(ptr) && !self.dying.borrow().contains(&ptr)
    }
    fn class(&self, class: Class) -> Option<usize> {
        self.classes.borrow().get(class.name()).copied()
    }
    fn now_ms(&self) -> u64 {
        self.now.get()
    }
    fn name(&self, object: usize) -> Option<String> {
        self.names.borrow().get(&object).cloned()
    }
    fn class_name(&self, object: usize) -> Option<String> {
        let class = self.ptr(object + 0x10);
        self.classes
            .borrow()
            .iter()
            .find(|(_, found)| **found == class)
            .map(|(name, _)| (*name).to_owned())
    }
    fn wake(&self, actor: usize) -> WakeCall {
        self.wakes.borrow_mut().push(actor);
        let answer = self.wake_answer.get();
        if answer == WakeCall::Made {
            self.put_u8(actor + at::ACTOR_NET_DORMANCY, at::DORM_AWAKE);
        }
        answer
    }
    fn log(&self, line: &str) {
        self.lines.borrow_mut().push(line.to_owned());
    }
}

// ---- 5.1, the map search

#[test]
fn map_search_hits_and_skips_holes() {
    let fake = Fake::new();
    let header = fake.fresh(0x30);
    let data = fake.map(header, &[(0xA00, 0x1), (0xB00, 0x2), (0xC00, 0x3)], 0x18);
    assert_eq!(map_find(&fake, header, 0xB00, 0x18), Some(0x2));
    assert_eq!(map_find(&fake, header, 0xD00, 0x18), None);
    // Bit 1 clear: its element is a hole even though its key matches
    fake.put_bytes(header + at::MAP_INLINE_BITS, &0b101u32.to_le_bytes());
    assert_eq!(map_find(&fake, header, 0xB00, 0x18), None);
    assert_eq!(map_find(&fake, header, 0xC00, 0x18), Some(0x3));
    // Only the key is compared: an unreadable value is still the answer, never read through
    fake.put(data + 2 * 0x18 + 8, 0xDEAD_BEEF);
    assert_eq!(map_find(&fake, header, 0xC00, 0x18), Some(0xDEAD_BEEF));
    assert_eq!(map_find(&fake, header, 0, 0x18), None);
    assert_eq!(map_find(&fake, 0, 0xC00, 0x18), None);
}

#[test]
fn map_search_caps() {
    let fake = Fake::new();
    let header = fake.fresh(0x30);
    let entries: Vec<(usize, usize)> = (0..200).map(|i| (0x10_0000 + i * 0x10, i + 1)).collect();
    fake.map(header, &entries, 0x18);
    // Inline words: 128 elements at most
    assert_eq!(map_find(&fake, header, entries[127].0, 0x18), Some(128));
    assert_eq!(map_find(&fake, header, entries[128].0, 0x18), None);
    // A secondary word array covers them all
    let words = fake.fresh(32);
    fake.put_bytes(words, &[0xFF; 32]);
    fake.put(header + at::MAP_SECONDARY_BITS, words);
    assert_eq!(map_find(&fake, header, entries[199].0, 0x18), Some(200));
    // NumBits smaller than ArrayNum caps the walk
    fake.put_i32(header + at::MAP_NUM_BITS, 150);
    assert_eq!(map_find(&fake, header, entries[149].0, 0x18), Some(150));
    assert_eq!(map_find(&fake, header, entries[150].0, 0x18), None);
    // An unreadable secondary array
    fake.put(header + at::MAP_SECONDARY_BITS, 0x7_0000_0000);
    assert_eq!(map_find(&fake, header, entries[0].0, 0x18), None);
}

#[test]
fn malformed_maps_answer_none() {
    let fake = Fake::new();
    let header = fake.fresh(0x30);
    fake.map(header, &[(0xA00, 0x1), (0xB00, 0x2)], 0x18);
    assert_eq!(map_find(&fake, header, 0xA00, 0x18), Some(0x1));
    for (num, max) in [(0, 2), (65_537, 70_000), (3, 2), (-1, 2)] {
        fake.put_i32(header + at::MAP_NUM, num);
        fake.put_i32(header + at::MAP_MAX, max);
        assert_eq!(map_find(&fake, header, 0xA00, 0x18), None, "num {num} max {max}");
    }
    fake.put_i32(header + at::MAP_NUM, 2);
    fake.put_i32(header + at::MAP_MAX, 2);
    let data = fake.ptr(header + at::MAP_DATA);
    // Data readable for fewer than num * stride bytes
    fake.blocks.borrow_mut().insert(data, vec![0; 0x18]);
    assert_eq!(map_find(&fake, header, 0xA00, 0x18), None);
    fake.put(header + at::MAP_DATA, 0);
    assert_eq!(map_find(&fake, header, 0xA00, 0x18), None);
    // An unreadable header
    assert_eq!(map_find(&fake, 0x7_0000_0000, 0xA00, 0x18), None);
}

// ---- 5.2 to 5.4, the toolkit

/// A manager for `connection` with an actor info map.
fn manager(fake: &Fake, connection: usize, infos: &[(usize, usize)]) -> usize {
    let manager = fake.fresh(at::CONN_MANAGER_SPAN);
    fake.put(manager + rg::CONN_MANAGER_CONNECTION, connection);
    fake.map(manager + at::CONN_MANAGER_ACTOR_INFO_MAP, infos, 0x20);
    manager
}

fn conn_info(fake: &Fake, cull: f32, cull_sq: f32) -> usize {
    let info = fake.fresh(at::CONN_INFO_SIZE);
    fake.put_f32(info + at::CONN_INFO_CULL_DISTANCE, cull);
    fake.put_f32(info + at::CONN_INFO_CULL_DISTANCE_SQ, cull_sq);
    info
}

#[test]
fn connection_manager_by_connection() {
    let fake = Fake::new();
    let first = manager(&fake, 0xC1, &[]);
    let second = manager(&fake, 0xC2, &[]);
    fake.set_connections(&[first, 0, second]);
    assert_eq!(connection_manager(&fake, GRAPH, 0xC2), Some(second));
    assert_eq!(connection_manager(&fake, GRAPH, 0xC1), Some(first));
    assert_eq!(connection_manager(&fake, GRAPH, 0xC3), None);
    assert_eq!(connection_manager(&fake, GRAPH, 0), None);
    fake.put_i32(GRAPH + rg::GRAPH_CONNECTIONS_NUM, 1025);
    fake.put_i32(GRAPH + rg::GRAPH_CONNECTIONS_MAX, 2000);
    assert_eq!(connection_manager(&fake, GRAPH, 0xC1), None, "above the limit");
    fake.put_i32(GRAPH + rg::GRAPH_CONNECTIONS_NUM, 3);
    fake.put_i32(GRAPH + rg::GRAPH_CONNECTIONS_MAX, 2);
    assert_eq!(connection_manager(&fake, GRAPH, 0xC1), None, "Num above Max");
}

#[test]
fn global_and_connection_infos() {
    let fake = Fake::new();
    let info = fake.info(1.0, 1.0, 4);
    fake.map(GRAPH + at::GRAPH_GLOBAL_ACTOR_MAP, &[(0xA0, info)], 0x18);
    assert_eq!(global_info(&fake, GRAPH, 0xA0), Some(info));
    assert_eq!(global_info(&fake, GRAPH, 0xB0), None);
    assert_eq!(global_info(&fake, 0, 0xA0), None);
    let snapshot = conn_info(&fake, 1.0, 1.0);
    let manager = manager(&fake, 0xC1, &[(0xA0, snapshot)]);
    assert_eq!(connection_info(&fake, manager, 0xA0), Some(snapshot));
    assert_eq!(connection_info(&fake, manager, 0xB0), None);
}

#[test]
fn settings_are_raised_only() {
    let fake = Fake::new();
    let info = fake.info(15_000.0, 225_000_000.0, 4);
    let settings = apply_settings(&fake, Some(info), true, 300);
    assert_eq!(settings.result, 1);
    assert_eq!(settings.before, (15_000.0, 225_000_000.0, 4));
    assert_eq!(settings.after, (0.0, 0.0, 300));
    assert_eq!(
        (fake.f32(info + 0x88), fake.f32(info + 0x8C), fake.u16(info + 0x30)),
        (0.0, 0.0, 300)
    );
    assert_eq!(apply_settings(&fake, Some(info), true, 300).result, 0, "already right");
    let long = fake.info(1.0, 1.0, 600);
    assert_eq!(apply_settings(&fake, Some(long), false, 300).result, 0);
    assert_eq!(fake.u16(long + 0x30), 600, "a longer grace is kept");
    assert_eq!(fake.f32(long + 0x88), 1.0, "no zero_cull, cull kept");
    let never = fake.info(0.0, 0.0, 0);
    assert_eq!(apply_settings(&fake, Some(never), false, 300).result, 0);
    assert_eq!(fake.u16(never + 0x30), 0, "0 never times out");
    let grace_only = fake.info(5.0, 25.0, 4);
    assert_eq!(apply_settings(&fake, Some(grace_only), true, 0).result, 1);
    assert_eq!(fake.u16(grace_only + 0x30), 4, "min_grace 0 leaves the grace");
    assert_eq!(apply_settings(&fake, None, true, 300), Settings::UNREADABLE);
    let short = fake.fresh(0x20);
    assert_eq!(apply_settings(&fake, Some(short), true, 300).result, -1);
}

#[test]
fn connection_snapshots_cleared() {
    let fake = Fake::new();
    let culled = conn_info(&fake, 15_000.0, 225_000_000.0);
    let clear = conn_info(&fake, 0.0, 0.0);
    let half = conn_info(&fake, 0.0, 9.0);
    let first = manager(&fake, 0xC1, &[(0xA0, culled)]);
    let second = manager(&fake, 0xC2, &[(0xA0, clear)]);
    let third = manager(&fake, 0xC3, &[(0xA0, half)]);
    let short = fake.fresh(0x30);
    let unreadable_info = manager(&fake, 0xC4, &[(0xA0, 0x7_0000_0000)]);
    fake.set_connections(&[first, second, short, 0, third, unreadable_info]);
    assert_eq!(clear_connection_cull(&fake, GRAPH, 0xA0), 2);
    assert_eq!((fake.f32(culled + 8), fake.f32(culled + 0xC)), (0.0, 0.0));
    assert_eq!((fake.f32(half + 8), fake.f32(half + 0xC)), (0.0, 0.0));
    assert_eq!(clear_connection_cull(&fake, GRAPH, 0xA0), 0);
    assert_eq!(clear_connection_cull(&fake, 0, 0xA0), 0);
}

// ---- 6, RG-2

struct Registered {
    info: usize,
    actor: usize,
    global: usize,
}

fn registered(fake: &Fake, class: usize, name: &str, flags58: u8) -> Registered {
    let actor = fake.object(0x420, class, name);
    fake.put_u8(actor + at::ACTOR_FLAGS_58, flags58);
    let info = fake.fresh(at::NEW_ACTOR_INFO_SIZE);
    fake.put(info, actor);
    let global = fake.info(15_000.0, 225_000_000.0, 4);
    Registered { info, actor, global }
}

fn route_of(fake: &Fake, budgets: &Budgets, actor: &Registered) -> Branch {
    route(fake, budgets, actor.info, actor.global)
}

#[test]
fn rg2_controller_goes_to_its_owner() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let pc = registered(&fake, CLASS_PC, "PC_0", 0x08);
    fake.put(pc.actor + rg::PLAYER_CONTROLLER_NET_CONNECTION, 0xC0FFEE00);
    assert_eq!(route_of(&fake, &budgets, &pc), Branch::Controller);
    assert_eq!(fake.u8(pc.actor + 0x58), 0x04);
    assert_eq!(
        fake.take_lines(),
        [format!(
            "[MitRelevance] route: controller actor=0x{:X} connection=0xC0FFEE00 flags58=0x08->0x04 route=owner",
            pc.actor
        )]
    );
    assert_eq!(fake.u16(pc.global + 0x30), 4, "no settings for a controller");
    assert_eq!(controller_flags(0x0C), 0x04);
    assert_eq!(controller_flags(0x80), 0x84);
}

#[test]
fn rg2_player_state_and_role_go_global() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let ps = registered(&fake, CLASS_PS, "PS_0", 0x04);
    fake.put_u8(ps.actor + 0x59, 0x03);
    assert_eq!(route_of(&fake, &budgets, &ps), Branch::PlayerState);
    assert_eq!((fake.u8(ps.actor + 0x58), fake.u8(ps.actor + 0x59)), (0x08, 0x01));
    assert_eq!((fake.f32(ps.global + 0x88), fake.u16(ps.global + 0x30)), (0.0, 300));
    assert_eq!(
        fake.take_lines(),
        [format!(
            "[MitRelevance] route: player-state actor=0x{:X} class=PlayerState flags58=0x04->0x08 flags59=0x03->0x01 \
             settings=1 cull=15000->0 cullSq=225000000->0 grace=4->300 route=global",
            ps.actor
        )]
    );
    // The role route is unconditional (the switch is dropped)
    let role = registered(&fake, CLASS_ROLE, "Role_0", 0x00);
    fake.put_u8(role.actor + 0x59, 0x02);
    assert_eq!(route_of(&fake, &budgets, &role), Branch::Role);
    assert_eq!((fake.u8(role.actor + 0x58), fake.u8(role.actor + 0x59)), (0x08, 0x00));
    assert_eq!(
        fake.take_lines(),
        [format!(
            "[MitRelevance] route: role actor=0x{:X} flags58=0x00->0x08 flags59=0x02->0x00 settings=1 cull=15000->0 \
             cullSq=225000000->0 grace=4->300 route=global",
            role.actor
        )]
    );
    assert_eq!(global_flags(0x0C, 0xFF), (0x08, 0xFD));
}

#[test]
fn rg2_character_gets_the_longer_grace() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let pawn = registered(&fake, CLASS_BP_PLAYER, "BP_PlayerCharacter_C_0", 0x00);
    assert_eq!(route_of(&fake, &budgets, &pawn), Branch::Character);
    assert_eq!(fake.u16(pawn.global + 0x30), 300);
    assert_eq!(
        fake.f32(pawn.global + 0x88),
        15_000.0,
        "the cull is kept: it stays spatial"
    );
    assert_eq!(fake.u8(pawn.actor + 0x58), 0x00);
    assert_eq!(
        fake.take_lines(),
        [format!(
            "[MitRelevance] route: character actor=0x{:X} class=BP_PlayerCharacter_C settings=1 grace=4->300",
            pawn.actor
        )]
    );
    assert_eq!(route_of(&fake, &budgets, &pawn), Branch::Character);
    assert!(fake.take_lines().is_empty(), "no line when nothing changed");
    // A behemoth is a sibling class: untouched
    let creature = registered(&fake, CLASS_CREATURE, "Behemoth", 0x00);
    assert_eq!(route_of(&fake, &budgets, &creature), Branch::None);
    assert_eq!(fake.u16(creature.global + 0x30), 4);
}

#[test]
fn rg2_order_and_refusals() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    // A class that is both a controller and a weapon never happens; the first test wins
    fake.add_class(0x1_0900, CLASS_WEAPON, "Odd");
    fake.put(CLASS_WEAPON + rg::STRUCT_SUPER, CLASS_PC);
    let odd = registered(&fake, 0x1_0900, "Odd", 0x08);
    assert_eq!(route_of(&fake, &budgets, &odd), Branch::Controller);
    fake.put(CLASS_WEAPON + rg::STRUCT_SUPER, CLASS_ROOT);
    fake.take_lines();
    // Dead, destroying, an unreadable info, no actor: nothing written
    let writes = fake.writes.get();
    let dead = registered(&fake, CLASS_PS, "PS_dead", 0x04);
    fake.live.borrow_mut().remove(&dead.actor);
    assert_eq!(route_of(&fake, &budgets, &dead), Branch::None);
    let dying = registered(&fake, CLASS_PS, "PS_dying", 0x04);
    fake.dying.borrow_mut().insert(dying.actor);
    assert_eq!(route_of(&fake, &budgets, &dying), Branch::None);
    assert_eq!(route(&fake, &budgets, 0x7_0000_0000, dying.global), Branch::None);
    let empty = fake.fresh(0x18);
    assert_eq!(route(&fake, &budgets, empty, dying.global), Branch::None);
    let other = registered(&fake, CLASS_OTHER, "Pawn", 0x04);
    assert_eq!(route_of(&fake, &budgets, &other), Branch::None);
    assert_eq!(fake.writes.get(), writes);
    // A class not found yet answers false
    fake.classes.borrow_mut().remove("PlayerState");
    let unfound = registered(&fake, CLASS_PS, "PS_1", 0x04);
    assert_eq!(route_of(&fake, &budgets, &unfound), Branch::None);
}

#[test]
fn rg2_weapon() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    // Dormant on the spatial route: woken, filed dynamic
    let dormant = registered(&fake, CLASS_WEAPON, "WP_IH_BEGINNER", 0x00);
    let owner = fake.object(0x748, CLASS_BP_PLAYER, "BP_PlayerCharacter_C_0");
    fake.put(dormant.actor + at::ACTOR_OWNER, owner);
    fake.put_u8(dormant.actor + 0xF1, 4);
    fake.put_u8(dormant.global + 0x14, 1);
    assert_eq!(route_of(&fake, &budgets, &dormant), Branch::Weapon);
    assert_eq!((fake.u8(dormant.actor + 0xF1), fake.u8(dormant.global + 0x14)), (1, 0));
    assert_eq!(fake.f32(dormant.global + 0x88), 15_000.0, "not global: cull untouched");
    assert_eq!(
        fake.take_lines(),
        [
            "[MitRelevance] route: weapon actor=WP_IH_BEGINNER owner=BP_PlayerCharacter_C_0 dormancy=4->1 \
          wantsDormant=1->0 woke=1 cull=-2 route=spatial-dynamic"
        ]
    );
    // Always relevant (0x88, the hammer) and dormant: culled, not woken
    let hammer = registered(&fake, CLASS_WEAPON, "WP_IH_ROCK", 0x88);
    fake.put_u8(hammer.actor + 0xF1, 2);
    assert_eq!(route_of(&fake, &budgets, &hammer), Branch::Weapon);
    assert_eq!(fake.u8(hammer.actor + 0xF1), 2);
    assert_eq!(
        (fake.f32(hammer.global + 0x88), fake.f32(hammer.global + 0x8C)),
        (0.0, 0.0)
    );
    assert_eq!(fake.u16(hammer.global + 0x30), 4, "grace untouched");
    assert_eq!(
        fake.take_lines(),
        [
            "[MitRelevance] route: weapon actor=WP_IH_ROCK owner=null dormancy=2->2 wantsDormant=0->0 woke=0 cull=1 \
          route=global"
        ]
    );
    // Awake on the spatial route: untouched
    let writes = fake.writes.get();
    let awake = registered(&fake, CLASS_WEAPON, "WP_awake", 0x00);
    fake.put_u8(awake.actor + 0xF1, 1);
    assert_eq!(route_of(&fake, &budgets, &awake), Branch::Weapon);
    assert_eq!(fake.writes.get(), writes);
    // The global info unreadable: untouched
    let blind = registered(&fake, CLASS_WEAPON, "WP_blind", 0x88);
    fake.put_u8(blind.actor + 0xF1, 4);
    assert_eq!(route(&fake, &budgets, blind.info, 0x7_0000_0000), Branch::Weapon);
    assert_eq!(fake.writes.get(), writes);
    let lines = fake.take_lines();
    assert!(lines[0].ends_with("woke=0 cull=-2 route=spatial-dynamic"), "{lines:?}");
    assert!(
        lines[1].ends_with("dormancy=4->4 wantsDormant=0->0 woke=0 cull=-2 route=global"),
        "{lines:?}"
    );
}

#[test]
fn weapon_decisions_and_route_text() {
    assert_eq!(
        weapon_plan(0x00, 4, true),
        WeaponPlan {
            wake: true,
            cull: false
        }
    );
    assert_eq!(
        weapon_plan(0x00, 4, false),
        WeaponPlan {
            wake: false,
            cull: false
        }
    );
    assert_eq!(
        weapon_plan(0x00, 1, true),
        WeaponPlan {
            wake: false,
            cull: false
        }
    );
    assert_eq!(
        weapon_plan(0x04, 4, true),
        WeaponPlan {
            wake: false,
            cull: false
        }
    );
    assert_eq!(
        weapon_plan(0x88, 3, true),
        WeaponPlan {
            wake: false,
            cull: true
        }
    );
    assert_eq!(weapon_route(0x88, 1), "global");
    assert_eq!(weapon_route(0x04, 1), "owner");
    assert_eq!(weapon_route(0x00, 1), "spatial-static");
    assert_eq!(weapon_route(0x00, 0), "spatial-dynamic");
}

// ---- 7, the weapon wake

struct Player {
    connection: usize,
    pc: usize,
    pawn: usize,
    weapon: usize,
    weapon_info: usize,
    role: usize,
    manager: usize,
}

/// One player: a connection, its controller, a character with a dormant weapon, a role, and its manager.
fn player(fake: &Fake, n: usize) -> Player {
    let connection = fake.object(0xA0, CLASS_OTHER, &format!("Conn_{n}"));
    let pc = fake.object(0x420, CLASS_PC, &format!("PC_{n}"));
    let pawn = fake.object(0x748, CLASS_BP_PLAYER, &format!("BP_PlayerCharacter_C_{n}"));
    let weapon = fake.object(0x300, CLASS_WEAPON, &format!("WP_{n}"));
    let role = fake.object(0x100, CLASS_ROLE, &format!("Role_{n}"));
    fake.put(connection + rg::CONNECTION_PLAYER_CONTROLLER, pc);
    fake.put(pc + at::PLAYER_CONTROLLER_PAWN, pawn);
    fake.put(pawn + at::CHARACTER_WEAPON, weapon);
    fake.put(pawn + at::CHARACTER_PLAYER_ROLE, role);
    fake.put_u8(weapon + 0xF1, 4);
    let weapon_info = fake.info(0.0, 0.0, 4);
    fake.put_u8(weapon_info + 0x14, 1);
    let manager = manager(fake, connection, &[]);
    Player {
        connection,
        pc,
        pawn,
        weapon,
        weapon_info,
        role,
        manager,
    }
}

/// Files every actor's global info in the graph's map and the managers in its connections.
fn file(fake: &Fake, players: &[&Player], extra: &[(usize, usize)]) {
    let mut entries: Vec<(usize, usize)> = players.iter().map(|p| (p.weapon, p.weapon_info)).collect();
    entries.extend_from_slice(extra);
    fake.map(GRAPH + at::GRAPH_GLOBAL_ACTOR_MAP, &entries, 0x18);
    let managers: Vec<usize> = players.iter().map(|p| p.manager).collect();
    fake.set_connections(&managers);
}

fn run_wake(fake: &Fake, state: &mut State, budgets: &Budgets, connections: &[usize]) {
    wake(fake, state, budgets, connections, &|| Some(GRAPH));
}

#[test]
fn wake_once_per_weapon_and_throttled() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let mut state = State::new();
    let a = player(&fake, 0);
    file(&fake, &[&a], &[]);
    run_wake(&fake, &mut state, &budgets, &[a.connection]);
    assert_eq!(*fake.wakes.borrow(), [a.weapon]);
    assert_eq!(
        fake.take_lines(),
        [format!(
            "[MitRelevance] weapon: woke weapon=WP_0 pawn=BP_PlayerCharacter_C_0 connection=0x{:X} dormancy=4->1 \
             wantsDormant=1->1 info=none call=made",
            a.connection
        )]
    );
    // Asleep again: within 250 ms nothing runs, after it the ring holds the weapon
    fake.put_u8(a.weapon + 0xF1, 4);
    fake.now.set(fake.now.get() + 249);
    run_wake(&fake, &mut state, &budgets, &[a.connection]);
    fake.now.set(fake.now.get() + 1);
    run_wake(&fake, &mut state, &budgets, &[a.connection]);
    assert_eq!(fake.wakes.borrow().len(), 1);
    // No graph: nothing
    let mut fresh = State::new();
    wake(&fake, &mut fresh, &budgets, &[a.connection], &|| None);
    assert_eq!(fake.wakes.borrow().len(), 1);
}

#[test]
fn wake_clears_the_cull_of_a_global_weapon() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let mut state = State::new();
    let a = player(&fake, 0);
    fake.put_u8(a.weapon + 0x58, 0x88);
    fake.put_u8(a.weapon + 0xF1, 1);
    fake.put_f32(a.weapon_info + 0x88, 15_000.0);
    fake.put_f32(a.weapon_info + 0x8C, 225_000_000.0);
    // Its own connection's info is unsent; another's holds a stale cull snapshot
    let unsent = conn_info(&fake, 15_000.0, 225_000_000.0);
    let own = manager(&fake, a.connection, &[(a.weapon, unsent)]);
    let stale = conn_info(&fake, 15_000.0, 225_000_000.0);
    let other = manager(&fake, 0xC9, &[(a.weapon, stale)]);
    fake.map(GRAPH + at::GRAPH_GLOBAL_ACTOR_MAP, &[(a.weapon, a.weapon_info)], 0x18);
    fake.set_connections(&[own, other]);
    run_wake(&fake, &mut state, &budgets, &[a.connection]);
    assert!(fake.wakes.borrow().is_empty(), "awake: no call");
    assert_eq!(fake.f32(a.weapon_info + 0x8C), 0.0);
    assert_eq!((fake.f32(stale + 8), fake.f32(unsent + 0xC)), (0.0, 0.0));
    assert_eq!(
        fake.take_lines(),
        [format!(
            "[MitRelevance] weapon: cull cleared weapon=WP_0 pawn=BP_PlayerCharacter_C_0 connection=0x{:X} \
             flags58=0x88 global=1 snapshotsCleared=2 info=unsent",
            a.connection
        )]
    );
}

#[test]
fn wake_skips() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let run = |connection: usize| {
        let mut state = State::new();
        run_wake(&fake, &mut state, &budgets, &[connection]);
        fake.wakes.borrow_mut().drain(..).count()
    };
    let p = (0..10).map(|n| player(&fake, n)).collect::<Vec<_>>();
    let all: Vec<&Player> = p.iter().collect();
    file(&fake, &all[..9], &[]);
    // No pawn
    fake.put(p[0].pc + at::PLAYER_CONTROLLER_PAWN, 0);
    assert_eq!(run(p[0].connection), 0);
    // Not a character
    fake.put(p[1].pawn + 0x10, CLASS_OTHER);
    assert_eq!(run(p[1].connection), 0);
    // No weapon
    fake.put(p[2].pawn + at::CHARACTER_WEAPON, 0);
    assert_eq!(run(p[2].connection), 0);
    // A destroying weapon
    fake.dying.borrow_mut().insert(p[3].weapon);
    assert_eq!(run(p[3].connection), 0);
    // Nothing to correct: awake and not global
    fake.put_u8(p[4].weapon + 0xF1, 1);
    assert_eq!(run(p[4].connection), 0);
    // No manager
    fake.put(p[5].manager + rg::CONN_MANAGER_CONNECTION, 0);
    assert_eq!(run(p[5].connection), 0);
    // Already sent, by channel and by frame
    let by_channel = fake.fresh(at::CONN_INFO_SIZE);
    fake.put(by_channel, 0xC4A2);
    fake.map(
        p[6].manager + at::CONN_MANAGER_ACTOR_INFO_MAP,
        &[(p[6].weapon, by_channel)],
        0x20,
    );
    assert_eq!(run(p[6].connection), 0);
    let by_frame = fake.fresh(at::CONN_INFO_SIZE);
    fake.put_i32(by_frame + at::CONN_INFO_LAST_REP_FRAME, 17);
    fake.map(
        p[7].manager + at::CONN_MANAGER_ACTOR_INFO_MAP,
        &[(p[7].weapon, by_frame)],
        0x20,
    );
    assert_eq!(run(p[7].connection), 0);
    // A dead connection
    fake.live.borrow_mut().remove(&p[8].connection);
    assert_eq!(run(p[8].connection), 0);
    // No global info: not registered with the graph yet
    assert_eq!(run(p[9].connection), 0);
    fake.live.borrow_mut().insert(p[8].connection);
    assert_eq!(run(p[8].connection), 1, "the control case wakes");
}

#[test]
fn the_ring_wraps_and_knows_reused_addresses() {
    let mut ring = Ring::new();
    for n in 0..16 {
        ring.record(0x1000 + n * 0x10, 1);
    }
    assert!(ring.holds(0x1000, 1));
    ring.record(0x9000, 2);
    assert!(!ring.holds(0x1000, 1), "the oldest overwritten");
    assert!(ring.holds(0x9000, 2));
    assert!(!ring.holds(0x9000, 3), "another index at the same address is new");
    assert!(!ring.holds(0, 0));

    let fake = Fake::new();
    let budgets = Budgets::new();
    let mut state = State::new();
    let a = player(&fake, 0);
    file(&fake, &[&a], &[]);
    run_wake(&fake, &mut state, &budgets, &[a.connection]);
    // The same address, another object: woken again
    fake.put_i32(a.weapon + 0x0C, 999);
    fake.put_u8(a.weapon + 0xF1, 4);
    fake.now.set(fake.now.get() + 250);
    run_wake(&fake, &mut state, &budgets, &[a.connection]);
    assert_eq!(fake.wakes.borrow().len(), 2);
}

#[test]
fn wake_call_texts() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    for (answer, text) in [(WakeCall::NotFound, "not found"), (WakeCall::Skipped, "skipped")] {
        let mut state = State::new();
        let a = player(&fake, 0);
        file(&fake, &[&a], &[]);
        fake.wake_answer.set(answer);
        run_wake(&fake, &mut state, &budgets, &[a.connection]);
        let lines = fake.take_lines();
        assert!(
            lines[0].ends_with(&format!("dormancy=4->4 wantsDormant=1->1 info=none call={text}")),
            "{lines:?}"
        );
    }
}

// ---- 8, the pawn rate write

#[test]
fn pawn_rates_on_player_characters() {
    let fake = Fake::new();
    let a = player(&fake, 0);
    let b = player(&fake, 1);
    fake.put(b.pawn + 0x10, CLASS_CHARACTER);
    let c = player(&fake, 2);
    fake.live.borrow_mut().remove(&c.pc);
    pawn_rates(&fake, &[a.connection, b.connection, c.connection, 0]);
    assert_eq!((fake.f32(a.pawn + 0x108), fake.f32(a.pawn + 0x10C)), (60.0, 30.0));
    assert_eq!(fake.f32(b.pawn + 0x108), 0.0, "not a BP_PlayerCharacter_C");
    assert_eq!(fake.f32(c.pawn + 0x108), 0.0, "a dead controller");
    assert!(fake.take_lines().is_empty());
    // The handler skips it when the flag was present
    let mut state = State::new();
    let budgets = Budgets::new();
    fake.put_f32(a.pawn + 0x108, 10.0);
    tick(&fake, &mut state, &budgets, &[a.connection], &|| None, false);
    assert_eq!(fake.f32(a.pawn + 0x108), 10.0);
    tick(&fake, &mut state, &budgets, &[a.connection], &|| None, true);
    assert_eq!(fake.f32(a.pawn + 0x108), 60.0);
}

// ---- 9, the relevance pass

/// Two players with registered pawns and roles, everything already right.
fn pass_world(fake: &Fake) -> (Player, Player, usize, usize, usize, usize) {
    let a = player(fake, 0);
    let b = player(fake, 1);
    let (pa, pb) = (fake.info(1.0, 1.0, 300), fake.info(1.0, 1.0, 300));
    let (ra, rb) = (fake.info(0.0, 0.0, 300), fake.info(0.0, 0.0, 300));
    file(
        fake,
        &[&a, &b],
        &[(a.pawn, pa), (b.pawn, pb), (a.role, ra), (b.role, rb)],
    );
    (a, b, pa, pb, ra, rb)
}

fn run_pass(fake: &Fake, state: &mut State, budgets: &Budgets, connections: &[usize]) {
    pass(fake, state, budgets, connections, &|| Some(GRAPH));
}

#[test]
fn pass_cadence() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let mut state = State::new();
    let (a, b, pa, _, _, _) = pass_world(&fake);
    let connections = [a.connection, b.connection];
    run_pass(&fake, &mut state, &budgets, &connections);
    assert_eq!(fake.take_lines(), [PASS_STARTED]);
    let first = state.pass_ran_ms;
    // Same inputs within 15 s: skipped, even with a setting reset
    fake.put_u16(pa + 0x30, 4);
    fake.now.set(fake.now.get() + 1000);
    run_pass(&fake, &mut state, &budgets, &connections);
    assert_eq!(state.pass_ran_ms, first);
    // At most once a second
    fake.put(b.pawn + at::CHARACTER_PLAYER_ROLE, 0);
    fake.now.set(fake.now.get() + 999);
    run_pass(&fake, &mut state, &budgets, &connections);
    assert_eq!(state.pass_ran_ms, first);
    // A changed triple runs
    fake.now.set(fake.now.get() + 1);
    run_pass(&fake, &mut state, &budgets, &connections);
    assert_ne!(state.pass_ran_ms, first);
    assert_eq!(
        fake.take_lines(),
        [format!(
            "[MitRelevance] pawn grace: corrected pawn=0x{:X} connection=0x{:X} grace=4->300",
            a.pawn, a.connection
        )]
    );
    // 15 s with the same inputs runs
    let ran = state.pass_ran_ms;
    fake.now.set(fake.now.get() + 14_000);
    run_pass(&fake, &mut state, &budgets, &connections);
    assert_eq!(state.pass_ran_ms, ran);
    fake.now.set(fake.now.get() + 1_000);
    run_pass(&fake, &mut state, &budgets, &connections);
    assert_ne!(state.pass_ran_ms, ran);
    // A changed graph runs
    let ran = state.pass_ran_ms;
    fake.now.set(fake.now.get() + 1_000);
    pass(&fake, &mut state, &budgets, &connections, &|| Some(GRAPH + 0x8));
    assert_ne!(state.pass_ran_ms, ran);
    // No graph: nothing
    let mut fresh = State::new();
    pass(&fake, &mut fresh, &budgets, &connections, &|| None);
    assert_eq!(fresh.pass_ran_ms, None);
}

#[test]
fn pass_roles() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let mut state = State::new();
    let (a, b, _, _, ra, rb) = pass_world(&fake);
    // Both connections draw the same role: handled once
    fake.put(b.pawn + at::CHARACTER_PLAYER_ROLE, a.role);
    fake.put_f32(ra + 0x88, 15_000.0);
    fake.put_u16(ra + 0x30, 4);
    let stale = conn_info(&fake, 15_000.0, 225_000_000.0);
    fake.map(a.manager + at::CONN_MANAGER_ACTOR_INFO_MAP, &[(a.role, stale)], 0x20);
    run_pass(&fake, &mut state, &budgets, &[a.connection, b.connection]);
    assert_eq!(
        fake.take_lines(),
        [
            PASS_STARTED.to_owned(),
            format!(
                "[MitRelevance] role cull: corrected role=0x{:X} pawn=0x{:X} global=1 snapshotsCleared=1 \
                 cull=15000->0 cullSq=0->0 grace=4->300",
                a.role, a.pawn
            )
        ]
    );
    // A snapshot alone makes the line; a dead, destroying or non-role pointer is skipped
    let mut state = State::new();
    fake.put(b.pawn + at::CHARACTER_PLAYER_ROLE, b.role);
    fake.put_f32(stale + 8, 1.0);
    run_pass(&fake, &mut state, &budgets, &[a.connection, b.connection]);
    let lines = fake.take_lines();
    assert!(lines[1].contains("global=0 snapshotsCleared=1"), "{lines:?}");
    assert_eq!(lines.len(), 2);
    for bad in [0, 1, 2] {
        let mut state = State::new();
        fake.put_u16(rb + 0x30, 4);
        match bad {
            0 => {
                fake.live.borrow_mut().remove(&b.role);
            }
            1 => {
                fake.live.borrow_mut().insert(b.role);
                fake.dying.borrow_mut().insert(b.role);
            }
            _ => {
                fake.dying.borrow_mut().remove(&b.role);
                fake.put(b.pawn + at::CHARACTER_PLAYER_ROLE, b.weapon);
            }
        }
        run_pass(&fake, &mut state, &budgets, &[b.connection]);
        assert_eq!(fake.take_lines(), [PASS_STARTED], "case {bad}");
        assert_eq!(fake.u16(rb + 0x30), 4, "case {bad}");
    }
}

#[test]
fn pass_only_characters() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    let mut state = State::new();
    let (a, _, pa, _, _, _) = pass_world(&fake);
    fake.put(a.pawn + 0x10, CLASS_OTHER);
    fake.put_u16(pa + 0x30, 4);
    run_pass(&fake, &mut state, &budgets, &[a.connection]);
    assert_eq!(fake.u16(pa + 0x30), 4);
    assert_eq!(
        state.pass_inputs,
        Some((
            GRAPH,
            vec![Triple {
                connection: a.connection,
                pawn: 0,
                role: 0
            }]
        ))
    );
}

// ---- 4.3, lines and budgets

#[test]
fn line_texts() {
    assert_eq!(addr(0), "null");
    assert_eq!(addr(0xabc), "0xABC");
    assert_eq!(byte(0x8), "0x08");
    assert_eq!(name(0, Some("x".into())), "null");
    assert_eq!(name(1, None), "unreadable");
    assert_eq!(
        suppressed(kinds::WEAPON),
        "[MitRelevance] route: weapon: further lines suppressed"
    );
    let settings = Settings {
        result: 0,
        before: (1.4, 2.6, 7),
        after: (1.4, 2.6, 7),
    };
    assert_eq!(
        route_character(0x10, "C", &settings),
        "[MitRelevance] route: character actor=0x10 class=C settings=0 grace=7->7"
    );
    assert_eq!(
        role_cull(0x10, 0, &settings, 0),
        "[MitRelevance] role cull: corrected role=0x10 pawn=null global=0 snapshotsCleared=0 cull=1->1 cullSq=3->3 \
         grace=7->7"
    );
    assert_eq!(
        weapon_cull_cleared("W", "P", 0, 0x88, -1, 0, false),
        "[MitRelevance] weapon: cull cleared weapon=W pawn=P connection=null flags58=0x88 global=-1 \
         snapshotsCleared=0 info=none"
    );
    assert_eq!(
        weapon_woke("W", "P", 0x20, (3, 1), (1, 0), true, WakeCall::Made),
        "[MitRelevance] weapon: woke weapon=W pawn=P connection=0x20 dormancy=3->1 wantsDormant=1->0 info=unsent \
         call=made"
    );
    assert_eq!(
        pawn_grace(0x10, 0x20, &settings),
        "[MitRelevance] pawn grace: corrected pawn=0x10 connection=0x20 grace=7->7"
    );
}

#[test]
fn route_budgets() {
    let fake = Fake::new();
    let budgets = Budgets::new();
    for n in 0..14 {
        let ps = registered(&fake, CLASS_PS, &format!("PS_{n}"), 0x04);
        route_of(&fake, &budgets, &ps);
    }
    let lines = fake.take_lines();
    assert_eq!(lines.len(), 13);
    assert_eq!(
        lines[12],
        "[MitRelevance] route: player-state: further lines suppressed"
    );
    for n in 0..70 {
        let pc = registered(&fake, CLASS_PC, &format!("PC_{n}"), 0x08);
        route_of(&fake, &budgets, &pc);
    }
    let lines = fake.take_lines();
    assert_eq!(lines.len(), 65);
    assert_eq!(lines[64], "[MitRelevance] route: controller: further lines suppressed");
}

#[test]
fn flags_and_classes() {
    assert_eq!(flags::DISABLE_PLAYER_REP_BOOST, "DISABLE_PLAYER_REP_BOOST");
    assert!(!flags::DROPPED.contains(&"PLAYER_ALWAYS_RELEVANT"), "R11's");
    assert!(!flags::DROPPED.contains(&"DISABLE_OWNER_PAWN_RELEVANCY"), "R20's");
    assert!(flags::DROPPED.contains(&"DISABLE_PLAYER_ROLE_GLOBAL_ROUTE"));
    assert_eq!(TICK_ORDER, 400);
    const { assert!(TICK_ORDER < crate::repgraph::DRIVE_ORDER) };
    for (i, class) in Class::ALL.iter().enumerate() {
        assert_eq!(class.index(), i);
    }
    assert_eq!(PLAYER_CHANNEL_GRACE_FRAMES, 300);
}
