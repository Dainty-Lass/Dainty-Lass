// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! PLACEHOLDER for a Category A file (DF4d, C2). The real `allowlist/modules.rs` lives in dainty-restricted and
//! moves here unchanged at integration (docs/port/clean-room/gateway.md); the clean-room implementer never saw
//! it. This stand-in is written only from spec-rd27-gateway.md section 3.1 so the library builds and its tests run.

use std::sync::{Arc, PoisonError, RwLock};

use module_api::{ErrorKind, ModuleError, ModuleId, PortRange};

use super::guard::PortGuard;

/// How a module's UDP ranges are guarded beside this gateway.
#[derive(Clone)]
pub enum UdpMode {
    /// Private mode: not guarded here
    Off,
    /// The helper runs in the library, on this guard
    InProcess(Arc<PortGuard>),
    /// A helper process guards `base`: a module's ranges must lie inside it
    External(PortRange),
    /// A public server without a helper: every module UDP port is refused
    Absent,
}

/// The allowlist's side of the UDP-ports core plugin.
pub struct UdpPorts {
    mode: RwLock<UdpMode>,
}

impl UdpPorts {
    pub fn new(mode: UdpMode) -> Self {
        Self {
            mode: RwLock::new(mode),
        }
    }

    pub fn set_mode(&self, mode: UdpMode) {
        *self.mode.write().unwrap_or_else(PoisonError::into_inner) = mode;
    }

    pub fn attach(&self, module: &ModuleId, ranges: &[PortRange]) -> Result<(), ModuleError> {
        if ranges.is_empty() {
            return Ok(());
        }
        let refuse = |message: String| ModuleError::new(ErrorKind::Unavailable, message);
        match &*self.mode.read().unwrap_or_else(PoisonError::into_inner) {
            UdpMode::Off => Ok(()),
            UdpMode::InProcess(guard) => guard.attach(module.as_str(), ranges).map_err(refuse),
            UdpMode::External(base) => match ranges.iter().find(|r| r.first < base.first || r.last > base.last) {
                Some(r) => Err(refuse(format!(
                    "{module}'s UDP ports {}-{} lie outside the allowlist helper's {}-{}",
                    r.first, r.last, base.first, base.last
                ))),
                None => Ok(()),
            },
            UdpMode::Absent => Err(refuse(format!(
                "{module} needs UDP ports, and this public server runs no allowlist helper to guard them"
            ))),
        }
    }

    pub fn detach(&self, module: &ModuleId) {
        if let UdpMode::InProcess(guard) = &*self.mode.read().unwrap_or_else(PoisonError::into_inner) {
            guard.detach(module.as_str());
        }
    }
}
