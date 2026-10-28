//! The Hunt Pass window arithmetic (`seasonWindow.ts`): a rolling 12-week window the served progression config, the
//! Gauntlet season and the bounty and cooldown limits all hang on (1.14.7 2026-10-08).

use std::cell::RefCell;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// `Date.parse("2024-09-26T17:00:00+00:00")`: season27's real start, a Thursday.
pub const ACTIVE_SEASON_ANCHOR_MS: i64 = 1_727_370_000_000;
pub const ACTIVE_SEASON_LENGTH_MS: i64 = 84 * 24 * 60 * 60 * 1000;

/// Header a replay runner sends to pin the request's clock (epoch milliseconds or ISO-8601 text).
pub const REPLAY_NOW_HEADER: &str = "x-isles-replay-now-ms";
/// Header a replay runner sends to pin the request's deterministic ID stream.
pub const REPLAY_SEED_HEADER: &str = "x-isles-replay-seed";

const NO_OVERRIDE: i64 = i64::MIN;
static NOW_OVERRIDE: AtomicI64 = AtomicI64::new(NO_OVERRIDE);

/// Per-request replay pinning state (clock and deterministic ID stream), propagated across async tasks and
/// synchronous store closures.
#[derive(Debug, Clone, Default)]
pub struct ReplayScope {
    pub now_ms: Option<i64>,
    pub seed: Option<[u8; 32]>,
    pub counter: Arc<AtomicU64>,
}

impl ReplayScope {
    pub fn new(now_ms: Option<i64>, seed_text: Option<&str>) -> Self {
        let seed = seed_text.filter(|s| !s.is_empty()).map(|text| {
            let mut hasher = Sha256::new();
            hasher.update(b"isles-replay-seed:");
            hasher.update(text.as_bytes());
            hasher.finalize().into()
        });
        Self {
            now_ms,
            seed,
            counter: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn is_active(&self) -> bool {
        self.now_ms.is_some() || self.seed.is_some()
    }

    fn next_seeded_uuid(&self) -> Option<uuid::Uuid> {
        let seed = self.seed?;
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut hasher = Sha256::new();
        hasher.update(seed);
        hasher.update(seq.to_le_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        // RFC 4122 version 4 and variant 1 bits so any UUID v4 validator accepts it.
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Some(uuid::Uuid::from_bytes(bytes))
    }

    /// Runs synchronous work (such as a `store.read` / `store.write` closure) with this scope installed on the thread.
    pub fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        if !self.is_active() {
            return f();
        }
        let prev = THREAD_REPLAY_SCOPE.with(|cell| cell.replace(Some(self.clone())));
        struct Guard(Option<ReplayScope>);
        impl Drop for Guard {
            fn drop(&mut self) {
                let prev = self.0.take();
                THREAD_REPLAY_SCOPE.with(|cell| {
                    cell.replace(prev);
                });
            }
        }
        let _guard = Guard(prev);
        f()
    }
}

tokio::task_local! {
    static TASK_REPLAY_SCOPE: ReplayScope;
}

thread_local! {
    static THREAD_REPLAY_SCOPE: RefCell<Option<ReplayScope>> = const { RefCell::new(None) };
}

/// Returns the active [`ReplayScope`] on the current thread or Tokio task, if any.
pub fn current_replay_scope() -> ReplayScope {
    if let Some(scope) = THREAD_REPLAY_SCOPE.with(|cell| cell.borrow().clone()) {
        return scope;
    }
    TASK_REPLAY_SCOPE.try_with(Clone::clone).unwrap_or_default()
}

/// Runs an async future with the given [`ReplayScope`] installed.
pub async fn with_replay_scope<F: Future>(scope: ReplayScope, future: F) -> F::Output {
    if !scope.is_active() {
        return future.await;
    }
    TASK_REPLAY_SCOPE.scope(scope, future).await
}

/// Whether the clock is currently pinned (by a request replay scope or `pin_now_for_tests`).
pub fn is_replay_pinned() -> bool {
    if THREAD_REPLAY_SCOPE.with(|cell| cell.borrow().as_ref().is_some_and(|s| s.now_ms.is_some())) {
        return true;
    }
    if TASK_REPLAY_SCOPE.try_with(|s| s.now_ms.is_some()).unwrap_or(false) {
        return true;
    }
    NOW_OVERRIDE.load(Ordering::Relaxed) != NO_OVERRIDE
}

/// Returns the replay-pinned timestamp in milliseconds if a [`ReplayScope`] is active on the current task/thread.
/// Does NOT read `NOW_OVERRIDE` (`pin_now_for_tests`), so season-window tests do not backdate live JWT `exp` claims.
pub fn replay_now_ms() -> Option<i64> {
    if let Some(ms) = THREAD_REPLAY_SCOPE.with(|cell| cell.borrow().as_ref().and_then(|s| s.now_ms)) {
        return Some(ms);
    }
    TASK_REPLAY_SCOPE.try_with(|s| s.now_ms).ok().flatten()
}

/// Mints a UUID v4: deterministic when a replay seed is active on the current task/thread, random otherwise.
pub fn next_uuid() -> uuid::Uuid {
    if let Some(uuid) = THREAD_REPLAY_SCOPE.with(|cell| cell.borrow().as_ref().and_then(ReplayScope::next_seeded_uuid))
    {
        return uuid;
    }
    if let Ok(Some(uuid)) = TASK_REPLAY_SCOPE.try_with(ReplayScope::next_seeded_uuid) {
        return uuid;
    }
    uuid::Uuid::new_v4()
}

/// `Date.now()`.
pub fn now_ms() -> i64 {
    if let Some(ms) = THREAD_REPLAY_SCOPE.with(|cell| cell.borrow().as_ref().and_then(|s| s.now_ms)) {
        return ms;
    }
    if let Ok(Some(ms)) = TASK_REPLAY_SCOPE.try_with(|s| s.now_ms) {
        return ms;
    }
    let pinned = NOW_OVERRIDE.load(Ordering::Relaxed);
    if pinned != NO_OVERRIDE {
        return pinned;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// Pins [`now_ms`] for golden tests, which compare against Node run at a fixed instant; `None` unpins.
#[doc(hidden)]
pub fn pin_now_for_tests(ms: Option<i64>) {
    NOW_OVERRIDE.store(ms.unwrap_or(NO_OVERRIDE), Ordering::Relaxed);
}

/// The window that contains `now_ms`, as instants: `[start, end)`.
pub fn active_season_window_ms(now_ms: i64) -> (i64, i64) {
    let cycle = (now_ms - ACTIVE_SEASON_ANCHOR_MS).div_euclid(ACTIVE_SEASON_LENGTH_MS);
    let start = ACTIVE_SEASON_ANCHOR_MS + cycle * ACTIVE_SEASON_LENGTH_MS;
    (start, start + ACTIVE_SEASON_LENGTH_MS)
}

/// The same window as the `+00:00` strings the served progression config carries.
pub fn active_season_window(now_ms: i64) -> (String, String) {
    let (start, end) = active_season_window_ms(now_ms);
    (iso_utc(start), iso_utc(end))
}

/// `new Date(ms).toISOString()`: `2026-10-22T17:00:00.000Z` (years 0000 to 9999; JavaScript widens beyond).
pub fn to_iso_string(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let of_day = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3_600_000,
        of_day / 60_000 % 60,
        of_day / 1000 % 60,
        of_day % 1000
    )
}

/// `IsoUtc`: `toISOString()` with `+00:00` for `.000Z`.
pub fn iso_utc(ms: i64) -> String {
    let iso = to_iso_string(ms);
    match iso.strip_suffix(".000Z") {
        Some(stem) => format!("{stem}+00:00"),
        None => iso,
    }
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use service_kit::js::date_parse;

    use super::*;

    fn ms(text: &str) -> i64 {
        date_parse(text).unwrap()
    }

    // seasonWindow.test.ts
    #[test]
    fn the_cycle_is_anchored_on_season27s_real_start_and_is_84_days_long() {
        assert_eq!(ACTIVE_SEASON_ANCHOR_MS, ms("2024-09-26T17:00:00+00:00"));
        assert_eq!(to_iso_string(ACTIVE_SEASON_ANCHOR_MS), "2024-09-26T17:00:00.000Z");
        assert_eq!(ACTIVE_SEASON_LENGTH_MS, 84 * 24 * 60 * 60 * 1000);
        // 1970-01-01 was a Thursday (day 4 of a Sunday-first week), and so is the anchor
        assert_eq!((ACTIVE_SEASON_ANCHOR_MS.div_euclid(86_400_000) + 4).rem_euclid(7), 4);
    }

    #[test]
    fn the_window_containing_now_is_start_inclusive_end_exclusive() {
        let roll = ms("2026-10-22T17:00:00.000Z");
        let before = ms("2026-07-30T17:00:00.000Z");
        assert_eq!(active_season_window_ms(roll - 1), (before, roll));
        assert_eq!(active_season_window_ms(roll), (roll, roll + ACTIVE_SEASON_LENGTH_MS));
        assert_eq!(active_season_window_ms(ms("2026-10-13T18:00:00.000Z")), (before, roll));
        assert_eq!(
            active_season_window_ms(ACTIVE_SEASON_ANCHOR_MS),
            (
                ACTIVE_SEASON_ANCHOR_MS,
                ACTIVE_SEASON_ANCHOR_MS + ACTIVE_SEASON_LENGTH_MS
            )
        );
    }

    #[test]
    fn before_the_anchor_the_cycles_continue_backwards() {
        let (start, end) = active_season_window_ms(ACTIVE_SEASON_ANCHOR_MS - 1);
        assert_eq!(end, ACTIVE_SEASON_ANCHOR_MS);
        assert_eq!(start, ACTIVE_SEASON_ANCHOR_MS - ACTIVE_SEASON_LENGTH_MS);
    }

    #[test]
    fn the_window_keeps_the_plus_zero_notation_of_the_served_config() {
        assert_eq!(
            active_season_window(ms("2026-10-13T18:00:00.000Z")),
            (
                "2026-07-30T17:00:00+00:00".to_owned(),
                "2026-10-22T17:00:00+00:00".to_owned()
            )
        );
        assert_eq!(iso_utc(ms("2026-10-22T17:00:00.000Z")), "2026-10-22T17:00:00+00:00");
        assert_eq!(iso_utc(ms("2026-10-22T17:00:00.250Z")), "2026-10-22T17:00:00.250Z");
        for now in [
            "2025-01-01T00:00:00.000Z",
            "2026-10-22T16:59:59.999Z",
            "2026-10-22T17:00:00.000Z",
            "2030-06-30T12:34:56.789Z",
        ] {
            let (start_ms, end_ms) = active_season_window_ms(ms(now));
            let (start, end) = active_season_window(ms(now));
            assert_eq!(ms(&start), start_ms, "{now}");
            assert_eq!(ms(&end), end_ms, "{now}");
        }
    }

    #[test]
    fn iso_strings_match_to_iso_string() {
        assert_eq!(to_iso_string(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(to_iso_string(-1), "1969-12-31T23:59:59.999Z");
        assert_eq!(
            to_iso_string(ms("2024-02-29T23:59:59.999Z")),
            "2024-02-29T23:59:59.999Z"
        );
        assert_eq!(
            to_iso_string(ms("2099-12-31T00:00:00+00:00")),
            "2099-12-31T00:00:00.000Z"
        );
    }
}
