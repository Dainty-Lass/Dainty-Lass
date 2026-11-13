//! The executable's `.pdata` (R0 5.4): which function an RVA belongs to, read from the image in memory.

use crate::memory::Memory;

/// The most chained unwind entries `primary_entry` follows.
pub const MAX_CHAIN: usize = 32;
const UNW_FLAG_CHAININFO: u8 = 0x4;
const RUNTIME_FUNCTION_SIZE: usize = 12;
const SECTION_SIZE: usize = 40;
const MAX_SECTIONS: u16 = 96;

/// One `RUNTIME_FUNCTION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFunction {
    /// `BeginAddress`.
    pub begin: u32,
    /// `EndAddress` (exclusive).
    pub end: u32,
    /// `UnwindData`.
    pub unwind: u32,
}

/// The `.pdata` section of an image mapped at `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pdata {
    /// The image base.
    pub base: usize,
    /// The section's RVA.
    pub rva: u32,
    /// Entries in it.
    pub count: u32,
}

impl Pdata {
    /// Parses the headers at `base` (DOS `MZ`, `e_lfanew`, `PE\0\0`, the section table) and finds `.pdata`.
    pub fn locate(memory: &dyn Memory, base: usize) -> Option<Pdata> {
        if memory.u16_at(base)? != u16::from_le_bytes(*b"MZ") {
            return None;
        }
        let lfanew = usize::try_from(memory.i32_at(base.checked_add(0x3C)?)?).ok()?;
        let nt = base.checked_add(lfanew)?;
        if memory.u32_at(nt)? != u32::from_le_bytes(*b"PE\0\0") {
            return None;
        }
        let sections = memory.u16_at(nt.checked_add(6)?)?;
        let optional = usize::from(memory.u16_at(nt.checked_add(0x14)?)?);
        let table = nt.checked_add(4 + 20)?.checked_add(optional)?;
        for index in 0..usize::from(sections.min(MAX_SECTIONS)) {
            let section = table.checked_add(index * SECTION_SIZE)?;
            let mut name = [0u8; 8];
            if !memory.read(section, &mut name) {
                return None;
            }
            if &name == b".pdata\0\0" {
                let size = memory.u32_at(section.checked_add(8)?)?;
                let rva = memory.u32_at(section.checked_add(12)?)?;
                return Some(Pdata {
                    base,
                    rva,
                    count: size / RUNTIME_FUNCTION_SIZE as u32,
                });
            }
        }
        None
    }

    fn entry(&self, memory: &dyn Memory, index: u32) -> Option<RuntimeFunction> {
        let offset = usize::try_from(self.rva)
            .ok()?
            .checked_add(usize::try_from(index).ok()? * RUNTIME_FUNCTION_SIZE)?;
        self.function_at(memory, offset)
    }

    fn function_at(&self, memory: &dyn Memory, rva: usize) -> Option<RuntimeFunction> {
        let at = self.base.checked_add(rva)?;
        Some(RuntimeFunction {
            begin: memory.u32_at(at)?,
            end: memory.u32_at(at.checked_add(4)?)?,
            unwind: memory.u32_at(at.checked_add(8)?)?,
        })
    }

    /// The entry with `begin <= rva < end`, by binary search.
    pub fn lookup(&self, memory: &dyn Memory, rva: u32) -> Option<RuntimeFunction> {
        let (mut low, mut high) = (0u32, self.count);
        while low < high {
            let middle = low + (high - low) / 2;
            let entry = self.entry(memory, middle)?;
            if rva < entry.begin {
                high = middle;
            } else if rva >= entry.end {
                low = middle + 1;
            } else {
                return Some(entry);
            }
        }
        None
    }

    /// `function_entry(rva)`: the begin of the function holding `rva`.
    pub fn function_entry(&self, memory: &dyn Memory, rva: u32) -> Option<u32> {
        self.lookup(memory, rva).map(|entry| entry.begin)
    }

    /// `primary_entry(rva)`: the same, following chained unwind info to the primary function, at most 32 steps.
    pub fn primary_entry(&self, memory: &dyn Memory, rva: u32) -> Option<u32> {
        let mut entry = self.lookup(memory, rva)?;
        for _ in 0..=MAX_CHAIN {
            let unwind = self.base.checked_add(usize::try_from(entry.unwind).ok()?)?;
            let flags = memory.u8_at(unwind)? >> 3;
            if flags & UNW_FLAG_CHAININFO == 0 {
                return Some(entry.begin);
            }
            let codes = usize::from(memory.u8_at(unwind.checked_add(2)?)?);
            let chained = usize::try_from(entry.unwind)
                .ok()?
                .checked_add(4 + codes.div_ceil(2) * 2 * 2)?;
            entry = self.function_at(memory, chained)?;
        }
        None
    }

    /// `verify_entry(rva, prologue)`: `rva` starts a function and its bytes are `prologue` (R0 5.4).
    pub fn verify_entry(&self, memory: &dyn Memory, rva: u32, prologue: &[u8]) -> bool {
        if self.function_entry(memory, rva) != Some(rva) {
            return false;
        }
        let Some(at) = usize::try_from(rva).ok().and_then(|rva| self.base.checked_add(rva)) else {
            return false;
        };
        let mut bytes = vec![0u8; prologue.len()];
        memory.read(at, &mut bytes) && bytes == prologue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::BufferMemory;

    const BASE: usize = 0x1_4000_0000;
    const PDATA: usize = 0x400;
    const UNWIND: usize = 0x800;

    fn put32(image: &mut [u8], at: usize, value: u32) {
        image[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// An image with a `.text` and a `.pdata` section holding `functions`, and unwind infos at `UNWIND`.
    fn image(functions: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut image = vec![0u8; 0x2000];
        image[0..2].copy_from_slice(b"MZ");
        put32(&mut image, 0x3C, 0x80);
        image[0x80..0x84].copy_from_slice(b"PE\0\0");
        image[0x86..0x88].copy_from_slice(&2u16.to_le_bytes());
        image[0x94..0x96].copy_from_slice(&0xF0u16.to_le_bytes());
        let table = 0x80 + 24 + 0xF0;
        image[table..table + 6].copy_from_slice(b".text\0");
        image[table + 40..table + 46].copy_from_slice(b".pdata");
        put32(&mut image, table + 40 + 8, u32::try_from(functions.len() * 12).unwrap());
        put32(&mut image, table + 40 + 12, PDATA as u32);
        for (index, &(begin, end, unwind)) in functions.iter().enumerate() {
            let at = PDATA + index * 12;
            put32(&mut image, at, begin);
            put32(&mut image, at + 4, end);
            put32(&mut image, at + 8, unwind);
        }
        image
    }

    fn pdata(memory: &BufferMemory) -> Pdata {
        Pdata::locate(memory, BASE).unwrap()
    }

    #[test]
    fn binary_search() {
        let unwind = UNWIND as u32;
        let image = image(&[
            (0x1000, 0x1010, unwind),
            (0x1010, 0x1040, unwind),
            (0x1050, 0x1060, unwind),
        ]);
        let memory = BufferMemory {
            base: BASE,
            bytes: &image,
        };
        let table = pdata(&memory);
        assert_eq!(table.count, 3);
        assert_eq!(table.function_entry(&memory, 0x1000), Some(0x1000));
        assert_eq!(table.function_entry(&memory, 0x100F), Some(0x1000));
        assert_eq!(table.function_entry(&memory, 0x1020), Some(0x1010));
        assert_eq!(table.function_entry(&memory, 0x105F), Some(0x1050), "last");
        assert_eq!(table.function_entry(&memory, 0x1045), None, "gap");
        assert_eq!(table.function_entry(&memory, 0x0FFF), None, "before the first");
        assert_eq!(table.function_entry(&memory, 0x1060), None, "after the last");
        let empty = image_empty();
        let memory = BufferMemory {
            base: BASE,
            bytes: &empty,
        };
        assert_eq!(pdata(&memory).function_entry(&memory, 0x1000), None);
    }

    fn image_empty() -> Vec<u8> {
        image(&[])
    }

    #[test]
    fn chained_unwind_followed() {
        // 0x1100..0x1120 is a chained part of 0x1000; its unwind info (2 codes) is followed by the parent entry
        let parent = UNWIND as u32;
        let child = (UNWIND + 0x20) as u32;
        let mut image = image(&[(0x1000, 0x1010, parent), (0x1100, 0x1120, child)]);
        image[UNWIND] = 1; // version 1, no flags
        image[UNWIND + 0x20] = 1 | (UNW_FLAG_CHAININFO << 3);
        image[UNWIND + 0x22] = 3; // three codes round up to four slots
        let chained = UNWIND + 0x20 + 4 + 8;
        put32(&mut image, chained, 0x1000);
        put32(&mut image, chained + 4, 0x1010);
        put32(&mut image, chained + 8, parent);
        let memory = BufferMemory {
            base: BASE,
            bytes: &image,
        };
        let table = pdata(&memory);
        assert_eq!(table.function_entry(&memory, 0x1110), Some(0x1100));
        assert_eq!(table.primary_entry(&memory, 0x1110), Some(0x1000));
        assert_eq!(table.primary_entry(&memory, 0x1004), Some(0x1000));
    }

    #[test]
    fn chain_limit() {
        // An unwind info chained to itself never ends: refused after 32 steps
        let looped = UNWIND as u32;
        let mut image = image(&[(0x1000, 0x1010, looped)]);
        image[UNWIND] = 1 | (UNW_FLAG_CHAININFO << 3);
        image[UNWIND + 2] = 0;
        put32(&mut image, UNWIND + 4, 0x1000);
        put32(&mut image, UNWIND + 8, 0x1010);
        put32(&mut image, UNWIND + 12, looped);
        let memory = BufferMemory {
            base: BASE,
            bytes: &image,
        };
        assert_eq!(pdata(&memory).primary_entry(&memory, 0x1000), None);
    }

    #[test]
    fn verify_entry_checks_start_and_bytes() {
        let mut image = image(&[(0x1000, 0x1010, UNWIND as u32)]);
        image[0x1000..0x1003].copy_from_slice(&[0x48, 0x89, 0x5C]);
        let memory = BufferMemory {
            base: BASE,
            bytes: &image,
        };
        let table = pdata(&memory);
        assert!(table.verify_entry(&memory, 0x1000, &[0x48, 0x89, 0x5C]));
        assert!(!table.verify_entry(&memory, 0x1000, &[0x48, 0x89, 0x5D]));
        assert!(!table.verify_entry(&memory, 0x1001, &[0x89]));
    }

    #[test]
    fn not_an_image() {
        let bytes = vec![0u8; 0x100];
        let memory = BufferMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(Pdata::locate(&memory, BASE), None);
    }
}
