//! Who is a controller, a connection, an actor channel, and when a controller is another player's (5).

use super::{Class, Game, MEMO_SLOTS};
use crate::offsets::repgraph as at;
use crate::offsets::{layout, r9};

/// The deepest class chain walked.
const MAX_CHAIN: usize = 64;
/// Bytes of a class checked readable before its super is read.
const CLASS_CHECKED: usize = 0x48;
/// Bytes of an actor the memo's cheap test checks (the flags byte's span).
const ACTOR_CHECKED: usize = 0x60;

/// Whether `class` or one of its supers is `target` (by pointer).
fn class_reaches(game: &dyn Game, class: usize, target: usize) -> bool {
    let mut current = class;
    for _ in 0..MAX_CHAIN {
        if current == 0 || !game.readable(current, CLASS_CHECKED) {
            return false;
        }
        if current == target {
            return true;
        }
        current = game.ptr(current + at::STRUCT_SUPER);
    }
    false
}

/// `is_a` by class pointer; false while the class is not found.
fn is_a(game: &dyn Game, object: usize, class: Class) -> bool {
    let Some(target) = game.class(class) else {
        return false;
    };
    class_reaches(game, game.ptr(object + layout::UOBJECT_CLASS), target)
}

fn live(game: &dyn Game, ptr: usize, span: usize, class: Class) -> bool {
    ptr != 0 && game.readable(ptr, span) && game.is_live_object(ptr) && is_a(game, ptr, class)
}

/// `conn(p)`: a live `NetConnection`.
pub fn conn(game: &dyn Game, ptr: usize) -> bool {
    live(game, ptr, at::CONNECTION_SPAN, Class::NetConnection)
}

/// `chan(p)`: a live `ActorChannel`.
pub fn chan(game: &dyn Game, ptr: usize) -> bool {
    live(game, ptr, r9::CHANNEL_SIZE, Class::ActorChannel)
}

/// `pc(p)`: a live `PlayerController`.
pub fn pc(game: &dyn Game, ptr: usize) -> bool {
    live(game, ptr, at::PLAYER_CONTROLLER_SPAN, Class::PlayerController)
}

/// The pointer at `addr` when it is a live object, else 0.
pub fn live_or_null(game: &dyn Game, addr: usize) -> usize {
    let ptr = game.ptr(addr);
    if ptr != 0 && game.is_live_object(ptr) { ptr } else { 0 }
}

/// 5.3, pure: `Some(reason)` when `actor` is provably another connection's controller. By connection needs a
/// proven target; by identity needs only the target's controller or owner (RG-6 keeps the AGPL's rule).
pub fn remote(
    actor: usize,
    actor_conn: usize,
    actor_conn_ok: bool,
    target: usize,
    target_ok: bool,
    target_pc: usize,
    target_owner: usize,
) -> Option<&'static str> {
    if actor_conn_ok {
        return (target_ok && actor_conn != target).then_some("actor-netconnection-mismatch");
    }
    let has_identity = target_pc != 0 || target_owner != 0;
    (has_identity && actor != target_pc && actor != target_owner).then_some("target-identity-mismatch")
}

/// The class memo (5.2): class pointer → "is a `PlayerController`", open-addressed, emptied on a world change.
pub struct ClassMemo {
    world: usize,
    keys: [usize; MEMO_SLOTS],
    controller: [bool; MEMO_SLOTS],
    used: usize,
}

impl Default for ClassMemo {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassMemo {
    /// Empty.
    pub const fn new() -> Self {
        Self {
            world: 0,
            keys: [0; MEMO_SLOTS],
            controller: [false; MEMO_SLOTS],
            used: 0,
        }
    }

    /// Classes remembered.
    pub fn len(&self) -> usize {
        self.used
    }

    /// Whether nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    fn clear(&mut self) {
        self.keys = [0; MEMO_SLOTS];
        self.controller = [false; MEMO_SLOTS];
        self.used = 0;
    }

    fn home(class: usize) -> usize {
        ((class >> 4) ^ (class >> 14)) % MEMO_SLOTS
    }

    /// The slot holding `class`, or the free slot it would take; `None` when absent and the table is full.
    fn find(&self, class: usize) -> Option<usize> {
        let home = Self::home(class);
        (0..MEMO_SLOTS)
            .map(|step| (home + step) % MEMO_SLOTS)
            .find(|&slot| self.keys[slot] == class || self.keys[slot] == 0)
    }

    /// Whether `class` is a controller class: remembered, else walked (and remembered unless the table is full
    /// or the controller class is not found yet).
    pub fn is_controller_class(&mut self, game: &dyn Game, class: usize) -> bool {
        let world = game.world();
        if world != self.world {
            self.clear();
            self.world = world;
        }
        let slot = self.find(class);
        if let Some(slot) = slot
            && self.keys[slot] == class
        {
            return self.controller[slot];
        }
        let Some(target) = game.class(Class::PlayerController) else {
            return false;
        };
        let answer = class_reaches(game, class, target);
        if let Some(slot) = slot {
            self.keys[slot] = class;
            self.controller[slot] = answer;
            self.used += 1;
        }
        answer
    }
}

/// 5.2's hot path: the memo first (a frame-cached read and the class pointer), then the full `pc` test.
pub fn is_controller(game: &dyn Game, memo: &mut ClassMemo, actor: usize) -> bool {
    if actor == 0 || !game.readable_this_frame(actor, ACTOR_CHECKED) {
        return false;
    }
    let class = game.ptr(actor + layout::UOBJECT_CLASS);
    class != 0 && memo.is_controller_class(game, class) && pc(game, actor)
}
