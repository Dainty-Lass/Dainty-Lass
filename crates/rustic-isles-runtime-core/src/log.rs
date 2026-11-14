//! The per-port log's text (R0 4.3). The DLL owns the file; these build what goes in it.

/// A local wall-clock time of day (`GetLocalTime`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineTime {
    /// 0 to 23.
    pub hour: u16,
    /// 0 to 59.
    pub minute: u16,
    /// 0 to 59.
    pub second: u16,
    /// 0 to 999.
    pub millis: u16,
}

/// The file name next to the executable; the launcher uploads it by this name.
pub fn file_name(port: u16) -> String {
    format!("rustic_isles_dll_port{port}.log")
}

/// A prefixed line: `[HH:MM:SS.mmm] pid=<pid> <message>\n`.
pub fn format_line(time: LineTime, pid: u32, message: &str) -> String {
    format!(
        "[{:02}:{:02}:{:02}.{:03}] pid={pid} {message}\n",
        time.hour, time.minute, time.second, time.millis
    )
}

/// A raw line: the message and a newline, no prefix (R9's networking lines).
pub fn format_raw(message: &str) -> String {
    format!("{message}\n")
}

/// Whether a line may be written at all: one `WriteFile` takes at most `i32::MAX` bytes.
pub fn writable(line: &str) -> bool {
    i32::try_from(line.len()).is_ok()
}

/// The first status line (R0 4.3).
pub fn status_line(version: &str, server: bool, port: u16, owned: usize) -> String {
    let role = if server { "server" } else { "client" };
    format!("[MitRuntime] rustic-isles-runtime {version} role={role} port={port} owns={owned} ids")
}

/// One install result line.
pub fn install_line(id: &str, result: Result<(), &str>) -> String {
    match result {
        Ok(()) => format!("[MitRuntime] install {id} ok"),
        Err(reason) => format!("[MitRuntime] install {id} {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_format() {
        let time = LineTime {
            hour: 7,
            minute: 5,
            second: 9,
            millis: 42,
        };
        assert_eq!(format_line(time, 1234, "hello"), "[07:05:09.042] pid=1234 hello\n");
        let time = LineTime {
            hour: 23,
            minute: 59,
            second: 59,
            millis: 999,
        };
        assert_eq!(format_line(time, 0, ""), "[23:59:59.999] pid=0 \n");
        assert_eq!(format_raw("raw text"), "raw text\n");
    }

    #[test]
    fn names_and_status() {
        assert_eq!(file_name(0), "rustic_isles_dll_port0.log");
        assert_eq!(file_name(7777), "rustic_isles_dll_port7777.log");
        assert_eq!(
            status_line("0.1.0", true, 7777, 3),
            "[MitRuntime] rustic-isles-runtime 0.1.0 role=server port=7777 owns=3 ids"
        );
        assert_eq!(install_line("SP-1", Ok(())), "[MitRuntime] install SP-1 ok");
        assert_eq!(
            install_line("SP-3", Err("MH_ERROR_NOT_EXECUTABLE")),
            "[MitRuntime] install SP-3 MH_ERROR_NOT_EXECUTABLE"
        );
        assert!(writable("x"));
    }
}
