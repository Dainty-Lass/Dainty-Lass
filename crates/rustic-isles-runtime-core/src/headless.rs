//! Headless server boot (R5, spec-runtime-r5): the patch facts, the viewport rule, the d3d11 path test, the sound
//! matcher and the log text. The DLL's `headless` modules install it.

use crate::names::{Kind, Matcher};
use crate::patch::PatchResult;

/// `GIsServer`, one byte of the executable's data (HL-1).
pub const GIS_SERVER: u32 = 0x06E1_EADA;
/// `GIsClient`, one byte (HL-1).
pub const GIS_CLIENT: u32 = 0x06E1_EAD9;

/// HL-2: the `jz` over `UGameEngine::Init`'s viewport and first-local-player block.
pub const BOOT_SITE: u32 = 0x03E0_7353;
/// HL-2's bytes before.
pub const BOOT_EXPECTED: [u8; 6] = [0x0F, 0x84, 0xD3, 0x00, 0x00, 0x00];
/// HL-2's bytes after: `jmp` to the same target, then `nop`.
pub const BOOT_REPLACEMENT: [u8; 6] = [0xE9, 0xD4, 0x00, 0x00, 0x00, 0x90];

/// HL-9: the RHI selector's `test al,al; jne` after `IsSupported`.
pub const RHI_SITE: u32 = 0x034A_B73D;
/// HL-9's bytes before.
pub const RHI_EXPECTED: [u8; 3] = [0x84, 0xC0, 0x75];
/// HL-9's bytes after (`jmp`).
pub const RHI_REPLACEMENT: [u8; 3] = [0x84, 0xC0, 0xEB];

/// The engine's cached "-nullrhi was on the line" byte. Never written: writing 0 gave every hub a D3D device (5.3).
pub const NULLRHI_CACHE: u32 = 0x06CD_7878;
/// Its guard (`i32`). Never written either.
pub const NULLRHI_GUARD: u32 = 0x06CD_787C;

/// `UFunction::Func`, UE 4.26's layout; HL-11's self-check fails closed if it is wrong.
pub const UFUNCTION_FUNC: usize = 0xD8;
/// HL-11's UFunction, matched exactly.
pub const VIEWPORT_FUNCTION: &str = "Function UMG.WidgetLayoutLibrary.GetViewportSize";
/// The native pointer HL-11 expects on 1.14.7 (HL-10's target).
pub const VIEWPORT_NATIVE_RVA: u32 = 0x0398_BB50;
/// How often HL-11 retries from the engine tick.
pub const VIEWPORT_RETRY_MS: u64 = 1000;
/// HL-11's answer for a headless (0,0) viewport.
pub const VIEWPORT_SIZE: (f32, f32) = (1920.0, 1080.0);
/// Substitutions logged.
pub const VIEWPORT_LOGGED: u32 = 3;
/// Absorbed sound posts logged.
pub const SOUND_LOGGED: u32 = 5;

/// `D3D_DRIVER_TYPE_WARP`.
pub const DRIVER_TYPE_WARP: i32 = 5;
/// `SW_HIDE`.
pub const SW_HIDE: i32 = 0;
/// `LC_ALL`.
pub const LC_ALL: i32 = 0;
/// `_ENABLE_PER_THREAD_LOCALE`.
pub const ENABLE_PER_THREAD_LOCALE: i32 = 1;
/// `_DISABLE_PER_THREAD_LOCALE`.
pub const DISABLE_PER_THREAD_LOCALE: i32 = 2;

/// The hider thread: fast rounds while the engine boots, then the slow period.
pub const HIDER_FAST_ROUNDS: u32 = 240;
/// Milliseconds between fast rounds.
pub const HIDER_FAST_MS: u32 = 250;
/// Milliseconds between slow rounds.
pub const HIDER_SLOW_MS: u32 = 3000;

/// SP-2@R5's handler order (a per-call slice that must see sound calls takes a smaller one).
pub const SOUND_ORDER: i32 = 50;
/// HL-1's tick writes and HL-11's retry run before every other SP-3 handler.
pub const TICK_ORDER: i32 = -1000;

/// `SP-2@R5` (RT-18): exactly the AGPL's test; catches `PostEvent` and `PostEventAtLocation`, not `PostAkEvent`.
pub const SOUND_MATCHER: Matcher = &[(Kind::Contains, "PostEvent")];

/// R5's flags (RT-14). `DISABLE_SERVER_AUDIO` and `VIEWPORT_FALLBACK` are dropped.
pub mod flags {
    /// HL-3 off.
    pub const SHOW_SERVER_WINDOW: &str = "SHOW_SERVER_WINDOW";
    /// HL-4 off.
    pub const KEEP_WEB_BROWSER: &str = "KEEP_WEB_BROWSER";
    /// `SP-2@R5` off.
    pub const ENABLE_SERVER_AUDIO: &str = "ENABLE_SERVER_AUDIO";
    /// HL-6 to HL-9 on.
    pub const MP_FORCE_WARP: &str = "MP_FORCE_WARP";
    /// HL-11 off.
    pub const DISABLE_SERVER_VIEWPORT_SIZE: &str = "DISABLE_SERVER_VIEWPORT_SIZE";
    /// HL-12 off (and R4's culture append, which reads it itself).
    pub const DISABLE_SERVER_LOCALE_FIX: &str = crate::login::cmdline::CULTURE_FLAG;
}

/// Install reasons (13.1) other than `ok`.
pub mod reason {
    /// HL-2, HL-9.
    pub const ALREADY_APPLIED: &str = "already applied";
    /// HL-6 to HL-9 without the flag.
    pub const NO_WARP: &str = "off (no MP_FORCE_WARP.flag)";
    /// HL-2 with an invalid auth mode.
    pub const INVALID_AUTH: &str = "skipped (invalid auth mode)";
    /// HL-5, HL-10.
    pub const DROPPED: &str = "not installed (dropped)";
    /// HL-11 at Init.
    pub const VIEWPORT_PENDING: &str = "pending (tried each second from the engine tick)";
    /// HL-7, HL-8 before `d3d11.dll` loads.
    pub const ON_D3D11_LOAD: &str = "on d3d11 load";
    /// HL-4's prologue check.
    pub const NOT_ENTRY: &str = "not a function entry or bytes differ";
    /// `GetModuleHandleW` answered null.
    pub const MODULE_NOT_LOADED: &str = "module not loaded";
    /// `GetProcAddress` answered null.
    pub const EXPORT_NOT_FOUND: &str = "export not found";
}

/// `off (<FLAG>.flag)`.
pub fn off(flag: &str) -> String {
    format!("off ({flag}.flag)")
}

/// A patch's install result: `Ok` for `Applied`, else its reason.
pub fn patch_reason(result: PatchResult) -> Result<(), &'static str> {
    match result {
        PatchResult::Applied => Ok(()),
        PatchResult::AlreadyApplied => Err(reason::ALREADY_APPLIED),
        PatchResult::Mismatch => Err("bytes differ"),
        PatchResult::Unreadable => Err("unreadable"),
        PatchResult::ProtectFailed => Err("protect failed"),
    }
}

/// Where a `jz rel32` (6 bytes) or `jmp rel32` (5 bytes) at `site` lands.
pub fn branch_target(site: u64, bytes: &[u8]) -> Option<u64> {
    let (len, disp) = match bytes {
        [0x0F, 0x84, d0, d1, d2, d3, ..] => (6, [*d0, *d1, *d2, *d3]),
        [0xE9, d0, d1, d2, d3, ..] => (5, [*d0, *d1, *d2, *d3]),
        _ => return None,
    };
    site.checked_add(len)?
        .checked_add_signed(i64::from(i32::from_le_bytes(disp)))
}

/// HL-11's rule: a size of at most 1 (or NaN) on either axis becomes 1920×1080; a real size is never touched.
#[allow(clippy::neg_cmp_op_on_partial_ord, reason = "NaN must count as no size")]
pub fn fix_viewport(x: f32, y: f32) -> Option<(f32, f32)> {
    (!(x > 1.0) || !(y > 1.0)).then_some(VIEWPORT_SIZE)
}

/// HL-6's test: the loaded path holds `d3d11`, ASCII letters compared case-insensitively (`d3d11on12.dll` too).
pub fn is_d3d11_path(path: Option<&[u16]>) -> bool {
    const NEEDLE: &[u8] = b"d3d11";
    let Some(path) = path else {
        return false;
    };
    path.windows(NEEDLE.len()).any(|window| {
        window
            .iter()
            .zip(NEEDLE)
            .all(|(&unit, &want)| u8::try_from(unit).is_ok_and(|byte| byte.to_ascii_lowercase() == want))
    })
}

/// HL-12's measurement, for its log line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Locale {
    /// `GetUserDefaultLocaleName`, `?` on failure.
    pub user_locale: String,
    /// `ucrtbase.dll` was loaded.
    pub game_crt: bool,
    /// The decimal point before, `?` when unknown.
    pub before: String,
    /// The decimal point after.
    pub after: String,
    /// `setlocale`'s answer, `None` when it did not run or answered null.
    pub numeric: Option<String>,
}

/// `[Headless] locale ...`, with ` (late)` when it ran at the first tick.
pub fn locale_line(locale: &Locale, late: bool) -> String {
    format!(
        "[Headless] locale windows_user_locale={} game_crt={} decimal_before='{}' decimal_after='{}' numeric={}{}",
        locale.user_locale,
        if locale.game_crt { "ucrtbase" } else { "not-found" },
        locale.before,
        locale.after,
        locale.numeric.as_deref().unwrap_or("?"),
        if late { " (late)" } else { "" }
    )
}

/// HL-2 failed.
pub const BOOT_FAILED_LINE: &str = "[Headless] HL-2 NOT applied: this server has a phantom local player and nothing suppresses it; check the game build";
/// The hider's first hide.
pub const HID_WINDOW_LINE: &str = "[Headless] hid the server window";
/// HL-4's detour, once.
pub const CEF_SKIPPED_LINE: &str = "[Headless] skipped WebBrowserWidget startup (no Chromium/CEF)";
/// `SP-2@R5`'s first call, suppressing.
pub const SOUND_SUPPRESSED_LINE: &str = "[Headless] sound posting suppressed (server)";
/// `SP-2@R5`'s first call, with the flag.
pub const SOUND_LEFT_ON_LINE: &str = "[Headless] sound posting left on (ENABLE_SERVER_AUDIO.flag)";

/// The hider thread could not start.
pub fn hider_failed_line(error: u32) -> String {
    format!("[Headless] hider thread failed (error {error})")
}

/// One absorbed sound post (the first 5).
pub fn absorbed_line(count: u32, function: Option<&str>, object: Option<&str>) -> String {
    format!(
        "[Headless] absorbed sound post #{count} fn={} obj={}",
        function.unwrap_or("null"),
        object.unwrap_or("null")
    )
}

/// HL-11 swapped the native pointer.
pub fn viewport_swapped_line(rva: u32) -> String {
    format!("[Headless] GetViewportSize answers 1920x1080 instead of 0 on this server (native +0x{rva:08X})")
}

/// HL-11 refused a pointer that is no function entry; `None` outside the executable.
pub fn viewport_refused_line(rva: Option<u32>) -> String {
    let rva = rva.map_or_else(|| "?".to_owned(), |rva| format!("{rva:08X}"));
    format!(
        "[Headless] HL-11 NOT installed: GetViewportSize's native pointer +0x{rva} is not a function entry (wrong \
         UFunction layout?)"
    )
}

/// One substitution (the first 3).
pub fn viewport_substituted_line(x: f32, y: f32) -> String {
    format!("[Headless] GetViewportSize answered {x}x{y} on the server; returned 1920x1080")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::matches;
    use crate::patch::decide;

    #[test]
    fn viewport_rule() {
        let fixed = Some((1920.0, 1080.0));
        assert_eq!(fix_viewport(0.0, 0.0), fixed);
        assert_eq!(fix_viewport(1.0, 1.0), fixed);
        assert_eq!(fix_viewport(1.0001, 1080.0), None);
        assert_eq!(fix_viewport(1920.0, 0.0), fixed);
        assert_eq!(fix_viewport(-5.0, 600.0), fixed);
        assert_eq!(fix_viewport(f32::NAN, 1080.0), fixed);
        assert_eq!(fix_viewport(1080.0, f32::NAN), fixed);
        assert_eq!(fix_viewport(f32::INFINITY, f32::INFINITY), None);
        assert_eq!(fix_viewport(1920.0, 1080.0), None);
    }

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn d3d11_path() {
        for path in ["d3d11.dll", r"C:\Windows\System32\D3D11.DLL", "d3d11on12.dll"] {
            assert!(is_d3d11_path(Some(&units(path))), "{path}");
        }
        for path in ["d3d10.dll", "d3d1", ""] {
            assert!(!is_d3d11_path(Some(&units(path))), "{path}");
        }
        assert!(!is_d3d11_path(None));
        assert!(!is_d3d11_path(Some(&units(r"C:\Spiele\Ünïcödé\d3d1ä.dll"))));
        assert!(is_d3d11_path(Some(&units(r"C:\Spiele\Ünïcödé\D3d11.dll"))));
        assert!(!is_d3d11_path(Some(&[0xD800, 0xDFFF, 0xFFFF])));
    }

    #[test]
    fn sound_matcher() {
        assert!(matches(
            SOUND_MATCHER,
            Some("Function AkAudio.AkGameplayStatics.PostEvent")
        ));
        assert!(matches(
            SOUND_MATCHER,
            Some("Function AkAudio.AkGameplayStatics.PostEventAtLocation")
        ));
        assert!(!matches(
            SOUND_MATCHER,
            Some("Function AkAudio.AkComponent.PostAkEvent")
        ));
        assert!(!matches(SOUND_MATCHER, None));
    }

    #[test]
    fn boot_bytes() {
        let site = u64::from(BOOT_SITE);
        assert_eq!(site + 6 + 0xD3, site + 5 + 0xD4);
        assert_eq!(branch_target(site, &BOOT_EXPECTED), Some(0x03E0_742C));
        assert_eq!(branch_target(site, &BOOT_REPLACEMENT), Some(0x03E0_742C));
        assert_eq!(BOOT_REPLACEMENT.last(), Some(&0x90));
        assert_eq!(branch_target(site, &[0x90]), None);
        for (expected, replacement) in [
            (&BOOT_EXPECTED[..], &BOOT_REPLACEMENT[..]),
            (&RHI_EXPECTED[..], &RHI_REPLACEMENT[..]),
        ] {
            assert_eq!(decide(expected, expected, replacement), PatchResult::Applied);
            assert_eq!(decide(replacement, expected, replacement), PatchResult::AlreadyApplied);
            let other = vec![0xCC; expected.len()];
            assert_eq!(decide(&other, expected, replacement), PatchResult::Mismatch);
        }
    }

    #[test]
    fn reasons_and_lines() {
        assert_eq!(patch_reason(PatchResult::Applied), Ok(()));
        assert_eq!(patch_reason(PatchResult::AlreadyApplied), Err("already applied"));
        assert_eq!(patch_reason(PatchResult::Mismatch), Err("bytes differ"));
        assert_eq!(patch_reason(PatchResult::Unreadable), Err("unreadable"));
        assert_eq!(patch_reason(PatchResult::ProtectFailed), Err("protect failed"));
        assert_eq!(off(flags::SHOW_SERVER_WINDOW), "off (SHOW_SERVER_WINDOW.flag)");
        assert_eq!(reason::NO_WARP, "off (no MP_FORCE_WARP.flag)");
        assert_eq!(reason::DROPPED, "not installed (dropped)");
        assert_eq!(
            reason::VIEWPORT_PENDING,
            "pending (tried each second from the engine tick)"
        );
        let locale = Locale {
            user_locale: "de-DE".into(),
            game_crt: true,
            before: ",".into(),
            after: ".".into(),
            numeric: Some("C".into()),
        };
        assert_eq!(
            locale_line(&locale, false),
            "[Headless] locale windows_user_locale=de-DE game_crt=ucrtbase decimal_before=',' decimal_after='.' \
             numeric=C"
        );
        let missing = Locale {
            user_locale: "?".into(),
            game_crt: false,
            before: "?".into(),
            after: "?".into(),
            numeric: None,
        };
        assert_eq!(
            locale_line(&missing, true),
            "[Headless] locale windows_user_locale=? game_crt=not-found decimal_before='?' decimal_after='?' \
             numeric=? (late)"
        );
        assert_eq!(
            BOOT_FAILED_LINE,
            "[Headless] HL-2 NOT applied: this server has a phantom local player and nothing suppresses it; check \
             the game build"
        );
        assert_eq!(hider_failed_line(5), "[Headless] hider thread failed (error 5)");
        assert_eq!(
            absorbed_line(1, Some("Function AkAudio.AkGameplayStatics.PostEvent"), None),
            "[Headless] absorbed sound post #1 fn=Function AkAudio.AkGameplayStatics.PostEvent obj=null"
        );
        assert_eq!(
            viewport_swapped_line(VIEWPORT_NATIVE_RVA),
            "[Headless] GetViewportSize answers 1920x1080 instead of 0 on this server (native +0x0398BB50)"
        );
        assert_eq!(
            viewport_refused_line(Some(0x1234)),
            "[Headless] HL-11 NOT installed: GetViewportSize's native pointer +0x00001234 is not a function entry \
             (wrong UFunction layout?)"
        );
        assert!(viewport_refused_line(None).contains("pointer +0x? is not"));
        assert_eq!(
            viewport_substituted_line(0.0, 0.0),
            "[Headless] GetViewportSize answered 0x0 on the server; returned 1920x1080"
        );
        assert_eq!(
            viewport_substituted_line(f32::NAN, 1.5),
            "[Headless] GetViewportSize answered NaNx1.5 on the server; returned 1920x1080"
        );
        for line in [
            HID_WINDOW_LINE,
            CEF_SKIPPED_LINE,
            SOUND_SUPPRESSED_LINE,
            SOUND_LEFT_ON_LINE,
        ] {
            assert!(line.starts_with("[Headless] "));
        }
    }
}
