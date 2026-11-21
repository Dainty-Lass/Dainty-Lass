// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The administrator bootstrap (admin spec §12): makes an existing password account an administrator, the only way
//! to a password administrator. A library function; the `dainty-host admin bootstrap` subcommand is a clap row.

use dainty_module_api::{Actor, ActorContext};
use rustic_isles_store::repo::auth::{AccessChange, ApprovalStatus, CreateOutcome, OperationalStatus};
use rustic_isles_store::{IslesStore, RepoError};

use super::user_id_of;
use crate::util::{self, iso, system_now_ms};

/// The bootstrap's usage text, shown on a bad command line.
pub const USAGE: &str = "Usage: dainty-host admin bootstrap --email=you@example.com [--from-email=old@example.com | \
                         --username=ExistingName] --confirm=GRANT_ADMIN_ROLE";

/// `dainty-host admin bootstrap`'s options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootstrapAdminRequest {
    /// The email the administrator signs in with.
    pub email: String,
    /// The account's current email, when it is to be renamed to `email`.
    pub from_email: Option<String>,
    /// The account's username, to find it by name instead.
    pub username: Option<String>,
    /// Must be `GRANT_ADMIN_ROLE`.
    pub confirm: String,
}

/// The account made administrator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapAdminOutcome {
    /// Its userId.
    pub user_id: String,
    /// Its email after the bootstrap.
    pub email: String,
}

/// Why the bootstrap refused.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    /// The options are missing or `--confirm` is wrong.
    #[error("{USAGE}")]
    Usage,
    /// No launcher account matches; the text says by what.
    #[error("Launcher account not found by {0}.")]
    NotFound(String),
    /// The account signs in with Discord, not a password.
    #[error("Matched account is not a password-based launcher account.")]
    NotPasswordBased,
    /// The account has no username yet.
    #[error("The launcher account must have a username before becoming an admin.")]
    NoUsername,
    /// The new email belongs to another account.
    #[error("Cannot rename account: {0} is already in use.")]
    EmailInUse(String),
    /// The store failed.
    #[error(transparent)]
    Store(RepoError),
}

fn lower(text: &str) -> String {
    util::trim(text).to_ascii_lowercase()
}

fn blank_none(text: Option<&str>) -> Option<String> {
    text.map(lower).filter(|t| !t.is_empty())
}

enum Done {
    Ok(BootstrapAdminOutcome),
    Refused(BootstrapError),
}

/// One write as `Console`: the account found, its email set, `admin` added, approved and active.
pub async fn bootstrap_admin(
    store: &IslesStore,
    request: BootstrapAdminRequest,
) -> Result<BootstrapAdminOutcome, BootstrapError> {
    let email = lower(&request.email);
    let from = blank_none(request.from_email.as_deref());
    let username = blank_none(request.username.as_deref());
    if email.is_empty() || request.confirm != "GRANT_ADMIN_ROLE" {
        return Err(BootstrapError::Usage);
    }
    let done = store
        .write(&ActorContext::new(Actor::Console), move |tx, repos| {
            let accounts = &repos.launcher_accounts;
            let mut found = match &username {
                Some(name) => accounts.find_by_display_name_normalized(tx.conn(), name)?,
                None => accounts.find_by_email(tx.conn(), from.as_deref().unwrap_or(&email))?,
            };
            if found.is_none() && from.as_ref().is_some_and(|f| *f != email) {
                found = accounts.find_by_email(tx.conn(), &email)?;
            }
            let Some(account) = found else {
                let by = match &username {
                    Some(name) => format!("username={name}"),
                    None => format!("email={}", from.as_deref().unwrap_or(&email)),
                };
                return Ok(Done::Refused(BootstrapError::NotFound(by)));
            };
            if account.password_hash.is_none() {
                return Ok(Done::Refused(BootstrapError::NotPasswordBased));
            }
            if account.username_set == Some(false) {
                return Ok(Done::Refused(BootstrapError::NoUsername));
            }
            if let Some(holder) = accounts.find_by_email(tx.conn(), &email)?
                && holder.account != account.account
            {
                return Ok(Done::Refused(BootstrapError::EmailInUse(email)));
            }
            if accounts.set_email(tx, &account.account, &email)? != CreateOutcome::Created {
                return Ok(Done::Refused(BootstrapError::EmailInUse(email)));
            }
            let now = iso(system_now_ms());
            if !account.roles.iter().any(|r| r == "admin") {
                let mut roles = account.roles.clone();
                roles.push("admin".into());
                accounts.update_roles(tx, &account.account, &roles, &now, "bootstrap-script")?;
            }
            accounts.set_access_state(
                tx,
                &account.account,
                &AccessChange {
                    approval_status: Some(ApprovalStatus::Approved),
                    status: Some(OperationalStatus::Active),
                    approval_updated_at: now,
                    approval_updated_by: "bootstrap-script".into(),
                    approval_reason: Some("Initial administrator bootstrap".into()),
                },
            )?;
            let user_id = user_id_of(tx.conn(), repos, &account.account);
            Ok(Done::Ok(BootstrapAdminOutcome { user_id, email }))
        })
        .await
        .map_err(BootstrapError::Store)?;
    match done {
        Done::Ok(outcome) => Ok(outcome),
        Done::Refused(error) => Err(error),
    }
}
