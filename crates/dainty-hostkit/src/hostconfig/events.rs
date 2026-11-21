// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `[events]` (module sharing A5): how long the controller keeps the registry's events. The recorder runs in the
//! host process, so this is read here, not through an env file.
//!
//! ```toml
//! [events]
//! retention_days = 90
//! ```

use serde_json::{Map, Value};

use crate::error::HostError;

/// The longest `retention_days` taken: ten years.
pub const MAX_RETENTION_DAYS: u32 = 3650;

/// `[events]`; left out, events are kept [`dainty_controller::events::DEFAULT_RETENTION_DAYS`] days.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventsConfig {
    /// Days a row is kept; 0 keeps every row.
    pub retention_days: Option<u32>,
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

impl EventsConfig {
    /// Reads `[events]`; fails on a key it doesn't know or a value it doesn't take.
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("events must be a table"))?;
        for key in obj.keys() {
            if key != "retentionDays" {
                return Err(bad(format!("events.{key} is not a setting")));
            }
        }
        let retention_days = match obj.get("retentionDays").filter(|v| !v.is_null()) {
            None => None,
            Some(v) => Some(
                v.as_u64()
                    .and_then(|d| u32::try_from(d).ok())
                    .filter(|d| *d <= MAX_RETENTION_DAYS)
                    .ok_or_else(|| {
                        bad(format!(
                            "events.retention_days must be a whole number from 0 to {MAX_RETENTION_DAYS}"
                        ))
                    })?,
            ),
        };
        Ok(Self { retention_days })
    }

    /// The table settings.toml writes.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(days) = self.retention_days {
            out.insert("retentionDays".into(), Value::from(days));
        }
        Value::Object(out)
    }

    /// What the recorder runs with.
    #[must_use]
    pub fn options(&self) -> dainty_controller::events::EventsOptions {
        dainty_controller::events::EventsOptions::from_days(
            self.retention_days
                .unwrap_or(dainty_controller::events::DEFAULT_RETENTION_DAYS),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_and_writes_back() {
        let value = json!({"retentionDays": 30});
        let events = EventsConfig::from_json(&value).unwrap();
        assert_eq!(events.retention_days, Some(30));
        assert_eq!(events.to_json(), value);
        assert_eq!(EventsConfig::from_json(&json!({})).unwrap(), EventsConfig::default());
        assert_eq!(
            EventsConfig::default().options(),
            dainty_controller::events::EventsOptions::default()
        );
        assert_eq!(
            EventsConfig::from_json(&json!({"retentionDays": 0}))
                .unwrap()
                .options()
                .retention,
            None
        );
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        for value in [
            json!({"retentionDays": -1}),
            json!({"retentionDays": 3651}),
            json!({"retentionDays": "90"}),
            json!({"retentionDays": 1.5}),
            json!({"keep": 90}),
            json!(90),
        ] {
            assert!(EventsConfig::from_json(&value).is_err(), "{value}");
        }
    }
}
