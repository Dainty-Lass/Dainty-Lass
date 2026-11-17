//! R14, the player role (spec-runtime-r14): PR-1 on both roles, RG-3, `SP-2@R14`, `SP-3@R14` on the server and
//! `SP-1@R14` on the client. The decisions and texts are `core::player_role`'s; this module reads, writes and calls.

mod apply;
mod client;
mod server;
#[cfg(test)]
mod tests;

use super::broker::NextCell;
use super::exit::Install;
use super::loading::{game, readiness};
use super::memory::{query, read_bytes, readable, readable_unaligned, verify_entry};
use super::{engine, flags, log, minhook, reflect, win};
use crate::handlers::{self, PeHandler, Point, TickHandler};
use crate::listen::{client_connections, net_driver};
use rustic_isles_runtime_core::names::Kind;
use rustic_isles_runtime_core::net::install::PROLOGUE_MISMATCH;
use rustic_isles_runtime_core::offsets::{FoundBy, r14, target};
use rustic_isles_runtime_core::player_role::{self as core, Captured, Game, Invoke, Route, State, order};
use rustic_isles_runtime_core::reflect::{is_writable_protect, param_buffer_len};
use rustic_isles_runtime_core::registry::owns;
use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

thread_local! {
    // Game-thread state (4.4); the client's handlers run only on the game thread (r14-omnicell-fix 2)
    static STATE: State = const { State::new() };
}

/// Runs `body` on this thread's state. The state's cells allow the re-entry game calls make.
fn with_state<R>(body: impl FnOnce(&State) -> R) -> Option<R> {
    STATE.try_with(|state| body(state)).ok()
}

/// The one `[Panic]` line of an id (4.1).
fn panicked(id: &str, flag: &AtomicBool) {
    if !flag.swap(true, Ordering::SeqCst) {
        log::line(&format!("[Panic] {id} body panicked; the call goes on to the game"));
    }
}

/// A checked write with no alignment rule: R14 writes single bytes at odd offsets (`bOnlineDataReady`).
fn write_bytes(addr: usize, bytes: &[u8]) -> bool {
    if bytes.is_empty()
        || !readable_unaligned(addr, bytes.len())
        || !query(addr).is_some_and(|region| is_writable_protect(region.protect))
    {
        return false;
    }
    // SAFETY: the range was just checked committed, readable and writable, inside one region
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
    true
}

/// `base + rva` of this executable.
fn address(rva: u32) -> usize {
    usize::try_from(rva)
        .ok()
        .and_then(|rva| win::image_base().checked_add(rva))
        .unwrap_or(0)
}

/// A game function R14 calls by pointer, its first bytes checked once.
struct Checked {
    rva: u32,
    bytes: &'static [u8],
    ok: OnceLock<bool>,
}

impl Checked {
    const fn new(rva: u32, bytes: &'static [u8]) -> Self {
        Self {
            rva,
            bytes,
            ok: OnceLock::new(),
        }
    }

    /// The function's address once its bytes checked, else 0.
    fn address(&self) -> usize {
        if *self.ok.get_or_init(|| verify_entry(self.rva, self.bytes)) {
            address(self.rva)
        } else {
            0
        }
    }
}

static APPLY: Checked = Checked::new(r14::APPLY_PLAYER_ROLE, r14::APPLY_PLAYER_ROLE_BYTES);
static ACTIVE_SLOT_DATA: Checked = Checked::new(r14::LOADOUT_ACTIVE_SLOT_DATA, r14::LOADOUT_ACTIVE_SLOT_DATA_BYTES);
static SLOT_ROLE_ITEM: Checked = Checked::new(r14::SLOT_ROLE_ITEM, r14::SLOT_ROLE_ITEM_BYTES);
static TRY_ACTIVATE: Checked = Checked::new(
    r14::INTERNAL_TRY_ACTIVATE_ABILITY,
    r14::INTERNAL_TRY_ACTIVATE_ABILITY_BYTES,
);

type GetterFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

/// One of 5.1's getters on an argument the caller checked readable.
fn getter(function: usize, argument: usize) -> usize {
    if function == 0 || argument == 0 || super::detaching() {
        return 0;
    }
    // SAFETY: a getter of this build, `(object) -> object`, its first bytes checked; the argument was checked
    // readable for 0x40 in this call
    unsafe { std::mem::transmute::<usize, GetterFn>(function)(argument as *mut c_void) as usize }
}

/// The `ArchonPlayerRole` class, found once.
fn is_role(actor: usize) -> bool {
    static CLASS: AtomicUsize = AtomicUsize::new(0);
    let mut class = CLASS.load(Ordering::Relaxed);
    if class == 0 {
        class = reflect::find_class("ArchonPlayerRole").unwrap_or(0);
        CLASS.store(class, Ordering::Relaxed);
    }
    if class == 0 {
        return false;
    }
    let Some(engine) = reflect::engine() else {
        return false;
    };
    rustic_isles_runtime_core::reflect::class_of(engine.memory(), actor)
        .is_some_and(|own| rustic_isles_runtime_core::reflect::class_chain(engine.memory(), own).contains(&class))
}

/// `params` into an 8-aligned block of R17's size, and back after the call.
fn invoke(object: usize, function: &str, route: Route, params: &mut [u8]) -> Invoke {
    if object == 0 || !engine::is_live_object(object) {
        return Invoke::Refused;
    }
    let Some(class) = reflect::class_of(object) else {
        return Invoke::Refused;
    };
    let Some(function) = reflect::function(class, function) else {
        return Invoke::NotFound;
    };
    let mut block = vec![0u64; param_buffer_len(params.len()) / 8];
    for (at, &byte) in params.iter().enumerate() {
        block[at / 8] |= u64::from(byte) << ((at % 8) * 8);
    }
    let called = match route {
        Route::Original => game::process_event_original(object, function, &mut block),
        Route::Hooked => game::call(object, function, &mut block),
    };
    for (at, byte) in params.iter_mut().enumerate() {
        *byte = (block[at / 8] >> ((at % 8) * 8)) as u8;
    }
    if called { Invoke::Called } else { Invoke::Refused }
}

type TryActivateFn =
    unsafe extern "C" fn(*mut c_void, u32, *const [u8; 16], *mut usize, *const c_void, *const c_void) -> u8;

/// `InternalTryActivateAbility` at `function` (the tests pass a fake): `on_ended` is always null.
pub fn call_try_activate(function: usize, component: usize, handle: u32, key: &mut [u8; 16], event: usize) -> bool {
    if function == 0 || component == 0 || super::detaching() {
        return false;
    }
    let mut instanced = 0usize;
    // SAFETY: the function's first bytes checked (or a test's own); its arguments are a live component from the
    // RPC, R14's own key copy and slot, and the RPC's own event data or null
    let activated = unsafe {
        std::mem::transmute::<usize, TryActivateFn>(function)(
            component as *mut c_void,
            handle,
            key,
            &mut instanced,
            std::ptr::null(),
            event as *const c_void,
        )
    };
    activated != 0
}

/// This process.
pub struct ProcessGame;

impl Game for ProcessGame {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        readable(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        read_bytes(addr, out)
    }
    fn write(&self, addr: usize, bytes: &[u8]) -> bool {
        write_bytes(addr, bytes)
    }
    fn is_live_object(&self, ptr: usize) -> bool {
        engine::is_live_object(ptr)
    }
    fn is_role(&self, actor: usize) -> bool {
        is_role(actor)
    }
    fn now_ms(&self) -> u64 {
        win::tick()
    }
    fn log(&self, line: &str) {
        log::line(line);
    }
    fn quiet(&self) -> bool {
        flags::production_log_profile()
    }
    fn name(&self, object: usize) -> Option<String> {
        reflect::engine()?.object_name(object)
    }
    fn active_slot_data(&self, loadout: usize) -> usize {
        getter(ACTIVE_SLOT_DATA.address(), loadout)
    }
    fn slot_role_item(&self, inter: usize) -> usize {
        getter(SLOT_ROLE_ITEM.address(), inter)
    }
    fn apply(&self, loadout: usize, wrapper: bool) {
        apply::call(loadout, wrapper);
    }
    fn invoke(&self, object: usize, function: &str, route: Route, params: &mut [u8]) -> Invoke {
        invoke(object, function, route, params)
    }
    fn ability_ready(&self) -> bool {
        static LOGGED: AtomicBool = AtomicBool::new(false);
        let ready = TRY_ACTIVATE.address() != 0;
        if !ready && !LOGGED.swap(true, Ordering::SeqCst) {
            log::line(core::ABILITY_DISABLED);
        }
        ready
    }
    fn try_activate(&self, component: usize, handle: u32, key: &mut [u8; 16], event_data: usize) -> bool {
        call_try_activate(TRY_ACTIVATE.address(), component, handle, key, event_data)
    }
    fn captured(&self) -> Option<Captured> {
        readiness::captured()
    }
    fn chain_for(&self, pc: usize, expected_pawn: Option<usize>) -> Option<usize> {
        readiness::chain_for(pc, expected_pawn)
    }
    fn possession_stable(&self, pc: usize) -> bool {
        readiness::possession_stable(pc).is_some()
    }
    fn connection_count(&self) -> Option<usize> {
        net_driver().map(|driver| client_connections(driver).len())
    }
    fn lifecycle_replay_off(&self) -> bool {
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| flags::flag(core::flags::DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY))
    }
}

/// The HUD names of 12.2 (`FunctionName`, RT-18).
const HUD_NAMES: &[(Kind, &str)] = &[
    (Kind::FunctionName, "OnAirshipStateUpdate"),
    (Kind::FunctionName, "ShowGameplayHUD"),
    (Kind::FunctionName, "Player_HUD_Ready"),
    (Kind::FunctionName, "Progression_HUD_Ready"),
    (Kind::FunctionName, "ReceiveGameplayStart"),
    (Kind::FunctionName, "Refresh_HUD_Widget_Visibility"),
    (Kind::FunctionName, "Show_HUD"),
    (Kind::FunctionName, "Show HUD"),
    (Kind::FunctionName, "OnHUDVisibilityChanged"),
];

fn pe(order: i32, matcher: rustic_isles_runtime_core::names::Matcher) -> PeHandler {
    PeHandler {
        slice: "R14",
        order,
        matcher,
        before: None,
        after: None,
    }
}

/// Init, before `handlers::freeze()`: the handlers of 4.4 for this process's role.
pub fn register(server: bool) {
    if server {
        if owns(b"SP-2@R14") {
            let _ = handlers::register_pe(
                Point::Sp2,
                PeHandler {
                    before: Some(server::ability),
                    ..pe(
                        order::ABILITY,
                        &[
                            (
                                Kind::Exact,
                                "Function GameplayAbilities.AbilitySystemComponent.ServerTryActivateAbility",
                            ),
                            (
                                Kind::Exact,
                                "Function GameplayAbilities.AbilitySystemComponent.ServerTryActivateAbilityWithEventData",
                            ),
                        ],
                    )
                },
            );
        }
        if owns(b"SP-3@R14") {
            let _ = handlers::register_tick(TickHandler {
                slice: "R14",
                order: order::SERVER_PUMP,
                before: None,
                after: Some(server::tick),
            });
        }
        return;
    }
    if !owns(b"SP-1@R14") {
        return;
    }
    let handlers = [
        PeHandler {
            before: Some(client::airship_capture),
            ..pe(order::AIRSHIP_CAPTURE, &[(Kind::FunctionName, "OnAirshipStateUpdate")])
        },
        PeHandler {
            before: Some(client::pump),
            ..pe(order::PUMP, &[(Kind::Prefix, "")])
        },
        PeHandler {
            after: Some(client::client_restart),
            ..pe(
                order::CLIENT_RESTART,
                &[(Kind::Exact, "Function Engine.PlayerController.ClientRestart")],
            )
        },
        PeHandler {
            after: Some(client::input_mode_changed),
            ..pe(
                order::INPUT_MODE,
                &[(
                    Kind::Exact,
                    "Function BP_PlayerCharacter.BP_PlayerCharacter_C.OnInputModeChanged",
                )],
            )
        },
        PeHandler {
            after: Some(client::hud),
            ..pe(order::HUD, HUD_NAMES)
        },
        PeHandler {
            after: Some(client::sheathe),
            ..pe(
                order::SHEATHE,
                &[(
                    Kind::Exact,
                    "Function BP_PlayerCharacter.BP_PlayerCharacter_C.OnSheathedStateChanged",
                )],
            )
        },
    ];
    for handler in handlers {
        let _ = handlers::register_pe(Point::Sp1, handler);
    }
}

/// One install line, hooked after its first-bytes check (4.4); `None` when R14 does not own the id.
fn install(
    id: &'static str,
    base: usize,
    hooks_off: Option<&'static str>,
    wrapper: usize,
    next: &'static NextCell,
) -> Option<Install> {
    if !owns(id.as_bytes()) {
        return None;
    }
    let Some(line) = target(id) else {
        return Some((id, Err(PROLOGUE_MISMATCH)));
    };
    let FoundBy::RvaAndBytes(prologue) = line.found_by else {
        return Some((id, Err(PROLOGUE_MISMATCH)));
    };
    if !verify_entry(line.rva, prologue) {
        return Some((id, Err(PROLOGUE_MISMATCH)));
    }
    if let Some(status) = hooks_off {
        return Some((id, Err(status)));
    }
    let Some(address) = usize::try_from(line.rva).ok().and_then(|rva| base.checked_add(rva)) else {
        return Some((id, Err(PROLOGUE_MISMATCH)));
    };
    Some((
        id,
        super::broker::hook(address, wrapper, 0, next).map_err(minhook::status_name),
    ))
}

/// PR-1's install line (both roles).
pub fn install_pr1(base: usize, hooks_off: Option<&'static str>) -> Option<Install> {
    install("PR-1", base, hooks_off, apply::DETOUR as usize, &apply::NEXT)
}

/// RG-3's install line (server).
pub fn install_rg3(base: usize, hooks_off: Option<&'static str>) -> Option<Install> {
    install(
        "RG-3",
        base,
        hooks_off,
        server::REMOVE_DETOUR as usize,
        &server::REMOVE_NEXT,
    )
}
