//! Headless server boot (R5, spec-runtime-r5): installed from Init on a server, before the handler lists freeze.
//! The unsafe bodies are `ffi::headless`; this decides what runs when and what is logged.

use crate::config;
use crate::ffi::headless::{self as hl, Swap};
use crate::ffi::{engine, flags, log, memory, minhook, win};
use crate::handlers::{self, PeCall, PeHandler, Point, TickHandler, Verdict};
use rustic_isles_runtime_core::config::auth::AuthMode;
use rustic_isles_runtime_core::headless::{
    self as core, BOOT_EXPECTED, BOOT_REPLACEMENT, BOOT_SITE, RHI_EXPECTED, RHI_REPLACEMENT, RHI_SITE, SOUND_LOGGED,
    SOUND_MATCHER, SOUND_ORDER, TICK_ORDER, VIEWPORT_RETRY_MS, flags as names, off, patch_reason, reason,
};
use rustic_isles_runtime_core::log::install_line;
use rustic_isles_runtime_core::offsets::{FoundBy, target};
use rustic_isles_runtime_core::patch::PatchResult;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

static BOOT_APPLIED: AtomicBool = AtomicBool::new(false);
static LOCALE_RETRY: AtomicBool = AtomicBool::new(false);
static VIEWPORT_DONE: AtomicBool = AtomicBool::new(false);
static VIEWPORT_TRIED: AtomicBool = AtomicBool::new(false);
static VIEWPORT_LAST: AtomicU64 = AtomicU64::new(0);
static ABSORBED: AtomicU32 = AtomicU32::new(0);

/// HL-2 took (`Applied` or `AlreadyApplied`): R20's post-login match start reads it.
pub fn boot_patch_applied() -> bool {
    BOOT_APPLIED.load(Ordering::SeqCst)
}

/// HL-2 at `address`, recording whether it took.
pub(crate) fn apply_boot_patch_at(address: usize) -> PatchResult {
    let result = memory::patch_bytes_at(address, &BOOT_EXPECTED, &BOOT_REPLACEMENT);
    BOOT_APPLIED.store(
        matches!(result, PatchResult::Applied | PatchResult::AlreadyApplied),
        Ordering::SeqCst,
    );
    result
}

fn reason_line(id: &str, result: Result<(), &str>) -> String {
    install_line(id, result)
}

/// Init, server only, before the handler lists freeze: the SP-2 and SP-3 handlers.
pub fn register() {
    for message in register_handlers() {
        log::defer(message);
    }
}

/// Init, server only, right after CL-1 (R0 7.6: HL-4 first among R5's hooks; otherwise RT-17's order HL-12, HL-1,
/// HL-3, HL-6..9, HL-2). Every line is deferred: Init runs under the loader lock.
pub fn install(minhook_status: i32) {
    let base = win::image_base();
    let hooks_ok = minhook_status == minhook::MH_OK;
    let status = minhook::status_name(minhook_status);
    let mut lines = Vec::new();

    // HL-12
    if flags::flag(names::DISABLE_SERVER_LOCALE_FIX) {
        lines.push(reason_line("HL-12", Err(&off(names::DISABLE_SERVER_LOCALE_FIX))));
    } else {
        let (locale, result) = hl::fix_locale();
        lines.push(reason_line("HL-12", result));
        lines.push(core::locale_line(&locale, false));
        // Never LoadLibrary under the loader lock: one retry at the first tick (call 3)
        LOCALE_RETRY.store(!locale.game_crt, Ordering::SeqCst);
    }

    // HL-1
    let flags_found = hl::resolve_server_flags(base);
    hl::force_server_flags();
    lines.push(reason_line(
        "HL-1",
        if flags_found { Ok(()) } else { Err("unreadable") },
    ));

    // HL-4
    let web_browser = install_web_browser(base, hooks_ok, status);
    lines.push(reason_line(
        "HL-4",
        web_browser.as_ref().map(|_| ()).map_err(String::as_str),
    ));

    // HL-3
    if flags::flag(names::SHOW_SERVER_WINDOW) {
        lines.push(reason_line("HL-3", Err(&off(names::SHOW_SERVER_WINDOW))));
    } else if !hooks_ok {
        lines.push(reason_line("HL-3", Err(status)));
    } else {
        let failed = hl::install_windows().into_iter().find_map(Result::err);
        lines.push(reason_line("HL-3", failed.as_deref().map_or(Ok(()), Err)));
    }

    // HL-5
    lines.push(reason_line("HL-5", Err(reason::DROPPED)));

    // HL-9, HL-6, HL-7, HL-8
    if flags::flag(names::MP_FORCE_WARP) {
        lines.push(reason_line(
            "HL-9",
            patch_reason(memory::patch_bytes(RHI_SITE, &RHI_EXPECTED, &RHI_REPLACEMENT)),
        ));
        let load = if hooks_ok {
            hl::install_load_library()
        } else {
            Err(status.to_owned())
        };
        lines.push(reason_line("HL-6", load.as_ref().map(|_| ()).map_err(String::as_str)));
        if !hooks_ok {
            lines.push(reason_line("HL-7", Err(status)));
            lines.push(reason_line("HL-8", Err(status)));
        } else if let Some(d3d11) = hl::module("d3d11.dll") {
            hl::hook_d3d11(d3d11);
            lines.extend(hl::d3d_lines());
        } else {
            lines.push(reason_line("HL-7", Err(reason::ON_D3D11_LOAD)));
            lines.push(reason_line("HL-8", Err(reason::ON_D3D11_LOAD)));
        }
    } else {
        for id in ["HL-9", "HL-6", "HL-7", "HL-8"] {
            lines.push(reason_line(id, Err(reason::NO_WARP)));
        }
    }

    // HL-2: in Init, never later; the AGPL reads these bytes once in its own Init
    if config::auth_mode() == AuthMode::Invalid {
        lines.push(reason_line("HL-2", Err(reason::INVALID_AUTH)));
    } else {
        let result = usize::try_from(BOOT_SITE)
            .ok()
            .and_then(|rva| base.checked_add(rva))
            .map_or(PatchResult::Unreadable, apply_boot_patch_at);
        lines.push(reason_line("HL-2", patch_reason(result)));
        if !boot_patch_applied() {
            lines.push(core::BOOT_FAILED_LINE.to_owned());
        }
    }

    // HL-10, HL-11
    lines.push(reason_line("HL-10", Err(reason::DROPPED)));
    lines.push(reason_line("HL-11", Err(reason::VIEWPORT_PENDING)));

    for line in lines {
        log::defer(line);
    }
}

fn install_web_browser(base: usize, hooks_ok: bool, status: &str) -> Result<(), String> {
    if flags::flag(names::KEEP_WEB_BROWSER) {
        return Err(off(names::KEEP_WEB_BROWSER));
    }
    let Some(line) = target("HL-4") else {
        return Err(reason::NOT_ENTRY.to_owned());
    };
    let FoundBy::RvaAndBytes(prologue) = line.found_by else {
        return Err(reason::NOT_ENTRY.to_owned());
    };
    // Fail closed: CEF starts as before
    if !memory::verify_entry(line.rva, prologue) {
        return Err(reason::NOT_ENTRY.to_owned());
    }
    if !hooks_ok {
        return Err(status.to_owned());
    }
    let address = usize::try_from(line.rva)
        .ok()
        .and_then(|rva| base.checked_add(rva))
        .ok_or_else(|| reason::NOT_ENTRY.to_owned())?;
    hl::install_web_browser(address).map_err(|status| minhook::status_name(status).to_owned())
}

/// The SP-2 and SP-3 handlers; a refused registration is logged.
fn register_handlers() -> Vec<String> {
    let results = [
        handlers::register_tick(TickHandler {
            slice: "R5",
            order: TICK_ORDER,
            before: Some(flags_tick),
            after: Some(flags_tick),
        }),
        handlers::register_tick(TickHandler {
            slice: "R5",
            order: TICK_ORDER,
            before: Some(tick_before),
            after: None,
        }),
        handlers::register_pe(
            Point::Sp2,
            PeHandler {
                slice: "R5",
                order: SOUND_ORDER,
                matcher: SOUND_MATCHER,
                before: Some(sound_before),
                after: None,
            },
        ),
    ];
    results
        .into_iter()
        .filter_map(Result::err)
        .map(|error| format!("[Headless] handler not registered: {error}"))
        .collect()
}

/// HL-1 around the engine tick: never fails, never logs.
fn flags_tick(_: usize, _: f32) {
    hl::force_server_flags();
}

fn tick_before(_: usize, _: f32) {
    hl::start_hider_once();
    if LOCALE_RETRY.swap(false, Ordering::SeqCst) {
        let (locale, _) = hl::fix_locale();
        log::line(&core::locale_line(&locale, true));
    }
    for line in hl::d3d_lines() {
        log::line(&line);
    }
    viewport_retry();
}

/// HL-11, once a second until done.
fn viewport_retry() {
    if VIEWPORT_DONE.load(Ordering::Relaxed) {
        return;
    }
    let now = win::tick();
    let last = VIEWPORT_LAST.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < VIEWPORT_RETRY_MS {
        return;
    }
    VIEWPORT_LAST.store(now.max(1), Ordering::Relaxed);
    if !VIEWPORT_TRIED.swap(true, Ordering::SeqCst) && flags::flag(names::DISABLE_SERVER_VIEWPORT_SIZE) {
        VIEWPORT_DONE.store(true, Ordering::SeqCst);
        log::line(&install_line("HL-11", Err(&off(names::DISABLE_SERVER_VIEWPORT_SIZE))));
        return;
    }
    match hl::swap_viewport() {
        Swap::NotFound | Swap::Unwritable => {}
        Swap::AlreadyMit => VIEWPORT_DONE.store(true, Ordering::SeqCst),
        Swap::Swapped(rva) => {
            VIEWPORT_DONE.store(true, Ordering::SeqCst);
            log::line(&core::viewport_swapped_line(rva));
        }
        Swap::Refused(rva) => {
            VIEWPORT_DONE.store(true, Ordering::SeqCst);
            log::line(&core::viewport_refused_line(rva));
        }
    }
}

/// `SP-2@R5`: absorbs the game's sound posting unless `ENABLE_SERVER_AUDIO.flag`.
fn sound_before(call: &PeCall) -> Verdict {
    static AUDIO_ON: OnceLock<bool> = OnceLock::new();
    let audio_on = *AUDIO_ON.get_or_init(|| {
        let on = flags::flag(names::ENABLE_SERVER_AUDIO);
        log::line(if on {
            core::SOUND_LEFT_ON_LINE
        } else {
            core::SOUND_SUPPRESSED_LINE
        });
        on
    });
    if audio_on {
        return Verdict::Continue;
    }
    if let Ok(count) = ABSORBED.try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
        (count < SOUND_LOGGED).then_some(count + 1)
    }) {
        log::line(&core::absorbed_line(
            count + 1,
            call.entry.full_name().as_deref(),
            engine::full_name(call.object).as_deref(),
        ));
    }
    Verdict::Absorb
}
