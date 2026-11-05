//! Dynamic loading of `ParadoxTransport.dll` / `.so` (`TransportLoader` and `TransportAbi`).
//!
//! This is the only module in `rifts-host` allowed `unsafe_code`: it verifies the transport file's SHA-256 before
//! handing the path to `libloading`, resolves all 29 ABI 3 exports in [`EXPORTS`] before the first call, probes
//! [`OPTIONAL_EXPORTS`], and wraps each C entry point behind [`TransportAbi`].
#![allow(unsafe_code)]

use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use rifts_seam::{AccountId, PeerId, Secret};
use sha2::{Digest, Sha256};

use crate::HostError;
use crate::transport::{
    ABI_VERSION, EXPORTS, Handle, HostOpen, JoinOpen, OPTIONAL_EXPORTS, ReadyState, SessionResult, Signal,
    TransportAbi, TransportLoader,
};

/// The largest ICE signal the backend relays (`src/paradox_transport_api.hpp`), plus its NUL terminator.
const MAXIMUM_SIGNAL_DATA: usize = 4097;

type AbiVersionFn = unsafe extern "C" fn() -> i32;
type InitializeFn = unsafe extern "C" fn(*mut u8, u32) -> i32;
type ShutdownFn = unsafe extern "C" fn();
type PeerIdFn = unsafe extern "C" fn() -> u64;
type NetworkingReadyFn = unsafe extern "C" fn() -> i32;
type HostOpenFn = unsafe extern "C" fn(*const i8, *const i8, *const i8, *const i8, *const i8, *mut u8, u32) -> i32;
type JoinOpenFn = unsafe extern "C" fn(*const i8, *const i8, *const i8, *const i8, *mut u8, u32) -> i32;
type WaitReadyHFn = unsafe extern "C" fn(i32, u32, *mut u8, u32) -> i32;
type HandleFn = unsafe extern "C" fn(i32) -> i32;
type ReadSignalHFn = unsafe extern "C" fn(i32, *mut u8, u32, *mut u8, u32, *mut u8, u32) -> i32;
type SubmitSignalHFn = unsafe extern "C" fn(i32, *const i8, *const i8, *const i8) -> i32;
type ReadLineHFn = unsafe extern "C" fn(i32, *mut u8, u32) -> i32;
type AddPeerHFn = unsafe extern "C" fn(i32, *const i8, u64, *const i8) -> i32;
type RemovePeerHFn = unsafe extern "C" fn(i32, *const i8, *const i8) -> i32;
type UpdateRelayCredentialHFn = unsafe extern "C" fn(i32, *const i8, *const i8) -> i32;

/// Computes the SHA-256 of a regular, non-empty file, refusing symlinks and empty files.
pub fn sha256_file(path: &Path) -> Result<[u8; 32], HostError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|err| HostError::Io(format!("transport artifact missing ({}): {err}", path.display())))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err(HostError::InvalidInput(format!(
            "transport artifact is not a regular non-empty file: {}",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|err| HostError::Io(format!("could not read {}: {err}", path.display())))?;
    let digest = Sha256::digest(&bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

fn cstring_arg(label: &'static str, value: &str) -> Result<CString, HostError> {
    CString::new(value).map_err(|_| HostError::InvalidInput(format!("{label} contains an interior NUL byte")))
}

fn cstring_path(label: &'static str, path: &Path) -> Result<CString, HostError> {
    let text = path
        .to_str()
        .ok_or_else(|| HostError::InvalidInput(format!("{label} path is not valid UTF-8: {}", path.display())))?;
    cstring_arg(label, text)
}

fn c_text_trimmed(buffer: &[u8]) -> String {
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).trim().to_owned()
}

fn c_text_raw(buffer: &[u8]) -> String {
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

/// Loads `ParadoxTransport.dll` / `.so` after checking its SHA-256 digest against the signed Guard manifest.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeTransportLoader;

impl TransportLoader for NativeTransportLoader {
    fn load(&self, dll: &Path, expected_sha256: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError> {
        let actual = sha256_file(dll)?;
        if actual != expected_sha256 {
            return Err(HostError::HashMismatch {
                expected: expected_sha256,
                actual,
            });
        }
        // SAFETY: `dll` was verified above as a regular file whose SHA-256 matches `expected_sha256` from the signed
        // Guard manifest. The transport library has no unsafe global constructors beyond standard C runtime init.
        let lib = unsafe { libloading::Library::new(dll) }
            .map_err(|err| HostError::Transport(format!("failed to load {}: {err}", dll.display())))?;

        for &export in &EXPORTS {
            let mut symbol = Vec::with_capacity(export.len() + 1);
            symbol.extend_from_slice(export.as_bytes());
            symbol.push(0);
            // SAFETY: Looking up a NUL-terminated symbol name as a raw pointer without invoking it only queries the
            // OS loader's export table for `lib`.
            let found = unsafe { lib.get::<*const ()>(&symbol) };
            if found.is_err() {
                return Err(HostError::MissingExport(export));
            }
        }

        let loaded = LoadedTransport::resolve(Arc::new(lib))?;
        let version = loaded.abi_version()?;
        if version != ABI_VERSION {
            return Err(HostError::UnsupportedAbi(version));
        }
        Ok(Box::new(loaded))
    }
}

struct LoadedTransport {
    _lib: Arc<libloading::Library>,
    abi_version: AbiVersionFn,
    initialize: InitializeFn,
    shutdown: ShutdownFn,
    peer_id: PeerIdFn,
    networking_ready: NetworkingReadyFn,
    host_open: HostOpenFn,
    join_open: JoinOpenFn,
    wait_ready: WaitReadyHFn,
    session_running: HandleFn,
    stop_session: HandleFn,
    last_session_result: HandleFn,
    read_signal: ReadSignalHFn,
    submit_signal: SubmitSignalHFn,
    read_peer_event: ReadLineHFn,
    read_diagnostic: ReadLineHFn,
    add_peer: AddPeerHFn,
    remove_peer: RemovePeerHFn,
    update_relay_credential: Option<UpdateRelayCredentialHFn>,
}

fn load_fn<T: Copy>(lib: &libloading::Library, name: &'static str) -> Result<T, HostError> {
    let mut nul_terminated = Vec::with_capacity(name.len() + 1);
    nul_terminated.extend_from_slice(name.as_bytes());
    nul_terminated.push(0);
    // SAFETY: Caller keeps `lib` alive in `LoadedTransport::_lib` for the lifetime of the function pointer, and the
    // signature `T` matches `src/paradox_transport_api.hpp` for `name`.
    let sym = unsafe { lib.get::<T>(&nul_terminated) }.map_err(|_| HostError::MissingExport(name))?;
    Ok(*sym)
}

fn load_optional_fn<T: Copy>(lib: &libloading::Library, name: &'static str) -> Option<T> {
    let mut nul_terminated = Vec::with_capacity(name.len() + 1);
    nul_terminated.extend_from_slice(name.as_bytes());
    nul_terminated.push(0);
    // SAFETY: Caller keeps `lib` alive in `LoadedTransport::_lib` for the lifetime of the function pointer, and the
    // optional symbol signature `T` matches `MysticTransport_UpdateRelayCredentialH`.
    unsafe { lib.get::<T>(&nul_terminated) }.ok().map(|sym| *sym)
}

impl LoadedTransport {
    fn resolve(lib: Arc<libloading::Library>) -> Result<Self, HostError> {
        Ok(Self {
            abi_version: load_fn(&lib, "MysticTransport_AbiVersion")?,
            initialize: load_fn(&lib, "MysticTransport_Initialize")?,
            shutdown: load_fn(&lib, "MysticTransport_Shutdown")?,
            peer_id: load_fn(&lib, "MysticTransport_PeerId")?,
            networking_ready: load_fn(&lib, "MysticTransport_NetworkingReady")?,
            host_open: load_fn(&lib, "MysticTransport_HostOpen")?,
            join_open: load_fn(&lib, "MysticTransport_JoinOpen")?,
            wait_ready: load_fn(&lib, "MysticTransport_WaitReadyH")?,
            session_running: load_fn(&lib, "MysticTransport_SessionRunningH")?,
            stop_session: load_fn(&lib, "MysticTransport_StopSessionH")?,
            last_session_result: load_fn(&lib, "MysticTransport_LastSessionResultH")?,
            read_signal: load_fn(&lib, "MysticTransport_ReadSignalH")?,
            submit_signal: load_fn(&lib, "MysticTransport_SubmitSignalH")?,
            read_peer_event: load_fn(&lib, "MysticTransport_ReadPeerEventH")?,
            read_diagnostic: load_fn(&lib, "MysticTransport_ReadDiagnosticH")?,
            add_peer: load_fn(&lib, "MysticTransport_AddPeerH")?,
            remove_peer: load_fn(&lib, "MysticTransport_RemovePeerH")?,
            update_relay_credential: load_optional_fn(&lib, OPTIONAL_EXPORTS[0]),
            _lib: lib,
        })
    }

    fn read_line(func: ReadLineHFn, handle: Handle, capacity: usize) -> Option<String> {
        let mut buffer = vec![0u8; capacity];
        // SAFETY: `buffer` is a valid mutable byte slice of length `capacity`, and `func` writes at most
        // `capacity` bytes (NUL-terminated) before returning 1 (copied) or 0 (empty).
        let rc = unsafe { func(handle.0, buffer.as_mut_ptr(), buffer.len() as u32) };
        (rc == 1)
            .then(|| c_text_trimmed(&buffer))
            .filter(|line| !line.is_empty())
    }
}

impl TransportAbi for LoadedTransport {
    fn abi_version(&self) -> Result<u32, HostError> {
        // SAFETY: `MysticTransport_AbiVersion` takes no arguments and returns a plain `int`.
        let version = unsafe { (self.abi_version)() };
        u32::try_from(version).map_err(|_| HostError::UnsupportedAbi(0))
    }

    fn initialize(&self) -> Result<(), HostError> {
        let mut error = [0u8; 512];
        // SAFETY: `error` is a stack buffer of 512 bytes; `MysticTransport_Initialize` writes at most `error.len()`
        // bytes on failure and is thread-safe.
        let rc = unsafe { (self.initialize)(error.as_mut_ptr(), error.len() as u32) };
        if rc == 1 {
            Ok(())
        } else {
            let detail = c_text_trimmed(&error);
            Err(HostError::Transport(if detail.is_empty() {
                "transport initialization failed".to_owned()
            } else {
                detail
            }))
        }
    }

    fn shutdown(&self) -> Result<(), HostError> {
        // SAFETY: `MysticTransport_Shutdown` takes no arguments and joins any active native sessions cleanly.
        unsafe { (self.shutdown)() };
        Ok(())
    }

    fn peer_id(&self) -> Result<PeerId, HostError> {
        // SAFETY: `MysticTransport_PeerId` takes no arguments and returns the instance's 64-bit peer id (0 if uninitialized).
        let id = unsafe { (self.peer_id)() };
        if id == 0 {
            Err(HostError::Transport(
                "transport peer id is 0 (not initialized)".to_owned(),
            ))
        } else {
            Ok(PeerId(id))
        }
    }

    fn networking_ready(&self) -> Result<bool, HostError> {
        // SAFETY: `MysticTransport_NetworkingReady` takes no arguments and returns non-zero when ready.
        let ready = unsafe { (self.networking_ready)() };
        Ok(ready != 0)
    }

    fn host_open(&self, open: &HostOpen) -> Result<Handle, HostError> {
        let public_key = cstring_path("public_key", &open.public_key)?;
        let config = cstring_path("config", &open.config)?;
        let capability = cstring_path("capability", &open.capability)?;
        let host_ticket = open
            .host_ticket
            .as_deref()
            .map(|p| cstring_path("host_ticket", p))
            .transpose()?;
        let secrets = cstring_path("secrets", &open.secrets)?;
        let mut error = [0u8; 1024];
        // SAFETY: All `CString` path arguments remain alive for the duration of this synchronous call; `host_ticket`
        // may be null when `host_local_client=0`; `error` is a valid 1024-byte writable buffer.
        let handle = unsafe {
            (self.host_open)(
                public_key.as_ptr(),
                config.as_ptr(),
                capability.as_ptr(),
                host_ticket.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                secrets.as_ptr(),
                error.as_mut_ptr(),
                error.len() as u32,
            )
        };
        if handle > 0 {
            Ok(Handle(handle))
        } else {
            let detail = c_text_trimmed(&error);
            Err(HostError::Transport(if detail.is_empty() {
                "transport host_open failed".to_owned()
            } else {
                detail
            }))
        }
    }

    fn join_open(&self, open: &JoinOpen) -> Result<Handle, HostError> {
        let public_key = cstring_path("public_key", &open.public_key)?;
        let config = cstring_path("config", &open.config)?;
        let ticket = cstring_path("ticket", &open.ticket)?;
        let secrets = cstring_path("secrets", &open.secrets)?;
        let mut error = [0u8; 1024];
        // SAFETY: All `CString` path arguments remain alive for the duration of this synchronous call; `error` is a
        // valid 1024-byte writable buffer.
        let handle = unsafe {
            (self.join_open)(
                public_key.as_ptr(),
                config.as_ptr(),
                ticket.as_ptr(),
                secrets.as_ptr(),
                error.as_mut_ptr(),
                error.len() as u32,
            )
        };
        if handle > 0 {
            Ok(Handle(handle))
        } else {
            let detail = c_text_trimmed(&error);
            Err(HostError::Transport(if detail.is_empty() {
                "transport join_open failed".to_owned()
            } else {
                detail
            }))
        }
    }

    fn wait_ready(&self, handle: Handle, timeout_ms: u32) -> Result<ReadyState, HostError> {
        let mut ready = [0u8; 1024];
        // SAFETY: `ready` is a valid 1024-byte writable buffer; `MysticTransport_WaitReadyH` writes a NUL-terminated
        // readiness line when returning 1, 0 on timeout, or -1 if the session ended before readiness.
        let rc = unsafe { (self.wait_ready)(handle.0, timeout_ms, ready.as_mut_ptr(), ready.len() as u32) };
        match rc {
            1 => Ok(ReadyState::Ready(c_text_trimmed(&ready))),
            0 => Ok(ReadyState::TimedOut),
            _ => Ok(ReadyState::Ended),
        }
    }

    fn session_running(&self, handle: Handle) -> Result<bool, HostError> {
        // SAFETY: `MysticTransport_SessionRunningH` takes the integer handle and returns 1 if running, 0 otherwise.
        let rc = unsafe { (self.session_running)(handle.0) };
        Ok(rc != 0)
    }

    fn stop_session(&self, handle: Handle) -> Result<(), HostError> {
        // SAFETY: `MysticTransport_StopSessionH` stops the session and joins its thread; returns 1 or 0 for unknown handle.
        let rc = unsafe { (self.stop_session)(handle.0) };
        if rc == 1 {
            Ok(())
        } else {
            Err(HostError::NotFound(format!("unknown session handle {}", handle.0)))
        }
    }

    fn last_session_result(&self, handle: Handle) -> Result<SessionResult, HostError> {
        // SAFETY: `MysticTransport_LastSessionResultH` takes the integer handle and returns its result code (-1 if unknown).
        let code = unsafe { (self.last_session_result)(handle.0) };
        Ok(SessionResult::from_code(code))
    }

    fn read_signal(&self, handle: Handle) -> Result<Option<Signal>, HostError> {
        let mut to_account = [0u8; 128];
        let mut kind = [0u8; 32];
        let mut data = vec![0u8; MAXIMUM_SIGNAL_DATA];
        // SAFETY: Each buffer is exclusively owned and sized as passed to `MysticTransport_ReadSignalH`.
        let rc = unsafe {
            (self.read_signal)(
                handle.0,
                to_account.as_mut_ptr(),
                to_account.len() as u32,
                kind.as_mut_ptr(),
                kind.len() as u32,
                data.as_mut_ptr(),
                data.len() as u32,
            )
        };
        match rc {
            1 => Ok(Some(Signal {
                to_account: AccountId(c_text_trimmed(&to_account)),
                kind: c_text_trimmed(&kind),
                data: c_text_raw(&data),
            })),
            0 => Ok(None),
            _ => Err(HostError::Transport("transport signal buffer too small".to_owned())),
        }
    }

    fn submit_signal(&self, handle: Handle, from: &AccountId, kind: &str, data: &str) -> Result<(), HostError> {
        let from_c = cstring_arg("from_account", from.0.as_str())?;
        let kind_c = cstring_arg("kind", kind)?;
        let data_c = cstring_arg("data", data)?;
        // SAFETY: `from_c`, `kind_c`, and `data_c` are valid NUL-terminated C strings for the call duration.
        let rc = unsafe { (self.submit_signal)(handle.0, from_c.as_ptr(), kind_c.as_ptr(), data_c.as_ptr()) };
        if rc == 1 {
            Ok(())
        } else {
            Err(HostError::Transport("transport refused incoming signal".to_owned()))
        }
    }

    fn read_peer_event(&self, handle: Handle) -> Result<Option<String>, HostError> {
        Ok(Self::read_line(self.read_peer_event, handle, 512))
    }

    fn read_diagnostic(&self, handle: Handle) -> Result<Option<String>, HostError> {
        Ok(Self::read_line(self.read_diagnostic, handle, 1024))
    }

    fn add_peer(&self, handle: Handle, account: &AccountId, peer: PeerId, pair_key: &Secret) -> Result<(), HostError> {
        let account_c = cstring_arg("account", account.0.as_str())?;
        let key_c = cstring_arg("pair_key", pair_key.expose())?;
        // SAFETY: `account_c` and `key_c` are valid NUL-terminated C strings for the call duration.
        let rc = unsafe { (self.add_peer)(handle.0, account_c.as_ptr(), peer.0, key_c.as_ptr()) };
        if rc == 1 {
            Ok(())
        } else {
            Err(HostError::Transport("transport refused peer addition".to_owned()))
        }
    }

    fn remove_peer(&self, handle: Handle, account: &AccountId, reason: &str) -> Result<(), HostError> {
        let account_c = cstring_arg("account", account.0.as_str())?;
        let reason_c = cstring_arg("reason", reason)?;
        // SAFETY: `account_c` and `reason_c` are valid NUL-terminated C strings for the call duration.
        let rc = unsafe { (self.remove_peer)(handle.0, account_c.as_ptr(), reason_c.as_ptr()) };
        if rc == 1 {
            Ok(())
        } else {
            Err(HostError::Transport("transport refused peer removal".to_owned()))
        }
    }

    fn update_relay_credential(&self, handle: Handle, username: &str, credential: &Secret) -> Result<(), HostError> {
        let user_c = cstring_arg("username", username)?;
        let cred_c = cstring_arg("credential", credential.expose())?;
        let Some(func) = self.update_relay_credential else {
            // Transport without the optional export keeps the credential it started with (transport.rs).
            return Ok(());
        };
        // SAFETY: `user_c` and `cred_c` are valid NUL-terminated C strings for the call duration.
        let rc = unsafe { func(handle.0, user_c.as_ptr(), cred_c.as_ptr()) };
        if rc == 1 {
            Ok(())
        } else {
            Err(HostError::Transport(
                "transport refused relay credential update".to_owned(),
            ))
        }
    }
}
