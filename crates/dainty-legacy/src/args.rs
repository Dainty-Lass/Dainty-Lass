// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The arguments both entry points take (spec §3), checked in the table's order as each is read.

use std::path::PathBuf;

use dainty_module_api::NodeId;

use crate::error::ImportError;

/// What is imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A Revived 1.4.4 server's database.
    Revived,
    /// A player's 1.4.4 offline save.
    OfflineSave,
    /// A Reforged Node kit's checkout.
    Reforged,
}

impl Kind {
    /// The name the commands use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Revived => "revived",
            Self::OfflineSave => "offline-save",
            Self::Reforged => "reforged",
        }
    }

    /// Reads a name [`Kind::as_str`] writes.
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Revived, Self::OfflineSave, Self::Reforged]
            .into_iter()
            .find(|k| k.as_str() == text)
    }
}

/// How the 1.14.7 database is named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `--to`: the SQLite file.
    To(PathBuf),
    /// `--env-file`: the 1.14.7 backend's env file.
    EnvFile(PathBuf),
}

/// One import's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportArgs {
    /// What is imported.
    pub kind: Kind,
    /// The database, save or checkout.
    pub source: PathBuf,
    /// The 1.14.7 database.
    pub target: Option<Target>,
    /// `--node`, overriding the default.
    pub node: Option<NodeId>,
    /// Run the writes and roll them back, or (reforged) change nothing.
    pub dry_run: bool,
    /// Reforged: start on a new database and leave MongoDB as it is.
    pub fresh_start: bool,
    /// Reforged: the program that runs `db migrate` and `db verify`.
    pub db_tool: Option<PathBuf>,
}

fn usage(text: impl Into<String>) -> ImportError {
    ImportError::Usage(text.into())
}

fn absolute(flag: &str, value: &str) -> Result<PathBuf, ImportError> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(usage(format!("{flag} must be an absolute path")))
    }
}

/// Reads `args`, everything after `import <kind>` (or the console command's name).
pub fn parse(kind: Kind, args: &[String]) -> Result<ImportArgs, ImportError> {
    let reforged = kind == Kind::Reforged;
    let mut out = ImportArgs {
        kind,
        source: PathBuf::new(),
        target: None,
        node: None,
        dry_run: false,
        fresh_start: false,
        db_tool: None,
    };
    let mut source = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |flag: &str| iter.next().ok_or_else(|| usage(format!("{flag} needs a value")));
        match arg.as_str() {
            "--dry-run" => out.dry_run = true,
            "--fresh-start" if reforged => out.fresh_start = true,
            flag @ ("--to" | "--env-file") => {
                if out.target.is_some() {
                    return Err(usage("give the 1.14.7 database once: --to or --env-file"));
                }
                let path = absolute(flag, value(flag)?)?;
                out.target = Some(if flag == "--to" {
                    Target::To(path)
                } else {
                    Target::EnvFile(path)
                });
            }
            "--node" => {
                let text = value("--node")?;
                out.node = Some(text.parse().map_err(|e| usage(format!("--node: {e}")))?);
            }
            "--db-tool" if reforged => out.db_tool = Some(absolute("--db-tool", value("--db-tool")?)?),
            "--isles-db" if reforged => {
                return Err(usage(
                    "--isles-db is gone: the database steps run through dainty-host db; name that program with \
                     --db-tool <program> (dainty-host from PATH by default)",
                ));
            }
            flag if flag.starts_with("--") => return Err(usage(format!("unknown option {flag}"))),
            path => {
                if source.is_some() {
                    return Err(usage("one source only"));
                }
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(usage("the source must be an absolute path"));
                }
                source = Some(path);
            }
        }
    }
    out.source = source.ok_or_else(|| usage("which install or file to import?"))?;
    if !reforged && out.target.is_none() {
        return Err(usage("which 1.14.7 database to import into? --to or --env-file"));
    }
    if out.fresh_start && out.target.is_some() {
        return Err(usage(
            "--fresh-start moves no MongoDB, so it takes no --to or --env-file",
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(name: &str) -> String {
        std::env::temp_dir().join(name).to_string_lossy().into_owned()
    }

    fn err(kind: Kind, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        match parse(kind, &args) {
            Err(ImportError::Usage(text)) => text,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reads_every_option() {
        let args = vec![
            abs("a.db"),
            "--to".into(),
            abs("t.db"),
            "--node".into(),
            "eu-west".into(),
            "--dry-run".into(),
        ];
        let parsed = parse(Kind::Revived, &args).unwrap();
        assert_eq!(parsed.source, PathBuf::from(abs("a.db")));
        assert_eq!(parsed.target, Some(Target::To(abs("t.db").into())));
        assert_eq!(parsed.node.unwrap().as_str(), "eu-west");
        assert!(parsed.dry_run);

        let args = vec![
            abs("old"),
            "--fresh-start".into(),
            "--db-tool".into(),
            abs("dainty-host"),
        ];
        let parsed = parse(Kind::Reforged, &args).unwrap();
        assert!(parsed.fresh_start && parsed.target.is_none());
        assert_eq!(parsed.db_tool, Some(abs("dainty-host").into()));
    }

    #[test]
    fn refuses_in_order() {
        let a = abs("a.db");
        let t = abs("t.db");
        assert_eq!(
            err(Kind::Revived, &[&a, "--to", &t, "--env-file", &t]),
            "give the 1.14.7 database once: --to or --env-file"
        );
        assert_eq!(
            err(Kind::Revived, &[&a, "--to", "t.db"]),
            "--to must be an absolute path"
        );
        assert_eq!(
            err(Kind::Revived, &[&a, "--env-file", "t"]),
            "--env-file must be an absolute path"
        );
        assert_eq!(err(Kind::Revived, &[&a, "--to"]), "--to needs a value");
        assert!(err(Kind::Revived, &[&a, "--node", "Bad Node"]).starts_with("--node: a node id must be"));
        assert_eq!(
            err(Kind::Revived, &[&a, "--fresh-start"]),
            "unknown option --fresh-start"
        );
        assert_eq!(
            err(Kind::OfflineSave, &[&a, "--db-tool", &t]),
            "unknown option --db-tool"
        );
        assert_eq!(
            err(Kind::Reforged, &[&a, "--db-tool", "x"]),
            "--db-tool must be an absolute path"
        );
        assert!(err(Kind::Reforged, &[&a, "--isles-db", "x"]).starts_with("--isles-db is gone"));
        assert_eq!(err(Kind::Revived, &[&a, "--isles-db"]), "unknown option --isles-db");
        assert_eq!(err(Kind::Revived, &[&a, "--what"]), "unknown option --what");
        assert_eq!(err(Kind::Revived, &["a.db"]), "the source must be an absolute path");
        assert_eq!(err(Kind::Revived, &[&a, &t]), "one source only");
        assert_eq!(err(Kind::Revived, &["--dry-run"]), "which install or file to import?");
        assert_eq!(
            err(Kind::OfflineSave, &[&a]),
            "which 1.14.7 database to import into? --to or --env-file"
        );
        assert_eq!(
            err(Kind::Reforged, &[&a, "--fresh-start", "--to", &t]),
            "--fresh-start moves no MongoDB, so it takes no --to or --env-file"
        );
    }

    #[test]
    fn kinds_round_trip() {
        for kind in [Kind::Revived, Kind::OfflineSave, Kind::Reforged] {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(Kind::parse("metagame"), None);
    }
}
