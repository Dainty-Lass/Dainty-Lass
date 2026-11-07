// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The game's console window, read into the launcher: whatever opens it (the game, the runtime or a
//! DLL beside it), a helper attaches to it, reads new rows from its screen buffer and writes them to its
//! stdout, and the launcher puts them on the Game log page. The helper is this executable run with
//! `HELPER_FLAG`, never the launcher itself: a process attached to a console is ended when that console
//! closes, which here is every time the game exits.
//!
//! Unreal prints the command line at start, and the command line carries the account key, so every line
//! is redacted (`redact`) before it is kept or sent to the page. Windows only; elsewhere nothing is read.

/// `dainty-launcher --read-game-console <pid> [--hide]`: the helper (`helper_main`).
pub const HELPER_FLAG: &str = "--read-game-console";

/// The lines kept for the page: the newest, a session's worth of a busy log.
pub const KEPT_LINES: usize = 5000;

/// The account key, and the argument that carries it, masked wherever they appear in a line.
pub fn redact(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        let lower = rest.to_ascii_lowercase();
        let key = rest.find("RRK_");
        let arg = lower.find("-auth_password=");
        let (at, skip, is_key) = match (key, arg) {
            (Some(k), Some(a)) if a < k => (a, "-AUTH_PASSWORD=".len(), false),
            (Some(k), _) => (k, "RRK_".len(), true),
            (None, Some(a)) => (a, "-AUTH_PASSWORD=".len(), false),
            (None, None) => {
                out.push_str(rest);
                break;
            }
        };
        out.push_str(&rest[..at + skip]);
        out.push_str("<hidden>");
        let tail = &rest[at + skip..];
        // A key is [A-Za-z0-9_-]; a quoted value runs to its closing quote, any other to the next space
        let end = match tail.chars().next() {
            _ if is_key => tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(tail.len()),
            Some(q @ ('"' | '\'')) => tail[1..].find(q).map_or(tail.len(), |close| close + 2),
            _ => tail
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(tail.len()),
        };
        rest = &tail[end..];
    }
    out
}

/// Which rows of a console's screen buffer are new. Rows above the cursor are finished; the cursor's
/// own row is still being written. A full row (its last cell not blank) wraps onto the next and joins
/// it. When the buffer scrolls or is cleared, the last rows read are found again by their text.
#[derive(Default)]
pub struct Tracker {
    next: usize,
    tail: Vec<String>,
    pending: String,
}

/// How many of the last rows read identify where reading stopped.
const TAIL: usize = 3;

impl Tracker {
    /// The finished lines since the last poll; `row(r)` is row r's cells, None past the buffer.
    pub fn poll(&mut self, cursor_row: usize, row: &mut dyn FnMut(usize) -> Option<String>) -> Vec<String> {
        if !self.tail.is_empty() && !self.tail_at(self.tail.len(), self.next, row) {
            // Found again by the longest run of the last rows still in the buffer
            let top = self.next.min(cursor_row);
            let found = (1..=self.tail.len())
                .rev()
                .find_map(|k| (k..=top).rev().find(|&end| self.tail_at(k, end, row)));
            self.next = found.unwrap_or(0);
        }
        let mut lines = Vec::new();
        for r in self.next..cursor_row {
            let Some(raw) = row(r) else { break };
            let full = raw.chars().last().is_some_and(|c| c != ' ');
            if full {
                self.pending.push_str(&raw);
            } else {
                self.pending.push_str(raw.trim_end());
                lines.push(std::mem::take(&mut self.pending));
            }
            self.tail.push(raw);
            if self.tail.len() > TAIL {
                self.tail.remove(0);
            }
            self.next = r + 1;
        }
        lines
    }

    /// Whether the `k` rows just above `end` are the last `k` rows read.
    fn tail_at(&self, k: usize, end: usize, row: &mut dyn FnMut(usize) -> Option<String>) -> bool {
        let Some(start) = end.checked_sub(k) else {
            return false;
        };
        self.tail[self.tail.len() - k..]
            .iter()
            .enumerate()
            .all(|(i, kept)| row(start + i).as_deref() == Some(kept.as_str()))
    }
}

pub use os::{helper_main, watch};

#[cfg(windows)]
mod os {
    #![allow(
        unsafe_code,
        reason = "consoles are a C API; each handle is opened, used and closed here"
    )]

    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::io::AsHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command, ExitCode, Stdio};
    use std::sync::Mutex;
    use std::time::Duration;

    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        AttachConsole, CONSOLE_SCREEN_BUFFER_INFO, COORD, FreeConsole, GetConsoleScreenBufferInfo, GetConsoleWindow,
        ReadConsoleOutputCharacterW, SetConsoleCtrlHandler,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    use windows::core::w;

    use super::{HELPER_FLAG, Tracker};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const POLL: Duration = Duration::from_millis(250);

    /// The helper reading the current game's console; a new game's replaces it.
    static HELPER: Mutex<Option<Child>> = Mutex::new(None);

    /// Starts reading game `pid`'s console, `hide` keeping its window hidden; `sink` gets each batch of
    /// new lines (unredacted) on a thread of its own.
    pub fn watch(pid: u32, hide: bool, sink: Box<dyn Fn(Vec<String>) + Send>) {
        let Ok(exe) = std::env::current_exe() else { return };
        let mut command = Command::new(exe);
        command.arg(HELPER_FLAG).arg(pid.to_string());
        if hide {
            command.arg("--hide");
        }
        let spawned = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                crate::platform::log::error(format!("game console reader: {error}"));
                return;
            }
        };
        let Some(stdout) = child.stdout.take() else { return };
        let mut slot = HELPER.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut old) = slot.replace(child) {
            let _ = old.kill();
            let _ = old.wait();
        }
        drop(slot);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut batch = Vec::new();
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => batch.push(line.trim_end_matches(['\r', '\n']).to_owned()),
                }
                // What else has arrived goes in the same batch, so a burst is one event
                while reader.buffer().contains(&b'\n') {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    batch.push(line.trim_end_matches(['\r', '\n']).to_owned());
                }
                sink(batch);
            }
        });
    }

    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle was opened here and is closed once
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    /// `--read-game-console <pid> [--hide]`; None when this process is not the helper.
    pub fn helper_main() -> Option<ExitCode> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) != Some(HELPER_FLAG) {
            return None;
        }
        let pid: u32 = args.get(1)?.parse().ok()?;
        let hide = args.iter().any(|arg| arg == "--hide");
        // Our stdout is the pipe to the launcher; kept before attaching, which may point it at the console
        let out = std::io::stdout().as_handle().try_clone_to_owned().ok()?;
        let mut out = std::fs::File::from(out);
        // SAFETY: plain calls on handles and ids this process owns or was given
        let game = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.ok().map(Owned);
        let gone = || {
            game.as_ref()
                // SAFETY: a process handle opened above
                .is_none_or(|game| unsafe { WaitForSingleObject(game.0, 0) } == WAIT_OBJECT_0)
        };
        // The console may come later than the game (a DLL opens it): keep trying while the game runs
        // SAFETY: no handles; FreeConsole drops a console this process may have been given
        unsafe {
            let _ = FreeConsole();
        }
        loop {
            if gone() {
                return Some(ExitCode::SUCCESS);
            }
            // SAFETY: attaches this process to another's console
            if unsafe { AttachConsole(pid) }.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        // Ctrl+C typed in the game's console is the game's, not ours
        // SAFETY: no handler, only "ignore"
        unsafe {
            let _ = SetConsoleCtrlHandler(None, true);
        }
        // SAFETY: opens the attached console's screen buffer
        let Ok(console) = (unsafe {
            CreateFileW(
                w!("CONOUT$"),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }) else {
            return Some(ExitCode::FAILURE);
        };
        let console = Owned(console);
        let mut tracker = Tracker::default();
        while !gone() {
            if hide {
                // SAFETY: the attached console's window, if it has one
                unsafe {
                    let window = GetConsoleWindow();
                    if !window.is_invalid() {
                        let _ = ShowWindow(window, SW_HIDE);
                    }
                }
            }
            let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
            // SAFETY: a valid screen buffer handle and an out struct
            if unsafe { GetConsoleScreenBufferInfo(console.0, &mut info) }.is_err() {
                break;
            }
            let width = usize::try_from(info.dwSize.X).unwrap_or(0);
            let height = usize::try_from(info.dwSize.Y).unwrap_or(0);
            let cursor = usize::try_from(info.dwCursorPosition.Y).unwrap_or(0);
            let mut cells = vec![0u16; width];
            let mut row = |r: usize| -> Option<String> {
                if r >= height || width == 0 {
                    return None;
                }
                let mut read = 0u32;
                let at = COORD {
                    X: 0,
                    Y: i16::try_from(r).ok()?,
                };
                // SAFETY: `cells` holds one row of the buffer
                unsafe { ReadConsoleOutputCharacterW(console.0, &mut cells, at, &mut read) }.ok()?;
                Some(String::from_utf16_lossy(&cells[..read as usize]))
            };
            // Masked here too, so the key never leaves this process
            for line in tracker.poll(cursor, &mut row) {
                if writeln!(out, "{}", super::redact(&line)).is_err() {
                    return Some(ExitCode::SUCCESS);
                }
            }
            let _ = out.flush();
            std::thread::sleep(POLL);
        }
        Some(ExitCode::SUCCESS)
    }
}

#[cfg(not(windows))]
mod os {
    /// No console to read outside Windows (the game runs under Proton or Wine there).
    pub fn watch(_pid: u32, _hide: bool, _sink: Box<dyn Fn(Vec<String>) + Send>) {}

    pub fn helper_main() -> Option<std::process::ExitCode> {
        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn the_key_never_survives_a_line() {
        let key = concat!("RRK_", "abcdefghijklmnop", "abcdefghijklmnop");
        let line = format!("LogInit: Command Line:  -NoEAC -AUTH_PASSWORD={key} -epicapp=Archon");
        let out = redact(&line);
        assert!(!out.contains("abcdefghijklmnop"), "{out}");
        assert_eq!(
            out,
            "LogInit: Command Line:  -NoEAC -AUTH_PASSWORD=<hidden> -epicapp=Archon"
        );
        assert_eq!(
            redact(&format!("key {key}, again {key}")),
            "key RRK_<hidden>, again RRK_<hidden>"
        );
        assert_eq!(redact("-auth_password=\"x y\" -next"), "-auth_password=<hidden> -next");
        assert_eq!(redact("nothing here"), "nothing here");
    }

    fn rows(lines: &[&str], width: usize) -> Vec<String> {
        lines.iter().map(|line| format!("{line:width$}")).collect()
    }

    fn poll(tracker: &mut Tracker, buffer: &[String], cursor: usize) -> Vec<String> {
        tracker.poll(cursor, &mut |r| buffer.get(r).cloned())
    }

    #[test]
    fn rows_above_the_cursor_are_read_once() {
        let mut tracker = Tracker::default();
        let mut buffer = rows(&["one", "two", "thr"], 6);
        assert_eq!(poll(&mut tracker, &buffer, 2), ["one", "two"]);
        assert!(poll(&mut tracker, &buffer, 2).is_empty());
        buffer.extend(rows(&["four", ""], 6));
        assert_eq!(poll(&mut tracker, &buffer, 4), ["thr", "four"]);
    }

    #[test]
    fn a_full_row_joins_the_next() {
        let mut tracker = Tracker::default();
        let buffer = vec!["abcdef".to_owned(), "gh    ".to_owned(), "      ".to_owned()];
        assert_eq!(poll(&mut tracker, &buffer, 2), ["abcdefgh"]);
    }

    #[test]
    fn a_scrolled_buffer_is_followed() {
        let mut tracker = Tracker::default();
        let buffer = rows(&["a", "b", "c", "d"], 4);
        assert_eq!(poll(&mut tracker, &buffer, 3), ["a", "b", "c"]);
        // Full: everything moved up two rows and two new lines came in at the bottom
        let scrolled = rows(&["c", "d", "e", "f"], 4);
        assert_eq!(poll(&mut tracker, &scrolled, 3), ["d", "e"]);
    }

    #[test]
    fn a_cleared_buffer_starts_over() {
        let mut tracker = Tracker::default();
        assert_eq!(poll(&mut tracker, &rows(&["a", "b", "c"], 4), 3), ["a", "b", "c"]);
        assert_eq!(poll(&mut tracker, &rows(&["x", ""], 4), 1), ["x"]);
    }
}
