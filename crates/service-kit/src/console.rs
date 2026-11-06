// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! How a log record reads on the console (`[logs]` in host.toml, `docs/logging.md`). The text form keeps Docker
//! Compose's `service | ` prefix, colored per service, then a fixed header a parser can split on:
//!
//! ```text
//! gateway        | 2026-11-06T12:00:00.123Z INFO  gateway::server: listening port=443 mode=public
//! ```
//!
//! The JSON form is one object per line for collectors that read stdout.

use serde_json::{Map, Value};

/// One record, already redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record<'a> {
    /// When it was logged, in Unix milliseconds.
    pub timestamp_ms: i64,
    /// Level name.
    pub level: &'a str,
    /// The service (or module) the record belongs to: `gateway`, `backend`, `isles-1-14-7`
    pub service: &'a str,
    /// The part of it that wrote the record: the Rust module path, such as `rustic_isles_director::gameservers`
    pub target: &'a str,
    /// The message, already redacted.
    pub message: &'a str,
    /// Other fields as `name`, `value` pairs.
    pub fields: &'a [(String, String)],
}

/// How a console line is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// Docker-style text.
    #[default]
    Text,
    /// One JSON object per line.
    Json,
}

impl LogFormat {
    /// Reads `text` or `json`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// The name used in host.toml.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

/// Whether console lines are colored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// On a terminal, unless `NO_COLOR` is set; `CLICOLOR_FORCE` or `FORCE_COLOR` turn it on anywhere
    #[default]
    Auto,
    /// Always.
    Always,
    /// Never.
    Never,
}

impl ColorMode {
    /// Reads `auto`, `always` or `never`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    /// The name used in host.toml.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    /// Whether to color, given whether the output is a terminal and how to read the environment.
    pub fn resolve(self, is_terminal: bool, var: impl Fn(&str) -> Option<String>) -> bool {
        let set = |name: &str| var(name).is_some_and(|v| !v.is_empty() && v != "0");
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto if var("NO_COLOR").is_some_and(|v| !v.is_empty()) => false,
            Self::Auto if set("CLICOLOR_FORCE") || set("FORCE_COLOR") => true,
            Self::Auto => is_terminal,
        }
    }
}

/// The console's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConsoleStyle {
    /// Text or JSON.
    pub format: LogFormat,
    /// Whether to color the text form.
    pub color: bool,
}

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
/// Docker Compose's prefix colors; red is left for errors.
const SERVICE_COLORS: [&str; 10] = [
    "\x1b[36m", "\x1b[33m", "\x1b[32m", "\x1b[35m", "\x1b[34m", "\x1b[96m", "\x1b[93m", "\x1b[92m", "\x1b[95m",
    "\x1b[94m",
];

/// The same service gets the same color in every run (FNV-1a over its name).
pub fn service_color(service: &str) -> &'static str {
    let hash = service
        .bytes()
        .fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    SERVICE_COLORS[hash as usize % SERVICE_COLORS.len()]
}

fn level_color(level: &str) -> &'static str {
    match level {
        "error" => "\x1b[1;31m",
        "warn" => "\x1b[33m",
        "info" => "\x1b[32m",
        "debug" => "\x1b[34m",
        _ => "\x1b[35m",
    }
}

/// `2026-11-06T12:00:00.123Z`, as Docker's `--timestamps` with milliseconds.
pub fn timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// A logfmt value: bare when it holds no space, quote, `=` or control character, quoted and escaped otherwise.
pub fn logfmt_value(value: &str) -> String {
    let bare = !value.is_empty()
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '=');
    if bare {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The lines for one record. A message of several lines gives several, each with the whole header, so every line
/// can be read alone.
pub fn render(record: &Record<'_>, style: ConsoleStyle) -> Vec<String> {
    match style.format {
        LogFormat::Json => vec![json_line(record)],
        LogFormat::Text => text_lines(record, style.color),
    }
}

fn text_lines(record: &Record<'_>, color: bool) -> Vec<String> {
    let (svc, lvl, dim, reset) = if color {
        (service_color(record.service), level_color(record.level), DIM, RESET)
    } else {
        ("", "", "", "")
    };
    let level = record.level.to_ascii_uppercase();
    let header = format!(
        "{svc}{service:<14} |{reset} {dim}{time}{reset} {lvl}{level:<5}{reset} {dim}{target}:{reset}",
        service = record.service,
        time = timestamp(record.timestamp_ms),
        target = record.target,
    );
    let mut fields = String::new();
    for (key, value) in record.fields {
        fields.push(' ');
        fields.push_str(dim);
        fields.push_str(key);
        fields.push('=');
        fields.push_str(reset);
        fields.push_str(&logfmt_value(value));
    }
    let mut lines: Vec<String> = record
        .message
        .lines()
        .map(|line| {
            if line.is_empty() {
                header.clone()
            } else {
                format!("{header} {line}")
            }
        })
        .collect();
    match lines.last_mut() {
        Some(last) => last.push_str(&fields),
        None => lines.push(format!("{header}{fields}")),
    }
    lines
}

fn json_line(record: &Record<'_>) -> String {
    let mut map = Map::new();
    map.insert("time".into(), Value::String(timestamp(record.timestamp_ms)));
    map.insert("level".into(), Value::String(record.level.to_owned()));
    map.insert("service".into(), Value::String(record.service.to_owned()));
    map.insert("target".into(), Value::String(record.target.to_owned()));
    map.insert("message".into(), Value::String(record.message.to_owned()));
    if !record.fields.is_empty() {
        let fields = record
            .fields
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        map.insert("fields".into(), Value::Object(fields));
    }
    serde_json::to_string(&Value::Object(map)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record<'a>(message: &'a str, fields: &'a [(String, String)]) -> Record<'a> {
        Record {
            timestamp_ms: 1_793_966_400_123,
            level: "warn",
            service: "gateway",
            target: "gateway::server",
            message,
            fields,
        }
    }

    #[test]
    fn text_is_docker_like_and_splits_on_its_header() {
        let fields = vec![
            ("port".to_owned(), "443".to_owned()),
            ("peer".to_owned(), "a b".to_owned()),
        ];
        let lines = render(&record("listening", &fields), ConsoleStyle::default());
        assert_eq!(
            lines,
            ["gateway        | 2026-11-06T12:00:00.123Z WARN  gateway::server: listening port=443 peer=\"a b\""]
        );
        let two = render(&record("one\ntwo", &[]), ConsoleStyle::default());
        assert_eq!(two.len(), 2);
        assert!(two[1].ends_with("gateway::server: two"), "{two:?}");
    }

    #[test]
    fn color_wraps_the_parts_and_nothing_else() {
        let style = ConsoleStyle {
            format: LogFormat::Text,
            color: true,
        };
        let line = &render(&record("hi", &[]), style)[0];
        assert!(line.starts_with(service_color("gateway")));
        assert!(line.contains("\x1b[33mWARN "));
        let plain: String = {
            let mut out = String::new();
            let mut skip = false;
            for c in line.chars() {
                match (skip, c) {
                    (false, '\x1b') => skip = true,
                    (true, 'm') => skip = false,
                    (false, c) => out.push(c),
                    _ => {}
                }
            }
            out
        };
        assert_eq!(plain, render(&record("hi", &[]), ConsoleStyle::default())[0]);
        assert_eq!(service_color("gateway"), service_color("gateway"));
    }

    #[test]
    fn json_is_one_object_per_record() {
        let fields = vec![("port".to_owned(), "443".to_owned())];
        let style = ConsoleStyle {
            format: LogFormat::Json,
            color: true,
        };
        let line = &render(&record("a\nb", &fields), style)[0];
        assert_eq!(
            line,
            r#"{"time":"2026-11-06T12:00:00.123Z","level":"warn","service":"gateway","target":"gateway::server","message":"a\nb","fields":{"port":"443"}}"#
        );
    }

    #[test]
    fn auto_color_follows_the_terminal_and_the_conventions() {
        let none = |_: &str| None;
        assert!(ColorMode::Auto.resolve(true, none));
        assert!(!ColorMode::Auto.resolve(false, none));
        assert!(!ColorMode::Auto.resolve(true, |n: &str| (n == "NO_COLOR").then(|| "1".to_owned())));
        assert!(ColorMode::Auto.resolve(false, |n: &str| (n == "FORCE_COLOR").then(|| "1".to_owned())));
        assert!(!ColorMode::Auto.resolve(false, |n: &str| (n == "FORCE_COLOR").then(|| "0".to_owned())));
        assert!(ColorMode::Always.resolve(false, none));
        assert!(!ColorMode::Never.resolve(true, none));
    }

    #[test]
    fn logfmt_quotes_what_would_break_a_field() {
        assert_eq!(logfmt_value("plain"), "plain");
        assert_eq!(logfmt_value(""), "\"\"");
        assert_eq!(logfmt_value("a=b"), "\"a=b\"");
        assert_eq!(logfmt_value("say \"hi\"\n"), "\"say \\\"hi\\\"\\n\"");
    }
}
