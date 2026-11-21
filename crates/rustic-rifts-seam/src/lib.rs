// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The seam between the 1.14.7 module and the Rustic Rifts crates: the hooks the module calls, what it provides back,
//! routes as data, node-tagged ids, storage shapes, and the dedicated-only answers used while the player-hosted module
//! is off. It names no web framework and runs no executor.

mod error;
mod http;
mod id;
mod store;

mod core;
mod dedicated;
mod hooks;

use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

pub use crate::{
    core::{GuardMode, HostGrantCatalog, IslesCore, NoCore, Party},
    dedicated::{DedicatedOnly, Rifts, RiftsParts, p2p_module_enabled},
    error::{Component, Refusal, SeamError, Unimplemented},
    hooks::{
        AuthOutcome, Candidate, CandidateLeave, CandidateLeaveOutcome, GuardHooks, HostRouteSample, HostWrite,
        HostWriteKind, HostWriteOptions, HubKind, HuntRouteContext, HuntRouteOutcome, InventoryDecision,
        InventoryFence, InventoryTransaction, Lifecycle, OperationsOverview, RealmEntry, RealmsEntry, RealmsHooks,
        ReleaseGate, Settings, TransactionPhase, TransactionReads,
    },
    http::{
        ADMISSION_KEY_PATH, BoxFuture, Caller, CapabilityScope, HOST_CAPABILITY_HEADER, Json, Method, Mount, Reply,
        RequestInfo, Route, RouteCall, RouteHandler, RuntimeAuth, StubRoutes,
    },
    id::{AccountId, AdmissionId, GlobalId, IdError, NodeId, PartyId, PeerId, RealmId, SessionId, TicketId, Ulid},
    store::{Actor, DeletionAction, DeletionStep, Secret, StoreError, TableSpec, UnixMillis},
};

/// An already-failed future for an operation a component has not written yet.
pub fn stub<T, E>(component: Component, operation: &'static str) -> BoxFuture<'static, Result<T, E>>
where
    T: Send + 'static,
    E: From<Unimplemented> + Send + 'static,
{
    let missing = Unimplemented::new(component, operation);
    Box::pin(std::future::ready(Err(missing.into())))
}

/// Polls `future` once with a waker that does nothing, so sync callers and tests can take a ready answer without a
/// runtime.
pub fn poll_once<F: Future>(future: F) -> Option<F::Output> {
    let mut context = Context::from_waker(Waker::noop());
    if let Poll::Ready(output) = pin!(future).poll(&mut context) {
        Some(output)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_is_ready_with_the_operation() {
        let seam: Option<Result<(), SeamError>> = poll_once(stub(Component::Guard, "guard.census"));
        assert_eq!(
            seam,
            Some(Err(SeamError::unimplemented(Component::Guard, "guard.census")))
        );
        let store: Option<Result<u8, StoreError>> = poll_once(stub(Component::Realms, "store.load"));
        let missing = Unimplemented::new(Component::Realms, "store.load");
        assert_eq!(store, Some(Err(StoreError::Unimplemented(missing))));
    }

    #[test]
    fn poll_once_leaves_a_pending_future() {
        assert!(poll_once(std::future::pending::<u8>()).is_none());
        assert_eq!(poll_once(async { 7 }), Some(7));
    }
}
