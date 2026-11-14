//! Runtime slice R18 (spec-runtime-r18): the Trials schedule rewrite (ESC-1) and the escalation buff-choice spawn.
//! Servers only.

pub mod buff_choice;
pub mod trials;

use crate::engine::Engine;
use crate::memory::Memory;
use crate::names;
use crate::offsets::{layout, r17};
use crate::reflect::class_chain;

/// The most `Children` fields walked per class (R18 6.3).
pub const MAX_CHILDREN: usize = 4096;
/// How much of every field must be readable before it is followed (R18 6.3).
pub const FIELD_READ: usize = 0x30;

/// A pointer as the R18 log lines write it: decimal, `null` for zero (the AGPL's format, R18 call 8).
pub fn pointer(value: usize) -> String {
    if value == 0 {
        "null".to_owned()
    } else {
        value.to_string()
    }
}

/// Renders the `FName` at an address with its number suffix (R0's `fname_at`), so tests can fake the name pool.
pub trait FNames {
    /// The raw name text, `_<n-1>` appended for a non-zero number.
    fn fname_at(&self, addr: usize) -> Option<String>;
}

impl FNames for Engine<'_> {
    fn fname_at(&self, addr: usize) -> Option<String> {
        Engine::fname_at(self, addr)
    }
}

/// An object's name, short form.
pub fn object_name(memory: &dyn Memory, names: &dyn FNames, object: usize) -> Option<String> {
    if object == 0 || !memory.readable(object, layout::UOBJECT_SIZE) {
        return None;
    }
    let raw = names.fname_at(object.checked_add(layout::UOBJECT_NAME)?)?;
    Some(names::short(&raw).to_owned())
}

/// The first field named `function` whose outer is named `declaring`, in `class` and its supers (R18 6.3). The
/// outer test is what tells the native function from a same-named Blueprint one.
pub fn find_declared_function(
    memory: &dyn Memory,
    names: &dyn FNames,
    class: usize,
    function: &str,
    declaring: &str,
) -> Option<usize> {
    for class in class_chain(memory, class) {
        let mut child = class
            .checked_add(r17::USTRUCT_CHILDREN)
            .and_then(|at| memory.ptr_at(at))
            .unwrap_or(0);
        let mut walked = 0;
        while child != 0 && walked < MAX_CHILDREN && memory.readable(child, FIELD_READ) {
            walked += 1;
            if object_name(memory, names, child).as_deref() == Some(function) {
                let outer = child
                    .checked_add(layout::UOBJECT_OUTER)
                    .and_then(|at| memory.ptr_at(at))
                    .unwrap_or(0);
                if object_name(memory, names, outer).as_deref() == Some(declaring) {
                    return Some(child);
                }
            }
            child = child
                .checked_add(r17::UFIELD_NEXT)
                .and_then(|at| memory.ptr_at(at))
                .unwrap_or(0);
        }
    }
    None
}

/// Test doubles: a writable fake memory and a name source keyed by comparison index.
#[cfg(test)]
pub(crate) mod fake {
    use super::FNames;
    use crate::memory::{BufferMemory, Memory};
    use crate::names;
    use std::collections::HashMap;

    /// Bytes at `base`, written by the tests.
    pub struct World {
        pub base: usize,
        pub bytes: Vec<u8>,
        pub names: HashMap<i32, String>,
        next_name: i32,
    }

    impl World {
        pub fn new(base: usize, size: usize) -> Self {
            Self {
                base,
                bytes: vec![0; size],
                names: HashMap::new(),
                next_name: 1,
            }
        }
        pub fn put(&mut self, at: usize, value: &[u8]) {
            let at = at - self.base;
            self.bytes[at..at + value.len()].copy_from_slice(value);
        }
        pub fn put_ptr(&mut self, at: usize, value: usize) {
            self.put(at, &(value as u64).to_le_bytes());
        }
        pub fn put_i32(&mut self, at: usize, value: i32) {
            self.put(at, &value.to_le_bytes());
        }
        pub fn put_i64(&mut self, at: usize, value: i64) {
            self.put(at, &value.to_le_bytes());
        }
        pub fn i64_at(&self, at: usize) -> i64 {
            let at = at - self.base;
            i64::from_le_bytes(self.bytes[at..at + 8].try_into().unwrap())
        }
        /// Writes an `FName` of `text` with `number` at `at`.
        pub fn put_fname(&mut self, at: usize, text: &str, number: u32) {
            let index = match self.names.iter().find(|(_, known)| *known == text) {
                Some((&index, _)) => index,
                None => {
                    let index = self.next_name;
                    self.next_name += 1;
                    self.names.insert(index, text.to_owned());
                    index
                }
            };
            self.put_i32(at, index);
            self.put(at + 4, &number.to_le_bytes());
        }
        pub fn memory(&self) -> BufferMemory<'_> {
            BufferMemory {
                base: self.base,
                bytes: &self.bytes,
            }
        }
        pub fn fnames(&self) -> FakeNames<'_> {
            FakeNames { world: self }
        }
    }

    pub struct FakeNames<'a> {
        world: &'a World,
    }

    impl FNames for FakeNames<'_> {
        fn fname_at(&self, addr: usize) -> Option<String> {
            let memory = self.world.memory();
            let index = memory.i32_at(addr)?;
            let number = memory.u32_at(addr + 4)?;
            let text = self.world.names.get(&index)?;
            Some(names::with_number(text, number))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::World;
    use super::*;

    const BASE: usize = 0x20_0000;
    const SLOT: usize = 0x80;

    fn at(slot: usize) -> usize {
        BASE + slot * SLOT
    }

    /// An object in `slot` named `name`, with an outer, a super, a first child and a next field.
    fn object(world: &mut World, slot: usize, name: &str, outer: usize, sup: usize, children: usize, next: usize) {
        let object = at(slot);
        world.put_fname(object + layout::UOBJECT_NAME, name, 0);
        world.put_ptr(object + layout::UOBJECT_OUTER, outer);
        world.put_ptr(object + r17::UFIELD_NEXT, next);
        world.put_ptr(object + r17::USTRUCT_SUPER, sup);
        world.put_ptr(object + r17::USTRUCT_CHILDREN, children);
    }

    const WANTED: &str = "SpawnPlayerBuffChoicesForFinishedRound";
    const DECLARING: &str = "EscalationGameModeComponent";

    /// Slot 1 a Blueprint class, its super slot 2, whose super slot 3 declares the function (slot 5). Slot 2 has a
    /// same-named child (slot 4) declared by another class.
    fn graph() -> World {
        let mut world = World::new(BASE, SLOT * 16);
        object(&mut world, 1, "BP_Mode_C", 0, at(2), 0, 0);
        object(&mut world, 2, "ModeBase", 0, at(3), at(4), 0);
        object(&mut world, 3, DECLARING, 0, 0, at(6), 0);
        object(&mut world, 4, WANTED, at(2), 0, 0, 0);
        object(&mut world, 6, "OtherFunction", at(3), 0, 0, at(5));
        object(&mut world, 5, WANTED, at(3), 0, 0, 0);
        world
    }

    #[test]
    fn finds_two_supers_up() {
        let world = graph();
        let memory = world.memory();
        let names = world.fnames();
        assert_eq!(
            find_declared_function(&memory, &names, at(1), WANTED, DECLARING),
            Some(at(5)),
            "the same-named child of slot 2 is declared elsewhere"
        );
        assert_eq!(find_declared_function(&memory, &names, at(1), WANTED, "Nope"), None);
        assert_eq!(find_declared_function(&memory, &names, 0, WANTED, DECLARING), None);
    }

    #[test]
    fn stops_at_the_bounds() {
        // A cycle of supers ends at the chain bound, a cycle of children after 4096 fields
        let mut world = graph();
        world.put_ptr(at(3) + r17::USTRUCT_SUPER, at(1));
        world.put_ptr(at(6) + r17::UFIELD_NEXT, at(6));
        let memory = world.memory();
        assert_eq!(
            find_declared_function(&memory, &world.fnames(), at(1), WANTED, DECLARING),
            None
        );
        // An unreadable first child ends that class's walk
        let mut world = graph();
        world.put_ptr(at(3) + r17::USTRUCT_CHILDREN, BASE + 0x100_0000);
        let memory = world.memory();
        assert_eq!(
            find_declared_function(&memory, &world.fnames(), at(1), WANTED, DECLARING),
            None
        );
    }
}
