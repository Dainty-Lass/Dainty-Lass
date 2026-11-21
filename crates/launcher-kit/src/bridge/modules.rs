// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The server GUI's module panels (DU3, DD11): what the page may see of a module, and what it may ask of one.
//! A module ships no UI, so everything here is data drawn from its manifest and registration. The checks are
//! pure and compiled into every build (so the default test run covers them); the registry behind them is
//! `server_gui/modules.rs`'s, and a player build has none ([`ModulePanel`] is never managed there).
//!
//! The page is trusted no further than any command's caller: ids are module ids (never a path, DD5), a
//! command's input is JSON text checked against the command's registered arguments, and settings against the
//! keys the module registered with the type of their defaults. What comes back from a module, which may be an
//! external process, is bounded and has its control characters removed before the page sees it.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use module_api::manifest::{ModuleManifest, PatchSetCapability, ThemeCapability, Transport};
use module_api::registrar::{CommandSpec, ConfigSection};
use module_api::{ModuleId, ModuleKind, PortRange};
use serde_json::Value;

use super::types::{
    AdminActionKind, AdminOutcome, ErrorCode, LauncherError, ModuleArgView, ModuleCapabilitiesView,
    ModuleCommandOutcome, ModuleCommandView, ModuleListOutcome, ModuleOrigin, ModuleOutcome, ModulePaletteEntry,
    ModulePatchSetView, ModulePortView, ModuleRole, ModuleRunState, ModuleSectionView, ModuleSettingValue,
    ModuleSettingView, ModuleThemeView, ModuleView,
};

type Boxed<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What `host.toml` holds for one module: `[modules.<id>.<section>]` tables of keys.
pub type Settings = BTreeMap<String, BTreeMap<String, Value>>;

/// The calls the page makes on the server's modules. Implemented over the embedded server's registry
/// (`server_gui`); the arguments are the page's text, checked by the implementation with the functions below.
pub trait ModulePanel: Send + Sync {
    fn list(&self) -> Boxed<ModuleListOutcome>;
    fn get(&self, module: String) -> Boxed<ModuleOutcome>;
    fn run(&self, module: String, command: String, input: String) -> Boxed<ModuleCommandOutcome>;
    fn set_settings(&self, module: String, values: String) -> Boxed<ModuleOutcome>;
    fn load(&self, module: String) -> Boxed<ModuleOutcome>;
    fn unload(&self, module: String) -> Boxed<ModuleOutcome>;
    fn admin(
        &self,
        action: AdminActionKind,
        player: String,
        reason: Option<String>,
        until: Option<String>,
        module: Option<String>,
    ) -> Boxed<AdminOutcome>;
}

/// The longest text one setting holds (host.toml's own bound, `hostconfig/modules.rs`).
pub const VALUE_MAX: usize = 4096;
/// The longest text one command argument holds
pub const ARG_MAX: usize = 1024;
/// What the page is shown of a command's answer, so a chatty module can't fill its memory
pub const OUTPUT_TEXT_MAX: usize = 16 * 1024;
pub const OUTPUT_DATA_MAX: usize = 64 * 1024;
/// A failure's words, shown under the sentence for it
pub const DETAIL_MAX: usize = 300;

pub fn refusal(code: ErrorCode, detail: impl Into<Option<String>>) -> LauncherError {
    LauncherError {
        code,
        detail: detail.into(),
    }
}

/// The module id the page named; anything else is no module, whatever it was.
pub fn module_id(text: &str) -> Result<ModuleId, LauncherError> {
    text.parse().map_err(|_| refusal(ErrorCode::ModuleUnknown, None))
}

/// A line of a module's words for the page: control characters become spaces, at most `max` characters.
pub fn plain(text: &str, max: usize) -> String {
    let mut out: String = text
        .chars()
        .take(max)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    out.truncate(out.trim_end().len());
    out
}

/// Text for the command output: line breaks and tabs stay, other control characters go.
pub fn output_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(OUTPUT_TEXT_MAX)
        .collect()
}

/// A name as the manifest's rules make them, safe to put in a detail
fn is_name(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

fn input_error(reason: &str, name: Option<&str>) -> LauncherError {
    let detail = match name {
        Some(name) if is_name(name) => format!("{reason}:{name}"),
        _ => reason.to_owned(),
    };
    refusal(ErrorCode::ModuleInputInvalid, detail)
}

/// A command's arguments, in the order it registered them, from the form's JSON (`{ "<argument>": "<text>" }`,
/// or null when it has none). The console passes text positionally; so does this: an argument left out
/// between two given is refused, since a module reads a position, not a name.
pub fn command_args(spec: &CommandSpec, input: &str) -> Result<Vec<String>, LauncherError> {
    let value: Value = serde_json::from_str(input).map_err(|_| input_error("json", None))?;
    let empty = serde_json::Map::new();
    let given = match &value {
        Value::Null => &empty,
        Value::Object(object) => object,
        _ => return Err(input_error("shape", None)),
    };
    if let Some(unknown) = given.keys().find(|key| !spec.args.iter().any(|arg| &arg.name == *key)) {
        return Err(input_error("unknown_argument", Some(unknown)));
    }
    let mut args = Vec::new();
    let mut gap: Option<&str> = None;
    for arg in &spec.args {
        let text = match given.get(&arg.name) {
            None | Some(Value::Null) => "",
            Some(Value::String(text)) => text,
            Some(_) => return Err(input_error("argument_text", Some(&arg.name))),
        };
        if text.chars().count() > ARG_MAX || text.chars().any(char::is_control) {
            return Err(input_error("argument_text", Some(&arg.name)));
        }
        if text.is_empty() {
            if arg.required {
                return Err(input_error("missing_argument", Some(&arg.name)));
            }
            gap.get_or_insert(&arg.name);
            continue;
        }
        if let Some(gap) = gap {
            return Err(input_error("argument_gap", Some(gap)));
        }
        args.push(text.to_owned());
    }
    Ok(args)
}

/// A setting the page can edit: text, a number or on/off. A module may register any JSON as a default;
/// the rest has no field here.
fn scalar(value: &Value) -> Option<ModuleSettingValue> {
    match value {
        Value::Bool(flag) => Some(ModuleSettingValue::Flag(*flag)),
        Value::Number(number) => number.as_f64().map(ModuleSettingValue::Number),
        Value::String(text) if text.chars().count() <= VALUE_MAX && !text.contains('\0') => {
            Some(ModuleSettingValue::Text(text.clone()))
        }
        _ => None,
    }
}

fn settings_error(reason: &str, place: Option<(&str, &str)>) -> LauncherError {
    let detail = match place {
        Some((section, key)) => format!("{reason}:{section}.{key}"),
        None => reason.to_owned(),
    };
    refusal(ErrorCode::ModuleSettingsInvalid, detail)
}

/// Whether a value has the type of the key's default; an integer default takes an integer.
fn same_type(default: &Value, new: &Value) -> bool {
    match (default, new) {
        (Value::Bool(_), Value::Bool(_)) | (Value::String(_), Value::String(_)) => true,
        (Value::Number(default), Value::Number(new)) => {
            new.as_f64().is_some_and(f64::is_finite)
                && (!(default.is_i64() || default.is_u64()) || new.is_i64() || new.is_u64())
        }
        _ => false,
    }
}

/// `saved` with the page's values laid over it, each checked against what the module registered: a known
/// section and key, and the type of the key's default. Nothing is returned unless every value fits.
/// `schema` is empty until the module loaded, so then nothing fits.
pub fn merge_settings(schema: &[ConfigSection], saved: &Settings, values: &str) -> Result<Settings, LauncherError> {
    let value: Value = serde_json::from_str(values).map_err(|_| settings_error("json", None))?;
    let Value::Object(sections) = value else {
        return Err(settings_error("shape", None));
    };
    let mut merged = saved.clone();
    for (section_name, keys) in sections {
        let Value::Object(keys) = keys else {
            return Err(settings_error("shape", None));
        };
        let section = schema
            .iter()
            .find(|section| section.name == section_name)
            .ok_or_else(|| settings_error("unknown_section", None))?;
        for (key_name, new) in keys {
            let place = Some((section_name.as_str(), key_name.as_str()));
            let key = section
                .keys
                .iter()
                .find(|key| key.name == key_name)
                .ok_or_else(|| settings_error("unknown_key", place))?;
            if !same_type(&key.default, &new) {
                return Err(settings_error("type", place));
            }
            if let Value::String(text) = &new
                && (text.chars().count() > VALUE_MAX || text.contains('\0'))
            {
                return Err(settings_error("length", place));
            }
            merged.entry(section_name.clone()).or_default().insert(key_name, new);
        }
    }
    Ok(merged)
}

/// What the registry says about one listed module, in the types this file draws from.
pub struct Listed {
    pub id: ModuleId,
    pub origin: ModuleOrigin,
    pub autoload: bool,
    pub state: ModuleRunState,
    pub detail: Option<String>,
    pub manifest: Option<ModuleManifest>,
    pub ports: BTreeMap<String, PortRange>,
    pub commands: Vec<CommandSpec>,
    pub config: Vec<ConfigSection>,
}

fn token_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// A colour a plugin's palette may set: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, or a colour function with
/// plain numbers in it. No nested function, `url(`, `var(`, quote, `;`, `{` or backslash can get through,
/// since the page sets it as a CSS variable's value.
pub fn is_colour(text: &str) -> bool {
    if let Some(hex) = text.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    let Some((name, rest)) = text.split_once('(') else {
        return false;
    };
    let Some(body) = rest.strip_suffix(')') else {
        return false;
    };
    matches!(
        name,
        "rgb" | "rgba" | "hsl" | "hsla" | "oklch" | "oklab" | "lab" | "lch"
    ) && text.len() <= 64
        && !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'.' | b',' | b'%' | b'/' | b'-' | b'+'))
}

fn theme(theme: &ThemeCapability) -> ModuleThemeView {
    ModuleThemeView {
        id: plain(&theme.id, 64),
        display_name: plain(&theme.display_name, 64),
        palette: theme
            .palette
            .iter()
            .filter(|(name, value)| token_name(name) && is_colour(value))
            .map(|(name, value)| ModulePaletteEntry {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
        layout: theme.layout.as_deref().map(|layout| plain(layout, 64)),
        logo: theme.logo.as_deref().map(|logo| plain(logo, 260)),
    }
}

fn capabilities(manifest: Option<&ModuleManifest>) -> ModuleCapabilitiesView {
    let Some(manifest) = manifest else {
        return ModuleCapabilitiesView {
            realms: false,
            network: false,
            chat: false,
            patch_set: None,
            theme: None,
        };
    };
    let caps = &manifest.capabilities;
    ModuleCapabilitiesView {
        realms: caps.realms,
        network: caps.network,
        chat: caps.chat,
        patch_set: caps.patch_set.as_ref().map(|set| match set {
            PatchSetCapability::Accepts => ModulePatchSetView {
                provides: false,
                display_name: None,
            },
            PatchSetCapability::Provides(set) => ModulePatchSetView {
                provides: true,
                display_name: Some(plain(&set.display_name, 64)),
            },
        }),
        theme: caps.theme.as_ref().map(theme),
    }
}

fn setting_sections(config: &[ConfigSection], saved: &Settings) -> Vec<ModuleSectionView> {
    config
        .iter()
        .filter_map(|section| {
            let keys: Vec<ModuleSettingView> = section
                .keys
                .iter()
                .filter_map(|key| {
                    let default = scalar(&key.default)?;
                    // A value of another type than the default's is not shown; the default is
                    let value = saved
                        .get(&section.name)
                        .and_then(|keys| keys.get(&key.name))
                        .filter(|value| same_type(&key.default, value))
                        .and_then(scalar)
                        .unwrap_or_else(|| default.clone());
                    Some(ModuleSettingView {
                        name: key.name.clone(),
                        help: plain(&key.help, 256),
                        default,
                        value,
                    })
                })
                .collect();
            (!keys.is_empty()).then(|| ModuleSectionView {
                name: section.name.clone(),
                keys,
            })
        })
        .collect()
}

/// The panel's view of a module; `saved` is what host.toml sets for it.
pub fn view(listed: &Listed, saved: &Settings) -> ModuleView {
    let manifest = listed.manifest.as_ref();
    ModuleView {
        id: listed.id.to_string(),
        display_name: manifest.map(|manifest| plain(&manifest.display_name, 64)),
        role: manifest.map(|manifest| match manifest.kind {
            ModuleKind::Game => ModuleRole::Game,
            ModuleKind::Plugin => ModuleRole::Plugin,
        }),
        game_version: manifest.map(|manifest| plain(&manifest.game_version.to_string(), 64)),
        origin: listed.origin,
        autoload: listed.autoload,
        state: listed.state,
        detail: listed
            .detail
            .as_deref()
            .map(|detail| plain(detail, DETAIL_MAX))
            .filter(|detail| !detail.is_empty()),
        unverified: manifest.is_some_and(|manifest| manifest.unverified),
        ports: listed
            .ports
            .iter()
            .map(|(name, range)| ModulePortView {
                name: plain(name, 64),
                transport: match manifest
                    .and_then(|manifest| manifest.port(name))
                    .map(|port| port.transport)
                {
                    Some(Transport::Udp) => "udp",
                    _ => "tcp",
                }
                .to_owned(),
                first: range.first,
                last: range.last,
            })
            .collect(),
        capabilities: capabilities(manifest),
        commands: listed
            .commands
            .iter()
            .map(|command| ModuleCommandView {
                name: command.name.clone(),
                summary: plain(&command.summary, 256),
                args: command
                    .args
                    .iter()
                    .map(|arg| ModuleArgView {
                        name: arg.name.clone(),
                        required: arg.required,
                        help: plain(&arg.help, 256),
                    })
                    .collect(),
            })
            .collect(),
        settings: setting_sections(&listed.config, saved),
    }
}

#[cfg(test)]
mod tests {
    use module_api::manifest::{Capabilities, PatchSet};
    use module_api::stub::StubModule;
    use module_api::{GameModule, Registrar};
    use serde_json::json;

    use super::*;

    fn spec() -> CommandSpec {
        CommandSpec::new("give", "Gives an item")
            .arg("account", true, "Who")
            .arg("item", false, "What")
            .arg("count", false, "How many")
    }

    fn code(error: &LauncherError) -> (ErrorCode, &str) {
        (error.code, error.detail.as_deref().unwrap_or(""))
    }

    #[test]
    fn a_form_becomes_positional_arguments() {
        let args = |input: &str| command_args(&spec(), input);
        assert_eq!(args(r#"{"account":"a1"}"#).unwrap(), ["a1"]);
        assert_eq!(args(r#"{"account":"a1","item":"sword"}"#).unwrap(), ["a1", "sword"]);
        assert_eq!(args(r#"{"count":"","item":"x","account":"a1"}"#).unwrap(), ["a1", "x"]);
        let ping = CommandSpec::new("ping", "Pings");
        assert_eq!(command_args(&ping, "null").unwrap(), Vec::<String>::new());
        assert_eq!(command_args(&ping, "{}").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn input_that_does_not_fit_the_command_is_refused_with_why() {
        let refused = |input: &str| command_args(&spec(), input).unwrap_err();
        let invalid = ErrorCode::ModuleInputInvalid;
        assert_eq!(code(&refused("{")), (invalid, "json"));
        assert_eq!(code(&refused("[]")), (invalid, "shape"));
        assert_eq!(code(&refused("\"a\"")), (invalid, "shape"));
        assert_eq!(code(&refused("{}")), (invalid, "missing_argument:account"));
        assert_eq!(
            code(&refused(r#"{"account":""}"#)),
            (invalid, "missing_argument:account")
        );
        assert_eq!(
            code(&refused(r#"{"account":"a","extra":"b"}"#)),
            (invalid, "unknown_argument:extra")
        );
        assert_eq!(
            code(&refused(r#"{"account":"a","Evil Name":"b"}"#)),
            (invalid, "unknown_argument")
        );
        assert_eq!(code(&refused(r#"{"account":7}"#)), (invalid, "argument_text:account"));
        assert_eq!(
            code(&refused(r#"{"account":"a\nb"}"#)),
            (invalid, "argument_text:account")
        );
        assert_eq!(
            code(&refused(r#"{"account":"a","count":"3"}"#)),
            (invalid, "argument_gap:item")
        );
        let long = json!({ "account": "x".repeat(ARG_MAX + 1) }).to_string();
        assert_eq!(code(&refused(&long)), (invalid, "argument_text:account"));
        let fits = json!({ "account": "x".repeat(ARG_MAX) }).to_string();
        assert!(command_args(&spec(), &fits).is_ok());
    }

    fn schema() -> Vec<ConfigSection> {
        vec![
            ConfigSection::new("game")
                .key("greeting", json!("hello"), "What echo starts with")
                .key("slots", json!(8), "Hunt slots")
                .key("rate", json!(1.5), "A rate")
                .key("pvp", json!(false), "Player fights")
                .key("tags", json!(["a"]), "A list the page can't edit"),
            ConfigSection::new("empty").key("list", json!([1]), "Nothing editable"),
        ]
    }

    #[test]
    fn settings_are_checked_against_what_the_module_registered() {
        let saved = Settings::from([("game".into(), BTreeMap::from([("greeting".into(), json!("hi"))]))]);
        let merged = merge_settings(
            &schema(),
            &saved,
            r#"{"game":{"slots":12,"rate":2,"pvp":true,"greeting":"hey"}}"#,
        )
        .unwrap();
        assert_eq!(
            merged["game"],
            BTreeMap::from([
                ("greeting".to_string(), json!("hey")),
                ("slots".to_string(), json!(12)),
                ("rate".to_string(), json!(2)),
                ("pvp".to_string(), json!(true)),
            ])
        );
        let refused = |values: &str| merge_settings(&schema(), &saved, values).unwrap_err();
        let invalid = ErrorCode::ModuleSettingsInvalid;
        assert_eq!(code(&refused("nope")), (invalid, "json"));
        assert_eq!(code(&refused("[]")), (invalid, "shape"));
        assert_eq!(code(&refused(r#"{"game":1}"#)), (invalid, "shape"));
        assert_eq!(code(&refused(r#"{"other":{}}"#)), (invalid, "unknown_section"));
        assert_eq!(
            code(&refused(r#"{"game":{"nope":1}}"#)),
            (invalid, "unknown_key:game.nope")
        );
        assert_eq!(
            code(&refused(r#"{"game":{"slots":"12"}}"#)),
            (invalid, "type:game.slots")
        );
        assert_eq!(
            code(&refused(r#"{"game":{"slots":1.5}}"#)),
            (invalid, "type:game.slots")
        );
        assert_eq!(code(&refused(r#"{"game":{"pvp":"yes"}}"#)), (invalid, "type:game.pvp"));
        assert_eq!(
            code(&refused(r#"{"game":{"greeting":null}}"#)),
            (invalid, "type:game.greeting")
        );
        assert_eq!(
            code(&refused(r#"{"game":{"tags":["b"]}}"#)),
            (invalid, "type:game.tags")
        );
        let long = json!({ "game": { "greeting": "x".repeat(VALUE_MAX + 1) } }).to_string();
        assert_eq!(code(&refused(&long)), (invalid, "length:game.greeting"));
        // One bad value saves none of them
        assert!(merge_settings(&schema(), &saved, r#"{"game":{"slots":3,"pvp":"x"}}"#).is_err());
        // A module that has not loaded has no schema, so nothing fits it
        assert!(merge_settings(&[], &saved, r#"{"game":{"slots":3}}"#).is_err());
        assert_eq!(merge_settings(&[], &saved, "{}").unwrap(), saved);
    }

    #[test]
    fn colours_cannot_carry_anything_else() {
        for ok in [
            "#fff",
            "#ffcc00",
            "#FFCC0080",
            "oklch(0.7 0.1 250)",
            "rgb(10, 20, 30)",
            "hsl(120deg 50% 50% / 0.5)",
        ] {
            assert!(is_colour(ok), "{ok}");
        }
        for bad in [
            "red",
            "#ff",
            "#gggggg",
            "url(x)",
            "var(--x)",
            "rgb(var(--x))",
            "rgb(1,2,3);color:red",
            "rgb(1,2,3)}",
            "calc(1px)",
            "rgb(1,2,3) !important",
            "rgb(\\31)",
            "rgb(1,2,3",
            "",
        ] {
            assert!(!is_colour(bad), "{bad}");
        }
    }

    fn listed(manifest: Option<ModuleManifest>) -> Listed {
        let stub = StubModule::new();
        let mut registrar = Registrar::new(stub.manifest());
        stub.register(&mut registrar).unwrap();
        let registration = registrar.finish().unwrap();
        Listed {
            id: "stub".parse().unwrap(),
            origin: ModuleOrigin::CompiledIn,
            autoload: true,
            state: ModuleRunState::Ready,
            detail: Some("up\nand running".into()),
            manifest,
            ports: BTreeMap::from([("http".into(), PortRange::single(65001))]),
            commands: registration.commands,
            config: registration.config,
        }
    }

    #[test]
    fn a_module_is_drawn_from_its_manifest_and_registration() {
        let stub = StubModule::new().manifest();
        let drawn = view(&listed(Some(stub)), &Settings::new());
        assert_eq!(drawn.id, "stub");
        assert_eq!(drawn.display_name.as_deref(), Some("Stub"));
        assert_eq!(drawn.game_version.as_deref(), Some("0.0.1"));
        assert_eq!(drawn.role, Some(ModuleRole::Game));
        assert!(drawn.unverified);
        assert_eq!(drawn.detail.as_deref(), Some("up and running"));
        assert_eq!(drawn.ports.len(), 1);
        assert_eq!(
            (drawn.ports[0].transport.as_str(), drawn.ports[0].first),
            ("tcp", 65001)
        );
        assert!(drawn.capabilities.chat);
        assert_eq!(
            drawn.capabilities.patch_set,
            Some(ModulePatchSetView {
                provides: false,
                display_name: None
            })
        );
        let echo = drawn.commands.iter().find(|c| c.name == "echo").unwrap();
        assert_eq!((echo.args[0].name.as_str(), echo.args[0].required), ("words", false));
        assert_eq!(drawn.settings.len(), 1);
        let key = &drawn.settings[0].keys[0];
        let hello = ModuleSettingValue::Text("hello".into());
        assert_eq!(
            (key.name.as_str(), &key.default, &key.value),
            ("greeting", &hello, &hello)
        );
        // host.toml's value shows once set, and one of the wrong type falls back to the default
        let saved = Settings::from([("stub".into(), BTreeMap::from([("greeting".into(), json!("hi"))]))]);
        let again = view(&listed(None), &saved);
        assert_eq!(again.settings[0].keys[0].value, ModuleSettingValue::Text("hi".into()));
        let wrong = Settings::from([("stub".into(), BTreeMap::from([("greeting".into(), json!(3))]))]);
        assert_eq!(view(&listed(None), &wrong).settings[0].keys[0].value, hello);
        // Without a manifest nothing is claimed of the module
        let bare = view(&listed(None), &Settings::new());
        assert_eq!(
            (bare.display_name, bare.role, bare.game_version, bare.unverified),
            (None, None, None, false)
        );
    }

    #[test]
    fn a_plugin_theme_is_drawn_with_the_colours_that_are_safe() {
        let mut manifest = StubModule::new().manifest();
        manifest.kind = ModuleKind::Plugin;
        manifest.capabilities = Capabilities {
            patch_set: Some(PatchSetCapability::Provides(PatchSet {
                id: "custom".into(),
                display_name: "Custom tuning".into(),
                runtime_patches: vec![],
                backend_rules: vec![],
            })),
            theme: Some(ThemeCapability {
                id: "ember".into(),
                display_name: "Ember".into(),
                palette: BTreeMap::from([
                    ("primary".into(), "#ff8800".into()),
                    ("background".into(), "oklch(0.2 0.02 260)".into()),
                    ("Bad Name".into(), "#fff".into()),
                    ("ring".into(), "url(https://x.test/a.png)".into()),
                ]),
                layout: Some("compact".into()),
                logo: Some("client/logo.png".into()),
            }),
            ..Capabilities::default()
        };
        let drawn = view(&listed(Some(manifest)), &Settings::new());
        assert_eq!(drawn.role, Some(ModuleRole::Plugin));
        let theme = drawn.capabilities.theme.unwrap();
        assert_eq!(
            theme.palette.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["background", "primary"]
        );
        assert_eq!(
            (theme.layout.as_deref(), theme.logo.as_deref()),
            (Some("compact"), Some("client/logo.png"))
        );
        assert_eq!(
            drawn.capabilities.patch_set,
            Some(ModulePatchSetView {
                provides: true,
                display_name: Some("Custom tuning".into())
            })
        );
    }

    #[test]
    fn module_words_are_bounded_and_lose_their_control_characters() {
        assert_eq!(plain("a\r\nb\u{1b}[31m", 100), "a  b [31m");
        assert_eq!(plain("abcdef", 3), "abc");
        assert_eq!(plain("trailing  \n", 100), "trailing");
        assert_eq!(output_text("one\ntwo\u{7}\t"), "one\ntwo\t");
        assert_eq!(output_text(&"x".repeat(OUTPUT_TEXT_MAX + 5)).len(), OUTPUT_TEXT_MAX);
        assert_eq!(code(&module_id("../etc").unwrap_err()), (ErrorCode::ModuleUnknown, ""));
        assert_eq!(
            code(&module_id("C:\\x.exe").unwrap_err()),
            (ErrorCode::ModuleUnknown, "")
        );
        assert!(module_id("rustic-isles").is_ok());
    }
}
