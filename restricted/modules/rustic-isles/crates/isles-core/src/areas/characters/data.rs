//! `controllers/character.ts`'s character-data rules, on JSON values: the normalisation every read and write applies,
//! the forced "returning player" state, the progression guard, the legacy Dojo series and the wire shape.
//! The stored `data` is a JSON string; its double-encoded members (`AppearanceData`, `CharacterFlagData`...) stay
//! strings, as the 1.12 client reads them and the 1.14.7 measurements kept (character.ts:30-77).

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use service_kit::jsjson;

/// Node threw (`JSON.parse` of stored or sent data); the route answers Express's 500.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DataError(pub String);

fn parse_object(text: &str) -> Result<Map<String, Value>, DataError> {
    match jsjson::parse(if text.is_empty() { "{}" } else { text }) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(DataError("character data is not a JSON object".into())),
        Err(error) => Err(DataError(format!("character data: {error}"))),
    }
}

pub const DEFAULT_CHARACTER_FLAGS: &str = "{\"Flags\":[]}";
pub const NO_RECENT_PLAYERS: &str = "{\"RecentPlayers\":[],\"Version\":0}";

pub fn default_appearance() -> Value {
    json!({
        "CreationState": "EArchonCharacterCreationState::FaceComplete",
        "Data": [{ "SkeletalMeshComponentName": "Head Slot", "MorphData": [] }],
        "AssetReferences": [],
        "StringData": [
            { "Key": "BodyType", "Data": "Feminine" },
            { "Key": "Hair", "Data": "Hair12" },
            { "Key": "SkinName", "Data": "Tan" },
            { "Key": "SkinValue", "Data": "(R=0.610496,G=0.417885,B=0.254152,A=1.000000)" },
            { "Key": "Hair_Color", "Data": "(R=0.196843,G=0.042531,B=0.019093,A=1.000000)" },
            { "Key": "Beard", "Data": "NoBeard" },
            { "Key": "Facepaint", "Data": "NoFacepaint" },
            { "Key": "Makeup", "Data": "NoMakeup" }
        ]
    })
}

/// 1.12 still consults this blob for the Training Grounds destination (character.ts:48-70).
pub fn dojo_series_data() -> String {
    jsjson::stringify(&json!({
        "ID": "Dojo",
        "Status": 0,
        "62B91BD94558409B4F7352B5B96F3ED7": {
            "Status": 0,
            "6CA2C43B46334BC06F73DEB5F2BFFEC1": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 },
            "3A7241AA43743647D3C1E39E8976E4F3": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 }
        },
        "816CBFD94D16EDA252BD1D8461209568": { "Status": 1 },
        "B152371947599B3C2D55BE9B91439C37": {
            "Status": 0,
            "D3F19E2248AECEF5C5C3C8B9E2AC2C67": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 },
            "0407C2134FE0BEFE3EC791999632D2BC": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 }
        },
        "DFE54F884C6FC60688B6C494D79ADD29": {
            "Status": 0,
            "9D1B0D754DBC896034F942AD625F9D93": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 }
        }
    }))
}

/// The 17 tutorial popups the client suppresses in Ramsgate when set.
pub const TUTORIAL_SLATE_FLAGS: [&str; 17] = [
    "TutorialSlate_LanternAbility",
    "TutorialSlate_QuickAttack",
    "TutorialSlate_HeavyAttack",
    "TutorialSlate_Dodge",
    "TutorialSlate_LockOn",
    "TutorialSlate_Sprint",
    "TutorialSlate_Consumables",
    "TutorialSlate_Interact",
    "TutorialSlate_Chat",
    "TutorialSlate_Emote",
    "TutorialSlate_Menu",
    "TutorialSlate_Inventory",
    "TutorialSlate_Loadout",
    "TutorialSlate_Store",
    "TutorialSlate_HuntPass",
    "TutorialSlate_Map",
    "TutorialSlate_Party",
];

/// `EPlayerAccountProgressState` by name (`PROGRESS_ORDER`).
fn progress_ordinal(step: Option<&Value>) -> usize {
    match step.and_then(Value::as_str) {
        Some("New") | None => 0,
        Some("DefeatedGnasher") => 1,
        Some("SavedCharacter") => 2,
        Some("EnteredRamsgate") => 3,
        Some("FinishedFirstHunt") => 4,
        Some("FinishedSecondHunt" | "Final") => 5,
        Some(_) => 0,
    }
}

const SAVED_CHARACTER: usize = 2;
const ENTERED_RAMSGATE: usize = 3;

fn progress_name(ordinal: usize) -> Option<&'static str> {
    [
        "New",
        "DefeatedGnasher",
        "SavedCharacter",
        "EnteredRamsgate",
        "FinishedFirstHunt",
        "FinishedSecondHunt",
    ]
    .get(ordinal)
    .copied()
}

/// `TryParseObj`: an object as it is, a string parsed, anything else (or an unparsable string) nothing.
fn try_parse_object(value: Option<&Value>) -> Option<Map<String, Value>> {
    match value? {
        Value::Object(map) => Some(map.clone()),
        Value::String(text) => match jsjson::parse(text) {
            Ok(Value::Object(map)) => Some(map),
            _ => None,
        },
        _ => None,
    }
}

fn nullish(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

/// `obj.key ??= default`
fn default_to(map: &mut Map<String, Value>, key: &str, value: impl FnOnce() -> Value) {
    if nullish(map.get(key)) {
        map.insert(key.to_owned(), value());
    }
}

/// `Now` as the client writes `LoginTime`: `YYYY.MM.DD-HH.MM.SS` in UTC.
pub fn login_time(now_secs: i64) -> String {
    let (year, month, day) = civil_from_days(now_secs.div_euclid(86_400));
    let in_day = now_secs.rem_euclid(86_400);
    format!(
        "{year:04}.{month:02}.{day:02}-{:02}.{:02}.{:02}",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60
    )
}

/// `toLocaleDateString("en-US", {month: "short", day: "numeric", year: "numeric"})`, in UTC.
pub fn display_date(now_secs: i64) -> String {
    let (year, month, day) = civil_from_days(now_secs.div_euclid(86_400));
    let name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][month as usize - 1];
    format!("{name} {day}, {year}")
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

pub fn now_secs() -> i64 {
    crate::game_data::season_window::now_ms().div_euclid(1000)
}

/// `NormalizeCharacterData`: fills what the client expects, stamps the changelist, and forces the returning-player
/// state (the FTUE bypass, character.ts:166-186) and the progression invariant. `changelist` is the version's
/// `TARGET_CHANGELIST`; an existing `LastChangelist` only survives when there is none.
pub fn normalize(text: &str, changelist: Option<u32>, now: i64) -> Result<String, DataError> {
    let mut data = parse_object(text)?;
    let time = login_time(now);
    default_to(&mut data, "RecentPlayers", || json!(NO_RECENT_PLAYERS));
    default_to(&mut data, "AppearanceData", || {
        json!(jsjson::stringify(&default_appearance()))
    });
    default_to(&mut data, "PlayerAccountProgressStep", || json!("New"));
    default_to(&mut data, "CharacterFlagData", || json!(DEFAULT_CHARACTER_FLAGS));
    default_to(&mut data, "PlayerDataRepair", || json!("{\"Data\":[]}"));
    default_to(&mut data, "SERIE_cr20_pjm_quests", || {
        json!("{\"ID\":\"CR20_PJM_Quests\"}")
    });
    default_to(&mut data, "SERIE_d24_a_main_quests", || {
        json!("{\"ID\":\"D24_A_MAIN_QUESTS\"}")
    });
    default_to(&mut data, "SERIE_d24_b_side_quests", || {
        json!("{\"ID\":\"D24_B_SIDE_QUESTS\"}")
    });
    default_to(&mut data, "SERIE_d24_d_tutorials", || {
        json!("{\"ID\":\"D24_D_TUTORIALS\"}")
    });
    default_to(&mut data, "LoginTime", || json!(time));
    default_to(&mut data, "LastLoginTime", || json!(time));
    let last = match changelist {
        Some(changelist) => json!(changelist.to_string()),
        None if nullish(data.get("LastChangelist")) => json!(""),
        None => data["LastChangelist"].clone(),
    };
    data.insert("LastChangelist".into(), last);
    force_returning_player_state(&mut data);
    normalize_progression_invariants(&mut data);
    Ok(jsjson::stringify(&Value::Object(data)))
}

/// Every account reads as "onboarding complete, in Ramsgate", so the login screen goes straight to the hub: the
/// tutorial hunt starves on beginner-loadout data absent from the catalogs and its cinematic recurses (character.ts:166).
fn force_returning_player_state(data: &mut Map<String, Value>) {
    let step = data.get("PlayerAccountProgressStep").filter(|v| !v.is_null());
    // `String(x ?? "New")` then the table lookup: only a known name has an ordinal
    if progress_ordinal(step) < ENTERED_RAMSGATE {
        data.insert("PlayerAccountProgressStep".into(), json!("EnteredRamsgate"));
    }

    let mut appearance = try_parse_object(data.get("AppearanceData")).unwrap_or_else(|| match default_appearance() {
        Value::Object(map) => map,
        _ => Map::new(),
    });
    let defaults = default_appearance();
    appearance.insert(
        "CreationState".into(),
        json!("EArchonCharacterCreationState::FaceComplete"),
    );
    if appearance
        .get("Data")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        appearance.insert("Data".into(), defaults["Data"].clone());
    }
    // 1.12's FAppearanceStringData is {Key,Data}; the legacy {CustomizationName,ValueName} entry never applies
    let legacy = appearance
        .get("StringData")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| match entry {
                Value::Object(e) => {
                    e.contains_key("CustomizationName") || e.contains_key("ValueName") || !e.contains_key("Key")
                }
                Value::Null | Value::Bool(false) => false,
                Value::String(s) if s.is_empty() => false,
                _ => true,
            })
        });
    if appearance
        .get("StringData")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
        || legacy
    {
        appearance.insert("StringData".into(), defaults["StringData"].clone());
    }
    if !appearance.get("AssetReferences").is_some_and(Value::is_array) {
        appearance.insert("AssetReferences".into(), json!([]));
    }
    data.insert(
        "AppearanceData".into(),
        json!(jsjson::stringify(&Value::Object(appearance))),
    );

    let mut flags = flag_list(data.get("CharacterFlagData"));
    let have: HashSet<String> = flag_keys(&flags).into_iter().map(str::to_owned).collect();
    for key in TUTORIAL_SLATE_FLAGS {
        if !have.contains(key) {
            flags.push(json!({ "FlagKey": key, "FlagValue": "True" }));
        }
    }
    if !have.contains("CharacterFlag_CityGatherableTutorialShown") {
        flags.push(json!({ "FlagKey": "CharacterFlag_CityGatherableTutorialShown", "FlagValue": "True" }));
    }
    data.insert(
        "CharacterFlagData".into(),
        flag_blob(&mut flags_object(data.get("CharacterFlagData")), flags),
    );

    // The "watched the new Ramsgate cinematic" flag lives inside LoginFlagData.Flags
    let mut login = flag_list(data.get("LoginFlagData"));
    if !flag_keys(&login).contains("WatchedNewRamsgateCinematic") {
        login.push(json!({ "FlagKey": "WatchedNewRamsgateCinematic", "FlagValue": "true" }));
    }
    data.insert(
        "LoginFlagData".into(),
        flag_blob(&mut flags_object(data.get("LoginFlagData")), login),
    );
}

/// The parsed flag blob, or `{Flags: []}`.
fn flags_object(value: Option<&Value>) -> Map<String, Value> {
    try_parse_object(value).unwrap_or_else(|| {
        let mut map = Map::new();
        map.insert("Flags".into(), json!([]));
        map
    })
}

fn flag_list(value: Option<&Value>) -> Vec<Value> {
    match flags_object(value).get("Flags") {
        Some(Value::Array(list)) => list.clone(),
        _ => Vec::new(),
    }
}

fn flag_keys(flags: &[Value]) -> HashSet<&str> {
    flags
        .iter()
        .filter_map(|flag| flag.get("FlagKey").and_then(Value::as_str))
        .collect()
}

/// The blob with its `Flags` replaced (other members and their order kept), as JSON text.
fn flag_blob(object: &mut Map<String, Value>, flags: Vec<Value>) -> Value {
    object.insert("Flags".into(), Value::Array(flags));
    json!(jsjson::stringify(&Value::Object(std::mem::take(object))))
}

fn creation_complete(appearance: Option<&Value>) -> bool {
    try_parse_object(appearance).is_some_and(|a| {
        a.get("CreationState").and_then(Value::as_str) == Some("EArchonCharacterCreationState::FaceComplete")
    })
}

/// Progress past the creator needs a completed creator (character.ts:339). The forced state above always completes
/// it, so this only acts on data that reaches it another way.
fn normalize_progression_invariants(data: &mut Map<String, Value>) {
    if creation_complete(data.get("AppearanceData")) {
        return;
    }
    if progress_ordinal(data.get("PlayerAccountProgressStep")) >= SAVED_CHARACTER {
        tracing::warn!(
            "[progression-invariant] Clamped progress to DefeatedGnasher because CreationState is not FaceComplete"
        );
        data.insert("PlayerAccountProgressStep".into(), json!("DefeatedGnasher"));
    }
    // A fresh account must not skip the arrival cinematic
    if data.remove("WatchedNewRamsgateCinematic").is_some() {
        tracing::warn!("[progression-invariant] Cleared top-level WatchedNewRamsgateCinematic");
    }
    if let Some(Value::String(text)) = data.get("LoginFlagData")
        && let Ok(Value::Object(mut login)) = jsjson::parse(text)
        && let Some(Value::Array(list)) = login.get_mut("Flags")
    {
        let before = list.len();
        list.retain(|f| f.get("FlagKey").and_then(Value::as_str) != Some("WatchedNewRamsgateCinematic"));
        if list.len() != before {
            data.insert("LoginFlagData".into(), json!(jsjson::stringify(&Value::Object(login))));
        }
    }
}

/// `EnsureLegacyDojoSeries`: the data normalised, with `SERIE_dojo` added when the Slayer's Path has Dojo unlocked.
/// Returns the data and whether it was added.
pub fn ensure_legacy_dojo_series(
    text: &str,
    dojo_unlocked: bool,
    changelist: Option<u32>,
    now: i64,
) -> Result<(String, bool), DataError> {
    let mut data = parse_object(&normalize(text, changelist, now)?)?;
    let added = dojo_unlocked && nullish(data.get("SERIE_dojo"));
    if added {
        data.insert("SERIE_dojo".into(), json!(dojo_series_data()));
    }
    Ok((jsjson::stringify(&Value::Object(data)), added))
}

/// `ProcessTriggers`: the FTUE series finishing unlocks the Dojo series.
pub fn process_triggers(text: &str, changelist: Option<u32>, now: i64) -> Result<String, DataError> {
    let mut data = parse_object(&normalize(text, changelist, now)?)?;
    if let Some(ftue) = data.get("SERIE_cr19_series_1_ftue").filter(|v| !v.is_null()) {
        let ftue = match ftue {
            Value::String(text) => {
                jsjson::parse(text).map_err(|e| DataError(format!("SERIE_cr19_series_1_ftue: {e}")))?
            }
            other => jsjson::parse(&service_kit::js::string(Some(other)))
                .map_err(|e| DataError(format!("SERIE_cr19_series_1_ftue: {e}")))?,
        };
        let status = ftue
            .get("929A333B40E413C41E47B0A425EC3349")
            .and_then(|n| n.get("Status"))
            .and_then(Value::as_f64);
        if status == Some(3.0) && nullish(data.get("SERIE_dojo")) {
            tracing::info!("Injecting SERIE_dojo!");
            data.insert("SERIE_dojo".into(), json!(dojo_series_data()));
        }
    }
    Ok(jsjson::stringify(&Value::Object(data)))
}

/// The progression guard: `PlayerAccountProgressStep` only moves forward, one step at a time. Returns the text to
/// carry on with: rewritten when the guard acts or the data is an object, as sent when it isn't one.
pub fn guard_progression(incoming: &str, existing: &str) -> String {
    let (Ok(Value::Object(mut incoming_map)), Ok(Value::Object(existing_map))) = (
        jsjson::parse(if incoming.is_empty() { "{}" } else { incoming }),
        jsjson::parse(if existing.is_empty() { "{}" } else { existing }),
    ) else {
        tracing::warn!("[progression-guard] Non-JSON incoming data or parse error - skipping guard");
        return incoming.to_owned();
    };
    let incoming_ordinal = progress_ordinal(incoming_map.get("PlayerAccountProgressStep"));
    let existing_step = existing_map.get("PlayerAccountProgressStep");
    let existing_ordinal = progress_ordinal(existing_step);
    if incoming_ordinal < existing_ordinal {
        tracing::warn!("[progression-guard] Rejecting backward progression; keeping existing");
        match existing_step {
            Some(step) => {
                incoming_map.insert("PlayerAccountProgressStep".into(), step.clone());
            }
            None => {
                incoming_map.remove("PlayerAccountProgressStep");
            }
        }
    } else if incoming_ordinal > existing_ordinal + 1 {
        let clamped = progress_name(existing_ordinal + 1).unwrap_or("New");
        tracing::warn!("[progression-guard] Rejecting skip; clamped to {clamped}");
        incoming_map.insert("PlayerAccountProgressStep".into(), json!(clamped));
    }
    jsjson::stringify(&Value::Object(incoming_map))
}

/// A Slayer's Path node that opens a legacy gate: `node_status` 1 or 2.
fn node_unlocked(node: &Value) -> bool {
    matches!(node.get("node_status").and_then(Value::as_f64), Some(1.0 | 2.0))
}

/// `IsDojoUnlockedForUser` on the journey's stored nodes (`nodes` is JSON text).
pub fn dojo_unlocked(nodes: &str) -> bool {
    let Ok(parsed) = jsjson::parse(if nodes.is_empty() { "{}" } else { nodes }) else {
        return false;
    };
    if parsed.get("Dojo").is_some_and(node_unlocked) {
        return true;
    }
    // Captures where the map key differs but node_id is authoritative
    parsed.as_object().is_some_and(|map| {
        map.values()
            .any(|node| node.get("node_id").and_then(Value::as_str) == Some("Dojo") && node_unlocked(node))
    })
}

/// Whether the persisted onboarding is "fresh": the creator is incomplete or progress is below `SavedCharacter`
/// (`IsFreshOnboarding`; the matchmaking route sends such accounts into the FTUE hunt). Unparsable data counts as fresh.
pub fn is_fresh(data_texts: &[&str]) -> bool {
    !data_texts.iter().any(|text| {
        parse_object(text).is_ok_and(|d| {
            creation_complete(d.get("AppearanceData"))
                && progress_ordinal(d.get("PlayerAccountProgressStep")) >= SAVED_CHARACTER
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_792_000_000; // 2026-10-14

    fn normalized(text: &str) -> Map<String, Value> {
        parse_object(&normalize(text, Some(647472), NOW).unwrap()).unwrap()
    }

    #[test]
    fn dates_are_written_as_the_client_and_node_write_them() {
        assert_eq!(login_time(0), "1970.01.01-00.00.00");
        assert_eq!(login_time(1_792_000_000), "2026.10.14-17.46.40");
        assert_eq!(display_date(1_792_000_000), "Oct 14, 2026");
        assert_eq!(display_date(951_782_400), "Feb 29, 2000");
    }

    #[test]
    fn an_empty_character_is_filled_and_forced_to_the_returning_state() {
        let data = normalized("{}");
        let keys: Vec<&str> = data.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "RecentPlayers",
                "AppearanceData",
                "PlayerAccountProgressStep",
                "CharacterFlagData",
                "PlayerDataRepair",
                "SERIE_cr20_pjm_quests",
                "SERIE_d24_a_main_quests",
                "SERIE_d24_b_side_quests",
                "SERIE_d24_d_tutorials",
                "LoginTime",
                "LastLoginTime",
                "LastChangelist",
                "LoginFlagData"
            ]
        );
        assert_eq!(data["PlayerAccountProgressStep"], "EnteredRamsgate");
        assert_eq!(data["LastChangelist"], "647472");
        assert_eq!(data["LoginTime"], "2026.10.14-17.46.40");
        assert_eq!(data["RecentPlayers"], "{\"RecentPlayers\":[],\"Version\":0}");
        let flags = jsjson::parse(data["CharacterFlagData"].as_str().unwrap()).unwrap();
        assert_eq!(flags["Flags"].as_array().unwrap().len(), 18);
        assert_eq!(
            flags["Flags"][0],
            json!({"FlagKey": "TutorialSlate_LanternAbility", "FlagValue": "True"})
        );
        let login = jsjson::parse(data["LoginFlagData"].as_str().unwrap()).unwrap();
        assert_eq!(
            login,
            json!({"Flags": [{"FlagKey": "WatchedNewRamsgateCinematic", "FlagValue": "true"}]})
        );
        let appearance = jsjson::parse(data["AppearanceData"].as_str().unwrap()).unwrap();
        assert_eq!(appearance, default_appearance());
    }

    #[test]
    fn normalising_twice_changes_nothing() {
        let once = normalize("{\"Custom\":1}", Some(392819), NOW).unwrap();
        assert_eq!(normalize(&once, Some(392819), NOW + 99).unwrap(), once);
    }

    #[test]
    fn what_the_player_already_has_is_kept() {
        let data = normalized(
            r#"{"PlayerAccountProgressStep":"FinishedFirstHunt","LoginTime":"x","LastChangelist":"1",
            "CharacterFlagData":"{\"Flags\":[{\"FlagKey\":\"TutorialSlate_Dodge\",\"FlagValue\":\"False\"}]}",
            "AppearanceData":"{\"CreationState\":\"EArchonCharacterCreationState::NewCharacter\",\"Data\":[1],\"StringData\":[{\"Key\":\"Hair\",\"Data\":\"H1\"}],\"X\":1}"}"#,
        );
        assert_eq!(data["PlayerAccountProgressStep"], "FinishedFirstHunt");
        assert_eq!(data["LoginTime"], "x");
        assert_eq!(data["LastChangelist"], "647472");
        let flags = jsjson::parse(data["CharacterFlagData"].as_str().unwrap()).unwrap();
        assert_eq!(flags["Flags"][0]["FlagValue"], "False");
        assert_eq!(flags["Flags"].as_array().unwrap().len(), 18);
        let appearance = jsjson::parse(data["AppearanceData"].as_str().unwrap()).unwrap();
        assert_eq!(
            appearance["CreationState"],
            "EArchonCharacterCreationState::FaceComplete"
        );
        assert_eq!(appearance["Data"], json!([1]));
        assert_eq!(appearance["StringData"], json!([{"Key": "Hair", "Data": "H1"}]));
        assert_eq!(appearance["X"], 1);
        assert_eq!(appearance["AssetReferences"], json!([]));
    }

    #[test]
    fn a_legacy_string_data_entry_is_healed() {
        let data =
            normalized(r#"{"AppearanceData":"{\"StringData\":[{\"CustomizationName\":\"a\",\"ValueName\":\"b\"}]}"}"#);
        let appearance = jsjson::parse(data["AppearanceData"].as_str().unwrap()).unwrap();
        assert_eq!(
            appearance["StringData"][0],
            json!({"Key": "BodyType", "Data": "Feminine"})
        );
    }

    #[test]
    fn the_changelist_stays_when_the_version_has_none() {
        let kept = parse_object(&normalize("{\"LastChangelist\":\"9\"}", None, NOW).unwrap()).unwrap();
        assert_eq!(kept["LastChangelist"], "9");
        let blank = parse_object(&normalize("{}", None, NOW).unwrap()).unwrap();
        assert_eq!(blank["LastChangelist"], "");
    }

    #[test]
    fn data_that_is_not_an_object_is_an_error() {
        assert!(normalize("not json", None, NOW).is_err());
        assert!(normalize("[1]", None, NOW).is_err());
        assert!(normalize("", None, NOW).is_ok());
    }

    #[test]
    fn progress_only_moves_forward_one_step() {
        let existing = r#"{"PlayerAccountProgressStep":"SavedCharacter"}"#;
        let step = |incoming: &str| -> Value {
            jsjson::parse(&guard_progression(incoming, existing)).unwrap()["PlayerAccountProgressStep"].clone()
        };
        assert_eq!(step(r#"{"PlayerAccountProgressStep":"New"}"#), "SavedCharacter");
        assert_eq!(
            step(r#"{"PlayerAccountProgressStep":"EnteredRamsgate"}"#),
            "EnteredRamsgate"
        );
        assert_eq!(
            step(r#"{"PlayerAccountProgressStep":"FinishedSecondHunt"}"#),
            "EnteredRamsgate"
        );
        assert_eq!(
            step(r#"{"PlayerAccountProgressStep":"SavedCharacter"}"#),
            "SavedCharacter"
        );
        // An unknown name has ordinal 0 and is pulled back to the stored one
        assert_eq!(step(r#"{"PlayerAccountProgressStep":"Bogus"}"#), "SavedCharacter");
        assert_eq!(guard_progression("not json", existing), "not json");
    }

    #[test]
    fn finishing_the_ftue_unlocks_the_dojo_series_once() {
        let done = r#"{"SERIE_cr19_series_1_ftue":"{\"929A333B40E413C41E47B0A425EC3349\":{\"Status\":3}}"}"#;
        let out = parse_object(&process_triggers(done, None, NOW).unwrap()).unwrap();
        assert!(out["SERIE_dojo"].as_str().unwrap().contains("\"ID\":\"Dojo\""));
        let open = r#"{"SERIE_dojo":"mine","SERIE_cr19_series_1_ftue":"{\"929A333B40E413C41E47B0A425EC3349\":{\"Status\":3}}"}"#;
        assert_eq!(
            parse_object(&process_triggers(open, None, NOW).unwrap()).unwrap()["SERIE_dojo"],
            "mine"
        );
        let early = r#"{"SERIE_cr19_series_1_ftue":"{\"929A333B40E413C41E47B0A425EC3349\":{\"Status\":2}}"}"#;
        assert!(
            !parse_object(&process_triggers(early, None, NOW).unwrap())
                .unwrap()
                .contains_key("SERIE_dojo")
        );
        assert!(process_triggers(r#"{"SERIE_cr19_series_1_ftue":"nope"}"#, None, NOW).is_err());
    }

    #[test]
    fn the_journey_unlocks_the_dojo_series_on_read() {
        let (text, added) = ensure_legacy_dojo_series("{}", true, None, NOW).unwrap();
        assert!(added);
        assert!(parse_object(&text).unwrap().contains_key("SERIE_dojo"));
        assert!(!ensure_legacy_dojo_series("{}", false, None, NOW).unwrap().1);
        let (_, again) = ensure_legacy_dojo_series(&text, true, None, NOW).unwrap();
        assert!(!again);
    }

    #[test]
    fn dojo_is_read_from_the_node_or_its_id() {
        assert!(dojo_unlocked(r#"{"Dojo":{"node_status":1}}"#));
        assert!(dojo_unlocked(r#"{"Dojo":{"node_status":2}}"#));
        assert!(!dojo_unlocked(r#"{"Dojo":{"node_status":0}}"#));
        assert!(dojo_unlocked(r#"{"Other":{"node_id":"Dojo","node_status":2}}"#));
        assert!(!dojo_unlocked("garbage"));
        assert!(!dojo_unlocked(""));
    }

    #[test]
    fn onboarding_is_fresh_until_the_creator_and_progress_are_done() {
        let done = normalize("{\"PlayerAccountProgressStep\":\"SavedCharacter\"}", None, NOW).unwrap();
        assert!(!is_fresh(&[&done]));
        assert!(is_fresh(&[]));
        assert!(is_fresh(&["{}", "garbage"]));
        let creator = r#"{"AppearanceData":"{\"CreationState\":\"EArchonCharacterCreationState::NewCharacter\"}","PlayerAccountProgressStep":"Final"}"#;
        assert!(is_fresh(&[creator]));
    }

    #[test]
    fn the_invariant_clamps_progress_without_a_completed_creator() {
        let mut data = Map::new();
        data.insert(
            "AppearanceData".into(),
            json!("{\"CreationState\":\"EArchonCharacterCreationState::NewCharacter\"}"),
        );
        data.insert("PlayerAccountProgressStep".into(), json!("EnteredRamsgate"));
        data.insert("WatchedNewRamsgateCinematic".into(), json!(true));
        data.insert(
            "LoginFlagData".into(),
            json!("{\"Flags\":[{\"FlagKey\":\"WatchedNewRamsgateCinematic\"},{\"FlagKey\":\"k\"}]}"),
        );
        normalize_progression_invariants(&mut data);
        assert_eq!(data["PlayerAccountProgressStep"], "DefeatedGnasher");
        assert!(!data.contains_key("WatchedNewRamsgateCinematic"));
        assert_eq!(data["LoginFlagData"], "{\"Flags\":[{\"FlagKey\":\"k\"}]}");
    }
}
