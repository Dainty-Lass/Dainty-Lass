// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `GET /progression/config`'s served file and the Hunt Pass window rewrite (spec §3.7): the client picks the
//! active pass by date, so the served pass is opened over the current season and every other pass is ended.

use rustic_isles::{ProgressionConfigRule, Settings, VersionTables, season};
use serde_json::{Map, Value};
use service_kit::js;

/// The live service's last pass cutoff (`Date.parse` = 1733184000000).
pub const ENDED_END: &str = "2024-12-03T00:00:00+00:00";
pub const ENDED_END_MS: i64 = 1_733_184_000_000;
pub const ENDED_START: &str = "2019-01-01T00:00:00+00:00";
pub const FUTURE_START: &str = "2099-01-02T00:00:00+00:00";
pub const FUTURE_END: &str = "2099-12-31T00:00:00+00:00";

const PASS_PREFIXES: [&str; 3] = ["eventpass_", "d24_", "season"];

/// The file as the version's rule serves it at `now_ms`; `file` itself is never changed.
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

/// `GAMESERVER_HUNTPASS_ID` when set (empty stays empty), else the table's, JS-trimmed.
pub fn served_huntpass_id(tables: &VersionTables, settings: &Settings) -> String {
    let raw = settings
        .raw("GAMESERVER_HUNTPASS_ID")
        .unwrap_or(tables.active_huntpass_id);
    js::trim(raw).to_owned()
}

/// `GAMESERVER_RECENT_SEASONS` when set (empty: none), else the table's; split on `,`, trimmed, empties dropped.
pub fn recent_seasons(tables: &VersionTables, settings: &Settings) -> Vec<String> {
    let list = match settings.raw("GAMESERVER_RECENT_SEASONS") {
        Some(raw) => raw.to_owned(),
        None => tables.recent_huntpass_seasons.join(","),
    };
    list.split(',')
        .map(|s| js::trim(s).to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn progression_id(path: &Value) -> Option<&str> {
    path.as_object()?.get("progression_id")?.as_str()
}

fn is_pass(path: &Value) -> bool {
    progression_id(path).is_some_and(|id| PASS_PREFIXES.iter().any(|p| starts_with_ci(id, p)))
}

/// Replaces in place when the key exists, else appends it.
fn set(path: &mut Map<String, Value>, key: &str, value: &str) {
    path.insert(key.to_owned(), Value::String(value.to_owned()));
}

fn parse_date(value: Option<&Value>) -> Option<i64> {
    js::date_parse(&js::string(value))
}

/// The rewrite on `config` (spec §3.7); off with `GAMESERVER_WIDEN_HUNTPASS_WINDOWS=0` exactly.
pub fn widen_huntpass_windows(mut config: Value, tables: &VersionTables, settings: &Settings, now_ms: i64) -> Value {
    if settings.raw("GAMESERVER_WIDEN_HUNTPASS_WINDOWS") == Some("0") {
        return config;
    }
    let Some(paths) = config
        .get_mut("payload")
        .and_then(Value::as_object_mut)
        .and_then(|p| p.get_mut("paths"))
        .and_then(Value::as_array_mut)
    else {
        return config;
    };
    let active = served_huntpass_id(tables, settings);
    let (start, end) = season::active_season_window(now_ms);
    let (mut opened, mut ended) = (0usize, 0usize);
    for path in paths.iter_mut() {
        if !is_pass(path) {
            continue;
        }
        let Some(object) = path.as_object_mut() else {
            continue;
        };
        if object.get("progression_id").and_then(Value::as_str) == Some(active.as_str()) {
            set(object, "start_date", &start);
            set(object, "end_date", &end);
            opened += 1;
            continue;
        }
        let ends = parse_date(object.get("end_date"));
        if ends.is_none_or(|e| e > now_ms) {
            set(object, "end_date", ENDED_END);
            if parse_date(object.get("start_date")).is_none_or(|s| s >= ENDED_END_MS) {
                set(object, "start_date", ENDED_START);
            }
            ended += 1;
        }
    }
    if opened == 0 {
        tracing::warn!("Hunt pass windows: the active pass '{active}' is not in the progression config");
    } else {
        tracing::info!(
            "Hunt pass windows: '{active}' {start}..{end} ({opened} path), {ended} other pass windows clamped to have \
             ended"
        );
    }

    let seasons = recent_seasons(tables, settings);
    let template = paths
        .iter()
        .find(|p| progression_id(p) == Some(active.as_str()))
        .or_else(|| {
            paths
                .iter()
                .find(|p| progression_id(p).is_some_and(|id| starts_with_ci(id, "season")))
        })
        .cloned();
    let Some(template) = template else {
        tracing::warn!("Progression config: no season template found; recent seasons not added");
        return config;
    };
    // Not updated as seasons are added, so a season listed twice is added twice (as Node)
    let existing: Vec<String> = paths.iter().filter_map(progression_id).map(str::to_owned).collect();
    let mut added = Vec::new();
    for season in seasons {
        if existing.contains(&season) {
            continue;
        }
        let mut copy = template.clone();
        if let Some(object) = copy.as_object_mut() {
            set(object, "progression_id", &season);
            set(object, "start_date", FUTURE_START);
            set(object, "end_date", FUTURE_END);
        }
        paths.push(copy);
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_isles::testing::SAMPLE_TABLES;
    use serde_json::json;

    static TABLES: VersionTables = VersionTables {
        active_huntpass_id: "season43",
        recent_huntpass_seasons: &["season43", "season44", "season45"],
        ..SAMPLE_TABLES
    };

    fn path(id: &str, start: &str, end: &str) -> Value {
        json!({"progression_id": id, "start_date": start, "end_date": end, "tracks": [1, 2]})
    }

    fn config() -> Value {
        json!({"code": null, "message": "OK", "payload": {"paths": [
            path("ExperienceTrack_Slayer", "2019-01-01T00:00:00+00:00", "2099-01-01T00:00:00+00:00"),
            path("season19", "2020-08-23T00:00:00+00:00", "2099-01-01T00:00:00+00:00"),
            path("season10a", "2022-01-01", "2022-03-01"),
            path("eventpass_x", "2030-01-01T00:00:00+00:00", "2031-01-01T00:00:00+00:00"),
            path("D24_old", "garbage", "garbage"),
            path("season43", "2020-01-01T00:00:00+00:00", "2021-01-01T00:00:00+00:00"),
            null,
            5
        ]}})
    }

    fn now() -> i64 {
        js::date_parse("2026-10-13T18:00:00.000Z").unwrap()
    }

    fn paths(value: &Value) -> Vec<Value> {
        value["payload"]["paths"].as_array().unwrap().clone()
    }

    #[test]
    fn worked_example() {
        let out = widen_huntpass_windows(config(), &TABLES, &Settings::default(), now());
        let p = paths(&out);
        let before = paths(&config());
        assert_eq!(p.len(), 10);
        assert_eq!(p[0], before[0]);
        assert_eq!(p[1], path("season19", "2020-08-23T00:00:00+00:00", ENDED_END));
        assert_eq!(p[2], before[2]);
        assert_eq!(p[3], path("eventpass_x", ENDED_START, ENDED_END));
        assert_eq!(p[4], path("D24_old", ENDED_START, ENDED_END));
        assert_eq!(
            p[5],
            path("season43", "2026-07-30T17:00:00+00:00", "2026-10-22T17:00:00+00:00")
        );
        assert_eq!((&p[6], &p[7]), (&Value::Null, &json!(5)));
        assert_eq!(
            service_kit::jsjson::stringify(&p[8]),
            r#"{"progression_id":"season44","start_date":"2099-01-02T00:00:00+00:00","end_date":"2099-12-31T00:00:00+00:00","tracks":[1,2]}"#
        );
        assert_eq!(p[9]["progression_id"], "season45");
        assert_eq!(js::date_parse(ENDED_END), Some(ENDED_END_MS));
    }

    #[test]
    fn served_id_and_seasons() {
        let s = Settings::from_pairs([("GAMESERVER_HUNTPASS_ID", " season99 ")]);
        assert_eq!(served_huntpass_id(&TABLES, &s), "season99");
        let out = widen_huntpass_windows(config(), &TABLES, &s, now());
        let p = paths(&out);
        assert_eq!(p[5], paths(&config())[5]);
        assert_eq!(p[8]["progression_id"], "season44");
        assert_eq!(p[8]["tracks"], json!([1, 2]));
        assert_eq!(p[8]["start_date"], FUTURE_START);

        let s = Settings::from_pairs([("GAMESERVER_HUNTPASS_ID", "")]);
        assert_eq!(served_huntpass_id(&TABLES, &s), "");
        assert_eq!(served_huntpass_id(&TABLES, &Settings::default()), "season43");

        let s = Settings::from_pairs([("GAMESERVER_RECENT_SEASONS", " season60 ,, season60,season43")]);
        assert_eq!(recent_seasons(&TABLES, &s), ["season60", "season60", "season43"]);
        let p = paths(&widen_huntpass_windows(config(), &TABLES, &s, now()));
        assert_eq!(p.len(), 10);
        assert_eq!(
            (&p[8]["progression_id"], &p[9]["progression_id"]),
            (&json!("season60"), &json!("season60"))
        );
        let s = Settings::from_pairs([("GAMESERVER_RECENT_SEASONS", "")]);
        assert!(recent_seasons(&TABLES, &s).is_empty());
    }

    #[test]
    fn off_and_shapes() {
        let off = Settings::from_pairs([("GAMESERVER_WIDEN_HUNTPASS_WINDOWS", "0")]);
        assert_eq!(widen_huntpass_windows(config(), &TABLES, &off, now()), config());
        let word = Settings::from_pairs([("GAMESERVER_WIDEN_HUNTPASS_WINDOWS", "off")]);
        assert_ne!(widen_huntpass_windows(config(), &TABLES, &word, now()), config());
        for shape in [json!({}), json!({"payload": []}), json!({"payload": {"paths": {}}})] {
            assert_eq!(
                widen_huntpass_windows(shape.clone(), &TABLES, &Settings::default(), now()),
                shape
            );
        }
        let only = json!({"payload": {"paths": [path("ExperienceTrack_Slayer", "a", "b")]}});
        assert_eq!(
            widen_huntpass_windows(only.clone(), &TABLES, &Settings::default(), now()),
            only
        );
        assert_eq!(
            served(
                ProgressionConfigRule::AsIs,
                &config(),
                &TABLES,
                &Settings::default(),
                now()
            ),
            config()
        );
    }

    #[test]
    fn absent_keys_append_in_rule_order() {
        let ended = json!({"payload": {"paths": [{"progression_id": "season1"}, {"progression_id": "season43"}]}});
        let out = widen_huntpass_windows(ended, &TABLES, &Settings::default(), now());
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let p = paths(&out);
        assert_eq!(keys(&p[0]), ["progression_id", "end_date", "start_date"]);
        assert_eq!(keys(&p[1]), ["progression_id", "start_date", "end_date"]);
    }
}
