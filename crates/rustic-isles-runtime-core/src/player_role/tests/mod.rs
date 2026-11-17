//! R14's core tests (16.1) over a fake game: blocks of bytes at made-up addresses and a fake clock.

mod apply;
mod client;
mod hud;
mod lines;
mod server;
mod sheathe;

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A side effect of a fake UFunction: the object and its block.
type Effect = fn(&Fake, usize, &mut [u8]);
/// A side effect of the fake application.
type ApplyEffect = fn(&Fake, usize, bool);
/// One `try_activate`: component, handle, key, event data.
type Activation = (usize, u32, [u8; 16], usize);

/// One recorded UFunction call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub object: usize,
    pub function: String,
    pub route: Route,
    pub params: Vec<u8>,
}

pub struct Fake {
    pub state: State,
    blocks: RefCell<BTreeMap<usize, Vec<u8>>>,
    read_only: RefCell<HashSet<usize>>,
    pub live: RefCell<HashSet<usize>>,
    pub roles: RefCell<HashSet<usize>>,
    pub now: Cell<u64>,
    pub lines: RefCell<Vec<String>>,
    next: Cell<usize>,
    pub quiet: Cell<bool>,
    pub inter: RefCell<HashMap<usize, usize>>,
    pub item: RefCell<HashMap<usize, usize>>,
    pub applies: RefCell<Vec<(usize, bool)>>,
    pub apply_effect: Cell<Option<ApplyEffect>>,
    pub calls: RefCell<Vec<Call>>,
    pub missing: RefCell<HashSet<&'static str>>,
    pub effects: RefCell<HashMap<&'static str, Effect>>,
    pub ability_ready: Cell<bool>,
    pub activates: RefCell<Vec<Activation>>,
    pub activated: Cell<bool>,
    pub captured: Cell<Option<Captured>>,
    pub chain_pawn: Cell<usize>,
    pub stable: Cell<bool>,
    pub connections: Cell<Option<usize>>,
    pub replay_off: Cell<bool>,
    pub names: RefCell<HashMap<usize, String>>,
    pub loadout: Cell<usize>,
    pub slot_index: Cell<i32>,
}

impl Fake {
    pub fn new() -> Self {
        Self {
            state: State::new(),
            blocks: RefCell::default(),
            read_only: RefCell::default(),
            live: RefCell::default(),
            roles: RefCell::default(),
            now: Cell::new(100_000),
            lines: RefCell::default(),
            next: Cell::new(0x100_0000),
            quiet: Cell::new(false),
            inter: RefCell::default(),
            item: RefCell::default(),
            applies: RefCell::default(),
            apply_effect: Cell::new(None),
            calls: RefCell::default(),
            missing: RefCell::default(),
            effects: RefCell::default(),
            ability_ready: Cell::new(true),
            activates: RefCell::default(),
            activated: Cell::new(true),
            captured: Cell::new(None),
            chain_pawn: Cell::new(0),
            stable: Cell::new(false),
            connections: Cell::new(None),
            replay_off: Cell::new(false),
            names: RefCell::default(),
            loadout: Cell::new(0),
            slot_index: Cell::new(0),
        }
    }

    fn block_of(&self, addr: usize, size: usize) -> Option<(usize, usize)> {
        let blocks = self.blocks.borrow();
        let (&base, bytes) = blocks.range(..=addr).next_back()?;
        let end = addr.checked_add(size)?;
        (end <= base + bytes.len()).then_some((base, addr - base))
    }

    /// A fresh live block, 0x100-aligned, past every other.
    pub fn block(&self, size: usize) -> usize {
        let base = self.next.get();
        self.next.set(base + size.div_ceil(0x100) * 0x100 + 0x100);
        self.blocks.borrow_mut().insert(base, vec![0; size]);
        self.live.borrow_mut().insert(base);
        base
    }

    pub fn read_only(&self, base: usize) {
        self.read_only.borrow_mut().insert(base);
    }

    pub fn kill(&self, object: usize) {
        self.live.borrow_mut().remove(&object);
    }

    pub fn put_bytes(&self, addr: usize, bytes: &[u8]) {
        let (base, offset) = self.block_of(addr, bytes.len()).expect("inside a block");
        self.blocks.borrow_mut().get_mut(&base).expect("block")[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    pub fn put(&self, addr: usize, value: usize) {
        self.put_bytes(addr, &(value as u64).to_le_bytes());
    }

    pub fn put_i32(&self, addr: usize, value: i32) {
        self.put_bytes(addr, &value.to_le_bytes());
    }

    pub fn put_u32(&self, addr: usize, value: u32) {
        self.put_bytes(addr, &value.to_le_bytes());
    }

    pub fn put_u8(&self, addr: usize, value: u8) {
        self.put_bytes(addr, &[value]);
    }

    pub fn byte(&self, addr: usize) -> u8 {
        self.u8_at(addr).expect("readable")
    }

    /// An `FString` at `addr` holding `text`.
    pub fn fstring(&self, addr: usize, text: &str) {
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let data = self.block(units.len() * 2);
        let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        self.put_bytes(data, &bytes);
        self.put(addr, data);
        self.put_i32(addr + 8, i32::try_from(units.len()).expect("short"));
        self.put_i32(addr + 12, i32::try_from(units.len()).expect("short"));
    }

    pub fn advance(&self, ms: u64) {
        self.now.set(self.now.get() + ms);
    }

    pub fn take_lines(&self) -> Vec<String> {
        std::mem::take(&mut *self.lines.borrow_mut())
    }

    pub fn has_line(&self, text: &str) -> bool {
        self.lines.borrow().iter().any(|line| line.contains(text))
    }

    pub fn count_lines(&self, text: &str) -> usize {
        self.lines.borrow().iter().filter(|line| line.contains(text)).count()
    }

    pub fn calls_of(&self, function: &str) -> Vec<Call> {
        self.calls
            .borrow()
            .iter()
            .filter(|call| call.function == function)
            .cloned()
            .collect()
    }

    pub fn call_names(&self) -> Vec<String> {
        self.calls.borrow().iter().map(|call| call.function.clone()).collect()
    }

    pub fn on(&self, function: &'static str, effect: Effect) {
        self.effects.borrow_mut().insert(function, effect);
    }
}

impl Game for Fake {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        ptr.is_multiple_of(8) && self.block_of(ptr, size).is_some()
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
        self.put_bytes(addr, bytes);
        true
    }
    fn is_live_object(&self, ptr: usize) -> bool {
        self.live.borrow().contains(&ptr)
    }
    fn is_role(&self, actor: usize) -> bool {
        self.roles.borrow().contains(&actor)
    }
    fn now_ms(&self) -> u64 {
        self.now.get()
    }
    fn log(&self, line: &str) {
        self.lines.borrow_mut().push(line.to_owned());
    }
    fn quiet(&self) -> bool {
        self.quiet.get()
    }
    fn name(&self, object: usize) -> Option<String> {
        self.names.borrow().get(&object).cloned()
    }
    fn active_slot_data(&self, loadout: usize) -> usize {
        self.inter.borrow().get(&loadout).copied().unwrap_or(0)
    }
    fn slot_role_item(&self, inter: usize) -> usize {
        self.item.borrow().get(&inter).copied().unwrap_or(0)
    }
    fn apply(&self, loadout: usize, wrapper: bool) {
        self.applies.borrow_mut().push((loadout, wrapper));
        if let Some(effect) = self.apply_effect.get() {
            effect(self, loadout, wrapper);
        }
    }
    fn invoke(&self, object: usize, function: &str, route: Route, params: &mut [u8]) -> Invoke {
        if self.missing.borrow().contains(function) {
            return Invoke::NotFound;
        }
        if !self.is_live_object(object) {
            return Invoke::Refused;
        }
        self.calls.borrow_mut().push(Call {
            object,
            function: function.to_owned(),
            route,
            params: params.to_vec(),
        });
        let effect = self.effects.borrow().get(function).copied();
        if let Some(effect) = effect {
            effect(self, object, params);
        }
        Invoke::Called
    }
    fn ability_ready(&self) -> bool {
        self.ability_ready.get()
    }
    fn try_activate(&self, component: usize, handle: u32, key: &mut [u8; 16], event_data: usize) -> bool {
        self.activates.borrow_mut().push((component, handle, *key, event_data));
        *key = [0xEE; 16];
        self.activated.get()
    }
    fn captured(&self) -> Option<Captured> {
        self.captured.get()
    }
    fn chain_for(&self, pc: usize, expected_pawn: Option<usize>) -> Option<usize> {
        let captured = self.captured.get()?;
        let pawn = self.chain_pawn.get();
        (captured.pc == pc && pawn != 0 && expected_pawn.is_none_or(|expected| expected == pawn)).then_some(pawn)
    }
    fn possession_stable(&self, pc: usize) -> bool {
        self.stable.get() && self.captured.get().is_some_and(|captured| captured.pc == pc)
    }
    fn connection_count(&self) -> Option<usize> {
        self.connections.get()
    }
    fn lifecycle_replay_off(&self) -> bool {
        self.replay_off.get()
    }
}

/// A loadout with its slot data, item, controller and pawn wired as 5.1 reads them.
pub struct World {
    pub loadout: usize,
    pub inter: usize,
    pub pc: usize,
    pub pawn: usize,
    pub player_state: usize,
}

impl Fake {
    /// A role actor with `item_id`, live, an `ArchonPlayerRole`.
    pub fn role(&self, item_id: &str) -> usize {
        let role = self.block(0x300);
        self.fstring(role + crate::offsets::r14::EQUIPMENT_ITEM_ID, item_id);
        self.roles.borrow_mut().insert(role);
        role
    }

    /// The four lifecycle bytes.
    pub fn set_active(&self, role: usize, active: bool) {
        for offset in [0x2F4, 0x2F5, 0x2F6, 0x2F7] {
            self.put_u8(role + offset, u8::from(active));
        }
    }

    /// A loadout whose item is `item_id` and whose pawn's role is `role_id` (empty: none).
    pub fn world(&self, item_id: Option<&str>, role_id: Option<&str>) -> World {
        let loadout = self.block(0x700);
        let inter = self.block(0x400);
        let slot = self.block(0x40);
        let pc = self.block(0xD00);
        let pawn = self.block(0x1700);
        let player_state = self.block(0x700);
        self.inter.borrow_mut().insert(loadout, inter);
        self.put(inter + 0x348, slot);
        if let Some(id) = item_id {
            let item = self.block(0x80);
            self.fstring(item + 0x70, id);
            self.item.borrow_mut().insert(inter, item);
        }
        self.put(loadout + 0x630, pc);
        self.put(pc + 0x250, pawn);
        self.put(pawn + 0x240, player_state);
        if let Some(id) = role_id {
            let role = self.role(id);
            self.put(pawn + 0x738, role);
        }
        World {
            loadout,
            inter,
            pc,
            pawn,
            player_state,
        }
    }
}
