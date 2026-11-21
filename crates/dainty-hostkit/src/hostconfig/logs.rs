// SPDX-License-Identifier: MIT OR Apache-2.0
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

use dainty_service_kit::console::{ColorMode, LogFormat};
use dainty_service_kit::gelf::GelfTarget;
use serde_json::{Map, Value};

use crate::error::HostError;

/// `[logs]`; each setting left out takes its default.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogsConfig {
    /// Text or JSON lines on the console.
    pub format: LogFormat,
    /// Whether the console lines are colored.
    pub color: ColorMode,
    /// Checked by [`GelfTarget::parse`] when read
    pub gelf_url: Option<String>,
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

impl LogsConfig {
    /// Reads `[logs]`; fails on a key it doesn't know or a value it doesn't take.
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

    /// The table settings.toml writes.
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

/// settings.toml's `[logs]` for the admin API's `logSettings` and `setLogSettings` (the dashboard).
#[cfg(feature = "admin")]
pub struct LogsFile {
    /// The settings.toml the settings are read from and written to.
    pub file: std::path::PathBuf,
    /// What this run started with
    pub started: LogsConfig,
}

#[cfg(feature = "admin")]
impl LogsFile {
    fn view(&self, logs: &LogsConfig) -> dainty_admin::host_settings::LogSettings {
        dainty_admin::host_settings::LogSettings {
            format: logs.format.as_str().to_owned(),
            color: logs.color.as_str().to_owned(),
            gelf_url: logs.gelf_url.clone(),
            restart_needed: *logs != self.started,
        }
    }

    fn read(&self) -> Result<super::HostConfig, String> {
        super::read_host_config(&self.file)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{} does not exist", self.file.display()))
    }
}

#[cfg(feature = "admin")]
impl dainty_admin::host_settings::HostSettings for LogsFile {
    fn logs(&self) -> Result<dainty_admin::host_settings::LogSettings, String> {
        Ok(self.view(&self.read()?.logs.unwrap_or_default()))
    }

    fn set_logs(
        &self,
        change: &dainty_admin::host_settings::LogSettingsInput,
    ) -> Result<dainty_admin::host_settings::LogSettings, String> {
        let mut config = self.read()?;
        let Value::Object(mut fields) = config.logs.clone().unwrap_or_default().to_json() else {
            return Err("logs did not read back as a table".into());
        };
        if let Some(format) = &change.format {
            fields.insert("format".into(), Value::from(format.as_str()));
        }
        if let Some(color) = &change.color {
            fields.insert("color".into(), Value::from(color.as_str()));
        }
        match change.gelf_url.as_deref().map(str::trim) {
            Some("") => {
                fields.remove("gelfUrl");
            }
            Some(url) => {
                fields.insert("gelfUrl".into(), Value::from(url));
            }
            None => {}
        }
        let logs = LogsConfig::from_json(&Value::Object(fields)).map_err(|e| e.to_string())?;
        config.logs = Some(logs.clone());
        super::write_host_config(&self.file, &config).map_err(|e| e.to_string())?;
        Ok(self.view(&logs))
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

    #[cfg(feature = "admin")]
    #[test]
    fn the_dashboard_edits_the_file() {
        use dainty_admin::host_settings::{HostSettings, LogSettingsInput};

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.toml");
        std::fs::write(
            &file,
            "version = 1
mode = \"private\"
name = \"Friday Hunts\"
host = \"100.101.102.103\"
             game_address = \"100.101.102.103\"
registration = \"INVITECODE\"
max_hunt_servers = 3
",
        )
        .unwrap();
        let settings = LogsFile {
            file: file.clone(),
            started: LogsConfig::default(),
        };
        let now = settings.logs().unwrap();
        assert_eq!(
            (now.format.as_str(), now.color.as_str(), now.restart_needed),
            ("text", "auto", false)
        );

        let change = LogSettingsInput {
            format: Some("json".into()),
            color: None,
            gelf_url: Some("tcp://graylog:12201".into()),
        };
        let after = settings.set_logs(&change).unwrap();
        assert_eq!(after.gelf_url.as_deref(), Some("tcp://graylog:12201"));
        assert!(after.restart_needed);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("[logs]") && text.contains("gelf_url = \"tcp://graylog:12201\""),
            "{text}"
        );
        assert!(text.contains("name = \"Friday Hunts\""), "{text}");

        let refused = LogSettingsInput {
            gelf_url: Some("http://logs.example.org/gelf".into()),
            ..LogSettingsInput::default()
        };
        assert!(settings.set_logs(&refused).is_err());
        assert_eq!(
            settings.logs().unwrap().gelf_url.as_deref(),
            Some("tcp://graylog:12201")
        );

        let cleared = LogSettingsInput {
            gelf_url: Some(String::new()),
            ..LogSettingsInput::default()
        };
        assert_eq!(settings.set_logs(&cleared).unwrap().gelf_url, None);
    }
}
