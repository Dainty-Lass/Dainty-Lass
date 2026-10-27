//! The reads and writes the auth code does over and over, so the controllers say what they mean: a launcher account
//! by the id the client knows (`userId`), and the actors a write is logged under (DD23).

use module_api::{Actor, ActorContext};

use crate::db::IslesStore;
use crate::http::Shared;
use crate::repo::auth::{AccountRecord, LauncherAccountRecord};
use crate::repo::{AccountId, RepoError, RepoResult, Repositories, Tx, UserId};

/// The database, or the error Node's repositories would throw without one (`GetRepositories()` before persistence).
pub fn store(shared: &Shared) -> RepoResult<&IslesStore> {
    shared
        .store
        .as_ref()
        .ok_or_else(|| RepoError::Data("no database is configured".into()))
}

/// A player acting on their own account.
pub fn player_actor(account: &AccountId) -> ActorContext {
    ActorContext::new(Actor::Player {
        account: account.global().clone(),
    })
}

/// The server itself: a sweep, a startup task, a login that has no account yet.
pub fn system_actor() -> ActorContext {
    ActorContext::new(Actor::System)
}

/// `launcherAccounts.findByUserId`.
pub async fn launcher_by_user(shared: &Shared, user_id: &str) -> RepoResult<Option<LauncherAccountRecord>> {
    let user = UserId(user_id.to_owned());
    store(shared)?
        .read(move |conn, repos| repos.launcher_accounts.find_by_user_id(conn, &user))
        .await
}

/// `launcherAccounts.findByEmail`.
pub async fn launcher_by_email(shared: &Shared, normalized: &str) -> RepoResult<Option<LauncherAccountRecord>> {
    let email = normalized.to_owned();
    store(shared)?
        .read(move |conn, repos| repos.launcher_accounts.find_by_email(conn, &email))
        .await
}

/// `launcherAccounts.findByDisplayNameNormalized`.
pub async fn launcher_by_display_name(shared: &Shared, normalized: &str) -> RepoResult<Option<LauncherAccountRecord>> {
    let name = normalized.to_owned();
    store(shared)?
        .read(move |conn, repos| repos.launcher_accounts.find_by_display_name_normalized(conn, &name))
        .await
}

/// `accounts.findByUserId`.
pub async fn account_by_user(shared: &Shared, user_id: &str) -> RepoResult<Option<AccountRecord>> {
    let user = UserId(user_id.to_owned());
    store(shared)?
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await
}

/// A write to one account's data, as that player (DD17: refused unless this node is the account's home).
pub async fn account_write<T, F>(shared: &Shared, account: &AccountId, f: F) -> RepoResult<T>
where
    F: FnOnce(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Send + 'static,
    T: Send + 'static,
{
    store(shared)?.account_write(account, &player_actor(account), f).await
}
