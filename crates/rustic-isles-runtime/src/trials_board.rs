//! The Trials boards in Ramsgate, asked for again on every arrival (spec-runtime-trials-board): after the client's
//! `ClientRestart` in Ramsgate, the local controller's `PlayerArenaComponent` drops its cached boards and asks for the
//! solo and group ones, whose answers reach the boards through the delegates they bound.

use crate::ffi::memory::read;
use crate::ffi::reflect::{self, Params};
use crate::ffi::{detaching, engine, flags, log, win};
use crate::handlers::{self, PeCall, PeHandler, Point};
use rustic_isles_runtime_core::names::{Kind, Matcher};
use rustic_isles_runtime_core::offsets::{data, layout};
use rustic_isles_runtime_core::reflect as core_reflect;
use rustic_isles_runtime_core::trials_board::{self as core, Once};
use std::sync::Mutex;

const CLIENT_RESTART: Matcher = &[(Kind::Exact, "Function Engine.PlayerController.ClientRestart")];

static ONCE: Mutex<Once> = Mutex::new(Once::new());

/// Init registers the handler before the lists freeze; the client only.
pub fn register() {
    let handler = PeHandler {
        slice: "TrialsBoard",
        order: 0,
        matcher: CLIENT_RESTART,
        before: None,
        after: Some(client_restart),
    };
    if let Err(error) = handlers::register_pe(Point::Sp1, handler) {
        log::defer(format!("[MitRuntime] TrialsBoard handler not registered: {error}"));
    }
}

fn world() -> usize {
    usize::try_from(data::GWORLD)
        .ok()
        .and_then(|rva| win::image_base().checked_add(rva))
        .and_then(read::<usize>)
        .unwrap_or(0)
}

/// The arena component the controller owns: the instance whose outer is the controller.
fn arena_component(controller: usize) -> Option<usize> {
    let engine = engine::engine()?;
    let class = reflect::find_class(core::ARENA_COMPONENT)?;
    core_reflect::scan(&engine, |header| {
        header.class == class && read::<usize>(header.object.wrapping_add(layout::UOBJECT_OUTER)) == Some(controller)
    })
}

fn client_restart(call: &PeCall) {
    if detaching() || call.object == 0 || flags::flag(core::DISABLE_FLAG) {
        return;
    }
    let world = world();
    if !engine::full_name(world).is_some_and(|name| core::is_ramsgate(&name)) {
        return;
    }
    if !ONCE.lock().is_ok_and(|mut once| once.take(world)) {
        return;
    }
    let Some(component) = arena_component(call.object) else {
        log::line("[TrialsBoard] Ramsgate: the controller has no PlayerArenaComponent; boards left as they are");
        return;
    };
    let invalidated = reflect::invoke(component, core::INVALIDATE, &mut Params::new(0));
    let queried = core::BOARDS.map(|board| {
        let mut params = Params::new(1);
        params.put(0, &[board]);
        reflect::invoke(component, core::QUERY, &mut params)
    });
    log::line(&core::refresh_line(invalidated, queried));
}
