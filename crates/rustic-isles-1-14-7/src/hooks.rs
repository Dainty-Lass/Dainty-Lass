// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! 1.14.7's answers to the areas (spec §3.3).

use rustic_isles::{
    Destination, Entitlement, GameMode, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule, Settings, StubReply,
    VersionHooks,
};
use serde_json::{Map, Value, json};

const RAMSGATE_HUNT: &str = "ShatteredIsles_ReturnToRamsgate";

/// A number as `JSON.stringify` writes it: integral without a fraction, non-finite `null`.
fn number(n: f64) -> Value {
    rustic_isles_economy::number_value(n)
}

/// 1.14.7's [`VersionHooks`]: the answers areas ask of the version.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hooks;

impl VersionHooks for Hooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        NoPlayerPolicy::ConfiguredAccount
    }

    /// Levels are kept only for exactly `true`, trimmed, any case (§14 item 14).
    fn weapons_at_base_level(&self, settings: &Settings) -> bool {
        !settings
            .raw("KEEP_WEAPON_ITEM_LEVELS")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    }

    fn reforge_rules(&self) -> bool {
        true
    }

    fn mastery_persistence(&self) -> bool {
        true
    }

    fn huntpass_payload(&self, id: &str) -> Value {
        json!({ "huntpass_id": id, "season": id })
    }

    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value {
        let mut out = Map::new();
        out.insert("phx_account_id".into(), Value::from(user_id));
        out.insert("objective_id".into(), Value::from(row.objective_id.as_str()));
        out.insert("progress".into(), number(row.value));
        out.insert("completed_count".into(), number(row.completed_count));
        out.insert("created_date".into(), Value::from(row.created.as_str()));
        out.insert("last_modified_date".into(), Value::from(row.last_modified.as_str()));
        Value::Object(out)
    }

    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::RollingHuntpassWindows
    }

    fn survey_config(&self) -> StubReply {
        StubReply {
            status: 200,
            body: Some(json!({ "code": null, "message": "OK", "payload": { "Triggers": [] } })),
        }
    }

    fn entitlements_payload(&self, list: &[Entitlement]) -> Value {
        let entries = list
            .iter()
            .map(|e| {
                let mut out = Map::new();
                out.insert(
                    "activatedDate".into(),
                    e.activated.as_deref().map_or(Value::Null, Value::from),
                );
                out.insert("duration".into(), number(e.duration));
                out.insert("name".into(), Value::from(e.name.as_str()));
                Value::Object(out)
            })
            .collect();
        json!({ "entitlements": Value::Array(entries) })
    }

    /// `TrainingDojo` is case-sensitive and a shared hub only on a realm hub (§14 item 15).
    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination {
        if hunt_id == RAMSGATE_HUNT {
            return Destination {
                game_mode: GameMode::City,
                hunt_id: None,
            };
        }
        let game_mode = if realm_hub && hunt_id.contains("TrainingDojo") {
            GameMode::Shared
        } else {
            GameMode::Island
        };
        Destination {
            game_mode,
            hunt_id: Some(hunt_id.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dainty_service_kit::jsjson::stringify;

    #[test]
    fn payloads_byte_for_byte() {
        let h = Hooks;
        assert_eq!(
            stringify(&h.huntpass_payload("season43")),
            r#"{"huntpass_id":"season43","season":"season43"}"#
        );
        let row = ObjectiveRow {
            objective_id: "o".into(),
            value: 3.0,
            completed_count: 1.0,
            created: "c".into(),
            last_modified: "m".into(),
        };
        assert_eq!(
            stringify(&h.objective_row("u", &row)),
            r#"{"phx_account_id":"u","objective_id":"o","progress":3,"completed_count":1,"created_date":"c","last_modified_date":"m"}"#
        );
        let survey = h.survey_config();
        assert_eq!(survey.status, 200);
        assert_eq!(
            stringify(survey.body.as_ref().unwrap()),
            r#"{"code":null,"message":"OK","payload":{"Triggers":[]}}"#
        );
        assert_eq!(stringify(&h.entitlements_payload(&[])), r#"{"entitlements":[]}"#);
        let one = Entitlement {
            name: "n".into(),
            duration: 30.0,
            activated: None,
        };
        assert_eq!(
            stringify(&h.entitlements_payload(std::slice::from_ref(&one))),
            r#"{"entitlements":[{"activatedDate":null,"duration":30,"name":"n"}]}"#
        );
        let two = Entitlement {
            name: "b".into(),
            duration: 1.5,
            activated: Some("2026-01-01T00:00:00.000Z".into()),
        };
        assert_eq!(
            stringify(&h.entitlements_payload(&[two, one])),
            r#"{"entitlements":[{"activatedDate":"2026-01-01T00:00:00.000Z","duration":1.5,"name":"b"},{"activatedDate":null,"duration":30,"name":"n"}]}"#
        );
        assert_eq!(h.no_player_policy(), NoPlayerPolicy::ConfiguredAccount);
        assert!(h.reforge_rules() && h.mastery_persistence());
        assert_eq!(h.progression_config(), ProgressionConfigRule::RollingHuntpassWindows);
    }

    #[test]
    fn weapons_at_base_level() {
        let h = Hooks;
        assert!(h.weapons_at_base_level(&Settings::default()));
        for keep in ["true", " TRUE "] {
            assert!(!h.weapons_at_base_level(&Settings::from_pairs([("KEEP_WEAPON_ITEM_LEVELS", keep)])));
        }
        for other in ["yes", "1", ""] {
            assert!(h.weapons_at_base_level(&Settings::from_pairs([("KEEP_WEAPON_ITEM_LEVELS", other)])));
        }
    }

    #[test]
    fn destinations() {
        let h = Hooks;
        let d = |hunt: &str, hub| {
            let d = h.candidate_destination(hunt, hub);
            (d.game_mode, d.hunt_id)
        };
        assert_eq!(d(RAMSGATE_HUNT, true), (GameMode::City, None));
        assert_eq!(d(RAMSGATE_HUNT, false), (GameMode::City, None));
        let dojo = "Arena_TrainingDojo_01";
        assert_eq!(d(dojo, true), (GameMode::Shared, Some(dojo.into())));
        assert_eq!(d(dojo, false), (GameMode::Island, Some(dojo.into())));
        assert_eq!(d("trainingdojo", true), (GameMode::Island, Some("trainingdojo".into())));
        assert_eq!(d("Hunt_X", false), (GameMode::Island, Some("Hunt_X".into())));
        assert_eq!(d("", true), (GameMode::Island, Some(String::new())));
    }
}
