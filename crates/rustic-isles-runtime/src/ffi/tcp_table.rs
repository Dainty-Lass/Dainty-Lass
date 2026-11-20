//! The in-game relay's owner check reads the IPv4 TCP table through the IP helper, in the netstat lines
//! `dainty_relay::owner::table` parses, so the check is the launcher's own (RD21). Under Wine there is none, as
//! the launcher has none off Windows: Wine's table can't name the game's pid.

use std::ffi::c_void;
use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddrV4};

use futures_util::future::BoxFuture;
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
};
use windows_sys::Win32::Networking::WinSock::AF_INET;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

use super::win::to_wide;

const TRIES: usize = 4;
/// The table grows between the size query and the read; leave room.
const SLACK_ROWS: usize = 64;

/// Whether this process runs under Wine (Proton): its `ntdll` exports `wine_get_version`.
pub fn is_wine() -> bool {
    let name = to_wide("ntdll.dll");
    // SAFETY: a NUL-terminated name; ntdll is always loaded
    let ntdll = unsafe { GetModuleHandleW(name.as_ptr()) };
    if ntdll.is_null() {
        return false;
    }
    // SAFETY: a loaded module and a NUL-terminated ASCII name
    unsafe { GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()) }.is_some()
}

/// The IPv4 TCP table with owning pids.
#[derive(Debug, Clone, Copy, Default)]
pub struct IpHelperTable;

impl dainty_relay::owner::TableSource for IpHelperTable {
    fn read(&self) -> BoxFuture<'static, Option<String>> {
        Box::pin(async { read_table().map(|rows| lines(&rows)) })
    }
}

/// `(local, remote, pid)` per row.
fn read_table() -> Option<Vec<(SocketAddrV4, SocketAddrV4, u32)>> {
    let mut size: u32 = 0;
    for _ in 0..TRIES {
        // u64s keep the buffer aligned for the table's u32 fields
        let words = (size as usize).div_ceil(8) + SLACK_ROWS * size_of::<MIB_TCPROW_OWNER_PID>() / 8;
        let mut buffer = vec![0u64; words.max(1)];
        let mut capacity = u32::try_from(buffer.len() * 8).ok()?;
        // SAFETY: `buffer` holds `capacity` writable bytes, aligned for MIB_TCPTABLE_OWNER_PID
        let status = unsafe {
            GetExtendedTcpTable(
                buffer.as_mut_ptr().cast::<c_void>(),
                &raw mut capacity,
                0,
                u32::from(AF_INET),
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        match status {
            NO_ERROR => return Some(rows(&buffer)),
            ERROR_INSUFFICIENT_BUFFER => size = capacity,
            _ => return None,
        }
    }
    None
}

fn rows(buffer: &[u64]) -> Vec<(SocketAddrV4, SocketAddrV4, u32)> {
    let bytes = buffer.len() * 8;
    let table = buffer.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
    // SAFETY: the call succeeded, so the buffer starts with a table header
    let count = unsafe { (*table).dwNumEntries } as usize;
    let first = size_of::<u32>();
    let fits = bytes.saturating_sub(first) / size_of::<MIB_TCPROW_OWNER_PID>();
    // SAFETY: rows follow the count, aligned as the header is; never read past the buffer
    let rows = unsafe {
        std::slice::from_raw_parts(
            (&raw const (*table).table).cast::<MIB_TCPROW_OWNER_PID>(),
            count.min(fits),
        )
    };
    rows.iter()
        .map(|row| {
            (
                end(row.dwLocalAddr, row.dwLocalPort),
                end(row.dwRemoteAddr, row.dwRemotePort),
                row.dwOwningPid,
            )
        })
        .collect()
}

/// Addresses are in network order as stored; ports in network order in the low 16 bits.
fn end(addr: u32, port: u32) -> SocketAddrV4 {
    let port = u16::try_from(port & 0xFFFF).unwrap_or(0);
    SocketAddrV4::new(Ipv4Addr::from(addr.to_ne_bytes()), u16::from_be(port))
}

/// netstat's columns: protocol, the two ends, a state the parser never reads, the pid.
fn lines(rows: &[(SocketAddrV4, SocketAddrV4, u32)]) -> String {
    let mut out = String::with_capacity(rows.len() * 64);
    for (local, remote, pid) in rows {
        let _ = writeln!(out, "  TCP    {local}    {remote}    -    {pid}");
    }
    out
}
