// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Port assignment for modules (DD4): each port a manifest asks for keeps its default unless something
//! already holds it, the server's own ports or another module's, and then moves up past the clash. The
//! module is told what it got in `ModuleConfig::ports`.

use std::collections::BTreeMap;

use dainty_module_api::manifest::{PortDefault, Transport};
use dainty_module_api::{ModuleId, PortRange};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    transport: Transport,
    range: PortRange,
    /// None: the server's own
    owner: Option<ModuleId>,
    /// The manifest's name for it; empty for the server's own
    name: String,
}

/// Who holds which ports.
#[derive(Debug, Clone, Default)]
pub struct PortBook {
    held: Vec<Held>,
    preferred: BTreeMap<String, u16>,
}

/// Returned by [`PortBook::assign`] when moving a port up past every clash runs out of port numbers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no free {transport:?} range of {width} for port {name} above {first}")]
pub struct NoFreePort {
    /// The manifest's name for the port.
    pub name: String,
    /// The protocol the port is for.
    pub transport: Transport,
    /// The first port the manifest asked for.
    pub first: u16,
    /// How many consecutive ports it needs.
    pub width: u32,
}

impl PortBook {
    /// The server's own ports, which no module is given.
    pub fn reserve(&mut self, transport: Transport, range: PortRange) {
        self.held.push(Held {
            transport,
            range,
            owner: None,
            name: String::new(),
        });
    }

    /// Starts a manifest's port called `name` at `first` instead of its default, keeping its width.
    pub fn prefer(&mut self, name: &str, first: u16) {
        self.preferred.insert(name.to_owned(), first);
    }

    fn clash(&self, transport: Transport, range: PortRange) -> Option<PortRange> {
        self.held
            .iter()
            .filter(|held| held.transport == transport && held.range.overlaps(range))
            .map(|held| held.range)
            .max_by_key(|range| range.last)
    }

    /// Ranges for every port `ports` asks for, held for `owner` until [`release`](Self::release). Nothing
    /// is held when one can't be placed.
    pub fn assign(
        &mut self,
        owner: &ModuleId,
        ports: &[PortDefault],
    ) -> Result<BTreeMap<String, PortRange>, NoFreePort> {
        let mut given = BTreeMap::new();
        let before = self.held.len();
        for port in ports {
            let width = port.range.width();
            let mut range = match self.preferred.get(&port.name) {
                Some(&first) => match u16::try_from(u32::from(first) + width - 1) {
                    Ok(last) => PortRange { first, last },
                    Err(_) => port.range,
                },
                None => port.range,
            };
            while let Some(taken) = self.clash(port.transport, range) {
                let first = u32::from(taken.last) + 1;
                let last = first + width - 1;
                let (Ok(first), Ok(last)) = (u16::try_from(first), u16::try_from(last)) else {
                    self.held.truncate(before);
                    return Err(NoFreePort {
                        name: port.name.clone(),
                        transport: port.transport,
                        first: port.range.first,
                        width,
                    });
                };
                range = PortRange { first, last };
            }
            self.held.push(Held {
                transport: port.transport,
                range,
                owner: Some(owner.clone()),
                name: port.name.clone(),
            });
            given.insert(port.name.clone(), range);
        }
        Ok(given)
    }

    /// The range `owner` holds for its port `name`, from the moment it is assigned.
    pub fn held(&self, owner: &ModuleId, name: &str) -> Option<PortRange> {
        self.held
            .iter()
            .find(|held| held.owner.as_ref() == Some(owner) && held.name == name)
            .map(|held| held.range)
    }

    /// Frees every port `owner` holds.
    pub fn release(&mut self, owner: &ModuleId) {
        self.held.retain(|held| held.owner.as_ref() != Some(owner));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, transport: Transport, first: u16, last: u16) -> PortDefault {
        PortDefault {
            name: name.into(),
            transport,
            range: PortRange { first, last },
        }
    }

    #[test]
    fn a_clash_moves_up_and_a_release_frees_the_default() {
        let mut book = PortBook::default();
        book.reserve(Transport::Tcp, PortRange::single(61000));
        let a: ModuleId = "a".parse().unwrap();
        let b: ModuleId = "b".parse().unwrap();
        let asked = [
            port("http", Transport::Tcp, 61000, 61000),
            port("game", Transport::Udp, 61000, 61009),
        ];
        let first = book.assign(&a, &asked).unwrap();
        // UDP is its own space: the same number is free there
        assert_eq!(first["http"], PortRange::single(61001));
        assert_eq!(
            first["game"],
            PortRange {
                first: 61000,
                last: 61009
            }
        );
        let second = book.assign(&b, &asked).unwrap();
        assert_eq!(second["http"], PortRange::single(61002));
        assert_eq!(
            second["game"],
            PortRange {
                first: 61010,
                last: 61019
            }
        );
        book.release(&a);
        let again = book.assign(&a, &asked).unwrap();
        assert_eq!(again["http"], PortRange::single(61001));
        assert_eq!(
            again["game"],
            PortRange {
                first: 61000,
                last: 61009
            }
        );
    }

    #[test]
    fn a_preferred_port_starts_where_the_host_put_it_and_still_moves_past_a_clash() {
        let mut book = PortBook::default();
        book.prefer("backend", 42000);
        book.prefer("game", 9000);
        book.reserve(Transport::Tcp, PortRange::single(42000));
        let a: ModuleId = "a".parse().unwrap();
        let given = book
            .assign(
                &a,
                &[
                    port("backend", Transport::Tcp, 41000, 41000),
                    port("director", Transport::Tcp, 41001, 41001),
                    port("game", Transport::Udp, 8770, 8777),
                ],
            )
            .unwrap();
        assert_eq!(given["backend"], PortRange::single(42001));
        assert_eq!(given["director"], PortRange::single(41001));
        assert_eq!(
            given["game"],
            PortRange {
                first: 9000,
                last: 9007
            }
        );
    }

    #[test]
    fn nothing_is_held_when_a_port_cannot_be_placed() {
        let mut book = PortBook::default();
        book.reserve(
            Transport::Tcp,
            PortRange {
                first: 65000,
                last: 65535,
            },
        );
        let a: ModuleId = "a".parse().unwrap();
        let asked = [
            port("ok", Transport::Tcp, 1000, 1000),
            port("high", Transport::Tcp, 65500, 65510),
        ];
        assert_eq!(book.assign(&a, &asked).unwrap_err().name, "high");
        assert_eq!(
            book.assign(&a, &asked[..1]).unwrap()["ok"],
            PortRange::single(1000),
            "the first port was not left held"
        );
    }
}
