//! `admin guarded list|approve|cancel` (run as `dainty-host admin guarded ...`): the host side of the guarded
//! actions (DD23 layer 4, DH6). The host kit has the machine and `admin.env`, so these need no key, and the
//! host counts as an owner no key can be: it may approve what any key asked, or cancel it. It only records
//! the decision. The running server carries an approved action out within 30 seconds, as the key that asked;
//! a stopped one does at its next start. Each change is logged under `host:admin guarded <command>`.
//!
//! The server GUI calls the same functions in its own process (`host::guarded`), so a decision there is the
//! command line one.

use std::path::Path;
use std::process::ExitCode;

use serde_json::{Value, json};
use service_kit::env::Env;

use crate::cli::GuardedCommand;
use crate::config::Config;
use crate::guard::{GuardError, Guards, Record, State, Who, is_id};
use crate::input::iso;
use crate::keys::Keys;

/// The most records a list shows.
const LIST_MAX: i64 = 100;

/// Opens the guarded actions in the database `admin.env` names. Blocking: it migrates the schema.
pub fn open(env_file: Option<&Path>) -> Result<Guards, String> {
    let env = Env::load(env_file).map_err(|e| e.to_string())?;
    let config = Config::from_env(&env)?;
    let keys = Keys::open(&Keys::database_for(&config.database), &config.api_key)?;
    Ok(Guards::new(keys.database().clone(), config.guard))
}

/// A failure with a stable code (`not_found`, `not_waiting`, `bad_id`, `failed`) for tools, and the words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// `not_found`, `not_waiting`, `bad_id` or `failed`.
    pub code: &'static str,
    /// What went wrong, for people.
    pub message: String,
}

impl Failure {
    fn failed(message: String) -> Self {
        Self {
            code: "failed",
            message,
        }
    }
}

impl From<GuardError> for Failure {
    fn from(error: GuardError) -> Self {
        let code = match error {
            GuardError::NotFound => "not_found",
            GuardError::NotWaiting(_) | GuardError::DelayOnly | GuardError::Expired => "not_waiting",
            _ => "failed",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

fn checked(given: &str) -> Result<&str, Failure> {
    let id = given.trim();
    if is_id(id) {
        Ok(id)
    } else {
        Err(Failure {
            code: "bad_id",
            message: "an id is 16 lower-case hex digits, as guarded list shows".to_owned(),
        })
    }
}

/// A record as `--json` and the server GUI show it: the fields of the API GuardedAction, never the
/// parameters it runs with.
pub fn record_json(r: &Record) -> Value {
    json!({
        "id": r.id,
        "kind": r.kind.as_str(),
        "summary": r.summary,
        "state": r.state.as_str(),
        "mode": if r.release_at.is_some() { "delay" } else { "approval" },
        "initiatedBy": r.initiated_by,
        "needsRole": r.needs_role.as_str(),
        "createdAt": iso(r.created_at),
        "releaseAt": r.release_at.map(iso),
        "expiresAt": iso(r.expires_at),
        "approvedBy": r.approved_by,
        "canceledBy": r.canceled_by,
        "endedAt": r.ended_at.map(iso),
        "result": r.result,
    })
}

/// The waiting actions; with `all`, the latest finished ones too.
pub async fn list(guards: &Guards, all: bool) -> Result<Vec<Record>, Failure> {
    Ok(guards.list(all, LIST_MAX).await?)
}

/// Approves `id` as the host kit; the server runs it as the key that asked.
pub async fn approve(guards: &Guards, id: &str) -> Result<Record, Failure> {
    let id = checked(id)?;
    Ok(guards.approve(id, Who::host("approve"), crate::now_ms()).await?)
}

/// Cancels `id` as the host kit.
pub async fn cancel(guards: &Guards, id: &str) -> Result<Record, Failure> {
    let id = checked(id)?;
    Ok(guards.cancel(id, Who::host("cancel"), crate::now_ms()).await?)
}

/// One row of the plain list.
fn row(r: &Record) -> String {
    let wait = match (r.state, r.release_at) {
        (State::Pending, Some(at)) => format!("runs at {}", iso(at)),
        (State::Pending, None) => format!("needs approval (a {} or higher)", r.needs_role),
        (State::Approved, _) => format!("approved by {}", r.approved_by.as_deref().unwrap_or("?")),
        (state, _) => state.as_str().to_owned(),
    };
    format!(
        "{:<18}{:<11}{:<10}{wait}\n{:<18}{}",
        r.id,
        r.kind.as_str(),
        r.initiated_by,
        "",
        r.summary
    )
}

fn runtime() -> Result<tokio::runtime::Runtime, Failure> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Failure::failed(e.to_string()))
}

/// What a command printed: the JSON value for tools and the text for people.
type Done = Result<(Value, String), Failure>;

fn run_list(guards: &Guards, all: bool) -> Done {
    let records = runtime()?.block_on(list(guards, all))?;
    let text = if records.is_empty() {
        "Nothing is waiting.".to_owned()
    } else {
        let mut lines = vec![format!("{:<18}{:<11}{:<10}WAITING FOR", "ID", "ACTION", "ASKED BY")];
        lines.extend(records.iter().map(row));
        lines.join("\n")
    };
    Ok((Value::Array(records.iter().map(record_json).collect()), text))
}

fn run_approve(guards: &Guards, id: &str) -> Done {
    let record = runtime()?.block_on(approve(guards, id))?;
    let text = format!(
        "Approved {}: {}\nThe running server carries it out within 30 seconds, as {}; a stopped one at its next start.",
        record.id, record.summary, record.initiated_by
    );
    Ok((record_json(&record), text))
}

fn run_cancel(guards: &Guards, id: &str) -> Done {
    let record = runtime()?.block_on(cancel(guards, id))?;
    Ok((
        record_json(&record),
        format!("Canceled {}: {}", record.id, record.summary),
    ))
}

/// With --json one line: ok true and the result, or ok false and the failure code.
fn show(json_output: bool, done: Done) -> Result<(), String> {
    match (json_output, done) {
        (true, Ok((value, _))) => {
            println!("{}", json!({"ok": true, "result": value}));
            Ok(())
        }
        (true, Err(failure)) => {
            println!(
                "{}",
                json!({"ok": false, "error": failure.code, "message": failure.message})
            );
            Err(failure.message)
        }
        (false, Ok((_, text))) => {
            println!("{text}");
            Ok(())
        }
        (false, Err(failure)) => Err(failure.message),
    }
}

/// Runs one `admin guarded` command; prints JSON with `--json`.
pub fn run(command: GuardedCommand) -> ExitCode {
    let (json_output, env_file) = match &command {
        GuardedCommand::List { json, env_file, .. }
        | GuardedCommand::Approve { json, env_file, .. }
        | GuardedCommand::Cancel { json, env_file, .. } => (*json, env_file.clone()),
    };
    let done = match open(env_file.as_deref()) {
        Err(message) => Err(Failure::failed(message)),
        Ok(guards) => match &command {
            GuardedCommand::List { all, .. } => run_list(&guards, *all),
            GuardedCommand::Approve { id, .. } => run_approve(&guards, id),
            GuardedCommand::Cancel { id, .. } => run_cancel(&guards, id),
        },
    };
    match show(json_output, done) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("\nError: {error}");
            ExitCode::FAILURE
        }
    }
}
