//! The game behind `core::repgraph::Game`: R0's checked reads, the liveness test, the found classes (5.1).

use crate::ffi::memory::{function_entry, read_bytes, readable, readable_this_frame};
use crate::ffi::{engine, listen, log, reflect, win};
use rustic_isles_runtime_core::repgraph::{Class, Game, SECOND_MS};
use rustic_isles_runtime_core::widgets::image_size;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static IMAGE_BASE: AtomicUsize = AtomicUsize::new(0);
static IMAGE_SIZE: AtomicUsize = AtomicUsize::new(0);
static FOUND: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static TRIED_MS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// Init records the executable's range (for the drive's slot check and the caller's RVA).
pub fn set_image(base: usize) {
    IMAGE_BASE.store(base, Ordering::SeqCst);
    let size = image_size(&crate::ffi::memory::ProcessMemory, base).unwrap_or(0);
    IMAGE_SIZE.store(size, Ordering::SeqCst);
}

/// The executable-relative address of `address`.
pub fn image_rva(address: usize) -> Option<u32> {
    let base = IMAGE_BASE.load(Ordering::Relaxed);
    let size = IMAGE_SIZE.load(Ordering::Relaxed);
    let offset = address.checked_sub(base)?;
    if base == 0 || offset >= size {
        return None;
    }
    u32::try_from(offset).ok()
}

fn index(class: Class) -> usize {
    match class {
        Class::PlayerController => 0,
        Class::NetConnection => 1,
        Class::ActorChannel => 2,
    }
}

/// The class, found once and kept; while missing, searched again at most once a second.
fn class(class: Class) -> Option<usize> {
    let slot = index(class);
    let found = FOUND[slot].load(Ordering::Relaxed);
    if found != 0 {
        return Some(found);
    }
    let now = win::tick();
    let tried = TRIED_MS[slot].load(Ordering::Relaxed);
    if tried != 0 && now.saturating_sub(tried) < SECOND_MS {
        return None;
    }
    TRIED_MS[slot].store(now.max(1), Ordering::Relaxed);
    let found = reflect::find_class(class.name())?;
    FOUND[slot].store(found, Ordering::Relaxed);
    Some(found)
}

/// This process.
pub struct ProcessGame;

impl Game for ProcessGame {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        readable(ptr, size)
    }
    fn readable_this_frame(&self, ptr: usize, size: usize) -> bool {
        readable_this_frame(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        read_bytes(addr, out)
    }
    fn write_ptr(&self, addr: usize, value: usize) -> bool {
        reflect::write::<usize>(addr, value)
    }
    fn is_live_object(&self, ptr: usize) -> bool {
        engine::is_live_object(ptr)
    }
    fn class(&self, which: Class) -> Option<usize> {
        class(which)
    }
    fn world(&self) -> usize {
        listen::gworld().unwrap_or(0)
    }
    fn now_ms(&self) -> u64 {
        win::tick()
    }
    fn name(&self, object: usize) -> Option<String> {
        reflect::engine()?.object_name(object)
    }
    fn class_name(&self, object: usize) -> Option<String> {
        reflect::class_name_of(object)
    }
    fn image_rva(&self, address: usize) -> Option<u32> {
        image_rva(address)
    }
    fn function_entry(&self, rva: u32) -> Option<u32> {
        function_entry(rva)
    }
    fn log(&self, line: &str) {
        log::line(line);
    }
}
