//! Finding classes, default objects and UFunctions through the engine's reflection data (spec-runtime-r17 section
//! 3), over R0's checked reads. A slice that calls game functions finds them here; the call itself is the DLL's.

use crate::engine::Engine;
use crate::memory::Memory;
use crate::names;
use crate::offsets::{layout, r17};
use std::collections::HashMap;
use std::sync::Mutex;

/// The deepest class chain walked (a cycle is a corrupt class).
pub const MAX_CHAIN: usize = 64;
/// The most fields walked in one class.
pub const MAX_FIELDS: usize = 8192;

fn at(base: usize, offset: usize) -> Option<usize> {
    base.checked_add(offset)
}

/// An object's class pointer.
pub fn class_of(memory: &dyn Memory, object: usize) -> Option<usize> {
    if object == 0 || !memory.readable(object, layout::UOBJECT_SIZE) {
        return None;
    }
    memory
        .ptr_at(at(object, layout::UOBJECT_CLASS)?)
        .filter(|&class| class != 0)
}

/// `class`, then each `SuperStruct`: at most 64, stopping at null or an unreadable class.
pub fn class_chain(memory: &dyn Memory, class: usize) -> Vec<usize> {
    let mut chain = Vec::new();
    let mut current = class;
    while current != 0 && chain.len() < MAX_CHAIN && memory.readable(current, r17::USTRUCT_CHILDREN + 8) {
        chain.push(current);
        match at(current, r17::USTRUCT_SUPER).and_then(|addr| memory.ptr_at(addr)) {
            Some(next) => current = next,
            None => break,
        }
    }
    chain
}

/// Whether `class` or one of its supers has the short name `class_name`.
pub fn class_is_a(engine: &Engine, class: usize, class_name: &str) -> bool {
    class_chain(engine.memory(), class)
        .into_iter()
        .any(|link| engine.object_name(link).as_deref() == Some(class_name))
}

/// `is_a(object, class_name)`: the object's class or one of its supers has that short name.
pub fn is_a(engine: &Engine, object: usize, class_name: &str) -> bool {
    class_of(engine.memory(), object).is_some_and(|class| class_is_a(engine, class, class_name))
}

/// The short name of an object's class.
pub fn class_name_of(engine: &Engine, object: usize) -> Option<String> {
    engine.object_name(class_of(engine.memory(), object)?)
}

fn fields(memory: &dyn Memory, class: usize) -> impl Iterator<Item = usize> + '_ {
    let first = at(class, r17::USTRUCT_CHILDREN).and_then(|addr| memory.ptr_at(addr));
    std::iter::successors(first.filter(|&field| field != 0), move |&field| {
        if !memory.readable(field, r17::UFIELD_NEXT + 8) {
            return None;
        }
        at(field, r17::UFIELD_NEXT)
            .and_then(|addr| memory.ptr_at(addr))
            .filter(|&next| next != 0)
    })
    .take(MAX_FIELDS)
}

fn is_function(engine: &Engine, field: usize) -> bool {
    class_name_of(engine, field).as_deref() == Some("Function")
}

/// The first `Function` field named `name` in `class` or its supers.
pub fn find_function(engine: &Engine, class: usize, name: &str) -> Option<usize> {
    class_chain(engine.memory(), class).into_iter().find_map(|link| {
        fields(engine.memory(), link)
            .find(|&field| engine.object_name(field).as_deref() == Some(name) && is_function(engine, field))
    })
}

/// The first `Function` field of `class` itself (no supers) whose name passes `pred`.
pub fn find_function_matching(engine: &Engine, class: usize, pred: impl Fn(&str) -> bool) -> Option<usize> {
    if class_chain(engine.memory(), class).is_empty() {
        return None;
    }
    fields(engine.memory(), class)
        .find(|&field| engine.object_name(field).is_some_and(|name| pred(&name)) && is_function(engine, field))
}

/// One object array slot's header, read in one go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// The object.
    pub object: usize,
    /// Its object flags.
    pub flags: u32,
    /// Its class.
    pub class: usize,
    /// Its name's comparison index.
    pub name_index: u32,
    /// Its name's number.
    pub name_number: u32,
}

fn header(memory: &dyn Memory, object: usize) -> Option<Header> {
    if !memory.readable(object, layout::UOBJECT_SIZE) {
        return None;
    }
    let mut bytes = [0u8; layout::UOBJECT_SIZE];
    if !memory.read(object, &mut bytes) {
        return None;
    }
    let word =
        |offset: usize| u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]);
    let class = u64::from(word(layout::UOBJECT_CLASS)) | (u64::from(word(layout::UOBJECT_CLASS + 4)) << 32);
    Some(Header {
        object,
        flags: word(layout::UOBJECT_FLAGS),
        class: usize::try_from(class).ok()?,
        name_index: word(layout::UOBJECT_NAME),
        name_number: word(layout::UOBJECT_NAME + 4),
    })
}

/// Visits every object in the array by index until `visit` returns true; returns that object. A chunk's items are
/// read in one go, since every read is a `VirtualQuery`.
pub fn scan(engine: &Engine, mut visit: impl FnMut(&Header) -> bool) -> Option<usize> {
    let memory = engine.memory();
    let count = usize::try_from(engine.object_count()?).ok()?;
    let objects = engine.objects();
    let chunks = usize::try_from(memory.i32_at(at(objects, layout::OBJECTS_CHUNK_COUNT)?)?).ok()?;
    let table = memory.ptr_at(at(objects, layout::OBJECTS_CHUNKS)?)?;
    let mut items = Vec::new();
    for chunk in 0..chunks {
        let first = chunk * layout::OBJECTS_PER_CHUNK;
        if first >= count {
            break;
        }
        let in_chunk = (count - first).min(layout::OBJECTS_PER_CHUNK);
        let Some(chunk_items) = at(table, chunk * 8).and_then(|addr| memory.ptr_at(addr)) else {
            continue;
        };
        items.resize(in_chunk * layout::OBJECT_ITEM_SIZE, 0);
        if chunk_items == 0 || !memory.read(chunk_items, &mut items) {
            continue;
        }
        for item in items.as_chunks::<{ layout::OBJECT_ITEM_SIZE }>().0 {
            let mut pointer = [0u8; 8];
            pointer.copy_from_slice(&item[..8]);
            let Ok(object) = usize::try_from(u64::from_le_bytes(pointer)) else {
                continue;
            };
            if object == 0 {
                continue;
            }
            if let Some(header) = header(memory, object)
                && visit(&header)
            {
                return Some(object);
            }
        }
    }
    None
}

/// Visits at most `budget` array slots from index `start`, in the order [`scan`] does; returns the index to start the
/// next call at, 0 once past the last object. A chunk is read only for its slots in the range.
pub fn scan_slice(engine: &Engine, start: usize, budget: usize, mut visit: impl FnMut(&Header)) -> usize {
    let memory = engine.memory();
    let Some(count) = engine.object_count().and_then(|count| usize::try_from(count).ok()) else {
        return 0;
    };
    let objects = engine.objects();
    let Some(table) = at(objects, layout::OBJECTS_CHUNKS).and_then(|addr| memory.ptr_at(addr)) else {
        return 0;
    };
    if start >= count {
        return 0;
    }
    let end = start.saturating_add(budget).min(count);
    let mut index = start;
    let mut items = Vec::new();
    while index < end {
        let chunk = index / layout::OBJECTS_PER_CHUNK;
        let in_chunk = index % layout::OBJECTS_PER_CHUNK;
        let take = (end - index).min(layout::OBJECTS_PER_CHUNK - in_chunk);
        let chunk_items = at(table, chunk * 8).and_then(|addr| memory.ptr_at(addr)).unwrap_or(0);
        items.resize(take * layout::OBJECT_ITEM_SIZE, 0);
        let first = chunk_items.checked_add(in_chunk * layout::OBJECT_ITEM_SIZE);
        if chunk_items != 0
            && let Some(first) = first
            && memory.read(first, &mut items)
        {
            for item in items.as_chunks::<{ layout::OBJECT_ITEM_SIZE }>().0 {
                let mut pointer = [0u8; 8];
                pointer.copy_from_slice(&item[..8]);
                if let Ok(object) = usize::try_from(u64::from_le_bytes(pointer))
                    && object != 0
                    && let Some(header) = header(memory, object)
                {
                    visit(&header);
                }
            }
        }
        index += take;
    }
    if end >= count { 0 } else { end }
}

fn header_name(engine: &Engine, header: &Header) -> Option<String> {
    let text = engine.decode_name(header.name_index)?;
    Some(names::with_number(names::short(&text), header.name_number))
}

/// Whether a class's own class makes it a class (`Class` or any `…GeneratedClass`).
pub fn is_class_class(name: &str) -> bool {
    name == "Class" || name.ends_with("GeneratedClass")
}

/// `find_class(name)`, uncached: the first object by index named `name` whose class is a class class.
pub fn find_class_uncached(engine: &Engine, name: &str) -> Option<usize> {
    scan(engine, |header| {
        header_name(engine, header).as_deref() == Some(name)
            && engine
                .object_name(header.class)
                .is_some_and(|class| is_class_class(&class))
    })
}

/// Classes by name, once found (a class is never collected).
#[derive(Default)]
pub struct ClassCache {
    found: Mutex<HashMap<String, usize>>,
}

impl ClassCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// `find_class(name)`, cached once found.
    pub fn find(&self, engine: &Engine, name: &str) -> Option<usize> {
        if let Ok(found) = self.found.lock()
            && let Some(&class) = found.get(name)
        {
            return Some(class);
        }
        let class = find_class_uncached(engine, name)?;
        if let Ok(mut found) = self.found.lock() {
            found.insert(name.to_owned(), class);
        }
        Some(class)
    }
}

/// The class's default object, required live.
pub fn default_object(engine: &Engine, class: usize) -> Option<usize> {
    let memory = engine.memory();
    if !memory.readable(class, r17::UCLASS_DEFAULT_OBJECT + 8) {
        return None;
    }
    memory
        .ptr_at(at(class, r17::UCLASS_DEFAULT_OBJECT)?)
        .filter(|&object| engine.is_live_object(object))
}

/// The first object by index that is not a default object and `is_a(class_name)`. A full scan.
pub fn first_instance(engine: &Engine, class_name: &str) -> Option<usize> {
    let mut answers: HashMap<usize, bool> = HashMap::new();
    scan(engine, |header| {
        header.flags & r17::RF_CLASS_DEFAULT_OBJECT == 0
            && *answers
                .entry(header.class)
                .or_insert_with(|| class_is_a(engine, header.class, class_name))
    })
}

/// The parameter block R17 lays out for a function whose dump struct is `expected` bytes.
pub fn param_buffer_len(expected: usize) -> usize {
    expected.next_multiple_of(8).max(16)
}

/// The parameter guard: `Ok` when `ParmsSize` is readable and fits `buffer_len`; `Err(Some(n))` when it does not
/// fit, `Err(None)` when it cannot be read.
pub fn params_fit(memory: &dyn Memory, function: usize, buffer_len: usize) -> Result<(), Option<u16>> {
    let size = at(function, r17::UFUNCTION_PARMS_SIZE)
        .and_then(|addr| memory.u16_at(addr))
        .ok_or(None)?;
    if usize::from(size) <= buffer_len {
        Ok(())
    } else {
        Err(Some(size))
    }
}

/// The guard's log line for a skipped call.
pub fn skipped_line(label: &str, size: Option<u16>, buffer_len: usize) -> String {
    let size = size.map_or_else(|| "unreadable".to_owned(), |size| size.to_string());
    format!("[MitBranding] {label}: parameter size {size} exceeds {buffer_len}, call skipped")
}

/// Whether a region's protection lets R17 write data into it (3.5).
pub fn is_writable_protect(protect: u32) -> bool {
    const WRITABLE: [u32; 4] = [0x04, 0x40, 0x08, 0x80];
    protect & (crate::memory::PAGE_GUARD | crate::memory::PAGE_NOACCESS) == 0 && WRITABLE.contains(&(protect & 0xFF))
}

/// An `FString` read through checked reads: at most `max` UTF-16 units, without the terminating NUL.
pub fn read_fstring(memory: &dyn Memory, string: usize, max: usize) -> Option<String> {
    let data = memory.ptr_at(string)?;
    let num = memory.i32_at(at(string, 8)?)?;
    let units = usize::try_from(num).ok()?.saturating_sub(1).min(max);
    if data == 0 || units == 0 {
        return Some(String::new());
    }
    let mut bytes = vec![0u8; units * 2];
    if !memory.read(data, &mut bytes) {
        return None;
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|&unit| unit != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::memory::BufferMemory;

    const BASE: usize = 0x20_0000;
    const OBJECTS: usize = BASE;
    const NAMES: usize = BASE + 0x100;
    const CHUNK_TABLE: usize = BASE + 0x200;
    const CHUNK: usize = BASE + 0x400;
    const BLOCK: usize = BASE + 0x2000;
    const OBJ: usize = BASE + 0x4000;
    const SLOT: usize = 0x200;
    const SIZE: usize = 0x20000;

    /// A synthetic class graph laid out as the engine lays it out.
    struct World {
        bytes: Vec<u8>,
        cursor: usize,
        count: i32,
        names: HashMap<String, u32>,
    }

    impl World {
        fn new() -> Self {
            let mut world = Self {
                bytes: vec![0u8; SIZE],
                cursor: 0,
                count: 0,
                names: HashMap::new(),
            };
            world.put_ptr(OBJECTS, CHUNK_TABLE);
            world.put(OBJECTS + 0x1C, &1i32.to_le_bytes());
            world.put_ptr(CHUNK_TABLE, CHUNK);
            world.put_ptr(NAMES + 0x10, BLOCK);
            world
        }
        fn put(&mut self, at: usize, value: &[u8]) {
            let at = at - BASE;
            self.bytes[at..at + value.len()].copy_from_slice(value);
        }
        fn put_ptr(&mut self, at: usize, value: usize) {
            self.put(at, &(value as u64).to_le_bytes());
        }
        fn name(&mut self, text: &str) -> u32 {
            if let Some(&index) = self.names.get(text) {
                return index;
            }
            let offset = self.cursor;
            let header = u16::try_from(text.len()).unwrap() << 6;
            self.put(BLOCK + offset, &header.to_le_bytes());
            self.put(BLOCK + offset + 2, text.as_bytes());
            self.cursor += (2 + text.len()).div_ceil(2) * 2;
            let index = u32::try_from(offset / 2).unwrap();
            self.names.insert(text.to_owned(), index);
            index
        }
        /// A new object in the next slot, listed in the object array.
        fn object(&mut self, class: usize, name: &str, outer: usize) -> usize {
            let index = self.count;
            self.count += 1;
            self.put(OBJECTS + 0x14, &self.count.to_le_bytes());
            let at = OBJ + usize::try_from(index).unwrap() * SLOT;
            let name = self.name(name);
            self.put(at + 0x0C, &index.to_le_bytes());
            self.put_ptr(at + 0x10, class);
            self.put(at + 0x18, &name.to_le_bytes());
            self.put_ptr(at + 0x20, outer);
            self.put_ptr(CHUNK + usize::try_from(index).unwrap() * 0x18, at);
            at
        }
        fn set_class(&mut self, object: usize, class: usize) {
            self.put_ptr(object + 0x10, class);
        }
        fn super_of(&mut self, class: usize, parent: usize) {
            self.put_ptr(class + r17::USTRUCT_SUPER, parent);
        }
        fn children(&mut self, class: usize, fields: &[usize]) {
            self.put_ptr(class + r17::USTRUCT_CHILDREN, fields.first().copied().unwrap_or(0));
            for pair in fields.windows(2) {
                self.put_ptr(pair[0] + r17::UFIELD_NEXT, pair[1]);
            }
        }
        fn memory(&self) -> BufferMemory<'_> {
            BufferMemory {
                base: BASE,
                bytes: &self.bytes,
            }
        }
    }

    /// Package, Class, Function, Property; classes Object <- Widget <- TextBlock; a BP class with a generated
    /// meta-class; functions on Widget and TextBlock.
    struct Graph {
        world: World,
        object_class: usize,
        widget: usize,
        text_block: usize,
        set_visibility: usize,
        set_text: usize,
        bp: usize,
        clicked: usize,
        decoy: usize,
    }

    fn graph() -> Graph {
        let mut world = World::new();
        let class = world.object(0, "Class", 0);
        world.set_class(class, class);
        let function_class = world.object(class, "Function", 0);
        let property_class = world.object(class, "Property", 0);
        let generated = world.object(class, "WidgetBlueprintGeneratedClass", 0);
        let object_class = world.object(class, "Object", 0);
        let widget = world.object(class, "Widget", 0);
        let text_block = world.object(class, "TextBlock", 0);
        world.super_of(widget, object_class);
        world.super_of(text_block, widget);
        let set_visibility = world.object(function_class, "SetVisibility", widget);
        let decoy = world.object(property_class, "SetText", text_block);
        let set_text = world.object(function_class, "SetText", text_block);
        world.children(widget, &[set_visibility]);
        world.children(text_block, &[decoy, set_text]);
        let bp = world.object(generated, "LoginScreen_bps_C", 0);
        world.super_of(bp, widget);
        let clicked = world.object(function_class, "BndEvt__w_loginbutton_K2Node_3_Clicked", bp);
        world.children(bp, &[clicked]);
        Graph {
            world,
            object_class,
            widget,
            text_block,
            set_visibility,
            set_text,
            bp,
            clicked,
            decoy,
        }
    }

    #[test]
    fn chains_stop() {
        let mut g = graph();
        let memory = g.world.memory();
        assert_eq!(
            class_chain(&memory, g.text_block),
            [g.text_block, g.widget, g.object_class]
        );
        assert_eq!(class_chain(&memory, 0), Vec::<usize>::new());
        g.world.super_of(g.object_class, BASE + SIZE + 0x1000);
        let memory = g.world.memory();
        assert_eq!(class_chain(&memory, g.text_block).len(), 3, "unreadable super ends it");
        g.world.super_of(g.object_class, g.text_block);
        let memory = g.world.memory();
        assert_eq!(
            class_chain(&memory, g.text_block).len(),
            MAX_CHAIN,
            "a cycle stops at 64"
        );
    }

    #[test]
    fn is_a_by_short_name() {
        let mut g = graph();
        let label = g.world.object(g.text_block, "Label", 0);
        let memory = g.world.memory();
        let engine = Engine::new(&memory, OBJECTS, NAMES, None);
        for name in ["TextBlock", "Widget", "Object"] {
            assert!(is_a(&engine, label, name), "{name}");
        }
        assert!(!is_a(&engine, label, "Image"));
        assert!(!is_a(&engine, 0, "Object"));
        assert_eq!(
            class_name_of(&engine, g.bp).as_deref(),
            Some("WidgetBlueprintGeneratedClass")
        );
        assert_eq!(class_name_of(&engine, label).as_deref(), Some("TextBlock"));
    }

    #[test]
    fn slices_visit_what_a_scan_does() {
        let mut g = graph();
        for n in 0..5 {
            g.world.object(g.text_block, &format!("Label{n}"), 0);
        }
        let memory = g.world.memory();
        let engine = Engine::new(&memory, OBJECTS, NAMES, None);
        let mut whole = Vec::new();
        scan(&engine, |header| {
            whole.push(header.object);
            false
        });
        let mut sliced = Vec::new();
        let mut start = 0;
        loop {
            start = scan_slice(&engine, start, 3, |header| sliced.push(header.object));
            if start == 0 {
                break;
            }
        }
        assert!(whole.len() > 5);
        assert_eq!(sliced, whole);
        assert_eq!(
            scan_slice(&engine, usize::MAX, 3, |_| {}),
            0,
            "past the end starts over"
        );
    }

    #[test]
    fn functions() {
        let mut g = graph();
        let memory = g.world.memory();
        let engine = Engine::new(&memory, OBJECTS, NAMES, None);
        assert_eq!(
            find_function(&engine, g.text_block, "SetVisibility"),
            Some(g.set_visibility),
            "on a super"
        );
        assert_eq!(
            find_function(&engine, g.text_block, "SetText"),
            Some(g.set_text),
            "skips the property"
        );
        assert_ne!(find_function(&engine, g.text_block, "SetText"), Some(g.decoy));
        assert_eq!(find_function(&engine, g.widget, "SetText"), None);
        assert_eq!(
            find_function_matching(&engine, g.bp, |name| name.contains("w_loginbutton")
                && name.contains("Clicked")),
            Some(g.clicked)
        );
        assert_eq!(
            find_function_matching(&engine, g.bp, |name| name == "SetVisibility"),
            None,
            "own class only"
        );
        // A field whose Next is itself stands for a list longer than the bound
        let function_class = find_class_uncached(&engine, "Function").unwrap();
        let filler = g.world.object(function_class, "Filler", g.widget);
        g.world.put_ptr(filler + r17::UFIELD_NEXT, filler);
        g.world.put_ptr(g.widget + r17::USTRUCT_CHILDREN, filler);
        let memory = g.world.memory();
        let engine = Engine::new(&memory, OBJECTS, NAMES, None);
        assert_eq!(
            find_function(&engine, g.widget, "SetVisibility"),
            None,
            "stops at 8 192 fields"
        );
        assert_eq!(fields(&memory, g.widget).count(), MAX_FIELDS);
    }

    #[test]
    fn classes_and_default_objects() {
        let mut g = graph();
        let impostor = g.world.object(g.widget, "TextBlock", 0);
        let cdo = g.world.object(g.text_block, "Default__TextBlock", 0);
        g.world.put(cdo + 0x08, &r17::RF_CLASS_DEFAULT_OBJECT.to_le_bytes());
        g.world.put_ptr(g.text_block + r17::UCLASS_DEFAULT_OBJECT, cdo);
        g.world.put_ptr(g.widget + r17::UCLASS_DEFAULT_OBJECT, BASE + 0x3000);
        let live = g.world.object(g.text_block, "TextBlock_0", 0);
        let memory = g.world.memory();
        let engine = Engine::new(&memory, OBJECTS, NAMES, None);
        assert_eq!(find_class_uncached(&engine, "TextBlock"), Some(g.text_block));
        assert_eq!(
            find_class_uncached(&engine, "LoginScreen_bps_C"),
            Some(g.bp),
            "a generated class"
        );
        assert_eq!(find_class_uncached(&engine, "Label"), None);
        assert_ne!(find_class_uncached(&engine, "TextBlock"), Some(impostor));
        let cache = ClassCache::new();
        assert_eq!(cache.find(&engine, "Widget"), Some(g.widget));
        assert_eq!(cache.find(&engine, "Widget"), Some(g.widget));
        assert_eq!(default_object(&engine, g.text_block), Some(cdo));
        assert_eq!(default_object(&engine, g.widget), None, "dead pointer");
        assert_eq!(
            first_instance(&engine, "TextBlock"),
            Some(live),
            "skips the default object"
        );
        assert_eq!(first_instance(&engine, "Image"), None);
        assert!(is_class_class("Class"));
        assert!(is_class_class("BlueprintGeneratedClass"));
        assert!(!is_class_class("Function"));
    }

    #[test]
    fn parameter_guard() {
        let mut g = graph();
        g.world
            .put(g.set_text + r17::UFUNCTION_PARMS_SIZE, &0x18u16.to_le_bytes());
        let memory = g.world.memory();
        assert_eq!(params_fit(&memory, g.set_text, 0x18), Ok(()), "equal");
        assert_eq!(params_fit(&memory, g.set_text, 0x20), Ok(()), "smaller");
        assert_eq!(params_fit(&memory, g.set_text, 0x10), Err(Some(0x18)), "larger");
        assert_eq!(params_fit(&memory, BASE + SIZE, 0x10), Err(None), "unreadable");
        assert_eq!(param_buffer_len(1), 16);
        assert_eq!(param_buffer_len(0), 16);
        assert_eq!(param_buffer_len(0x18), 0x18);
        assert_eq!(param_buffer_len(0x19), 0x20);
        assert_eq!(param_buffer_len(0x58), 0x58);
        assert_eq!(
            skipped_line("TextBlock.SetText", Some(0x30), 0x18),
            "[MitBranding] TextBlock.SetText: parameter size 48 exceeds 24, call skipped"
        );
    }

    #[test]
    fn writable_protections() {
        for protect in [0x04, 0x40, 0x08, 0x80] {
            assert!(is_writable_protect(protect), "{protect:#x}");
        }
        for protect in [0x01, 0x02, 0x10, 0x20, 0x04 | 0x100, 0] {
            assert!(!is_writable_protect(protect), "{protect:#x}");
        }
    }

    #[test]
    fn fstrings() {
        let mut world = World::new();
        let text: Vec<u8> = "pt-BR\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        world.put(BASE + 0x1000, &text);
        world.put_ptr(BASE + 0x1100, BASE + 0x1000);
        world.put(BASE + 0x1108, &6i32.to_le_bytes());
        let memory = world.memory();
        assert_eq!(read_fstring(&memory, BASE + 0x1100, 32).as_deref(), Some("pt-BR"));
        assert_eq!(read_fstring(&memory, BASE + 0x1100, 2).as_deref(), Some("pt"));
        assert_eq!(read_fstring(&memory, BASE + SIZE, 32), None);
    }
}
