//! A synthetic game image: an object array, a name pool, classes, data tables and the containers the exporter
//! reads, laid out from the synthetic SDK's facts. Every id and value in the tests is made up.
#![allow(dead_code, reason = "each test file uses part of the builder")]

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use catalog_exporter::engine::Engine;
use catalog_exporter::export::Ctx;
use catalog_exporter::memory::{Calls, Synthetic};
use catalog_exporter::sdk;
use catalog_exporter::sdk_parse::{Kind, Member};
use catalog_exporter::status::{Status, no_debug};

const POOL_BLOCK: usize = 0x1_0000;
const CHUNKS: usize = 4;
pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x10;

pub struct World {
    pub mem: Synthetic,
    pub gobjects: u64,
    pub pool: u64,
    block: u64,
    cursor: usize,
    names: HashMap<String, i32>,
    chunks: Vec<u64>,
    count: i32,
    classes: HashMap<String, u64>,
}

impl World {
    pub fn new() -> Self {
        let mut mem = Synthetic::new();
        let gobjects = mem.alloc(0x40);
        let pool = mem.alloc(0x10 + 8 * 4);
        let block = mem.alloc(POOL_BLOCK);
        mem.write_u64(pool + 0x10, block);
        let table = mem.alloc(8 * CHUNKS);
        let per = usize::try_from(sdk::ELEMENTS_PER_CHUNK).unwrap();
        let mut chunks = Vec::new();
        for chunk in 0..CHUNKS {
            let at = mem.alloc(per * sdk::OBJECT_ITEM_SIZE as usize);
            mem.write_u64(table + chunk as u64 * 8, at);
            chunks.push(at);
        }
        mem.write_u64(gobjects + u64::from(sdk::OBJECT_ARRAY_OBJECTS.offset), table);
        let mut world = Self {
            mem,
            gobjects,
            pool,
            block,
            cursor: 0,
            names: HashMap::new(),
            chunks,
            count: 0,
            classes: HashMap::new(),
        };
        world.intern("None");
        let meta = world.raw_object("Class", 0, 0, 0x100);
        world.mem.write_u64(meta + u64::from(sdk::UOBJECT_CLASS.offset), meta);
        world.classes.insert("Class".to_owned(), meta);
        for name in ["Object", "ScriptStruct", "Function"] {
            world.class(name, None);
        }
        world.class("DataTable", Some("Object"));
        world
    }

    pub fn engine<'a>(&'a self, calls: &'a dyn Calls) -> Engine<'a> {
        Engine::new(&self.mem, calls, self.gobjects, self.pool)
    }

    /// The name's pool index (narrow entries, 2-aligned).
    pub fn intern(&mut self, text: &str) -> i32 {
        if let Some(&index) = self.names.get(text) {
            return index;
        }
        let index = i32::try_from(self.cursor / 2).unwrap();
        let header = u16::try_from(text.len() << 6).unwrap();
        let entry = self.block + self.cursor as u64;
        self.mem.write(entry, &header.to_le_bytes());
        self.mem.write(entry + 2, text.as_bytes());
        self.cursor += (2 + text.len()).div_ceil(2) * 2;
        self.names.insert(text.to_owned(), index);
        index
    }

    /// A wide (UTF-16) name entry.
    pub fn intern_wide(&mut self, text: &str) -> i32 {
        let index = i32::try_from(self.cursor / 2).unwrap();
        let units: Vec<u16> = text.encode_utf16().collect();
        let header = u16::try_from(units.len() << 6 | 1).unwrap();
        let entry = self.block + self.cursor as u64;
        self.mem.write(entry, &header.to_le_bytes());
        let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        self.mem.write(entry + 2, &bytes);
        self.cursor += 2 + bytes.len();
        index
    }

    pub fn write_name(&mut self, at: u64, text: &str, number: i32) {
        let index = self.intern(text);
        self.mem.write_i32(at, index);
        self.mem.write_i32(at + 4, number);
    }

    pub fn write_fstring(&mut self, at: u64, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut units: Vec<u8> = text.encode_utf16().flat_map(|unit| unit.to_le_bytes()).collect();
        units.extend_from_slice(&[0, 0]);
        let data = self.mem.put(&units);
        let num = i32::try_from(units.len() / 2).unwrap();
        self.mem.write_u64(at, data);
        self.mem.write_i32(at + 8, num);
        self.mem.write_i32(at + 12, num);
    }

    pub fn write_ftext(&mut self, at: u64, text: &str) {
        let data = self.mem.alloc(sdk::TEXT_DATA_SOURCE.offset as usize + 16);
        self.write_fstring(data + u64::from(sdk::TEXT_DATA_SOURCE.offset), text);
        self.mem.write_u64(at, data);
    }

    /// An array of `count` zeroed elements; returns their addresses.
    pub fn write_array(&mut self, at: u64, count: usize, stride: u32) -> Vec<u64> {
        let data = self.mem.alloc(count * stride as usize);
        self.mem.write_u64(at, data);
        self.mem.write_i32(at + 8, i32::try_from(count).unwrap());
        self.mem.write_i32(at + 12, i32::try_from(count).unwrap());
        (0..count as u64).map(|i| data + i * u64::from(stride)).collect()
    }

    /// A sparse array (a `TMap`'s storage) with `slots` elements, those in `free` unallocated.
    pub fn write_sparse(&mut self, at: u64, slots: usize, stride: u32, free: &[usize]) -> Vec<u64> {
        let elements = self.write_array(at, slots, stride);
        let mut bits = [0u8; 16];
        for slot in 0..slots {
            if !free.contains(&slot) {
                bits[slot / 8] |= 1 << (slot % 8);
            }
        }
        self.mem.write(at + 0x10, &bits);
        self.mem.write_i32(at + 0x34, i32::try_from(free.len()).unwrap());
        elements
    }

    /// Writes `text` into a member as its kind stores text.
    pub fn set_text(&mut self, base: u64, member: &Member, text: &str) {
        let at = base + u64::from(member.offset);
        match member.kind {
            Kind::Str => self.write_fstring(at, text),
            Kind::Name => self.write_name(at, text, 0),
            Kind::Text => self.write_ftext(at, text),
            Kind::Tag => self.write_name(at + u64::from(sdk::TAG_NAME.offset), text, 0),
            Kind::RowHandle => self.write_name(at + u64::from(sdk::ROW_HANDLE_ROW.offset), text, 0),
            Kind::SoftPtr => {
                self.write_name(at + catalog_exporter::engine::SOFT_PATH, text, 0);
            }
            Kind::ObjPtr | Kind::ClassPtr => {
                let object = self.raw_object(text, 0, 0, 0x40);
                self.mem.write_u64(at, object);
            }
            other => panic!("no text in {other:?}"),
        }
    }

    pub fn set_int(&mut self, base: u64, member: &Member, value: i64) {
        let at = base + u64::from(member.offset);
        match member.kind {
            Kind::Int { size: 1, .. } => self.mem.write_u8(at, value as u8),
            Kind::Int { size: 4, .. } => self.mem.write_i32(at, value as i32),
            Kind::Int { .. } | Kind::DateTime => self.mem.write_u64(at, value as u64),
            other => panic!("no integer in {other:?}"),
        }
    }

    pub fn set_float(&mut self, base: u64, member: &Member, value: f32) {
        assert_eq!(member.kind, Kind::Float { size: 4 });
        self.mem.write_f32(base + u64::from(member.offset), value);
    }

    pub fn set_flag(&mut self, base: u64, member: &Member) {
        let at = base + u64::from(member.offset);
        let mut byte = [0u8];
        catalog_exporter::memory::Memory::read(&self.mem, at, &mut byte);
        match member.kind {
            Kind::Bool => self.mem.write_u8(at, 1),
            Kind::Bit(bit) => self.mem.write_u8(at, byte[0] | 1 << bit),
            other => panic!("no flag in {other:?}"),
        }
    }

    /// An array member's elements, made zeroed.
    pub fn set_items(&mut self, base: u64, member: &Member, count: usize) -> Vec<u64> {
        assert_eq!(member.kind, Kind::Array, "{member:?}");
        self.write_array(base + u64::from(member.offset), count, member.stride)
    }

    pub fn set_texts(&mut self, base: u64, member: &Member, texts: &[&str]) {
        let elements = self.set_items(base, member, texts.len());
        let element = Member::plain(0, 0, member.elem);
        for (at, text) in elements.into_iter().zip(texts) {
            self.set_text(at, &element, text);
        }
    }

    /// An object outside the object array (a row struct's pointee, a pointer target).
    pub fn raw_object(&mut self, name: &str, class: u64, outer: u64, size: usize) -> u64 {
        let object = self.mem.alloc(size.max(0x60));
        self.write_name(object + u64::from(sdk::UOBJECT_NAME.offset), name, 0);
        self.mem.write_u64(object + u64::from(sdk::UOBJECT_CLASS.offset), class);
        self.mem.write_u64(object + u64::from(sdk::UOBJECT_OUTER.offset), outer);
        object
    }

    /// Adds `object` to the object array; returns its index.
    pub fn register(&mut self, object: u64) -> i32 {
        let per = usize::try_from(sdk::ELEMENTS_PER_CHUNK).unwrap();
        let index = usize::try_from(self.count).unwrap();
        let item = self.chunks[index / per] + (index % per) as u64 * u64::from(sdk::OBJECT_ITEM_SIZE);
        self.mem
            .write_u64(item + u64::from(sdk::OBJECT_ITEM_OBJECT.offset), object);
        self.mem
            .write_i32(item + u64::from(sdk::OBJECT_ITEM_SERIAL.offset), self.count + 100);
        self.count += 1;
        self.mem
            .write_i32(self.gobjects + u64::from(sdk::OBJECT_ARRAY_NUM.offset), self.count);
        self.count - 1
    }

    /// Empties the object's slot in the array.
    pub fn unregister(&mut self, object: u64) {
        let per = usize::try_from(sdk::ELEMENTS_PER_CHUNK).unwrap();
        for index in 0..usize::try_from(self.count).unwrap() {
            let item = self.chunks[index / per] + (index % per) as u64 * u64::from(sdk::OBJECT_ITEM_SIZE);
            let mut word = [0u8; 8];
            catalog_exporter::memory::Memory::read(&self.mem, item, &mut word);
            if u64::from_le_bytes(word) == object {
                self.mem.write_u64(item, 0);
            }
        }
    }

    /// A null slot in the object array.
    pub fn register_null(&mut self) {
        self.register(0);
    }

    pub fn serial(&self, index: i32) -> i32 {
        index + 100
    }

    pub fn object(&mut self, name: &str, class: &str, size: usize) -> u64 {
        let class = self.classes[class];
        let object = self.raw_object(name, class, 0, size);
        self.register(object);
        object
    }

    pub fn default_object(&mut self, name: &str, class: &str, size: usize) -> u64 {
        let object = self.object(name, class, size);
        self.mem
            .write_u32(object + u64::from(sdk::UOBJECT_FLAGS.offset), RF_CLASS_DEFAULT_OBJECT);
        object
    }

    pub fn class(&mut self, name: &str, parent: Option<&str>) -> u64 {
        let meta = self.classes["Class"];
        let class = self.raw_object(name, meta, 0, 0x100);
        if let Some(parent) = parent {
            let parent = self.classes[parent];
            self.mem.write_u64(class + u64::from(sdk::USTRUCT_SUPER.offset), parent);
        }
        self.register(class);
        self.classes.insert(name.to_owned(), class);
        class
    }

    pub fn class_ptr(&self, name: &str) -> u64 {
        self.classes[name]
    }

    /// A data table with `rows` (name, row size) in map order, `holes` free slots before them.
    pub fn table(&mut self, name: &str, row_struct: &str, rows: &[(&str, usize)]) -> (u64, Vec<u64>) {
        self.table_with_holes(name, row_struct, rows, &[])
    }

    pub fn table_with_holes(
        &mut self,
        name: &str,
        row_struct: &str,
        rows: &[(&str, usize)],
        holes: &[usize],
    ) -> (u64, Vec<u64>) {
        let script_struct = self.classes["ScriptStruct"];
        let row_struct = self.raw_object(row_struct, script_struct, 0, 0x60);
        let table = self.object(name, "DataTable", 0x100);
        self.mem
            .write_u64(table + u64::from(sdk::DATATABLE_ROW_STRUCT.offset), row_struct);
        let map = sdk::DATATABLE_ROW_MAP;
        let slots = rows.len() + holes.len();
        let elements = self.write_sparse(table + u64::from(map.offset), slots, map.stride, holes);
        let mut datas = Vec::new();
        let mut rows = rows.iter();
        for (slot, element) in elements.into_iter().enumerate() {
            if holes.contains(&slot) {
                continue;
            }
            let Some((row_name, size)) = rows.next() else { break };
            self.write_name(element, row_name, 0);
            let data = self.mem.alloc(*size);
            self.mem.write_u64(element + u64::from(map.value_offset), data);
            datas.push(data);
        }
        (table, datas)
    }
}

/// `ProcessEvent` answering `GetAllItems` with a prepared array.
pub struct FakeCatalog {
    pub catalog: u64,
    pub function: u64,
    pub data: u64,
    pub count: i32,
    pub calls: Cell<u32>,
}

impl Calls for FakeCatalog {
    fn process_event(&self, object: u64, function: u64, params: &mut [u64]) -> bool {
        if object != self.catalog || function != self.function {
            return false;
        }
        self.calls.set(self.calls.get() + 1);
        let at = sdk::GET_ALL_ITEMS_OUT.offset as usize / 8;
        params[at] = self.data;
        params[at + 1] = u64::from(self.count as u32) | u64::from(self.count as u32) << 32;
        true
    }
}

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "catalog-exporter-it-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn ctx<'e, 'a>(engine: &'e Engine<'a>, out: &Path, status: &'e Status) -> Ctx<'e, 'a> {
    Ctx {
        engine,
        out: out.to_path_buf(),
        status,
    }
}

pub fn status(out: &Path) -> Status {
    Status::new(out, no_debug)
}

pub fn read(path: &Path) -> String {
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

pub fn status_text(out: &Path) -> String {
    read(&out.join("catalog_export_status.txt"))
}
