//! The import commands' arguments, the same from the console (a module command) and from the command line (the
//! executable run directly, before Dainty Lass first starts):
//!
//! ```text
//! revived      <database>  (--to <1.14.7 database file> | --env-file <backend env file>) [--node <id>] [--dry-run]
//! offline-save <save>      (--to <1.14.7 database file> | --env-file <backend env file>) [--node <id>] [--dry-run]
//! reforged     <checkout>  [--to <file> | --env-file <file>] [--fresh-start] [--db-tool <program>] [--dry-run]
//! ```
//!
//! Every path is absolute: a module's working directory is not the host's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dainty_module_api::NodeId;
use dainty_store::DatabaseConfig;

use crate::error::{ImportError, Result};
use crate::reforged::mongo::{DEFAULT_DB_TOOL, DbTool, MongoPlan};
use crate::report::Report;
use crate::revived::Options;
use crate::target::Target;

pub const USAGE: &str = "\
usage:
  import revived      <database> (--to <1.14.7 database> | --env-file <backend env file>) [--node <id>] [--dry-run]
  import offline-save <.drsave or database> (--to <file> | --env-file <file>) [--node <id>] [--dry-run]
  import reforged     <checkout> [--to <file> | --env-file <file>] [--fresh-start] [--db-tool <program>] [--dry-run]
every path is absolute";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Revived,
    OfflineSave,
    Reforged,
}

impl Kind {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "revived" => Some(Self::Revived),
            "offline-save" => Some(Self::OfflineSave),
            "reforged" => Some(Self::Reforged),
            _ => None,
        }
    }
}

/// Where the 1.14.7 database is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetArg {
    File(PathBuf),
    /// The backend's env file the host kit wrote, read for `REFORGED_DB_*` (its password is never printed)
    EnvFile(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub kind: Kind,
    pub source: PathBuf,
    pub target: Option<TargetArg>,
    pub node: Option<NodeId>,
    pub dry_run: bool,
    pub fresh_start: bool,
    pub db_tool: Option<PathBuf>,
}

fn usage(message: impl Into<String>) -> ImportError {
    ImportError::Usage(format!("{}\n{USAGE}", message.into()))
}

fn absolute(text: &str, what: &str) -> Result<PathBuf> {
    let path = PathBuf::from(text);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(usage(format!("{what} must be an absolute path")))
    }
}

/// `args` after the kind.
pub fn parse(kind: Kind, args: &[String]) -> Result<Request> {
    let mut source = None;
    let mut request = Request {
        kind,
        source: PathBuf::new(),
        target: None,
        node: None,
        dry_run: false,
        fresh_start: false,
        db_tool: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| args.next().ok_or_else(|| usage(format!("{flag} needs a value")));
        match arg.as_str() {
            "--dry-run" => request.dry_run = true,
            "--fresh-start" if kind == Kind::Reforged => request.fresh_start = true,
            "--to" | "--env-file" if request.target.is_some() => {
                return Err(usage("give the 1.14.7 database once: --to or --env-file"));
            }
            "--to" => request.target = Some(TargetArg::File(absolute(value("--to")?, "--to")?)),
            "--env-file" => request.target = Some(TargetArg::EnvFile(absolute(value("--env-file")?, "--env-file")?)),
            "--node" => {
                let text = value("--node")?;
                request.node = Some(text.parse().map_err(|e| usage(format!("--node: {e}")))?);
            }
            "--db-tool" if kind == Kind::Reforged => {
                request.db_tool = Some(absolute(value("--db-tool")?, "--db-tool")?);
            }
            // isles-db was retired for `dainty-host db` (RD27): a script still naming it gets told, not misrun
            "--isles-db" if kind == Kind::Reforged => {
                return Err(usage(
                    "--isles-db is gone: the database steps run through dainty-host db; name that program with \
                     --db-tool <program> (dainty-host from PATH by default)",
                ));
            }
            flag if flag.starts_with("--") => return Err(usage(format!("unknown option {flag}"))),
            path if source.is_none() => source = Some(absolute(path, "the source")?),
            _ => return Err(usage("one source only")),
        }
    }
    request.source = source.ok_or_else(|| usage("which install or file to import?"))?;
    if kind != Kind::Reforged && request.target.is_none() {
        return Err(usage("which 1.14.7 database to import into? --to or --env-file"));
    }
    if request.fresh_start && request.target.is_some() {
        return Err(usage(
            "--fresh-start moves no MongoDB, so it takes no --to or --env-file",
        ));
    }
    Ok(request)
}

/// `NAME=value` lines; the last one wins, as in the host kit's env files.
fn read_env_file(file: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(file).map_err(|e| ImportError::io(file, e))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().trim_matches(['"', '\'']).to_owned()))
        .collect())
}

/// store's `engine::ENV_ENGINE` and `ENV_FILE`, which it doesn't export
const ENV_ENGINE: &str = "REFORGED_DB_ENGINE";
const ENV_FILE: &str = "REFORGED_DB_FILE";

pub fn database_config(target: &TargetArg) -> Result<DatabaseConfig> {
    match target {
        TargetArg::File(file) => Ok(DatabaseConfig::Sqlite { file: file.clone() }),
        TargetArg::EnvFile(file) => {
            let env = read_env_file(file)?;
            let engine = env.get(ENV_ENGINE).map(String::as_str).unwrap_or("sqlite");
            if engine.trim() == "sqlite" && !env.contains_key(ENV_FILE) {
                return Err(ImportError::Usage(format!(
                    "{}: no {}, so not the 1.14.7 backend's env file",
                    file.display(),
                    ENV_FILE
                )));
            }
            DatabaseConfig::from_lookup(|name| env.get(name).cloned(), PathBuf::new())
                .map_err(|e| ImportError::Usage(format!("{}: {e}", file.display())))
        }
    }
}

/// Runs `request`. `default_node` is the node accounts are homed on when `--node` isn't given: the running node's
/// as a module command, `local` from the command line (as `dainty-host db migrate` does).
pub async fn run(request: &Request, default_node: &NodeId) -> Result<Report> {
    let node = request.node.clone().unwrap_or_else(|| default_node.clone());
    let options = Options::now(request.dry_run);
    match request.kind {
        Kind::Revived | Kind::OfflineSave => {
            let Some(target) = &request.target else {
                return Err(usage("which 1.14.7 database to import into? --to or --env-file"));
            };
            let target = Target::open(&database_config(target)?, node)?;
            if request.kind == Kind::Revived {
                crate::revived::import_server(&request.source, &target, options).await
            } else {
                crate::revived::import_offline_save(&request.source, &target, options).await
            }
        }
        Kind::Reforged => {
            let config = request.target.as_ref().map(database_config).transpose()?;
            let db_tool = DbTool {
                program: request
                    .db_tool
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_DB_TOOL)),
            };
            let plan = match &config {
                _ if request.fresh_start => MongoPlan::FreshStart,
                Some(target) => MongoPlan::Migrate {
                    target,
                    migrator: &db_tool,
                },
                None => MongoPlan::Refuse,
            };
            crate::reforged::upgrade(&request.source, &plan, request.dry_run)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[cfg(windows)]
    const ROOT: &str = "C:/";
    #[cfg(not(windows))]
    const ROOT: &str = "/";

    #[test]
    fn arguments() {
        let ok = parse(
            Kind::Revived,
            &args(&format!(
                "{ROOT}srv/meta.db --to {ROOT}srv/isles.db --node eu-1 --dry-run"
            )),
        )
        .unwrap();
        assert!(ok.dry_run && ok.node.unwrap().as_str() == "eu-1");
        assert_eq!(
            ok.target,
            Some(TargetArg::File(PathBuf::from(format!("{ROOT}srv/isles.db"))))
        );

        for bad in [
            "".to_owned(),
            format!("{ROOT}a.db"),
            format!("relative.db --to {ROOT}b.db"),
            format!("{ROOT}a.db --to relative.db"),
            format!("{ROOT}a.db --to {ROOT}b.db --env-file {ROOT}c.env"),
            format!("{ROOT}a.db {ROOT}b.db --to {ROOT}c.db"),
            format!("{ROOT}a.db --to {ROOT}b.db --fresh-start"),
            format!("{ROOT}a.db --to"),
            format!("{ROOT}a.db --to {ROOT}b.db --node Bad_Node"),
        ] {
            assert!(parse(Kind::Revived, &args(&bad)).is_err(), "{bad}");
        }
        let reforged = parse(Kind::Reforged, &args(&format!("{ROOT}srv/kit --fresh-start"))).unwrap();
        assert!(reforged.fresh_start && reforged.target.is_none());
        assert!(
            parse(
                Kind::Reforged,
                &args(&format!("{ROOT}srv/kit --fresh-start --to {ROOT}x.db"))
            )
            .is_err()
        );
        let tool = parse(
            Kind::Reforged,
            &args(&format!(
                "{ROOT}srv/kit --to {ROOT}x.db --db-tool {ROOT}srv/dainty-host"
            )),
        )
        .unwrap();
        assert_eq!(tool.db_tool, Some(PathBuf::from(format!("{ROOT}srv/dainty-host"))));
        let ImportError::Usage(message) = parse(
            Kind::Reforged,
            &args(&format!("{ROOT}srv/kit --to {ROOT}x.db --isles-db {ROOT}bin/isles-db")),
        )
        .unwrap_err() else {
            panic!("a usage error")
        };
        assert!(message.contains("--db-tool"), "{message}");
        assert!(
            parse(
                Kind::Revived,
                &args(&format!("{ROOT}a.db --to {ROOT}b.db --db-tool {ROOT}t"))
            )
            .is_err()
        );
    }

    #[test]
    fn an_env_file_gives_the_backend_s_database() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join("backend.env");
        std::fs::write(&env, "PORT=61000\nREFORGED_DB_FILE=/srv/isles.db\n").unwrap();
        assert_eq!(
            database_config(&TargetArg::EnvFile(env.clone())).unwrap(),
            DatabaseConfig::Sqlite {
                file: PathBuf::from("/srv/isles.db")
            }
        );
        std::fs::write(&env, "PORT=61000\n").unwrap();
        assert!(database_config(&TargetArg::EnvFile(env.clone())).is_err());
        std::fs::write(
            &env,
            "REFORGED_DB_ENGINE=postgres\nREFORGED_DB_PASSWORD=pg-s3cret\nREFORGED_DB_HOST=127.0.0.1\n",
        )
        .unwrap();
        let config = database_config(&TargetArg::EnvFile(env)).unwrap();
        assert!(matches!(&config, DatabaseConfig::Postgres(_)));
        assert!(!format!("{config:?}").contains("pg-s3cret"));
    }
}
