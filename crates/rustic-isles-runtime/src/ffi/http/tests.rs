//! HTTP-1's steps on a fake request in test memory (R2 10.1): a `0xD0`-byte request, a URL `FString`, a vtable
//! whose `GetHeader` and `SetHeader` record their calls, and a fake `GMalloc` and `FMemory::Realloc`.

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hint::black_box;

const BASE: &str = "https://backend.invalid";

thread_local! {
    static REALLOC_FAILS: Cell<bool> = const { Cell::new(false) };
    static REALLOCS: Cell<u32> = const { Cell::new(0) };
    static HEADERS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static SETS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    static GETS: Cell<u32> = const { Cell::new(0) };
    static HANDED_OUT: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static FREED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn w(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// A leaked engine-style buffer of `units` UTF-16 units.
fn engine_buffer(units: usize) -> *mut u16 {
    Box::leak(vec![0u16; units.max(1)].into_boxed_slice()).as_mut_ptr()
}

fn fstring_text(string: *const FString) -> String {
    // SAFETY: test FStrings are R2's own or the fake's, NUL counted in num
    unsafe {
        let string = &*string;
        let len = usize::try_from(string.num - 1).unwrap();
        String::from_utf16(std::slice::from_raw_parts(string.data, len)).unwrap()
    }
}

unsafe extern "C" fn fake_realloc(_ptr: *mut c_void, size: usize, alignment: u32) -> *mut c_void {
    assert_eq!(alignment, 0);
    REALLOCS.with(|count| count.set(count.get() + 1));
    if REALLOC_FAILS.with(Cell::get) {
        return std::ptr::null_mut();
    }
    engine_buffer(size / 2).cast()
}

unsafe extern "C" fn fake_get_header(_this: *mut c_void, out: *mut FString, name: *const FString) -> *mut FString {
    GETS.with(|count| count.set(count.get() + 1));
    let name = fstring_text(name);
    // SAFETY: `out` is the caller's FString
    unsafe { *out = FString::empty() };
    if let Some(value) = HEADERS.with(|headers| headers.borrow().get(&name).cloned()) {
        let units: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let data = engine_buffer(units.len());
        // SAFETY: a fresh buffer of `units.len()` units
        unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), data, units.len()) };
        HANDED_OUT.with(|handed| handed.borrow_mut().push(data as usize));
        let num = i32::try_from(units.len()).unwrap();
        // SAFETY: as above
        unsafe { *out = FString { data, num, max: num } };
    }
    out
}

unsafe extern "C" fn fake_set_header(_this: *mut c_void, name: *const FString, value: *const FString) {
    let (name, value) = (fstring_text(name), fstring_text(value));
    SETS.with(|sets| sets.borrow_mut().push((name.clone(), value.clone())));
    HEADERS.with(|headers| headers.borrow_mut().insert(name, value));
}

unsafe extern "C" fn fake_free(_this: *mut c_void, ptr: *mut c_void) {
    FREED.with(|freed| freed.borrow_mut().push(ptr as usize));
}

/// A fake `GMalloc` cell; leaked so the detour test can keep it.
fn fake_allocator() -> Allocator {
    let mut vtable = [0usize; 7];
    vtable[6] = fake_free as *const () as usize;
    let vtable = Box::leak(Box::new(vtable));
    let object = Box::leak(Box::new(vtable.as_ptr() as usize));
    let cell = Box::leak(Box::new(object as *const usize as usize));
    Allocator {
        realloc: fake_realloc as *const () as usize,
        gmalloc: cell as *const usize as usize,
    }
}

struct Fake {
    words: Box<[u64; 26]>,
    _vtable: Box<[usize; 17]>,
}

impl Fake {
    fn new(url: &str, spare: usize) -> Self {
        let mut vtable = Box::new([0usize; 17]);
        vtable[2] = fake_get_header as *const () as usize;
        vtable[16] = fake_set_header as *const () as usize;
        let mut words = Box::new([0u64; 26]);
        words[0] = vtable.as_ptr() as u64;
        let mut fake = Self { words, _vtable: vtable };
        fake.set_url(url, spare);
        fake
    }

    fn set_url(&mut self, url: &str, spare: usize) {
        let units: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let data = engine_buffer(units.len() + spare);
        // SAFETY: a fresh buffer large enough
        unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), data, units.len()) };
        let num = units.len() as u64;
        self.words[0x70 / 8] = data as u64;
        self.words[0x78 / 8] = num | ((num + spare as u64) << 32);
    }

    fn address(&self) -> usize {
        self.words.as_ptr() as usize
    }

    fn set_in_flight(&mut self, on: bool) {
        let low = self.words[0xC0 / 8] & 0xFFFF_FFFF;
        self.words[0xC0 / 8] = low | (u64::from(on) << 32);
    }

    fn data(&self) -> u64 {
        self.words[0x70 / 8]
    }

    fn num_max(&self) -> (i32, i32) {
        let word = self.words[0x78 / 8];
        (word as u32 as i32, (word >> 32) as u32 as i32)
    }

    fn url(&self) -> String {
        let (num, _) = self.num_max();
        // SAFETY: the fake's own buffer, `num` units with the NUL
        let units = unsafe { std::slice::from_raw_parts(self.data() as *const u16, num as usize) };
        assert_eq!(units.last(), Some(&0), "NUL-terminated");
        String::from_utf16(&units[..units.len() - 1]).unwrap()
    }
}

fn reset() {
    REALLOC_FAILS.with(|fails| fails.set(false));
    REALLOCS.with(|count| count.set(0));
    HEADERS.with(|headers| headers.borrow_mut().clear());
    SETS.with(|sets| sets.borrow_mut().clear());
    GETS.with(|count| count.set(0));
    HANDED_OUT.with(|handed| handed.borrow_mut().clear());
    FREED.with(|freed| freed.borrow_mut().clear());
}

fn sets() -> Vec<(String, String)> {
    SETS.with(|sets| sets.borrow().clone())
}

fn run(fake: &Fake, server: bool, mode: AuthMode, key: Option<&[u16]>) {
    let base = w(BASE);
    let context = Context {
        allocator: fake_allocator(),
        api_base: &base,
        server,
        mode,
        key,
    };
    process(&context, fake.address());
}

fn client(fake: &Fake) {
    run(fake, false, AuthMode::GlobalGameserver, None);
}

const REDIRECTED: &str = "https://backend.invalid/__origin/auth.steelyard.ca/tags?x=1";

#[test]
fn rewrite_in_place() {
    reset();
    let fake = Fake::new("https://auth.steelyard.ca/tags?x=1", 100);
    let (data, (_, max)) = (fake.data(), fake.num_max());
    client(&fake);
    assert_eq!(fake.url(), REDIRECTED);
    assert_eq!(fake.data(), data, "the buffer was large enough");
    assert_eq!(fake.num_max(), (REDIRECTED.len() as i32 + 1, max));
    assert_eq!(REALLOCS.with(Cell::get), 0);
    assert!(sets().is_empty(), "a client sets no header");
}

#[test]
fn rewrite_grows_through_realloc() {
    reset();
    let fake = Fake::new("https://auth.steelyard.ca/tags?x=1", 0);
    let data = fake.data();
    client(&fake);
    assert_eq!(REALLOCS.with(Cell::get), 1);
    assert_ne!(fake.data(), data);
    assert_eq!(fake.url(), REDIRECTED);
    let new_num = REDIRECTED.len() as i32 + 1;
    assert_eq!(fake.num_max(), (new_num, new_num));
}

#[test]
fn failed_realloc_leaves_the_url() {
    reset();
    REALLOC_FAILS.with(|fails| fails.set(true));
    let fake = Fake::new("https://auth.steelyard.ca/tags?x=1", 0);
    let (data, num_max) = (fake.data(), fake.num_max());
    client(&fake);
    assert_eq!(REALLOCS.with(Cell::get), 1);
    assert_eq!((fake.data(), fake.num_max()), (data, num_max));
    assert_eq!(fake.url(), "https://auth.steelyard.ca/tags?x=1");
}

#[test]
fn in_flight_and_other_hosts_are_left_alone() {
    reset();
    let mut fake = Fake::new("https://auth.steelyard.ca/tags", 100);
    fake.set_in_flight(true);
    client(&fake);
    assert_eq!(fake.url(), "https://auth.steelyard.ca/tags");
    fake.set_in_flight(false);
    client(&fake);
    assert_eq!(fake.url(), "https://backend.invalid/__origin/auth.steelyard.ca/tags");

    for url in [
        "https://example.com/x",
        "https://steelyard.ca.evil.com/x",
        "ftp://steelyard.ca/x",
    ] {
        let fake = Fake::new(url, 100);
        client(&fake);
        assert_eq!(fake.url(), url);
    }
    assert_eq!(REALLOCS.with(Cell::get), 0);
}

#[test]
fn bad_url_fields_are_skipped() {
    reset();
    let mut fake = Fake::new("https://auth.steelyard.ca/tags", 100);
    let good = fake.words[0x78 / 8];
    for (num, max) in [(1u64, 100u64), (0, 0), (40, 10), (40_000, 40_000)] {
        fake.words[0x78 / 8] = num | (max << 32);
        client(&fake);
        assert_eq!(fake.words[0x78 / 8], num | (max << 32), "num={num} max={max}");
    }
    fake.words[0x78 / 8] = good;
    let data = fake.data();
    fake.words[0x70 / 8] = 0;
    client(&fake);
    assert_eq!(fake.data(), 0);
    fake.words[0x70 / 8] = data;
    client(&fake);
    assert!(fake.url().starts_with(BASE));
}

const GRANT_URL: &str = "https://ol.epicgames.com/progression/acc/xp/10";

#[test]
fn grant_id_set_once_then_reused() {
    reset();
    let fake = Fake::new(GRANT_URL, 200);
    run(&fake, true, AuthMode::PlayerHost, None);
    assert_eq!(
        fake.url(),
        "https://backend.invalid/__origin/ol.epicgames.com/progression/acc/xp/10"
    );
    let first = sets();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].0, GRANT_HEADER);
    let id = first[0].1.clone();
    assert!(id.starts_with("gs-") && id.len() == 35, "{id}");
    assert!(FREED.with(|freed| freed.borrow().is_empty()), "nothing was handed out");

    // The engine's retry calls ProcessRequest again on the same request: the id is read back and kept
    run(&fake, true, AuthMode::PlayerHost, None);
    assert_eq!(sets().len(), 1, "no second id");
    assert_eq!(
        HEADERS.with(|headers| headers.borrow().get(GRANT_HEADER).cloned()),
        Some(id)
    );
    let handed = HANDED_OUT.with(|handed| handed.borrow().clone());
    assert_eq!(handed.len(), 1);
    assert_eq!(
        FREED.with(|freed| freed.borrow().clone()),
        handed,
        "GetHeader's copy goes back to GMalloc"
    );
    assert_eq!(GETS.with(Cell::get), 2);
}

#[test]
fn grant_only_for_server_grant_paths() {
    reset();
    let fake = Fake::new(GRANT_URL, 200);
    client(&fake);
    run(&fake, true, AuthMode::Invalid, None);
    assert!(sets().is_empty());
    assert_eq!(GETS.with(Cell::get), 0);
    for url in [
        "https://ol.epicgames.com/progression/acc/xp",
        "https://example.com/other",
    ] {
        let fake = Fake::new(url, 200);
        run(&fake, true, AuthMode::GlobalGameserver, None);
    }
    assert!(sets().is_empty());

    let mut in_flight = Fake::new(GRANT_URL, 200);
    in_flight.set_in_flight(true);
    run(&in_flight, true, AuthMode::GlobalGameserver, None);
    assert!(sets().is_empty());
    assert_eq!(GETS.with(Cell::get), 0);

    // A direct backend URL still gets one
    let direct = Fake::new("https://backend.invalid/progression/a/b/1", 0);
    run(&direct, true, AuthMode::GlobalGameserver, None);
    assert_eq!(sets().len(), 1);
}

#[test]
fn key_header_per_section_6() {
    let key = w("SECRETKEY");
    let keyed = |url: &str, server: bool, mode: AuthMode, key: Option<&[u16]>, flight: bool| {
        reset();
        let mut fake = Fake::new(url, 200);
        fake.set_in_flight(flight);
        run(&fake, server, mode, key);
        sets()
            .into_iter()
            .filter(|(name, _)| name == KEY_HEADER)
            .map(|(_, value)| value)
            .collect::<Vec<_>>()
    };
    let gg = AuthMode::GlobalGameserver;
    assert_eq!(
        keyed("https://auth.steelyard.ca/x", true, gg, Some(&key), false),
        ["SECRETKEY"]
    );
    assert_eq!(
        keyed("https://backend.invalid/direct", true, gg, Some(&key), false),
        ["SECRETKEY"]
    );
    assert_eq!(
        keyed("https://auth.steelyard.ca/x", true, gg, Some(&[]), false),
        [""],
        "empty sent empty"
    );
    assert!(
        keyed("https://example.com/x", true, gg, Some(&key), false).is_empty(),
        "another host"
    );
    assert!(
        keyed("https://backend.invalidx/x", true, gg, Some(&key), false).is_empty(),
        "a shared prefix"
    );
    assert!(
        keyed("https://auth.steelyard.ca/x", true, gg, Some(&key), true).is_empty(),
        "in flight"
    );
    assert!(
        keyed(
            "https://auth.steelyard.ca/x",
            true,
            AuthMode::PlayerHost,
            Some(&key),
            false
        )
        .is_empty()
    );
    assert!(keyed("https://auth.steelyard.ca/x", true, gg, None, false).is_empty());
    assert!(keyed("https://auth.steelyard.ca/x", false, gg, Some(&key), false).is_empty());
}

static SEEN: AtomicU32 = AtomicU32::new(0);

#[inline(never)]
extern "C" fn fake_process_request(request: *mut c_void) -> u8 {
    let fake = black_box(request) as *const u64;
    // SAFETY: the test's fake request
    let data = unsafe { *fake.add(0x70 / 8) } as *const u16;
    // SAFETY: as above: the URL starts with the API base after the redirect
    let first = unsafe { *data.add(8) };
    SEEN.fetch_add(1, Ordering::SeqCst);
    black_box(u8::from(first == u16::from(b'b')) + 1)
}

#[test]
fn detour_through_the_broker() {
    reset();
    let target = fake_process_request as *const () as usize;
    let _ = ALLOCATOR.set(fake_allocator());
    broker::hook(target, process_request_detour as *const () as usize, 0, &NEXT).unwrap();
    let fake = Fake::new("https://auth.steelyard.ca/tags", 100);
    type Call = extern "C" fn(*mut c_void) -> u8;
    // SAFETY: through the patched entry, so the detour runs
    let call: Call = unsafe { std::mem::transmute::<usize, Call>(black_box(target)) };
    let result = call(fake.address() as *mut c_void);
    assert_eq!(SEEN.load(Ordering::SeqCst), 1, "every request reaches the game");
    let base = config::backend_url().api_base();
    assert_eq!(fake.url(), format!("{base}/__origin/auth.steelyard.ca/tags"));
    assert_eq!(result, if base == BASE { 2 } else { 1 }, "the return passes through");
}
