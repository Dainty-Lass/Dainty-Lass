// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The version data (spec §4.3) in two files (port plan item 9, rulings V-1 to V-3): `version_facts.json`, the
//! game's identifiers and GUIDs, and `version_data.json`, the design (the reward cache plan, the starter wallet, the
//! legacy starter and the default appearance), this project's own since restricted's was retired; both MIT, in
//! `data/` beside this crate.
//! The host installs both in the backend game data folder; a design file that still holds everything (a release
//! before the split) is read alone.

use std::collections::HashSet;
use std::fmt;

use rustic_isles::GameDataRoot;
use rustic_isles_characters::{CharacterData, StarterKit};
use rustic_isles_db::LegacyStarter;
use rustic_isles_gamedata::{RewardCachePlan, TokenSource, VersionInputs};
use serde_json::{Map, Value};

/// The design file's name in the backend game data folder (before the split, the whole document).
pub const DATA_FILE: &str = "version_data.json";

/// The facts file's name in the backend game data folder.
pub const FACTS_FILE: &str = "version_facts.json";

/// The facts this crate ships for 1.14.7 (V-1, V-3), as a release installs them.
pub const FACTS_DOCUMENT: &str = include_str!("../data/version_facts.json");

const MEMBERS: [&str; 7] = [
    "format",
    "gameVersion",
    "characterData",
    "starterKit",
    "legacyStarter",
    "rewardCachePlan",
    "storageTokenSeeds",
];

/// 1.14.7's checked `version_data.json`: character data, the starter kit and the reward and token tables.
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

    /// The facts and the design merged ([`merge`]) and checked; no facts reads the design as the whole document.
    pub fn from_parts(facts: Option<&Value>, design: &Value) -> Result<VersionData, String> {
        match facts {
            Some(facts) => Self::from_json(&merge(facts, design)?),
            None => Self::from_json(design),
        }
    }

    /// Core's required read of the backend game data folder's design file (its missing-file text) and the facts
    /// beside it, then the checks.
    pub fn load(root: &GameDataRoot) -> Result<VersionData, String> {
        let design = root.load_backend(DATA_FILE).map_err(|e| e.to_string())?;
        let facts = root.try_load_backend(FACTS_FILE).map_err(|e| e.to_string())?;
        if facts.is_none() && design.pointer("/starterKit/instanced").is_none() {
            // A design file without its facts: core's missing-file text for the facts
            return Err(root
                .load_backend(FACTS_FILE)
                .err()
                .map_or_else(|| fail(format!("{FACTS_FILE} is missing")), |e| e.to_string()));
        }
        Self::from_parts(facts.as_ref(), &design)
    }

    /// The character data new characters start from.
    pub fn character_data(&self) -> CharacterData {
        self.character_data.clone()
    }

    /// The kit a new account is granted.
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

fn fail_facts(reason: impl fmt::Display) -> String {
    format!("{FACTS_FILE}: {reason}")
}

/// One document from the facts and the design: objects merge member by member, the design's value winning, and
/// `characterData.fill`'s pairs are replaced by key where they stand (the facts hold `["AppearanceData", null]`,
/// the design its text). A fill value still null afterwards is refused.
pub fn merge(facts: &Value, design: &Value) -> Result<Value, String> {
    let (Some(f), Some(d)) = (facts.as_object(), design.as_object()) else {
        return Err(if facts.is_object() {
            fail("the document must be an object")
        } else {
            fail_facts("the document must be an object")
        });
    };
    for key in ["format", "gameVersion"] {
        if let (Some(a), Some(b)) = (f.get(key), d.get(key))
            && a != b
        {
            return Err(fail_facts(format!("{key} {a} differs from {DATA_FILE}'s {b}")));
        }
    }
    let mut merged = merge_objects(f, d);
    if let Some(Value::Array(fill)) = merged
        .get_mut("characterData")
        .and_then(Value::as_object_mut)
        .and_then(|c| c.get_mut("fill"))
    {
        if let Some(Value::Array(over)) = d.get("characterData").and_then(|c| c.get("fill")) {
            *fill = merge_pairs(f.get("characterData").and_then(|c| c.get("fill")), over);
        }
        if let Some(key) = fill.iter().find_map(|entry| match entry.as_array().map(Vec::as_slice) {
            Some([key, Value::Null]) => Some(key.as_str().unwrap_or("?").to_owned()),
            _ => None,
        }) {
            return Err(fail(format!(
                "characterData.fill {key} has no value (the design supplies it)"
            )));
        }
    }
    Ok(Value::Object(merged))
}

fn merge_objects(facts: &Map<String, Value>, design: &Map<String, Value>) -> Map<String, Value> {
    let mut out = facts.clone();
    for (key, value) in design {
        let merged = match (out.get(key), value) {
            (Some(Value::Object(a)), Value::Object(b)) => Value::Object(merge_objects(a, b)),
            _ => value.clone(),
        };
        out.insert(key.clone(), merged);
    }
    out
}

fn merge_pairs(facts: Option<&Value>, design: &[Value]) -> Vec<Value> {
    let key = |entry: &Value| entry.as_array().and_then(|p| p.first()).cloned();
    let mut out: Vec<Value> = facts.and_then(Value::as_array).cloned().unwrap_or_default();
    for entry in design {
        match out.iter_mut().find(|e| key(e).is_some() && key(e) == key(entry)) {
            Some(slot) => *slot = entry.clone(),
            None => out.push(entry.clone()),
        }
    }
    out
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
