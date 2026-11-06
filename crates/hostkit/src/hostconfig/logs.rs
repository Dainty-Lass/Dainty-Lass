// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `[logs]` (`docs/logging.md`): how the server's console reads, and the GELF collector (Graylog) its log is shipped
//! to. The host process logs for every in-process service, so this is read here, not through an env file.
//!
//! ```toml
//! [logs]
//! format = "text"
//! color = "auto"
//! gelf_url = "https://graylog.example.org:12201/gelf"
//! ```

use serde_json::{Map, Value};
use service_kit::console::{ColorMode, LogFormat};
use service_kit::gelf::GelfTarget;

use crate::error::HostError;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogsConfig {
    pub format: LogFormat,
    pub color: ColorMode,
    /// Checked by [`GelfTarget::parse`] when read
    pub gelf_url: Option<String>,
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

impl LogsConfig {
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("logs must be a table"))?;
        for key in obj.keys() {
            if !["format", "color", "gelfUrl"].contains(&key.as_str()) {
                return Err(bad(format!("logs.{key} is not a setting")));
            }
        }
        let text = |key: &str| obj.get(key).filter(|v| !v.is_null()).map(|v| v.as_str());
        let format = match text("format") {
            None => LogFormat::default(),
            Some(v) => v
                .and_then(LogFormat::parse)
                .ok_or_else(|| bad("logs.format must be \"text\" or \"json\""))?,
        };
        let color = match text("color") {
            None => ColorMode::default(),
            Some(v) => v
                .and_then(ColorMode::parse)
                .ok_or_else(|| bad("logs.color must be \"auto\", \"always\" or \"never\""))?,
        };
        let gelf_url = match text("gelfUrl") {
            None => None,
            Some(Some(url)) if url.trim().is_empty() => None,
            Some(Some(url)) => {
                GelfTarget::parse(url).map_err(|e| bad(format!("logs.gelf_url: {e}")))?;
                Some(url.trim().to_owned())
            }
            Some(None) => return Err(bad("logs.gelf_url must be a URL string")),
        };
        Ok(Self {
            format,
            color,
            gelf_url,
        })
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("format".into(), Value::from(self.format.as_str()));
        out.insert("color".into(), Value::from(self.color.as_str()));
        if let Some(url) = &self.gelf_url {
            out.insert("gelfUrl".into(), Value::from(url.as_str()));
        }
        Value::Object(out)
    }

    /// Where the records are shipped, if anywhere.
    pub fn gelf_target(&self) -> Option<GelfTarget> {
        self.gelf_url.as_deref().and_then(|url| GelfTarget::parse(url).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_and_writes_back() {
        let value = json!({"format": "json", "color": "never", "gelfUrl": "https://logs.example.org:12201/gelf"});
        let logs = LogsConfig::from_json(&value).unwrap();
        assert_eq!((logs.format, logs.color), (LogFormat::Json, ColorMode::Never));
        assert!(logs.gelf_target().is_some());
        assert_eq!(logs.to_json(), value);
        assert_eq!(LogsConfig::from_json(&json!({})).unwrap(), LogsConfig::default());
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        for value in [
            json!({"format": "xml"}),
            json!({"color": true}),
            json!({"gelfUrl": "http://logs.example.org/gelf"}),
            json!({"gelfUrl": 5}),
            json!({"level": "info"}),
        ] {
            assert!(LogsConfig::from_json(&value).is_err(), "{value}");
        }
    }
}
