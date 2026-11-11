//! The engine as the exporter sees it (spec §4.1): the object array, names, strings, texts, arrays, maps, data tables
//! and soft pointers, all through guarded reads. Layouts come from the SDK (`crate::sdk`); the few fixed ones here
//! are UE 4.26's own containers and the spec's fixed offsets (§4.5).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::memory::{Calls, Memory};
use crate::sdk;
use crate::sdk_parse::{Kind, Member};

/// `RF_ClassDefaultObject`.
const CLASS_DEFAULT_OBJECT: u32 = 0x10;
/// UE 4.26's `FNamePool`: the block table after the allocator's lock and cursors, 2-byte entry stride, 16 offset
/// bits; an entry header is `bIsWide:1, LowercaseProbeHash:5, Len:10` (spec §12 ambiguity 1).
const NAME_POOL_BLOCKS: u64 = 0x10;
const NAME_STRIDE: u64 = 2;
const NAME_OFFSET_BITS: u32 = 16;
const NAME_BLOCKS: u32 = 8192;
/// `TMap` = `TSet` = `TSparseArray` (a `TArray`; the allocation `TBitArray` with 4 inline words and a heap
/// pointer; first free, free count).
const SPARSE_BITS_INLINE: u64 = 0x10;
const SPARSE_BITS_HEAP: u64 = 0x20;
const SPARSE_FREE_COUNT: u64 = 0x34;
/// The path in a hunt's soft pointer: 8 bytes weak pointer, 4 tag, 4 padding (spec §4.5). Other reads use the SDK's.
pub const SOFT_PATH: u64 = 0x10;
/// Bounds that keep a corrupt count from walking all of memory.
const MAX_OBJECTS: i32 = 4_000_000;
const MAX_ELEMENTS: i32 = 1_000_000;
const MAX_STRING_UNITS: i32 = 1 << 20;
const MAX_SUPER_HOPS: usize = 256;

/// A number as its SDK type prints it (spec §5): integers in decimal, floats as `%f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

pub struct Engine<'a> {
    mem: &'a dyn Memory,
    calls: &'a dyn Calls,
    gobjects: u64,
    names: u64,
    fault: Cell<bool>,
    name_cache: RefCell<HashMap<u32, String>>,
    class_cache: RefCell<HashMap<String, u64>>,
}

impl<'a> Engine<'a> {
    /// `gobjects` is the `TUObjectArray`, `names` the `FNamePool` (the image base plus the SDK's offsets).
    pub fn new(mem: &'a dyn Memory, calls: &'a dyn Calls, gobjects: u64, names: u64) -> Self {
        Self {
            mem,
            calls,
            gobjects,
            names,
            fault: Cell::new(false),
            name_cache: RefCell::new(HashMap::new()),
            class_cache: RefCell::new(HashMap::new()),
        }
    }

    // ---- guarded reads ----

    /// Clears the fault flag, returning whether a read failed since the last clear.
    pub fn take_fault(&self) -> bool {
        self.fault.replace(false)
    }

    /// Runs `read` without letting its faults count against the row (the old exporter's per-string guards: a fault
    /// gives empty text, not a skipped row).
    fn quiet<T>(&self, read: impl FnOnce() -> T) -> T {
        let before = self.fault.get();
        let value = read();
        self.fault.set(before);
        value
    }

    pub fn bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
        let mut buf = vec![0; len];
        if addr != 0 && self.mem.read(addr, &mut buf) {
            Some(buf)
        } else {
            self.fault.set(true);
            None
        }
    }

    fn fixed<const N: usize>(&self, addr: u64) -> Option<[u8; N]> {
        let mut buf = [0; N];
        if addr != 0 && self.mem.read(addr, &mut buf) {
            Some(buf)
        } else {
            self.fault.set(true);
            None
        }
    }

    pub fn u8(&self, addr: u64) -> u8 {
        self.fixed::<1>(addr).map_or(0, |b| b[0])
    }
    pub fn u16(&self, addr: u64) -> u16 {
        self.fixed(addr).map_or(0, u16::from_le_bytes)
    }
    pub fn i32(&self, addr: u64) -> i32 {
        self.fixed(addr).map_or(0, i32::from_le_bytes)
    }
    pub fn u32(&self, addr: u64) -> u32 {
        self.fixed(addr).map_or(0, u32::from_le_bytes)
    }
    pub fn i64(&self, addr: u64) -> i64 {
        self.fixed(addr).map_or(0, i64::from_le_bytes)
    }
    pub fn ptr(&self, addr: u64) -> u64 {
        self.fixed(addr).map_or(0, u64::from_le_bytes)
    }

    // ---- objects ----

    pub fn object_count(&self) -> i32 {
        self.i32(self.gobjects.wrapping_add(u64::from(sdk::OBJECT_ARRAY_NUM.offset)))
    }

    fn object_item(&self, index: i32) -> u64 {
        let per = u64::try_from(sdk::ELEMENTS_PER_CHUNK).unwrap_or(0x10000).max(1);
        let Ok(index) = u64::try_from(index) else {
            return 0;
        };
        let chunks = self.ptr(self.gobjects.wrapping_add(u64::from(sdk::OBJECT_ARRAY_OBJECTS.offset)));
        if chunks == 0 {
            return 0;
        }
        let chunk = self.ptr(chunks.wrapping_add(index / per * 8));
        if chunk == 0 {
            return 0;
        }
        chunk.wrapping_add(index % per * u64::from(sdk::OBJECT_ITEM_SIZE))
    }

    pub fn object_at(&self, index: i32) -> u64 {
        let item = self.object_item(index);
        if item == 0 {
            0
        } else {
            self.ptr(item.wrapping_add(u64::from(sdk::OBJECT_ITEM_OBJECT.offset)))
        }
    }

    /// Every live object with its index, in array order (null slots skipped).
    pub fn objects(&self) -> Vec<(i32, u64)> {
        let count = self.object_count().clamp(0, MAX_OBJECTS);
        let objects = (0..count)
            .filter_map(|index| {
                let object = self.object_at(index);
                (object != 0).then_some((index, object))
            })
            .collect();
        self.fault.set(false);
        objects
    }

    pub fn class_of(&self, object: u64) -> u64 {
        self.ptr(object.wrapping_add(u64::from(sdk::UOBJECT_CLASS.offset)))
    }

    pub fn outer_of(&self, object: u64) -> u64 {
        self.ptr(object.wrapping_add(u64::from(sdk::UOBJECT_OUTER.offset)))
    }

    pub fn is_default(&self, object: u64) -> bool {
        self.u32(object.wrapping_add(u64::from(sdk::UOBJECT_FLAGS.offset))) & CLASS_DEFAULT_OBJECT != 0
    }

    /// The object's name, raw form.
    pub fn raw_name_of(&self, object: u64) -> String {
        if object == 0 {
            return String::new();
        }
        self.quiet(|| self.fname_raw(object.wrapping_add(u64::from(sdk::UOBJECT_NAME.offset))))
    }

    /// The object's name, short form (spec §4.3).
    pub fn name_of(&self, object: u64) -> String {
        short(&self.raw_name_of(object)).to_owned()
    }

    pub fn class_name_of(&self, object: u64) -> String {
        self.name_of(self.class_of(object))
    }

    /// `StaticClass()` by name: the first object named `name` whose own class is a class (`Class` or a generated
    /// class). Found once, then cached.
    pub fn find_class(&self, name: &str) -> u64 {
        if let Some(&class) = self.class_cache.borrow().get(name) {
            return class;
        }
        let found = self
            .objects()
            .into_iter()
            .map(|(_, object)| object)
            .find(|&object| {
                self.raw_name_of(object) == name && {
                    let meta = self.raw_name_of(self.class_of(object));
                    meta == "Class" || meta.ends_with("GeneratedClass")
                }
            })
            .unwrap_or(0);
        self.fault.set(false);
        if found != 0 {
            self.class_cache.borrow_mut().insert(name.to_owned(), found);
        }
        found
    }

    pub fn is_a(&self, object: u64, class: u64) -> bool {
        if class == 0 {
            return false;
        }
        let mut current = self.class_of(object);
        for _ in 0..MAX_SUPER_HOPS {
            if current == 0 {
                return false;
            }
            if current == class {
                return true;
            }
            current = self.ptr(current.wrapping_add(u64::from(sdk::USTRUCT_SUPER.offset)));
        }
        false
    }

    /// The `UFunction` `function` of class `class` (by names), for `ProcessEvent`.
    pub fn find_function(&self, class: &str, function: &str) -> u64 {
        let found = self
            .objects()
            .into_iter()
            .map(|(_, object)| object)
            .find(|&object| {
                self.raw_name_of(object) == function
                    && self.raw_name_of(self.class_of(object)) == "Function"
                    && self.raw_name_of(self.outer_of(object)) == class
            })
            .unwrap_or(0);
        self.fault.set(false);
        found
    }

    pub fn process_event(&self, object: u64, function: u64, params: &mut [u64]) -> bool {
        object != 0 && function != 0 && self.calls.process_event(object, function, params)
    }

    // ---- names ----

    /// An `FName`'s raw form: the entry's text, then `_<n-1>` for a number n > 0.
    pub fn name_text(&self, index: i32, number: i32) -> String {
        let Ok(index) = u32::try_from(index) else {
            return String::new();
        };
        let cached = self.name_cache.borrow().get(&index).cloned();
        let text = match cached {
            Some(text) => text,
            None => match self.quiet(|| self.decode_name(index)) {
                Some(text) => {
                    self.name_cache.borrow_mut().insert(index, text.clone());
                    text
                }
                None => return String::new(),
            },
        };
        if number > 0 {
            format!("{text}_{}", number - 1)
        } else {
            text
        }
    }

    fn decode_name(&self, index: u32) -> Option<String> {
        let block = index >> NAME_OFFSET_BITS;
        if block >= NAME_BLOCKS {
            return None;
        }
        let offset = u64::from(index & ((1 << NAME_OFFSET_BITS) - 1)) * NAME_STRIDE;
        let base = self.ptr(self.names.wrapping_add(NAME_POOL_BLOCKS + u64::from(block) * 8));
        if base == 0 {
            return None;
        }
        let entry = base.wrapping_add(offset);
        let header = self.fixed::<2>(entry).map(u16::from_le_bytes)?;
        let wide = header & 1 != 0;
        let len = usize::from(header >> 6);
        if wide {
            let bytes = self.bytes(entry.wrapping_add(2), len * 2)?;
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            Some(String::from_utf16_lossy(&units))
        } else {
            let bytes = self.bytes(entry.wrapping_add(2), len)?;
            Some(bytes.iter().map(|&byte| char::from(byte)).collect())
        }
    }

    /// The `FName` at `addr`, raw form.
    pub fn fname_raw(&self, addr: u64) -> String {
        self.quiet(|| {
            let index = self.i32(addr);
            let number = self.i32(addr.wrapping_add(4));
            if self.fault.get() {
                String::new()
            } else {
                self.name_text(index, number)
            }
        })
    }

    /// The `FName` at `addr`, short form.
    pub fn fname(&self, addr: u64) -> String {
        short(&self.fname_raw(addr)).to_owned()
    }

    // ---- strings, texts, containers ----

    /// An `FString`: empty when its length is 0 or its data null; up to the first NUL.
    pub fn fstring(&self, addr: u64) -> String {
        self.quiet(|| {
            let data = self.ptr(addr);
            let num = self.i32(addr.wrapping_add(8));
            if data == 0 || num <= 0 || num > MAX_STRING_UNITS {
                return String::new();
            }
            let Some(bytes) = self.bytes(data, usize::try_from(num).unwrap_or(0) * 2) else {
                return String::new();
            };
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .take_while(|&unit| unit != 0)
                .collect();
            String::from_utf16_lossy(&units)
        })
    }

    /// An `FText`'s display string, through its text data.
    pub fn ftext(&self, addr: u64) -> String {
        self.quiet(|| {
            let data = self.ptr(addr);
            if data == 0 {
                String::new()
            } else {
                self.fstring(data.wrapping_add(u64::from(sdk::TEXT_DATA_SOURCE.offset)))
            }
        })
    }

    /// A `TArray`'s data and count; a negative or absurd count reads as empty and faults.
    pub fn array(&self, addr: u64) -> (u64, i32) {
        let data = self.ptr(addr);
        let num = self.i32(addr.wrapping_add(8));
        if !(0..=MAX_ELEMENTS).contains(&num) {
            self.fault.set(true);
            return (0, 0);
        }
        if num > 0 && data == 0 {
            self.fault.set(true);
            return (0, 0);
        }
        (data, num)
    }

    /// The addresses of a `TArray`'s elements.
    pub fn elements(&self, addr: u64, stride: u32) -> Vec<u64> {
        let (data, num) = self.array(addr);
        (0..u64::try_from(num).unwrap_or(0))
            .map(|i| data.wrapping_add(i * u64::from(stride)))
            .collect()
    }

    /// A `TMap`'s (a `TSparseArray`'s) count and its allocated elements in storage order, free slots skipped.
    pub fn map_elements(&self, addr: u64, stride: u32) -> (i32, Vec<u64>) {
        let (data, array_num) = self.array(addr);
        let free = self.i32(addr.wrapping_add(SPARSE_FREE_COUNT));
        let heap = self.ptr(addr.wrapping_add(SPARSE_BITS_HEAP));
        let bits_at = if heap != 0 {
            heap
        } else {
            addr.wrapping_add(SPARSE_BITS_INLINE)
        };
        let words = usize::try_from(array_num).unwrap_or(0).div_ceil(32);
        let bits = if words == 0 {
            Some(Vec::new())
        } else {
            self.bytes(bits_at, words * 4)
        };
        let Some(bits) = bits else {
            return (0, Vec::new());
        };
        let elements = (0..usize::try_from(array_num).unwrap_or(0))
            .filter(|&i| bits[i / 8] >> (i % 8) & 1 != 0)
            .map(|i| data.wrapping_add(i as u64 * u64::from(stride)))
            .collect();
        (array_num.saturating_sub(free.max(0)), elements)
    }

    /// A soft pointer's asset path name, raw form (spec §4.5).
    pub fn soft_path_raw(&self, addr: u64) -> String {
        self.fname_raw(addr.wrapping_add(SOFT_PATH))
    }

    /// A soft pointer's asset path name through the SDK's offsets, short form (store images, spec §6.13).
    pub fn soft_path_sdk(&self, addr: u64) -> String {
        let path = sdk::SOFT_OBJECT_ID.offset.saturating_add(sdk::SOFT_PATH_NAME.offset);
        self.fname(addr.wrapping_add(u64::from(path)))
    }

    /// The object a weak pointer (a soft pointer's first part) names when it is loaded, else 0.
    pub fn weak_object(&self, addr: u64) -> u64 {
        self.quiet(|| {
            let index = self.i32(addr);
            let serial = self.i32(addr.wrapping_add(4));
            if index < 0 || index >= self.object_count() || serial == 0 {
                return 0;
            }
            let item = self.object_item(index);
            if item == 0 || self.i32(item.wrapping_add(u64::from(sdk::OBJECT_ITEM_SERIAL.offset))) != serial {
                return 0;
            }
            self.object_at(index)
        })
    }

    // ---- data tables ----

    /// Every object that is a `DataTable`, default objects included, in array order.
    pub fn data_tables(&self) -> Vec<(i32, u64)> {
        let class = self.find_class("DataTable");
        if class == 0 {
            return Vec::new();
        }
        let tables = self
            .objects()
            .into_iter()
            .filter(|&(_, object)| self.is_a(object, class))
            .collect();
        self.fault.set(false);
        tables
    }

    /// The row struct's name, or `None` when the table has none.
    pub fn row_struct_name(&self, table: u64) -> Option<String> {
        let row_struct = self.ptr(table.wrapping_add(u64::from(sdk::DATATABLE_ROW_STRUCT.offset)));
        (row_struct != 0).then(|| self.name_of(row_struct))
    }

    /// Tables whose row struct is named `name`, in array order.
    pub fn tables_by_row_struct(&self, name: &str) -> Vec<u64> {
        let tables = self
            .data_tables()
            .into_iter()
            .map(|(_, table)| table)
            .filter(|&table| self.row_struct_name(table).as_deref() == Some(name))
            .collect();
        self.fault.set(false);
        tables
    }

    /// The first table named `name`.
    pub fn table_by_name(&self, name: &str) -> Option<u64> {
        let table = self
            .data_tables()
            .into_iter()
            .map(|(_, table)| table)
            .find(|&table| self.raw_name_of(table) == name);
        self.fault.set(false);
        table
    }

    /// The row count and the rows (name short form, row data) in map order.
    pub fn rows(&self, table: u64) -> (i32, Vec<(String, u64)>) {
        let map = table.wrapping_add(u64::from(sdk::DATATABLE_ROW_MAP.offset));
        let (count, elements) = self.map_elements(map, sdk::DATATABLE_ROW_MAP.stride);
        let value = u64::from(sdk::DATATABLE_ROW_MAP.value_offset);
        let rows = elements
            .into_iter()
            .map(|element| (self.fname(element), self.ptr(element.wrapping_add(value))))
            .collect();
        (count, rows)
    }

    // ---- members read by kind ----

    /// A member as text (spec §5's `s`): strings, names (short form), texts, object names, soft paths (short
    /// form), a tag's or a row handle's name; numbers in decimal; anything else empty.
    pub fn text(&self, base: u64, member: &Member) -> String {
        self.text_at(base.wrapping_add(u64::from(member.offset)), member.kind)
    }

    pub fn text_at(&self, addr: u64, kind: Kind) -> String {
        self.quiet(|| match kind {
            Kind::Str => self.fstring(addr),
            Kind::Name => self.fname(addr),
            Kind::Text => self.ftext(addr),
            Kind::ObjPtr | Kind::ClassPtr => self.name_of(self.ptr(addr)),
            Kind::SoftPtr => self.soft_path_sdk(addr),
            Kind::Tag => self.fname(addr.wrapping_add(u64::from(sdk::TAG_NAME.offset))),
            Kind::RowHandle => self.fname(addr.wrapping_add(u64::from(sdk::ROW_HANDLE_ROW.offset))),
            Kind::Int { .. } | Kind::Bool | Kind::Bit(_) | Kind::Float { .. } | Kind::DateTime => {
                match self.number_at(addr, kind) {
                    Num::Int(value) => value.to_string(),
                    Num::Float(value) => crate::json::printf_f(value),
                }
            }
            _ => String::new(),
        })
    }

    pub fn number(&self, base: u64, member: &Member) -> Num {
        self.number_at(base.wrapping_add(u64::from(member.offset)), member.kind)
    }

    pub fn number_at(&self, addr: u64, kind: Kind) -> Num {
        match kind {
            Kind::Int { size: 1, signed } => {
                let byte = self.u8(addr);
                Num::Int(if signed { i64::from(byte as i8) } else { i64::from(byte) })
            }
            Kind::Int { size: 2, signed } => {
                let value = self.u16(addr);
                Num::Int(if signed {
                    i64::from(value as i16)
                } else {
                    i64::from(value)
                })
            }
            Kind::Int { size: 4, signed } => {
                let value = self.u32(addr);
                Num::Int(if signed {
                    i64::from(value as i32)
                } else {
                    i64::from(value)
                })
            }
            Kind::Int { .. } | Kind::DateTime => Num::Int(self.i64(addr)),
            Kind::Float { size: 4 } => Num::Float(f64::from(f32::from_le_bytes(self.u32(addr).to_le_bytes()))),
            Kind::Float { .. } => Num::Float(f64::from_bits(self.i64(addr) as u64)),
            Kind::Bool | Kind::Bit(_) => Num::Int(i64::from(self.flag_at(addr, kind))),
            _ => Num::Int(0),
        }
    }

    /// A member as an integer (counts, enum values); a float is truncated.
    pub fn int(&self, base: u64, member: &Member) -> i64 {
        match self.number(base, member) {
            Num::Int(value) => value,
            Num::Float(value) => value as i64,
        }
    }

    pub fn flag(&self, base: u64, member: &Member) -> bool {
        self.flag_at(base.wrapping_add(u64::from(member.offset)), member.kind)
    }

    pub fn flag_at(&self, addr: u64, kind: Kind) -> bool {
        match kind {
            Kind::Bool => self.u8(addr) != 0,
            Kind::Bit(bit) => self.u8(addr) >> (bit & 7) & 1 != 0,
            Kind::Int { .. } | Kind::Float { .. } => self.number_at(addr, kind) != Num::Int(0),
            _ => false,
        }
    }

    /// An array's or map's element count.
    pub fn count(&self, base: u64, member: &Member) -> i64 {
        let addr = base.wrapping_add(u64::from(member.offset));
        match member.kind {
            Kind::Array => i64::from(self.array(addr).1),
            Kind::Map => i64::from(self.map_elements(addr, member.stride).0),
            _ => 0,
        }
    }

    /// An array member's elements.
    pub fn items(&self, base: u64, member: &Member) -> Vec<u64> {
        if member.kind != Kind::Array {
            return Vec::new();
        }
        self.elements(base.wrapping_add(u64::from(member.offset)), member.stride)
    }

    /// An array member's elements as text.
    pub fn strings(&self, base: u64, member: &Member) -> Vec<String> {
        self.items(base, member)
            .into_iter()
            .map(|element| self.text_at(element, member.elem))
            .collect()
    }

    /// An array member's elements as numbers.
    pub fn numbers(&self, base: u64, member: &Member) -> Vec<Num> {
        self.items(base, member)
            .into_iter()
            .map(|element| self.number_at(element, member.elem))
            .collect()
    }
}

/// The short form of a name: after its last `/` (spec §4.3).
pub fn short(raw: &str) -> &str {
    raw.rsplit('/').next().unwrap_or(raw)
}

/// Rows in ascending byte order of their names; each name takes the first row of that name (spec §5, §12
/// ambiguity 4: two equal names both print the first one's data).
pub fn sorted_rows(rows: &[(String, u64)]) -> Vec<(String, u64)> {
    let mut sorted: Vec<(String, u64)> = rows.to_vec();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut first: HashMap<&str, u64> = HashMap::new();
    for (name, row) in rows {
        first.entry(name.as_str()).or_insert(*row);
    }
    sorted
        .iter()
        .map(|(name, _)| (name.clone(), first.get(name.as_str()).copied().unwrap_or(0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_cuts_after_the_last_slash() {
        assert_eq!(short("/Game/Items/WP_AXE.WP_AXE_C"), "WP_AXE.WP_AXE_C");
        assert_eq!(short("Plain"), "Plain");
        assert_eq!(short("trailing/"), "");
    }

    #[test]
    fn sorts_rows_by_bytes_and_repeats_the_first_of_equal_names() {
        let rows = vec![
            ("b".to_owned(), 1),
            ("B".to_owned(), 2),
            ("a".to_owned(), 3),
            ("b".to_owned(), 4),
        ];
        let sorted = sorted_rows(&rows);
        assert_eq!(
            sorted,
            [
                ("B".to_owned(), 2),
                ("a".to_owned(), 3),
                ("b".to_owned(), 1),
                ("b".to_owned(), 1)
            ]
        );
    }
}
