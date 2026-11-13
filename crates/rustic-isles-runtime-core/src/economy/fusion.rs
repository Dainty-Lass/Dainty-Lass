//! Cell fusion time (section 5, patch 0029): the data table search and walk.

use super::{find_instance, i64_at, map_elements, text};
use crate::economy::settings::TICKS_PER_MINUTE;
use crate::memory::Memory;
use crate::offsets::r15;

/// The most rows a readable table holds.
pub const MAX_ROWS: i32 = 1_000_000;
/// The rows the log names.
pub const LOGGED: usize = 8;

/// The first non-default `UDataTable` (`data_table` is its class) whose row struct is named
/// `CellExchangeTableData` (`short_name`: R0's short object name).
pub fn find_table(
    memory: &dyn Memory,
    objects: usize,
    data_table: usize,
    short_name: &dyn Fn(usize) -> Option<String>,
) -> Option<usize> {
    find_instance(memory, objects, data_table, |table| {
        table
            .checked_add(r15::TABLE_ROW_STRUCT)
            .and_then(|at| memory.ptr_at(at))
            .filter(|&row_struct| row_struct != 0)
            .and_then(short_name)
            .is_some_and(|name| name == r15::CELL_EXCHANGE_ROW)
    })
}

/// Sets every row's `TimeRequired` to `minutes` (non-zero) through `write_i64`; `name_of` decodes a row's `FName`
/// (number suffix included), `table_name` is the table's object name. The log line when a row changed.
pub fn set_time(
    memory: &dyn Memory,
    table: usize,
    table_name: &str,
    minutes: u32,
    name_of: &dyn Fn(usize) -> Option<String>,
    write_i64: &mut dyn FnMut(usize, i64) -> bool,
) -> Option<String> {
    let target = i64::from(minutes) * TICKS_PER_MINUTE;
    let map = table.checked_add(r15::TABLE_ROW_MAP)?;
    let elements = map_elements(memory, map, r15::ROW_STRIDE, MAX_ROWS)?;
    let (mut rows, mut changed) = (0usize, 0usize);
    let mut list = Vec::new();
    for element in elements {
        let Some(row) = element
            .checked_add(r15::ROW_POINTER)
            .and_then(|at| memory.ptr_at(at))
            .filter(|&row| row != 0)
        else {
            continue;
        };
        rows += 1;
        let Some(time_at) = row.checked_add(r15::ROW_TIME_REQUIRED) else {
            continue;
        };
        let Some(ticks) = i64_at(memory, time_at) else {
            continue;
        };
        if ticks == target || !write_i64(time_at, target) {
            continue;
        }
        changed += 1;
        if list.len() < LOGGED {
            let name = name_of(element).unwrap_or_default();
            list.push(format!("{name}({}min)", ticks / TICKS_PER_MINUTE));
        }
    }
    (changed > 0).then(|| text::fusion_set(changed, rows, table_name, minutes, &list))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::economy::fake::{Fake, map};
    use std::collections::HashMap;

    const BASE: usize = 0xC0_0000;

    #[test]
    fn rows_set_once() {
        let fake = Fake::new(BASE, 0x10000);
        let table = fake.alloc(0x80);
        let slots = map(
            &fake,
            table + r15::TABLE_ROW_MAP,
            r15::ROW_STRIDE,
            &[true, true, false, true, true],
            false,
        );
        let mut names = HashMap::new();
        for (i, (minutes, name)) in [
            (Some(30), "Fuse_A"),
            (Some(60), "Fuse_B"),
            (None, ""),
            (None, "Null"),
            (Some(7), "Fuse_C_1"),
        ]
        .into_iter()
        .enumerate()
        {
            names.insert(slots[i], name.to_owned());
            if let Some(minutes) = minutes {
                let row = fake.alloc(0x40);
                fake.i64(row + r15::ROW_TIME_REQUIRED, minutes * TICKS_PER_MINUTE);
                fake.ptr(slots[i] + r15::ROW_POINTER, row);
            }
        }
        let name_of = |element: usize| names.get(&element).cloned();
        let mut write = |at: usize, value: i64| fake.write(at, &value.to_le_bytes());
        assert_eq!(
            set_time(&fake, table, "DT_CellExchange", 30, &name_of, &mut write).as_deref(),
            Some(
                "[CellFusion] set 2/3 fusion recipe(s) of DT_CellExchange to 30 min; before: Fuse_B(60min),Fuse_C_1(7min)"
            )
        );
        for &i in &[0, 1, 4] {
            let row = fake.ptr_at(slots[i] + r15::ROW_POINTER).unwrap();
            assert_eq!(fake.get_i64(row + r15::ROW_TIME_REQUIRED), 18_000_000_000);
        }
        assert_eq!(
            set_time(&fake, table, "DT_CellExchange", 30, &name_of, &mut write),
            None
        );
        assert_eq!(
            set_time(&fake, table, "DT", 5, &name_of, &mut write).as_deref(),
            Some(
                "[CellFusion] set 3/3 fusion recipe(s) of DT to 5 min; before: Fuse_A(30min),Fuse_B(30min),Fuse_C_1(30min)"
            )
        );
    }

    /// An object array at the returned address holding `objects`.
    pub fn object_array(fake: &Fake, objects: &[usize]) -> usize {
        let array = fake.alloc(0x20);
        let items = fake.alloc(objects.len().max(1) * 0x18);
        let table = fake.alloc(8);
        fake.ptr(table, items);
        fake.ptr(array, table);
        fake.i32(array + 0x14, i32::try_from(objects.len()).unwrap());
        fake.i32(array + 0x1C, 1);
        for (i, &object) in objects.iter().enumerate() {
            fake.ptr(items + i * 0x18, object);
        }
        array
    }

    #[test]
    fn table_search() {
        let fake = Fake::new(BASE, 0x10000);
        let object_class = fake.alloc(0x50);
        let data_table = fake.alloc(0x50);
        fake.ptr(data_table + 0x40, object_class);
        let composite = fake.alloc(0x50);
        fake.ptr(composite + 0x40, data_table);
        let cell_row = fake.alloc(0x30);
        let other_row = fake.alloc(0x30);
        let new_table = |class: usize, flags: u32, row: usize| {
            let table = fake.alloc(0x80);
            fake.ptr(table + 0x10, class);
            fake.u32(table + 0x08, flags);
            fake.ptr(table + r15::TABLE_ROW_STRUCT, row);
            table
        };
        let default = new_table(data_table, r15::RF_CLASS_DEFAULT_OBJECT, cell_row);
        let other = new_table(data_table, 0, other_row);
        let unrelated = new_table(object_class, 0, cell_row);
        let null_row = new_table(data_table, 0, 0);
        let wanted = new_table(composite, 0, cell_row);
        let second = new_table(data_table, 0, cell_row);
        let objects = object_array(&fake, &[default, other, unrelated, null_row, wanted, second]);
        let short = |object: usize| {
            Some(
                if object == cell_row {
                    "CellExchangeTableData"
                } else {
                    "OtherRow"
                }
                .to_owned(),
            )
        };
        assert_eq!(find_table(&fake, objects, data_table, &short), Some(wanted));
        let objects = object_array(&fake, &[default, other]);
        assert_eq!(find_table(&fake, objects, data_table, &short), None);
    }
}
