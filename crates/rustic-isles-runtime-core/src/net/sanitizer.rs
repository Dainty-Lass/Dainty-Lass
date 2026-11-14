//! The connection-list sanitiser's decisions (5): what to write and which lines, over a `Memory`. The DLL applies
//! the stores; nothing here writes.

use super::lines::sanitize as text;
use super::{CLEANUP_ELEMENT, CONNECTION_CHECKED, DRIVER_CHECKED, MAX_CONNECTIONS};
use crate::engine::sane_pointer_array;
use crate::memory::Memory;
use crate::offsets::net::{NETDRIVER_CLEANUP, NETDRIVER_CLIENT_CONNECTIONS};

/// One write the DLL applies, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    /// An 8-byte word (a pointer, or `num` and `max` together).
    Word(usize, u64),
    /// A 4-byte `Num`.
    Num(usize, i32),
}

/// The sanitiser's answer for one driver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// True only when the driver is safe to walk this frame unchanged.
    pub safe: bool,
    /// The repairs.
    pub stores: Vec<Store>,
    /// The texts, without the site prefix.
    pub texts: Vec<String>,
}

impl Plan {
    fn refuse(text: impl Into<String>) -> Self {
        Self {
            safe: false,
            stores: Vec::new(),
            texts: vec![text.into()],
        }
    }
}

/// An array header: data, `Num`, `Max`.
fn header(memory: &dyn Memory, at: usize) -> Option<(usize, i32, i32)> {
    Some((memory.ptr_at(at)?, memory.i32_at(at + 8)?, memory.i32_at(at + 12)?))
}

/// Resets a header to empty; the engine's allocation is not freed (an engine allocation may not be freed from
/// outside the engine).
fn reset(at: usize) -> Vec<Store> {
    vec![Store::Word(at, 0), Store::Word(at + 8, 0)]
}

/// Steps 1 to 7 of section 5 for `driver`.
pub fn plan_sanitize(memory: &dyn Memory, driver: usize) -> Plan {
    if !memory.readable(driver, DRIVER_CHECKED) {
        return Plan::refuse(text::INVALID_DRIVER);
    }
    let list = driver + NETDRIVER_CLIENT_CONNECTIONS;
    let Some((data, num, max)) = memory.readable(list, 0x10).then(|| header(memory, list)).flatten() else {
        return Plan::refuse(text::UNREADABLE_CONNECTIONS);
    };
    if !sane_pointer_array(memory, data, num, max, MAX_CONNECTIONS) {
        return Plan {
            safe: false,
            stores: reset(list),
            texts: vec![text::reset_connections(data, num, max)],
        };
    }
    let old = usize::try_from(num).unwrap_or(0);
    let mut kept = Vec::with_capacity(old);
    let mut texts = Vec::new();
    for index in 0..old {
        let element = memory.ptr_at(data + index * 8).unwrap_or(0);
        if memory.readable(element, CONNECTION_CHECKED) {
            kept.push(element);
        } else {
            texts.push(text::dropped(index, element));
        }
    }
    if kept.len() != old {
        let mut stores: Vec<Store> = (0..old)
            .map(|slot| Store::Word(data + slot * 8, kept.get(slot).map_or(0, |&element| element as u64)))
            .collect();
        let new = i32::try_from(kept.len()).unwrap_or(0);
        stores.push(Store::Num(list + 8, new));
        texts.push(text::compacted(num, new));
        return Plan {
            safe: false,
            stores,
            texts,
        };
    }
    let cleanup = driver + NETDRIVER_CLEANUP;
    let Some((data, num, max)) = memory
        .readable(cleanup, 0x10)
        .then(|| header(memory, cleanup))
        .flatten()
    else {
        return Plan::refuse(text::UNREADABLE_CLEANUP);
    };
    let valid = num >= 0
        && max >= 0
        && num <= max
        && num <= MAX_CONNECTIONS
        && (num == 0
            || usize::try_from(num)
                .ok()
                .and_then(|num| num.checked_mul(CLEANUP_ELEMENT))
                .is_some_and(|size| memory.readable(data, size)));
    if !valid {
        return Plan {
            safe: false,
            stores: reset(cleanup),
            texts: vec![text::reset_cleanup(data, num, max)],
        };
    }
    Plan {
        safe: true,
        ..Plan::default()
    }
}
