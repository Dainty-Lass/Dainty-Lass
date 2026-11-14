use super::*;
use crate::config::launch::{Mode, parse_launch};
use crate::config::roster::map_url;
use crate::memory::BufferMemory;
use crate::wide;

const BASE: usize = 0x30_0000;
const SIZE: usize = 0x8000;

struct Bytes(Vec<u8>);

impl Bytes {
    fn new() -> Self {
        Self(vec![0u8; SIZE])
    }
    fn put(&mut self, at: usize, value: &[u8]) {
        let at = at - BASE;
        self.0[at..at + value.len()].copy_from_slice(value);
    }
    fn ptr(&mut self, at: usize, value: usize) {
        self.put(at, &(value as u64).to_le_bytes());
    }
    fn i32(&mut self, at: usize, value: i32) {
        self.put(at, &value.to_le_bytes());
    }
    fn memory(&self) -> BufferMemory<'_> {
        BufferMemory {
            base: BASE,
            bytes: &self.0,
        }
    }
}

#[test]
fn fstring_plans() {
    assert_eq!(
        fstring_plan(0, 0, true, 10),
        FStringPlan {
            realloc: true,
            num: 10,
            max: 10
        }
    );
    assert_eq!(
        fstring_plan(5, 9, false, 10),
        FStringPlan {
            realloc: true,
            num: 10,
            max: 10
        },
        "max below"
    );
    assert_eq!(
        fstring_plan(5, 10, false, 10),
        FStringPlan {
            realloc: false,
            num: 10,
            max: 10
        },
        "max at"
    );
    assert_eq!(
        fstring_plan(30, 40, false, 10),
        FStringPlan {
            realloc: false,
            num: 10,
            max: 40
        },
        "max above is kept"
    );
    assert!(
        fstring_plan(0, 64, true, 10).realloc,
        "a null buffer is always allocated"
    );
}

#[test]
fn map_url_never_listens() {
    let args: Vec<Vec<u16>> = [
        "game.exe",
        "K",
        "7777",
        "/Game/Maps/Hub",
        "B",
        "H",
        "p:1",
        "1.2.3.4:7777",
        "-server",
    ]
    .iter()
    .map(|arg| wide(arg))
    .collect();
    let launch = parse_launch(&args, &wide("game.exe -server"), AuthMode::GlobalGameserver).unwrap();
    assert_eq!(launch.mode, Mode::Full);
    let url = String::from_utf16_lossy(&map_url(&launch, 4, false));
    assert!(!url.contains("?listen"), "{url}");
    assert!(url.ends_with("?MaxPlayers=4"), "{url}");
    let line = map_url_line(&launch, url.encode_utf16().count(), 4);
    assert_eq!(
        line,
        format!(
            "[MitRuntime] [MapUrl] map=/Game/Maps/Hub length={} maxPlayers=4",
            url.encode_utf16().count()
        )
    );
    assert!(
        !line.contains("p:1") && !line.contains("HuntId"),
        "no roster or hunt id"
    );
}

#[test]
fn schedule() {
    let mut schedule = ListenSchedule::new();
    assert_eq!(schedule.poll(1_000, false), None);
    assert_eq!(schedule.poll(2_000, true), Some(Request::WorldPresent));
    assert!(schedule.requested());
    assert_eq!(schedule.poll(3_000, true), None, "once only");

    let mut schedule = ListenSchedule::new();
    assert_eq!(schedule.poll(10_000, false), None);
    assert_eq!(schedule.poll(10_000 + 29_999, false), None);
    assert_eq!(schedule.poll(10_000 + 30_000, false), Some(Request::Timeout));
    assert_eq!(schedule.poll(10_000 + 90_000, false), None);

    let mut schedule = ListenSchedule::new();
    assert_eq!(schedule.poll(5, true), Some(Request::WorldPresent), "the first frame");
}

#[test]
fn decisions() {
    assert_eq!(listen_decision(AuthMode::GlobalGameserver, false), Some(Decision::Mit));
    assert_eq!(listen_decision(AuthMode::GlobalGameserver, true), Some(Decision::Mit));
    assert_eq!(listen_decision(AuthMode::PlayerHost, true), Some(Decision::Mit));
    assert_eq!(
        listen_decision(AuthMode::PlayerHost, false),
        Some(Decision::AgplListens)
    );
    assert_eq!(listen_decision(AuthMode::Invalid, false), None);
    assert_eq!(listen_decision(AuthMode::Invalid, true), None);
}

#[test]
fn array_checks() {
    assert!(!world_list_ok(-1, true));
    assert!(world_list_ok(0, true));
    assert!(world_list_ok(64, true));
    assert!(!world_list_ok(65, true));
    assert!(!world_list_ok(1, false), "unreadable array");
    assert!(!named_drivers_ok(-1, 4));
    assert!(!named_drivers_ok(5, 4));
    assert!(named_drivers_ok(4, 128));
    assert!(!named_drivers_ok(4, 129));
}

#[test]
fn ready_lines_parse_as_the_director_does() {
    let id = "0f3c9a2e-4b1d-4e8a-9c7f-1a2b3c4d5e6f";
    let [first] = ready_lines(id, 8780);
    assert_eq!(
        first,
        "RUSTIC_ISLES_GAMESERVER_READY launchId=0f3c9a2e-4b1d-4e8a-9c7f-1a2b3c4d5e6f port=8780\r\n"
    );
    // rustic-isles-director's `parse_marker`, copied: trim, the marker, a lowercase-hex-and-hyphen id, a port
    let parse = |line: &str| -> Option<(String, String)> {
        let rest = line
            .trim()
            .strip_prefix("RUSTIC_ISLES_GAMESERVER_READY")?
            .strip_prefix(" launchId=")?;
        let (id, port) = rest.split_once(" port=")?;
        let id_ok = !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-');
        let port_ok = !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
        (id_ok && port_ok).then(|| (id.to_owned(), port.to_owned()))
    };
    assert_eq!(parse(&first), Some((id.to_owned(), "8780".to_owned())));
}

#[test]
fn furl() {
    let block = furl_block(8780, None);
    assert_eq!(block.len(), 0x68);
    assert_eq!(&block[0x20..0x24], &8780i32.to_le_bytes());
    assert!(
        block
            .iter()
            .enumerate()
            .all(|(at, &byte)| (0x20..0x24).contains(&at) || byte == 0)
    );
    let block = furl_block(7777, Some((0x1122_3344_5566_7788, 10)));
    assert_eq!(&block[0x10..0x18], &0x1122_3344_5566_7788u64.to_le_bytes());
    assert_eq!(&block[0x18..0x20], &[10, 0, 0, 0, 10, 0, 0, 0]);
    assert_eq!(&block[0x20..0x24], &7777i32.to_le_bytes());
    assert!(block[..0x10].iter().chain(&block[0x24..]).all(|&byte| byte == 0));
}

#[test]
fn pins() {
    assert_eq!(
        pin_check("127.0.0.1", Some("127.0.0.1")),
        Ok("[MitRuntime] [Listen] engine MULTIHOME matches the pinned listen address 127.0.0.1".to_owned())
    );
    assert!(
        pin_check("Host.Example", Some("host.example")).is_ok(),
        "ASCII case ignored"
    );
    assert_eq!(
        pin_check("127.0.0.1", None),
        Err(
            "[MitRuntime] [Listen] failed: pinned listen address 127.0.0.1 is not the engine's MULTIHOME (none)"
                .to_owned()
        )
    );
    assert_eq!(
        pin_check("127.0.0.1", Some("0.0.0.0")),
        Err(
            "[MitRuntime] [Listen] failed: pinned listen address 127.0.0.1 is not the engine's MULTIHOME (0.0.0.0)"
                .to_owned()
        )
    );
}

#[test]
fn names_and_notes() {
    let name = name_from_bytes([0x34, 0x12, 0, 0, 2, 0, 0, 0]);
    assert_eq!(name, (0x1234, 2));
    assert_eq!(name_value(name), 0x0000_0002_0000_1234);
    assert_eq!(name_value((-1, 0)), 0xFFFF_FFFF);
    assert_eq!(world_driver_note(0, 5), "was null");
    assert_eq!(world_driver_note(5, 5), "already ours");
    assert_eq!(world_driver_note(6, 5), "replaced another driver");
    assert_eq!(
        found_line(0xABC, false),
        "[MitRuntime] [Listen] GameNetDriver 0xabc (active drivers)"
    );
    assert_eq!(
        found_line(0xABC, true),
        "[MitRuntime] [Listen] GameNetDriver 0xabc (object scan)"
    );
    assert_eq!(
        listening_line(7777, 0x10),
        "[MitRuntime] [Listen] listening on port 7777, driver 0x10"
    );
    assert_eq!(
        skipped_line(Skip::NoWorld),
        "[MitRuntime] [Listen] driver maintenance skipped: no world"
    );
}

const ENGINE: usize = BASE;
const LIST: usize = BASE + 0x1000;
const CONTEXT: usize = BASE + 0x1100;
const DRIVERS: usize = BASE + 0x1400;
const DRIVER: usize = BASE + 0x1800;
const WORLD: usize = BASE + 0x7F00;

fn engine_with_context(count: i32) -> Bytes {
    let mut bytes = Bytes::new();
    bytes.ptr(ENGINE + 0xC38, LIST);
    bytes.i32(ENGINE + 0xC40, count);
    bytes.ptr(LIST, 0);
    bytes.ptr(LIST + 8, CONTEXT);
    bytes.ptr(CONTEXT + 0x280, WORLD);
    bytes
}

#[test]
fn world_contexts() {
    let bytes = engine_with_context(2);
    assert_eq!(find_world_context(&bytes.memory(), ENGINE, WORLD), Some(CONTEXT));
    assert_eq!(find_world_context(&bytes.memory(), ENGINE, WORLD + 8), None);
    assert_eq!(find_world_context(&bytes.memory(), ENGINE, 0), None);
    assert_eq!(
        find_world_context(&engine_with_context(1).memory(), ENGINE, WORLD),
        None
    );
    assert_eq!(
        find_world_context(&engine_with_context(65).memory(), ENGINE, WORLD),
        None
    );
    assert_eq!(
        find_world_context(&engine_with_context(-1).memory(), ENGINE, WORLD),
        None
    );
    let mut bytes = engine_with_context(2);
    bytes.ptr(ENGINE + 0xC38, BASE + SIZE - 8);
    assert_eq!(
        find_world_context(&bytes.memory(), ENGINE, WORLD),
        None,
        "array unreadable"
    );
}

#[test]
fn named_drivers() {
    let name = (77, 0);
    let mut bytes = engine_with_context(2);
    bytes.ptr(CONTEXT + 0x220, DRIVERS);
    bytes.i32(CONTEXT + 0x228, 2);
    bytes.i32(CONTEXT + 0x22C, 4);
    bytes.ptr(DRIVERS, DRIVER + 0x400);
    bytes.ptr(DRIVERS + 0x10, DRIVER);
    bytes.put(DRIVER + 0x190, &[77, 0, 0, 0, 0, 0, 0, 0]);
    bytes.put(DRIVER + 0x400 + 0x190, &[78, 0, 0, 0, 0, 0, 0, 0]);
    let memory = bytes.memory();
    let drivers = active_drivers(&memory, CONTEXT).unwrap();
    assert_eq!(drivers, [DRIVER + 0x400, DRIVER]);
    let found: Vec<usize> = drivers
        .into_iter()
        .filter(|&driver| driver_matches(&memory, driver, name, |_| true))
        .collect();
    assert_eq!(found, [DRIVER]);
    assert!(!driver_matches(&memory, DRIVER, name, |_| false), "not a NetDriver");
    assert!(!driver_matches(&memory, DRIVER, (77, 1), |_| true), "the number counts");
    let mut bytes = engine_with_context(2);
    bytes.i32(CONTEXT + 0x228, 5);
    bytes.i32(CONTEXT + 0x22C, 4);
    assert_eq!(active_drivers(&bytes.memory(), CONTEXT), None, "Num > Max");
    bytes.i32(CONTEXT + 0x22C, 129);
    assert_eq!(active_drivers(&bytes.memory(), CONTEXT), None, "Max over 128");
}

const CONNECTION: usize = BASE + 0x2000;
const CHANNELS: usize = BASE + 0x2100;
const CHANNEL: usize = BASE + 0x2200;
const ACTOR_CHANNEL: usize = BASE + 0x3000;
const OTHER_CLASS: usize = BASE + 0x3100;
const ACTOR: usize = BASE + 0x3200;

fn connection_with(channels: &[(usize, usize, usize)], num: i32, max: i32) -> Bytes {
    let mut bytes = Bytes::new();
    bytes.ptr(CONNECTION + 0x70, CHANNELS);
    bytes.i32(CONNECTION + 0x78, num);
    bytes.i32(CONNECTION + 0x7C, max);
    for (index, &(at, class, actor)) in channels.iter().enumerate() {
        bytes.ptr(CHANNELS + index * 8, at);
        if at.is_multiple_of(8) {
            bytes.ptr(at + 0x10, class);
            bytes.ptr(at + 0x70, actor);
        }
    }
    bytes
}

#[test]
fn channels() {
    let three = [
        (CHANNEL, OTHER_CLASS, ACTOR),
        (CHANNEL + 0x100, ACTOR_CHANNEL, ACTOR + 8),
        (CHANNEL + 0x200, ACTOR_CHANNEL, ACTOR),
    ];
    let bytes = connection_with(&three, 3, 4);
    let memory = bytes.memory();
    assert_eq!(
        actor_channel(&memory, CONNECTION, ACTOR, ACTOR_CHANNEL),
        Some(CHANNEL + 0x200)
    );
    assert_eq!(channel_presence(&memory, ACTOR, CONNECTION, ACTOR_CHANNEL), 1);
    assert_eq!(channel_presence(&memory, ACTOR + 0x10, CONNECTION, ACTOR_CHANNEL), -1);
    assert_eq!(channel_presence(&memory, 0, CONNECTION, ACTOR_CHANNEL), -2);
    assert_eq!(channel_presence(&memory, ACTOR, BASE + SIZE - 8, ACTOR_CHANNEL), -2);

    let unaligned = connection_with(&[(CHANNEL + 4, ACTOR_CHANNEL, ACTOR)], 1, 1);
    assert_eq!(
        actor_channel(&unaligned.memory(), CONNECTION, ACTOR, ACTOR_CHANNEL),
        None
    );
    let over = connection_with(&three, 4, 3);
    assert_eq!(
        actor_channel(&over.memory(), CONNECTION, ACTOR, ACTOR_CHANNEL),
        None,
        "Num > Max"
    );
    let huge = connection_with(&three, 4097, 4097);
    assert_eq!(
        actor_channel(&huge.memory(), CONNECTION, ACTOR, ACTOR_CHANNEL),
        None,
        "over 4096"
    );
}

#[test]
fn connections() {
    let mut bytes = Bytes::new();
    bytes.ptr(DRIVER + 0x90, CHANNELS);
    bytes.i32(DRIVER + 0x98, 3);
    bytes.i32(DRIVER + 0x9C, 3);
    bytes.ptr(CHANNELS, CONNECTION);
    bytes.ptr(CHANNELS + 8, 0);
    bytes.ptr(CHANNELS + 16, CONNECTION + 0x100);
    assert_eq!(
        client_connections(&bytes.memory(), DRIVER),
        [CONNECTION, CONNECTION + 0x100],
        "null entries kept out"
    );
    bytes.i32(DRIVER + 0x98, 1025);
    bytes.i32(DRIVER + 0x9C, 1025);
    assert!(client_connections(&bytes.memory(), DRIVER).is_empty(), "over 1024");
}
