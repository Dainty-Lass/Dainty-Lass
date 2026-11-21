// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Between the links and the UDP bridges: gameplay from admitted peers only, bounded per attached peer.

use std::collections::{BTreeMap, HashSet, VecDeque};

const MESSAGES_PER_PEER: usize = 256;
const BYTES_PER_PEER: usize = 1 << 20;

/// What a bridge sends through and reads from.
pub trait MessageTransport {
    fn send(&mut self, peer: u64, payload: &[u8]) -> bool;
    fn receive(&mut self, max: usize) -> Vec<(u64, Vec<u8>)>;
}

#[derive(Debug, Default)]
pub struct Router {
    attached: HashSet<u64>,
    inbox: VecDeque<(u64, Vec<u8>)>,
    bytes: usize,
    pub dropped: u64,
}

impl Router {
    pub fn attach(&mut self, peer: u64) {
        self.attached.insert(peer);
    }

    /// Detaching also drops what the peer had queued.
    pub fn detach(&mut self, peer: u64) {
        self.attached.remove(&peer);
        self.inbox.retain(|(from, _)| *from != peer);
        self.bytes = self.inbox.iter().map(|(_, m)| m.len()).sum();
    }

    pub fn is_attached(&self, peer: u64) -> bool {
        self.attached.contains(&peer)
    }

    pub fn attached_count(&self) -> usize {
        self.attached.len()
    }

    pub fn deliver(&mut self, peer: u64, message: Vec<u8>) {
        let slots = self.attached.len() + 1;
        let fits = self.inbox.len() < MESSAGES_PER_PEER * slots && self.bytes + message.len() <= BYTES_PER_PEER * slots;
        if !self.attached.contains(&peer) || message.is_empty() || !fits {
            self.dropped += 1;
            return;
        }
        self.bytes += message.len();
        self.inbox.push_back((peer, message));
    }

    pub fn read(&mut self, max: usize) -> Vec<(u64, Vec<u8>)> {
        let count = max.min(self.inbox.len());
        let out: Vec<_> = self.inbox.drain(..count).collect();
        self.bytes -= out.iter().map(|(_, m)| m.len()).sum::<usize>();
        out
    }
}

/// An in-memory network of endpoints for tests: each endpoint's inbox holds 256 messages and 1 MiB.
#[derive(Debug, Default)]
pub struct MemoryNetwork {
    inboxes: BTreeMap<u64, VecDeque<(u64, Vec<u8>)>>,
}

impl MemoryNetwork {
    /// Refuses peer id 0 and a second endpoint with the same id.
    pub fn add_endpoint(&mut self, peer: u64) -> bool {
        if peer == 0 || self.inboxes.contains_key(&peer) {
            return false;
        }
        self.inboxes.insert(peer, VecDeque::new());
        true
    }

    pub fn endpoint(&mut self, peer: u64) -> MemoryEndpoint<'_> {
        MemoryEndpoint { network: self, peer }
    }
}

pub struct MemoryEndpoint<'a> {
    network: &'a mut MemoryNetwork,
    peer: u64,
}

impl MessageTransport for MemoryEndpoint<'_> {
    fn send(&mut self, peer: u64, payload: &[u8]) -> bool {
        let from = self.peer;
        let Some(inbox) = self.network.inboxes.get_mut(&peer) else {
            return false;
        };
        let bytes: usize = inbox.iter().map(|(_, m)| m.len()).sum();
        if payload.is_empty() || inbox.len() >= MESSAGES_PER_PEER || bytes + payload.len() > BYTES_PER_PEER {
            return false;
        }
        inbox.push_back((from, payload.to_vec()));
        true
    }

    fn receive(&mut self, max: usize) -> Vec<(u64, Vec<u8>)> {
        self.network.inboxes.get_mut(&self.peer).map_or_else(Vec::new, |inbox| {
            let count = max.min(inbox.len());
            inbox.drain(..count).collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_bounds() {
        let mut router = Router::default();
        router.deliver(5, vec![1]);
        assert_eq!(router.dropped, 1);
        router.attach(5);
        router.deliver(5, vec![]);
        assert_eq!(router.dropped, 2);
        for _ in 0..600 {
            router.deliver(5, vec![1]);
        }
        assert_eq!(router.read(1000).len(), 512);
        router.deliver(5, vec![2]);
        router.detach(5);
        assert!(router.read(10).is_empty());
    }

    #[test]
    fn memory_network() {
        let mut network = MemoryNetwork::default();
        assert!(!network.add_endpoint(0));
        assert!(network.add_endpoint(1));
        assert!(!network.add_endpoint(1));
        assert!(network.add_endpoint(2));
        assert!(network.endpoint(1).send(2, b"hi"));
        assert!(!network.endpoint(1).send(2, b""));
        assert!(!network.endpoint(1).send(3, b"hi"));
        assert_eq!(network.endpoint(2).receive(4), vec![(1, b"hi".to_vec())]);
    }
}
