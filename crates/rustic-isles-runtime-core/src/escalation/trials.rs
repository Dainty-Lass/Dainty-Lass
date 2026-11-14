//! ESC-1 (R18 3): reading the arena schedule rows `FGameplayScheduler::Initialize` is about to copy, and planning
//! the start and end writes that make the current Trials week active.

use super::FNames;
use crate::config::trials::{TICKS_PER_MINUTE, WEEKS, parse_week_row, rotation_at, week_row};
use crate::memory::Memory;
use crate::names;
use crate::offsets::{layout, r18};
use std::collections::BTreeMap;

/// One hour in .NET ticks.
pub const HOUR_TICKS: i64 = 36_000_000_000;
/// One day in .NET ticks.
pub const DAY_TICKS: i64 = 864_000_000_000;
/// The most tables the array may hold.
pub const MAX_TABLES: i32 = 64;
/// How much of a table must be readable.
pub const TABLE_READ: usize = 0x40;
/// The row map's element bound (R18 call 6).
pub const MAX_ROW_MAP: i32 = 4096;

/// One arena table's rows, by week.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaTable {
    /// The table's name, short form with its number.
    pub name: String,
    /// The row pointer of each week found (a later duplicate replaces an earlier one).
    pub rows: BTreeMap<u32, usize>,
    /// Every parsed row, duplicates counted.
    pub found: u32,
}

/// Whether a table name is an arena table: two case-sensitive substrings.
pub fn is_arena_table(name: &str) -> bool {
    name.contains("arena") || name.contains("Arena")
}

fn usize_of(value: i32) -> Option<usize> {
    usize::try_from(value).ok()
}

/// The arena tables of the `TArray<UDataTable*>` at `tables` (R18 3.3). An unusable array is empty.
pub fn read_tables(memory: &dyn Memory, names: &dyn FNames, tables: usize) -> Vec<ArenaTable> {
    let mut out = Vec::new();
    if !memory.readable(tables, 0x10) {
        return out;
    }
    let (Some(data), Some(num)) = (
        memory.ptr_at(tables + r18::TARRAY_DATA),
        memory.i32_at(tables + r18::TARRAY_NUM),
    ) else {
        return out;
    };
    if data == 0 || !(1..=MAX_TABLES).contains(&num) {
        return out;
    }
    let Some(num) = usize_of(num) else {
        return out;
    };
    // (departure) every entry checked readable, not only the first
    if !memory.readable(data, num * 8) {
        return out;
    }
    for index in 0..num {
        let Some(table) = data.checked_add(index * 8).and_then(|at| memory.ptr_at(at)) else {
            continue;
        };
        if table == 0 || !memory.readable(table, TABLE_READ) {
            continue;
        }
        let Some(name) = table
            .checked_add(layout::UOBJECT_NAME)
            .and_then(|at| names.fname_at(at))
            .map(|raw| names::short(&raw).to_owned())
        else {
            continue;
        };
        if !is_arena_table(&name) {
            continue;
        }
        if let Some((rows, found)) = read_row_map(memory, names, table) {
            out.push(ArenaTable { name, rows, found });
        }
    }
    out
}

/// The table's week rows, or `None` when its row map fails a bound (the table is skipped).
fn read_row_map(memory: &dyn Memory, names: &dyn FNames, table: usize) -> Option<(BTreeMap<u32, usize>, u32)> {
    let map = table.checked_add(r18::DATATABLE_ROW_MAP)?;
    let data = memory.ptr_at(map + r18::SPARSE_DATA)?;
    let num = memory.i32_at(map + r18::SPARSE_NUM)?;
    let max = memory.i32_at(map + r18::SPARSE_MAX)?;
    if num < 0 || num > max || max > MAX_ROW_MAP {
        return None;
    }
    let mut rows = BTreeMap::new();
    let mut found = 0;
    let count = usize_of(num)?;
    if count == 0 {
        return Some((rows, found));
    }
    if !memory.readable(data, count * r18::ROW_MAP_STRIDE) {
        return None;
    }
    let heap = memory.ptr_at(map + r18::SPARSE_BITS_HEAP)?;
    let bits_at = if heap != 0 { heap } else { map + r18::SPARSE_BITS_INLINE };
    let words = count.div_ceil(32);
    if !memory.readable(bits_at, words * 4) {
        return None;
    }
    let mut bits = vec![0u8; words * 4];
    if !memory.read(bits_at, &mut bits) {
        return None;
    }
    for index in 0..count {
        let word = u32::from_le_bytes([
            bits[index / 32 * 4],
            bits[index / 32 * 4 + 1],
            bits[index / 32 * 4 + 2],
            bits[index / 32 * 4 + 3],
        ]);
        if word & (1 << (index % 32)) == 0 {
            continue;
        }
        let element = data + index * r18::ROW_MAP_STRIDE;
        let Some(row) = memory.ptr_at(element + r18::ROW_MAP_VALUE) else {
            continue;
        };
        if row == 0 || !memory.readable(row, r18::SCHEDULE_ROW_READ) {
            continue;
        }
        // The key keeps its number: `Scheduled_Arena_Hunt_181` is stored as text `..._Hunt` with number 182
        let Some(week) = names
            .fname_at(element + r18::ROW_MAP_KEY)
            .and_then(|name| parse_week_row(&name))
        else {
            continue;
        };
        rows.insert(week, row);
        found += 1;
    }
    Some((rows, found))
}

/// One row's two writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowWrite {
    /// The week.
    pub week: u32,
    /// The row.
    pub row: usize,
    /// `StartTime`.
    pub start: i64,
    /// `EndTime`.
    pub end: i64,
}

impl RowWrite {
    /// `(address, value)` for `StartTime` then `EndTime`.
    pub fn writes(&self) -> [(usize, i64); 2] {
        [
            (self.row.wrapping_add(r18::SCHEDULE_ROW_START), self.start),
            (self.row.wrapping_add(r18::SCHEDULE_ROW_END), self.end),
        ]
    }
}

/// What one table's line says, apart from the writes applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The Director pinned a week for this launch.
    Pinned {
        /// The week.
        week: u32,
        /// Whether the table had that row.
        present: bool,
    },
    /// Contiguous windows from the current week on.
    Rotating {
        /// The rotation in minutes.
        minutes: i64,
        /// The current week.
        current: u32,
        /// The current bucket's first tick.
        bucket_start: i64,
        /// Rows parsed, duplicates counted.
        found: u32,
    },
}

/// One arena table's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablePlan {
    /// The table's name.
    pub table: String,
    /// The rows to write, in week order.
    pub rows: Vec<RowWrite>,
    /// The line's mode.
    pub mode: Mode,
}

impl TablePlan {
    /// The table's log line; `patched` is the rows whose two writes were both applied (R18 departure 2).
    pub fn line(&self, patched: u32) -> String {
        match &self.mode {
            Mode::Pinned { week, present: true } => format!(
                "[TrialsSchedule] Initialize: launch-pinned row='{}' duration=7d table={}",
                week_row(*week),
                self.table
            ),
            Mode::Pinned { week, present: false } => format!(
                "[TrialsSchedule] Initialize: launch-pinned row missing '{}' table={}",
                week_row(*week),
                self.table
            ),
            Mode::Rotating {
                minutes,
                current,
                bucket_start,
                found,
            } => format!(
                "[TrialsSchedule] Initialize: rotating intervalMinutes={minutes} currentRow='{}' \
                 patchedRows={patched}/{found} bucketStartTicks={bucket_start} horizonEndTicks={} table={}",
                week_row(*current),
                bucket_start.saturating_add(WEEKS.saturating_mul(rotation_ticks(*minutes))),
                self.table
            ),
        }
    }
}

fn rotation_ticks(minutes: i64) -> i64 {
    minutes.max(1).saturating_mul(TICKS_PER_MINUTE)
}

/// The writes for one table (R18 3.4, 3.5): `now` in .NET ticks, the rotation in minutes, the pinned week.
pub fn plan(table: &ArenaTable, now: i64, minutes: i64, pinned: Option<u32>) -> TablePlan {
    if let Some(week) = pinned {
        let row = table.rows.get(&week).copied();
        let start = now.saturating_sub(HOUR_TICKS);
        return TablePlan {
            table: table.name.clone(),
            rows: row
                .map(|row| RowWrite {
                    week,
                    row,
                    start,
                    end: start.saturating_add(7 * DAY_TICKS),
                })
                .into_iter()
                .collect(),
            mode: Mode::Pinned {
                week,
                present: row.is_some(),
            },
        };
    }
    let rotation = rotation_ticks(minutes);
    let at = rotation_at(now, minutes);
    let current = i64::from(at.week);
    let rows = table
        .rows
        .iter()
        .map(|(&week, &row)| {
            let forward = (i64::from(week) - current + WEEKS).rem_euclid(WEEKS);
            let start = at.bucket_start.saturating_add(forward.saturating_mul(rotation));
            RowWrite {
                week,
                row,
                start,
                end: start.saturating_add(rotation),
            }
        })
        .collect();
    TablePlan {
        table: table.name.clone(),
        rows,
        mode: Mode::Rotating {
            minutes,
            current: at.week,
            bucket_start: at.bucket_start,
            found: table.found,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::trials::UNIX_EPOCH_TICKS;
    use crate::escalation::fake::World;

    const BASE: usize = 0x40_0000;
    const ARRAY: usize = BASE;
    const TABLE_PTRS: usize = BASE + 0x100;
    const TABLES: usize = BASE + 0x1000;
    const ELEMENTS: usize = BASE + 0x4000;
    const HEAP_BITS: usize = BASE + 0x8000;
    const ROWS: usize = BASE + 0x1_0000;
    const SIZE: usize = 0x2_0000;

    /// 2026-11-12T00:00:00Z.
    const NOV_12: i64 = UNIX_EPOCH_TICKS + 1_794_441_600 * 10_000_000;

    fn table_at(slot: usize) -> usize {
        TABLES + slot * 0x100
    }

    fn row_at(slot: usize) -> usize {
        ROWS + slot * 0x30
    }

    fn world() -> World {
        World::new(BASE, SIZE)
    }

    fn set_array(world: &mut World, data: usize, num: i32) {
        world.put_ptr(ARRAY, data);
        world.put_i32(ARRAY + 8, num);
    }

    /// Table `slot` named `name` with `keys` (text, number) as used elements, rows at `row_at(first_row + i)`.
    fn table(world: &mut World, slot: usize, name: &str, keys: &[(&str, u32)], elements: usize, first_row: usize) {
        let table = table_at(slot);
        world.put_fname(table + layout::UOBJECT_NAME, name, 0);
        let map = table + r18::DATATABLE_ROW_MAP;
        world.put_ptr(map, elements);
        let num = i32::try_from(keys.len()).unwrap();
        world.put_i32(map + r18::SPARSE_NUM, num);
        world.put_i32(map + r18::SPARSE_MAX, num);
        let bits_at = if keys.len() > 128 {
            world.put_ptr(map + r18::SPARSE_BITS_HEAP, HEAP_BITS + slot * 0x40);
            HEAP_BITS + slot * 0x40
        } else {
            map + r18::SPARSE_BITS_INLINE
        };
        for (index, (text, number)) in keys.iter().enumerate() {
            let element = elements + index * r18::ROW_MAP_STRIDE;
            world.put_fname(element, text, *number);
            world.put_ptr(element + 8, row_at(first_row + index));
            let word = bits_at + index / 32 * 4;
            let old = u32::from_le_bytes(world.bytes[word - BASE..word - BASE + 4].try_into().unwrap());
            world.put(word, &(old | 1 << (index % 32)).to_le_bytes());
        }
    }

    fn all_weeks() -> Vec<(String, u32)> {
        // How UE stores them: a trailing number without a leading zero moves into the number
        (1..=181u32)
            .map(|week| {
                if week >= 100 {
                    ("Scheduled_Arena_Hunt".to_owned(), week + 1)
                } else {
                    (week_row(week), 0)
                }
            })
            .collect()
    }

    fn keys(list: &[(String, u32)]) -> Vec<(&str, u32)> {
        list.iter().map(|(text, number)| (text.as_str(), *number)).collect()
    }

    fn read(world: &World) -> Vec<ArenaTable> {
        read_tables(&world.memory(), &world.fnames(), ARRAY)
    }

    fn one_table(name: &str) -> World {
        let mut world = world();
        set_array(&mut world, TABLE_PTRS, 1);
        world.put_ptr(TABLE_PTRS, table_at(0));
        table(&mut world, 0, name, &[("Scheduled_Arena_Hunt_005", 0)], ELEMENTS, 0);
        world
    }

    #[test]
    fn table_array() {
        let mut world = one_table("DT_ArenaSchedule");
        assert_eq!(read(&world).len(), 1);
        set_array(&mut world, TABLE_PTRS, 0);
        assert!(read(&world).is_empty(), "Num 0");
        set_array(&mut world, 0, 1);
        assert!(read(&world).is_empty(), "null data");
        // 64 entries, the last the table, the rest null
        set_array(&mut world, TABLE_PTRS, 64);
        world.put_ptr(TABLE_PTRS, 0);
        world.put_ptr(TABLE_PTRS + 63 * 8, table_at(0));
        assert_eq!(read(&world).len(), 1, "Num 64, null entries skipped");
        set_array(&mut world, TABLE_PTRS, 65);
        assert!(read(&world).is_empty(), "Num 65");
        // The data runs off the end of memory for Num × 8
        let tail = BASE + SIZE - 8;
        world.put_ptr(tail, table_at(0));
        set_array(&mut world, tail, 2);
        assert!(read(&world).is_empty(), "data short of Num × 8");
        set_array(&mut world, tail, 1);
        assert_eq!(read(&world).len(), 1);
        // An unreadable table entry is skipped, the next still read
        set_array(&mut world, TABLE_PTRS, 2);
        world.put_ptr(TABLE_PTRS, BASE + SIZE + 0x1000);
        world.put_ptr(TABLE_PTRS + 8, table_at(0));
        assert_eq!(read(&world).len(), 1);
        assert!(read_tables(&world.memory(), &world.fnames(), BASE + SIZE - 8).is_empty());
    }

    #[test]
    fn table_names() {
        for (name, arena) in [
            ("DT_ArenaSchedule", true),
            ("arena_rows", true),
            ("ARENA_ROWS", false),
            ("DT_Escalation", false),
        ] {
            assert_eq!(read(&one_table(name)).len(), usize::from(arena), "{name}");
        }
    }

    #[test]
    fn row_map() {
        let mut world = one_table("DT_ArenaSchedule");
        let names = [
            ("Scheduled_Arena_Hunt_001", 0),
            ("Scheduled_Arena_Hunt", 0),
            ("Something_Else", 0),
            ("Scheduled_Arena_Hunt_002", 0),
            ("Scheduled_Arena_Hunt_001", 0),
            ("Scheduled_Arena_Hunt_003", 0),
            ("Scheduled_Arena_Hunt_004", 0),
        ];
        table(&mut world, 0, "DT_ArenaSchedule", &names, ELEMENTS, 0);
        // Element 3 is free; element 5's row is null; element 6's row is short (off the end of memory)
        let bits = table_at(0) + r18::DATATABLE_ROW_MAP + r18::SPARSE_BITS_INLINE;
        world.put(bits, &0b111_0111u32.to_le_bytes());
        world.put_ptr(ELEMENTS + 5 * 0x18 + 8, 0);
        world.put_ptr(ELEMENTS + 6 * 0x18 + 8, BASE + SIZE - 0x10);
        let tables = read(&world);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].rows.len(), 1, "{:?}", tables[0].rows);
        assert_eq!(tables[0].rows.get(&1), Some(&row_at(4)), "the duplicate replaces");
        assert_eq!(tables[0].found, 2, "both duplicates count");

        // ArrayNum over 4096 skips the table
        let map = table_at(0) + r18::DATATABLE_ROW_MAP;
        world.put_i32(map + r18::SPARSE_NUM, 4097);
        world.put_i32(map + r18::SPARSE_MAX, 4097);
        assert!(read(&world).is_empty());
        world.put_i32(map + r18::SPARSE_MAX, 7);
        assert!(read(&world).is_empty(), "num above max");
        world.put_i32(map + r18::SPARSE_NUM, 7);
        // An element block that runs off memory skips the table
        world.put_ptr(map, BASE + SIZE - 0x20);
        assert!(read(&world).is_empty());
        world.put_ptr(map, ELEMENTS);
        // A heap bits pointer that is unreadable skips the table
        world.put_ptr(map + r18::SPARSE_BITS_HEAP, BASE + SIZE + 0x100);
        assert!(read(&world).is_empty());
        world.put_ptr(map + r18::SPARSE_BITS_HEAP, 0);
        assert_eq!(read(&world).len(), 1);
    }

    #[test]
    fn number_suffix() {
        let mut world = one_table("DT_ArenaSchedule");
        table(
            &mut world,
            0,
            "DT_ArenaSchedule",
            &[
                ("Scheduled_Arena_Hunt", 182),
                ("Scheduled_Arena_Hunt_042", 0),
                ("Scheduled_Arena_Hunt", 0),
            ],
            ELEMENTS,
            0,
        );
        let tables = read(&world);
        assert_eq!(tables[0].rows.keys().copied().collect::<Vec<_>>(), [42, 181]);
        assert_eq!(tables[0].found, 2);
    }

    fn full_world(minutes_rows: &[(String, u32)]) -> (World, ArenaTable) {
        let mut world = world();
        set_array(&mut world, TABLE_PTRS, 1);
        world.put_ptr(TABLE_PTRS, table_at(0));
        table(&mut world, 0, "DT_ArenaSchedule", &keys(minutes_rows), ELEMENTS, 0);
        let mut tables = read(&world);
        assert_eq!(tables.len(), 1);
        let table = tables.remove(0);
        (world, table)
    }

    /// Applies a plan to the fake memory, as the DLL does; returns the rows patched.
    fn apply(world: &mut World, plan: &TablePlan) -> u32 {
        let mut patched = 0;
        for row in &plan.rows {
            for (address, value) in row.writes() {
                world.put_i64(address, value);
            }
            patched += 1;
        }
        patched
    }

    #[test]
    fn heap_bits() {
        let (_, table) = full_world(&all_weeks());
        assert_eq!(table.rows.len(), 181);
        assert_eq!(table.found, 181);
    }

    #[test]
    fn rotating_plan() {
        let (mut world, table) = full_world(&all_weeks());
        let rotation = 10_080 * TICKS_PER_MINUTE;
        let plan = plan(&table, NOV_12, 10_080, None);
        let bucket_start = rotation_at(NOV_12, 10_080).bucket_start;
        let start_of = |week: usize| plan.rows.iter().find(|row| row.week as usize == week).unwrap().start;
        assert_eq!(start_of(72), bucket_start);
        assert_eq!(start_of(73), bucket_start + rotation);
        assert_eq!(start_of(71), bucket_start + 180 * rotation);
        assert!(plan.rows.iter().all(|row| row.end == row.start + rotation));
        let patched = apply(&mut world, &plan);
        // Row 72 is element 71
        assert_eq!(world.i64_at(row_at(71) + 0x20), bucket_start);
        assert_eq!(world.i64_at(row_at(71) + 0x28), bucket_start + rotation);
        assert_eq!(
            plan.line(patched),
            format!(
                "[TrialsSchedule] Initialize: rotating intervalMinutes=10080 \
                 currentRow='Scheduled_Arena_Hunt_072' patchedRows=181/181 bucketStartTicks={bucket_start} \
                 horizonEndTicks={} table=DT_ArenaSchedule",
                bucket_start + 181 * rotation
            )
        );
        assert_eq!(
            plan.line(180),
            plan.line(181).replace("patchedRows=181/181", "patchedRows=180/181"),
            "only applied writes count"
        );

        let fast = super::plan(&table, NOV_12, 30, None);
        assert!(
            fast.line(181)
                .contains("intervalMinutes=30 currentRow='Scheduled_Arena_Hunt_146'")
        );
        assert_eq!(
            fast.rows.iter().find(|row| row.week == 146).unwrap().start,
            rotation_at(NOV_12, 30).bucket_start
        );

        let ten: Vec<(String, u32)> = (1..=10).map(|week| (week_row(week), 0)).collect();
        let (_, table) = full_world(&ten);
        let plan = super::plan(&table, NOV_12, 10_080, None);
        assert_eq!(plan.rows.len(), 10);
        assert!(plan.line(10).contains("patchedRows=10/10"));
    }

    #[test]
    fn pinned_plan() {
        let (mut world, table) = full_world(&all_weeks());
        let plan = plan(&table, NOV_12, 10_080, Some(5));
        assert_eq!(plan.rows.len(), 1, "only the pinned row");
        let row = plan.rows[0];
        assert_eq!((row.week, row.row), (5, row_at(4)));
        assert_eq!(row.start, NOV_12 - 36_000_000_000);
        assert_eq!(row.end, row.start + 7 * 864_000_000_000);
        apply(&mut world, &plan);
        assert_eq!(world.i64_at(row_at(4) + 0x20), row.start);
        assert_eq!(world.i64_at(row_at(5) + 0x20), 0, "other rows keep their windows");
        assert_eq!(
            plan.line(1),
            "[TrialsSchedule] Initialize: launch-pinned row='Scheduled_Arena_Hunt_005' duration=7d \
             table=DT_ArenaSchedule"
        );

        let ten: Vec<(String, u32)> = (6..=10).map(|week| (week_row(week), 0)).collect();
        let (_, table) = full_world(&ten);
        let plan = super::plan(&table, NOV_12, 10_080, Some(5));
        assert!(plan.rows.is_empty());
        assert_eq!(
            plan.line(0),
            "[TrialsSchedule] Initialize: launch-pinned row missing 'Scheduled_Arena_Hunt_005' table=DT_ArenaSchedule"
        );
    }

    #[test]
    fn two_arena_tables() {
        let mut world = world();
        set_array(&mut world, TABLE_PTRS, 3);
        world.put_ptr(TABLE_PTRS, table_at(0));
        world.put_ptr(TABLE_PTRS + 8, table_at(1));
        world.put_ptr(TABLE_PTRS + 16, table_at(2));
        table(
            &mut world,
            0,
            "DT_ArenaSchedule",
            &[("Scheduled_Arena_Hunt_001", 0)],
            ELEMENTS,
            0,
        );
        table(
            &mut world,
            1,
            "DT_Escalation",
            &[("Scheduled_Arena_Hunt_002", 0)],
            ELEMENTS + 0x100,
            1,
        );
        let two = [("Scheduled_Arena_Hunt_003", 0), ("Scheduled_Arena_Hunt_004", 0)];
        table(&mut world, 2, "arena_rows", &two, ELEMENTS + 0x200, 2);
        let tables = read(&world);
        assert_eq!(
            tables.iter().map(|table| table.name.as_str()).collect::<Vec<_>>(),
            ["DT_ArenaSchedule", "arena_rows"]
        );
        let lines: Vec<String> = tables
            .iter()
            .map(|table| {
                let plan = plan(table, NOV_12, 10_080, None);
                plan.line(u32::try_from(plan.rows.len()).unwrap())
            })
            .collect();
        assert!(lines[0].contains("patchedRows=1/1") && lines[0].ends_with("table=DT_ArenaSchedule"));
        assert!(lines[1].contains("patchedRows=2/2") && lines[1].ends_with("table=arena_rows"));
    }
}
