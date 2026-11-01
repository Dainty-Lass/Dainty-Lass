//! `GET /progression/config` and the rolling Hunt Pass windows of 1.14.7 (`routes/system.ts`'s
//! `WidenHuntPassWindows`, 2026-10-03 and 2026-10-08): the client picks the active pass by date, so the served file's
//! windows are rewritten at every read; `ProgressionConfigRule::AsIs` serves the file as it is.

use std::collections::HashSet;

use serde_json::Value;
use service_kit::js;

use super::season_window::active_season_window;
use crate::hooks::ProgressionConfigRule;
use crate::settings::Settings;
use crate::tables::VersionTables;

const HUNT_PASS_WINDOW_START: &str = "2019-01-01T00:00:00+00:00";
/// The live service's last pass cutoff
const ENDED_PASS_WINDOW_END: &str = "2024-12-03T00:00:00+00:00";
const FUTURE_PASS_WINDOW_START: &str = "2099-01-02T00:00:00+00:00";
const FUTURE_PASS_WINDOW_END: &str = "2099-12-31T00:00:00+00:00";

/// `GetServedHuntPassId`: the pass `GET /huntpass` serves, `GAMESERVER_HUNTPASS_ID` over `ACTIVE_HUNTPASS_ID`
/// (a value that is set but empty stays empty, as `??` keeps it).
pub fn served_huntpass_id(tables: &VersionTables, settings: &Settings) -> String {
    js::trim(
        settings
            .raw("GAMESERVER_HUNTPASS_ID")
            .unwrap_or(tables.active_huntpass_id),
    )
    .to_owned()
}

/// `GAMESERVER_RECENT_SEASONS` (comma separated) over `RECENT_HUNTPASS_SEASONS`.
pub fn recent_seasons(tables: &VersionTables, settings: &Settings) -> Vec<String> {
    let list = settings
        .raw("GAMESERVER_RECENT_SEASONS")
        .map_or_else(|| tables.recent_huntpass_seasons.join(","), str::to_owned);
    list.split(',')
        .map(|season| js::trim(season).to_owned())
        .filter(|season| !season.is_empty())
        .collect()
}

/// `IsHuntPassProgressionId`: `/^(eventpass_|d24_|season)/i`.
fn is_huntpass_id(id: Option<&Value>) -> bool {
    let Some(Value::String(id)) = id else { return false };
    let id = id.to_ascii_lowercase();
    ["eventpass_", "d24_", "season"]
        .iter()
        .any(|prefix| id.starts_with(prefix))
}

fn progression_id(path: &Value) -> Option<&Value> {
    path.as_object()?.get("progression_id")
}

/// `Date.parse(x)` of a value: `None` for NaN.
fn date_ms(value: Option<&Value>) -> Option<i64> {
    js::date_parse(&js::string(value))
}

fn set_window(path: &mut Value, start: &str, end: &str) {
    if let Some(object) = path.as_object_mut() {
        object.insert("start_date".into(), Value::String(start.to_owned()));
        object.insert("end_date".into(), Value::String(end.to_owned()));
    }
}

/// `WidenHuntPassWindows(config)`: the active pass on the rolling 12-week window, every other pass ended, and the
/// recent seasons added with 2099 windows. `config` is a copy the caller owns.
pub fn widen_huntpass_windows(mut config: Value, tables: &VersionTables, settings: &Settings, now_ms: i64) -> Value {
    if settings.raw("GAMESERVER_WIDEN_HUNTPASS_WINDOWS").unwrap_or("1") == "0" {
        return config;
    }
    let Some(Value::Array(paths)) = config.pointer_mut("/payload/paths") else {
        return config;
    };
    let active_id = served_huntpass_id(tables, settings);
    let ended_end = date_ms(Some(&Value::String(ENDED_PASS_WINDOW_END.to_owned()))).unwrap_or(0);
    let (window_start, window_end) = active_season_window(now_ms);
    let (mut opened, mut ended) = (0, 0);
    for path in paths.iter_mut() {
        if !is_huntpass_id(progression_id(path)) {
            continue;
        }
        if progression_id(path).and_then(Value::as_str) == Some(active_id.as_str()) {
            set_window(path, &window_start, &window_end);
            opened += 1;
            continue;
        }
        let end = date_ms(path.get("end_date"));
        if end.is_none_or(|end| end > now_ms) {
            let start = date_ms(path.get("start_date"));
            if let Some(object) = path.as_object_mut() {
                object.insert("end_date".into(), Value::String(ENDED_PASS_WINDOW_END.to_owned()));
                if start.is_none_or(|start| start >= ended_end) {
                    object.insert("start_date".into(), Value::String(HUNT_PASS_WINDOW_START.to_owned()));
                }
            }
            ended += 1;
        }
    }
    tracing::info!(
        "Hunt pass windows: '{active_id}' {window_start}..{window_end} ({opened} path), {ended} other pass windows \
         clamped to have ended"
    );
    if opened == 0 {
        tracing::warn!("Hunt pass windows: the active pass '{active_id}' is not in the progression config");
    }

    // The newest seasons the game's table exposes, cloned from the active track with a future window, so the lookup
    // for an enabled row succeeds without competing with the active pass
    let template = paths
        .iter()
        .find(|path| progression_id(path).and_then(Value::as_str) == Some(active_id.as_str()))
        .or_else(|| {
            paths.iter().find(|path| {
                progression_id(path)
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.to_ascii_lowercase().starts_with("season"))
            })
        })
        .cloned();
    let Some(template) = template else {
        tracing::warn!("Progression config: no season template found; recent seasons not added");
        return config;
    };
    // Not updated as seasons are added: a repeated id in the list is added twice, as Node does
    let existing: HashSet<String> = paths
        .iter()
        .filter_map(|path| progression_id(path)?.as_str().map(str::to_owned))
        .collect();
    let mut added = Vec::new();
    for season in recent_seasons(tables, settings) {
        if existing.contains(&season) {
            continue;
        }
        let mut clone = template.clone();
        if let Some(object) = clone.as_object_mut() {
            object.insert("progression_id".into(), Value::String(season.clone()));
        }
        set_window(&mut clone, FUTURE_PASS_WINDOW_START, FUTURE_PASS_WINDOW_END);
        paths.push(clone);
        added.push(season);
    }
    if !added.is_empty() {
        tracing::info!(
            "Progression config: added {} recent season paths with future windows ({})",
            added.len(),
            added.join(", ")
        );
    }
    config
}

/// What `GET /progression/config` answers (and `GET /progression/objectives` builds on): the file, rewritten when the
/// version says so.
pub fn served(
    rule: ProgressionConfigRule,
    file: &Value,
    tables: &VersionTables,
    settings: &Settings,
    now_ms: i64,
) -> Value {
    match rule {
        ProgressionConfigRule::AsIs => file.clone(),
        ProgressionConfigRule::RollingHuntpassWindows => widen_huntpass_windows(file.clone(), tables, settings, now_ms),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::tables::StarterManifest;

    const TABLES: VersionTables = VersionTables {
        target_changelist: 1,
        game_version: "0",
        active_huntpass_id: "season43",
        recent_huntpass_seasons: &["season43", "season44", "season45"],
        patchnotes_release_version: "",
        bounty_item_grant_data: &[],
        store_sections: &[],
        seasonal_coin: None,
        masked_journey_nodes: &[],
        starter_manifest: StarterManifest {
            bootstrap_version: "",
            stacked: &[],
            quick_emojis: None,
        },
        switch_defaults: &[],
    };

    fn ms(text: &str) -> i64 {
        js::date_parse(text).unwrap()
    }

    fn pass(id: &str, start: &str, end: &str) -> Value {
        json!({ "progression_id": id, "start_date": start, "end_date": end, "tracks": [1, 2] })
    }

    fn config() -> Value {
        json!({ "code": null, "message": "OK", "payload": { "paths": [
            pass("ExperienceTrack_Slayer", "2019-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00"),
            pass("season19", "2020-08-23T00:00:00+00:00", "2099-01-01T00:00:00+00:00"),
            pass("season10a", "2022-01-01T00:00:00+00:00", "2022-03-01T00:00:00+00:00"),
            pass("eventpass_x", "2030-01-01T00:00:00+00:00", "2031-01-01T00:00:00+00:00"),
            pass("D24_old", "garbage", "garbage"),
            pass("season43", "2020-01-01T00:00:00+00:00", "2021-01-01T00:00:00+00:00"),
            Value::Null,
            json!(5),
        ] } })
    }

    fn paths(config: &Value) -> &Vec<Value> {
        config["payload"]["paths"].as_array().unwrap()
    }

    fn by_id<'a>(config: &'a Value, id: &str) -> &'a Value {
        paths(config).iter().find(|p| p["progression_id"] == id).unwrap()
    }

    #[test]
    fn the_active_pass_gets_the_rolling_window_and_the_others_are_clamped() {
        let now = ms("2026-10-13T18:00:00.000Z");
        let out = widen_huntpass_windows(config(), &TABLES, &Settings::default(), now);
        let active = by_id(&out, "season43");
        assert_eq!(active["start_date"], "2026-07-30T17:00:00+00:00");
        assert_eq!(active["end_date"], "2026-10-22T17:00:00+00:00");
        // a non-pass path is left alone
        assert_eq!(
            by_id(&out, "ExperienceTrack_Slayer")["end_date"],
            "2099-01-01T00:00:00+00:00"
        );
        // a pass that hasn't ended is clamped; its start survives when it began before the cutoff
        let season19 = by_id(&out, "season19");
        assert_eq!(season19["end_date"], "2024-12-03T00:00:00+00:00");
        assert_eq!(season19["start_date"], "2020-08-23T00:00:00+00:00");
        // one that starts after the cutoff gets the 2019 start
        let event = by_id(&out, "eventpass_x");
        assert_eq!(event["start_date"], "2019-01-01T00:00:00+00:00");
        assert_eq!(event["end_date"], "2024-12-03T00:00:00+00:00");
        // a pass that ended is untouched; an unparsable window is clamped as NaN is
        assert_eq!(by_id(&out, "season10a")["end_date"], "2022-03-01T00:00:00+00:00");
        let bad = by_id(&out, "D24_old");
        assert_eq!(bad["start_date"], "2019-01-01T00:00:00+00:00");
        assert_eq!(bad["end_date"], "2024-12-03T00:00:00+00:00");
    }

    #[test]
    fn recent_seasons_are_cloned_from_the_active_pass_with_future_windows() {
        let now = ms("2026-10-13T18:00:00.000Z");
        let out = widen_huntpass_windows(config(), &TABLES, &Settings::default(), now);
        // season43 exists; 44 and 45 are added after the file's own paths
        let ids: Vec<&str> = paths(&out)
            .iter()
            .filter_map(|p| p["progression_id"].as_str())
            .collect();
        assert_eq!(&ids[ids.len() - 2..], ["season44", "season45"]);
        let added = by_id(&out, "season44");
        assert_eq!(added["start_date"], "2099-01-02T00:00:00+00:00");
        assert_eq!(added["end_date"], "2099-12-31T00:00:00+00:00");
        assert_eq!(added["tracks"], json!([1, 2]));
        assert_eq!(
            serde_json::to_string(&added).unwrap(),
            r#"{"progression_id":"season44","start_date":"2099-01-02T00:00:00+00:00","end_date":"2099-12-31T00:00:00+00:00","tracks":[1,2]}"#
        );
    }

    #[test]
    fn without_the_active_pass_the_first_season_path_is_the_template() {
        let settings = Settings::from_pairs([("GAMESERVER_HUNTPASS_ID", " season99 ")]);
        let out = widen_huntpass_windows(config(), &TABLES, &settings, ms("2026-10-13T18:00:00.000Z"));
        // nothing opened; season43 is now just another pass and ended; season19 is the template
        assert_eq!(by_id(&out, "season43")["end_date"], "2021-01-01T00:00:00+00:00");
        assert_eq!(by_id(&out, "season44")["tracks"], json!([1, 2]));
        assert_eq!(by_id(&out, "season19")["end_date"], "2024-12-03T00:00:00+00:00");
    }

    #[test]
    fn a_repeated_recent_season_is_added_twice_and_blanks_are_dropped() {
        let settings = Settings::from_pairs([("GAMESERVER_RECENT_SEASONS", " season60 ,, season60,season43")]);
        let out = widen_huntpass_windows(config(), &TABLES, &settings, 0);
        assert_eq!(
            paths(&out).iter().filter(|p| p["progression_id"] == "season60").count(),
            2
        );
        assert_eq!(recent_seasons(&TABLES, &settings), ["season60", "season60", "season43"]);
    }

    #[test]
    fn the_switch_off_and_a_file_without_paths_serve_the_file_as_it_is() {
        let off = Settings::from_pairs([("GAMESERVER_WIDEN_HUNTPASS_WINDOWS", "0")]);
        assert_eq!(widen_huntpass_windows(config(), &TABLES, &off, 0), config());
        // only exactly "0" is off
        let other = Settings::from_pairs([("GAMESERVER_WIDEN_HUNTPASS_WINDOWS", "off")]);
        assert_ne!(widen_huntpass_windows(config(), &TABLES, &other, 0), config());
        for odd in [
            json!({}),
            json!({ "payload": [] }),
            json!({ "payload": { "paths": {} } }),
        ] {
            assert_eq!(
                widen_huntpass_windows(odd.clone(), &TABLES, &Settings::default(), 0),
                odd
            );
        }
    }

    #[test]
    fn a_config_with_no_season_path_gets_no_recent_seasons() {
        let only = json!({ "payload": { "paths": [pass("ExperienceTrack_Slayer", "a", "b")] } });
        assert_eq!(
            widen_huntpass_windows(only.clone(), &TABLES, &Settings::default(), 0),
            only
        );
    }

    #[test]
    fn the_rule_picks_the_rewrite() {
        let file = config();
        let settings = Settings::default();
        assert_eq!(served(ProgressionConfigRule::AsIs, &file, &TABLES, &settings, 0), file);
        assert_ne!(
            served(
                ProgressionConfigRule::RollingHuntpassWindows,
                &file,
                &TABLES,
                &settings,
                0
            ),
            file
        );
    }

    #[test]
    fn the_served_pass_follows_the_setting_over_the_table() {
        assert_eq!(served_huntpass_id(&TABLES, &Settings::default()), "season43");
        let set = Settings::from_pairs([("GAMESERVER_HUNTPASS_ID", " season19 ")]);
        assert_eq!(served_huntpass_id(&TABLES, &set), "season19");
        // `??` keeps a value that is set but empty
        let empty = Settings::from_pairs([("GAMESERVER_HUNTPASS_ID", "")]);
        assert_eq!(served_huntpass_id(&TABLES, &empty), "");
    }

    #[test]
    fn the_ended_cutoff_constant_is_the_dates_parse() {
        assert_eq!(ms(ENDED_PASS_WINDOW_END), 1_733_184_000_000);
    }
}
