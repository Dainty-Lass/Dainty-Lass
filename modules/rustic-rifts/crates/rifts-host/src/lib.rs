//! The client side of realms (RF7), linked by DaintyLass's launcher with its realms feature (DD12): hosting and
//! joining, the session folder, and the transport it loads.
//!
//! Nothing here opens a listener. The transport binds what its `bridge.conf` says (ICE on libjuice), and the hosted
//! game server listens on 127.0.0.1 only (`PrepareListen`).

mod files;
mod session;
mod transport;

pub use files::{ADMISSION_PUBLIC_KEY_FILE, HostState, SESSION_FILES, TRANSPORT_DLL};
pub use session::{CLEARED_VARIABLES, HostedRole, HubLimits, LAUNCH_VARIABLES, LocalSession, LocalState, RealmHosting};
pub use transport::{
    ABI_VERSION, EXPORTS, Handle, HostOpen, JoinOpen, OPTIONAL_EXPORTS, ReadyState, SessionResult, Signal,
    TransportAbi, TransportLoader,
};

use std::path::Path;

use rifts_seam::{AccountId, BoxFuture, Component, HubKind, PeerId, Secret, SessionId, Unimplemented, stub};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
}

/// Every operation answers [`Unimplemented`] until RF7 (RD18).
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

const COMPONENT: Component = Component::Host;

fn refuse<T>(operation: &'static str) -> Result<T, HostError> {
    Err(Unimplemented::new(COMPONENT, operation).into())
}

impl TransportLoader for Stub {
    fn load(&self, _: &Path, _: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError> {
        refuse("transport.load")
    }
}

impl TransportAbi for Stub {
    fn abi_version(&self) -> Result<u32, HostError> {
        refuse("transport.abi_version")
    }
    fn initialize(&self) -> Result<(), HostError> {
        refuse("transport.initialize")
    }
    fn shutdown(&self) -> Result<(), HostError> {
        refuse("transport.shutdown")
    }
    fn peer_id(&self) -> Result<PeerId, HostError> {
        refuse("transport.peer_id")
    }
    fn networking_ready(&self) -> Result<bool, HostError> {
        refuse("transport.networking_ready")
    }
    fn host_open(&self, _: &HostOpen) -> Result<Handle, HostError> {
        refuse("transport.host_open")
    }
    fn join_open(&self, _: &JoinOpen) -> Result<Handle, HostError> {
        refuse("transport.join_open")
    }
    fn wait_ready(&self, _: Handle, _: u32) -> Result<ReadyState, HostError> {
        refuse("transport.wait_ready")
    }
    fn session_running(&self, _: Handle) -> Result<bool, HostError> {
        refuse("transport.session_running")
    }
    fn stop_session(&self, _: Handle) -> Result<(), HostError> {
        refuse("transport.stop_session")
    }
    fn last_session_result(&self, _: Handle) -> Result<SessionResult, HostError> {
        refuse("transport.last_session_result")
    }
    fn read_signal(&self, _: Handle) -> Result<Option<Signal>, HostError> {
        refuse("transport.read_signal")
    }
    fn submit_signal(&self, _: Handle, _: &AccountId, _: &str, _: &str) -> Result<(), HostError> {
        refuse("transport.submit_signal")
    }
    fn read_peer_event(&self, _: Handle) -> Result<Option<String>, HostError> {
        refuse("transport.read_peer_event")
    }
    fn read_diagnostic(&self, _: Handle) -> Result<Option<String>, HostError> {
        refuse("transport.read_diagnostic")
    }
    fn add_peer(&self, _: Handle, _: &AccountId, _: PeerId, _: &Secret) -> Result<(), HostError> {
        refuse("transport.add_peer")
    }
    fn remove_peer(&self, _: Handle, _: &AccountId, _: &str) -> Result<(), HostError> {
        refuse("transport.remove_peer")
    }
    fn update_relay_credential(&self, _: Handle, _: &str, _: &Secret) -> Result<(), HostError> {
        refuse("transport.update_relay_credential")
    }
}

impl RealmHosting for Stub {
    fn coop_available(&self) -> BoxFuture<'_, Result<bool, HostError>> {
        stub(COMPONENT, "coop_available")
    }
    fn host_hunt(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        stub(COMPONENT, "host_hunt")
    }
    fn join_hunt(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        stub(COMPONENT, "join_hunt")
    }
    fn start_hub(&self, _: AccountId, _: HubKind, _: HubLimits) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        stub(COMPONENT, "start_hub")
    }
    fn stop_hub(&self, _: AccountId, _: HubKind) -> BoxFuture<'_, Result<(), HostError>> {
        stub(COMPONENT, "stop_hub")
    }
    fn stop(&self, _: SessionId) -> BoxFuture<'_, Result<(), HostError>> {
        stub(COMPONENT, "stop")
    }
    fn sessions(&self) -> BoxFuture<'_, Result<Vec<LocalSession>, HostError>> {
        stub(COMPONENT, "sessions")
    }
}

impl HostState for Stub {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError> {
        refuse("state.hub_limits")
    }
    fn save_hub_limits(&self, _: &HubLimits) -> Result<(), HostError> {
        refuse("state.save_hub_limits")
    }
    fn control_cursor(&self, _: &AccountId) -> Result<Option<u64>, HostError> {
        refuse("state.control_cursor")
    }
    fn set_control_cursor(&self, _: &AccountId, _: u64) -> Result<(), HostError> {
        refuse("state.set_control_cursor")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::poll_once;
    use std::collections::BTreeSet;

    #[test]
    fn exports_are_the_headers() {
        let header = include_str!("../../../src/paradox_transport_api.hpp");
        let declared: BTreeSet<&str> = header
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|word| word.starts_with("MysticTransport_"))
            .collect();
        assert_eq!(declared, EXPORTS.into_iter().collect());
        assert!(OPTIONAL_EXPORTS.iter().all(|name| !declared.contains(name)));
    }

    #[test]
    fn session_results_keep_their_codes() {
        assert_eq!(SessionResult::from_code(0), SessionResult::Clean);
        assert_eq!(SessionResult::from_code(4), SessionResult::RefusedAdmission);
        assert_eq!(SessionResult::from_code(-1), SessionResult::UnknownHandle);
        assert_eq!(SessionResult::from_code(9), SessionResult::Other(9));
    }

    #[test]
    fn stub_refuses_everything() {
        assert_eq!(
            Stub.load(Path::new(TRANSPORT_DLL), [0; 32]).err(),
            refuse::<()>("transport.load").err()
        );
        assert_eq!(Stub.abi_version(), refuse("transport.abi_version"));
        assert_eq!(
            Stub.update_relay_credential(Handle(1), "u", &Secret::new("c".to_owned())),
            refuse("transport.update_relay_credential")
        );
        assert_eq!(poll_once(Stub.coop_available()).unwrap(), refuse("coop_available"));
        assert_eq!(Stub.hub_limits(), refuse("state.hub_limits"));
    }
}
