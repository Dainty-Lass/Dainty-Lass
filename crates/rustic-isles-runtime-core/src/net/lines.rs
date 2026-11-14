//! Every `[MitNet]` text (4.3): addresses as `0x<HEX>` (`null` for 0), counts in decimal.

use super::Collections;

/// R11's tag (R0 call 6).
pub const TAG: &str = "[MitNet]";

/// `0x<HEX>`, upper case, no padding; `null` for 0.
pub fn addr(ptr: usize) -> String {
    if ptr == 0 {
        "null".to_owned()
    } else {
        format!("0x{ptr:X}")
    }
}

/// The line a budget writes in place of its `B+1`-th.
pub fn suppressed(kind: &str) -> String {
    format!("{TAG} {kind}: further lines suppressed")
}

/// The sanitiser's budget kind for `site`.
pub fn sanitize_kind(site: &str) -> String {
    format!("sanitize {site}")
}

/// `[MitNet] sanitize <site>: <text>`.
pub fn sanitize_line(site: &str, text: &str) -> String {
    format!("{TAG} sanitize {site}: {text}")
}

/// The sanitiser's texts (5).
pub mod sanitize {
    use super::addr;

    /// Step 1.
    pub const INVALID_DRIVER: &str = "invalid driver";
    /// Step 2.
    pub const UNREADABLE_CONNECTIONS: &str = "unreadable connection list";
    /// Step 5.
    pub const UNREADABLE_CLEANUP: &str = "unreadable cleanup list";

    /// Step 3.
    pub fn reset_connections(data: usize, num: i32, max: i32) -> String {
        format!("reset connection list data={} num={num} max={max}", addr(data))
    }

    /// Step 4, one per dropped element.
    pub fn dropped(index: usize, ptr: usize) -> String {
        format!("dropped connection index={index} ptr={}", addr(ptr))
    }

    /// Step 4, after the dropped lines.
    pub fn compacted(old: i32, new: i32) -> String {
        format!("compacted connection list {old} -> {new}")
    }

    /// Step 6.
    pub fn reset_cleanup(data: usize, num: i32, max: i32) -> String {
        format!("reset cleanup list data={} num={num} max={max}", addr(data))
    }
}

/// NETD-1's budget kind.
pub const DISPATCH_SKIPPED_KIND: &str = "TickDispatch skipped";
/// NETD-1, absorbed.
pub const DISPATCH_SKIPPED: &str = "[MitNet] TickDispatch skipped this frame: unsafe connection state";

/// NETD-1's first call.
pub fn dispatch_first(driver: usize) -> String {
    format!("{TAG} TickDispatch first call driver={}", addr(driver))
}

/// 7.3 step 3.
pub fn registration_deferred(world: usize, driver_world: usize, collections: &Collections) -> String {
    let count = match collections {
        Collections::Unavailable => "unavailable".to_owned(),
        Collections::Unusable(num) => num.to_string(),
        Collections::Types(types) => types.len().to_string(),
    };
    format!(
        "{TAG} level collections: registration deferred world={} driverWorld={} collections={count}",
        addr(world),
        addr(driver_world)
    )
}

/// 7.3 step 4.
pub fn registration_refused(world: usize, resolved: usize, driver: usize) -> String {
    format!(
        "{TAG} level collections: registration refused, the world's context does not resolve this driver world={} \
         resolved={} driver={}",
        addr(world),
        addr(resolved),
        addr(driver)
    )
}

/// 7.3 step 5.
pub fn registration_done(count: usize, world: usize, driver: usize) -> String {
    format!(
        "{TAG} level collections: driver set on {count} gameplay collection(s) world={} driver={}",
        addr(world),
        addr(driver)
    )
}

/// 7.3: `FindNamedNetDriver`'s check failed.
pub const FIND_NAMED_MISMATCH: &str =
    "[MitNet] level collections: FindNamedNetDriver prologue mismatch, registration off";

/// The manual tick's two calls (8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickCall {
    /// The IP driver's `TickDispatch`.
    Dispatch,
    /// `UNetDriver::TickFlush`.
    Flush,
}

impl TickCall {
    /// Its name in the lines.
    pub fn name(self) -> &'static str {
        match self {
            TickCall::Dispatch => "TickDispatch",
            TickCall::Flush => "TickFlush",
        }
    }
}

/// 8.3: a call target's check failed.
pub fn manual_tick_mismatch(call: TickCall, rva: u32) -> String {
    format!(
        "{TAG} manual net tick: {} prologue mismatch at +0x{rva:08X}, call off",
        call.name()
    )
}

/// 8.3: the first frame that calls both.
pub fn manual_tick_on(driver: usize) -> String {
    format!("{TAG} manual net tick on driver={}", addr(driver))
}

/// A replication driver's class as NETD-2 and NETD-3 print it: its short name, `null` for no object, `no-class`
/// when its class cannot be named.
pub fn class_text(object: usize, class_name: Option<&str>) -> String {
    match (object, class_name) {
        (0, _) => "null".to_owned(),
        (_, Some(name)) => name.to_owned(),
        (_, None) => "no-class".to_owned(),
    }
}

/// NETD-2 step 1.
pub fn set_rep_driver(n: u32, driver: usize, rep_driver: usize, class: &str) -> String {
    format!(
        "{TAG} SetReplicationDriver #{n} driver={} repDriver={} class={class}",
        addr(driver),
        addr(rep_driver)
    )
}

/// 9.1: the rate boost's first write.
pub fn rates_boosted(old_rate: f32, old_min: f32) -> String {
    format!("{TAG} player default object: NetUpdateFrequency {old_rate} -> 60, MinNetUpdateFrequency {old_min} -> 30")
}

/// 9.1: the always-relevant bit's first write.
pub fn always_relevant_set(old: u8, new: u8) -> String {
    format!("{TAG} player default object: bAlwaysRelevant set (byte 0x{old:02X} -> 0x{new:02X})")
}

/// 9.1's budget kind.
pub const DEFAULT_OBJECT_MISSING_KIND: &str = "player default object";
/// 9.1: the class is not loaded yet.
pub const DEFAULT_OBJECT_MISSING: &str = "[MitNet] player default object: BP_PlayerCharacter_C not loaded yet";

/// 10 step 1.
pub fn create_gate(n: u32, old: u32) -> String {
    format!("{TAG} CreateReplicationDriver #{n} gate {old} -> 1")
}

/// 10 step 2.
pub fn create_class(n: u32, before: usize, after: usize, name: &str) -> String {
    format!(
        "{TAG} CreateReplicationDriver #{n} class before={} after={} ({name})",
        addr(before),
        addr(after)
    )
}

/// 10 step 5.
pub fn create_constructed(n: u32, object: usize, class: &str) -> String {
    format!(
        "{TAG} CreateReplicationDriver #{n} constructed {} ({class})",
        addr(object)
    )
}

/// 10 step 5, a null answer.
pub fn create_construct_null(n: u32) -> String {
    format!("{TAG} CreateReplicationDriver #{n} construct returned null, calling the game's own")
}

/// 10 step 8.
pub fn create_game_returned(n: u32, object: usize, class: &str) -> String {
    format!(
        "{TAG} CreateReplicationDriver #{n} game returned {} ({class})",
        addr(object)
    )
}

/// 10: `StaticConstructObject_Internal`'s check failed.
pub const CONSTRUCT_MISMATCH: &str =
    "[MitNet] CreateReplicationDriver: StaticConstructObject_Internal prologue mismatch, self-construct off";

/// 10.1, the first search.
pub fn graph_class_line(chosen: Option<(usize, &str)>) -> String {
    match chosen {
        Some((class, name)) => format!("{TAG} replication graph class: {name} {}", addr(class)),
        None => format!("{TAG} replication graph class: none {}", addr(0)),
    }
}

/// NETD-6's role word.
pub fn role(server: bool) -> &'static str {
    if server { "server" } else { "client" }
}

/// 12 step 2.
pub fn close_invalid(server: bool, seq: u32, connection: usize) -> String {
    format!(
        "{TAG} Close {} #{seq} invalid connection={}",
        role(server),
        addr(connection)
    )
}

/// 12 step 4; `stack` is the frames joined by single spaces.
pub fn close_enter(server: bool, seq: u32, connection: usize, state: u8, last_receive: f64, stack: &str) -> String {
    format!(
        "{TAG} Close {} #{seq} enter conn={} state={state} lastReceive={last_receive:.6} stack={stack}",
        role(server),
        addr(connection)
    )
}

/// 12 step 6.
pub fn close_exit(server: bool, seq: u32, connection: usize) -> String {
    format!("{TAG} Close {} #{seq} exit conn={}", role(server), addr(connection))
}

/// NETD-7's budget kind.
pub const SPLINE_KIND: &str = "spline guard";

/// 13.1 step 4.
pub fn spline_unusable(component: usize, data: usize, num: i32) -> String {
    format!(
        "{TAG} spline guard: unusable curve on component={} data={} num={num}, returned a zero direction",
        addr(component),
        addr(data)
    )
}

/// 13.1, once at install.
pub fn spline_guard(on: bool) -> &'static str {
    if on {
        "[MitNet] spline guard on"
    } else {
        "[MitNet] spline guard off (DISABLE_SPLINE_GUARD.flag)"
    }
}

/// Install reasons (4.2, 13.3).
pub mod install {
    /// The target's first bytes differ.
    pub const PROLOGUE_MISMATCH: &str = "prologue mismatch";
    /// NETD-8.
    pub const NOT_PORTED: &str = "not ported";
    /// NETD-7 off by its flag.
    pub const FLAG_SPLINE: &str = "flag DISABLE_SPLINE_GUARD";
}
