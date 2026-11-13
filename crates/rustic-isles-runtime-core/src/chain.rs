//! The broker's chain bookkeeping for one target (R0 7.2). The DLL applies these answers with MinHook and atomic
//! stores; nothing here touches memory.

/// Which DLL a wrapper belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// This runtime's slices: outer, by `order` then registration.
    Mit,
    /// The AGPL runtime's one wrapper per target: innermost, its next always the trampoline.
    Agpl,
}

/// A refused change, with MinHook's status number (R0 7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
    /// `MH_ERROR_ALREADY_CREATED` (3): the AGPL band already has a wrapper, or the MIT wrapper is already there.
    AlreadyCreated,
    /// `MH_ERROR_NOT_CREATED` (4): no such wrapper.
    NotCreated,
}

impl ChainError {
    /// MinHook's number for it.
    pub fn status(self) -> i32 {
        match self {
            ChainError::AlreadyCreated => 3,
            ChainError::NotCreated => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MitWrapper {
    entry: usize,
    order: i32,
    seq: u64,
}

/// The chain on one hooked target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainModel {
    trampoline: usize,
    mit: Vec<MitWrapper>,
    agpl: Option<usize>,
    seq: u64,
}

impl ChainModel {
    /// An empty chain: the head is the trampoline, so the original runs.
    pub fn new(trampoline: usize) -> Self {
        Self {
            trampoline,
            mit: Vec::new(),
            agpl: None,
            seq: 0,
        }
    }

    /// MinHook's trampoline (the original function past every wrapper).
    pub fn trampoline(&self) -> usize {
        self.trampoline
    }

    /// Adds a MIT wrapper; smaller `order` is outer, ties by registration.
    pub fn add_mit(&mut self, entry: usize, order: i32) -> Result<(), ChainError> {
        if self.mit.iter().any(|wrapper| wrapper.entry == entry) {
            return Err(ChainError::AlreadyCreated);
        }
        self.seq += 1;
        let wrapper = MitWrapper {
            entry,
            order,
            seq: self.seq,
        };
        let at = self
            .mit
            .iter()
            .position(|other| other.order > order)
            .unwrap_or(self.mit.len());
        self.mit.insert(at, wrapper);
        Ok(())
    }

    /// Adds the AGPL wrapper; a second one is refused.
    pub fn add_agpl(&mut self, entry: usize) -> Result<(), ChainError> {
        if self.agpl.is_some() {
            return Err(ChainError::AlreadyCreated);
        }
        self.agpl = Some(entry);
        Ok(())
    }

    /// Removes a MIT wrapper.
    pub fn remove_mit(&mut self, entry: usize) -> Result<(), ChainError> {
        let at = self
            .mit
            .iter()
            .position(|wrapper| wrapper.entry == entry)
            .ok_or(ChainError::NotCreated)?;
        self.mit.remove(at);
        Ok(())
    }

    /// Removes the AGPL wrapper `entry`.
    pub fn remove_agpl(&mut self, entry: usize) -> Result<(), ChainError> {
        if self.agpl != Some(entry) {
            return Err(ChainError::NotCreated);
        }
        self.agpl = None;
        Ok(())
    }

    /// Every wrapper, outermost first.
    pub fn order(&self) -> Vec<(Band, usize)> {
        let mut order: Vec<(Band, usize)> = self.mit.iter().map(|wrapper| (Band::Mit, wrapper.entry)).collect();
        order.extend(self.agpl.map(|entry| (Band::Agpl, entry)));
        order
    }

    /// What the stub jumps to: the outermost wrapper, else the trampoline.
    pub fn head(&self) -> usize {
        self.order().first().map_or(self.trampoline, |&(_, entry)| entry)
    }

    /// The function a wrapper calls next.
    pub fn next_of(&self, entry: usize) -> Option<usize> {
        let order = self.order();
        let at = order.iter().position(|&(_, other)| other == entry)?;
        Some(order.get(at + 1).map_or(self.trampoline, |&(_, next)| next))
    }

    /// Each MIT wrapper's next cell value, innermost first: the order the DLL writes them in, `head` last.
    pub fn mit_cells(&self) -> Vec<(usize, usize)> {
        self.mit
            .iter()
            .rev()
            .filter_map(|wrapper| self.next_of(wrapper.entry).map(|next| (wrapper.entry, next)))
            .collect()
    }
}

/// `original(T)`: the trampoline when the broker hooked `T`, else `T` itself.
pub fn original(chain: Option<&ChainModel>, target: usize) -> usize {
    chain.map_or(target, ChainModel::trampoline)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRAMPOLINE: usize = 0x7000;

    #[test]
    fn bands_and_order() {
        let mut chain = ChainModel::new(TRAMPOLINE);
        assert_eq!(chain.head(), TRAMPOLINE);
        chain.add_agpl(0xA0).unwrap();
        assert_eq!(chain.head(), 0xA0);
        assert_eq!(chain.next_of(0xA0), Some(TRAMPOLINE));
        chain.add_mit(0x10, 5).unwrap();
        chain.add_mit(0x20, -1).unwrap();
        chain.add_mit(0x30, 5).unwrap();
        assert_eq!(
            chain.order(),
            [
                (Band::Mit, 0x20),
                (Band::Mit, 0x10),
                (Band::Mit, 0x30),
                (Band::Agpl, 0xA0)
            ]
        );
        assert_eq!(chain.head(), 0x20);
        assert_eq!(chain.next_of(0x20), Some(0x10));
        assert_eq!(chain.next_of(0x10), Some(0x30));
        assert_eq!(chain.next_of(0x30), Some(0xA0));
        assert_eq!(chain.mit_cells(), [(0x30, 0xA0), (0x10, 0x30), (0x20, 0x10)]);
        assert_eq!(chain.next_of(0x99), None);
    }

    #[test]
    fn agpl_band_holds_one() {
        let mut chain = ChainModel::new(TRAMPOLINE);
        chain.add_agpl(0xA0).unwrap();
        assert_eq!(chain.add_agpl(0xB0), Err(ChainError::AlreadyCreated));
        assert_eq!(ChainError::AlreadyCreated.status(), 3);
        assert_eq!(chain.add_mit(0x10, 0), Ok(()));
        assert_eq!(chain.add_mit(0x10, 1), Err(ChainError::AlreadyCreated));
    }

    #[test]
    fn removal_rewires() {
        let mut chain = ChainModel::new(TRAMPOLINE);
        chain.add_mit(0x10, 0).unwrap();
        chain.add_mit(0x20, 1).unwrap();
        chain.add_agpl(0xA0).unwrap();
        chain.remove_agpl(0xA0).unwrap();
        assert_eq!(chain.next_of(0x20), Some(TRAMPOLINE));
        assert_eq!(chain.mit_cells(), [(0x20, TRAMPOLINE), (0x10, 0x20)]);
        assert_eq!(chain.remove_agpl(0xA0), Err(ChainError::NotCreated));
        assert_eq!(ChainError::NotCreated.status(), 4);
        chain.remove_mit(0x10).unwrap();
        assert_eq!(chain.head(), 0x20);
        chain.remove_mit(0x20).unwrap();
        assert_eq!(chain.head(), TRAMPOLINE);
        assert_eq!(chain.remove_mit(0x20), Err(ChainError::NotCreated));
    }

    #[test]
    fn original_with_and_without_a_hook() {
        let chain = ChainModel::new(TRAMPOLINE);
        assert_eq!(original(Some(&chain), 0x5000), TRAMPOLINE);
        assert_eq!(original(None, 0x5000), 0x5000);
    }
}
