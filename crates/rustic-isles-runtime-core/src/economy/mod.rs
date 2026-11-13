//! R15, inventory and economy (spec-runtime-r15): supplies without a stack cap (patch 0024), the cell fusion time
//! (0029) and the loadout slot repair (0027, 0037). The pure half: the settings, the walks over checked memory,
//! the repair's bookkeeping and the log texts. The DLL scans the live engine, writes and calls the game.

pub mod catalog;
pub mod fusion;
pub mod loadout;
pub mod settings;
pub mod text;

use crate::memory::Memory;
use crate::offsets::{layout, r15};
use crate::reflect::{class_chain, class_of};
use std::collections::HashMap;

/// The longest `FString` read, in UTF-16 units.
pub const MAX_STRING_UNITS: i32 = 0x1_0000;

/// An `i64` at `addr`.
pub fn i64_at(memory: &dyn Memory, addr: usize) -> Option<i64> {
    let mut bytes = [0u8; 8];
    memory.read(addr, &mut bytes).then(|| i64::from_le_bytes(bytes))
}

/// A `TArray`'s data pointer and `Num`.
pub fn array_at(memory: &dyn Memory, addr: usize) -> Option<(usize, i32)> {
    let data = memory.ptr_at(addr.checked_add(r15::ARRAY_DATA)?)?;
    let num = memory.i32_at(addr.checked_add(r15::ARRAY_NUM)?)?;
    Some((data, num))
}

/// An `FString`'s UTF-16 units without the terminating NUL; `None` when unreadable or implausibly long.
pub fn fstring_units(memory: &dyn Memory, addr: usize) -> Option<Vec<u16>> {
    let (data, num) = array_at(memory, addr)?;
    if !(0..=MAX_STRING_UNITS).contains(&num) {
        return None;
    }
    let len = usize::try_from(num).ok()?.saturating_sub(1);
    if len == 0 {
        return Some(Vec::new());
    }
    let mut bytes = vec![0u8; len * 2];
    if !memory.read(data, &mut bytes) {
        return None;
    }
    Some(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect(),
    )
}

/// An `FString`'s text (lossy).
pub fn fstring(memory: &dyn Memory, addr: usize) -> Option<String> {
    fstring_units(memory, addr).map(|units| String::from_utf16_lossy(&units))
}

/// The allocated elements of the `TMap` at `addr`, in storage order: `None` when the map cannot be read or holds
/// more than `limit` slots.
pub fn map_elements(memory: &dyn Memory, addr: usize, stride: usize, limit: i32) -> Option<Vec<usize>> {
    let (data, num) = array_at(memory, addr)?;
    if num < 0 || num > limit {
        return None;
    }
    let num = usize::try_from(num).ok()?;
    if num == 0 {
        return Some(Vec::new());
    }
    let words = num.div_ceil(32);
    let heap = memory.ptr_at(addr.checked_add(r15::MAP_BITS_HEAP)?)?;
    let bits_at = if heap != 0 {
        heap
    } else if words <= r15::MAP_BITS_INLINE_WORDS {
        addr.checked_add(r15::MAP_BITS_INLINE)?
    } else {
        return None;
    };
    let mut bits = vec![0u8; words * 4];
    if !memory.read(bits_at, &mut bits) {
        return None;
    }
    (0..num)
        .filter(|&i| bits[i / 8] >> (i % 8) & 1 != 0)
        .map(|i| i.checked_mul(stride).and_then(|offset| data.checked_add(offset)))
        .collect()
}

/// `is_a(object, class)` (7.2): the object's class or one of its supers is `class`; a failed read is false.
pub fn is_a(memory: &dyn Memory, object: usize, class: usize) -> bool {
    class != 0 && class_of(memory, object).is_some_and(|own| class_chain(memory, own).contains(&class))
}

/// Whether `class` (a `UStruct`) is `target` or derives from it, cached per class for one scan.
pub struct ClassTest {
    target: usize,
    known: HashMap<usize, bool>,
}

impl ClassTest {
    /// A test for objects of `target`.
    pub fn new(target: usize) -> Self {
        Self {
            target,
            known: HashMap::new(),
        }
    }

    /// `is_a(object, target)`, the chain walked once per class.
    pub fn object(&mut self, memory: &dyn Memory, object: usize) -> bool {
        let Some(class) = class_of(memory, object) else {
            return false;
        };
        *self
            .known
            .entry(class)
            .or_insert_with(|| class_chain(memory, class).contains(&self.target))
    }
}

/// Every non-null object in the object array at `objects` (`FUObjectArray`), by index from 0, until `visit` says
/// stop. The chunk table is read once; every item read is checked.
pub fn for_each_object(memory: &dyn Memory, objects: usize, mut visit: impl FnMut(usize) -> bool) {
    let Some(count) = objects
        .checked_add(layout::OBJECTS_COUNT)
        .and_then(|at| memory.i32_at(at))
    else {
        return;
    };
    if !(0..=layout::MAX_OBJECTS).contains(&count) {
        return;
    }
    let (Some(chunks), Some(table)) = (
        objects
            .checked_add(layout::OBJECTS_CHUNK_COUNT)
            .and_then(|at| memory.i32_at(at)),
        objects
            .checked_add(layout::OBJECTS_CHUNKS)
            .and_then(|at| memory.ptr_at(at)),
    ) else {
        return;
    };
    let (Ok(count), Ok(chunks)) = (usize::try_from(count), usize::try_from(chunks)) else {
        return;
    };
    for index in 0..count {
        let chunk = index / layout::OBJECTS_PER_CHUNK;
        if chunk >= chunks {
            return;
        }
        let Some(items) = table.checked_add(chunk * 8).and_then(|at| memory.ptr_at(at)) else {
            return;
        };
        let item = (index % layout::OBJECTS_PER_CHUNK)
            .checked_mul(layout::OBJECT_ITEM_SIZE)
            .and_then(|offset| items.checked_add(offset));
        let Some(object) = item.and_then(|at| memory.ptr_at(at)) else {
            continue;
        };
        if object != 0 && !visit(object) {
            return;
        }
    }
}

/// Whether the object is a class default object.
pub fn is_default_object(memory: &dyn Memory, object: usize) -> bool {
    object
        .checked_add(layout::UOBJECT_FLAGS)
        .and_then(|at| memory.u32_at(at))
        .is_none_or(|flags| flags & r15::RF_CLASS_DEFAULT_OBJECT != 0)
}

/// The first object, by index, that is not a class default object, is a `class`, and passes `also`.
pub fn find_instance(
    memory: &dyn Memory,
    objects: usize,
    class: usize,
    mut also: impl FnMut(usize) -> bool,
) -> Option<usize> {
    let mut test = ClassTest::new(class);
    let mut found = None;
    for_each_object(memory, objects, |object| {
        if !is_default_object(memory, object) && test.object(memory, object) && also(object) {
            found = Some(object);
            return false;
        }
        true
    });
    found
}

/// One pass that finds objects by full name (7.2 call): the short name (`short_name`) is compared first and the
/// full name (`full_name`, R0's format) built only for those. `wanted` is `(short, full)`; answers in its order.
pub fn find_named(
    memory: &dyn Memory,
    objects: usize,
    wanted: &[(&str, &str)],
    short_name: &dyn Fn(usize) -> Option<String>,
    full_name: &dyn Fn(usize) -> Option<String>,
) -> Vec<Option<usize>> {
    let mut found: Vec<Option<usize>> = vec![None; wanted.len()];
    if wanted.is_empty() {
        return found;
    }
    for_each_object(memory, objects, |object| {
        let Some(short) = short_name(object) else {
            return true;
        };
        for (at, (want_short, want_full)) in wanted.iter().enumerate() {
            if found[at].is_none() && short == *want_short && full_name(object).as_deref() == Some(*want_full) {
                found[at] = Some(object);
            }
        }
        found.iter().any(Option::is_none)
    });
    found
}

/// The image's `SizeOfImage` from its PE headers (PE32+ optional header `+0x38`).
pub fn image_size(memory: &dyn Memory, base: usize) -> Option<usize> {
    if memory.u16_at(base)? != u16::from_le_bytes(*b"MZ") {
        return None;
    }
    let lfanew = usize::try_from(memory.i32_at(base.checked_add(0x3C)?)?).ok()?;
    let nt = base.checked_add(lfanew)?;
    if memory.u32_at(nt)? != u32::from_le_bytes(*b"PE\0\0") {
        return None;
    }
    usize::try_from(memory.u32_at(nt.checked_add(0x18 + 0x38)?)?).ok()
}

/// A run timer: due at once, then at the time the last run set when it started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timer {
    due: Option<u64>,
}

impl Timer {
    /// Due at once.
    pub const fn new() -> Self {
        Self { due: None }
    }

    /// Whether a run is due at `now`; a due run sets the next due time to `now + period` before anything else.
    pub fn start(&mut self, now: u64, period: u64) -> bool {
        if self.due.is_some_and(|due| now < due) {
            return false;
        }
        self.due = Some(now.saturating_add(period));
        true
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use crate::memory::Memory;
    use std::cell::RefCell;

    /// Memory at `base` that tests lay structures into and the walks write through; `holes` are unreadable.
    pub struct Fake {
        pub base: usize,
        pub bytes: RefCell<Vec<u8>>,
        pub holes: RefCell<Vec<(usize, usize)>>,
        cursor: RefCell<usize>,
    }

    impl Fake {
        pub fn new(base: usize, size: usize) -> Self {
            Self {
                base,
                bytes: RefCell::new(vec![0; size]),
                holes: RefCell::new(Vec::new()),
                cursor: RefCell::new(0x100),
            }
        }

        /// A zeroed, 16-aligned block.
        pub fn alloc(&self, size: usize) -> usize {
            let mut cursor = self.cursor.borrow_mut();
            let at = *cursor;
            *cursor = (at + size + 15) & !15;
            assert!(*cursor <= self.bytes.borrow().len(), "fake memory full");
            self.base + at
        }

        pub fn put(&self, at: usize, value: &[u8]) {
            let at = at - self.base;
            self.bytes.borrow_mut()[at..at + value.len()].copy_from_slice(value);
        }
        pub fn ptr(&self, at: usize, value: usize) {
            self.put(at, &(value as u64).to_le_bytes());
        }
        pub fn i32(&self, at: usize, value: i32) {
            self.put(at, &value.to_le_bytes());
        }
        pub fn u32(&self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        pub fn i64(&self, at: usize, value: i64) {
            self.put(at, &value.to_le_bytes());
        }
        pub fn get_i32(&self, at: usize) -> i32 {
            self.i32_at(at).unwrap()
        }
        pub fn get_i64(&self, at: usize) -> i64 {
            super::i64_at(self, at).unwrap()
        }
        /// An `FString` at `at` holding `text` (with its NUL).
        pub fn fstring(&self, at: usize, text: &str) {
            let mut units: Vec<u16> = text.encode_utf16().collect();
            units.push(0);
            let data = self.alloc(units.len() * 2);
            let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
            self.put(data, &bytes);
            self.ptr(at, data);
            self.i32(at + 8, i32::try_from(units.len()).unwrap());
            self.i32(at + 12, i32::try_from(units.len()).unwrap());
        }
        pub fn hole(&self, at: usize, len: usize) {
            self.holes.borrow_mut().push((at, at + len));
        }
        fn open(&self, addr: usize, len: usize) -> bool {
            let Some(end) = addr.checked_add(len) else {
                return false;
            };
            addr >= self.base
                && end <= self.base + self.bytes.borrow().len()
                && !self.holes.borrow().iter().any(|&(from, to)| addr < to && end > from)
        }
        /// The checked write the walks are given.
        pub fn write(&self, addr: usize, value: &[u8]) -> bool {
            if !self.open(addr, value.len()) {
                return false;
            }
            self.put(addr, value);
            true
        }
    }

    impl Memory for Fake {
        fn readable(&self, ptr: usize, size: usize) -> bool {
            ptr != 0 && ptr.is_multiple_of(8) && self.open(ptr, size)
        }
        fn read(&self, addr: usize, out: &mut [u8]) -> bool {
            if !self.open(addr, out.len()) {
                return false;
            }
            let at = addr - self.base;
            out.copy_from_slice(&self.bytes.borrow()[at..at + out.len()]);
            true
        }
    }

    /// A `TMap` at `map` with `slots` element slots of `stride`; `live` marks which are allocated. `heap` puts the
    /// bits on the heap. Returns each slot's address.
    pub fn map(fake: &Fake, map: usize, stride: usize, live: &[bool], heap: bool) -> Vec<usize> {
        let data = fake.alloc(stride * live.len().max(1));
        fake.ptr(map, data);
        fake.i32(map + 8, i32::try_from(live.len()).unwrap());
        fake.i32(map + 12, i32::try_from(live.len()).unwrap());
        let words = live.len().div_ceil(32).max(1);
        let mut bits = vec![0u8; words * 4];
        for (i, &on) in live.iter().enumerate() {
            if on {
                bits[i / 8] |= 1 << (i % 8);
            }
        }
        if heap {
            let at = fake.alloc(bits.len());
            fake.put(at, &bits);
            fake.ptr(map + 0x20, at);
        } else {
            assert!(words <= 4);
            fake.put(map + 0x10, &bits);
        }
        fake.i32(
            map + 0x34,
            i32::try_from(live.iter().filter(|&&on| !on).count()).unwrap(),
        );
        (0..live.len()).map(|i| data + i * stride).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Fake, map};
    use super::*;

    const BASE: usize = 0x40_0000;

    #[test]
    fn strings_and_maps() {
        let fake = Fake::new(BASE, 0x4000);
        let at = fake.alloc(0x10);
        fake.fstring(at, "supplies");
        assert_eq!(fstring(&fake, at).as_deref(), Some("supplies"));
        let empty = fake.alloc(0x10);
        assert_eq!(fstring(&fake, empty).as_deref(), Some(""));
        fake.i32(empty + 8, -1);
        assert_eq!(fstring(&fake, empty), None);
        let m = fake.alloc(0x50);
        let slots = map(&fake, m, 0x18, &[true, false, true], false);
        assert_eq!(map_elements(&fake, m, 0x18, 10), Some(vec![slots[0], slots[2]]));
        assert_eq!(map_elements(&fake, m, 0x18, 2), None);
        let big = fake.alloc(0x50);
        let mut live = vec![false; 200];
        live[150] = true;
        let slots = map(&fake, big, 0x18, &live, true);
        assert_eq!(map_elements(&fake, big, 0x18, 1000), Some(vec![slots[150]]));
        // More than the inline words with no heap: unreadable, not a read past the map
        fake.ptr(big + 0x20, 0);
        assert_eq!(map_elements(&fake, big, 0x18, 1000), None);
    }

    #[test]
    fn is_a_chain() {
        let fake = Fake::new(BASE, 0x4000);
        let base_class = fake.alloc(0x50);
        let mid = fake.alloc(0x50);
        let leaf = fake.alloc(0x50);
        let other = fake.alloc(0x50);
        fake.ptr(mid + 0x40, base_class);
        fake.ptr(leaf + 0x40, mid);
        let object = fake.alloc(0x30);
        fake.ptr(object + 0x10, leaf);
        assert!(is_a(&fake, object, leaf));
        assert!(is_a(&fake, object, mid));
        assert!(is_a(&fake, object, base_class));
        assert!(!is_a(&fake, object, other));
        assert!(ClassTest::new(base_class).object(&fake, object));
        assert!(!ClassTest::new(other).object(&fake, object));
        // A cycle stops
        fake.ptr(base_class + 0x40, leaf);
        assert!(!is_a(&fake, object, other));
        assert!(!ClassTest::new(other).object(&fake, object));
        // An unreadable link is false
        fake.ptr(base_class + 0x40, 0);
        fake.ptr(mid + 0x40, BASE + 0x10_0000);
        assert!(!is_a(&fake, object, base_class));
        assert!(!ClassTest::new(base_class).object(&fake, object));
        assert!(!is_a(&fake, 0, base_class));
    }

    #[test]
    fn timer() {
        let mut timer = Timer::default();
        assert!(timer.start(100, 1000));
        assert!(!timer.start(1099, 1000));
        assert!(timer.start(1100, 1000));
        assert!(!timer.start(2099, 1000));
    }

    #[test]
    fn image_size_from_headers() {
        let fake = Fake::new(BASE, 0x1000);
        fake.put(BASE, b"MZ");
        fake.i32(BASE + 0x3C, 0x80);
        fake.put(BASE + 0x80, b"PE\0\0");
        fake.u32(BASE + 0x80 + 0x50, 0x0700_0000);
        assert_eq!(image_size(&fake, BASE), Some(0x0700_0000));
        fake.put(BASE, b"ZM");
        assert_eq!(image_size(&fake, BASE), None);
    }
}
