// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The seam between the 1.14.7 module and the Rustic Rifts crates: the hooks the module calls, what it provides back,
//! routes as data, node-tagged ids, storage shapes, and the dedicated-only answers used while the player-hosted module
//! is off. It names no web framework and runs no executor.

mod core;
mod dedicated;
mod error;
mod hooks;
mod http;
mod id;
mod store;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

pub use crate::core::{GuardMode, HostGrantCatalog, IslesCore, NoCore, Party};
pub use crate::dedicated::{DedicatedOnly, Rifts, RiftsParts, p2p_module_enabled};
pub use crate::error::{Component, Refusal, SeamError, Unimplemented};
pub use crate::hooks::{
    AuthOutcome, Candidate, CandidateLeave, CandidateLeaveOutcome, GuardHooks, HostRouteSample, HostWrite,
    HostWriteKind, HostWriteOptions, HubKind, HuntRouteContext, HuntRouteOutcome, InventoryDecision, InventoryFence,
    InventoryTransaction, Lifecycle, OperationsOverview, RealmEntry, RealmsEntry, RealmsHooks, ReleaseGate, Settings,
    TransactionPhase, TransactionReads,
};
pub use crate::http::{
    BoxFuture, Caller, CapabilityScope, HOST_CAPABILITY_HEADER, Json, Method, Mount, Reply, RequestInfo, Route,
    RouteCall, RouteHandler, RuntimeAuth, StubRoutes,
};
pub use crate::id::{
    AccountId, AdmissionId, GlobalId, IdError, NodeId, PartyId, PeerId, RealmId, SessionId, TicketId, Ulid,
};
pub use crate::store::{Actor, DeletionAction, DeletionStep, Secret, StoreError, TableSpec, UnixMillis};

/// An already-failed future for an operation a component has not written yet.
pub fn stub<T, E>(component: Component, operation: &'static str) -> BoxFuture<'static, Result<T, E>>
where
    T: Send + 'static,
    E: From<Unimplemented> + Send + 'static,
{
    Box::pin(std::future::ready(Err(E::from(Unimplemented::new(
        component, operation,
    )))))
}

/// Polls `future` once with a waker that does nothing, so sync callers and tests can take a ready answer without a
/// runtime.
pub fn poll_once<F: Future>(future: F) -> Option<F::Output> {
    let future = pin!(future);
    match future.poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(output) => Some(output),
        Poll::Pending => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_is_ready_with_the_operation() {
        let answer: Result<(), SeamError> = poll_once(stub(Component::Guard, "guard.census")).unwrap();
        assert_eq!(answer, Err(SeamError::unimplemented(Component::Guard, "guard.census")));
        let store: Result<u8, StoreError> = poll_once(stub(Component::Realms, "store.load")).unwrap();
        assert_eq!(
            store,
            Err(StoreError::Unimplemented(Unimplemented::new(
                Component::Realms,
                "store.load"
            )))
        );
    }

    #[test]
    fn poll_once_leaves_a_pending_future() {
        assert_eq!(poll_once(std::future::pending::<u8>()), None);
        assert_eq!(poll_once(async { 7 }), Some(7));
    }
}
