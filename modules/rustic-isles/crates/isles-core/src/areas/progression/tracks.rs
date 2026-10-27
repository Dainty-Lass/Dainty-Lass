//! XP tracks: the rank curves of `progression_config.json`, the approved tracks, the host limits and the test
//! multiplier (`controllers/progression.ts`, `testMultipliers.ts`). Pure apart from the game-data read.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::http::Shared;

use super::util::reply;

/// `MAX_PROGRESSION_GRANT_AMOUNT`.
pub const MAX_PROGRESSION_GRANT_AMOUNT: i64 = 100_000;
pub const PROGRESSION_ID_MAX_LENGTH: usize = 128;
/// `NO_PLAYER_SENTINEL`.
pub const NO_PLAYER: &str = "INVALID";
pub const SELECTED_HUNTPASS_TRACK_ALIAS: &str = "selected_huntpass";

pub const HOST_WEAPON_XP_GRANT_CEILING: i64 = 1000;
pub const HOST_PLAYER_XP_GRANT_CEILING: i64 = 1000;
pub const HOST_HUNT_PASS_XP_GRANT_CEILING: i64 = 200;
pub const HOST_PRESTIGE_GRANT_AMOUNT: i64 = 1;
const PLAYER_EXPERIENCE_TRACK: &str = "ExperienceTrack_PlayerLevel";
const MAX_PRESTIGE_RANKS: i64 = 100_000;
/// `MASTERY_CREDIT_PER_OBJECTIVE`.
pub const MASTERY_CREDIT_PER_OBJECTIVE: i64 = 1;

/// `DEFAULT_PROGRESSION_IDS` without the active hunt pass, which the table supplies last.
pub const DEFAULT_PROGRESSION_IDS: [&str; 23] = [
    "MasteryTrack_PlayerLevel",
    "MasteryTrack_Weapon_Axe",
    "MasteryTrack_Weapon_ChainBlades",
    "MasteryTrack_Weapon_Hammer",
    "MasteryTrack_Weapon_Repeaters",
    "MasteryTrack_Weapon_Sword",
    "MasteryTrack_Weapon_Spear",
    "MasteryTrack_Weapon_Strikers",
    "ExperienceTrack_PlayerLevel",
    "ExperienceTrack_Weapon_Axe",
    "ExperienceTrack_Weapon_ChainBlades",
    "ExperienceTrack_Weapon_Hammer",
    "ExperienceTrack_Weapon_Repeaters",
    "ExperienceTrack_Weapon_Sword",
    "ExperienceTrack_Weapon_Spear",
    "ExperienceTrack_Weapon_Strikers",
    "PrestigeTrack_Weapon_Axe",
    "PrestigeTrack_Weapon_ChainBlades",
    "PrestigeTrack_Weapon_Hammer",
    "PrestigeTrack_Weapon_Repeaters",
    "PrestigeTrack_Weapon_Sword",
    "PrestigeTrack_Weapon_Spear",
    "PrestigeTrack_Weapon_Strikers",
];

/// `APPROVED_XP_PROGRESSION_TRACK_IDS` without the active hunt pass.
const APPROVED_XP_TRACKS: [&str; 24] = [
    "MasteryTrack_PlayerLevel",
    "MasteryTrack_Behemoth",
    "MasteryTrack_Weapon_Axe",
    "MasteryTrack_Weapon_ChainBlades",
    "MasteryTrack_Weapon_Hammer",
    "MasteryTrack_Weapon_Repeaters",
    "MasteryTrack_Weapon_Spear",
    "MasteryTrack_Weapon_Sword",
    "MasteryTrack_Weapon_Strikers",
    "ExperienceTrack_PlayerLevel",
    "ExperienceTrack_Weapon_Axe",
    "ExperienceTrack_Weapon_ChainBlades",
    "ExperienceTrack_Weapon_Hammer",
    "ExperienceTrack_Weapon_Repeaters",
    "ExperienceTrack_Weapon_Spear",
    "ExperienceTrack_Weapon_Sword",
    "ExperienceTrack_Weapon_Strikers",
    "PrestigeTrack_Weapon_Axe",
    "PrestigeTrack_Weapon_ChainBlades",
    "PrestigeTrack_Weapon_Hammer",
    "PrestigeTrack_Weapon_Repeaters",
    "PrestigeTrack_Weapon_Spear",
    "PrestigeTrack_Weapon_Sword",
    "PrestigeTrack_Weapon_Strikers",
];

/// One track's rank curve. `cumulative` holds `(rank_id, cumulative XP)` in the order Node's `Map` kept them.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackCurve {
    pub max_rank: i64,
    pub cumulative: Vec<(i64, f64)>,
    pub prestige_xp_per_level: Option<f64>,
}

impl TrackCurve {
    fn cumulative_for(&self, rank: i64) -> Option<f64> {
        self.cumulative.iter().find(|(id, _)| *id == rank).map(|(_, xp)| *xp)
    }

    /// `RequiredXpForRank`: the cumulative progress a rank needs, prestige ranks included.
    pub fn required_xp_for_rank(&self, rank: i64) -> Option<f64> {
        if rank < 0 {
            return None;
        }
        if rank <= self.max_rank {
            return Some(self.cumulative_for(rank).unwrap_or(0.0));
        }
        let per_level = self.prestige_xp_per_level?;
        if rank > self.max_rank + MAX_PRESTIGE_RANKS {
            return None;
        }
        Some(self.cumulative_for(self.max_rank).unwrap_or(0.0) + (rank - self.max_rank) as f64 * per_level)
    }

    /// `TrackCapXp`: the XP to the top regular rank.
    pub fn cap_xp(&self) -> f64 {
        self.cumulative_for(self.max_rank).unwrap_or(0.0)
    }
}

/// The curves of `progression_config.json` by track id (`BuildTrackCurves`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curves(HashMap<String, TrackCurve>);

impl Curves {
    pub fn from_config(config: &Value) -> Self {
        let mut curves = HashMap::new();
        let paths = config
            .pointer("/payload/paths")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for path in paths {
            let Some(id) = path
                .get("progression_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let Some(requirements) = path
                .get("requirements")
                .and_then(Value::as_array)
                .filter(|r| !r.is_empty())
            else {
                continue;
            };
            let mut sorted: Vec<(i64, f64)> = requirements
                .iter()
                .map(|r| {
                    let rank = service_kit::js::number(r.get("rank_id"));
                    let xp = service_kit::js::number(r.get("xp_required"));
                    (rank as i64, if xp.is_nan() { 0.0 } else { xp })
                })
                .collect();
            sorted.sort_by_key(|(rank, _)| *rank);
            let mut running = 0.0;
            let mut cumulative: Vec<(i64, f64)> = Vec::new();
            for (rank, xp) in &sorted {
                running += xp;
                match cumulative.iter_mut().find(|(id, _)| id == rank) {
                    Some(entry) => entry.1 = running,
                    None => cumulative.push((*rank, running)),
                }
            }
            let per_level = service_kit::js::number(path.pointer("/prestige/xp_per_level"));
            let max_rank = sorted.last().map_or(0, |(rank, _)| *rank);
            curves.insert(
                id.to_owned(),
                TrackCurve {
                    max_rank,
                    cumulative,
                    prestige_xp_per_level: (per_level.is_finite() && per_level > 0.0).then_some(per_level),
                },
            );
        }
        Self(curves)
    }

    pub fn get(&self, id: &str) -> Option<&TrackCurve> {
        self.0.get(id)
    }

    pub fn has(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }

    /// `RankForProgress`.
    pub fn rank_for_progress(&self, id: &str, progress: f64) -> Option<i64> {
        let curve = self.get(id)?;
        if !progress.is_finite() {
            return None;
        }
        let mut rank = 0;
        for (rank_id, cumulative) in &curve.cumulative {
            if *cumulative <= progress && *rank_id > rank {
                rank = *rank_id;
            }
        }
        Some(rank)
    }
}

type Cache = Mutex<HashMap<PathBuf, Arc<Curves>>>;

/// The curves of this module's `progression_config.json`, read once per process as Node does. A missing or unreadable
/// file gives no curves, so no XP track is approved and every grant answers 400.
pub fn curves(shared: &Shared) -> Arc<Curves> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let Some(path) = shared.game_data.backend_file("progression_config.json") else {
        return Arc::new(Curves::default());
    };
    let cache = CACHE.get_or_init(Cache::default);
    let Ok(mut held) = cache.lock() else {
        return Arc::new(Curves::default());
    };
    if let Some(curves) = held.get(&path) {
        return curves.clone();
    }
    let loaded = match read_game_data(&path) {
        Ok(config) => Curves::from_config(&config),
        Err(error) => {
            tracing::error!("progression_config.json: {error}");
            Curves::default()
        }
    };
    let loaded = Arc::new(loaded);
    held.insert(path, loaded.clone());
    loaded
}

/// `loadGameData`: JSON, with a UTF-8 BOM stripped.
pub fn read_game_data(path: &std::path::Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|error| format!("{}: {error}", path.display()))
}

/// `ACTIVE_HUNTPASS_PROGRESSION_ID`: the version's table.
pub fn active_huntpass_id(shared: &Shared) -> &'static str {
    shared.version.tables.active_huntpass_id
}

/// `ServedHuntPassProgressionId`: `GAMESERVER_HUNTPASS_ID` overrides the active pass (set but empty stays empty).
pub fn served_huntpass_id(shared: &Shared) -> String {
    service_kit::js::trim(
        shared
            .settings
            .raw("GAMESERVER_HUNTPASS_ID")
            .unwrap_or_else(|| active_huntpass_id(shared)),
    )
    .to_owned()
}

/// `IsHuntPassXpTrack`.
pub fn is_huntpass_xp_track(shared: &Shared, id: &str) -> bool {
    id == active_huntpass_id(shared) || id == served_huntpass_id(shared)
}

/// `/^(season|eventpass_|d24_)/i`.
pub fn is_huntpass_progression_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    ["season", "eventpass_", "d24_"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

/// `ResolveProgressionTrackId`: `selected_huntpass` is the active pass; anything else needs a curve.
pub fn resolve_track_id(shared: &Shared, id: &str) -> Option<String> {
    if id == SELECTED_HUNTPASS_TRACK_ALIAS {
        return Some(active_huntpass_id(shared).to_owned());
    }
    curves(shared).has(id).then(|| id.to_owned())
}

/// `IsApprovedXpProgressionTrack`.
pub fn is_approved_xp_track(shared: &Shared, id: &str) -> bool {
    let served = served_huntpass_id(shared);
    let approved = APPROVED_XP_TRACKS.contains(&id)
        || id == active_huntpass_id(shared)
        || (id == served && is_huntpass_progression_id_pattern(&served));
    approved && curves(shared).has(id)
}

/// `HUNT_PASS_ID_PATTERN`.
fn is_huntpass_progression_id_pattern(id: &str) -> bool {
    is_huntpass_progression_id(id)
}

/// `DescribeUnapprovedXpTrack`.
pub fn describe_unapproved_xp_track(shared: &Shared, id: &str) -> String {
    let linked = id
        .strip_prefix("Linked_Slayer_Slot_")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if linked {
        return "Slayer Link track (the hunt pass also_progress target) - deliberately not approved".into();
    }
    if !curves(shared).has(id) {
        return "no rank curve in progression_config.json".into();
    }
    if is_huntpass_progression_id(id) {
        return format!("hunt pass that is not served (served={})", served_huntpass_id(shared));
    }
    "not on the XP allowlist".into()
}

/// `TrackCapXp`.
pub fn track_cap_xp(shared: &Shared, id: &str) -> Option<f64> {
    curves(shared).get(id).map(TrackCurve::cap_xp)
}

/// `BuildProgressionGrantRequestHash`: sha256 of `JSON.stringify({version, userId, progressionId, amount})`.
pub fn grant_request_hash(user_id: &str, progression_id: &str, amount: i64) -> String {
    let body = json!({ "version": 1, "userId": user_id, "progressionId": progression_id, "amount": amount });
    sha256_hex(super::util::stringify(&body).as_bytes())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `IsValidProgressionGrantId`: `/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/`.
pub fn is_valid_grant_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostGrantRefusal {
    pub reason: &'static str,
    pub progression_id: String,
    pub amount: i64,
    pub limit: i64,
    pub stored: Option<i64>,
}

/// `HostXpGrantCeiling`: 0 = not writable by a host, `None` = no opinion.
pub fn host_xp_grant_ceiling(shared: &Shared, id: &str) -> Option<i64> {
    if id == PLAYER_EXPERIENCE_TRACK {
        Some(HOST_PLAYER_XP_GRANT_CEILING)
    } else if id.starts_with("ExperienceTrack_Weapon_") {
        Some(HOST_WEAPON_XP_GRANT_CEILING)
    } else if id.starts_with("PrestigeTrack_Weapon_") {
        Some(HOST_PRESTIGE_GRANT_AMOUNT)
    } else if id.starts_with("MasteryTrack_") {
        Some(0)
    } else if is_huntpass_xp_track(shared, id) {
        Some(HOST_HUNT_PASS_XP_GRANT_CEILING)
    } else {
        None
    }
}

/// `CheckHostXpGrant`.
pub fn check_host_xp_grant(shared: &Shared, id: &str, amount: i64) -> Option<HostGrantRefusal> {
    let ceiling = host_xp_grant_ceiling(shared, id)?;
    let refusal = |reason, limit| HostGrantRefusal {
        reason,
        progression_id: id.to_owned(),
        amount,
        limit,
        stored: None,
    };
    if ceiling == 0 {
        return Some(refusal("track_not_writable", 0));
    }
    if id.starts_with("PrestigeTrack_Weapon_") {
        return (amount != HOST_PRESTIGE_GRANT_AMOUNT).then(|| refusal("amount_not_one", HOST_PRESTIGE_GRANT_AMOUNT));
    }
    (amount > ceiling).then(|| refusal("amount_over_ceiling", ceiling))
}

/// `HostTrackTotalCap`: only the player level track has a total a host cannot push past.
pub fn host_track_total_cap(shared: &Shared, id: &str) -> Option<i64> {
    if id != PLAYER_EXPERIENCE_TRACK {
        return None;
    }
    track_cap_xp(shared, id).filter(|cap| *cap > 0.0).map(|cap| cap as i64)
}

/// `CheckHostTrackTotal`: refused only when the track is already full.
pub fn check_host_track_total(shared: &Shared, id: &str, stored: i64, amount: i64) -> Option<HostGrantRefusal> {
    let cap = host_track_total_cap(shared, id)?;
    (stored >= cap).then(|| HostGrantRefusal {
        reason: "track_full",
        progression_id: id.to_owned(),
        amount,
        limit: cap,
        stored: Some(stored),
    })
}

/// `DescribeHostGrantRefusal`.
pub fn describe_host_refusal(refusal: &HostGrantRefusal) -> String {
    let track: String = refusal
        .progression_id
        .chars()
        .take(64)
        .map(|c| if ('\u{21}'..='\u{7e}').contains(&c) { c } else { '?' })
        .collect();
    let mut text = format!(
        "reason={} track={track} amount={} limit={}",
        refusal.reason, refusal.amount, refusal.limit
    );
    if let Some(stored) = refusal.stored {
        text.push_str(&format!(" stored={stored}"));
    }
    text
}

/// `HostGrantLimitDenial`: 403, like the module's own refusals; never a clamp.
pub fn host_limit_denial() -> Response {
    reply(
        StatusCode::FORBIDDEN.as_u16(),
        json!({ "code": "HOST_WRITE_LIMIT", "message": "This write is beyond what a hosted server may report.", "payload": null }),
    )
}

/// `ExperienceTrackForPrestige`: `PrestigeTrack_Weapon_X` to `ExperienceTrack_Weapon_X`.
pub fn experience_track_for_prestige(id: &str) -> Option<String> {
    let weapon = id.strip_prefix("PrestigeTrack_Weapon_")?;
    is_weapon(weapon).then(|| format!("ExperienceTrack_Weapon_{weapon}"))
}

/// `IsReforgeableTrack`.
pub fn is_reforgeable_track(id: &str) -> bool {
    id.strip_prefix("ExperienceTrack_Weapon_").is_some_and(is_weapon)
}

fn is_weapon(name: &str) -> bool {
    matches!(
        name,
        "Axe" | "ChainBlades" | "Hammer" | "Repeaters" | "Spear" | "Strikers" | "Sword"
    )
}

pub fn is_mastery_track(id: &str) -> bool {
    id.starts_with("MasteryTrack_")
}

/// The test multipliers (`testMultipliers.ts`): XP only, rewards are `POST /inventory`'s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TestMultipliers {
    pub xp: f64,
    pub rewards: f64,
}

const MAX_TEST_MULTIPLIER: f64 = 100.0;

fn parse_multiplier(raw: Option<&str>) -> Option<f64> {
    let Some(raw) = raw.filter(|r| !service_kit::js::trim(r).is_empty()) else {
        return Some(1.0);
    };
    let value = super::util::js_number(raw);
    (value.is_finite() && (1.0..=MAX_TEST_MULTIPLIER).contains(&value)).then_some(value)
}

/// `IsTailscaleHost`: a Tailscale IPv4 (100.64.0.0/10) or a `*.ts.net` name.
pub fn is_tailscale_host(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    if let Ok(address) = host.parse::<std::net::Ipv4Addr>() {
        let [a, b, ..] = address.octets();
        return a == 100 && (64..=127).contains(&b);
    }
    host.ends_with(".ts.net")
        && host.len() > ".ts.net".len()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// `ReadTestMultipliers`: off unless the host kit set up a private server on a Tailscale address.
pub fn test_multipliers(shared: &Shared) -> TestMultipliers {
    let off = TestMultipliers { xp: 1.0, rewards: 1.0 };
    let (Some(xp), Some(rewards)) = (
        parse_multiplier(shared.settings.raw("REFORGED_XP_MULTIPLIER")),
        parse_multiplier(shared.settings.raw("REFORGED_REWARD_MULTIPLIER")),
    ) else {
        return off;
    };
    if xp == 1.0 && rewards == 1.0 {
        return off;
    }
    if shared.settings.raw("REFORGED_SERVER_MODE") != Some("private") {
        return off;
    }
    if !is_tailscale_host(shared.settings.raw("REFORGED_SERVER_HOST").unwrap_or_default()) {
        return off;
    }
    TestMultipliers { xp, rewards }
}

/// `MultiplyXpGrant`.
pub fn multiply_xp_grant(amount: i64, max_amount: i64, multipliers: TestMultipliers) -> i64 {
    if multipliers.xp == 1.0 {
        return amount;
    }
    let multiplied = (amount as f64 * multipliers.xp).floor() as i64;
    max_amount.min(amount.max(multiplied))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_ids_follow_the_pattern() {
        assert!(is_valid_grant_id("gs-0b2f"));
        assert!(is_valid_grant_id(&"a".repeat(128)));
        assert!(!is_valid_grant_id(&"a".repeat(129)));
        assert!(!is_valid_grant_id("-lead"));
        assert!(!is_valid_grant_id("has space"));
        assert!(!is_valid_grant_id(""));
    }

    #[test]
    fn weapon_tracks_map_to_their_experience_track() {
        assert_eq!(
            experience_track_for_prestige("PrestigeTrack_Weapon_Axe").as_deref(),
            Some("ExperienceTrack_Weapon_Axe")
        );
        assert_eq!(experience_track_for_prestige("PrestigeTrack_Weapon_Bow"), None);
        assert!(is_reforgeable_track("ExperienceTrack_Weapon_Sword"));
        assert!(!is_reforgeable_track("ExperienceTrack_PlayerLevel"));
    }

    #[test]
    fn tailscale_hosts() {
        assert!(is_tailscale_host("100.64.0.1"));
        assert!(is_tailscale_host("100.127.255.254"));
        assert!(!is_tailscale_host("100.128.0.1"));
        assert!(!is_tailscale_host("192.168.0.1"));
        assert!(is_tailscale_host("box.tail1234.ts.net"));
        assert!(!is_tailscale_host(".ts.net"));
        assert!(!is_tailscale_host("evil.example"));
    }

    #[test]
    fn xp_multiplier_is_capped_at_the_route_maximum() {
        let on = TestMultipliers { xp: 3.0, rewards: 1.0 };
        assert_eq!(multiply_xp_grant(100, 100_000, on), 300);
        assert_eq!(multiply_xp_grant(50_000, 100_000, on), 100_000);
        assert_eq!(
            multiply_xp_grant(100, 100_000, TestMultipliers { xp: 1.0, rewards: 1.0 }),
            100
        );
        assert_eq!(
            multiply_xp_grant(7, 100_000, TestMultipliers { xp: 1.5, rewards: 1.0 }),
            10
        );
    }

    #[test]
    fn curves_sum_the_incremental_requirements() {
        let config = json!({ "payload": { "paths": [
            { "progression_id": "t", "requirements": [
                { "rank_id": 2, "xp_required": 20 }, { "rank_id": 1, "xp_required": 10 }, { "rank_id": 3, "xp_required": 5 }],
              "prestige": { "xp_per_level": 100 } },
            { "progression_id": "empty", "requirements": [] },
        ] } });
        let curves = Curves::from_config(&config);
        assert!(!curves.has("empty"));
        let t = curves.get("t").unwrap();
        assert_eq!((t.max_rank, t.cap_xp()), (3, 35.0));
        assert_eq!(t.required_xp_for_rank(2), Some(30.0));
        assert_eq!(t.required_xp_for_rank(0), Some(0.0));
        assert_eq!(t.required_xp_for_rank(5), Some(235.0));
        assert_eq!(t.required_xp_for_rank(-1), None);
        assert_eq!(curves.rank_for_progress("t", 31.0), Some(2));
        assert_eq!(curves.rank_for_progress("t", 5.0), Some(0));
        assert_eq!(curves.rank_for_progress("missing", 5.0), None);
    }
}
