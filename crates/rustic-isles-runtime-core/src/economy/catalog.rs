//! Supplies without a stack cap (section 4, patch 0024): the catalog walk.

use super::{array_at, fstring, map_elements, text};
use crate::memory::Memory;
use crate::offsets::r15;

/// The most catalog slots a readable map holds.
pub const MAX_ELEMENTS: i32 = 1_000_000;
/// The most tags an item holds.
pub const MAX_TAGS: i32 = 1024;
/// The items the log names.
pub const LOGGED: usize = 8;
/// The tag that marks a supply, exactly.
pub const SUPPLIES: &str = "supplies";

/// Whether the tag array at `tags` holds exactly `supplies`; `None` when it cannot be read.
fn tagged_supplies(memory: &dyn Memory, tags: usize) -> Option<bool> {
    let (data, num) = array_at(memory, tags)?;
    if !(0..=MAX_TAGS).contains(&num) {
        return None;
    }
    for index in 0..usize::try_from(num).ok()? {
        let tag = data.checked_add(index.checked_mul(r15::FSTRING_SIZE)?)?;
        if fstring(memory, tag)? == SUPPLIES {
            return Some(true);
        }
    }
    Some(false)
}

/// Walks the catalog of `catalog` (a `UArchonCatalog`) and writes 0 to every supply's `MaxQuantity` above 1 through
/// `write_i32`. Returns the log line when at least one item was written.
pub fn uncap(memory: &dyn Memory, catalog: usize, write_i32: &mut dyn FnMut(usize, i32) -> bool) -> Option<String> {
    let map = catalog.checked_add(r15::CATALOG_MAP)?;
    let elements = map_elements(memory, map, r15::CATALOG_STRIDE, MAX_ELEMENTS)?;
    let mut written = 0usize;
    let mut list = Vec::new();
    for element in elements {
        let Some(quantity_at) = element.checked_add(r15::CATALOG_MAX_QUANTITY) else {
            continue;
        };
        let Some(quantity) = memory.i32_at(quantity_at) else {
            continue;
        };
        if quantity <= 1 {
            continue;
        }
        let tags = element.checked_add(r15::CATALOG_TAGS);
        if tags.and_then(|tags| tagged_supplies(memory, tags)) != Some(true) {
            continue;
        }
        if !write_i32(quantity_at, 0) {
            continue;
        }
        written += 1;
        if list.len() < LOGGED {
            let id = fstring(memory, element).unwrap_or_default();
            list.push(format!("{id}({quantity})"));
        }
    }
    (written > 0).then(|| text::caps_removed(written, &list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::fake::{Fake, map};

    const BASE: usize = 0x80_0000;

    struct Item {
        id: &'static str,
        cap: i32,
        tags: &'static [&'static str],
    }

    fn build(fake: &Fake, items: &[Option<Item>], heap: bool) -> (usize, Vec<usize>) {
        let catalog = fake.alloc(0x80);
        let live: Vec<bool> = items.iter().map(Option::is_some).collect();
        let slots = map(fake, catalog + r15::CATALOG_MAP, r15::CATALOG_STRIDE, &live, heap);
        for (item, &slot) in items.iter().zip(&slots) {
            let Some(item) = item else { continue };
            fake.fstring(slot, item.id);
            fake.i32(slot + r15::CATALOG_MAX_QUANTITY, item.cap);
            let tags = fake.alloc(0x10 * item.tags.len().max(1));
            for (i, tag) in item.tags.iter().enumerate() {
                fake.fstring(tags + i * 0x10, tag);
            }
            fake.ptr(slot + r15::CATALOG_TAGS, tags);
            fake.i32(slot + r15::CATALOG_TAGS + 8, i32::try_from(item.tags.len()).unwrap());
        }
        (catalog, slots)
    }

    fn walk(fake: &Fake, catalog: usize) -> (Option<String>, Vec<usize>) {
        let mut writes = Vec::new();
        let line = uncap(fake, catalog, &mut |at, value| {
            writes.push(at);
            fake.write(at, &value.to_le_bytes())
        });
        (line, writes)
    }

    #[test]
    fn only_supplies_above_one() {
        let fake = Fake::new(BASE, 0x20000);
        let items = vec![
            Some(Item {
                id: "Zero",
                cap: 0,
                tags: &["supplies"],
            }),
            None,
            Some(Item {
                id: "Flare",
                cap: 1,
                tags: &["supplies"],
            }),
            Some(Item {
                id: "Two",
                cap: 2,
                tags: &["misc", "supplies"],
            }),
            None,
            Some(Item {
                id: "Heartbreaker",
                cap: 30,
                tags: &["supplies"],
            }),
            Some(Item {
                id: "Upper",
                cap: 30,
                tags: &["Supplies"],
            }),
            Some(Item {
                id: "Spaced",
                cap: 30,
                tags: &["supplies "],
            }),
            Some(Item {
                id: "Untagged",
                cap: 30,
                tags: &[],
            }),
        ];
        for heap in [false, true] {
            let (catalog, slots) = build(&fake, &items, heap);
            let (line, _) = walk(&fake, catalog);
            assert_eq!(
                line.as_deref(),
                Some("[SupplyCap] removed the stack cap of 2 supply item(s): Two(2),Heartbreaker(30)")
            );
            let caps: Vec<i32> = [0, 2, 3, 5, 6, 7, 8]
                .iter()
                .map(|&i| fake.get_i32(slots[i] + r15::CATALOG_MAX_QUANTITY))
                .collect();
            assert_eq!(caps, [0, 1, 0, 0, 30, 30, 30]);
            assert_eq!(
                walk(&fake, catalog),
                (None, vec![]),
                "a second walk writes and logs nothing"
            );
        }
    }

    #[test]
    fn list_stops_at_eight() {
        let fake = Fake::new(BASE, 0x40000);
        let ids = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"];
        let items: Vec<Option<Item>> = ids
            .iter()
            .map(|&id| {
                Some(Item {
                    id,
                    cap: 30,
                    tags: &["supplies"],
                })
            })
            .collect();
        let (catalog, _) = build(&fake, &items, true);
        assert_eq!(
            walk(&fake, catalog).0.as_deref(),
            Some(
                "[SupplyCap] removed the stack cap of 10 supply item(s): \
                 A(30),B(30),C(30),D(30),E(30),F(30),G(30),H(30)"
            )
        );
        let fake = Fake::new(BASE, 0x20000);
        let (catalog, _) = build(&fake, &items[..3], false);
        assert_eq!(
            walk(&fake, catalog).0.as_deref(),
            Some("[SupplyCap] removed the stack cap of 3 supply item(s): A(30),B(30),C(30)")
        );
    }

    #[test]
    fn unreadable_parts_are_skipped() {
        let fake = Fake::new(BASE, 0x20000);
        let items = vec![
            Some(Item {
                id: "BadTags",
                cap: 30,
                tags: &["supplies"],
            }),
            Some(Item {
                id: "Hole",
                cap: 30,
                tags: &["supplies"],
            }),
            Some(Item {
                id: "BadTag",
                cap: 30,
                tags: &["supplies"],
            }),
            Some(Item {
                id: "Locked",
                cap: 30,
                tags: &["supplies"],
            }),
        ];
        let (catalog, slots) = build(&fake, &items, false);
        fake.i32(slots[0] + r15::CATALOG_TAGS + 8, 1025);
        fake.hole(slots[1] + r15::CATALOG_MAX_QUANTITY, 4);
        let tag = fake.ptr_at(slots[2] + r15::CATALOG_TAGS).unwrap();
        fake.i32(tag + 8, -5);
        let (line, writes) = walk(&fake, catalog);
        assert_eq!(writes, [slots[3] + r15::CATALOG_MAX_QUANTITY]);
        assert_eq!(
            line.as_deref(),
            Some("[SupplyCap] removed the stack cap of 1 supply item(s): Locked(30)")
        );
        let fake = Fake::new(BASE, 0x20000);
        let (catalog, slots) = build(&fake, &items[3..], false);
        fake.hole(slots[0] + r15::CATALOG_MAX_QUANTITY + 1, 1);
        let (line, writes) = walk(&fake, catalog);
        assert_eq!((line, writes.len()), (None, 0), "unreadable cap: no write");
        let fake = Fake::new(BASE, 0x20000);
        let (catalog, _) = build(&fake, &items[3..], false);
        let line = uncap(&fake, catalog, &mut |_, _| false);
        assert_eq!(line, None, "a refused write is not counted");
        let fake = Fake::new(BASE, 0x20000);
        let catalog = fake.alloc(0x80);
        fake.i32(catalog + r15::CATALOG_MAP + 8, MAX_ELEMENTS + 1);
        assert_eq!(walk(&fake, catalog), (None, vec![]), "1 000 001 elements: skipped");
    }
}
