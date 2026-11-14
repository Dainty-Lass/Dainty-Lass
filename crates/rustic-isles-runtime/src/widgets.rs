//! The widget crash guards (R6, spec-runtime-r6): installed from Init after the dispatch hooks and R5. The detour
//! bodies are `ffi::widgets`; this verifies, hooks, and defers the install lines (13.1).

use crate::ffi::widgets::{self as guards, GameCode};
use crate::ffi::{flags, log, minhook};
use crate::headless;
use rustic_isles_runtime_core::log::install_line;
use rustic_isles_runtime_core::offsets::target;
use rustic_isles_runtime_core::widgets::{
    CALLOUT_SINGLES, INPUT_GROUP, flags as names, group_aborted, off, reason, status_line, unhook_failed, verify,
};

/// One id's install result: `Ok` or its reason.
pub type Outcome = (&'static str, Result<(), String>);

/// Init: R6's hooks for this role (9.3's order), every line deferred. `minhook_status` is Init's `MH_Initialize`.
pub fn install(base: usize, server: bool, minhook_status: i32) {
    guards::set_image(base);
    let hooks_ok = minhook_status == minhook::MH_OK;
    let status = minhook::status_name(minhook_status);
    let address = |id: &str| {
        target(id)
            .and_then(|line| usize::try_from(line.rva).ok())
            .and_then(|rva| base.checked_add(rva))
    };
    let checked = |id: &'static str| -> Result<usize, String> {
        let line = target(id).ok_or_else(|| reason::NOT_ENTRY.to_owned())?;
        verify(&GameCode, line)?;
        if !hooks_ok {
            return Err(status.to_owned());
        }
        address(id).ok_or_else(|| reason::NOT_ENTRY.to_owned())
    };
    let single = |id: &'static str| -> Outcome {
        let result = checked(id).and_then(|at| guards::hook(id, at).map_err(|e| minhook::status_name(e).to_owned()));
        (id, result)
    };

    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut lines = Vec::new();
    if server {
        lines.push(status_line(headless::boot_patch_applied()));
        outcomes.extend(["UI-1", "UI-2", "UI-3"].map(single));
        if flags::flag(names::DISABLE_CALLOUT_GUARDS) {
            let off = off(names::DISABLE_CALLOUT_GUARDS);
            outcomes.extend(
                INPUT_GROUP
                    .iter()
                    .chain(&CALLOUT_SINGLES)
                    .map(|&id| (id, Err(off.clone()))),
            );
        } else {
            let verified = INPUT_GROUP.map(|id| (id, checked(id)));
            outcomes.extend(install_group(verified, guards::hook, guards::unhook));
            outcomes.extend(CALLOUT_SINGLES.map(single));
        }
        if flags::flag(names::DISABLE_INVENTORY_REFRESH_GUARD) {
            outcomes.push(("UI-12", Err(off(names::DISABLE_INVENTORY_REFRESH_GUARD))));
        } else {
            outcomes.push(single("UI-12"));
        }
    } else if flags::flag(names::DISABLE_HUD_LEGENDARY_GUARD) {
        outcomes.push(("UI-13", Err(off(names::DISABLE_HUD_LEGENDARY_GUARD))));
    } else {
        outcomes.push(single("UI-13"));
    }
    lines.extend(
        outcomes
            .iter()
            .map(|(id, result)| install_line(id, result.as_ref().map(|_| ()).map_err(String::as_str))),
    );
    for line in lines {
        log::defer(line);
    }
}

/// The input group, all or nothing (7.1): `verified` holds each id's address or its verify reason. A hook that fails
/// unhooks the ones already added, so the broker's empty chain runs the game's function, the effect of no guard.
pub fn install_group(
    verified: [(&'static str, Result<usize, String>); 3],
    hook: impl Fn(&str, usize) -> Result<(), i32>,
    unhook: impl Fn(&str, usize) -> Result<(), i32>,
) -> Vec<Outcome> {
    if let Some((failed, why)) = verified
        .iter()
        .find_map(|(id, result)| result.as_ref().err().map(|e| (*id, e)))
    {
        return verified
            .iter()
            .map(|(id, _)| (*id, Err(group_aborted(failed, why))))
            .collect();
    }
    let addresses: Vec<(&'static str, usize)> = verified
        .iter()
        .filter_map(|(id, result)| result.as_ref().ok().map(|&at| (*id, at)))
        .collect();
    for (position, &(id, at)) in addresses.iter().enumerate() {
        let Err(code) = hook(id, at) else {
            continue;
        };
        let status = minhook::status_name(code);
        let mut outcomes: Vec<Outcome> = Vec::new();
        for &(done, done_at) in &addresses[..position] {
            // A half-installed group moves the crash, so a failed rollback must be visible
            let result = match unhook(done, done_at) {
                Ok(()) => Err(group_aborted(id, status)),
                Err(unhooked) => Err(unhook_failed(minhook::status_name(unhooked))),
            };
            outcomes.push((done, result));
        }
        outcomes.push((id, Err(status.to_owned())));
        outcomes.extend(
            addresses[position + 1..]
                .iter()
                .map(|&(rest, _)| (rest, Err(group_aborted(id, status)))),
        );
        return outcomes;
    }
    addresses.into_iter().map(|(id, _)| (id, Ok(()))).collect()
}
