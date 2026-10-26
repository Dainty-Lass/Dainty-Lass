//! Realms (RF5): the realm directory, player-hosted hunts and realm hubs, host capabilities, leases, host selection
//! and the launcher's control channel, on `store`.
//!
//! Ported from `realms/` and most of `p2p/` (RF1). This crate answers the `P2PExtension` hooks RF1 gives realms
//! ([`rifts_seam::RealmsHooks`]) and the `realms/index.ts` exports ([`rifts_seam::RealmsEntry`]). RF1's ambiguous
//! splits land here as its tables have them: `host-ready` and `host-bootstrap` (the capability is realms'; the host
//! ticket comes from rifts-admission), the census goes to rifts-guard.

mod directory;
mod hosting;
mod routes;
mod store;
mod stub;

pub use directory::{InviteOptions, JoinRealm, MemberAction, NewRealm, RealmDirectory, RealmSettings};
pub use hosting::{
    ControlChannel, HostBootstrap, HostCapabilities, HostPresence, HostSelection, HuntLifecycle, HuntStatus, Leases,
    Observation, StartHub,
};
pub use routes::{ROUTES, stub_routes};
pub use store::{
    ControlMessageRecord, HostCapabilityRecord, HostOfferRecord, HuntSessionRecord, HuntState, HuntStore, JoinPolicy,
    OfferStatus, RealmInviteRecord, RealmPresenceRecord, RealmRecord, RealmStore, SessionKind, StubRealmsStore, TABLES,
};
pub use stub::Stub;

use rifts_seam::{Refusal, Unimplemented};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RealmsError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    /// A `REALM_*` or `P2P_*` launcher error code.
    #[error(transparent)]
    Refused(#[from] Refusal),
}
