// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;

#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error("{path}: {message}")]
    File { path: String, message: String },
    #[error("{name}: {message}")]
    Value { name: String, message: String },
}

impl Env {
    pub fn load(file: Option<&Path>) -> Result<Self, EnvError> {
        let mut vars = BTreeMap::new();
        if let Some(path) = file {
            let iter = dotenvy::from_path_iter(path).map_err(|err| EnvError::File {
                path: path.display().to_string(),
                message: err.to_string(),
            })?;
            for item in iter {
                let (k, v) = item.map_err(|err| EnvError::File {
                    path: path.display().to_string(),
                    message: err.to_string(),
                })?;
                vars.insert(k, v);
            }
        }
        for (k, v) in std::env::vars() {
            vars.insert(k, v);
        }
        Ok(Self { vars })
    }

    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let vars = pairs.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
        Self { vars }
    }

    pub fn strip_prefix(&self, prefix: &str) -> Self {
        let vars = self
            .vars
            .iter()
            .filter_map(|(k, v)| k.strip_prefix(prefix).map(|stripped| (stripped.to_owned(), v.clone())))
            .collect();
        Self { vars }
    }

    pub fn vars(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn raw(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.raw(name).filter(|val| !val.is_empty())
    }

    pub fn parse<T>(&self, name: &str) -> Result<Option<T>, EnvError>
    where
        T: FromStr,
        T::Err: Display,
    {
        let Some(val) = self.get(name) else {
            return Ok(None);
        };
        val.trim().parse::<T>().map(Some).map_err(|err| EnvError::Value {
            name: name.to_owned(),
            message: err.to_string(),
        })
    }

    pub fn flag(&self, name: &str, default: bool) -> Result<bool, EnvError> {
        match self.get(name) {
            None => Ok(default),
            Some("1") => Ok(true),
            Some("0") => Ok(false),
            Some(other) => Err(EnvError::Value {
                name: name.to_owned(),
                message: format!("{other:?} is not 0 or 1"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_ignores_empty_while_raw_preserves_it() {
        let env = Env::from_pairs([("EMPTY", ""), ("SET", "value")]);
        assert_eq!(env.raw("EMPTY"), Some(""));
        assert_eq!(env.get("EMPTY"), None);
        assert_eq!(env.get("SET"), Some("value"));
        assert_eq!(env.raw("MISSING"), None);
    }

    #[test]
    fn strip_prefix_filters_and_strips_keys() {
        let env = Env::from_pairs([("APP_PORT", "8080"), ("APP_HOST", "localhost"), ("OTHER", "1")]);
        let stripped = env.strip_prefix("APP_");
        assert_eq!(stripped.get("PORT"), Some("8080"));
        assert_eq!(stripped.get("HOST"), Some("localhost"));
        assert_eq!(stripped.get("OTHER"), None);
    }

    #[test]
    fn parse_and_flag_handle_valid_and_invalid_values() {
        let env = Env::from_pairs([
            ("PORT", " 3000 "),
            ("BAD_PORT", "abc"),
            ("FLAG_ON", "1"),
            ("FLAG_OFF", "0"),
            ("FLAG_BAD", "true"),
        ]);
        assert_eq!(env.parse::<u16>("PORT").unwrap(), Some(3000));
        assert_eq!(env.parse::<u16>("UNSET").unwrap(), None);
        assert!(env.parse::<u16>("BAD_PORT").is_err());

        assert!(env.flag("FLAG_ON", false).unwrap());
        assert!(!env.flag("FLAG_OFF", true).unwrap());
        assert!(env.flag("UNSET", true).unwrap());
        let err = env.flag("FLAG_BAD", false).unwrap_err();
        assert_eq!(err.to_string(), "FLAG_BAD: \"true\" is not 0 or 1");
    }
}
