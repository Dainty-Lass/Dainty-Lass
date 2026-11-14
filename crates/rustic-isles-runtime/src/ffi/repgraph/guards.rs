//! The four detours (7–10). Each decides through `core::repgraph` first and calls `next` only on `Verdict::Next`;
//! a panic fails open for that call (4.1).

use super::identity::{ProcessGame, image_rva};
use super::{BUDGETS, hook, panicked, with_state};
use crate::ffi::broker::NextCell;
use crate::ffi::detaching;
use crate::ffi::exit::{Install, first_foreign_frame};
use rustic_isles_runtime_core::repgraph::{self as core, PreChannel, Rpc, Verdict};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;

type Ptr = *mut c_void;

/// `ReplicateSingleActor(graph, actor, conn_actor_info, global_info, actor_info_map, conn_manager, frame)`.
pub type ReplicateSingleFn = unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr, Ptr, Ptr, u32) -> u64;
/// `SetChannelActor(channel, actor, flags)`.
pub type SetChannelActorFn = unsafe extern "C" fn(Ptr, Ptr, u64);
/// `ReplicateActor(channel)`: the bits written.
pub type ReplicateActorFn = unsafe extern "C" fn(Ptr) -> i64;
/// `ProcessRemoteFunctionForChannel` (UE 4.26's order).
pub type RemoteFunctionFn = unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr, Ptr, Ptr, Ptr, Ptr, Ptr, Ptr, u8, i32, Ptr);

static PRE_NEXT: NextCell = NextCell::new();
static BIND_NEXT: NextCell = NextCell::new();
static SEND_NEXT: NextCell = NextCell::new();
static RPC_NEXT: NextCell = NextCell::new();
static PRE_PANICKED: AtomicBool = AtomicBool::new(false);
static BIND_PANICKED: AtomicBool = AtomicBool::new(false);
static SEND_PANICKED: AtomicBool = AtomicBool::new(false);
static RPC_PANICKED: AtomicBool = AtomicBool::new(false);

/// 4.1: the verdict, or `Next` after one `[Panic]` line.
fn decide(id: &str, flag: &AtomicBool, body: impl FnOnce() -> Verdict) -> Verdict {
    crate::lifecycle::write_status_once();
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| {
        panicked(id, flag);
        Verdict::Next
    })
}

/// RG-4 with its `next` (tests pass their own).
#[allow(clippy::too_many_arguments, reason = "the engine function's own arguments")]
pub fn pre_channel_with(
    next: Option<ReplicateSingleFn>,
    graph: Ptr,
    actor: Ptr,
    conn_actor_info: Ptr,
    global_info: Ptr,
    actor_info_map: Ptr,
    conn_manager: Ptr,
    frame: u32,
) -> u64 {
    let call_next = || match next {
        // SAFETY: the original arguments to the original function
        Some(next) => unsafe {
            next(
                graph,
                actor,
                conn_actor_info,
                global_info,
                actor_info_map,
                conn_manager,
                frame,
            )
        },
        None => 0,
    };
    if detaching() {
        return call_next();
    }
    let verdict = decide("RG-4", &PRE_PANICKED, || {
        with_state(|state| {
            core::pre_channel(
                &ProcessGame,
                state,
                &BUDGETS,
                PreChannel {
                    actor: actor as usize,
                    conn_actor_info: conn_actor_info as usize,
                    conn_manager: conn_manager as usize,
                    frame,
                },
            )
        })
    });
    match verdict {
        // Nothing written: no channel for that controller on that connection this frame
        Verdict::Absorb => 0,
        Verdict::Next => call_next(),
    }
}

unsafe extern "C" fn pre_channel_detour(
    graph: Ptr,
    actor: Ptr,
    conn_actor_info: Ptr,
    global_info: Ptr,
    actor_info_map: Ptr,
    conn_manager: Ptr,
    frame: u32,
) -> u64 {
    // SAFETY: the broker wrote a function of this signature into the cell (0 before it is hooked)
    let next =
        (PRE_NEXT.load() != 0).then(|| unsafe { std::mem::transmute::<usize, ReplicateSingleFn>(PRE_NEXT.load()) });
    pre_channel_with(
        next,
        graph,
        actor,
        conn_actor_info,
        global_info,
        actor_info_map,
        conn_manager,
        frame,
    )
}

/// The caller's executable RVA: the first of 4 frames outside this DLL (R3's trim), 0 when none is the game's.
fn caller_rva() -> usize {
    first_foreign_frame(4).and_then(image_rva).map_or(0, |rva| rva as usize)
}

/// RG-5 with its `next`.
pub fn bind_with(next: Option<SetChannelActorFn>, caller: usize, channel: Ptr, actor: Ptr, flags: u64) {
    let call_next = || {
        if let Some(next) = next {
            // SAFETY: the original arguments to the original function
            unsafe { next(channel, actor, flags) }
        }
    };
    if detaching() {
        return call_next();
    }
    let verdict = decide("RG-5", &BIND_PANICKED, || {
        with_state(|state| {
            core::bind(
                &ProcessGame,
                state,
                &BUDGETS,
                channel as usize,
                actor as usize,
                flags,
                || caller,
            )
        })
    });
    if verdict == Verdict::Next {
        call_next();
    }
}

unsafe extern "C" fn bind_detour(channel: Ptr, actor: Ptr, flags: u64) {
    // SAFETY: as for RG-4
    let next =
        (BIND_NEXT.load() != 0).then(|| unsafe { std::mem::transmute::<usize, SetChannelActorFn>(BIND_NEXT.load()) });
    // Step 1, here so the four frames reach the caller
    let caller = if detaching() { 0 } else { caller_rva() };
    bind_with(next, caller, channel, actor, flags);
}

/// RG-6 with its `next`.
pub fn send_with(next: Option<ReplicateActorFn>, channel: Ptr) -> i64 {
    let call_next = || match next {
        // SAFETY: the original argument to the original function
        Some(next) => unsafe { next(channel) },
        None => 0,
    };
    if detaching() {
        return call_next();
    }
    let verdict = decide("RG-6", &SEND_PANICKED, || {
        with_state(|state| core::send(&ProcessGame, state, &BUDGETS, channel as usize))
    });
    match verdict {
        // The full register: the graph adds it to the connection's bandwidth counters
        Verdict::Absorb => 0,
        Verdict::Next => call_next(),
    }
}

/// RG-6's detour (the tests call it directly).
pub unsafe extern "C" fn send_detour(channel: Ptr) -> i64 {
    // SAFETY: as for RG-4
    let next =
        (SEND_NEXT.load() != 0).then(|| unsafe { std::mem::transmute::<usize, ReplicateActorFn>(SEND_NEXT.load()) });
    send_with(next, channel)
}

/// RG-7's arguments, in order.
#[allow(missing_docs, reason = "the engine function's parameters, in order")]
#[derive(Clone, Copy)]
pub struct RemoteCall {
    pub driver: Ptr,
    pub channel: Ptr,
    pub class_cache: Ptr,
    pub field_cache: Ptr,
    pub target_obj: Ptr,
    pub connection: Ptr,
    pub function: Ptr,
    pub parms: Ptr,
    pub out_parms: Ptr,
    pub stack: Ptr,
    pub is_server: u8,
    pub send_policy: i32,
    pub remote_function_flags: Ptr,
}

/// RG-7 with its `next`.
pub fn rpc_with(next: Option<RemoteFunctionFn>, call: RemoteCall) {
    let call_next = || {
        if let Some(next) = next {
            // SAFETY: the original arguments to the original function
            unsafe {
                next(
                    call.driver,
                    call.channel,
                    call.class_cache,
                    call.field_cache,
                    call.target_obj,
                    call.connection,
                    call.function,
                    call.parms,
                    call.out_parms,
                    call.stack,
                    call.is_server,
                    call.send_policy,
                    call.remote_function_flags,
                )
            }
        }
    };
    if detaching() {
        return call_next();
    }
    let verdict = decide("RG-7", &RPC_PANICKED, || {
        with_state(|state| {
            core::rpc(
                &ProcessGame,
                state,
                &BUDGETS,
                Rpc {
                    channel: call.channel as usize,
                    target_obj: call.target_obj as usize,
                    connection: call.connection as usize,
                    function: call.function as usize,
                    is_server: call.is_server,
                },
            )
        })
    });
    if verdict == Verdict::Next {
        call_next();
    }
}

#[allow(clippy::too_many_arguments, reason = "the engine function's own arguments")]
unsafe extern "C" fn rpc_detour(
    driver: Ptr,
    channel: Ptr,
    class_cache: Ptr,
    field_cache: Ptr,
    target_obj: Ptr,
    connection: Ptr,
    function: Ptr,
    parms: Ptr,
    out_parms: Ptr,
    stack: Ptr,
    is_server: u8,
    send_policy: i32,
    remote_function_flags: Ptr,
) {
    // SAFETY: as for RG-4
    let next =
        (RPC_NEXT.load() != 0).then(|| unsafe { std::mem::transmute::<usize, RemoteFunctionFn>(RPC_NEXT.load()) });
    rpc_with(
        next,
        RemoteCall {
            driver,
            channel,
            class_cache,
            field_cache,
            target_obj,
            connection,
            function,
            parms,
            out_parms,
            stack,
            is_server,
            send_policy,
            remote_function_flags,
        },
    );
}

/// Installs RG-4.
pub fn install_pre_channel(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook(
        "RG-4",
        base,
        pre_channel_detour as *const () as usize,
        &PRE_NEXT,
        hooks_off,
    )
}

/// Installs RG-5.
pub fn install_bind(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook("RG-5", base, bind_detour as *const () as usize, &BIND_NEXT, hooks_off)
}

/// Installs RG-6.
pub fn install_send(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook("RG-6", base, send_detour as *const () as usize, &SEND_NEXT, hooks_off)
}

/// Installs RG-7.
pub fn install_rpc(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook("RG-7", base, rpc_detour as *const () as usize, &RPC_NEXT, hooks_off)
}
