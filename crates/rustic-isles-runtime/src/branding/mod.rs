//! Client branding, the scripted login and the F2 console (spec-runtime-r17), as SP-1 handlers. They read game
//! memory through `ffi::reflect`'s checked reads and call the game only through `ffi::reflect::call`.

mod auto_login;
mod console;

pub use console::install_exec_guard;
mod credits;
mod login;
mod press_start;

use crate::ffi::{flags, log, reflect};
use crate::handlers::{PeHandler, Point, register_pe};
use rustic_isles_runtime_core::branding::{self as core, Lang, lang_for_culture, order};
use rustic_isles_runtime_core::offsets::r17;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

/// Init, client only, before the freeze (RT-17 for R17: no order dependency).
pub fn register() {
    let handlers = [
        PeHandler {
            slice: "R17",
            order: order::AUTO_LOGIN,
            matcher: core::AUTO_LOGIN,
            before: Some(auto_login::before),
            after: None,
        },
        PeHandler {
            slice: "R17",
            order: order::LOGIN,
            matcher: core::LOGIN,
            before: None,
            after: Some(login::after),
        },
        PeHandler {
            slice: "R17",
            order: order::CREDITS,
            matcher: core::CREDITS,
            before: None,
            after: Some(credits::after),
        },
        PeHandler {
            slice: "R17",
            order: order::PRESS_START,
            matcher: core::PRESS_START,
            before: None,
            after: Some(press_start::after),
        },
        PeHandler {
            slice: "R17",
            order: order::CONSOLE,
            matcher: core::CONSOLE,
            before: None,
            after: Some(console::after),
        },
    ];
    for handler in handlers {
        if let Err(error) = register_pe(Point::Sp1, handler) {
            log::defer(format!("[MitBranding] handler not registered: {error}"));
        }
    }
}

/// A `DISABLE_*` flag, read at the handler's first match and kept (R0 4.2 read-once).
fn read_once(cell: &OnceLock<bool>, name: &str) -> bool {
    *cell.get_or_init(|| flags::flag(name))
}

/// The first `limit` uses of a log line.
struct Budget(AtomicU32);

impl Budget {
    const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    fn take(&self, limit: u32) -> bool {
        self.0.fetch_add(1, Ordering::Relaxed) < limit
    }
}

/// The texts' language, from the game's culture once (section 7).
fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| lang_for_culture(reflect::current_culture().as_deref()))
}

fn text(key: &str) -> &'static str {
    core::text(lang(), key)
}

/// `Widget.SetVisibility(Collapsed)`, skipped when the widget already reads collapsed. True when it was called.
fn collapse(widget: usize) -> bool {
    const COLLAPSED: u8 = 1;
    if reflect::u8_at(widget + r17::UWIDGET_VISIBILITY) == Some(COLLAPSED) {
        return false;
    }
    let mut params = reflect::Params::new(1);
    params.put(0, &[COLLAPSED]);
    reflect::invoke(widget, "SetVisibility", &mut params)
}
