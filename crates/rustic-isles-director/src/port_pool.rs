// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hunt ports (spec §6): a FIFO of free ports, so a just-freed port is reused last, and owner-checked leases.

use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub port: u16,
    /// The launch id that took it
    pub owner: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Default)]
pub struct PortPool {
    free: VecDeque<u16>,
    held: Vec<Lease>,
}

impl PortPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ignored when the port is already free or held.
    pub fn add(&mut self, port: u16) {
        if !self.free.contains(&port) && !self.held.iter().any(|l| l.port == port) {
            self.free.push_back(port);
        }
    }

    pub fn take(&mut self, owner: &str) -> Option<u16> {
        let port = self.free.pop_front()?;
        self.held.push(Lease {
            port,
            owner: owner.to_owned(),
            pid: None,
        });
        Some(port)
    }

    /// Only for the port's current owner.
    pub fn attach_pid(&mut self, port: u16, owner: &str, pid: u32) -> bool {
        match self.held.iter_mut().find(|l| l.port == port && l.owner == owner) {
            Some(lease) => {
                lease.pid = Some(pid);
                true
            }
            None => false,
        }
    }

    /// Only the current owner frees it; a late or duplicate release changes nothing.
    pub fn release(&mut self, port: u16, owner: &str) -> bool {
        let Some(at) = self.held.iter().position(|l| l.port == port && l.owner == owner) else {
            return false;
        };
        self.held.remove(at);
        self.free.push_back(port);
        true
    }

    /// Releases every lease whose owner is not live and whose pid is none or gone (the watchdog's fallback when
    /// an exit never arrives); a live pid keeps its lease, as that process may still own the socket.
    pub fn reclaim(&mut self, is_owner_live: impl Fn(&str) -> bool, is_pid_alive: impl Fn(u32) -> bool) -> Vec<Lease> {
        let (gone, kept): (Vec<Lease>, Vec<Lease>) = std::mem::take(&mut self.held)
            .into_iter()
            .partition(|l| !is_owner_live(&l.owner) && !l.pid.is_some_and(&is_pid_alive));
        self.held = kept;
        for lease in &gone {
            self.free.push_back(lease.port);
            let pid = lease.pid.map_or_else(|| "none".to_owned(), |p| p.to_string());
            tracing::warn!(
                "[PortPool] reclaimed port {} from launch {} (pid={pid} gone, no exit event)",
                lease.port,
                lease.owner
            );
        }
        gone
    }

    pub fn owner_of(&self, port: u16) -> Option<&str> {
        self.held.iter().find(|l| l.port == port).map(|l| l.owner.as_str())
    }

    pub fn pid_of(&self, port: u16) -> Option<u32> {
        self.held.iter().find(|l| l.port == port).and_then(|l| l.pid)
    }

    pub fn free_count(&self) -> usize {
        self.free.len()
    }

    pub fn held_count(&self) -> usize {
        self.held.len()
    }

    pub fn leases(&self) -> &[Lease] {
        &self.held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(ports: std::ops::RangeInclusive<u16>) -> PortPool {
        let mut pool = PortPool::new();
        for port in ports {
            pool.add(port);
        }
        pool
    }

    #[test]
    fn owners_and_fifo() {
        let mut p = pool(8780..=8780);
        assert_eq!(p.take("A"), Some(8780));
        assert!(p.release(8780, "A"));
        assert_eq!(p.take("B"), Some(8780));
        assert!(!p.release(8780, "A"));
        assert_eq!(p.take("C"), None);
        assert_eq!(p.owner_of(8780), Some("B"));
        assert!(p.release(8780, "B"));
        assert!(!p.release(8780, "B"));

        let mut p = pool(8780..=8782);
        p.add(8780);
        let a = p.take("A").unwrap();
        p.add(a);
        assert_eq!((p.free_count(), p.held_count()), (2, 1));
        assert!(!p.release(8790, "A"));
        assert_eq!((p.free_count(), p.held_count()), (2, 1));
        p.release(a, "A");
        assert_eq!(p.take("B"), Some(8781));
        assert_eq!(p.take("C"), Some(8782));
        assert_eq!(p.take("D"), Some(8780));
    }

    #[test]
    fn a_timed_out_start_never_gives_a_port_two_owners() {
        let mut p = pool(8780..=8782);
        let a = p.take("A").unwrap();
        p.release(a, "A");
        let taken: Vec<u16> = ["B", "C", "D"].iter().map(|o| p.take(o).unwrap()).collect();
        assert!(!p.release(a, "A"));
        let mut sorted = taken.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), 3);
        assert_eq!(p.held_count(), 3);
    }

    #[test]
    fn pids_and_reclaim() {
        let mut p = pool(8780..=8783);
        let a = p.take("A").unwrap();
        assert!(!p.attach_pid(a, "B", 1));
        assert!(p.attach_pid(a, "A", 11));
        let b = p.take("B").unwrap();
        p.attach_pid(b, "B", 12);
        let c = p.take("C").unwrap();
        p.attach_pid(c, "C", 13);
        let d = p.take("D").unwrap();
        // A live pid, a listed owner, an unlisted dead one, and one without a pid
        let gone = p.reclaim(|o| o == "B", |pid| pid == 11);
        let ports: Vec<u16> = gone.iter().map(|l| l.port).collect();
        assert_eq!(ports, [c, d]);
        assert_eq!(p.pid_of(a), Some(11));
        assert!(!p.release(c, "C"));
        assert_eq!(p.owner_of(b), Some("B"));
    }
}
