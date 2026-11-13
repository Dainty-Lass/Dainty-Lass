//! The hook broker (R0 7.2): at most one MinHook hook per target, running an ordered chain of wrappers. The hook
//! jumps to a 6-byte stub `FF 25 <disp32>` that jumps through a pointer cell; only the cell is ever rewritten, by
//! one aligned atomic store, so the chain changes while game threads run and needs no signature.

use super::minhook;
use rustic_isles_runtime_core::chain::{ChainError, ChainModel};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualAlloc, VirtualProtect,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const PAGE: usize = 4096;
const STUB_SIZE: usize = 8;
const STUBS_PER_POOL: usize = PAGE / STUB_SIZE;

/// A MIT wrapper's `next`: the wrapper inside it, or the trampoline. Loaded on every call (R0 7.2).
pub struct NextCell(AtomicUsize);

impl NextCell {
    /// Empty until the broker wires it.
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    /// The function to call next (0 before the wrapper is hooked).
    pub fn load(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

impl Default for NextCell {
    fn default() -> Self {
        Self::new()
    }
}

/// One stub page (executable, read-only) and its cell page right after it, so every `disp32` is in range.
struct Pool {
    stubs: usize,
    cells: usize,
    used: usize,
}

impl Pool {
    fn allocate() -> Option<Pool> {
        // SAFETY: a fresh private allocation; the stub page is written before it becomes executable
        unsafe {
            let base = VirtualAlloc(std::ptr::null(), 2 * PAGE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) as usize;
            if base == 0 {
                return None;
            }
            let cells = base + PAGE;
            for index in 0..STUBS_PER_POOL {
                let stub = base + index * STUB_SIZE;
                let cell = cells + index * 8;
                let disp = i32::try_from(cell as isize - (stub + 6) as isize).ok()?;
                let mut code = [0xCCu8; STUB_SIZE];
                code[0] = 0xFF;
                code[1] = 0x25;
                code[2..6].copy_from_slice(&disp.to_le_bytes());
                std::ptr::copy_nonoverlapping(code.as_ptr(), stub as *mut u8, STUB_SIZE);
            }
            let mut old: PAGE_PROTECTION_FLAGS = 0;
            if VirtualProtect(base as *const _, PAGE, PAGE_EXECUTE_READ, &mut old) == 0 {
                return None;
            }
            FlushInstructionCache(GetCurrentProcess(), base as *const _, PAGE);
            Some(Pool {
                stubs: base,
                cells,
                used: 0,
            })
        }
    }

    fn peek(&self) -> Option<(usize, usize)> {
        (self.used < STUBS_PER_POOL).then(|| (self.stubs + self.used * STUB_SIZE, self.cells + self.used * 8))
    }
}

/// The cell at `address` (aligned, in a cell page that is never freed).
fn cell(address: usize) -> &'static AtomicUsize {
    // SAFETY: cell pages are committed read-write, 8-aligned and never freed (the DLL is pinned)
    unsafe { &*(address as *const AtomicUsize) }
}

struct Hook {
    head: usize,
    chain: ChainModel,
}

struct Broker {
    hooks: BTreeMap<usize, Hook>,
    nexts: BTreeMap<usize, &'static NextCell>,
    pool: Option<Pool>,
}

static BROKER: Mutex<Broker> = Mutex::new(Broker {
    hooks: BTreeMap::new(),
    nexts: BTreeMap::new(),
    pool: None,
});

fn broker() -> std::sync::MutexGuard<'static, Broker> {
    BROKER.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Broker {
    /// The hook on `target`, made on first use: stub, `MH_CreateHook`, head = trampoline, `MH_EnableHook`.
    fn hooked(&mut self, target: usize) -> Result<&mut Hook, i32> {
        if !self.hooks.contains_key(&target) {
            let status = minhook::initialize();
            if status != minhook::MH_OK {
                return Err(status);
            }
            if self.pool.as_ref().and_then(Pool::peek).is_none() {
                self.pool = Some(Pool::allocate().ok_or(9)?);
            }
            let (stub, head) = self.pool.as_ref().and_then(Pool::peek).ok_or(9)?;
            let trampoline = minhook::create(target, stub)?;
            if let Some(pool) = self.pool.as_mut() {
                pool.used += 1;
            }
            cell(head).store(trampoline, Ordering::Release);
            let status = minhook::enable(target);
            if status != minhook::MH_OK {
                minhook::remove(target);
                return Err(status);
            }
            self.hooks.insert(
                target,
                Hook {
                    head,
                    chain: ChainModel::new(trampoline),
                },
            );
        }
        self.hooks.get_mut(&target).ok_or(minhook::MH_UNKNOWN)
    }

    /// Rewrites every MIT next cell innermost first, then the head (R0 7.2).
    fn publish(&self, target: usize) {
        let Some(hook) = self.hooks.get(&target) else {
            return;
        };
        for (entry, next) in hook.chain.mit_cells() {
            if let Some(cell) = self.nexts.get(&entry) {
                cell.0.store(next, Ordering::Release);
            }
        }
        cell(hook.head).store(hook.chain.head(), Ordering::Release);
    }
}

/// Adds a MIT wrapper on `target` (band `Mit`, smaller `order` outer). `next` is the wrapper's own cell, which it
/// loads on every call. A slice never calls MinHook itself.
pub fn hook(target: usize, wrapper: usize, order: i32, next: &'static NextCell) -> Result<(), i32> {
    let mut broker = broker();
    let hook = broker.hooked(target)?;
    hook.chain.add_mit(wrapper, order).map_err(ChainError::status)?;
    broker.nexts.insert(wrapper, next);
    broker.publish(target);
    Ok(())
}

/// Removes a MIT wrapper; its cell keeps its last value so a call in flight finishes.
pub fn unhook(target: usize, wrapper: usize) -> Result<(), i32> {
    let mut broker = broker();
    let hook = broker.hooks.get_mut(&target).ok_or(4)?;
    hook.chain.remove_mit(wrapper).map_err(ChainError::status)?;
    broker.publish(target);
    broker.nexts.remove(&wrapper);
    Ok(())
}

/// `rir_chain_add_inner`: the AGPL wrapper, innermost; its next is the trampoline.
pub fn add_inner(target: usize, wrapper: usize) -> Result<usize, i32> {
    let mut broker = broker();
    let hook = broker.hooked(target)?;
    hook.chain.add_agpl(wrapper).map_err(ChainError::status)?;
    let trampoline = hook.chain.trampoline();
    broker.publish(target);
    Ok(trampoline)
}

/// `rir_chain_remove_inner`.
pub fn remove_inner(target: usize, wrapper: usize) -> Result<(), i32> {
    let mut broker = broker();
    let hook = broker.hooks.get_mut(&target).ok_or(ChainError::NotCreated.status())?;
    hook.chain.remove_agpl(wrapper).map_err(ChainError::status)?;
    broker.publish(target);
    Ok(())
}

/// `original(T)`: the trampoline when the broker hooked `T`, else `T`.
pub fn original(target: usize) -> usize {
    rustic_isles_runtime_core::chain::original(broker().hooks.get(&target).map(|hook| &hook.chain), target)
}
