//! Frame pacing settings (R1 8, patch 0020), read by R8 at the first server tick.

use crate::config::numbers::strtol;
use crate::env::Raw;

/// The active cap's variable.
pub const MAX_FPS: &str = "DAINTY_SERVER_MAX_FPS";
/// The idle cap's variable.
pub const IDLE_FPS: &str = "DAINTY_SERVER_IDLE_FPS";
/// The raw read's cap (a 15-character buffer).
pub const READ_CAP: usize = 15;

/// `fps_setting(name, raw, default, min, max)`: the value and, when the default was forced, a note for R8's line.
pub fn fps_setting(name: &str, raw: &Raw, default: i32, min: i32, max: i32) -> (i32, Option<String>) {
    match raw {
        Raw::Unset => (default, None),
        Raw::TooLong(_) => (default, Some(format!(" {name} too long (default used)"))),
        Raw::Value(text) => match strtol(text) {
            Some(value) if value == 0 || (min..=max).contains(&value) => (value, None),
            _ => (default, Some(format!(" {name}='{text}' invalid (default used)"))),
        },
    }
}

/// The active cap: default 30, 0 or 10 to 240, 0 = no cap.
pub fn max_fps(raw: &Raw) -> (i32, Option<String>) {
    fps_setting(MAX_FPS, raw, 30, 10, 240)
}

/// The idle cap: default 0, 0 or 1 to 240, 0 = the same as the active cap.
pub fn idle_fps(raw: &Raw) -> (i32, Option<String>) {
    fps_setting(IDLE_FPS, raw, 0, 1, 240)
}

/// R8's line, notes in the order MAX, IDLE.
pub fn frame_pace_line(max: (i32, Option<String>), idle: (i32, Option<String>)) -> String {
    let active = if max.0 == 0 {
        "off".to_owned()
    } else {
        format!("{} fps", max.0)
    };
    let idle_text = if idle.0 == 0 {
        "same".to_owned()
    } else {
        format!("{} fps", idle.0)
    };
    format!(
        "[FramePace] server frame cap: {active}, idle (no client for 15 s): {idle_text} (DAINTY_SERVER_MAX_FPS / \
         DAINTY_SERVER_IDLE_FPS){}{}",
        max.1.unwrap_or_default(),
        idle.1.unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn max(text: Option<&str>) -> (i32, Option<String>) {
        max_fps(&Raw::from_text(text, READ_CAP))
    }

    fn idle(text: Option<&str>) -> (i32, Option<String>) {
        idle_fps(&Raw::from_text(text, READ_CAP))
    }

    #[test]
    fn max_cap() {
        assert_eq!(max(None), (30, None));
        assert_eq!(max(Some("0")), (0, None));
        assert_eq!(max(Some("10")), (10, None));
        assert_eq!(max(Some("240")), (240, None));
        assert_eq!(max(Some("9")).0, 30);
        assert_eq!(max(Some("241")).0, 30);
        assert_eq!(max(Some(" 30")), (30, None));
        assert_eq!(max(Some("+30")), (30, None));
        assert_eq!(max(Some("-0")), (0, None));
        assert_eq!(
            max(Some("30 ")),
            (30, Some(" DAINTY_SERVER_MAX_FPS='30 ' invalid (default used)".into()))
        );
        assert_eq!(
            max(Some("x")),
            (30, Some(" DAINTY_SERVER_MAX_FPS='x' invalid (default used)".into()))
        );
        assert_eq!(max(Some("000000000000060")), (60, None), "15 characters");
        assert_eq!(
            max(Some("0000000000000060")),
            (30, Some(" DAINTY_SERVER_MAX_FPS too long (default used)".into()))
        );
    }

    #[test]
    fn idle_cap() {
        assert_eq!(idle(None), (0, None));
        assert_eq!(idle(Some("0")), (0, None));
        assert_eq!(idle(Some("1")), (1, None));
        assert_eq!(idle(Some("240")), (240, None));
        assert_eq!(idle(Some("241")).0, 0);
        assert_eq!(idle(Some("-1")).0, 0);
    }

    #[test]
    fn line() {
        assert_eq!(
            frame_pace_line(max(None), idle(None)),
            "[FramePace] server frame cap: 30 fps, idle (no client for 15 s): same (DAINTY_SERVER_MAX_FPS / \
             DAINTY_SERVER_IDLE_FPS)"
        );
        assert_eq!(
            frame_pace_line(max(Some("x")), idle(Some("500"))),
            "[FramePace] server frame cap: 30 fps, idle (no client for 15 s): same (DAINTY_SERVER_MAX_FPS / \
             DAINTY_SERVER_IDLE_FPS) DAINTY_SERVER_MAX_FPS='x' invalid (default used) DAINTY_SERVER_IDLE_FPS='500' \
             invalid (default used)"
        );
        assert_eq!(
            frame_pace_line(max(Some("0")), idle(Some("5"))),
            "[FramePace] server frame cap: off, idle (no client for 15 s): 5 fps (DAINTY_SERVER_MAX_FPS / \
             DAINTY_SERVER_IDLE_FPS)"
        );
    }
}
