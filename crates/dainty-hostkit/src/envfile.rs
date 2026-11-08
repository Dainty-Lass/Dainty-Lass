// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `.env` files: read into ordered variables, written from them, and edited in place keeping the comments, blank lines,
//! line endings and BOM a host left there.

use crate::error::HostError;
use dainty_service_kit::js;

/// An env file's variables in the order first set; a later line for a key replaces its value in place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvMap(Vec<(String, String)>);

impl EnvMap {
    /// No variables.
    #[must_use]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// The value of `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Sets `key`, in its first place if it was set before, else at the end.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if let Some((_, existing)) = self.0.iter_mut().find(|(k, _)| *k == key) {
            *existing = value;
        } else {
            self.0.push((key, value));
        }
    }

    /// Each variable as (key, value), in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// How many variables there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl IntoIterator for EnvMap {
    type Item = (String, String);
    type IntoIter = std::vec::IntoIter<(String, String)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a EnvMap {
    type Item = &'a (String, String);
    type IntoIter = std::slice::Iter<'a, (String, String)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl FromIterator<(String, String)> for EnvMap {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(iter: T) -> Self {
        let mut map = Self::new();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

/// Whether `key` can be an env variable: a letter or `_`, then letters, digits and `_`.
#[must_use]
pub fn valid_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn unquote_double(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn strip_inline_comment(value: &str) -> &str {
    let mut prev_space = false;
    for (idx, ch) in value.char_indices() {
        if ch == '#' && prev_space {
            return js::trim(&value[..idx]);
        }
        prev_space = js::is_space(ch);
    }
    js::trim(value)
}

fn parse_line(raw_line: &str) -> Option<(String, String)> {
    let trimmed = js::trim(raw_line);
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let body = if let Some(after_export) = trimmed.strip_prefix("export") {
        if after_export.chars().next().is_some_and(js::is_space) {
            js::trim(after_export)
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    let (raw_key, raw_val) = body.split_once('=')?;
    let key = js::trim(raw_key);
    if !valid_key(key) {
        return None;
    }
    let val_trimmed = js::trim(raw_val);
    let value = if val_trimmed.len() >= 2 && val_trimmed.starts_with('"') && val_trimmed.ends_with('"') {
        unquote_double(&val_trimmed[1..val_trimmed.len() - 1])
    } else if val_trimmed.len() >= 2 && val_trimmed.starts_with('\'') && val_trimmed.ends_with('\'') {
        val_trimmed[1..val_trimmed.len() - 1].to_owned()
    } else {
        strip_inline_comment(val_trimmed).to_owned()
    };
    Some((key.to_owned(), value))
}

/// The variables of an env file's text; a BOM, comments, blank lines and malformed lines are skipped.
#[must_use]
pub fn parse_env(text: &str) -> EnvMap {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut map = EnvMap::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some((k, v)) = parse_line(line) {
            map.insert(k, v);
        }
    }
    map
}

fn is_plain_value_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b':' | b'@' | b',' | b'+' | b'=' | b'%' | b'-')
}

/// `value` as an env file writes it: bare when it is plain, else double-quoted with `\` and `"` escaped.
/// Fails on a line break, which no env file can hold.
pub fn format_value(value: &str) -> Result<String, HostError> {
    if value.contains('\n') || value.contains('\r') {
        return Err(HostError::message("env values cannot contain line breaks"));
    }
    if value.is_empty() || value.bytes().all(is_plain_value_byte) {
        return Ok(value.to_owned());
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

/// An env file's text: `header` as comment lines, a blank line, then each `KEY=value`; fails on an invalid key or
/// value.
pub fn format_env<K: AsRef<str>, V: AsRef<str>>(header: &[&str], entries: &[(K, V)]) -> Result<String, HostError> {
    let mut lines = Vec::new();
    for h in header {
        let h = *h;
        if h.is_empty() {
            lines.push("#".to_owned());
        } else {
            lines.push(format!("# {h}"));
        }
    }
    if !header.is_empty() {
        lines.push(String::new());
    }
    for (k, v) in entries {
        let key = k.as_ref();
        if !valid_key(key) {
            return Err(HostError::message(format!(
                "invalid env key: {}",
                serde_json::Value::String(key.into())
            )));
        }
        let formatted = format_value(v.as_ref())?;
        lines.push(format!("{key}={formatted}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    Raw(String),
    Entry { key: String, value: String, raw: String },
}

/// An env file kept line by line, so an edit changes only the line it touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvDocument {
    bom: bool,
    line_ending: &'static str,
    lines: Vec<Line>,
}

impl EnvDocument {
    /// Reads `text`, remembering its BOM and line ending.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let (bom, body) = match text.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let line_ending = if body.contains("\r\n") { "\r\n" } else { "\n" };
        let mut lines = Vec::new();
        for raw_part in body.split('\n') {
            let line = raw_part.strip_suffix('\r').unwrap_or(raw_part);
            if let Some((key, value)) = parse_line(line) {
                lines.push(Line::Entry {
                    key,
                    value,
                    raw: line.to_owned(),
                });
            } else {
                lines.push(Line::Raw(line.to_owned()));
            }
        }
        Self {
            bom,
            line_ending,
            lines,
        }
    }

    /// The variables it sets; a later line for a key wins.
    #[must_use]
    pub fn values(&self) -> EnvMap {
        let mut map = EnvMap::new();
        for line in &self.lines {
            if let Line::Entry { key, value, .. } = line {
                map.insert(key.clone(), value.clone());
            }
        }
        map
    }

    /// The value the last line for `key` sets.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        self.lines.iter().rev().find_map(|line| match line {
            Line::Entry { key: k, value: v, .. } if k == key => Some(v.clone()),
            _ => None,
        })
    }

    /// Sets `key`: its last line is rewritten, or a new line is added at the end. Fails on an invalid key or value.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), HostError> {
        if !valid_key(key) {
            return Err(HostError::message(format!(
                "invalid env key: {}",
                serde_json::Value::String(key.into())
            )));
        }
        let formatted = format_value(value)?;
        let raw = format!("{key}={formatted}");
        if let Some(existing) = self
            .lines
            .iter_mut()
            .rev()
            .find(|line| matches!(line, Line::Entry { key: k, .. } if k == key))
        {
            *existing = Line::Entry {
                key: key.to_owned(),
                value: value.to_owned(),
                raw,
            };
            return Ok(());
        }
        let entry = Line::Entry {
            key: key.to_owned(),
            value: value.to_owned(),
            raw,
        };
        if matches!(self.lines.last(), Some(Line::Raw(s)) if s.is_empty()) {
            let insert_idx = self.lines.len() - 1;
            self.lines.insert(insert_idx, entry);
        } else {
            self.lines.push(entry);
            self.lines.push(Line::Raw(String::new()));
        }
        Ok(())
    }

    /// Renames every line of `old` to `new` where it stands, value and all.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), HostError> {
        if !valid_key(new) {
            return Err(HostError::message(format!(
                "invalid env key: {}",
                serde_json::Value::String(new.into())
            )));
        }
        for line in &mut self.lines {
            if let Line::Entry { key, raw, .. } = line
                && key == old
            {
                *raw = raw.replacen(old, new, 1);
                *key = new.to_owned();
            }
        }
        Ok(())
    }

    /// Drops every line that sets `key`.
    pub fn remove(&mut self, key: &str) {
        self.lines
            .retain(|line| !matches!(line, Line::Entry { key: k, .. } if k == key));
    }

    /// The text, with its BOM and line ending as read.
    #[must_use]
    pub fn to_text(&self) -> String {
        let joined = self
            .lines
            .iter()
            .map(|line| match line {
                Line::Raw(raw) | Line::Entry { raw, .. } => raw.as_str(),
            })
            .collect::<Vec<_>>()
            .join(self.line_ending);
        if self.bom { format!("\u{feff}{joined}") } else { joined }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_key_checks_ascii_identifier() {
        assert!(valid_key("PORT"));
        assert!(valid_key("_SECRET_1"));
        assert!(!valid_key(""));
        assert!(!valid_key("1PORT"));
        assert!(!valid_key("BAD-KEY"));
        assert!(!valid_key("BAD KEY"));
    }

    #[test]
    fn parse_env_handles_bom_export_quotes_and_comments() {
        let input = "\u{feff}# header\r\nexport PORT = 61000 # inline comment\r\nNAME=\"My \\\"Server\\\"\\nLine2\"\r\nRAW='single\\nquoted # not comment'\r\nURL=http://127.0.0.1:61000#hash\r\nPORT=61001\r\n";
        let env = parse_env(input);
        assert_eq!(env.get("PORT"), Some("61001"));
        assert_eq!(env.get("NAME"), Some("My \"Server\"\nLine2"));
        assert_eq!(env.get("RAW"), Some("single\\nquoted # not comment"));
        assert_eq!(env.get("URL"), Some("http://127.0.0.1:61000#hash"));
        let keys: Vec<&str> = env.iter().collect::<Vec<_>>().into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["PORT", "NAME", "RAW", "URL"]);
    }

    #[test]
    fn format_value_quotes_when_needed_and_rejects_newlines() {
        assert_eq!(
            format_value("http://127.0.0.1:61000/?a=1&b=2"),
            Ok("\"http://127.0.0.1:61000/?a=1&b=2\"".to_owned())
        );
        assert_eq!(
            format_value("plain_value-1.0/path:ok@host,a+b=c%d"),
            Ok("plain_value-1.0/path:ok@host,a+b=c%d".to_owned())
        );
        assert_eq!(format_value(""), Ok(String::new()));
        assert_eq!(
            format_value("a \"quote\" \\ slash"),
            Ok("\"a \\\"quote\\\" \\\\ slash\"".to_owned())
        );
        assert!(format_value("line1\nline2").is_err());
        assert!(format_value("line1\rline2").is_err());
    }

    #[test]
    fn env_document_preserves_bom_crlf_and_comments() {
        let original = "\u{feff}# comment\r\nFIRST=1\r\nSECOND=\"two\"\r\n";
        let mut doc = EnvDocument::parse(original);
        assert_eq!(doc.get("FIRST").as_deref(), Some("1"));
        assert!(doc.set("FIRST", "updated value").is_ok());
        assert!(doc.set("THIRD", "3").is_ok());
        assert_eq!(
            doc.to_text(),
            "\u{feff}# comment\r\nFIRST=\"updated value\"\r\nSECOND=\"two\"\r\nTHIRD=3\r\n"
        );
    }
}
