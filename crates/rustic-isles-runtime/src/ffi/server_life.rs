//! R8's raw calls (spec-runtime-r8 2): the reserve's region and release, the process memory figures, the working-set
//! trim, the UDP port probe and the process end.

use super::reflect::writable;
use rustic_isles_runtime_core::server_life::{BlockRegion, MemoryInfo};
use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Memory::{MEM_RELEASE, MEMORY_BASIC_INFORMATION, VirtualFree, VirtualQuery};
use windows_sys::Win32::System::Threading::{ExitProcess, GetCurrentProcess, TerminateProcess};

#[repr(C)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut ProcessMemoryCounters, cb: u32) -> i32;
    fn SetProcessWorkingSetSizeEx(process: *mut c_void, minimum: usize, maximum: usize, flags: u32) -> i32;
}

#[repr(C)]
struct SockaddrIn {
    family: u16,
    port: u16,
    address: [u8; 4],
    zero: [u8; 8],
}

#[repr(C, align(8))]
struct WsaData([u8; 512]);

const AF_INET: i32 = 2;
const SOCK_DGRAM: i32 = 2;
const IPPROTO_UDP: i32 = 17;
const INVALID_SOCKET: usize = usize::MAX;

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut WsaData) -> i32;
    fn socket(family: i32, kind: i32, protocol: i32) -> usize;
    fn bind(socket: usize, address: *const SockaddrIn, length: i32) -> i32;
    fn closesocket(socket: usize) -> i32;
}

/// `VirtualQuery`'s full answer for the reserve check (7.6 check 4).
pub fn block_region(block: usize) -> Option<BlockRegion> {
    if block == 0 {
        return None;
    }
    // SAFETY: plain data VirtualQuery fills; any address may be queried
    let info: MEMORY_BASIC_INFORMATION = unsafe {
        let mut info = std::mem::zeroed();
        let size = std::mem::size_of::<MEMORY_BASIC_INFORMATION>();
        if VirtualQuery(block as *const _, &mut info, size) != size {
            return None;
        }
        info
    };
    Some(BlockRegion {
        base: info.BaseAddress as usize,
        allocation_base: info.AllocationBase as usize,
        size: info.RegionSize,
        state: info.State,
        kind: info.Type,
        protect: info.Protect,
    })
}

/// Compare-exchange the pointer global at `global` from `from` to `to` (7.6 steps 5 and 6).
pub fn swap_global(global: usize, from: usize, to: usize) -> bool {
    if !writable(global, std::mem::size_of::<usize>()) {
        return false;
    }
    // SAFETY: the 8 aligned bytes were just checked committed and writable; the engine reads them only on an
    // out-of-memory death
    let atomic = unsafe { AtomicUsize::from_ptr(global as *mut usize) };
    atomic
        .compare_exchange(from, to, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// `VirtualFree(block, 0, MEM_RELEASE)`; `Err` is `GetLastError`.
pub fn release(block: usize) -> Result<(), u32> {
    // SAFETY: the block is the engine's own allocation, verified and unhooked from its only reader first
    unsafe {
        if VirtualFree(block as *mut c_void, 0, MEM_RELEASE) == 0 {
            return Err(GetLastError());
        }
    }
    Ok(())
}

/// `K32GetProcessMemoryInfo` on this process.
pub fn memory_info() -> Option<MemoryInfo> {
    let cb = u32::try_from(std::mem::size_of::<ProcessMemoryCounters>()).ok()?;
    // SAFETY: plain data the call fills, with its size in `cb`
    let counters = unsafe {
        let mut counters: ProcessMemoryCounters = std::mem::zeroed();
        counters.cb = cb;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, cb) == 0 {
            return None;
        }
        counters
    };
    Some(MemoryInfo {
        working_set: counters.working_set_size,
        peak_working_set: counters.peak_working_set_size,
        private_commit: counters.private_usage,
        page_faults: counters.page_fault_count,
    })
}

/// Empties the working set (7.7); `Err` is `GetLastError`.
pub fn trim_working_set() -> Result<(), u32> {
    // SAFETY: the pseudo-handle of this process; (MAX, MAX) is the documented "empty the working set" form
    unsafe {
        if SetProcessWorkingSetSizeEx(GetCurrentProcess(), usize::MAX, usize::MAX, 0) == 0 {
            return Err(GetLastError());
        }
    }
    Ok(())
}

/// `WSAStartup(2.2)` once (call 7): the engine has Winsock up, this only adds a reference.
fn winsock() -> Result<(), i32> {
    static STARTED: OnceLock<i32> = OnceLock::new();
    let result = *STARTED.get_or_init(|| {
        let mut data = WsaData([0; 512]);
        // SAFETY: `data` is larger than WSADATA and lives across the call
        unsafe { WSAStartup(0x0202, &mut data) }
    });
    if result == 0 { Ok(()) } else { Err(result) }
}

/// Whether a UDP bind on `address:port` succeeds (7.5): free. `Err` is a `WSAStartup` failure.
pub fn port_free(address: [u8; 4], port: u16) -> Result<bool, i32> {
    winsock()?;
    // SAFETY: Winsock is started; the socket is closed before return
    unsafe {
        let handle = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
        if handle == INVALID_SOCKET {
            return Ok(false);
        }
        let addr = SockaddrIn {
            family: AF_INET as u16,
            port: port.to_be(),
            address,
            zero: [0; 8],
        };
        let bound = bind(handle, &addr, std::mem::size_of::<SockaddrIn>() as i32) == 0;
        closesocket(handle);
        Ok(bound)
    }
}

/// `TerminateProcess(GetCurrentProcess(), 0)`; should that return, `ExitProcess(0)` (8.1).
pub fn end_process() -> ! {
    // SAFETY: the pseudo-handle of this process
    unsafe {
        TerminateProcess(GetCurrentProcess(), 0);
        ExitProcess(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_probe() {
        let held = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
        let port = held.local_addr().expect("address").port();
        assert_eq!(port_free([127, 0, 0, 1], port), Ok(false), "held by the test");
        drop(held);
        assert_eq!(port_free([127, 0, 0, 1], port), Ok(true), "free once closed");
    }

    #[test]
    fn working_set_trim() {
        let before = memory_info().expect("memory info");
        assert!(before.working_set > 0);
        assert_eq!(trim_working_set(), Ok(()));
    }

    #[test]
    fn block_regions() {
        let block = vec![0u8; 4096];
        let region = block_region(block.as_ptr() as usize).expect("region");
        assert_eq!(region.state, 0x1000);
        assert!(block_region(0).is_none());
    }
}
