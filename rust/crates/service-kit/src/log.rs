// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fmt;
use std::io::Write;
use std::sync::OnceLock;

use chrono::Utc;
use serde_json::{Map, Number, Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

#[macro_export]
macro_rules! fatal {
    ($($arg:tt)+) => { ::tracing::error!(fatal = true, $($arg)+) };
}

pub fn parse_level(text: Option<&str>) -> Result<LevelFilter, String> {
    match text.map(str::trim) {
        None | Some("" | "info") => Ok(LevelFilter::INFO),
        Some("trace") => Ok(LevelFilter::TRACE),
        Some("debug") => Ok(LevelFilter::DEBUG),
        Some("warn") => Ok(LevelFilter::WARN),
        Some("error" | "fatal") => Ok(LevelFilter::ERROR),
        Some("silent") => Ok(LevelFilter::OFF),
        Some(other) => Err(format!("invalid log level: {other:?}")),
    }
}

pub fn init(level: LevelFilter, pretty: bool) {
    if pretty {
        let _ = tracing_subscriber::registry()
            .with(level)
            .with(tracing_subscriber::fmt::layer())
            .try_init();
    } else {
        let _ = tracing_subscriber::registry()
            .with(level)
            .with(PinoLayer::new(std::io::stdout))
            .try_init();
    }
}

fn hostname() -> &'static str {
    static HOSTNAME: OnceLock<String> = OnceLock::new();
    HOSTNAME.get_or_init(|| gethostname::gethostname().to_string_lossy().into_owned())
}

#[derive(Default)]
struct PinoVisitor {
    fatal: bool,
    msg: Option<String>,
    fields: Map<String, Value>,
}

impl Visit for PinoVisitor {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(Number::from(value)));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(Number::from(value)));
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        let val = i64::try_from(value)
            .map_or_else(|_| Value::String(value.to_string()), |v| Value::Number(Number::from(v)));
        self.fields.insert(field.name().to_owned(), val);
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        let val = u64::try_from(value)
            .map_or_else(|_| Value::String(value.to_string()), |v| Value::Number(Number::from(v)));
        self.fields.insert(field.name().to_owned(), val);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        let val = Number::from_f64(value).map_or(Value::Null, Value::Number);
        self.fields.insert(field.name().to_owned(), val);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "fatal" && value {
            self.fatal = true;
            return;
        }
        self.fields.insert(field.name().to_owned(), Value::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.msg = Some(value.to_owned());
        } else {
            self.fields
                .insert(field.name().to_owned(), Value::String(value.to_owned()));
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.fields.insert(
            field.name().to_owned(),
            json!({
                "type": "Error",
                "message": value.to_string(),
            }),
        );
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let formatted = format!("{value:?}");
        if field.name() == "message" {
            self.msg = Some(formatted);
        } else {
            self.fields
                .insert(field.name().to_owned(), Value::String(formatted));
        }
    }
}

pub struct PinoLayer<W> {
    make: W,
}

impl<W> PinoLayer<W> {
    pub fn new(make: W) -> Self {
        Self { make }
    }
}

impl<S, W, O> Layer<S> for PinoLayer<W>
where
    S: Subscriber,
    W: Fn() -> O + 'static,
    O: Write,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = PinoVisitor::default();
        event.record(&mut visitor);

        let level_num: u64 = match *event.metadata().level() {
            Level::TRACE => 10,
            Level::DEBUG => 20,
            Level::INFO => 30,
            Level::WARN => 40,
            Level::ERROR => {
                if visitor.fatal {
                    60
                } else {
                    50
                }
            }
        };

        let time_ms = Utc::now().timestamp_millis().max(0) as u64;
        let pid = std::process::id();

        let mut map = Map::with_capacity(5 + visitor.fields.len());
        map.insert("level".to_owned(), Value::Number(Number::from(level_num)));
        map.insert("time".to_owned(), Value::Number(Number::from(time_ms)));
        map.insert("pid".to_owned(), Value::Number(Number::from(pid)));
        map.insert("hostname".to_owned(), Value::String(hostname().to_owned()));

        for (k, v) in visitor.fields {
            map.insert(k, v);
        }
        if let Some(msg) = visitor.msg {
            map.insert("msg".to_owned(), Value::String(msg));
        }

        if let Ok(line) = serde_json::to_string(&Value::Object(map)) {
            let mut writer = (self.make)();
            let _ = writeln!(writer, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct VecWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for VecWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn parses_pino_levels() {
        assert_eq!(parse_level(None).unwrap(), LevelFilter::INFO);
        assert_eq!(parse_level(Some("")).unwrap(), LevelFilter::INFO);
        assert_eq!(parse_level(Some("info")).unwrap(), LevelFilter::INFO);
        assert_eq!(parse_level(Some("trace")).unwrap(), LevelFilter::TRACE);
        assert_eq!(parse_level(Some("debug")).unwrap(), LevelFilter::DEBUG);
        assert_eq!(parse_level(Some("warn")).unwrap(), LevelFilter::WARN);
        assert_eq!(parse_level(Some("error")).unwrap(), LevelFilter::ERROR);
        assert_eq!(parse_level(Some("fatal")).unwrap(), LevelFilter::ERROR);
        assert_eq!(parse_level(Some("silent")).unwrap(), LevelFilter::OFF);
        assert!(parse_level(Some("verbose")).is_err());
    }

    #[test]
    fn pino_layer_formats_fields_errors_and_fatal_level() {
        let buf = VecWriter::default();
        let w = buf.clone();
        let sub = tracing_subscriber::registry().with(PinoLayer::new(move || w.clone()));
        tracing::subscriber::with_default(sub, || {
            let err = std::io::Error::other("disk full");
            tracing::info!(port = 3000, err = &err as &dyn std::error::Error, "server listening");
            fatal!(code = 1, "unrecoverable failure");
        });

        let output = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: Value = serde_json::from_str(lines[0]).unwrap();
        let keys: Vec<&str> = first.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["level", "time", "pid", "hostname", "port", "err", "msg"]);
        assert_eq!(first["level"], 30);
        assert_eq!(first["port"], 3000);
        assert_eq!(first["err"], json!({"type": "Error", "message": "disk full"}));
        assert_eq!(first["msg"], "server listening");

        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["level"], 60);
        assert!(second.get("fatal").is_none());
        assert_eq!(second["code"], 1);
        assert_eq!(second["msg"], "unrecoverable failure");
    }
}
