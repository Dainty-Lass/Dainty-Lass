//! The graph's settings toolkit (5): the map search, the three lookups, the settings write and the snapshot clear.
//! Public for R14 (14).

use super::{Game, Reads};
use crate::engine::sane_pointer_array;
use crate::offsets::relevance as at;
use crate::offsets::repgraph as rg;

/// Most elements a map may hold before it is treated as malformed.
const MAP_LIMIT: i32 = 65_536;
/// The inline allocation words cover this many elements.
const INLINE_BITS: usize = 128;
/// Elements copied per read.
const CHUNK: usize = 64;
/// The graph's connection managers at most.
const MANAGERS_LIMIT: i32 = 1024;
/// The graph span read before its connections.
const GRAPH_CONNECTIONS_SPAN: usize = 0x48;

fn bit(words: &[u8], index: usize) -> bool {
    words.get(index / 8).is_some_and(|byte| byte & (1 << (index % 8)) != 0)
}

/// 5.1: the value stored for `key` in the engine `TMap` at `base` whose elements are `stride` bytes. A malformed
/// map answers `None`; only the keys are compared and nothing but the answer is read as a pointer.
pub fn map_find(game: &dyn Game, base: usize, key: usize, stride: usize) -> Option<usize> {
    if base == 0 || key == 0 || stride < at::MAP_VALUE + 8 || !game.readable(base, at::MAP_SPAN) {
        return None;
    }
    let num = game.i32_at(base + at::MAP_NUM)?;
    let max = game.i32_at(base + at::MAP_MAX)?;
    if num <= 0 || num > MAP_LIMIT || max < num {
        return None;
    }
    let num = usize::try_from(num).ok()?;
    let data = game.ptr(base + at::MAP_DATA);
    if data == 0 || !game.readable(data, num.checked_mul(stride)?) {
        return None;
    }
    let num_bits = game.i32_at(base + at::MAP_NUM_BITS).unwrap_or(0);
    let mut count = usize::try_from(num_bits)
        .ok()
        .filter(|&bits| bits > 0 && bits < num)
        .unwrap_or(num);
    let secondary = game.ptr(base + at::MAP_SECONDARY_BITS);
    let words = if secondary != 0 {
        let len = count.div_ceil(32) * 4;
        if !game.readable(secondary, len) {
            return None;
        }
        let mut words = vec![0u8; len];
        game.read(secondary, &mut words).then_some(words)?
    } else {
        count = count.min(INLINE_BITS);
        let mut words = vec![0u8; INLINE_BITS / 8];
        game.read(base + at::MAP_INLINE_BITS, &mut words).then_some(words)?
    };
    let mut elements = vec![0u8; CHUNK * stride];
    for first in (0..count).step_by(CHUNK) {
        let len = (count - first).min(CHUNK);
        if !(first..first + len).any(|index| bit(&words, index)) {
            continue;
        }
        let chunk = &mut elements[..len * stride];
        if !game.read(data + first * stride, chunk) {
            return None;
        }
        for index in (first..first + len).filter(|&index| bit(&words, index)) {
            let element = &chunk[(index - first) * stride..];
            let mut pointer = [0u8; 8];
            pointer.copy_from_slice(&element[..8]);
            if usize::try_from(u64::from_le_bytes(pointer)).ok() == Some(key) {
                pointer.copy_from_slice(&element[at::MAP_VALUE..at::MAP_VALUE + 8]);
                return usize::try_from(u64::from_le_bytes(pointer)).ok();
            }
        }
    }
    None
}

/// `global_info(graph, actor)`: the actor's `FGlobalActorReplicationInfo`.
pub fn global_info(game: &dyn Game, graph: usize, actor: usize) -> Option<usize> {
    if graph == 0 || !game.readable(graph, at::GRAPH_GLOBAL_ACTOR_SPAN) {
        return None;
    }
    map_find(
        game,
        graph + at::GRAPH_GLOBAL_ACTOR_MAP,
        actor,
        at::GRAPH_GLOBAL_ACTOR_STRIDE,
    )
    .filter(|&info| info != 0)
}

/// The graph's connection managers (sane, limit 1 024), or none, once `graph` is readable for `span`.
fn managers(game: &dyn Game, graph: usize, span: usize) -> Vec<usize> {
    if graph == 0 || !game.readable(graph, span) {
        return Vec::new();
    }
    let data = game.ptr(graph + rg::GRAPH_CONNECTIONS);
    let num = game.i32_at(graph + rg::GRAPH_CONNECTIONS_NUM).unwrap_or(-1);
    let max = game.i32_at(graph + rg::GRAPH_CONNECTIONS_MAX).unwrap_or(-1);
    if !sane_pointer_array(&Reads(game), data, num, max, MANAGERS_LIMIT) {
        return Vec::new();
    }
    let count = usize::try_from(num).unwrap_or(0);
    (0..count).map(|index| game.ptr(data + index * 8)).collect()
}

/// `connection_manager(graph, connection)`: the first manager readable for `0x30` serving `connection`.
pub fn connection_manager(game: &dyn Game, graph: usize, connection: usize) -> Option<usize> {
    if connection == 0 {
        return None;
    }
    managers(game, graph, GRAPH_CONNECTIONS_SPAN)
        .into_iter()
        .find(|&manager| {
            manager != 0
                && game.readable(manager, rg::CONN_MANAGER_SPAN)
                && game.ptr(manager + rg::CONN_MANAGER_CONNECTION) == connection
        })
}

/// `connection_info(manager, actor)`: the connection's `FConnectionReplicationActorInfo`.
pub fn connection_info(game: &dyn Game, manager: usize, actor: usize) -> Option<usize> {
    if manager == 0 {
        return None;
    }
    map_find(
        game,
        manager + at::CONN_MANAGER_ACTOR_INFO_MAP,
        actor,
        at::CONN_MANAGER_ACTOR_INFO_STRIDE,
    )
    .filter(|&info| info != 0)
}

/// What `apply_settings` found and left: `(CullDistance, CullDistanceSquared, ActorChannelFrameTimeout)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// −1 unreadable, 0 already right, 1 changed.
    pub result: i32,
    /// Before.
    pub before: (f32, f32, u16),
    /// After.
    pub after: (f32, f32, u16),
}

impl Settings {
    /// An info that could not be read.
    pub const UNREADABLE: Self = Self {
        result: -1,
        before: (0.0, 0.0, 0),
        after: (0.0, 0.0, 0),
    };
}

/// 5.3 on a global info, or `UNREADABLE` for `None`. The grace is raised only: a longer one and 0 (never time the
/// channel out) are kept.
pub fn apply_settings(game: &dyn Game, info: Option<usize>, zero_cull: bool, min_grace: u16) -> Settings {
    let Some(info) = info.filter(|&info| game.readable(info, at::GLOBAL_INFO_SIZE)) else {
        return Settings::UNREADABLE;
    };
    let (Some(cull), Some(cull_sq), Some(grace)) = (
        game.f32_at(info + at::GLOBAL_CULL_DISTANCE),
        game.f32_at(info + at::GLOBAL_CULL_DISTANCE_SQ),
        game.u16_at(info + at::GLOBAL_CHANNEL_TIMEOUT),
    ) else {
        return Settings::UNREADABLE;
    };
    let before = (cull, cull_sq, grace);
    let mut after = before;
    if zero_cull && (cull != 0.0 || cull_sq != 0.0) && game.write(info + at::GLOBAL_CULL_DISTANCE, &[0; 8]) {
        after.0 = 0.0;
        after.1 = 0.0;
    }
    if min_grace != 0
        && grace != 0
        && grace < min_grace
        && game.write(info + at::GLOBAL_CHANNEL_TIMEOUT, &min_grace.to_le_bytes())
    {
        after.2 = min_grace;
    }
    Settings {
        result: i32::from(after != before),
        before,
        after,
    }
}

/// 5.4: zeroes every connection's non-zero cull snapshot of `actor`; the count cleared.
pub fn clear_connection_cull(game: &dyn Game, graph: usize, actor: usize) -> u32 {
    let mut cleared = 0;
    for manager in managers(game, graph, rg::GRAPH_SPAN) {
        if manager == 0 || !game.readable(manager, at::CONN_MANAGER_SPAN) {
            continue;
        }
        let Some(info) = connection_info(game, manager, actor).filter(|&info| game.readable(info, at::CONN_INFO_SIZE))
        else {
            continue;
        };
        let cull = game.f32_at(info + at::CONN_INFO_CULL_DISTANCE).unwrap_or(0.0);
        let cull_sq = game.f32_at(info + at::CONN_INFO_CULL_DISTANCE_SQ).unwrap_or(0.0);
        if (cull != 0.0 || cull_sq != 0.0) && game.write(info + at::CONN_INFO_CULL_DISTANCE, &[0; 8]) {
            cleared += 1;
        }
    }
    cleared
}
