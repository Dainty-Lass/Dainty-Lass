//! EXIT-14 (R3 7): `ReportCrash`'s 60 000 ms wait for the crash reporter becomes 5 000 ms on servers, so a stalled
//! reporter freezes every client for 5 s rather than a minute. One verified code patch at Init, nothing at run time.

use super::super::memory::{function_entry, patch_bytes};
use super::super::{flags, log};
use rustic_isles_runtime_core::exit::{
    CrashWait, DISABLE_CRASH_WAIT_SHORTEN, crash_wait_install, crash_wait_line, crash_wait_skip,
};
use rustic_isles_runtime_core::offsets::exit::{REPORT_CRASH_WAIT, REPORT_CRASH_WAIT_BYTES, REPORT_CRASH_WAIT_PATCHED};

/// The flag, the `.pdata` check, then the patch; the line is deferred past the status (Init holds the loader lock).
pub(super) fn install() -> Result<(), &'static str> {
    let outcome = crash_wait_skip(
        flags::flag(DISABLE_CRASH_WAIT_SHORTEN),
        function_entry(REPORT_CRASH_WAIT),
    )
    .unwrap_or_else(|| {
        CrashWait::Patched(patch_bytes(
            REPORT_CRASH_WAIT,
            &REPORT_CRASH_WAIT_BYTES,
            &REPORT_CRASH_WAIT_PATCHED,
        ))
    });
    log::defer(crash_wait_line(outcome));
    crash_wait_install(outcome)
}
