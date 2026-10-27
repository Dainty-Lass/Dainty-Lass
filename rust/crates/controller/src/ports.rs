//! Port assignment for modules (DD4): each port a manifest asks for keeps its default unless something
//! already holds it, the server's own ports or another module's, and then moves up past the clash. The
//! module is told what it got in `ModuleConfig::ports`.

use std::collections::BTreeMap;

use module_api::manifest::{PortDefault, Transport};
use module_api::{ModuleId, PortRange};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    transport: Transport,
    range: PortRange,
    /// None: the server's own
    owner: Option<ModuleId>,
}

/// Who holds which ports.
#[derive(Debug, Clone, Default)]
pub struct PortBook {
    held: Vec<Held>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no free {transport:?} range of {width} for port {name} above {first}")]
pub struct NoFreePort {
    pub name: String,
    pub transport: Transport,
    pub first: u16,
    pub width: u32,
}

impl PortBook {
    /// The server's own ports, which no module is given.
    pub fn reserve(&mut self, transport: Transport, range: PortRange) {
        self.held.push(Held {
            transport,
            range,
            owner: None,
        });
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
            let mut range = port.range;
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
            });
            given.insert(port.name.clone(), range);
        }
        Ok(given)
    }

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
