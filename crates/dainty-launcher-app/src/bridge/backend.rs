// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! What the commands share beyond the controller (§2.3.5): the page's progress channel, the invite link waiting
//! for the page, and the manager's module panel.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use dainty_launcher_kit::bridge::modules::ModulePanel;
use dainty_launcher_kit::bridge::progress::ProgressChannel;
use dainty_launcher_kit::bridge::types::TaskProgress;
use tauri::ipc::Channel;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The bridge's state, managed by Tauri.
#[derive(Default)]
pub struct Backend {
    progress: Mutex<ProgressChannel<Channel<TaskProgress>>>,
    invite_link: Mutex<Option<String>>,
    panel: OnceLock<Arc<dyn ModulePanel>>,
}

impl Backend {
    /// The state with `link` from the start-up arguments waiting.
    pub fn new(link: Option<String>) -> Self {
        Self {
            invite_link: Mutex::new(link),
            ..Self::default()
        }
    }

    /// `subscribe_progress`: replaces any earlier channel.
    pub fn subscribe(&self, channel: Channel<TaskProgress>) {
        lock(&self.progress).subscribe(channel);
    }

    /// Sends `progress` when the throttle passes it.
    pub fn progress(&self, progress: &TaskProgress) {
        let mut channel = lock(&self.progress);
        if let Some(sink) = channel.offer(progress, Instant::now()) {
            let _ = sink.send(progress.clone());
        }
    }

    /// Keeps a link for the page, replacing an older one.
    pub fn set_invite_link(&self, link: String) {
        *lock(&self.invite_link) = Some(link);
    }

    /// `take_invite_link`: hands the waiting link over and forgets it.
    pub fn take_invite_link(&self) -> Option<String> {
        lock(&self.invite_link).take()
    }

    /// The manager's module panel; set once, from the embedded server.
    #[cfg_attr(
        not(feature = "server-gui"),
        allow(dead_code, reason = "only the manager has a panel")
    )]
    pub fn set_panel(&self, panel: Arc<dyn ModulePanel>) {
        let _ = self.panel.set(panel);
    }

    /// The module panel, in the manager with an embedded server.
    pub fn panel(&self) -> Option<Arc<dyn ModulePanel>> {
        self.panel.get().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_is_taken_once() {
        let backend = Backend::new(Some("dainty://a".to_owned()));
        assert_eq!(backend.take_invite_link().as_deref(), Some("dainty://a"));
        assert_eq!(backend.take_invite_link(), None);
        backend.set_invite_link("dainty://b".to_owned());
        backend.set_invite_link("dainty://c".to_owned());
        assert_eq!(backend.take_invite_link().as_deref(), Some("dainty://c"));
        assert!(backend.panel().is_none());
    }
}
