//! The 30-day erasure of an account whose deletion hold has ended (`accountDeletion/erasure.ts`, run by
//! `scripts/process_account_deletions.ts`, Privacy Policy section 5): the sessions, codes, links and keys of the
//! account, its friend edges, and the launcher and game accounts themselves, in one transaction.
//!
//! Account rows are soft-deleted (DD23), so the host's `revert` can bring one back; the change log keeps their
//! before-images until the retention purge (`Database::purge_soft_deleted`), which is the erasure's last step.
//! What this area cannot erase is the other areas' data (characters, inventory, wallet, progression...): those are
//! their repositories' deletions, which an [`ErasureSeam`] adds to the same transaction. Nothing runs this on a timer:
//! Node's script needed an operator's `--apply --confirm`, and so does this, from the host's console or the admin API.

use module_api::BoxFuture;

use super::data::{account_write, store};
use super::errors::LauncherApiError;
use crate::http::Shared;
use crate::repo::auth::LauncherAccountRecord;
use crate::repo::{AccountId, RepoResult, Repositories, Tx};

/// What the other areas erase of an account in the same transaction, and what blocks the erasure (live activity: a
/// hunt, an online realm). Defaults to nothing.
pub trait ErasureSeam: Send + Sync {
    /// Reasons the account cannot be erased now; the run skips it and reports them
    fn blockers<'a>(&'a self, _shared: &'a Shared, _account: &'a AccountId) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async { Vec::new() })
    }

    /// Removes or pseudonymises the area's rows of the account; how many
    fn erase(&self, _tx: &mut Tx<'_>, _repos: &Repositories, _account: &AccountId) -> RepoResult<u64> {
        Ok(0)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoOtherData;

impl ErasureSeam for NoOtherData {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasureOutcome {
    /// Live activity: the account is left for the next run
    Blocked(Vec<String>),
    /// `apply` was false: nothing changed
    DryRun,
    /// Rows removed, by what they were
    Erased(Vec<(&'static str, u64)>),
}

/// `listDeletionsDue`: the accounts whose hold has ended by `now`, oldest first.
pub async fn due_deletions(
    shared: &Shared,
    now_iso: &str,
    limit: u32,
) -> Result<Vec<LauncherAccountRecord>, LauncherApiError> {
    let now = now_iso.to_owned();
    Ok(store(shared)?
        .read(move |conn, repos| repos.launcher_accounts.list_deletions_due(conn, &now, limit))
        .await?)
}

/// `EraseAccount`: an account with a deletion request whose hold has ended. `apply` false only reports.
pub async fn erase_account(
    shared: &Shared,
    account: &LauncherAccountRecord,
    apply: bool,
) -> Result<ErasureOutcome, LauncherApiError> {
    let Some(deletion) = &account.deletion else {
        return Err(LauncherApiError::new(
            "INTERNAL",
            "EraseAccount needs an account with a deletion request",
        ));
    };
    if super::util::parse_ms(&deletion.scheduled_for).is_none_or(|due| due > super::util::now_ms()) {
        return Err(LauncherApiError::new(
            "DELETION_HOLD_EXPIRED",
            "The hold has not ended yet.",
        ));
    }
    let seam = shared.auth.erasure.clone();
    let blockers = seam.blockers(shared, &account.account).await;
    if !blockers.is_empty() {
        return Ok(ErasureOutcome::Blocked(blockers));
    }
    if !apply {
        return Ok(ErasureOutcome::DryRun);
    }
    let id = account.account.clone();
    let counts = account_write(shared, &account.account, move |tx, repos| {
        repos.refresh_sessions.delete_all_for_account(tx, &id)?;
        let mut counts: Vec<(&'static str, u64)> = vec![
            ("emailCodes", repos.email_codes.delete_for_account(tx, &id)?),
            (
                "gameExchangeCodes",
                repos.game_exchange_codes.delete_for_account(tx, &id)?,
            ),
            (
                "discordOAuthTransactions",
                repos.discord_oauth_transactions.delete_for_account(tx, &id)?,
            ),
            ("authIdentities", repos.auth_identities.delete_for_account(tx, &id)?),
            ("userApiKeys", u64::from(repos.api_keys.delete_user_key_hash(tx, &id)?)),
            ("friendships", repos.friendships.remove_all_for_account(tx, &id)?),
            ("otherAreas", seam.erase(tx, repos, &id)?),
        ];
        counts.push(("accounts", u64::from(repos.launcher_accounts.erase(tx, &id)?)));
        Ok(counts)
    })
    .await?;
    tracing::info!("Account {} erased", account.user_id);
    Ok(ErasureOutcome::Erased(counts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::areas::auth::test_support::TestServer;

    #[tokio::test(flavor = "multi_thread")]
    async fn an_account_past_its_hold_is_erased_with_its_sessions_codes_and_links() {
        let server = TestServer::new(&[]);
        let account = server
            .add_launcher_account_with("Leaver", Some("leaver@example.test"), Some("password-123"), |a| {
                a.deletion = Some(crate::repo::auth::LauncherDeletion {
                    requested_at: "2026-09-01T00:00:00.000Z".into(),
                    scheduled_for: "2026-10-01T00:00:00.000Z".into(),
                });
            })
            .await;
        let stay = server
            .add_launcher_account("Stayer", "stayer@example.test", "password-123")
            .await;
        server.launcher_session(&account).await;
        server.user_key(&account, "RRK_leaver").await;
        let due = due_deletions(&server.shared, "2026-10-27T00:00:00.000Z", 10)
            .await
            .unwrap();
        assert_eq!(
            due.iter().map(|a| a.user_id.clone()).collect::<Vec<_>>(),
            vec![account.user_id.clone()]
        );

        assert_eq!(
            erase_account(&server.shared, &due[0], false).await.unwrap(),
            ErasureOutcome::DryRun
        );
        let ErasureOutcome::Erased(counts) = erase_account(&server.shared, &due[0], true).await.unwrap() else {
            panic!("not erased")
        };
        assert!(
            counts.contains(&("userApiKeys", 1)) && counts.contains(&("accounts", 1)),
            "{counts:?}"
        );
        // The account is gone from every lookup; another one is not touched
        assert!(
            super::super::data::launcher_by_user(&server.shared, account.user_id.as_str())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            super::super::data::account_by_user(&server.shared, account.user_id.as_str())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            super::super::data::launcher_by_user(&server.shared, stay.user_id.as_str())
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            super::super::apikeys::user_id_for_api_key(&server.shared, "RRK_leaver")
                .await
                .unwrap()
                .is_none()
        );
        // Its address and name are free again
        assert!(
            server
                .add_launcher_account("Leaver", "leaver@example.test", "password-123")
                .await
                .display_name
                == "Leaver"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hold_that_has_not_ended_is_refused_and_live_activity_blocks() {
        struct Busy;
        impl ErasureSeam for Busy {
            fn blockers<'a>(&'a self, _: &'a Shared, _: &'a AccountId) -> BoxFuture<'a, Vec<String>> {
                Box::pin(async { vec!["a realm it hosts is online".to_owned()] })
            }
        }
        let mut server = TestServer::new(&[]);
        let future = crate::areas::auth::util::iso_from_ms(crate::areas::auth::util::now_ms() + 86_400_000);
        let waiting = server
            .add_launcher_account_with("Waiting", Some("waiting@example.test"), None, |a| {
                a.deletion = Some(crate::repo::auth::LauncherDeletion {
                    requested_at: "2026-09-01T00:00:00.000Z".into(),
                    scheduled_for: future.clone(),
                });
            })
            .await;
        assert_eq!(
            erase_account(&server.shared, &waiting, true).await.unwrap_err().code,
            "DELETION_HOLD_EXPIRED"
        );
        let due = server
            .add_launcher_account_with("Due", Some("due@example.test"), None, |a| {
                a.deletion = Some(crate::repo::auth::LauncherDeletion {
                    requested_at: "2026-09-01T00:00:00.000Z".into(),
                    scheduled_for: "2026-10-01T00:00:00.000Z".into(),
                });
            })
            .await;
        std::sync::Arc::get_mut(&mut server.shared).unwrap().auth.erasure = std::sync::Arc::new(Busy);
        assert_eq!(
            erase_account(&server.shared, &due, true).await.unwrap(),
            ErasureOutcome::Blocked(vec!["a realm it hosts is online".to_owned()])
        );
        assert!(
            super::super::data::launcher_by_user(&server.shared, due.user_id.as_str())
                .await
                .unwrap()
                .is_some()
        );
    }
}
