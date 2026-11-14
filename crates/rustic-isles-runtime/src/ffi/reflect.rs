//! Calling game functions (spec-runtime-r17 3.4, 3.5): the one `ProcessEvent` call, checked writes of game data,
//! parameter blocks, `FString` parameters and the process-lifetime `FText`s. Lookups are the core crate's.

use super::memory::{ProcessMemory, query, readable};
use super::{detaching, engine, log, win};
use rustic_isles_runtime_core::engine::Engine;
use rustic_isles_runtime_core::memory::Memory;
use rustic_isles_runtime_core::offsets::target;
use rustic_isles_runtime_core::reflect::{self, ClassCache, is_writable_protect, param_buffer_len, skipped_line};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{LazyLock, Mutex};

type ProcessEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

static CLASSES: LazyLock<ClassCache> = LazyLock::new(ClassCache::new);
static FUNCTIONS: LazyLock<Mutex<HashMap<(usize, String), usize>>> = LazyLock::new(Mutex::default);
static SKIPPED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);
static TEXTS: LazyLock<Mutex<HashMap<&'static str, [u8; FTEXT_SIZE]>>> = LazyLock::new(Mutex::default);

/// `FText`'s size: opaque, only ever passed back to the game.
pub const FTEXT_SIZE: usize = 0x18;

/// The engine over this process, or `None` before Init.
pub fn engine() -> Option<Engine<'static>> {
    engine::engine()
}

/// A pointer read through the region rule.
pub fn ptr(addr: usize) -> Option<usize> {
    ProcessMemory.ptr_at(addr)
}

/// A non-null pointer at `addr` to a live object.
pub fn live_ptr(addr: usize) -> Option<usize> {
    ptr(addr).filter(|&object| object != 0 && engine::is_live_object(object))
}

/// A byte.
pub fn u8_at(addr: usize) -> Option<u8> {
    ProcessMemory.u8_at(addr)
}

/// `count` bytes.
pub fn bytes_at(addr: usize, count: usize) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; count];
    ProcessMemory.read(addr, &mut bytes).then_some(bytes)
}

/// An object's class.
pub fn class_of(object: usize) -> Option<usize> {
    reflect::class_of(&ProcessMemory, object)
}

/// `is_a(object, class_name)`, by short names.
pub fn is_a(object: usize, class_name: &str) -> bool {
    engine().is_some_and(|engine| reflect::is_a(&engine, object, class_name))
}

/// The short name of an object's class.
pub fn class_name_of(object: usize) -> Option<String> {
    reflect::class_name_of(&engine()?, object)
}

/// `find_class(name)`, cached once found.
pub fn find_class(name: &str) -> Option<usize> {
    CLASSES.find(&engine()?, name)
}

/// `first_instance(class_name)`: a full scan.
pub fn first_instance(class_name: &str) -> Option<usize> {
    reflect::first_instance(&engine()?, class_name)
}

/// The class's default object.
pub fn default_object(class: usize) -> Option<usize> {
    reflect::default_object(&engine()?, class)
}

/// `find_function(class, name)`, cached once found (a class and its functions are never collected).
pub fn function(class: usize, name: &str) -> Option<usize> {
    let key = (class, name.to_owned());
    if let Ok(found) = FUNCTIONS.lock()
        && let Some(&function) = found.get(&key)
    {
        return Some(function);
    }
    let function = reflect::find_function(&engine()?, class, name)?;
    if let Ok(mut found) = FUNCTIONS.lock() {
        found.insert(key, function);
    }
    Some(function)
}

/// `function_matching(object's own class, pred)`, uncached.
pub fn function_matching(object: usize, pred: impl Fn(&str) -> bool) -> Option<usize> {
    reflect::find_function_matching(&engine()?, class_of(object)?, pred)
}

/// A zeroed parameter block, 8-byte aligned, `max(round_up(expected, 8), 16)` bytes (3.3).
pub struct Params {
    words: Vec<u64>,
}

impl Params {
    /// The block for a function whose parameter struct is `expected` bytes.
    pub fn new(expected: usize) -> Self {
        Self {
            words: vec![0; param_buffer_len(expected) / 8],
        }
    }

    /// Its length in bytes.
    pub fn len(&self) -> usize {
        self.words.len() * 8
    }

    /// Never: the block is at least 16 bytes.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Copies `bytes` in at `offset`; past the end is dropped.
    pub fn put(&mut self, offset: usize, bytes: &[u8]) {
        for (i, &byte) in bytes.iter().enumerate() {
            let at = offset + i;
            if let Some(word) = self.words.get_mut(at / 8) {
                let shift = (at % 8) * 8;
                *word = (*word & !(0xFF << shift)) | (u64::from(byte) << shift);
            }
        }
    }

    /// A pointer at `offset`.
    pub fn put_ptr(&mut self, offset: usize, value: usize) {
        self.put(offset, &(value as u64).to_le_bytes());
    }

    /// `count` bytes from `offset`.
    pub fn get(&self, offset: usize, count: usize) -> Vec<u8> {
        (offset..offset + count)
            .map(|at| self.words.get(at / 8).map_or(0, |word| (word >> ((at % 8) * 8)) as u8))
            .collect()
    }

    /// The pointer at `offset`.
    pub fn get_ptr(&self, offset: usize) -> usize {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.get(offset, 8));
        usize::try_from(u64::from_le_bytes(bytes)).unwrap_or(0)
    }

    /// Where the block lives (for reading a returned value through checked reads).
    pub fn address(&self) -> usize {
        self.words.as_ptr() as usize
    }

    fn as_mut_ptr(&mut self) -> *mut c_void {
        self.words.as_mut_ptr().cast()
    }
}

/// An `FString` parameter: R17 owns the NUL-terminated buffer and keeps it alive across the call (3.5).
pub struct FStringParam {
    units: Vec<u16>,
}

impl FStringParam {
    /// The string's buffer.
    pub fn new(text: &str) -> Self {
        let mut units: Vec<u16> = text.encode_utf16().collect();
        units.push(0);
        Self { units }
    }

    /// Writes data pointer, `Num = len + 1` and `Max = len + 1` at `offset`.
    pub fn put(&self, params: &mut Params, offset: usize) {
        let count = i32::try_from(self.units.len()).unwrap_or(0);
        params.put_ptr(offset, self.units.as_ptr() as usize);
        params.put(offset + 8, &count.to_le_bytes());
        params.put(offset + 12, &count.to_le_bytes());
    }
}

fn process_event() -> Option<usize> {
    let rva = usize::try_from(target("SP-1")?.rva).ok()?;
    win::image_base().checked_add(rva)
}

fn skip_once(object: usize, function: usize, size: Option<u16>, len: usize) {
    let class = class_name_of(object).unwrap_or_else(|| "null".to_owned());
    let name = engine()
        .and_then(|engine| engine.object_name(function))
        .unwrap_or_else(|| "null".to_owned());
    let label = format!("{class}.{name}");
    if SKIPPED.lock().is_ok_and(|mut seen| seen.insert(label.clone())) {
        log::line(&skipped_line(&label, size, len));
    }
}

/// `call(object, function, params)`: `ProcessEvent` at its hooked entry, so the call goes down the whole SP-1
/// chain as the game's own calls do (R17 call 4). `function` must come from the object's class chain.
pub fn call(object: usize, function: usize, params: &mut Params) -> bool {
    if detaching() || object == 0 || function == 0 || !engine::is_live_object(object) {
        return false;
    }
    if let Err(size) = reflect::params_fit(&ProcessMemory, function, params.len()) {
        skip_once(object, function, size, params.len());
        return false;
    }
    let Some(entry) = process_event() else {
        return false;
    };
    // SAFETY: the hooked ProcessEvent entry of this build (RT-2); a live object, a function of its class chain
    // and a parameter block the guard checked is at least the function's ParmsSize
    unsafe {
        let entry: ProcessEventFn = std::mem::transmute::<usize, ProcessEventFn>(entry);
        entry(object as *mut c_void, function as *mut c_void, params.as_mut_ptr());
    }
    true
}

/// Finds `name` in the object's class chain and calls it; false when not found or not called.
pub fn invoke(object: usize, name: &str, params: &mut Params) -> bool {
    match class_of(object).and_then(|class| function(class, name)) {
        Some(function) => call(object, function, params),
        None => false,
    }
}

/// A Blueprint library function, called on its class's default object.
pub fn static_call(class_name: &str, function_name: &str, params: &mut Params) -> bool {
    match find_class(class_name).and_then(default_object) {
        Some(object) => invoke(object, function_name, params),
        None => false,
    }
}

/// Whether `[addr, addr + size)` is readable (R0's rule) and in a writable region.
pub fn writable(addr: usize, size: usize) -> bool {
    readable(addr, size) && query(addr).is_some_and(|region| is_writable_protect(region.protect))
}

/// `write<T>(addr, value)`: readable, in a writable region, then a volatile write (3.5).
pub fn write<T: Copy>(addr: usize, value: T) -> bool {
    if !writable(addr, std::mem::size_of::<T>()) {
        return false;
    }
    // SAFETY: the range was just checked committed, readable, 8-byte aligned and writable
    unsafe { std::ptr::write_volatile(addr as *mut T, value) };
    true
}

/// `text_for(s)`: an `FText` made once per text by `Conv_StringToText` and kept for the process (R17 call 5).
pub fn text_for(text: &'static str) -> Option<[u8; FTEXT_SIZE]> {
    if let Ok(texts) = TEXTS.lock()
        && let Some(made) = texts.get(text)
    {
        return Some(*made);
    }
    let string = FStringParam::new(text);
    let mut params = Params::new(0x28);
    string.put(&mut params, 0);
    if !static_call("KismetTextLibrary", "Conv_StringToText", &mut params) {
        return None;
    }
    let mut made = [0u8; FTEXT_SIZE];
    made.copy_from_slice(&params.get(0x10, FTEXT_SIZE));
    if made.iter().all(|&byte| byte == 0) {
        return None;
    }
    if let Ok(mut texts) = TEXTS.lock() {
        texts.insert(text, made);
    }
    Some(made)
}

/// `TextBlock.SetText(text)` on `widget`.
pub fn set_text(widget: usize, text: &'static str) -> bool {
    let Some(made) = text_for(text) else {
        return false;
    };
    let mut params = Params::new(FTEXT_SIZE);
    params.put(0, &made);
    invoke(widget, "SetText", &mut params)
}

/// `KismetStringLibrary.Conv_StringToName(text)`: the `FName`'s 8 bytes.
pub fn name_for(text: &str) -> Option<[u8; 8]> {
    let string = FStringParam::new(text);
    let mut params = Params::new(0x18);
    string.put(&mut params, 0);
    if !static_call("KismetStringLibrary", "Conv_StringToName", &mut params) {
        return None;
    }
    let mut name = [0u8; 8];
    name.copy_from_slice(&params.get(0x10, 8));
    Some(name)
}

/// `KismetInternationalizationLibrary.GetCurrentCulture()`, at most 32 characters. The engine's string is not
/// freed (once per process).
pub fn current_culture() -> Option<String> {
    let mut params = Params::new(0x10);
    if !static_call("KismetInternationalizationLibrary", "GetCurrentCulture", &mut params) {
        return None;
    }
    reflect::read_fstring(&ProcessMemory, params.address(), 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_blocks() {
        let mut params = Params::new(0x19);
        assert_eq!(params.len(), 0x20);
        assert_eq!(params.address() % 8, 0);
        params.put(3, &[0xAA, 0xBB]);
        assert_eq!(params.get(2, 4), [0, 0xAA, 0xBB, 0]);
        params.put_ptr(8, 0x1122_3344_5566_7788);
        assert_eq!(params.get_ptr(8), 0x1122_3344_5566_7788);
        params.put(0x1F, &[1, 2, 3]);
        assert_eq!(params.get(0x1F, 2), [1, 0], "past the end is dropped");
        assert_eq!(Params::new(1).len(), 16);
    }

    #[test]
    fn fstring_parameters() {
        let string = FStringParam::new("F2");
        let mut params = Params::new(0x18);
        string.put(&mut params, 0);
        assert_eq!(params.get(8, 8), [3, 0, 0, 0, 3, 0, 0, 0]);
        assert_eq!(
            reflect::read_fstring(&ProcessMemory, params.address(), 32).as_deref(),
            Some("F2")
        );
    }

    #[test]
    fn writes_need_a_writable_region() {
        let mut word = Box::new(0u64);
        let addr = &mut *word as *mut u64 as usize;
        assert!(write::<u64>(addr, 7));
        assert_eq!(*word, 7);
        assert!(!write::<u64>(0, 7));
        assert!(!write::<u64>(addr + 4, 7), "unaligned");
        let code = write::<u8> as *const () as usize & !7;
        assert!(!write::<u8>(code, 0), "code is not writable");
    }
}
