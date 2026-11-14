//! NETD-7, the spline direction query (13.1): a component's curve is checked before the evaluator reads it. The
//! crash it was written for was the AGPL's own stale 1.12 call, so it has never caught a real fault (R11 call 7).

use super::{budgeted, hook, log, panicked, store};
use crate::ffi::broker::NextCell;
use crate::ffi::exit::Install;
use crate::ffi::memory::{read, readable};
use crate::ffi::{detaching, flags};
use rustic_isles_runtime_core::net::{
    Curve, LineBudget, SPLINE_COMPONENT_CHECKED, SPLINE_KIND, budgets, flags as names, install as reason, spline_guard,
    spline_unusable, spline_usable,
};
use rustic_isles_runtime_core::offsets::net::SPLINE_CURVE;
use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;

type DirectionFn = unsafe extern "C" fn(*mut c_void, *mut f32, f32, i32) -> *mut f32;

pub(super) static NEXT: NextCell = NextCell::new();
static PANICKED: AtomicBool = AtomicBool::new(false);
static UNUSABLE: LineBudget = LineBudget::new(budgets::SPLINE);

thread_local! {
    // The last accepted (component, data, num); only the game thread calls the query
    static LAST: Cell<Option<Curve>> = const { Cell::new(None) };
}

/// What the component's curve reads as; `None` for a null or unreadable component.
fn curve_of(component: usize) -> Option<Curve> {
    if component == 0 || !readable(component, SPLINE_COMPONENT_CHECKED) {
        return None;
    }
    Some(Curve {
        component,
        data: read::<usize>(component + SPLINE_CURVE)?,
        num: read::<i32>(component + SPLINE_CURVE + 8)?,
    })
}

/// 13.1 step 2.
pub(super) fn usable(component: usize) -> bool {
    let curve = curve_of(component);
    LAST.with(|last| {
        let mut remembered = last.get();
        let usable = spline_usable(&mut remembered, curve, readable);
        last.set(remembered);
        usable
    })
}

pub(super) unsafe extern "C" fn detour(component: *mut c_void, out: *mut f32, input: f32, apply: i32) -> *mut f32 {
    let next = NEXT.load();
    let call_next = || {
        if next == 0 {
            return out;
        }
        // SAFETY: the broker wrote a function of the query's signature into this cell; the original arguments
        unsafe {
            let next = std::mem::transmute::<usize, DirectionFn>(next);
            next(component, out, input, apply)
        }
    };
    if detaching() || out.is_null() {
        return call_next();
    }
    crate::lifecycle::write_status_once();
    let checked = catch_unwind(AssertUnwindSafe(|| usable(component as usize)));
    match checked {
        Ok(true) => call_next(),
        Ok(false) => {
            for index in 0..3 {
                store::<f32>(out as usize + index * 4, 0.0);
            }
            let curve = curve_of(component as usize);
            budgeted(&UNUSABLE, SPLINE_KIND, || {
                spline_unusable(
                    component as usize,
                    curve.map_or(0, |curve| curve.data),
                    curve.map_or(0, |curve| curve.num),
                )
            });
            out
        }
        Err(_) => {
            panicked("NETD-7", &PANICKED);
            call_next()
        }
    }
}

/// Installs NETD-7 unless `DISABLE_SPLINE_GUARD.flag` is present (read once, here).
pub fn install(base: usize, hooks_off: Option<&'static str>) -> Install {
    let off = flags::flag(names::DISABLE_SPLINE_GUARD);
    log::defer(spline_guard(!off).to_owned());
    if off {
        return ("NETD-7", Err(reason::FLAG_SPLINE));
    }
    hook("NETD-7", base, detour as *const () as usize, &NEXT, hooks_off)
}
