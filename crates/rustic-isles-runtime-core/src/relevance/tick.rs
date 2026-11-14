//! `SP-3@R13` after the engine tick: the weapon wake (7), the pawn rate write (8) and the relevance pass (9), in
//! that order.

use super::{
    Budgets, Class, Game, PLAYER_CHANNEL_GRACE_FRAMES, State, apply_settings, budgeted, clear_connection_cull,
    connection_info, connection_manager, global_info, is_a, lines, readable_ptr,
};
use crate::offsets::relevance as at;
use crate::offsets::{layout, repgraph as rg};

/// The connection span the wake reads (through `OwningActor`).
const WAKE_CONNECTION_SPAN: usize = 0x98;
/// The connection span the pass reads (through `PlayerController`).
const PASS_CONNECTION_SPAN: usize = 0x38;
/// The controller span the pass reads (through `Pawn`).
const PASS_CONTROLLER_SPAN: usize = 0x258;
/// `NetUpdateFrequency` 60 and `MinNetUpdateFrequency` 30, one aligned word (8).
const PAWN_RATES: [u8; 8] = {
    let rate = 60.0f32.to_bits().to_le_bytes();
    let min = 30.0f32.to_bits().to_le_bytes();
    [rate[0], rate[1], rate[2], rate[3], min[0], min[1], min[2], min[3]]
};

/// One connected player as the pass sees it (9.2): 0 for what cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triple {
    /// The connection.
    pub connection: usize,
    /// Its possessed character.
    pub pawn: usize,
    /// The character's `PlayerRole` pointer.
    pub role: usize,
}

/// The handler (7.1): the wake, the pawn write unless its flag was present, then the pass. `graph` is R12's
/// `repgraph::graph()`, asked only when a piece needs it.
pub fn tick(
    game: &dyn Game,
    state: &mut State,
    budgets: &Budgets,
    connections: &[usize],
    graph: &dyn Fn() -> Option<usize>,
    pawn_boost: bool,
) {
    wake(game, state, budgets, connections, graph);
    if pawn_boost {
        pawn_rates(game, connections);
    }
    pass(game, state, budgets, connections, graph);
}

/// The live character a connection's controller possesses, after `connection` was checked.
fn character(game: &dyn Game, connection: usize, controller_span: usize, live_controller: bool) -> usize {
    let pc = readable_ptr(game, connection + rg::CONNECTION_PLAYER_CONTROLLER, controller_span);
    if pc == 0 || (live_controller && !game.is_live_object(pc)) {
        return 0;
    }
    let pawn = readable_ptr(game, pc + at::PLAYER_CONTROLLER_PAWN, at::CHARACTER_SPAN);
    if pawn != 0 && game.is_live_object(pawn) && is_a(game, pawn, Class::Character) {
        pawn
    } else {
        0
    }
}

/// 7.2: every 250 ms, a current weapon no connection has had yet is un-culled and woken, once per weapon.
pub fn wake(
    game: &dyn Game,
    state: &mut State,
    budgets: &Budgets,
    connections: &[usize],
    graph: &dyn Fn() -> Option<usize>,
) {
    let now = game.now_ms();
    if state
        .wake_ms
        .is_some_and(|last| now.saturating_sub(last) < at::WEAPON_WAKE_PERIOD_MS)
    {
        return;
    }
    state.wake_ms = Some(now);
    let Some(graph) = graph() else {
        return;
    };
    for &connection in connections {
        wake_one(game, state, budgets, graph, connection);
    }
}

fn wake_one(game: &dyn Game, state: &mut State, budgets: &Budgets, graph: usize, connection: usize) -> Option<()> {
    if connection == 0 || !game.readable(connection, WAKE_CONNECTION_SPAN) || !game.is_live_object(connection) {
        return None;
    }
    let pawn = Some(character(game, connection, rg::PLAYER_CONTROLLER_SPAN, false)).filter(|&pawn| pawn != 0)?;
    let weapon = readable_ptr(game, pawn + at::CHARACTER_WEAPON, at::WEAPON_SPAN);
    if weapon == 0 || !game.is_alive_for_replication(weapon) || !is_a(game, weapon, Class::Weapon) {
        return None;
    }
    let index = game.i32_at(weapon + layout::UOBJECT_INDEX).unwrap_or(-1);
    if state.ring.holds(weapon, index) {
        return None;
    }
    let g = global_info(game, graph, weapon).filter(|&g| game.readable(g, at::GLOBAL_INFO_SIZE))?;
    let dormancy = game.u8_at(weapon + at::ACTOR_NET_DORMANCY).unwrap_or(0);
    let wants = game.u8_at(g + at::GLOBAL_WANTS_DORMANT).unwrap_or(0);
    let flags58 = game.u8_at(weapon + at::ACTOR_FLAGS_58).unwrap_or(0);
    let cull_sq = game.f32_at(g + at::GLOBAL_CULL_DISTANCE_SQ).unwrap_or(0.0);
    let needs_wake = dormancy > at::DORM_AWAKE;
    let needs_cull = flags58 & at::ALWAYS_RELEVANT != 0 && cull_sq != 0.0;
    if !needs_wake && !needs_cull {
        return None;
    }
    let manager = connection_manager(game, graph, connection)?;
    let info = connection_info(game, manager, weapon).filter(|&info| game.readable(info, at::CONN_INFO_SIZE));
    if let Some(info) = info
        && (game.ptr(info + at::CONN_INFO_CHANNEL) != 0
            || game.i32_at(info + at::CONN_INFO_LAST_REP_FRAME).unwrap_or(0) != 0)
    {
        // This connection has had the weapon already
        return None;
    }
    state.ring.record(weapon, index);
    let weapon_name = lines::name(weapon, game.name(weapon));
    let pawn_name = lines::name(pawn, game.name(pawn));
    if needs_cull {
        let global = apply_settings(game, Some(g), true, 0).result;
        let cleared = clear_connection_cull(game, graph, weapon);
        budgeted(game, &budgets.cull_cleared, lines::kinds::CULL_CLEARED, || {
            lines::weapon_cull_cleared(
                &weapon_name,
                &pawn_name,
                connection,
                flags58,
                global,
                cleared,
                info.is_some(),
            )
        });
    }
    if needs_wake {
        let call = game.wake(weapon);
        let dormancy_after = game.u8_at(weapon + at::ACTOR_NET_DORMANCY).unwrap_or(0);
        let wants_after = game.u8_at(g + at::GLOBAL_WANTS_DORMANT).unwrap_or(0);
        budgeted(game, &budgets.woke, lines::kinds::WOKE, || {
            lines::weapon_woke(
                &weapon_name,
                &pawn_name,
                connection,
                (dormancy, dormancy_after),
                (wants, wants_after),
                info.is_some(),
                call,
            )
        });
    }
    Some(())
}

/// 8: every frame, each connected player character's update rates to 60 and 30. No line.
pub fn pawn_rates(game: &dyn Game, connections: &[usize]) {
    for &connection in connections {
        if connection == 0 {
            continue;
        }
        let pc = game.ptr(connection + rg::CONNECTION_PLAYER_CONTROLLER);
        if pc == 0 || !game.is_live_object(pc) {
            continue;
        }
        let pawn = game.ptr(pc + at::PLAYER_CONTROLLER_PAWN);
        if pawn != 0
            && game.is_live_object(pawn)
            && is_a(game, pawn, Class::PlayerCharacter)
            && game.readable(pawn + at::ACTOR_NET_UPDATE_FREQUENCY, 8)
        {
            let _ = game.write(pawn + at::ACTOR_NET_UPDATE_FREQUENCY, &PAWN_RATES);
        }
    }
}

fn triple(game: &dyn Game, connection: usize) -> Triple {
    let pawn = if connection != 0 && game.readable(connection, PASS_CONNECTION_SPAN) {
        character(game, connection, PASS_CONTROLLER_SPAN, true)
    } else {
        0
    };
    let role = if pawn == 0 {
        0
    } else {
        game.ptr(pawn + at::CHARACTER_PLAYER_ROLE)
    };
    Triple { connection, pawn, role }
}

/// 9: at most once a second, and then only on changed inputs or after 15 s, re-asserts each connected character's
/// grace and its role's settings and cull snapshots.
pub fn pass(
    game: &dyn Game,
    state: &mut State,
    budgets: &Budgets,
    connections: &[usize],
    graph: &dyn Fn() -> Option<usize>,
) {
    let now = game.now_ms();
    if state
        .pass_checked_ms
        .is_some_and(|last| now.saturating_sub(last) < at::GRACE_PASS_FLOOR_MS)
    {
        return;
    }
    state.pass_checked_ms = Some(now);
    let Some(graph) = graph() else {
        return;
    };
    let triples: Vec<Triple> = connections.iter().map(|&connection| triple(game, connection)).collect();
    let unchanged = state
        .pass_inputs
        .as_ref()
        .is_some_and(|(last_graph, last)| *last_graph == graph && *last == triples);
    match state.pass_ran_ms {
        None => game.log(lines::PASS_STARTED),
        Some(ran) if unchanged && now.saturating_sub(ran) < at::GRACE_PASS_MAX_MS => return,
        Some(_) => {}
    }
    state.pass_ran_ms = Some(now);
    let mut roles: Vec<usize> = Vec::new();
    for triple in &triples {
        if triple.pawn != 0 {
            reassert(game, budgets, graph, triple, &mut roles);
        }
    }
    state.pass_inputs = Some((graph, triples));
}

fn reassert(game: &dyn Game, budgets: &Budgets, graph: usize, triple: &Triple, roles: &mut Vec<usize>) {
    let Triple { connection, pawn, role } = *triple;
    let settings = apply_settings(game, global_info(game, graph, pawn), false, PLAYER_CHANNEL_GRACE_FRAMES);
    if settings.result == 1 {
        budgeted(game, &budgets.pawn_grace, lines::kinds::PAWN_GRACE, || {
            lines::pawn_grace(pawn, connection, &settings)
        });
    }
    if role == 0
        || roles.contains(&role)
        || !game.is_alive_for_replication(role)
        || !is_a(game, role, Class::PlayerRole)
    {
        return;
    }
    roles.push(role);
    let global = apply_settings(game, global_info(game, graph, role), true, PLAYER_CHANNEL_GRACE_FRAMES);
    let cleared = clear_connection_cull(game, graph, role);
    if global.result == 1 || cleared > 0 {
        budgeted(game, &budgets.role_cull, lines::kinds::ROLE_CULL, || {
            lines::role_cull(role, pawn, &global, cleared)
        });
    }
}
