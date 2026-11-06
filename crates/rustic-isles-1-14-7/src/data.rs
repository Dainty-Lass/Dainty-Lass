// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `version_data.json` (spec §4.3): the game-derived values that are not wire facts, which restricted ships and the
//! host installs in the backend game data folder. Nothing here is committed with values.

use std::collections::HashSet;
use std::fmt;

use rustic_isles::GameDataRoot;
use rustic_isles_characters::{CharacterData, StarterKit};
use rustic_isles_db::LegacyStarter;
use rustic_isles_gamedata::{RewardCachePlan, TokenSource, VersionInputs};
use serde_json::{Map, Value};

pub const DATA_FILE: &str = "version_data.json";

const MEMBERS: [&str; 7] = [
    "format",
    "gameVersion",
    "characterData",
    "starterKit",
    "legacyStarter",
    "rewardCachePlan",
    "storageTokenSeeds",
];

#[derive(Clone, PartialEq)]
pub struct VersionData {
    character_data: CharacterData,
    starter_kit: StarterKit,
    legacy: Option<(String, String)>,
    reward_cache: Option<RewardCachePlan>,
    token_seeds: Option<Vec<String>>,
}

// Counts only, so a log line never carries the values
impl fmt::Debug for VersionData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VersionData")
            .field("fill", &self.character_data.fill.len())
            .field("instanced", &self.starter_kit.instanced.len())
            .field("equipment", &self.starter_kit.equipment.len())
            .field("wallet", &self.starter_kit.wallet.len())
            .field("legacy_starter", &self.legacy.is_some())
            .field("reward_cache_plan", &self.reward_cache.is_some())
            .field("storage_token_seeds", &self.token_seeds.as_ref().map(Vec::len))
            .finish()
    }
}

fn fail(reason: impl fmt::Display) -> String {
    format!("{DATA_FILE}: {reason}")
}

/// Reads members of one object, each failure naming its path.
struct Reader<'a> {
    object: &'a Map<String, Value>,
    path: &'a str,
}

impl<'a> Reader<'a> {
    fn name(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.path)
        }
    }

    fn value(&self, key: &str) -> Result<&'a Value, String> {
        self.object
            .get(key)
            .ok_or_else(|| fail(format!("{} is missing", self.name(key))))
    }

    fn wrong(&self, key: &str, what: &str) -> String {
        fail(format!("{} must be {what}", self.name(key)))
    }

    fn text(&self, key: &str) -> Result<String, String> {
        self.value(key)?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| self.wrong(key, "a text"))
    }

    fn number(&self, key: &str) -> Result<f64, String> {
        self.value(key)?
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| self.wrong(key, "a finite number"))
    }

    fn object(&self, key: &str) -> Result<&'a Map<String, Value>, String> {
        self.value(key)?.as_object().ok_or_else(|| self.wrong(key, "an object"))
    }

    fn array(&self, key: &str) -> Result<&'a Vec<Value>, String> {
        self.value(key)?.as_array().ok_or_else(|| self.wrong(key, "an array"))
    }

    fn texts(&self, key: &str) -> Result<Vec<String>, String> {
        self.array(key)?
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| self.wrong(key, "an array of texts"))
    }

    /// `[[first, second], ...]`, each second read by `second`.
    fn pairs<T>(
        &self,
        key: &str,
        what: &str,
        second: impl Fn(&Value) -> Option<T>,
    ) -> Result<Vec<(String, T)>, String> {
        self.array(key)?
            .iter()
            .map(|entry| pair(entry, &second))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| self.wrong(key, what))
    }

    fn pair_member<T>(
        &self,
        key: &str,
        what: &str,
        second: impl Fn(&Value) -> Option<T>,
    ) -> Result<(String, T), String> {
        pair(self.value(key)?, &second).ok_or_else(|| self.wrong(key, what))
    }
}

fn pair<T>(entry: &Value, second: &impl Fn(&Value) -> Option<T>) -> Option<(String, T)> {
    match entry.as_array()?.as_slice() {
        [first, value] => Some((first.as_str()?.to_owned(), second(value)?)),
        _ => None,
    }
}

fn text_value(v: &Value) -> Option<Value> {
    v.is_string().then(|| v.clone())
}

fn character_data(object: &Map<String, Value>) -> Result<CharacterData, String> {
    let r = Reader {
        object,
        path: "characterData",
    };
    let appearance = r.object("defaultAppearance")?;
    let appearance_reader = Reader {
        object: appearance,
        path: "characterData.defaultAppearance",
    };
    appearance_reader.array("Data")?;
    appearance_reader.array("StringData")?;
    Ok(CharacterData {
        fill: r.pairs("fill", "an array of [key, value] pairs", |v| Some(v.clone()))?,
        default_appearance: appearance.clone(),
        forced_character_flags: r.pairs(
            "forcedCharacterFlags",
            "an array of [flag, value] text pairs",
            text_value,
        )?,
        forced_login_flag: r.pair_member("forcedLoginFlag", "a [flag, value] text pair", text_value)?,
        dojo_series_key: r.text("dojoSeriesKey")?,
        dojo_series: r.object("dojoSeries")?.clone(),
        dojo_node_id: r.text("dojoNodeId")?,
        trigger_series_key: r.text("triggerSeriesKey")?,
        trigger_node: r.text("triggerNode")?,
        trigger_status: r.number("triggerStatus")?,
    })
}

fn starter_kit(object: &Map<String, Value>, appearance: &Map<String, Value>) -> Result<StarterKit, String> {
    let r = Reader {
        object,
        path: "starterKit",
    };
    let kit = StarterKit {
        instanced: r.texts("instanced")?,
        default_weapon_parts: r.texts("defaultWeaponParts")?,
        equipment: r.pairs("equipment", "an array of [slot key, catalog id] pairs", |v| {
            v.as_str().map(str::to_owned)
        })?,
        instance_data: r.pairs("instanceData", "an array of [catalog id, object] pairs", |v| {
            v.as_object().cloned()
        })?,
        flask: r.text("flask")?,
        banner: r.text("banner")?,
        glider: r.text("glider")?,
        banner_customization: r.object("bannerCustomization")?.clone(),
        intro_emote: r.text("introEmote")?,
        flare: r.text("flare")?,
        default_quick_chats: r.texts("defaultQuickChats")?,
        wallet: r.pairs("wallet", "an array of [catalog id, finite number] pairs", |v| {
            v.as_f64().filter(|n| n.is_finite())
        })?,
        default_appearance: appearance.clone(),
    };
    let mut seen = HashSet::new();
    if let Some(dup) = kit.instanced.iter().find(|id| !seen.insert(id.as_str())) {
        return Err(fail(format!("starterKit.instanced repeats {dup}")));
    }
    let instanced = |what: &str, id: &str| {
        if seen.contains(id) {
            Ok(())
        } else {
            Err(fail(format!("starterKit.{what} {id} is not in starterKit.instanced")))
        }
    };
    let mut slots = HashSet::new();
    for (slot, id) in &kit.equipment {
        if slot.is_empty() {
            return Err(fail("starterKit.equipment has an empty slot key"));
        }
        if !slots.insert(slot.as_str()) {
            return Err(fail(format!("starterKit.equipment repeats the slot {slot}")));
        }
        instanced("equipment", id)?;
    }
    instanced("flask", &kit.flask)?;
    instanced("banner", &kit.banner)?;
    instanced("glider", &kit.glider)?;
    for part in &kit.default_weapon_parts {
        instanced("defaultWeaponParts", part)?;
    }
    if kit.default_quick_chats.len() != 9 {
        return Err(fail("starterKit.defaultQuickChats must be exactly nine texts"));
    }
    Ok(kit)
}

impl VersionData {
    /// The document, checked; the first failure is the answer.
    pub fn from_json(value: &Value) -> Result<VersionData, String> {
        let object = value
            .as_object()
            .ok_or_else(|| fail("the document must be an object"))?;
        if let Some(unknown) = object.keys().find(|k| !MEMBERS.contains(&k.as_str())) {
            return Err(fail(format!("unknown member {unknown}")));
        }
        let r = Reader { object, path: "" };
        if r.number("format")? != 1.0 {
            return Err(fail("format must be 1"));
        }
        let game_version = r.text("gameVersion")?;
        if game_version != crate::TABLES.game_version {
            return Err(fail(format!(
                "gameVersion {game_version} is not {}",
                crate::TABLES.game_version
            )));
        }
        let character_data = character_data(r.object("characterData")?)?;
        let starter_kit = starter_kit(r.object("starterKit")?, &character_data.default_appearance)?;
        let legacy = match object.get("legacyStarter") {
            None => None,
            Some(_) => {
                let l = Reader {
                    object: r.object("legacyStarter")?,
                    path: "legacyStarter",
                };
                Some((l.text("defaultDye")?, l.text("defaultSigil")?))
            }
        };
        let reward_cache = match object.get("rewardCachePlan") {
            None => None,
            Some(plan) => Some(RewardCachePlan::from_json(plan).map_err(|e| fail(format!("rewardCachePlan: {e}")))?),
        };
        let token_seeds = match object.get("storageTokenSeeds") {
            None => None,
            Some(_) => Some(r.texts("storageTokenSeeds")?),
        };
        Ok(VersionData {
            character_data,
            starter_kit,
            legacy,
            reward_cache,
            token_seeds,
        })
    }

    /// Core's required read of the backend game data folder's file (its missing-file text), then the checks.
    pub fn load(root: &GameDataRoot) -> Result<VersionData, String> {
        let value = root.load_backend(DATA_FILE).map_err(|e| e.to_string())?;
        Self::from_json(&value)
    }

    pub fn character_data(&self) -> CharacterData {
        self.character_data.clone()
    }

    pub fn starter_kit(&self) -> StarterKit {
        self.starter_kit.clone()
    }

    /// The db tool's starter values, when the document has `legacyStarter`.
    pub fn legacy_starter(&self) -> Option<LegacyStarter> {
        let (dye, sigil) = self.legacy.as_ref()?;
        Some(LegacyStarter::from_kit(&self.starter_kit, dye, sigil))
    }

    /// The generators' inputs, when both generator members are present.
    pub fn generator_inputs(&self) -> Option<VersionInputs> {
        Some(VersionInputs {
            reward_cache: self.reward_cache.clone()?,
            storage_tokens: TokenSource::Seeds(self.token_seeds.clone()?),
        })
    }
}

/// The inputs of a module without a database: every route that needs them answers "no database" first.
pub(crate) fn empty_inputs() -> (CharacterData, StarterKit) {
    let data = CharacterData {
        fill: Vec::new(),
        default_appearance: Map::new(),
        forced_character_flags: Vec::new(),
        forced_login_flag: (String::new(), Value::Null),
        dojo_series_key: String::new(),
        dojo_series: Map::new(),
        dojo_node_id: String::new(),
        trigger_series_key: String::new(),
        trigger_node: String::new(),
        trigger_status: 0.0,
    };
    let kit = StarterKit {
        instanced: Vec::new(),
        default_weapon_parts: Vec::new(),
        equipment: Vec::new(),
        instance_data: Vec::new(),
        flask: String::new(),
        banner: String::new(),
        glider: String::new(),
        banner_customization: Map::new(),
        intro_emote: String::new(),
        flare: String::new(),
        default_quick_chats: Vec::new(),
        wallet: Vec::new(),
        default_appearance: Map::new(),
    };
    (data, kit)
}
