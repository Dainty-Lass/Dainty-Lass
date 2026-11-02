// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! PLACEHOLDER for a Category A file (DF4d). The real `allowlist/guard.rs` lives in dainty-restricted and moves
//! here unchanged at integration (docs/port/clean-room/gateway.md); the clean-room implementer never saw it. This
//! stand-in is written only from spec-rd27-gateway-allowlist.md section 12 so the helper builds and its tests run.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use module_api::PortRange;

use super::ports::{MAX_RANGES, check_ports, ranges};

/// Told the guard's text and every module's ranges after each change.
pub type OnChange = Arc<dyn Fn(&str, &BTreeMap<String, Vec<String>>) + Send + Sync>;

/// `ALLOWLIST_PORTS` plus every attached module's UDP ranges.
pub struct PortGuard {
    base: (u16, u16),
    modules: Mutex<BTreeMap<String, Vec<(u16, u16)>>>,
    on_change: Mutex<Option<OnChange>>,
}

fn text(range: (u16, u16)) -> String {
    if range.0 == range.1 {
        range.0.to_string()
    } else {
        format!("{}-{}", range.0, range.1)
    }
}

/// Sorted, adjacent and overlapping ranges joined.
fn merge(mut all: Vec<(u16, u16)>) -> Vec<(u16, u16)> {
    all.sort_unstable();
    let mut out: Vec<(u16, u16)> = Vec::new();
    for (low, high) in all {
        match out.last_mut() {
            Some(last) if u32::from(low) <= u32::from(last.1) + 1 => last.1 = last.1.max(high),
            _ => out.push((low, high)),
        }
    }
    out
}

impl PortGuard {
    pub fn new(base: &str) -> Result<Self, String> {
        let checked = check_ports(base)?;
        let base = ranges(&checked).first().copied().ok_or_else(|| "no ports".to_owned())?;
        Ok(Self {
            base,
            modules: Mutex::default(),
            on_change: Mutex::default(),
        })
    }

    pub fn set_on_change(&self, on_change: Option<OnChange>) {
        *self.on_change.lock().unwrap_or_else(PoisonError::into_inner) = on_change;
    }

    fn merged(&self, modules: &BTreeMap<String, Vec<(u16, u16)>>) -> Vec<(u16, u16)> {
        merge(
            std::iter::once(self.base)
                .chain(modules.values().flatten().copied())
                .collect(),
        )
    }

    /// The plan's ports at every run, and `/status`'s `ports`.
    pub fn text(&self) -> String {
        let modules = self.modules.lock().unwrap_or_else(PoisonError::into_inner);
        self.merged(&modules)
            .into_iter()
            .map(text)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn covers(&self, port: u16) -> bool {
        let modules = self.modules.lock().unwrap_or_else(PoisonError::into_inner);
        self.merged(&modules)
            .iter()
            .any(|(low, high)| (*low..=*high).contains(&port))
    }

    pub fn module_ports(&self) -> BTreeMap<String, Vec<String>> {
        let modules = self.modules.lock().unwrap_or_else(PoisonError::into_inner);
        modules
            .iter()
            .map(|(m, r)| (m.clone(), r.iter().copied().map(text).collect()))
            .collect()
    }

    fn changed(&self) {
        let callback = self.on_change.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(callback) = callback {
            callback(&self.text(), &self.module_ports());
        }
    }

    /// Either succeeds with the text grown or leaves it unchanged.
    pub fn attach(&self, module: &str, udp: &[PortRange]) -> Result<(), String> {
        {
            let mut modules = self.modules.lock().unwrap_or_else(PoisonError::into_inner);
            let mut next = modules.clone();
            let owned: Vec<(u16, u16)> = udp.iter().map(|r| (r.first, r.last)).collect();
            if owned.iter().any(|(low, high)| *low == 0 || low > high) {
                return Err(format!("{module}: a UDP range must be within 1-65535, low to high"));
            }
            next.insert(module.to_owned(), owned);
            if self.merged(&next).len() > MAX_RANGES {
                return Err(format!("ALLOWLIST_PORTS holds at most {MAX_RANGES} ranges"));
            }
            *modules = next;
        }
        self.changed();
        Ok(())
    }

    pub fn detach(&self, module: &str) {
        let removed = self
            .modules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(module)
            .is_some();
        if removed {
            self.changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_join() {
        let g = PortGuard::new("8770-8777").unwrap();
        g.attach(
            "a",
            &[
                PortRange {
                    first: 8778,
                    last: 8780,
                },
                PortRange::single(64000),
            ],
        )
        .unwrap();
        assert_eq!(g.text(), "8770-8780,64000");
        assert!(g.covers(64000) && !g.covers(64001));
        let many: Vec<PortRange> = (0..40).map(|i| PortRange::single(10_000 + i * 2)).collect();
        assert!(g.attach("b", &many).is_err());
        assert_eq!(g.text(), "8770-8780,64000");
        g.detach("a");
        assert_eq!(g.text(), "8770-8777");
    }
}
