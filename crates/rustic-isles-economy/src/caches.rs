// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The game-data reads kept for a module's life (inventory decision item 3: per module, never process-wide).

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use rustic_isles::Shared;

/// The composer puts one in the module's slots; without it every read loads afresh (the same answers, uncached).
#[derive(Default)]
pub struct EconomyCaches {
    entries: Mutex<HashMap<&'static str, Arc<dyn Any + Send + Sync>>>,
}

impl std::fmt::Debug for EconomyCaches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EconomyCaches(..)")
    }
}

impl EconomyCaches {
    /// The kept value of `key`, else `load`'s; kept only when `load` says so (a failed read is tried again).
    pub(crate) fn get_or_load<T, F>(&self, key: &'static str, load: F) -> Arc<T>
    where
        T: Any + Send + Sync,
        F: FnOnce() -> (Arc<T>, bool),
    {
        if let Some(found) = self.lookup::<T>(key) {
            return found;
        }
        let (value, keep) = load();
        if keep {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            // Another request may have loaded it meanwhile: the first kept value wins
            if let Some(found) = entries.get(key).and_then(|v| Arc::clone(v).downcast::<T>().ok()) {
                return found;
            }
            entries.insert(key, Arc::clone(&value) as Arc<dyn Any + Send + Sync>);
        }
        value
    }

    fn lookup<T: Any + Send + Sync>(&self, key: &'static str) -> Option<Arc<T>> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.get(key).and_then(|v| Arc::clone(v).downcast::<T>().ok())
    }
}

/// `load` through the module's caches, or straight through when the composer gave none.
pub(crate) fn cached<T, F>(shared: &Shared, key: &'static str, load: F) -> Arc<T>
where
    T: Any + Send + Sync,
    F: FnOnce() -> (Arc<T>, bool),
{
    match shared.slot::<EconomyCaches>() {
        Some(caches) => caches.get_or_load(key, load),
        None => load().0,
    }
}
