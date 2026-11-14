//! Runtime slice R16, travel (spec-runtime-r16): TR-1 on `UEngine::LoadMap` and the world-change watcher on SP-1.
//! Both run the leftover-world pass (`core::travel`) on the game thread; the atomic clear of `Async` is the only
//! write to game memory.

use super::broker::{self, NextCell};
use super::memory::{query, read, verify_entry};
use super::{detaching, engine, flags, log, minhook, win};
use crate::handlers::{self, PeCall, PeHandler, Point, Verdict};
use rustic_isles_runtime_core::engine::{Engine, NameCache};
use rustic_isles_runtime_core::memory::{Memory, Region, region_allows};
use rustic_isles_runtime_core::names::{Kind, Matcher};
use rustic_isles_runtime_core::offsets::{FoundBy, data, layout, target};
use rustic_isles_runtime_core::travel::{self, Reason, Travel, flag_names};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, TryLockError};

type LoadMapFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void, *mut c_void, *mut c_void) -> u8;

/// Every named call (RT-18 for R16): the watcher only reads `GWorld` and the clock.
const EVERY_CALL: Matcher = &[(Kind::Prefix, "")];
/// Regions one pass remembers (call 3).
pub const CACHED_REGIONS: usize = 8;
const WRITABLE: u32 = 0x04 | 0x08 | 0x40 | 0x80;

static BASE: AtomicUsize = AtomicUsize::new(0);
static GAME_THREAD: AtomicU32 = AtomicU32::new(0);
static STATE: Mutex<Travel> = Mutex::new(Travel::new());
static NAMES: LazyLock<NameCache> = LazyLock::new(NameCache::default);
static NEXT: NextCell = NextCell::new();
static PANICKED: AtomicBool = AtomicBool::new(false);

/// A `Memory` that answers from the last 8 regions `VirtualQuery` returned. One pass only: no collection runs
/// while it lives, since the pass calls no game code.
pub struct RegionCache {
    query: fn(usize) -> Option<Region>,
    regions: RefCell<[Option<Region>; CACHED_REGIONS]>,
    next: Cell<usize>,
}

impl RegionCache {
    /// An empty cache over `query`.
    pub fn new(query: fn(usize) -> Option<Region>) -> Self {
        Self {
            query,
            regions: RefCell::new([None; CACHED_REGIONS]),
            next: Cell::new(0),
        }
    }

    fn region(&self, ptr: usize) -> Option<Region> {
        if ptr == 0 {
            return None;
        }
        let cached = self
            .regions
            .borrow()
            .iter()
            .flatten()
            .copied()
            .find(|region| ptr >= region.base && region.base.checked_add(region.size).is_some_and(|end| ptr < end));
        if cached.is_some() {
            return cached;
        }
        let region = (self.query)(ptr)?;
        let slot = self.next.get();
        self.regions.borrow_mut()[slot] = Some(region);
        self.next.set((slot + 1) % CACHED_REGIONS);
        Some(region)
    }
}

impl Memory for RegionCache {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        region_allows(ptr, size, self.region(ptr), true)
    }

    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        if out.is_empty() {
            return true;
        }
        if !region_allows(addr, out.len(), self.region(addr), false) {
            return false;
        }
        // SAFETY: the source range lies in one committed, accessible region; `out` is a distinct buffer
        unsafe { std::ptr::copy_nonoverlapping(addr as *const u8, out.as_mut_ptr(), out.len()) };
        true
    }
}

/// Clears only `Async` in an item's flags word, with one atomic AND (the engine's own flag writes are atomic).
pub fn clear_async(flags: usize) -> bool {
    if flags == 0 || !flags.is_multiple_of(4) {
        return false;
    }
    let Some(region) = query(flags) else {
        return false;
    };
    if !region_allows(flags, 4, Some(region), false) || region.protect & WRITABLE == 0 {
        return false;
    }
    // SAFETY: aligned, committed and writable; the engine writes this word atomically too
    let word = unsafe { &*(flags as *const AtomicU32) };
    word.fetch_and(!layout::ITEM_ASYNC, Ordering::SeqCst);
    true
}

fn state() -> Option<MutexGuard<'static, Travel>> {
    match STATE.try_lock() {
        Ok(state) => Some(state),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

/// The game engine's arrays through a fresh region cache.
struct Pass {
    memory: RegionCache,
    objects: usize,
    names: usize,
}

impl Pass {
    fn new(base: usize) -> Option<Self> {
        Some(Self {
            memory: RegionCache::new(query),
            objects: base.checked_add(usize::try_from(data::GOBJECTS).ok()?)?,
            names: base.checked_add(usize::try_from(data::GNAMES).ok()?)?,
        })
    }

    fn engine(&self) -> Engine<'_> {
        Engine::new(&self.memory, self.objects, self.names, Some(&NAMES))
    }

    /// Runs the pass, clears, logs; the leftovers it found.
    fn run(&self, state: &mut Travel, gworld: usize, reason: Reason) -> Vec<usize> {
        if !engine::names_enabled() {
            return Vec::new();
        }
        let engine = self.engine();
        let outcome = state.pass(&self.memory, self.objects, &engine, gworld, reason, &mut || {
            flags::flag(flag_names::KEEP_STUCK_ASYNC_FLAGS)
        });
        for &address in &outcome.clear {
            clear_async(address);
        }
        for line in &outcome.lines {
            log::line(line);
        }
        outcome.leftovers
    }
}

fn gworld(base: usize) -> usize {
    base.checked_add(data::GWORLD as usize)
        .and_then(read::<usize>)
        .unwrap_or(0)
}

/// TR-1's step 4 (4.2): the `loadmap-enter` pass, then the guard. True refuses the travel.
fn enter(context: usize, url: usize) -> bool {
    let base = BASE.load(Ordering::SeqCst);
    let Some(mut state) = state() else {
        return false;
    };
    let Some(pass) = (base != 0).then(|| Pass::new(base)).flatten() else {
        return false;
    };
    let leftovers = pass.run(&mut state, gworld(base), Reason::LoadMapEnter);
    let engine = pass.engine();
    let guard = state.guard(
        &mut || flags::flag(flag_names::ENABLE_ZOMBIE_LOADMAP_GUARD),
        &mut || travel::read_map_path(&pass.memory, url),
        &leftovers,
        travel::context_world(&pass.memory, context),
        &engine,
    );
    for line in &guard.lines {
        log::line(line);
    }
    guard.refused
}

/// The TR-1 wrapper body, `decide` standing for step 4 so a test can fake a refusal.
#[allow(
    clippy::too_many_arguments,
    reason = "LoadMap's five arguments and the wrapper's own three"
)]
pub(crate) fn loadmap_call(
    next: &NextCell,
    panicked: &AtomicBool,
    decide: fn(usize, usize) -> bool,
    engine: *mut c_void,
    context: *mut c_void,
    url: *const c_void,
    pending: *mut c_void,
    error: *mut c_void,
) -> u8 {
    let next = next.load();
    if next == 0 {
        return 0;
    }
    // SAFETY: the broker wrote a function of LoadMap's signature into this cell
    let next: LoadMapFn = unsafe { std::mem::transmute::<usize, LoadMapFn>(next) };
    let call_next = || {
        // SAFETY: the original arguments
        unsafe { next(engine, context, url, pending, error) }
    };
    if detaching() {
        return call_next();
    }
    crate::lifecycle::write_status_once();
    // LoadMap runs only on the game thread: it is the authority (call 5)
    GAME_THREAD.store(win::thread_id(), Ordering::SeqCst);
    let refused = catch_unwind(AssertUnwindSafe(|| decide(context as usize, url as usize))).unwrap_or_else(|_| {
        if !panicked.swap(true, Ordering::SeqCst) {
            log::line("[Panic] TR-1 handler panicked; the travel goes on");
        }
        false
    });
    // A refusal absorbs: LoadMap's false is a failed load, which browses to the default map
    if refused { 0 } else { call_next() }
}

unsafe extern "C" fn loadmap_detour(
    engine: *mut c_void,
    context: *mut c_void,
    url: *const c_void,
    pending: *mut c_void,
    error: *mut c_void,
) -> u8 {
    loadmap_call(&NEXT, &PANICKED, enter, engine, context, url, pending, error)
}

/// The world-change watcher (5.1), before every named SP-1 call.
fn watch(_: &PeCall) -> Verdict {
    let me = win::thread_id();
    if let Err(game) = GAME_THREAD.compare_exchange(0, me, Ordering::SeqCst, Ordering::SeqCst)
        && game != me
    {
        return Verdict::Continue;
    }
    let base = BASE.load(Ordering::SeqCst);
    if base == 0 {
        return Verdict::Continue;
    }
    let Some(mut state) = state() else {
        return Verdict::Continue;
    };
    let mut world = 0;
    let Some(reason) = state.watcher.poll(win::tick(), &mut || {
        world = gworld(base);
        world
    }) else {
        return Verdict::Continue;
    };
    if let Some(pass) = Pass::new(base) {
        pass.run(&mut state, world, reason);
    }
    Verdict::Continue
}

/// Init, client only, before `handlers::freeze()`: the watcher on SP-1.
pub fn register(base: usize) -> Result<(), &'static str> {
    BASE.store(base, Ordering::SeqCst);
    handlers::register_pe(
        Point::Sp1,
        PeHandler {
            slice: "R16",
            order: 0,
            matcher: EVERY_CALL,
            before: Some(watch),
            after: None,
        },
    )
}

/// Installs TR-1 (4.4): off by `NO_LOADMAP_PROBE.flag`, never on a prologue mismatch.
pub fn install(base: usize) -> Result<(), &'static str> {
    if flags::flag(flag_names::NO_LOADMAP_PROBE) {
        return Err("off (NO_LOADMAP_PROBE.flag)");
    }
    let unknown = minhook::status_name(minhook::MH_UNKNOWN);
    let target = target("TR-1").ok_or(unknown)?;
    if let FoundBy::RvaAndBytes(prologue) = target.found_by
        && !verify_entry(target.rva, prologue)
    {
        return Err("prologue mismatch");
    }
    let address = usize::try_from(target.rva)
        .ok()
        .and_then(|rva| base.checked_add(rva))
        .ok_or(unknown)?;
    broker::hook(address, loadmap_detour as *const () as usize, 0, &NEXT).map_err(minhook::status_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;
    use std::sync::atomic::AtomicU64;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualAlloc,
        VirtualFree, VirtualProtect,
    };

    const PAGE: usize = 4096;

    struct Pages(usize);

    impl Pages {
        fn new(count: usize) -> Self {
            let base =
                unsafe { VirtualAlloc(std::ptr::null(), count * PAGE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
            assert!(!base.is_null());
            Self(base as usize)
        }
        fn protect(&self, page: usize, protect: PAGE_PROTECTION_FLAGS) {
            let mut old = 0;
            assert_ne!(
                unsafe { VirtualProtect((self.0 + page * PAGE) as *const _, PAGE, protect, &mut old) },
                0
            );
        }
    }

    impl Drop for Pages {
        fn drop(&mut self) {
            unsafe { VirtualFree(self.0 as *mut _, 0, MEM_RELEASE) };
        }
    }

    #[test]
    fn clear_changes_only_async() {
        let pages = Pages::new(1);
        let word = pages.0 + 8;
        unsafe { std::ptr::write(word as *mut u32, 0xFFFF_FFFF) };
        unsafe { std::ptr::write((word + 4) as *mut u32, 0x1234_5678) };
        assert!(clear_async(word));
        assert_eq!(unsafe { std::ptr::read(word as *const u32) }, 0xFBFF_FFFF);
        assert_eq!(unsafe { std::ptr::read((word + 4) as *const u32) }, 0x1234_5678);
        unsafe { std::ptr::write(word as *mut u32, 0x2000_0001) };
        assert!(clear_async(word));
        assert_eq!(
            unsafe { std::ptr::read(word as *const u32) },
            0x2000_0001,
            "no Async: unchanged"
        );
        assert!(!clear_async(word + 1), "unaligned");
        assert!(!clear_async(0));
        let locked = Pages::new(1);
        locked.protect(0, PAGE_NOACCESS);
        assert!(!clear_async(locked.0));
    }

    thread_local! {
        static QUERIES: Cell<u32> = const { Cell::new(0) };
    }

    fn counted(ptr: usize) -> Option<Region> {
        QUERIES.with(|queries| queries.set(queries.get() + 1));
        query(ptr)
    }

    fn queries() -> u32 {
        QUERIES.with(Cell::get)
    }

    #[test]
    fn region_cache() {
        let pages = Pages::new(3);
        pages.protect(2, PAGE_NOACCESS);
        let cache = RegionCache::new(counted);
        assert!(cache.readable(pages.0, 8));
        let after_first = queries();
        assert!(cache.readable(pages.0 + 0x100, 0x100));
        assert_eq!(cache.ptr_at(pages.0 + 0x10), Some(0));
        assert_eq!(queries(), after_first, "inside a cached region: no new query");
        assert!(
            !cache.readable(pages.0 + 2 * PAGE - 8, 16),
            "across into the no-access page"
        );
        assert!(
            !cache.readable(pages.0 + 2 * PAGE, 8),
            "no-access, next to a cached region"
        );
        let mut out = [0u8; 8];
        assert!(!cache.read(pages.0 + 2 * PAGE, &mut out));
        assert!(!cache.readable(0, 8));
    }

    static SEEN: AtomicU64 = AtomicU64::new(0);
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static REFUSE: AtomicBool = AtomicBool::new(false);
    static TEST_NEXT: NextCell = NextCell::new();
    static TEST_PANICKED: AtomicBool = AtomicBool::new(false);

    #[inline(never)]
    extern "C" fn fake_loadmap(
        engine: *mut c_void,
        context: *mut c_void,
        url: *const c_void,
        pending: *mut c_void,
        error: *mut c_void,
    ) -> u8 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        let sum = [engine as u64, context as u64, url as u64, pending as u64, error as u64]
            .iter()
            .enumerate()
            .fold(0u64, |sum, (index, value)| {
                sum.wrapping_add(black_box(*value) << (index * 8))
            });
        SEEN.store(sum, Ordering::SeqCst);
        black_box(1)
    }

    fn fake_decide(_: usize, _: usize) -> bool {
        REFUSE.load(Ordering::SeqCst)
    }

    unsafe extern "C" fn test_detour(
        engine: *mut c_void,
        context: *mut c_void,
        url: *const c_void,
        pending: *mut c_void,
        error: *mut c_void,
    ) -> u8 {
        loadmap_call(
            &TEST_NEXT,
            &TEST_PANICKED,
            fake_decide,
            engine,
            context,
            url,
            pending,
            error,
        )
    }

    #[test]
    fn wrapper_passes_through_or_refuses() {
        let target = fake_loadmap as *const () as usize;
        broker::hook(target, test_detour as *const () as usize, 0, &TEST_NEXT).unwrap();
        type Fn5 = extern "C" fn(*mut c_void, *mut c_void, *const c_void, *mut c_void, *mut c_void) -> u8;
        let call = black_box(fake_loadmap as Fn5);
        let arg = |value: usize| value as *mut c_void;
        let expected = [1u64, 2, 3, 4, 5]
            .iter()
            .enumerate()
            .fold(0u64, |sum, (index, value)| sum.wrapping_add(value << (index * 8)));
        assert_eq!(call(arg(1), arg(2), arg(3) as *const c_void, arg(4), arg(5)), 1);
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(
            SEEN.load(Ordering::SeqCst),
            expected,
            "all five arguments reach LoadMap"
        );
        REFUSE.store(true, Ordering::SeqCst);
        assert_eq!(call(arg(1), arg(2), arg(3) as *const c_void, arg(4), arg(5)), 0);
        assert_eq!(CALLS.load(Ordering::SeqCst), 1, "a refusal never calls LoadMap");
        REFUSE.store(false, Ordering::SeqCst);
        assert_eq!(call(arg(9), arg(8), arg(7) as *const c_void, arg(6), arg(5)), 1);
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    }
}
