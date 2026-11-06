// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Where a [`TransportAbi`] comes from. The transport is linked in (umbrella spec D1): nothing is loaded from disk, so
//! there is no symbol table, no ABI handshake with a file and no `unsafe`.

use std::fs;
use std::path::Path;

use sha2::{Digest as _, Sha256};

use crate::HostError;
use crate::transport::{TransportAbi, TransportLoader};

/// The SHA-256 of a regular, non-empty file, read without following a link.
pub fn sha256_file(path: &Path) -> Result<[u8; 32], HostError> {
    let shown = path.display();
    let meta =
        fs::symlink_metadata(path).map_err(|e| HostError::Io(format!("transport artifact missing ({shown}): {e}")))?;
    if !meta.file_type().is_file() || meta.len() == 0 {
        return Err(HostError::InvalidInput(format!(
            "transport artifact is not a regular non-empty file: {shown}"
        )));
    }
    let bytes = fs::read(path).map_err(|e| HostError::Io(format!("could not read {shown}: {e}")))?;
    Ok(Sha256::digest(&bytes).into())
}

/// The launcher's loader. With `in-process` it hands out the linked transport and ignores the path and digest, since
/// the transport now ships inside the signed launcher (D1); without it every load is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeTransportLoader;

impl TransportLoader for NativeTransportLoader {
    fn load(&self, _dll: &Path, _expected_sha256: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError> {
        #[cfg(feature = "in-process")]
        {
            Ok(Box::new(in_process::InProcessTransport::new()))
        }
        #[cfg(not(feature = "in-process"))]
        {
            Err(HostError::Transport(
                "this build has no co-op transport (feature in-process is off)".to_owned(),
            ))
        }
    }
}

#[cfg(feature = "in-process")]
pub mod in_process {
    use rifts_seam::{AccountId, PeerId, Secret};
    use rifts_transport::{Ready, Transport};

    use crate::HostError;
    use crate::transport::{Handle, HostOpen, JoinOpen, ReadyState, SessionResult, Signal, TransportAbi};

    /// [`TransportAbi`] over `rifts_transport::Transport`, with ABI 3's error texts.
    #[derive(Debug, Clone, Default)]
    pub struct InProcessTransport {
        transport: Transport,
    }

    impl InProcessTransport {
        /// Wraps a new transport.
        pub fn new() -> Self {
            Self::default()
        }

        /// Wraps an existing transport.
        pub fn from_transport(transport: Transport) -> Self {
            Self { transport }
        }

        /// The transport underneath.
        pub fn transport(&self) -> &Transport {
            &self.transport
        }
    }

    fn handle(handle: Handle) -> rifts_transport::Handle {
        rifts_transport::Handle(handle.0)
    }

    fn or_default(text: String, default: &str) -> String {
        let text = text.trim();
        if text.is_empty() {
            default.to_owned()
        } else {
            text.to_owned()
        }
    }

    // The C ABI refused strings it could not pass; the same inputs are refused here so callers see one contract.
    fn no_nul(value: &str, label: &str) -> Result<(), HostError> {
        if value.contains('\0') {
            Err(HostError::InvalidInput(format!(
                "{label} contains an interior NUL byte"
            )))
        } else {
            Ok(())
        }
    }

    fn line(text: Option<String>) -> Option<String> {
        text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
    }

    impl TransportAbi for InProcessTransport {
        fn abi_version(&self) -> Result<u32, HostError> {
            Ok(self.transport.abi_version())
        }

        fn initialize(&self) -> Result<(), HostError> {
            self.transport
                .initialize()
                .map_err(|e| HostError::Transport(or_default(e, "transport initialization failed")))
        }

        fn shutdown(&self) -> Result<(), HostError> {
            self.transport.shutdown();
            Ok(())
        }

        fn peer_id(&self) -> Result<PeerId, HostError> {
            match self.transport.peer_id() {
                0 => Err(HostError::Transport(
                    "transport peer id is 0 (not initialized)".to_owned(),
                )),
                id => Ok(PeerId(id)),
            }
        }

        fn networking_ready(&self) -> Result<bool, HostError> {
            Ok(self.transport.networking_ready())
        }

        fn host_open(&self, open: &HostOpen) -> Result<Handle, HostError> {
            self.transport
                .host_open(
                    &open.public_key,
                    &open.config,
                    &open.capability,
                    open.host_ticket.as_deref(),
                    &open.secrets,
                )
                .map(|h| Handle(h.0))
                .map_err(|e| HostError::Transport(or_default(e, "transport host_open failed")))
        }

        fn join_open(&self, open: &JoinOpen) -> Result<Handle, HostError> {
            self.transport
                .join_open(&open.public_key, &open.config, &open.ticket, &open.secrets)
                .map(|h| Handle(h.0))
                .map_err(|e| HostError::Transport(or_default(e, "transport join_open failed")))
        }

        fn wait_ready(&self, h: Handle, timeout_ms: u32) -> Result<ReadyState, HostError> {
            Ok(match self.transport.wait_ready(handle(h), timeout_ms) {
                Ready::Ready(text) => ReadyState::Ready(text.trim().to_owned()),
                Ready::TimedOut => ReadyState::TimedOut,
                Ready::Ended => ReadyState::Ended,
            })
        }

        fn session_running(&self, h: Handle) -> Result<bool, HostError> {
            Ok(self.transport.session_running(handle(h)))
        }

        fn stop_session(&self, h: Handle) -> Result<(), HostError> {
            if self.transport.stop_session(handle(h)) {
                Ok(())
            } else {
                Err(HostError::NotFound(format!("unknown session handle {}", h.0)))
            }
        }

        fn last_session_result(&self, h: Handle) -> Result<SessionResult, HostError> {
            Ok(SessionResult::from_code(self.transport.last_session_result(handle(h))))
        }

        fn read_signal(&self, h: Handle) -> Result<Option<Signal>, HostError> {
            // Data is left as written: ICE descriptions end in CRLF.
            Ok(self.transport.read_signal(handle(h)).map(|s| Signal {
                to_account: AccountId(s.to.trim().to_owned()),
                kind: s.kind.trim().to_owned(),
                data: s.data,
            }))
        }

        fn submit_signal(&self, h: Handle, from: &AccountId, kind: &str, data: &str) -> Result<(), HostError> {
            no_nul(&from.0, "from_account")?;
            no_nul(kind, "kind")?;
            no_nul(data, "data")?;
            if self.transport.submit_signal(handle(h), &from.0, kind, data) {
                Ok(())
            } else {
                Err(HostError::Transport("transport refused incoming signal".to_owned()))
            }
        }

        fn read_peer_event(&self, h: Handle) -> Result<Option<String>, HostError> {
            Ok(line(self.transport.read_peer_event(handle(h))))
        }

        fn read_diagnostic(&self, h: Handle) -> Result<Option<String>, HostError> {
            Ok(line(self.transport.read_diagnostic(handle(h))))
        }

        fn add_peer(&self, h: Handle, account: &AccountId, peer: PeerId, pair_key: &Secret) -> Result<(), HostError> {
            no_nul(&account.0, "account")?;
            no_nul(pair_key.expose(), "pair_key")?;
            if self
                .transport
                .add_peer(handle(h), &account.0, peer.0, pair_key.expose())
            {
                Ok(())
            } else {
                Err(HostError::Transport("transport refused peer addition".to_owned()))
            }
        }

        fn remove_peer(&self, h: Handle, account: &AccountId, reason: &str) -> Result<(), HostError> {
            no_nul(&account.0, "account")?;
            no_nul(reason, "reason")?;
            if self.transport.remove_peer(handle(h), &account.0, Some(reason)) {
                Ok(())
            } else {
                Err(HostError::Transport("transport refused peer removal".to_owned()))
            }
        }

        /// The in-process transport has no credential update, so it keeps the one it started with (ABI 3's optional
        /// export absent).
        fn update_relay_credential(&self, _h: Handle, username: &str, credential: &Secret) -> Result<(), HostError> {
            no_nul(username, "username")?;
            no_nul(credential.expose(), "credential")?;
            Ok(())
        }
    }
}
