//! The seam between rustic-isles' 1.14.7 module and realms (RF2).
//!
//! Node's backend reaches realms through the `P2PExtension` hook object (`extensions/p2p.ts`), the `realms/` exports
//! and the anticheat hooks (`p2p/antiCheat.ts`). Here those become [`RealmsHooks`], [`RealmsEntry`] and
//! [`GuardHooks`], plus [`RouteHandler`] for the routes and [`IslesCore`] for what realms take from the core
//! (RF1's inventory, section 2). `isles-1-14-7` depends on this crate only; it gets [`Rifts::dedicated_only`] without
//! the component crates, and their stubs with them.

mod core;
mod dedicated;
mod error;
mod hooks;
mod http;
mod id;
mod store;

pub use crate::core::{GuardMode, IslesCore, NoCore, Party};
pub use dedicated::{DedicatedOnly, Rifts, RiftsParts, p2p_module_enabled};
pub use error::{Component, Refusal, SeamError, Unimplemented};
pub use hooks::{
    AuthOutcome, Candidate, CandidateLeave, CandidateLeaveOutcome, GuardHooks, HostWrite, HostWriteKind,
    HostWriteOptions, HubKind, HuntRouteContext, HuntRouteOutcome, InventoryDecision, InventoryFence,
    InventoryTransaction, Lifecycle, OperationsOverview, RealmEntry, RealmsEntry, RealmsHooks, ReleaseGate, Settings,
    TransactionPhase,
};
pub use http::{
    BoxFuture, Caller, CapabilityScope, HOST_CAPABILITY_HEADER, Json, Method, Mount, Reply, RequestInfo, Route,
    RouteCall, RouteHandler, RuntimeAuth, StubRoutes,
};
pub use id::{AccountId, AdmissionId, GlobalId, IdError, NodeId, PartyId, PeerId, RealmId, SessionId, TicketId, Ulid};
pub use store::{Actor, DeletionAction, DeletionStep, Secret, StoreError, TableSpec, UnixMillis};

/// What a stub's async method returns: [`Unimplemented`] for this component and operation (RD18).
pub fn stub<T, E>(component: Component, operation: &'static str) -> BoxFuture<'static, Result<T, E>>
where
    T: Send + 'static,
    E: From<Unimplemented> + Send + 'static,
{
    Box::pin(async move { Err(Unimplemented::new(component, operation).into()) })
}

/// Polls a future once, with no runtime: stubs and the dedicated-only default never wait, so their tests need none.
pub fn poll_once<F: std::future::Future>(future: F) -> Option<F::Output> {
    use std::task::{Context, Poll, Waker};
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}
