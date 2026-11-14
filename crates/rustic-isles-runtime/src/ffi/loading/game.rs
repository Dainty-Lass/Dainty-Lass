//! R7's game reads, writes and calls. The class and function lookups follow R17's reflection kit (spec-runtime-r17
//! 3.2 to 3.5) with the same names; they fold into `ffi::reflect` once R17 lands.

use super::super::memory::{self, query};
use super::super::{broker, detaching, engine};
use rustic_isles_runtime_core::memory::region_allows;
use rustic_isles_runtime_core::offsets::{data, layout, r7, target};
use rustic_isles_runtime_core::reflect;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::System::Memory::{
    PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_READWRITE, PAGE_WRITECOPY,
};

type ProcessEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
type ObjectFn = unsafe extern "C" fn(*mut c_void);
type WeakGetFn = unsafe extern "C" fn(*const [i32; 2]) -> *mut c_void;

static BASE: AtomicUsize = AtomicUsize::new(0);
static BEGIN: AtomicUsize = AtomicUsize::new(0);
static WEAK_GET: AtomicUsize = AtomicUsize::new(0);
static COMPLETE_LOAD: AtomicUsize = AtomicUsize::new(0);

/// Install: the image base and the three functions R7 calls (3.1).
pub fn set_base(base: usize) {
    let at = |rva: u32| {
        usize::try_from(rva)
            .ok()
            .and_then(|rva| base.checked_add(rva))
            .unwrap_or(0)
    };
    BASE.store(base, Ordering::SeqCst);
    set_calls(
        at(r7::LOAD_MANAGER_BEGIN),
        at(r7::WEAK_OBJECT_GET),
        at(r7::COMPLETE_LOAD_INTERNAL),
    );
}

/// The functions `Begin`, the weak resolver and `CompleteLoadInternal` are called at (tests point them at fakes).
pub fn set_calls(begin: usize, weak_get: usize, complete_load: usize) {
    BEGIN.store(begin, Ordering::SeqCst);
    WEAK_GET.store(weak_get, Ordering::SeqCst);
    COMPLETE_LOAD.store(complete_load, Ordering::SeqCst);
}

/// `base + rva`, 0 before install.
pub fn address(rva: u32) -> usize {
    let base = BASE.load(Ordering::SeqCst);
    if base == 0 {
        return 0;
    }
    usize::try_from(rva)
        .ok()
        .and_then(|rva| base.checked_add(rva))
        .unwrap_or(0)
}

/// `GetTickCount64`.
pub fn now() -> u64 {
    super::super::win::tick()
}

/// `GWorld`'s value (0 when unreadable or before install).
pub fn world() -> usize {
    match address(data::GWORLD) {
        0 => 0,
        at => read_ptr(at).unwrap_or(0),
    }
}

/// R0's `readable`.
pub fn readable(ptr: usize, size: usize) -> bool {
    memory::readable(ptr, size)
}

/// A pointer field.
pub fn read_ptr(addr: usize) -> Option<usize> {
    memory::read_unaligned::<usize>(addr)
}

/// A byte field.
pub fn read_u8(addr: usize) -> Option<u8> {
    memory::read_unaligned::<u8>(addr)
}

/// An `i32` field.
pub fn read_i32(addr: usize) -> Option<i32> {
    memory::read_unaligned::<i32>(addr)
}

/// A `u32` field.
pub fn read_u32(addr: usize) -> Option<u32> {
    memory::read_unaligned::<u32>(addr)
}

/// `is_live_object`.
pub fn is_live(ptr: usize) -> bool {
    #[cfg(test)]
    if fake::classes(ptr).is_some() {
        return true;
    }
    engine::is_live_object(ptr)
}

/// The short names of the object's class and its supers, at most 64 (R17 3.2's `class_chain`).
pub fn class_names(object: usize) -> Vec<String> {
    #[cfg(test)]
    if let Some(classes) = fake::classes(object) {
        return classes;
    }
    let Some(engine) = engine::engine() else {
        return Vec::new();
    };
    let Some(class) = reflect::class_of(engine.memory(), object) else {
        return Vec::new();
    };
    reflect::class_chain(engine.memory(), class)
        .into_iter()
        .map_while(|link| engine.object_name(link))
        .collect()
}

/// `is_a(object, class_name)` by short names.
pub fn is_a(object: usize, class_name: &str) -> bool {
    object != 0 && class_names(object).iter().any(|name| name == class_name)
}

/// The short name of the object's class.
pub fn class_name(object: usize) -> Option<String> {
    class_names(object).into_iter().next()
}

/// An object's full name.
pub fn full_name(object: usize) -> Option<String> {
    engine::full_name(object)
}

/// The object flags.
pub fn object_flags(object: usize) -> Option<u32> {
    memory::read_unaligned::<u32>(object.wrapping_add(layout::UOBJECT_FLAGS))
}

/// `find_function(class of object, name)`: the first `Function` field named `name` in its class chain.
pub fn find_function(object: usize, name: &str) -> Option<usize> {
    let engine = engine::engine()?;
    reflect::find_function(&engine, reflect::class_of(engine.memory(), object)?, name)
}

/// R17 3.3's guard: `ParmsSize` readable and at most the buffer.
pub fn params_fit(function: usize, buffer_len: usize) -> Result<(), Option<u16>> {
    match memory::read_unaligned::<u16>(function + r7::FUNCTION_PARMS_SIZE) {
        Some(size) if usize::from(size) <= buffer_len => Ok(()),
        size => Err(size),
    }
}

/// Whether `[addr, addr + size)` is mapped in one writable region (no alignment rule: R7 writes single bytes).
pub fn writable(addr: usize, size: usize) -> bool {
    let Some(region) = query(addr) else {
        return false;
    };
    let base = region.protect & 0xFF;
    region_allows(addr, size, Some(region), false)
        && matches!(
            base,
            PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
        )
}

/// A checked write (R17 3.5, without its alignment rule).
pub fn write<T: Copy>(addr: usize, value: T) -> bool {
    if !writable(addr, std::mem::size_of::<T>()) {
        return false;
    }
    // SAFETY: the range was just checked committed and writable
    unsafe { std::ptr::write_unaligned(addr as *mut T, value) };
    true
}

/// A zeroed, 8-aligned parameter block of at least `len` bytes (R17 3.3).
pub fn params(len: usize) -> Vec<u64> {
    vec![0; len.div_ceil(8).max(2)]
}

/// An `FString` parameter: R7 owns the NUL-terminated buffer for the call (R17 3.5).
pub struct FStringParam(Vec<u16>);

impl FStringParam {
    /// The string's buffer.
    pub fn new(text: &str) -> Self {
        Self(text.encode_utf16().chain(std::iter::once(0)).collect())
    }

    /// Writes data pointer, `Num` and `Max` (both `len + 1`) into the block's first 16 bytes.
    pub fn put(&self, block: &mut [u64]) {
        let count = u64::try_from(self.0.len()).unwrap_or(0) & 0xFFFF_FFFF;
        if let [data, sizes, ..] = block {
            *data = self.0.as_ptr() as u64;
            *sizes = count | (count << 32);
        }
    }
}

fn process_event_entry() -> usize {
    target("SP-2").map_or(0, |target| address(target.rva))
}

fn checked(object: usize, function: usize, block: &[u64]) -> bool {
    !detaching() && object != 0 && function != 0 && is_live(object) && params_fit(function, block.len() * 8).is_ok()
}

/// R17's `call`: `ProcessEvent` at its hooked entry, so the whole dispatch chain sees it.
pub fn call(object: usize, function: usize, block: &mut [u64]) -> bool {
    let entry = process_event_entry();
    if entry == 0 || !checked(object, function, block) {
        return false;
    }
    // SAFETY: the hooked ProcessEvent of this build; a live object, a function of its class chain and a block the
    // guard checked
    unsafe {
        let entry = std::mem::transmute::<usize, ProcessEventFn>(entry);
        entry(
            object as *mut c_void,
            function as *mut c_void,
            block.as_mut_ptr().cast(),
        );
    }
    true
}

/// `process_event_original`: past every wrapper of either DLL (3.4).
pub fn process_event_original(object: usize, function: usize, block: &mut [u64]) -> bool {
    let entry = process_event_entry();
    if entry == 0 || !checked(object, function, block) {
        return false;
    }
    let original = broker::original(entry);
    // SAFETY: the trampoline (or the function itself when unhooked) of ProcessEvent; arguments as for `call`
    unsafe {
        let original = std::mem::transmute::<usize, ProcessEventFn>(original);
        original(
            object as *mut c_void,
            function as *mut c_void,
            block.as_mut_ptr().cast(),
        );
    }
    true
}

fn call_object(function: usize, object: usize) -> bool {
    if function == 0 || object == 0 || detaching() {
        return false;
    }
    // SAFETY: `function` is `Begin` or `CompleteLoadInternal` (3.1), both `(object)`; the caller checked `object`
    unsafe {
        let function = std::mem::transmute::<usize, ObjectFn>(function);
        function(object as *mut c_void);
    }
    true
}

/// `UArchonLoadManager::Begin(manager)`, the caller having checked the manager.
pub fn begin(manager: usize) -> bool {
    call_object(BEGIN.load(Ordering::SeqCst), manager)
}

/// `CompleteLoadInternal(component)` at its hooked entry, so LD-6 records the completion (3.1).
pub fn complete_load_internal(component: usize) -> bool {
    call_object(COMPLETE_LOAD.load(Ordering::SeqCst), component)
}

/// The game's weak object resolver on R7's own copy of an entry's `{index, serial}`.
pub fn resolve_weak(weak: [i32; 2]) -> usize {
    let function = WEAK_GET.load(Ordering::SeqCst);
    if function == 0 {
        return 0;
    }
    // SAFETY: the resolver's signature (3.1); it bounds- and serial-checks the pair and reads only R7's stack copy
    unsafe {
        let function = std::mem::transmute::<usize, WeakGetFn>(function);
        function(&weak) as usize
    }
}

/// `original(LD-5)`: the predicate past every wrapper (8.4 step 5).
pub fn predicate_original(game_instance: usize) -> Option<bool> {
    type PredicateFn = unsafe extern "C" fn(*mut c_void) -> u8;
    let target = target("LD-5").map_or(0, |target| address(target.rva));
    if target == 0 || game_instance == 0 || detaching() {
        return None;
    }
    let original = broker::original(target);
    // SAFETY: the predicate's trampoline (or the predicate itself), `(game_instance) -> u8`, on a live instance
    let answer = unsafe { std::mem::transmute::<usize, PredicateFn>(original)(game_instance as *mut c_void) };
    Some(answer != 0)
}

/// HL-2's bytes are in place: this server has no phantom local player (3.3), read once.
pub fn no_phantom(server: bool) -> bool {
    use std::sync::OnceLock;
    static NO_PHANTOM: OnceLock<bool> = OnceLock::new();
    server
        && *NO_PHANTOM.get_or_init(|| {
            let site = address(r7::HL2_SITE);
            site != 0 && memory::read_unaligned::<[u8; 6]>(site) == Some(r7::HL2_APPLIED)
        })
}

#[cfg(test)]
pub mod fake {
    //! Objects the tests declare live, with their class chain's short names (no engine runs in a test).
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static OBJECTS: RefCell<HashMap<usize, Vec<&'static str>>> = RefCell::new(HashMap::new());
    }

    /// Declares `object` live, of `classes` (its class first).
    pub fn object(object: usize, classes: &[&'static str]) {
        OBJECTS.with(|objects| objects.borrow_mut().insert(object, classes.to_vec()));
    }

    /// Forgets every declared object.
    pub fn clear() {
        OBJECTS.with(|objects| objects.borrow_mut().clear());
    }

    pub(super) fn classes(object: usize) -> Option<Vec<String>> {
        OBJECTS.with(|objects| {
            objects
                .borrow()
                .get(&object)
                .map(|classes| classes.iter().map(|&name| name.to_owned()).collect())
        })
    }
}
