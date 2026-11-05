// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The matchmaking request (spec §4): what the backend's body asks for and how the Director answers it.

use std::sync::Arc;

use serde_json::Value;
use service_kit::js;

use crate::gameservers::{Details, Director, RosterEntry, ServerKind, TRAINING_DOJO_HUNT_ID};

fn trim_error() -> String {
    // Node's throw reads the same for a non-string GameArgs (§16 item 3)
    "HuntId.trim is not a function".to_owned()
}

/// Absent or `null` is none; a string is itself when its trimmed text is not empty; any other type throws.
fn non_blank(value: Option<&Value>) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok((!js::trim(text).is_empty()).then(|| text.clone())),
        Some(_) => Err(trim_error()),
    }
}

/// An array's item count, a string's UTF-16 length, otherwise none.
fn length(value: Option<&Value>) -> Option<usize> {
    match value? {
        Value::Array(items) => Some(items.len()),
        Value::String(text) => Some(text.encode_utf16().count()),
        _ => None,
    }
}

/// `map?Key=Value?...`: the map, and a key's first value (`Key` alone is empty).
pub struct TravelArgs<'a> {
    pub map: &'a str,
    parts: Vec<&'a str>,
}

impl<'a> TravelArgs<'a> {
    pub fn parse(text: &'a str) -> Self {
        let mut pieces = text.split('?');
        let map = pieces.next().unwrap_or("");
        Self {
            map,
            parts: pieces.collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.parts.iter().find_map(|part| match part.split_once('=') {
            Some((k, v)) if k == key => Some(v),
            None if *part == key => Some(""),
            _ => None,
        })
    }
}

fn unsupported(mode: &str, hunt: &str) -> String {
    tracing::error!("Matchmaking failed: unsupported request GameMode='{mode}' HuntId='{hunt}'");
    format!("Unsupported matchmaking request GameMode='{mode}' HuntId='{hunt}'")
}

fn roster(expected: &[Value], hunt: &str) -> Vec<RosterEntry> {
    expected.iter().map(|id| (id.clone(), hunt.to_owned())).collect()
}

impl Director {
    /// The connection details, or the error the route logs and answers 500.
    pub async fn handle_matchmaking(self: &Arc<Self>, body: Option<&Value>) -> Result<Details, String> {
        let Some(body) = body else {
            return Err("Cannot read properties of undefined (reading 'GameMode')".to_owned());
        };
        let (mode, args, hunt, expected) = (
            body.get("GameMode"),
            body.get("GameArgs"),
            body.get("HuntId"),
            body.get("ExpectedPlayers"),
        );
        let (mode_text, hunt_text) = (js::string(mode), js::string(hunt));
        tracing::info!(
            "Handling matchmaking with GameMode: {mode_text} HuntId: {hunt_text} and GameArgs: {}",
            js::string(args)
        );
        let present = |v: Option<&Value>| v.is_some_and(|v| !v.is_null());
        match mode.and_then(Value::as_str) {
            Some("CITY") => self.hub_details(ServerKind::Ramsgate).map_err(|e| e.message),
            Some("SHARED") => {
                let Some(hunt_id) = non_blank(hunt)? else {
                    return Err(unsupported(&mode_text, &hunt_text));
                };
                if hunt_id == TRAINING_DOJO_HUNT_ID {
                    return self.training_for_request().await.map_err(|e| e.message);
                }
                if present(expected) && length(expected).is_some_and(|n| n > 0) {
                    let players = expected.unwrap_or(&Value::Null);
                    if let Some(details) = self.reuse_public_hunt(&hunt_id, players) {
                        return Ok(details);
                    }
                    return self.hunt_from_id(hunt, expected).await.inspect_err(|error| {
                        tracing::error!("SHARED hunt '{hunt_id}' failed to resolve/start: {error}");
                    });
                }
                tracing::error!("SHARED hunt '{hunt_id}' had no ExpectedPlayers; cannot start a hunt server");
                Err(format!("SHARED hunt '{hunt_id}' had no ExpectedPlayers"))
            }
            Some("ISLAND") => {
                let travel = non_blank(args)?;
                let started = match travel {
                    Some(travel) => self.hunt_from_travel(&travel, hunt, expected).await,
                    None => {
                        if non_blank(hunt)?.is_none() || !present(expected) {
                            return Err(unsupported(&mode_text, &hunt_text));
                        }
                        self.hunt_from_id(hunt, expected).await
                    }
                };
                started.inspect_err(|error| {
                    tracing::error!("ISLAND hunt (huntId='{hunt_text}') could not start: {error}");
                })
            }
            _ => Err(unsupported(&mode_text, &hunt_text)),
        }
    }

    async fn hunt_from_travel(
        self: &Arc<Self>,
        travel: &str,
        hunt: Option<&Value>,
        expected: Option<&Value>,
    ) -> Result<Details, String> {
        let args = TravelArgs::parse(travel);
        let behemoth = args.get("MonsterClass").map(str::to_owned);
        let matchmaker = match args.get("HuntID").filter(|id| !js::trim(id).is_empty()) {
            Some(id) => Some(id.to_owned()),
            None => match non_blank(hunt)? {
                Some(hunt_id) => Some(self.resolver().matchmaker_hunt_id_for(&hunt_id)?),
                None => None,
            },
        };
        let empty = expected.is_none_or(Value::is_null) || length(expected) == Some(0);
        let players = if empty {
            None
        } else {
            match non_blank(hunt)? {
                None => None,
                Some(hunt_id) => match expected {
                    Some(Value::Array(ids)) => Some(roster(ids, &hunt_id)),
                    _ => return Err("PlayerIds.map is not a function".to_owned()),
                },
            }
        };
        self.start_hunt(args.map.to_owned(), behemoth, matchmaker, players)
            .await
            .map_err(|e| e.message)
    }

    async fn hunt_from_id(self: &Arc<Self>, hunt: Option<&Value>, expected: Option<&Value>) -> Result<Details, String> {
        let manifest = self.resolver().resolve_manifest(hunt, expected)?;
        let text = |key: &str| js::string(manifest.get(key));
        let behemoth = Some(text("behemothPath")).filter(|b| b != "NO_BEHEMOTH");
        let hunt_text = js::string(hunt);
        let ids = expected.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        self.start_hunt(
            text("mapPath"),
            behemoth,
            Some(text("matchmakerHuntId")),
            Some(roster(ids, &hunt_text)),
        )
        .await
        .map_err(|e| e.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn travel_args() {
        let args = TravelArgs::parse("/Game/Maps/x/y?MonsterClass=/Game/Monsters/a/b_bp.b_bp_C?HuntID=MM_1?Flag");
        assert_eq!(args.map, "/Game/Maps/x/y");
        assert_eq!(args.get("MonsterClass"), Some("/Game/Monsters/a/b_bp.b_bp_C"));
        assert_eq!(args.get("HuntID"), Some("MM_1"));
        assert_eq!(args.get("Flag"), Some(""));
        assert_eq!(args.get("Missing"), None);
        assert_eq!(TravelArgs::parse("a?K=v=w").get("K"), Some("v=w"));
        assert_eq!(TravelArgs::parse("?K").map, "");
        assert_eq!(
            crate::gameservers::label_for(ServerKind::Hunt, Some("/Game/Monsters/fulgar/fulgar_bp.fulgar_bp_C")),
            "fulgar_bp"
        );
        assert_eq!(crate::gameservers::label_for(ServerKind::Hunt, None), "hunt");
        assert_eq!(non_blank(Some(&Value::from(" x "))), Ok(Some(" x ".to_owned())));
        assert_eq!(non_blank(Some(&Value::from("  "))), Ok(None));
        assert!(non_blank(Some(&Value::from(1))).is_err());
        assert_eq!(length(Some(&Value::from("ab"))), Some(2));
    }
}
