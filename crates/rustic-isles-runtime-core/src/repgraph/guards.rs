//! The four guards' decisions (7–10), in their step order, over `Game`'s reads and its one write.

use super::identity::{chan, conn, is_controller, live_or_null, pc, remote};
use super::lines::{self, Blocked, Refused, Reset, reset_reason, rpc_reason};
use super::{Budgets, Game, Pair, Queued, Source, State, budgeted, kinds, once_a_second};
use crate::offsets::{r9, repgraph as at};

/// What the detour does with its call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Call `next` with every argument and return its answer.
    Next,
    /// Return without calling `next` (0 where the hook returns a value).
    Absorb,
}

/// A slot read when readable for 8, else 0.
fn slot(game: &dyn Game, addr: usize) -> usize {
    if game.readable(addr, 8) { game.ptr(addr) } else { 0 }
}

/// Clears the cached-channel slot while it still holds `expected`: 1 written, 0 no longer matched, -1 refused.
fn clear_slot(game: &dyn Game, addr: usize, expected: usize) -> i32 {
    if slot(game, addr) != expected {
        return 0;
    }
    if game.write_ptr(addr, 0) { 1 } else { -1 }
}

/// RG-4's arguments R12 reads.
#[derive(Debug, Clone, Copy)]
pub struct PreChannel {
    /// The actor to replicate.
    pub actor: usize,
    /// The connection's cached actor info (its channel at `+0x00`).
    pub conn_actor_info: usize,
    /// The graph's per-connection manager.
    pub conn_manager: usize,
    /// The graph's frame number.
    pub frame: u32,
}

/// RG-4, `ReplicateSingleActor` (7).
pub fn pre_channel(game: &dyn Game, state: &mut State, budgets: &Budgets, call: PreChannel) -> Verdict {
    let PreChannel {
        actor,
        conn_actor_info,
        conn_manager,
        frame,
    } = call;
    if !is_controller(game, &mut state.memo, actor) {
        return Verdict::Next;
    }
    let manager_ok = game.readable(conn_manager, at::CONN_MANAGER_SPAN) && game.is_live_object(conn_manager);
    let target = if manager_ok {
        game.ptr(conn_manager + at::CONN_MANAGER_CONNECTION)
    } else {
        0
    };
    let target_ok = conn(game, target);
    let actor_conn = game.ptr(actor + at::PLAYER_CONTROLLER_NET_CONNECTION);
    let actor_conn_ok = conn(game, actor_conn);

    // Step 3: the cached channel can belong to the other player's connection too
    let cached = slot(game, conn_actor_info);
    let cached_ok = chan(game, cached);
    let (cached_conn, cached_actor) = if cached_ok {
        (
            game.ptr(cached + at::CHANNEL_CONNECTION),
            game.ptr(cached + r9::CHANNEL_ACTOR),
        )
    } else {
        (0, 0)
    };
    let cached_conn_ok = conn(game, cached_conn);
    let conn_mismatch = cached_ok && target_ok && cached_conn_ok && cached_conn != target;
    let actor_mismatch = cached_ok && cached_actor != 0 && cached_actor != actor;
    let unusable = cached != 0 && (!cached_ok || !cached_conn_ok);
    if conn_mismatch || actor_mismatch || unusable {
        if conn_mismatch && (cached_actor == 0 || cached_actor == actor) {
            let correct = if actor_conn_ok { actor_conn } else { target };
            state.quarantine.queue(
                game,
                budgets,
                Queued {
                    channel: cached,
                    actor,
                    wrong: cached_conn,
                    correct,
                    source: Source::ConnActorInfoMismatch,
                    caller: 0,
                },
            );
        }
        let cleared = clear_slot(game, conn_actor_info, cached);
        let reason = if conn_mismatch {
            reset_reason::CONNECTION
        } else if actor_mismatch {
            reset_reason::ACTOR
        } else {
            reset_reason::UNUSABLE
        };
        budgeted(game, &budgets.pre_reset, kinds::PRE_RESET, || {
            lines::pre_reset(&Reset {
                actor,
                actor_connection: actor_conn,
                target,
                channel: cached,
                channel_connection: cached_conn,
                channel_actor: cached_actor,
                cleared,
                reason,
            })
        });
    }

    if !target_ok {
        if once_a_second(&mut state.unproven_ms, game.now_ms()) {
            budgeted(game, &budgets.pre_unproven, kinds::PRE_UNPROVEN, || {
                lines::pre_unproven(actor, conn_manager, target, frame)
            });
        }
        return Verdict::Next;
    }
    let target_pc = live_or_null(game, target + at::CONNECTION_PLAYER_CONTROLLER);
    let target_owner = live_or_null(game, target + at::CONNECTION_OWNING_ACTOR);
    let Some(reason) = remote(actor, actor_conn, actor_conn_ok, target, true, target_pc, target_owner) else {
        return Verdict::Next;
    };

    let before = slot(game, conn_actor_info);
    if chan(game, before) {
        let before_conn = game.ptr(before + at::CHANNEL_CONNECTION);
        let before_actor = game.ptr(before + r9::CHANNEL_ACTOR);
        if before_conn == target && (before_actor == 0 || before_actor == actor) {
            state.quarantine.queue(
                game,
                budgets,
                Queued {
                    channel: before,
                    actor,
                    wrong: target,
                    correct: if actor_conn_ok { actor_conn } else { 0 },
                    source: Source::RemoteControllerPreChannel,
                    caller: 0,
                },
            );
        }
        clear_slot(game, conn_actor_info, before);
    }
    let blocked = Blocked {
        actor,
        actor_connection: actor_conn,
        target,
        target_controller: target_pc,
        target_owner,
        reason,
    };
    match state.pre_pairs.note((actor, target)) {
        Pair::New => game.log(&lines::pre_blocked(&blocked, before, frame)),
        Pair::Seen => {}
        Pair::Full => budgeted(game, &budgets.pre_blocked, kinds::PRE_BLOCKED, || {
            lines::pre_blocked(&blocked, before, frame)
        }),
    }
    Verdict::Absorb
}

/// RG-5, `SetChannelActor` (8). `caller` computes the caller's RVA, only when a line needs it.
pub fn bind(
    game: &dyn Game,
    state: &mut State,
    budgets: &Budgets,
    channel: usize,
    actor: usize,
    flags: u64,
    caller: impl FnOnce() -> usize,
) -> Verdict {
    if state.quarantine.contains(channel) {
        let caller = caller();
        budgeted(game, &budgets.bind_quarantined, kinds::BIND_QUARANTINED, || {
            lines::bind_quarantined(channel, actor, caller)
        });
        return Verdict::Absorb;
    }
    if !chan(game, channel) || !pc(game, actor) {
        return Verdict::Next;
    }
    let chan_conn = game.ptr(channel + at::CHANNEL_CONNECTION);
    let actor_conn = game.ptr(actor + at::PLAYER_CONTROLLER_NET_CONNECTION);
    let previous = game.ptr(channel + r9::CHANNEL_ACTOR);
    let chan_conn_ok = conn(game, chan_conn);
    let actor_conn_ok = conn(game, actor_conn);
    // The channel's connection is the target here, and must itself be proven
    if !chan_conn_ok {
        return Verdict::Next;
    }
    let chan_pc = live_or_null(game, chan_conn + at::CONNECTION_PLAYER_CONTROLLER);
    let chan_owner = live_or_null(game, chan_conn + at::CONNECTION_OWNING_ACTOR);
    let Some(reason) = remote(actor, actor_conn, actor_conn_ok, chan_conn, true, chan_pc, chan_owner) else {
        return Verdict::Next;
    };
    let caller = caller();
    let queued = state.quarantine.queue(
        game,
        budgets,
        Queued {
            channel,
            actor,
            wrong: chan_conn,
            correct: if actor_conn_ok { actor_conn } else { 0 },
            source: Source::SetChannelActor,
            caller,
        },
    );
    budgeted(game, &budgets.bind_refused, kinds::BIND_REFUSED, || {
        lines::bind_refused(&Refused {
            actor,
            actor_connection: actor_conn,
            channel,
            channel_connection: chan_conn,
            channel_controller: chan_pc,
            channel_owner: chan_owner,
            previous_actor: previous,
            flags,
            caller,
            reason,
            queued,
        })
    });
    Verdict::Absorb
}

/// RG-6, `ReplicateActor` (9).
pub fn send(game: &dyn Game, state: &mut State, budgets: &Budgets, channel: usize) -> Verdict {
    if state.quarantine.contains(channel) {
        return Verdict::Absorb;
    }
    if channel == 0 || !game.readable(channel, r9::CHANNEL_SIZE) {
        return Verdict::Next;
    }
    let actor = game.ptr(channel + r9::CHANNEL_ACTOR);
    if !is_controller(game, &mut state.memo, actor) {
        return Verdict::Next;
    }
    let target = game.ptr(channel + at::CHANNEL_CONNECTION);
    let target_ok = conn(game, target);
    let (target_pc, target_owner) = if target_ok {
        (
            live_or_null(game, target + at::CONNECTION_PLAYER_CONTROLLER),
            live_or_null(game, target + at::CONNECTION_OWNING_ACTOR),
        )
    } else {
        (0, 0)
    };
    let actor_conn = game.ptr(actor + at::PLAYER_CONTROLLER_NET_CONNECTION);
    let known = conn(game, actor_conn);
    let Some(reason) = remote(actor, actor_conn, known, target, target_ok, target_pc, target_owner) else {
        return Verdict::Next;
    };
    state.quarantine.queue(
        game,
        budgets,
        Queued {
            channel,
            actor,
            wrong: target,
            correct: if known { actor_conn } else { 0 },
            source: Source::ReplicateActorFinal,
            caller: 0,
        },
    );
    let blocked = Blocked {
        actor,
        actor_connection: actor_conn,
        target,
        target_controller: target_pc,
        target_owner,
        reason,
    };
    let flags = game.u8_at(actor + at::ACTOR_FLAGS_58).unwrap_or(0);
    match state.send_pairs.note((actor, target)) {
        Pair::New => game.log(&lines::send_blocked(&blocked, flags)),
        Pair::Seen => {}
        Pair::Full => budgeted(game, &budgets.send_blocked, kinds::SEND_BLOCKED, || {
            lines::send_blocked(&blocked, flags)
        }),
    }
    Verdict::Absorb
}

/// RG-7's arguments R12 reads.
#[derive(Debug, Clone, Copy)]
pub struct Rpc {
    /// The actor channel.
    pub channel: usize,
    /// The object the RPC is called on.
    pub target_obj: usize,
    /// The connection.
    pub connection: usize,
    /// The `UFunction`.
    pub function: usize,
    /// `bIsServer`.
    pub is_server: u8,
}

fn name_text(game: &dyn Game, object: usize) -> String {
    if object == 0 {
        return "null".to_owned();
    }
    game.name(object).unwrap_or_else(|| "unreadable".to_owned())
}

/// RG-7, `ProcessRemoteFunctionForChannel` (10, patch 0039).
pub fn rpc(game: &dyn Game, state: &State, budgets: &Budgets, call: Rpc) -> Verdict {
    if call.channel == 0 || call.is_server == 0 {
        return Verdict::Next;
    }
    let reason = if state.quarantine.contains(call.channel) {
        rpc_reason::QUARANTINED
    } else if game.ptr_checked(call.channel + r9::CHANNEL_ACTOR).unwrap_or(0) == 0 {
        rpc_reason::WITHOUT_ACTOR
    } else {
        return Verdict::Next;
    };
    let (count, logged) = budgets.rpc_drop();
    if logged {
        game.log(&lines::rpc_dropped(
            &name_text(game, call.function),
            &name_text(game, call.target_obj),
            call.channel,
            call.connection,
            reason,
            count,
        ));
    }
    Verdict::Absorb
}
