//! `FWeakObjectPtr` resolution over checked reads (spec-runtime-r6 3.4): the 1.14.7 engine's own `Get`, so a dead or
//! reused object index is `None`, never a fault.

use crate::engine::Engine;
use crate::offsets::{layout, r6};

/// The object a weak pointer `(index, serial)` names, while its item is alive and still carries that serial.
pub fn resolve_weak(engine: &Engine, index: i32, serial: i32) -> Option<usize> {
    if serial == 0 || index < 0 || index >= engine.object_count()? {
        return None;
    }
    let memory = engine.memory();
    let objects = engine.objects();
    let slot = usize::try_from(index).ok()?;
    let chunk = slot / layout::OBJECTS_PER_CHUNK;
    let chunks = usize::try_from(memory.i32_at(objects.checked_add(layout::OBJECTS_CHUNK_COUNT)?)?).ok()?;
    if chunk >= chunks {
        return None;
    }
    let table = memory.ptr_at(objects.checked_add(layout::OBJECTS_CHUNKS)?)?;
    let items = memory.ptr_at(table.checked_add(chunk.checked_mul(8)?)?)?;
    let item = items.checked_add((slot % layout::OBJECTS_PER_CHUNK).checked_mul(layout::OBJECT_ITEM_SIZE)?)?;
    if !memory.readable(item, layout::OBJECT_ITEM_SIZE) {
        return None;
    }
    if memory.i32_at(item.checked_add(r6::ITEM_SERIAL)?)? != serial {
        return None;
    }
    let flags = memory.u32_at(item.checked_add(r6::ITEM_FLAGS)?)?;
    if flags & r6::ITEM_DEAD != 0 {
        return None;
    }
    memory.ptr_at(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::BufferMemory;

    const BASE: usize = 0x40_0000;
    const OBJECTS: usize = BASE;
    const TABLE: usize = BASE + 0x100;
    const CHUNK0: usize = BASE + 0x1000;
    const CHUNK1: usize = BASE + 0x2000;

    /// Two chunks; index 3 and index 65 536 + 1 are items with serial 7.
    fn world() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x3000];
        let mut put = |at: usize, value: &[u8]| bytes[at - BASE..at - BASE + value.len()].copy_from_slice(value);
        put(OBJECTS, &(TABLE as u64).to_le_bytes());
        put(OBJECTS + layout::OBJECTS_COUNT, &(65_536i32 + 10).to_le_bytes());
        put(OBJECTS + layout::OBJECTS_CHUNK_COUNT, &2i32.to_le_bytes());
        put(TABLE, &(CHUNK0 as u64).to_le_bytes());
        put(TABLE + 8, &(CHUNK1 as u64).to_le_bytes());
        for (item, object) in [(CHUNK0 + 3 * 0x18, 0xAAA0usize), (CHUNK1 + 0x18, 0xBBB0)] {
            put(item, &(object as u64).to_le_bytes());
            put(item + r6::ITEM_SERIAL, &7i32.to_le_bytes());
        }
        bytes
    }

    fn resolve(bytes: &[u8], index: i32, serial: i32) -> Option<usize> {
        let memory = BufferMemory { base: BASE, bytes };
        resolve_weak(&Engine::new(&memory, OBJECTS, 0, None), index, serial)
    }

    #[test]
    fn weak_pointer_rules() {
        let bytes = world();
        assert_eq!(resolve(&bytes, 3, 7), Some(0xAAA0));
        assert_eq!(resolve(&bytes, 65_537, 7), Some(0xBBB0), "second chunk");
        assert_eq!(resolve(&bytes, 3, 0), None, "serial 0");
        assert_eq!(resolve(&bytes, -1, 7), None);
        assert_eq!(resolve(&bytes, 65_536 + 10, 7), None, "index at the count");
        assert_eq!(resolve(&bytes, 3, 8), None, "serial mismatch");
        for bit in [0x1000_0000u32, 0x2000_0000] {
            let mut dead = world();
            let at = CHUNK0 + 3 * 0x18 + r6::ITEM_FLAGS - BASE;
            dead[at..at + 4].copy_from_slice(&bit.to_le_bytes());
            assert_eq!(resolve(&dead, 3, 7), None, "{bit:#x}");
        }
        let mut other = world();
        let at = CHUNK0 + 3 * 0x18 + r6::ITEM_FLAGS - BASE;
        other[at..at + 4].copy_from_slice(&0x0800_0000u32.to_le_bytes());
        assert_eq!(resolve(&other, 3, 7), Some(0xAAA0), "other flags do not kill it");
    }
}
