//! The engine reads R0 needs (R0 5.3, 6.2): the chunked object array, the name pool and object full names, all
//! through checked reads so a bad pointer is `None`, never a fault.

use crate::memory::Memory;
use crate::names;
use crate::offsets::layout;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Names by comparison index; they never change in a process.
pub type NameCache = Mutex<HashMap<u32, Arc<str>>>;

/// The deepest outer chain walked (a cycle is a corrupt object).
pub const MAX_OUTERS: usize = 64;

/// The engine of one process: its memory and the absolute addresses of `GObjects` and `GNames`.
pub struct Engine<'m> {
    memory: &'m dyn Memory,
    objects: usize,
    names: usize,
    cache: Option<&'m NameCache>,
}

impl<'m> Engine<'m> {
    /// `objects` is the `FUObjectArray`, `names` the `FNamePool` (image base + the data RVAs).
    pub fn new(memory: &'m dyn Memory, objects: usize, names: usize, cache: Option<&'m NameCache>) -> Self {
        Self {
            memory,
            objects,
            names,
            cache,
        }
    }

    fn at(base: usize, offset: usize) -> Option<usize> {
        base.checked_add(offset)
    }

    /// The element count, bounded by `MAX_OBJECTS`.
    pub fn object_count(&self) -> Option<i32> {
        let count = self.memory.i32_at(Self::at(self.objects, layout::OBJECTS_COUNT)?)?;
        (0..=layout::MAX_OBJECTS).contains(&count).then_some(count)
    }

    /// `object_at(i)`: `0 <= i < count`, its chunk below the chunk count, both pointers readable.
    pub fn object_at(&self, index: i32) -> Option<usize> {
        let count = self.object_count()?;
        if index < 0 || index >= count {
            return None;
        }
        let index = usize::try_from(index).ok()?;
        let chunk = index / layout::OBJECTS_PER_CHUNK;
        let chunks = usize::try_from(
            self.memory
                .i32_at(Self::at(self.objects, layout::OBJECTS_CHUNK_COUNT)?)?,
        )
        .ok()?;
        if chunk >= chunks {
            return None;
        }
        let table = self.memory.ptr_at(Self::at(self.objects, layout::OBJECTS_CHUNKS)?)?;
        if !self.memory.readable(table, (chunk + 1) * 8) {
            return None;
        }
        let chunk_items = self.memory.ptr_at(Self::at(table, chunk * 8)?)?;
        let item = Self::at(
            chunk_items,
            (index % layout::OBJECTS_PER_CHUNK) * layout::OBJECT_ITEM_SIZE,
        )?;
        if !self.memory.readable(item, layout::OBJECT_ITEM_SIZE) {
            return None;
        }
        self.memory.ptr_at(item).filter(|&object| object != 0)
    }

    /// `is_live_object`: the object's index round-trips through the object array.
    pub fn is_live_object(&self, ptr: usize) -> bool {
        if ptr == 0 || !self.memory.readable(ptr, layout::UOBJECT_SIZE) {
            return false;
        }
        let Some(index) = Self::at(ptr, layout::UOBJECT_INDEX).and_then(|at| self.memory.i32_at(at)) else {
            return false;
        };
        self.object_at(index) == Some(ptr)
    }

    /// `is_actor_alive_for_replication` (R11 to R13 use it).
    pub fn is_actor_alive_for_replication(&self, ptr: usize) -> bool {
        if !self.memory.readable(ptr, layout::ACTOR_SIZE) || !self.is_live_object(ptr) {
            return false;
        }
        let flags = Self::at(ptr, layout::UOBJECT_FLAGS).and_then(|at| self.memory.u32_at(at));
        let actor = Self::at(ptr, layout::ACTOR_FLAGS_BYTE).and_then(|at| self.memory.u8_at(at));
        match (flags, actor) {
            (Some(flags), Some(actor)) => {
                flags & (layout::RF_BEGIN_DESTROYED | layout::RF_FINISH_DESTROYED) == 0
                    && actor & layout::ACTOR_BEING_DESTROYED == 0
            }
            _ => false,
        }
    }

    /// The text of name pool entry `index` (no number), decoded as the exporter does.
    pub fn decode_name(&self, index: u32) -> Option<Arc<str>> {
        if let Some(cache) = self.cache
            && let Ok(cache) = cache.lock()
            && let Some(text) = cache.get(&index)
        {
            return Some(Arc::clone(text));
        }
        let block = index >> layout::NAME_OFFSET_BITS;
        if block >= layout::NAME_BLOCKS {
            return None;
        }
        let offset = usize::try_from(index & ((1 << layout::NAME_OFFSET_BITS) - 1)).ok()? * layout::NAME_STRIDE;
        let blocks = Self::at(self.names, layout::NAME_POOL_BLOCKS)?;
        let base = self
            .memory
            .ptr_at(Self::at(blocks, usize::try_from(block).ok()? * 8)?)?;
        if base == 0 {
            return None;
        }
        let entry = Self::at(base, offset)?;
        // Header: bIsWide:1, probe hash:5, length:10
        let header = self.memory.u16_at(entry)?;
        let wide = header & 1 != 0;
        let len = usize::from(header >> 6);
        let text: String = if wide {
            let mut bytes = vec![0u8; len * 2];
            if !self.memory.read(Self::at(entry, 2)?, &mut bytes) {
                return None;
            }
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            let mut bytes = vec![0u8; len];
            if !self.memory.read(Self::at(entry, 2)?, &mut bytes) {
                return None;
            }
            bytes.iter().map(|&byte| char::from(byte)).collect()
        };
        let text: Arc<str> = Arc::from(text);
        if let Some(cache) = self.cache
            && let Ok(mut cache) = cache.lock()
        {
            cache.insert(index, Arc::clone(&text));
        }
        Some(text)
    }

    /// The `FName` at `addr` (comparison index, number), raw form with its number.
    pub fn fname_at(&self, addr: usize) -> Option<String> {
        let index = u32::try_from(self.memory.i32_at(addr)?).ok()?;
        let number = self.memory.u32_at(Self::at(addr, 4)?)?;
        let text = self.decode_name(index)?;
        Some(names::with_number(&text, number))
    }

    /// An object's name, short form.
    pub fn object_name(&self, object: usize) -> Option<String> {
        if object == 0 || !self.memory.readable(object, layout::UOBJECT_SIZE) {
            return None;
        }
        let raw = self.fname_at(Self::at(object, layout::UOBJECT_NAME)?)?;
        Some(names::short(&raw).to_owned())
    }

    /// The object's outer pointer.
    pub fn outer_of(&self, object: usize) -> Option<usize> {
        self.memory.ptr_at(Self::at(object, layout::UOBJECT_OUTER)?)
    }

    /// `<class short name> <outer chain>.<name>` (R0 6.2); `None` for null or anything unreadable.
    pub fn full_name(&self, object: usize) -> Option<String> {
        let name = self.object_name(object)?;
        let class = self.memory.ptr_at(Self::at(object, layout::UOBJECT_CLASS)?)?;
        let class_name = self.object_name(class)?;
        let mut outers = Vec::new();
        let mut outer = self.outer_of(object)?;
        while outer != 0 {
            if outers.len() == MAX_OUTERS {
                return None;
            }
            outers.push(self.object_name(outer)?);
            outer = self.outer_of(outer)?;
        }
        outers.reverse();
        Some(names::full_name(&class_name, &outers, &name))
    }

    /// The self-check: object 0 is `Package CoreUObject`. `Err` carries what it was instead.
    pub fn self_check(&self) -> Result<(), Option<String>> {
        let found = self.object_at(0).and_then(|object| self.full_name(object));
        if found.as_deref() == Some(names::SELF_CHECK) {
            Ok(())
        } else {
            Err(found)
        }
    }
}

/// `sane_pointer_array(data, num, max, limit)` (R0 5.3).
pub fn sane_pointer_array(memory: &dyn Memory, data: usize, num: i32, max: i32, limit: i32) -> bool {
    if num < 0 || num > max || num > limit {
        return false;
    }
    num == 0 || usize::try_from(num).is_ok_and(|num| num.checked_mul(8).is_some_and(|size| memory.readable(data, size)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::BufferMemory;

    const BASE: usize = 0x10_0000;
    const OBJECTS: usize = BASE;
    const NAMES: usize = BASE + 0x100;
    const CHUNK_TABLE: usize = BASE + 0x200;
    const CHUNK: usize = BASE + 0x300;
    const BLOCK: usize = BASE + 0x800;
    const OBJ: usize = BASE + 0x1000;

    /// A tiny engine: object 0 the `/Script/CoreUObject` package, a class `Package`, a class `Function`, a
    /// package `/Script/Engine`, a class `PlayerController` in it, and its function `ClientRestart`.
    struct World {
        bytes: Vec<u8>,
        name_cursor: usize,
    }

    impl World {
        fn new() -> Self {
            Self {
                bytes: vec![0u8; 0x3000],
                name_cursor: 0,
            }
        }
        fn put(&mut self, at: usize, value: &[u8]) {
            let at = at - BASE;
            self.bytes[at..at + value.len()].copy_from_slice(value);
        }
        fn put_ptr(&mut self, at: usize, value: usize) {
            self.put(at, &(value as u64).to_le_bytes());
        }
        fn name(&mut self, text: &str, wide: bool) -> u32 {
            let offset = self.name_cursor;
            let len = if wide { text.encode_utf16().count() } else { text.len() };
            let header = (u16::try_from(len).unwrap() << 6) | u16::from(wide);
            self.put(BLOCK + offset, &header.to_le_bytes());
            let body: Vec<u8> = if wide {
                text.encode_utf16().flat_map(u16::to_le_bytes).collect()
            } else {
                text.as_bytes().to_vec()
            };
            self.put(BLOCK + offset + 2, &body);
            self.name_cursor += (2 + body.len()).div_ceil(2) * 2;
            u32::try_from(offset / 2).unwrap()
        }
        fn object(&mut self, slot: usize, index: i32, class: usize, name: u32, number: u32, outer: usize) -> usize {
            let at = OBJ + slot * 0x80;
            self.put(at + 0x0C, &index.to_le_bytes());
            self.put_ptr(at + 0x10, class);
            self.put(at + 0x18, &name.to_le_bytes());
            self.put(at + 0x1C, &number.to_le_bytes());
            self.put_ptr(at + 0x20, outer);
            self.put_ptr(CHUNK + usize::try_from(index).unwrap() * 0x18, at);
            at
        }
        fn build() -> (Self, [usize; 6]) {
            let mut world = Self::new();
            world.put_ptr(OBJECTS, CHUNK_TABLE);
            world.put(OBJECTS + 0x14, &6i32.to_le_bytes());
            world.put(OBJECTS + 0x1C, &1i32.to_le_bytes());
            world.put_ptr(CHUNK_TABLE, CHUNK);
            world.put_ptr(NAMES + 0x10, BLOCK);
            let core = world.name("/Script/CoreUObject", false);
            let package = world.name("Package", false);
            let function = world.name("Function", true);
            let engine = world.name("/Script/Engine", false);
            let controller = world.name("PlayerController", false);
            let restart = world.name("ClientRestart", false);
            let class_package = OBJ + 0x80;
            let o0 = world.object(0, 0, class_package, core, 0, 0);
            let o1 = world.object(1, 1, class_package, package, 0, 0);
            let o2 = world.object(2, 2, class_package, function, 0, 0);
            let o3 = world.object(3, 3, class_package, engine, 0, 0);
            let o4 = world.object(4, 4, class_package, controller, 0, o3);
            let o5 = world.object(5, 5, o2, restart, 0, o4);
            (world, [o0, o1, o2, o3, o4, o5])
        }
    }

    #[test]
    fn full_names_and_self_check() {
        let (world, objects) = World::build();
        let memory = BufferMemory {
            base: BASE,
            bytes: &world.bytes,
        };
        let cache = NameCache::default();
        let engine = Engine::new(&memory, OBJECTS, NAMES, Some(&cache));
        assert_eq!(engine.self_check(), Ok(()));
        assert_eq!(
            engine.full_name(objects[5]).as_deref(),
            Some("Function Engine.PlayerController.ClientRestart")
        );
        assert_eq!(engine.full_name(0), None);
        assert!(!cache.lock().unwrap().is_empty());
    }

    #[test]
    fn numbers_and_liveness() {
        let (mut world, objects) = World::build();
        world.put(objects[4] + 0x1C, &3u32.to_le_bytes());
        // A recycled object: slot 5's index now says 2
        world.put(objects[5] + 0x0C, &2i32.to_le_bytes());
        let memory = BufferMemory {
            base: BASE,
            bytes: &world.bytes,
        };
        let engine = Engine::new(&memory, OBJECTS, NAMES, None);
        assert_eq!(engine.object_name(objects[4]).as_deref(), Some("PlayerController_2"));
        assert!(engine.is_live_object(objects[4]));
        assert!(!engine.is_live_object(objects[5]));
        assert!(!engine.is_live_object(objects[4] + 8));
        assert_eq!(engine.object_at(6), None);
        assert_eq!(engine.object_at(-1), None);
    }

    #[test]
    fn actor_flags() {
        let (mut world, objects) = World::build();
        let memory = BufferMemory {
            base: BASE,
            bytes: &world.bytes,
        };
        assert!(Engine::new(&memory, OBJECTS, NAMES, None).is_actor_alive_for_replication(objects[4]));
        world.put(objects[4] + 0x5C, &[0x08]);
        let memory = BufferMemory {
            base: BASE,
            bytes: &world.bytes,
        };
        assert!(!Engine::new(&memory, OBJECTS, NAMES, None).is_actor_alive_for_replication(objects[4]));
        world.put(objects[4] + 0x5C, &[0]);
        world.put(objects[4] + 0x08, &0x8000u32.to_le_bytes());
        let memory = BufferMemory {
            base: BASE,
            bytes: &world.bytes,
        };
        assert!(!Engine::new(&memory, OBJECTS, NAMES, None).is_actor_alive_for_replication(objects[4]));
    }

    #[test]
    fn self_check_fails_closed() {
        let (mut world, objects) = World::build();
        world.put_ptr(objects[0] + 0x10, objects[2]);
        let memory = BufferMemory {
            base: BASE,
            bytes: &world.bytes,
        };
        let engine = Engine::new(&memory, OBJECTS, NAMES, None);
        assert_eq!(engine.self_check(), Err(Some("Function CoreUObject".into())));
        let empty = vec![0u8; 0x100];
        let memory = BufferMemory {
            base: BASE,
            bytes: &empty,
        };
        assert_eq!(Engine::new(&memory, OBJECTS, NAMES, None).self_check(), Err(None));
    }

    #[test]
    fn pointer_arrays() {
        let bytes = vec![0u8; 64];
        let memory = BufferMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert!(sane_pointer_array(&memory, BASE, 8, 8, 8));
        assert!(!sane_pointer_array(&memory, BASE, 9, 9, 9));
        assert!(sane_pointer_array(&memory, 0, 0, 0, 0));
        assert!(!sane_pointer_array(&memory, BASE, 2, 1, 8));
        assert!(!sane_pointer_array(&memory, BASE, 2, 8, 1));
        assert!(!sane_pointer_array(&memory, BASE, -1, 8, 8));
    }
}
