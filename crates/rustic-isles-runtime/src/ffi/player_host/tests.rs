//! R10 16.1's DLL tests: the control pipe against a client in this process, the ready pipe against a server the
//! test makes, the DACL, and HTTP-1@R10's decision over a fake request.

use super::*;
use rustic_isles_runtime_core::player_host::{PIPE_PREFIX, fence_request};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};
use windows_sys::Win32::Security::{ACL, GetSecurityDescriptorDacl};
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_INBOUND;

const SESSION: &str = "0f3c9a2e-4b1d-4e8a-9c7f-1a2b3c4d5e6f";

fn pipe_name(kind: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "{PIPE_PREFIX}{kind}.test{}x{}abcdef",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

/// Opens `name` as a client once the server made it.
fn open_client(name: &str) -> File {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        match OpenOptions::new().read(true).write(true).open(name) {
            Ok(file) => return file,
            Err(error) if Instant::now() < until => {
                let _ = error;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

/// Serves one control connection on a thread; returns its outcome and whether the fence ran.
fn control(expected_pid: u32, message: &[u8]) -> (Outcome, bool, Vec<u8>) {
    let name = pipe_name("control");
    let fenced = Arc::new(AtomicBool::new(false));
    let server = {
        let (name, fenced) = (name.clone(), fenced.clone());
        std::thread::spawn(move || {
            serve_control(&name, expected_pid, SESSION, "7", &|| {
                fenced.store(true, Ordering::SeqCst)
            })
        })
    };
    let mut client = open_client(&name);
    let _ = client.write_all(message);
    let mut answer = vec![0u8; 512];
    let got = client.read(&mut answer).unwrap_or(0);
    answer.truncate(got);
    drop(client);
    let outcome = server.join().unwrap();
    (outcome, fenced.load(Ordering::SeqCst), answer)
}

#[test]
fn the_exact_fence_is_acknowledged() {
    let (outcome, fenced, answer) = control(std::process::id(), fence_request(SESSION, "7").as_bytes());
    assert_eq!(outcome, Outcome::Fenced);
    assert!(fenced);
    assert_eq!(
        answer,
        format!("RIS1 FENCE_ACK session={SESSION} generation=7\n").into_bytes()
    );
}

#[test]
fn a_malformed_message_fences_without_an_ack() {
    for message in [
        fence_request(SESSION, "8").into_bytes(),
        format!("{}\r\n", fence_request(SESSION, "7").trim_end()).into_bytes(),
        vec![b'R'; 600],
    ] {
        let (outcome, fenced, answer) = control(std::process::id(), &message);
        assert_eq!(outcome, Outcome::Refused);
        assert!(fenced);
        assert!(answer.is_empty());
    }
}

#[test]
fn another_client_pid_fences_without_an_ack() {
    let (outcome, fenced, answer) = control(std::process::id() ^ 0x55, fence_request(SESSION, "7").as_bytes());
    assert_eq!(outcome, Outcome::Refused);
    assert!(fenced);
    assert!(answer.is_empty());
}

#[test]
fn the_dacl_holds_two_entries() {
    let descriptor = descriptor().unwrap();
    let (mut present, mut defaulted) = (0, 0);
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: a valid descriptor; the out-pointers are locals
    let ok = unsafe { GetSecurityDescriptorDacl(descriptor.as_ptr(), &mut present, &mut dacl, &mut defaulted) };
    assert_ne!(ok, 0);
    assert_ne!(present, 0);
    assert!(!dacl.is_null());
    // SAFETY: the descriptor's own ACL
    assert_eq!(unsafe { (*dacl).AceCount }, 2);
}

/// A ready pipe's server end: one inbound instance, reading one message per connection.
struct ReadyServer(HANDLE);

// SAFETY: a pipe handle is usable from any thread
unsafe impl Send for ReadyServer {}

impl ReadyServer {
    fn new(name: &str) -> Self {
        let wide = win::to_wide(name);
        // SAFETY: a NUL-terminated name, default security
        let pipe = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                512,
                512,
                0,
                std::ptr::null(),
            )
        };
        assert_ne!(pipe, INVALID_HANDLE_VALUE);
        Self(pipe)
    }

    fn accept_and_read(&self) -> String {
        // SAFETY: the test's own pipe
        unsafe { ConnectNamedPipe(self.0, std::ptr::null_mut()) };
        let mut buffer = [0u8; 512];
        let mut read = 0u32;
        // SAFETY: as above
        unsafe { ReadFile(self.0, buffer.as_mut_ptr(), 512, &mut read, std::ptr::null_mut()) };
        // SAFETY: as above
        unsafe { DisconnectNamedPipe(self.0) };
        String::from_utf8_lossy(&buffer[..read as usize]).into_owned()
    }
}

impl Drop for ReadyServer {
    fn drop(&mut self) {
        // SAFETY: the test's own handle
        unsafe { CloseHandle(self.0) };
    }
}

#[test]
fn ready_message_arrives() {
    let name = pipe_name("ready");
    let server = ReadyServer::new(&name);
    let reader = std::thread::spawn(move || server.accept_and_read());
    assert!(report_ready(&name, "RIS1 READY x\n"));
    assert_eq!(reader.join().unwrap(), "RIS1 READY x\n");
}

#[test]
fn a_missing_ready_pipe_answers_false() {
    assert!(!report_ready(&pipe_name("missing"), "RIS1 READY x\n"));
}

#[test]
fn a_busy_ready_pipe_is_waited_for() {
    let name = pipe_name("busy");
    let server = ReadyServer::new(&name);
    // Another client holds the single instance until the server lets it go
    let other = OpenOptions::new().write(true).open(&name).unwrap();
    let reader = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        // SAFETY: the test's own pipe
        unsafe { DisconnectNamedPipe(server.0) };
        server.accept_and_read()
    });
    assert!(report_ready(&name, "RIS1 READY busy\n"));
    drop(other);
    assert_eq!(reader.join().unwrap(), "RIS1 READY busy\n");
}

// HTTP-1@R10 over a fake request: 0xD0 bytes, a URL FString at +0x70, the in-flight word at +0xC4, a vtable of
// 17 slots whose slot 16 records `SetHeader`.

#[repr(C)]
struct Text {
    data: *const u16,
    num: i32,
    max: i32,
}

thread_local! {
    static SETS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

fn text(string: *const Text) -> String {
    // SAFETY: the header's own FString, NUL counted in num
    unsafe {
        let string = &*string;
        let units = std::slice::from_raw_parts(string.data, (string.num - 1) as usize);
        assert_eq!(string.num, string.max);
        assert_eq!(*string.data.add(units.len()), 0);
        String::from_utf16(units).unwrap()
    }
}

unsafe extern "C" fn record_set_header(_this: *mut c_void, name: *const Text, value: *const Text) {
    SETS.with(|sets| sets.borrow_mut().push((text(name), text(value))));
}

struct Request {
    words: Box<[u64; 26]>,
    _vtable: Box<[usize; 17]>,
    _url: Vec<u16>,
}

impl Request {
    fn new(url: &str, in_flight: bool) -> Self {
        let mut vtable = Box::new([0usize; 17]);
        vtable[16] = record_set_header as *const () as usize;
        let url: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let mut words = Box::new([0u64; 26]);
        words[0] = vtable.as_ptr() as u64;
        words[0x70 / 8] = url.as_ptr() as u64;
        let num = url.len() as u64;
        words[0x78 / 8] = num | (num << 32);
        words[0xC0 / 8] = u64::from(in_flight) << 32;
        Self {
            words,
            _vtable: vtable,
            _url: url,
        }
    }

    fn sets(&self) -> Vec<(String, String)> {
        SETS.with(|sets| sets.borrow_mut().clear());
        let base: Vec<u16> = "https://backend.invalid".encode_utf16().collect();
        let capability: Vec<u16> = "C".repeat(43).encode_utf16().collect();
        add_header(self.words.as_ptr() as usize, &base, &capability);
        SETS.with(|sets| sets.borrow().clone())
    }
}

#[test]
fn header_only_under_the_api_base_and_idle() {
    let set = Request::new("https://backend.invalid/hunts/1", false).sets();
    assert_eq!(set, [("x-rustic-host-capability".to_owned(), "C".repeat(43))]);
    assert!(Request::new("https://other.invalid/hunts/1", false).sets().is_empty());
    assert!(Request::new("https://backend.invalid.evil/x", false).sets().is_empty());
    assert!(Request::new("https://backend.invalid", false).sets().is_empty());
    assert!(Request::new("https://backend.invalid/hunts/1", true).sets().is_empty());
}
