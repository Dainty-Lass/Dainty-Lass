//! UTC time for the manifests (spec §6.14, §6.15).

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utc {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Utc {
    pub fn now() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0))
            .unwrap_or(0);
        Self::from_unix(seconds)
    }

    pub fn from_unix(seconds: i64) -> Self {
        let days = seconds.div_euclid(86_400);
        let rest = u32::try_from(seconds.rem_euclid(86_400)).unwrap_or(0);
        // Howard Hinnant's days-to-civil.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
        let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year,
            month,
            day,
            hour: rest / 3_600,
            minute: rest / 60 % 60,
            second: rest % 60,
        }
    }

    /// `YYYY-MM-DD`
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// `YYYY-MM-DDTHH:MM:SSZ`
    pub fn timestamp(&self) -> String {
        format!(
            "{}T{:02}:{:02}:{:02}Z",
            self.date(),
            self.hour,
            self.minute,
            self.second
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_unix_time() {
        assert_eq!(Utc::from_unix(0).timestamp(), "1970-01-01T00:00:00Z");
        assert_eq!(Utc::from_unix(951_782_400).date(), "2000-02-29");
        assert_eq!(Utc::from_unix(1_793_565_645).timestamp(), "2026-11-01T20:40:45Z");
    }
}
