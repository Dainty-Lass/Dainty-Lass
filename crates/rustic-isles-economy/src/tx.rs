// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! One store transaction whose closure may end in a domain error: the error rolls the transaction back and comes
//! out typed.

use std::sync::{Arc, Mutex, PoisonError};

use module_api::ActorContext;
use rustic_isles_store::{AccountId, IslesStore, RepoError, Repositories, Tx};

/// `f` in one `account_write` (with `account`) or plain `write`; an `Err` from `f` rolls everything back.
pub(crate) async fn guarded<T, E, F>(
    store: &IslesStore,
    account: Option<&AccountId>,
    actor: &ActorContext,
    f: F,
) -> Result<T, E>
where
    T: Send + 'static,
    E: From<RepoError> + Send + 'static,
    F: FnOnce(&mut Tx<'_>, &Repositories) -> Result<T, E> + Send + 'static,
{
    let held: Arc<Mutex<Option<E>>> = Arc::new(Mutex::new(None));
    let inner = Arc::clone(&held);
    let wrapped = move |tx: &mut Tx<'_>, repos: &Repositories| match f(tx, repos) {
        Ok(value) => Ok(value),
        Err(error) => {
            *inner.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
            Err(RepoError::Data(String::from("rolled back")))
        }
    };
    let outcome = match account {
        Some(account) => store.account_write(account, actor, wrapped).await,
        None => store.write(actor, wrapped).await,
    };
    match outcome {
        Ok(value) => Ok(value),
        Err(error) => match held.lock().unwrap_or_else(PoisonError::into_inner).take() {
            Some(domain) => Err(domain),
            None => Err(E::from(error)),
        },
    }
}
