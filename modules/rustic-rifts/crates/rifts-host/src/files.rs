//! What a realm host's PC keeps (RF1, section 6). The launcher keeps no database (DD15): these are files in its
//! config directory, and the session folder's secrets die with the session.

use rifts_seam::AccountId;

use crate::HostError;
use crate::session::HubLimits;

/// Written into `<app local data>/p2p-sessions/<sessionId>-<generation>-<16 hex>/`.
pub const SESSION_FILES: [&str; 4] = ["bridge.conf", "transport.secret", "capability.secret", "ticket.secret"];

/// In the game folder, from the runtime feed's extra files (Guard role `ADMISSION_KEY`).
pub const ADMISSION_PUBLIC_KEY_FILE: &str = "p2p-admission-public.xy";

/// In the game folder (Guard roles `TRANSPORT_MODULE`, `TRANSPORT_RUNTIME`); DD1 keeps the name.
pub const TRANSPORT_DLL: &str = "ParadoxTransport.dll";

/// `p2p-host-settings.json` and `realm-host-limits.json`.
pub trait HostState: Send + Sync {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError>;
    /// Written through a `.tmp` beside it; a file that fails to parse is kept as `.bad`.
    fn save_hub_limits(&self, limits: &HubLimits) -> Result<(), HostError>;
    /// `control_cursors`: the last control sequence seen per account.
    fn control_cursor(&self, account: &AccountId) -> Result<Option<u64>, HostError>;
    fn set_control_cursor(&self, account: &AccountId, sequence: u64) -> Result<(), HostError>;
}
