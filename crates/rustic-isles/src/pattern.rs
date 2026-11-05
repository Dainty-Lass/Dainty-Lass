// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Express 5's matching for the table's two kinds of path (core spec §4.1). `service_kit::routing` refuses empty
//! segments and regex rows, both of which the table has, so the crate keeps its own.

use std::fmt;

/// A compiled row path.
#[derive(Debug, Clone)]
pub struct Pattern {
    kind: Kind,
}

#[derive(Debug, Clone)]
enum Kind {
    Express(Vec<Segment>),
    Regex(regex::Regex),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Param(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("route path {path:?}: {reason}")]
pub struct PatternError {
    pub path: String,
    pub reason: String,
}

/// A parameter that is not valid percent-encoded UTF-8 (Express answers 400).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadParam {
    pub value: String,
}

/// Decoded params in the order they appear; regex captures are named `0`, `1`, ...
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params(Vec<(String, String)>);

impl Params {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    pub fn into_pairs(self) -> Vec<(String, String)> {
        self.0
    }
}

impl fmt::Display for BadParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad param {}", self.value)
    }
}

fn error(path: &str, reason: &str) -> PatternError {
    PatternError {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
}

/// One trailing `/` goes, except from `/` itself.
fn trim_one_slash(path: &str) -> &str {
    if path.len() > 1 {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    }
}

/// `decodeURIComponent` without `+` handling, as Express decodes params.
pub fn decode_param(raw: &str) -> Result<String, BadParam> {
    service_kit::js::decode_uri_component(raw).ok_or_else(|| BadParam { value: raw.to_owned() })
}

impl Pattern {
    pub fn parse(path: &str) -> Result<Self, PatternError> {
        if path.starts_with("/^") {
            return Self::parse_regex(path);
        }
        let Some(rest) = trim_one_slash(path).strip_prefix('/') else {
            return Err(error(path, "an Express path starts with /"));
        };
        let mut segments = Vec::new();
        for segment in rest.split('/') {
            if let Some(name) = segment.strip_prefix(':') {
                let named = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
                if !named {
                    return Err(error(path, "a parameter needs a name of letters, digits or _"));
                }
                segments.push(Segment::Param(name.to_owned()));
            } else if segment.contains(['*', '{', '}', '?', '(', ')', ':']) {
                return Err(error(path, "a literal segment has a pattern character"));
            } else {
                segments.push(Segment::Literal(segment.to_owned()));
            }
        }
        Ok(Self {
            kind: Kind::Express(segments),
        })
    }

    fn parse_regex(path: &str) -> Result<Self, PatternError> {
        let Some(last) = path.rfind('/').filter(|&i| i > 0) else {
            return Err(error(path, "a regex literal ends with / and its flags"));
        };
        let source = &path[1..last];
        let insensitive = match &path[last + 1..] {
            "" => false,
            "i" => true,
            _ => return Err(error(path, "the only regex flag is i")),
        };
        let regex = regex::RegexBuilder::new(source)
            .case_insensitive(insensitive)
            .build()
            .map_err(|e| error(path, &e.to_string()))?;
        Ok(Self {
            kind: Kind::Regex(regex),
        })
    }

    /// `None` when the path does not match; `Some(Err)` when it does but a param fails to decode.
    pub fn matches(&self, path: &str) -> Option<Result<Params, BadParam>> {
        match &self.kind {
            Kind::Express(segments) => {
                let rest = trim_one_slash(path).strip_prefix('/')?;
                let mut raw = Vec::with_capacity(segments.len());
                let mut parts = rest.split('/');
                for segment in segments {
                    let part = parts.next()?;
                    match segment {
                        Segment::Literal(text) if text.eq_ignore_ascii_case(part) => {}
                        Segment::Literal(_) => return None,
                        Segment::Param(_) if part.is_empty() => return None,
                        Segment::Param(name) => raw.push((name, part)),
                    }
                }
                if parts.next().is_some() {
                    return None;
                }
                Some(decode_all(raw.into_iter().map(|(n, v)| (n.clone(), v))))
            }
            Kind::Regex(regex) => {
                let captures = regex.captures(path)?;
                let groups = (1..captures.len()).map(|i| {
                    let text = captures.get(i).map_or("", |m| m.as_str());
                    ((i - 1).to_string(), text)
                });
                Some(decode_all(groups))
            }
        }
    }
}

fn decode_all<'a>(raw: impl Iterator<Item = (String, &'a str)>) -> Result<Params, BadParam> {
    let mut out = Vec::new();
    for (name, value) in raw {
        out.push((name, decode_param(value)?));
    }
    Ok(Params(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(pattern: &str, path: &str) -> Option<Result<Vec<(String, String)>, BadParam>> {
        Pattern::parse(pattern)
            .unwrap()
            .matches(path)
            .map(|r| r.map(Params::into_pairs))
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn the_spec_table() {
        assert_eq!(
            check("/account/api/oauth/token", "/Account/API/oauth/TOKEN/"),
            Some(Ok(vec![]))
        );
        assert_eq!(check("/account/api/oauth/token", "/account/api/oauth/token//"), None);
        assert_eq!(check("/motd/", "/motd"), Some(Ok(vec![])));
        assert_eq!(check("/", "/"), Some(Ok(vec![])));
        assert_eq!(check("/", "/x"), None);
        assert_eq!(check("/friends/:id", "/friends/"), None);
        assert_eq!(
            check("/loadout/:userId//all", "/loadout/A%20B//ALL"),
            Some(Ok(pairs(&[("userId", "A B")])))
        );
        assert_eq!(check("/loadout/:userId//all", "/loadout/a/all"), None);
        assert_eq!(check("/loadout/:userId/:characterId/all", "/loadout/a//all"), None);
        let inventory = r"/^\/inventory\/([^/]*)\/(.+)$/";
        assert_eq!(
            check(inventory, "/inventory//dauntlessrel-1.14.7:647472"),
            Some(Ok(pairs(&[("0", ""), ("1", "dauntlessrel-1.14.7:647472")])))
        );
        assert_eq!(check(inventory, "/INVENTORY/a/b"), None);
        let leaderboard = r"/^\/leaderboard\/get_leaderboard\//i";
        assert_eq!(check(leaderboard, "/Leaderboard/get_leaderboard/x/y"), Some(Ok(vec![])));
        assert_eq!(check(leaderboard, "/leaderboard/get_leaderboard"), None);
        assert_eq!(
            check("/x/:id", "/x/%E2%28"),
            Some(Err(BadParam {
                value: "%E2%28".to_owned()
            }))
        );
    }

    #[test]
    fn more_edges() {
        assert_eq!(check("/a", "/a//"), None);
        assert_eq!(check("/a", "a"), None);
        assert_eq!(check("/x/:id", "/x/a+b"), Some(Ok(pairs(&[("id", "a+b")]))));
        assert_eq!(check("/x/:id", "/x/%ZZ").map(|r| r.is_err()), Some(true));
        assert!(Pattern::parse("/a/:").is_err());
        assert!(Pattern::parse("/a/*").is_err());
        assert!(Pattern::parse("a").is_err());
        assert!(Pattern::parse("/^x/g").is_err());
    }

    #[test]
    fn every_row_compiles() {
        for row in crate::routes::all() {
            Pattern::parse(row.path).unwrap_or_else(|e| panic!("{e}"));
        }
    }
}
