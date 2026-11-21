// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the bot says, from `catalog/en.json` and `catalog/pt-BR.json` (DU2): whole sentences with `{placeholders}`,
//! chosen by the interaction's locale. Rust picks a key, never builds a sentence.

use std::collections::BTreeMap;

use dainty_module_api::game::UnavailableReason;

use crate::interaction::Language;

const EN: &str = include_str!("../catalog/en.json");
const PT_BR: &str = include_str!("../catalog/pt-BR.json");

/// Both catalogs, read once.
#[derive(Debug, Clone)]
pub struct Catalog {
    en: BTreeMap<String, String>,
    pt_br: BTreeMap<String, String>,
}

impl Catalog {
    /// The catalogs compiled in; fails only if one is not a JSON object of strings (a test keeps them so).
    pub fn load() -> Result<Self, String> {
        let read = |name: &str, text: &str| {
            serde_json::from_str::<BTreeMap<String, String>>(text).map_err(|e| format!("the {name} catalog: {e}"))
        };
        Ok(Self {
            en: read("en", EN)?,
            pt_br: read("pt-BR", PT_BR)?,
        })
    }

    /// `key` in `language` (English when that catalog lacks it), with each `{name}` replaced once.
    pub fn text(&self, language: Language, key: &str, values: &[(&str, &str)]) -> String {
        let table = match language {
            Language::En => &self.en,
            Language::PtBr => &self.pt_br,
        };
        let Some(template) = table.get(key).or_else(|| self.en.get(key)) else {
            return key.to_owned();
        };
        fill(template, values)
    }

    /// The text for a reason the game module gave, or `reason.unknown`.
    pub fn reason(&self, language: Language, reason: Option<UnavailableReason>) -> String {
        let key = reason.map_or_else(|| "reason.unknown".to_owned(), |r| format!("reason.{}", r.code()));
        self.text(language, &key, &[])
    }

    /// `key` in Portuguese, for a command's `description_localizations`.
    pub fn pt_br(&self, key: &str) -> Option<&str> {
        self.pt_br.get(key).map(String::as_str)
    }
}

/// One pass over `template`: a value is never searched for placeholders itself.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match values.iter().find(|(n, _)| *n == name) {
                    Some((_, value)) => out.push_str(value),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::collections::BTreeSet;

    use super::*;

    fn placeholders(text: &str) -> BTreeSet<String> {
        text.split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name.to_owned()))
            .collect()
    }

    #[test]
    fn both_catalogs_have_the_same_keys_and_placeholders() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(
            catalog.en.keys().collect::<Vec<_>>(),
            catalog.pt_br.keys().collect::<Vec<_>>()
        );
        for (key, en) in &catalog.en {
            let pt = &catalog.pt_br[key];
            assert_eq!(placeholders(en), placeholders(pt), "{key}");
            assert_ne!(en, pt, "{key} is untranslated");
        }
        for reason in [
            UnavailableReason::DirectorUnreachable,
            UnavailableReason::NoHub,
            UnavailableReason::HubNotReady,
            UnavailableReason::ModuleNotReady,
        ] {
            let key = format!("reason.{}", reason.code());
            assert!(catalog.en.contains_key(&key), "{key}");
        }
        // Discord refuses a command description over 100 characters
        for key in ["command.key.description", "command.status.description"] {
            assert!(catalog.en[key].chars().count() <= 100 && catalog.pt_br[key].chars().count() <= 100);
        }
    }

    #[test]
    fn fills_each_placeholder_once() {
        assert_eq!(fill("a {x} b {y} {z}", &[("x", "{y}"), ("y", "2")]), "a {y} b 2 {z}");
        assert_eq!(fill("open { brace", &[]), "open { brace");
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.text(Language::PtBr, "no.such.key", &[]), "no.such.key");
        assert!(catalog.reason(Language::PtBr, None).starts_with("Não"));
    }
}
