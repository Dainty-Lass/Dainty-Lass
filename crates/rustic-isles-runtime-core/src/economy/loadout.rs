//! Loadout slot repair (section 6, patches 0027 and 0037): the collect walk and the repair's bookkeeping. The DLL
//! re-runs the collected slots' notifications through `ProcessEvent`.

use super::{Timer, array_at, is_a, text};
use crate::memory::Memory;
use crate::offsets::r15;
use std::collections::HashMap;

/// Slot data entries walked.
pub const MAX_SLOT_DATA: usize = 8;
/// Slots collected in one pass.
pub const MAX_COLLECTED: usize = 64;
/// Attempts per item slot.
pub const MAX_ATTEMPTS: u32 = 3;
/// The attempts map is cleared past this many entries.
pub const MAX_TRACKED: usize = 512;
/// The time between passes.
pub const PERIOD_MS: u64 = 1000;
/// The eight item slots of a slot data entry, in order, by their log names.
pub const SLOT_NAMES: [&str; 8] = [
    "role",
    "weapon",
    "subweapon",
    "helmet",
    "chest",
    "legs",
    "arms",
    "lantern",
];

/// One item slot to repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// The `UArchonLoadoutItemSlot`.
    pub slot: usize,
    /// Its slot data index.
    pub data_index: usize,
    /// Its position (an index into `SLOT_NAMES`).
    pub position: usize,
    /// Its `ItemInstanceIdCrc`.
    pub crc: u32,
}

/// What one collect found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    /// The loadout, 0 when none was reached.
    pub loadout: usize,
    /// Slots with a CRC whose inventory is missing or still loading.
    pub waiting: usize,
    /// Slots to repair, in walk order.
    pub slots: Vec<Slot>,
}

/// The engine answers the collect needs.
pub struct Checks<'a> {
    /// `AArchonPlayerController`'s class.
    pub controller_class: usize,
    /// R0's `is_live_object`.
    pub live: &'a dyn Fn(usize) -> bool,
    /// The game's loading evaluator, past every wrapper on LD-1.
    pub loaded: &'a dyn Fn(usize) -> bool,
}

fn pointer_field(memory: &dyn Memory, object: usize, offset: usize) -> Option<usize> {
    object.checked_add(offset).and_then(|at| memory.ptr_at(at))
}

fn object_ok(memory: &dyn Memory, checks: &Checks, object: usize, size: usize) -> bool {
    object != 0 && memory.readable(object, size) && (checks.live)(object)
}

/// The collect (6.3 step 3) from the local controller; any failure above the item slots ends it with what it has.
pub fn collect(memory: &dyn Memory, controller: usize, checks: &Checks) -> Collected {
    let mut found = Collected::default();
    if !object_ok(memory, checks, controller, r15::CONTROLLER_SIZE)
        || !is_a(memory, controller, checks.controller_class)
    {
        return found;
    }
    let Some(loadout) = pointer_field(memory, controller, r15::CONTROLLER_LOADOUT) else {
        return found;
    };
    if !object_ok(memory, checks, loadout, r15::LOADOUT_SIZE) {
        return found;
    }
    found.loadout = loadout;
    let Some((data, num)) = loadout
        .checked_add(r15::LOADOUT_SLOT_DATA)
        .and_then(|at| array_at(memory, at))
    else {
        return found;
    };
    let count = usize::try_from(num).unwrap_or(0).min(MAX_SLOT_DATA);
    for data_index in 0..count {
        let Some(slot_data) = data.checked_add(data_index * 8).and_then(|at| memory.ptr_at(at)) else {
            return found;
        };
        if !object_ok(memory, checks, slot_data, r15::SLOT_DATA_SIZE) {
            return found;
        }
        for position in 0..SLOT_NAMES.len() {
            let Some(slot) = pointer_field(memory, slot_data, r15::SLOT_DATA_ITEM_SLOTS + position * 8) else {
                return found;
            };
            if !object_ok(memory, checks, slot, r15::ITEM_SLOT_SIZE) {
                continue;
            }
            let crc = slot.checked_add(r15::ITEM_SLOT_CRC).and_then(|at| memory.u32_at(at));
            let id_num = slot.checked_add(r15::ITEM_SLOT_ID_NUM).and_then(|at| memory.i32_at(at));
            let (Some(crc), Some(id_num)) = (crc, id_num) else {
                return found;
            };
            if crc == 0 || id_num > 0 {
                continue;
            }
            let inventory = pointer_field(memory, slot, r15::ITEM_SLOT_INVENTORY).unwrap_or(0);
            if !object_ok(memory, checks, inventory, r15::INVENTORY_SIZE) || !(checks.loaded)(inventory) {
                found.waiting += 1;
                continue;
            }
            found.slots.push(Slot {
                slot,
                data_index,
                position,
                crc,
            });
            if found.slots.len() == MAX_COLLECTED {
                return found;
            }
        }
    }
    found
}

/// Whether a slot knows its item now: still readable and its id `Num > 0`.
pub fn resolved(memory: &dyn Memory, slot: usize) -> bool {
    memory.readable(slot, r15::ITEM_SLOT_SIZE)
        && slot
            .checked_add(r15::ITEM_SLOT_ID_NUM)
            .and_then(|at| memory.i32_at(at))
            .is_some_and(|num| num > 0)
}

/// The local controller R15 captures at `ClientRestart` (6.2, departure 4), with the world it was captured in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Controller {
    controller: usize,
    world: usize,
}

impl Controller {
    /// None captured yet.
    pub const fn new() -> Self {
        Self {
            controller: 0,
            world: 0,
        }
    }

    /// `ClientRestart` on a non-null object.
    pub fn capture(&mut self, controller: usize, world: usize) {
        if controller != 0 {
            *self = Controller { controller, world };
        }
    }

    /// The controller, forgotten when the world changed since its capture.
    pub fn current(&mut self, world: usize) -> usize {
        if world != self.world {
            *self = Controller::default();
        }
        self.controller
    }
}

/// The repair's state between passes.
#[derive(Debug, Default)]
pub struct Repair {
    timer: Timer,
    logged_for: Option<usize>,
    attempts_for: usize,
    attempts: HashMap<usize, u32>,
}

impl Repair {
    /// Whether a pass may run at `now` (1000 ms after the last one started).
    pub fn due(&mut self, now: u64) -> bool {
        self.timer.start(now, PERIOD_MS)
    }

    /// The waiting line, once per loadout.
    pub fn waiting_line(&mut self, collected: &Collected) -> Option<String> {
        if collected.waiting == 0 || self.logged_for == Some(collected.loadout) {
            return None;
        }
        self.logged_for = Some(collected.loadout);
        Some(text::waiting(collected.loadout, collected.waiting))
    }

    /// Before the slots of `loadout` are tried: a new loadout or an overfull map starts over.
    pub fn begin(&mut self, loadout: usize) {
        if loadout != self.attempts_for || self.attempts.len() > MAX_TRACKED {
            self.attempts.clear();
            self.attempts_for = loadout;
        }
    }

    /// Adds an attempt for `slot` and returns its number, `None` when it already had three.
    pub fn attempt(&mut self, slot: usize) -> Option<u32> {
        let attempts = self.attempts.entry(slot).or_insert(0);
        if *attempts >= MAX_ATTEMPTS {
            return None;
        }
        *attempts += 1;
        Some(*attempts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::fake::Fake;
    use std::cell::RefCell;
    use std::collections::HashSet;

    const BASE: usize = 0x100_0000;

    struct Graph {
        fake: Fake,
        class: usize,
        controller: usize,
        loadout: usize,
        dead: RefCell<HashSet<usize>>,
        unloaded: RefCell<HashSet<usize>>,
    }

    impl Graph {
        fn new(slot_data: usize) -> Self {
            let fake = Fake::new(BASE, 0x80000);
            let class = fake.alloc(0x50);
            let controller = fake.alloc(r15::CONTROLLER_SIZE);
            fake.ptr(controller + 0x10, class);
            let loadout = fake.alloc(r15::LOADOUT_SIZE);
            fake.ptr(controller + r15::CONTROLLER_LOADOUT, loadout);
            let array = fake.alloc(slot_data.max(1) * 8);
            fake.ptr(loadout + r15::LOADOUT_SLOT_DATA, array);
            fake.i32(loadout + r15::LOADOUT_SLOT_DATA + 8, i32::try_from(slot_data).unwrap());
            for i in 0..slot_data {
                let data = fake.alloc(r15::SLOT_DATA_SIZE);
                fake.ptr(array + i * 8, data);
            }
            Graph {
                fake,
                class,
                controller,
                loadout,
                dead: RefCell::default(),
                unloaded: RefCell::default(),
            }
        }

        fn slot_data(&self, index: usize) -> usize {
            let array = self.fake.ptr_at(self.loadout + r15::LOADOUT_SLOT_DATA).unwrap();
            self.fake.ptr_at(array + index * 8).unwrap()
        }

        /// An item slot at `position` of slot data `index`, with a CRC, an id length and an inventory.
        fn slot(&self, index: usize, position: usize, crc: u32, id_num: i32, inventory: bool) -> usize {
            let slot = self.fake.alloc(r15::ITEM_SLOT_SIZE);
            self.fake
                .ptr(self.slot_data(index) + r15::SLOT_DATA_ITEM_SLOTS + position * 8, slot);
            self.fake.u32(slot + r15::ITEM_SLOT_CRC, crc);
            self.fake.i32(slot + r15::ITEM_SLOT_ID_NUM, id_num);
            if inventory {
                let inventory = self.fake.alloc(r15::INVENTORY_SIZE);
                self.fake.ptr(slot + r15::ITEM_SLOT_INVENTORY, inventory);
            }
            slot
        }

        fn collect(&self) -> Collected {
            let live = |object: usize| !self.dead.borrow().contains(&object);
            let loaded = |inventory: usize| !self.unloaded.borrow().contains(&inventory);
            collect(
                &self.fake,
                self.controller,
                &Checks {
                    controller_class: self.class,
                    live: &live,
                    loaded: &loaded,
                },
            )
        }
    }

    #[test]
    fn collect_walk() {
        let graph = Graph::new(2);
        graph.slot(0, 0, 0, 0, true);
        graph.slot(0, 1, 0x11, 5, true);
        graph.slot(0, 2, 0x22, 0, false);
        let unloaded = graph.slot(0, 3, 0x33, 0, true);
        graph
            .unloaded
            .borrow_mut()
            .insert(graph.fake.ptr_at(unloaded + r15::ITEM_SLOT_INVENTORY).unwrap());
        let a = graph.slot(0, 4, 0x44, 0, true);
        let dead = graph.slot(1, 0, 0x55, 0, true);
        graph.dead.borrow_mut().insert(dead);
        let b = graph.slot(1, 7, 0x77, 0, true);
        let found = graph.collect();
        assert_eq!(found.loadout, graph.loadout);
        assert_eq!(found.waiting, 2);
        assert_eq!(
            found.slots,
            [
                Slot {
                    slot: a,
                    data_index: 0,
                    position: 4,
                    crc: 0x44
                },
                Slot {
                    slot: b,
                    data_index: 1,
                    position: 7,
                    crc: 0x77
                },
            ]
        );
        assert!(!resolved(&graph.fake, a));
        graph.fake.i32(a + r15::ITEM_SLOT_ID_NUM, 37);
        assert!(resolved(&graph.fake, a));
        // A controller of another class, or a dead one, collects nothing
        graph.dead.borrow_mut().insert(graph.controller);
        assert_eq!(graph.collect(), Collected::default());
        graph.dead.borrow_mut().clear();
        graph.fake.ptr(graph.controller + 0x10, graph.loadout);
        assert_eq!(graph.collect(), Collected::default());
    }

    #[test]
    fn only_eight_slot_data_and_sixty_four_slots() {
        let graph = Graph::new(9);
        for index in 0..9 {
            for position in 0..8 {
                graph.slot(
                    index,
                    position,
                    0x100 + u32::try_from(index * 8 + position).unwrap(),
                    0,
                    true,
                );
            }
        }
        let found = graph.collect();
        assert_eq!(found.slots.len(), MAX_COLLECTED);
        assert!(found.slots.iter().all(|slot| slot.data_index < 8));
        assert_eq!(found.slots[63].crc, 0x100 + 63);
    }

    #[test]
    fn bookkeeping() {
        let mut repair = Repair::default();
        assert!(repair.due(5000));
        assert!(!repair.due(5999));
        assert!(repair.due(6000));
        let collected = Collected {
            loadout: 10,
            waiting: 2,
            slots: vec![],
        };
        assert_eq!(repair.waiting_line(&collected), Some(text::waiting(10, 2)));
        assert_eq!(repair.waiting_line(&collected), None, "once per loadout");
        assert_eq!(
            repair.waiting_line(&Collected {
                loadout: 11,
                ..collected.clone()
            }),
            Some(text::waiting(11, 2))
        );
        assert_eq!(
            repair.waiting_line(&Collected {
                waiting: 0,
                loadout: 12,
                slots: vec![]
            }),
            None
        );
        repair.begin(10);
        assert_eq!(repair.attempt(0xA0), Some(1));
        assert_eq!(repair.attempt(0xA0), Some(2));
        assert_eq!(repair.attempt(0xA0), Some(3));
        assert_eq!(repair.attempt(0xA0), None, "three attempts, not forever");
        repair.begin(10);
        assert_eq!(repair.attempt(0xA0), None);
        repair.begin(20);
        assert_eq!(repair.attempt(0xA0), Some(1), "a new loadout starts over");
        for slot in 0..=MAX_TRACKED {
            repair.attempt(0x1000 + slot);
        }
        repair.attempt(0xA0);
        repair.begin(20);
        assert_eq!(repair.attempt(0xA0), Some(1), "past 512 entries the map is cleared");
    }

    #[test]
    fn controller_forgotten_with_its_world() {
        let mut controller = Controller::default();
        assert_eq!(controller.current(0), 0);
        controller.capture(0xC0, 0xD0);
        controller.capture(0, 0xE0);
        assert_eq!(controller.current(0xD0), 0xC0);
        assert_eq!(controller.current(0xE0), 0, "a new world forgets it");
        assert_eq!(controller.current(0xD0), 0);
        controller.capture(0xC8, 0xE0);
        assert_eq!(controller.current(0xE0), 0xC8);
    }
}
