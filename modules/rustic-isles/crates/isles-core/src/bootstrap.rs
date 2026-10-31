//! Port of `reference/1.14.7/ParadoxBackend/scripts/bootstrap_admin.ts` (RB11).

use chrono::{SecondsFormat, Utc};
use diesel::prelude::*;
use store::Tx;

use crate::ids::UserId;
use crate::repo::auth::{AccessChange, ApprovalStatus, OperationalStatus, launcher_accounts};
use crate::repo::{RepoError, Repositories};

pub const CONFIRM_GRANT_ADMIN_ROLE: &str = "GRANT_ADMIN_ROLE";

#[derive(Debug, Clone)]
pub struct BootstrapAdminRequest {
    pub email: String,
    pub from_email: Option<String>,
    pub username: Option<String>,
    pub confirm: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapAdminOutcome {
    pub user_id: UserId,
    pub email: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error(
        "Usage: npm run admin:bootstrap -- --email=you@example.com [--from-email=old@example.com | --username=ExistingName] --confirm=GRANT_ADMIN_ROLE"
    )]
    Usage,
    #[error("Launcher account not found by {selector}.")]
    AccountNotFound { selector: String },
    #[error("Matched account is not a password-based launcher account.")]
    NotPasswordAccount,
    #[error("The launcher account must have a username before becoming an admin.")]
    UsernameNotSet,
    #[error("Cannot rename account: {email} is already in use.")]
    EmailCollision { email: String },
    #[error(transparent)]
    Repo(#[from] RepoError),
}

pub fn bootstrap_admin(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    req: &BootstrapAdminRequest,
) -> Result<BootstrapAdminOutcome, BootstrapError> {
    let email = req.email.trim().to_ascii_lowercase();
    let from_email = req
        .from_email
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let username = req
        .username
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    if email.is_empty() || req.confirm != CONFIRM_GRANT_ADMIN_ROLE {
        return Err(BootstrapError::Usage);
    }

    let mut account = if let Some(ref uname) = username {
        repos
            .launcher_accounts
            .find_by_display_name_normalized(tx.conn(), uname)?
    } else {
        let target = from_email.as_deref().unwrap_or(&email);
        repos.launcher_accounts.find_by_email(tx.conn(), target)?
    };

    if account.is_none()
        && let Some(ref from) = from_email
        && from != &email
    {
        account = repos.launcher_accounts.find_by_email(tx.conn(), &email)?;
    }

    let Some(account) = account else {
        let selector = if let Some(uname) = username {
            format!("username={uname}")
        } else {
            format!("email={}", from_email.as_deref().unwrap_or(&email))
        };
        return Err(BootstrapError::AccountNotFound { selector });
    };

    if account.password_hash.is_none() {
        return Err(BootstrapError::NotPasswordAccount);
    }
    if account.username_set == Some(false) {
        return Err(BootstrapError::UsernameNotSet);
    }

    if let Some(existing) = repos.launcher_accounts.find_by_email(tx.conn(), &email)?
        && existing.account != account.account
    {
        return Err(BootstrapError::EmailCollision { email });
    }

    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut roles = account.roles.clone();
    if !roles.iter().any(|r| r == "admin") {
        roles.push("admin".to_owned());
    }
    let roles_json = serde_json::to_string(&roles).map_err(|e| RepoError::Data(e.to_string()))?;

    diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(account.account.to_string())))
        .set((
            launcher_accounts::email.eq(Some(&email)),
            launcher_accounts::roles.eq(roles_json),
        ))
        .execute(tx.conn())
        .map_err(RepoError::from)?;

    repos.launcher_accounts.set_access_state(
        tx,
        &account.account,
        &AccessChange {
            approval_status: Some(ApprovalStatus::Approved),
            status: Some(OperationalStatus::Active),
            approval_updated_at: now,
            approval_updated_by: "bootstrap-script".to_owned(),
            approval_reason: Some("Initial administrator bootstrap".to_owned()),
        },
    )?;

    Ok(BootstrapAdminOutcome {
        user_id: account.user_id,
        email,
    })
}
