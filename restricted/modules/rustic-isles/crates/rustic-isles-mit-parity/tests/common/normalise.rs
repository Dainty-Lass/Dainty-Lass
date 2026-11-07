//! The recorders' normalisers, which make a golden's volatile parts comparable (from isles-core's golden rigs,
//! retired for the MIT module, RD27).

use std::collections::HashMap;

use regex::Regex;

/// The characters golden's: every uuid and instance id becomes a number by first appearance (so which ones are the
/// same still matters), and the clock-bound dates a fixed word.
pub struct Normalizer {
    ids: HashMap<String, usize>,
    patterns: [(Regex, &'static str); 4],
}

impl Default for Normalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Normalizer {
    pub fn new() -> Self {
        let regex = |p: &str| Regex::new(p).expect("a valid pattern");
        Self {
            ids: HashMap::new(),
            patterns: [
                (regex(r"\b\d{4}\.\d{2}\.\d{2}-\d{2}\.\d{2}\.\d{2}\b"), "<time>"),
                (
                    regex(r"\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{1,2}, \d{4}\b"),
                    "<date>",
                ),
                (
                    regex(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b"),
                    "uuid",
                ),
                (regex(r"\b[A-Z0-9]{26}\b"), "iid"),
            ],
        }
    }

    pub fn apply(&mut self, text: &str) -> String {
        let mut out = text.to_owned();
        let ids = &mut self.ids;
        for (pattern, label) in &self.patterns {
            out = pattern
                .replace_all(&out, |caps: &regex::Captures<'_>| {
                    if label.starts_with('<') {
                        return (*label).to_owned();
                    }
                    let next = ids.len();
                    let n = *ids.entry(caps[0].to_owned()).or_insert(next);
                    format!("<{label}{n}>")
                })
                .into_owned();
        }
        out
    }
}

/// The inventory recorder's (`record.cjs`): timestamps, uuids and instance ids.
pub fn normalize_inventory(text: &str) -> String {
    let ts = Regex::new(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z").unwrap_or_else(|e| panic!("{e}"));
    let uuid =
        Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap_or_else(|e| panic!("{e}"));
    let id = Regex::new(r"[A-Z2-7]{26}").unwrap_or_else(|e| panic!("{e}"));
    let text = ts.replace_all(text, "<TS>");
    let text = uuid.replace_all(&text, "<UUID>");
    id.replace_all(&text, "<ID>").into_owned()
}

/// The progression capture's: dates and minted grant ids.
pub fn normalise_progression(text: &str) -> String {
    let date = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|\+00:00)");
    let grant = Regex::new(r"gs-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}");
    let text = match date {
        Ok(date) => date.replace_all(text, "<date>").into_owned(),
        Err(_) => text.to_owned(),
    };
    match grant {
        Ok(grant) => grant.replace_all(&text, "gs-<uuid>").into_owned(),
        Err(_) => text,
    }
}
