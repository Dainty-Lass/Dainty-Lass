// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Short-lived external commands (`git`, `npm`, `netsh`, `tailscale`...): run with their output streamed to a log, or
//! captured.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use dainty_service_kit::js;

use crate::error::HostError;

const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// How [`run`] runs a command.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// The folder it runs in.
    pub cwd: PathBuf,
    /// Its whole environment; the inherited one when `None`.
    pub env: Option<Vec<(String, String)>>,
    /// A file each output line is appended to, after a header naming the command.
    pub log_file: Option<PathBuf>,
    /// Prints only error lines instead of every line.
    pub quiet: bool,
}

/// What a command did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunResult {
    /// Its exit code; 1 when it had none (killed by a signal).
    pub code: i32,
    /// Its last non-blank output line.
    pub last_line: Option<String>,
    /// The end of its output, at most 64 KiB.
    pub output: String,
}

/// Whether `line` starts with a JavaScript error name (`TypeError: ...`, `Uncaught Error ...`).
#[must_use]
pub fn is_error_line(line: &str) -> bool {
    let trimmed = js::trim(line);
    let rest = trimmed.strip_prefix("Uncaught ").unwrap_or(trimmed);
    let word_len = rest.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
    if word_len == 0 {
        return false;
    }
    let word = &rest[..word_len];
    word.ends_with("Error")
}

#[cfg(windows)]
fn resolve_windows_command(command: &str, env: Option<&[(String, String)]>) -> PathBuf {
    if command.contains('/') || command.contains('\\') {
        return PathBuf::from(command);
    }
    let path_val = env
        .and_then(|pairs| {
            pairs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
                .map(|(_, v)| std::ffi::OsString::from(v))
        })
        .or_else(|| std::env::var_os("PATH"));
    let Some(path_os) = path_val else {
        return PathBuf::from(command);
    };
    let has_ext = Path::new(command).extension().is_some();
    for dir in std::env::split_paths(&path_os) {
        if has_ext {
            let candidate = dir.join(command);
            if candidate.is_file() {
                return candidate;
            }
        } else {
            for ext in ["cmd", "exe", "bat"] {
                let candidate = dir.join(format!("{command}.{ext}"));
                if candidate.is_file() {
                    return candidate;
                }
            }
            let bare = dir.join(command);
            if bare.is_file() {
                return bare;
            }
        }
    }
    PathBuf::from(command)
}

#[cfg(not(windows))]
fn resolve_windows_command(command: &str, _env: Option<&[(String, String)]>) -> PathBuf {
    PathBuf::from(command)
}

fn retain_tail(buf: &mut String, addition: &str) {
    buf.push_str(addition);
    if buf.len() > MAX_OUTPUT_BYTES {
        let drop_from = buf.len() - MAX_OUTPUT_BYTES;
        let mut idx = drop_from;
        while idx < buf.len() && !buf.is_char_boundary(idx) {
            idx += 1;
        }
        buf.drain(..idx);
    }
}

/// Runs `command` and waits for it, streaming its stdout and stderr to the log and the console as `options` say.
/// Fails when it cannot be started; a failing exit is in the result.
pub fn run(command: &str, args: &[&str], options: &RunOptions) -> Result<RunResult, HostError> {
    let mut log_handle = match &options.log_file {
        Some(file) => {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)
                .map_err(|e| HostError::io(file, e))?;
            let header = if args.is_empty() {
                format!("\n> {command}\n")
            } else {
                format!("\n> {command} {}\n", args.join(" "))
            };
            f.write_all(header.as_bytes()).map_err(|e| HostError::io(file, e))?;
            Some(f)
        }
        None => None,
    };

    let program = resolve_windows_command(command, options.env.as_deref());
    let mut cmd = Command::new(&program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if !options.cwd.as_os_str().is_empty() {
        cmd.current_dir(&options.cwd);
    }
    if let Some(env) = &options.env {
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| HostError::message(format!("failed to start {command}: {e}")))?;

    let state = Arc::new(Mutex::new((RunResult::default(), log_handle.take(), options.quiet)));

    let handle_stream = |stream: Box<dyn std::io::Read + Send>,
                         state: Arc<Mutex<(RunResult, Option<std::fs::File>, bool)>>| {
        std::thread::spawn(move || {
            let reader = BufReader::new(stream);
            for line_res in reader.lines() {
                let Ok(line) = line_res else {
                    break;
                };
                let trimmed = js::trim(&line);
                if let Ok(mut guard) = state.lock() {
                    let (ref mut res, ref mut log_file, quiet) = *guard;
                    if !trimmed.is_empty() {
                        res.last_line = Some(trimmed.to_owned());
                    }
                    let line_with_nl = format!("{line}\n");
                    retain_tail(&mut res.output, &line_with_nl);
                    if let Some(f) = log_file.as_mut() {
                        let _ = f.write_all(line_with_nl.as_bytes());
                    }
                    if quiet {
                        if is_error_line(&line) {
                            eprintln!("  ! {trimmed}");
                        }
                    } else {
                        println!("{line}");
                    }
                }
            }
        })
    };

    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(handle_stream(Box::new(stdout), Arc::clone(&state)));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(handle_stream(Box::new(stderr), Arc::clone(&state)));
    }

    let status = child
        .wait()
        .map_err(|e| HostError::message(format!("failed waiting for {command}: {e}")))?;

    for r in readers {
        let _ = r.join();
    }

    let mut final_res = state.lock().map(|g| g.0.clone()).unwrap_or_default();
    final_res.code = status.code().unwrap_or(1);
    Ok(final_res)
}

/// The command's trimmed stdout; `None` when it can't start, fails or prints nothing.
pub fn capture(command: &str, args: &[&str], cwd: Option<&Path>) -> Result<Option<String>, HostError> {
    let program = resolve_windows_command(command, None);
    let mut cmd = Command::new(&program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = match cmd.output() {
        Ok(out) => out,
        Err(_) => return Ok(None),
    };
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = js::trim(&stdout);
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_line_detection_matches_js_error_patterns() {
        assert!(is_error_line("Error: something failed"));
        assert!(is_error_line("  TypeError: invalid argument"));
        assert!(is_error_line("Uncaught ReferenceError: x is not defined"));
        assert!(is_error_line("SyntaxError"));
        assert!(!is_error_line("error: lowercase"));
        assert!(!is_error_line("Warning: something"));
        assert!(!is_error_line("ErrorCode: 42"));
    }
}
