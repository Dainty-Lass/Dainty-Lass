//! RG-2 (6): the route at registration. Every correction is written before the original runs, since the route
//! files an actor once from the bits it reads then.

use super::{Budgets, Class, Game, PLAYER_CHANNEL_GRACE_FRAMES, apply_settings, budgeted, first_class, lines};
use crate::offsets::relevance as at;
use crate::offsets::repgraph as rg;

/// The order the classes are tried in (6.1 step 2): the first that applies wins.
const ORDER: [Class; 5] = [
    Class::PlayerController,
    Class::PlayerState,
    Class::PlayerRole,
    Class::Weapon,
    Class::Character,
];
/// The span of a global info the weapon branch needs (its dormancy field).
const GLOBAL_DORMANCY_SPAN: usize = 0x18;

/// Which branch an actor took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// No actor, not alive, or no class of interest.
    None,
    /// 6.2.
    Controller,
    /// 6.3.
    PlayerState,
    /// 6.4.
    Role,
    /// 6.5.
    Weapon,
    /// 6.6.
    Character,
}

/// 6.2: only to its owner (always-relevant is tested first, so it must go).
pub fn controller_flags(flags58: u8) -> u8 {
    (flags58 | at::ONLY_RELEVANT_TO_OWNER) & !at::ALWAYS_RELEVANT
}

/// 6.3 and 6.4: globally relevant, and the owner's relevancy not consulted.
pub fn global_flags(flags58: u8, flags59: u8) -> (u8, u8) {
    (
        (flags58 | at::ALWAYS_RELEVANT) & !at::ONLY_RELEVANT_TO_OWNER,
        flags59 & !at::NET_USE_OWNER_RELEVANCY,
    )
}

/// 6.5's two fixes for a weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponPlan {
    /// 0015: on the spatial route and dormant, wake it so the grid files it dynamic.
    pub wake: bool,
    /// 0016: on the global route, clear its cull distance.
    pub cull: bool,
}

/// 6.5's decision from the flag byte, `NetDormancy` and whether the global info is readable.
pub fn weapon_plan(flags58: u8, dormancy: u8, global_ok: bool) -> WeaponPlan {
    let relevancy = flags58 & (at::ALWAYS_RELEVANT | at::ONLY_RELEVANT_TO_OWNER);
    WeaponPlan {
        wake: relevancy == 0 && global_ok && dormancy > at::DORM_AWAKE,
        cull: flags58 & at::ALWAYS_RELEVANT != 0 && global_ok,
    }
}

/// The route the graph takes for these bits (6.5's `route=`).
pub fn weapon_route(flags58: u8, wants_dormant: u8) -> &'static str {
    if flags58 & at::ALWAYS_RELEVANT != 0 {
        "global"
    } else if flags58 & at::ONLY_RELEVANT_TO_OWNER != 0 {
        "owner"
    } else if wants_dormant == 1 {
        "spatial-static"
    } else {
        "spatial-dynamic"
    }
}

/// Writes `new` over `old` at `addr` when they differ; what the byte now is.
fn set_byte(game: &dyn Game, addr: usize, old: u8, new: u8) -> u8 {
    if old == new || game.write(addr, &[new]) {
        new
    } else {
        old
    }
}

fn short_class(game: &dyn Game, actor: usize) -> String {
    lines::name(actor, game.class_name(actor))
}

/// 6.1: the corrections for the actor of `info` (an `FNewReplicatedActorInfo`), before the original is called.
pub fn route(game: &dyn Game, budgets: &Budgets, info: usize, global_info: usize) -> Branch {
    let actor = if game.readable(info, at::NEW_ACTOR_INFO_SIZE) {
        game.ptr(info)
    } else {
        0
    };
    if actor == 0 || !game.is_alive_for_replication(actor) {
        return Branch::None;
    }
    let Some(flags58) = game.u8_at(actor + at::ACTOR_FLAGS_58) else {
        return Branch::None;
    };
    let global = (global_info != 0).then_some(global_info);
    match first_class(game, actor, &ORDER) {
        Some(Class::PlayerController) => {
            let after = set_byte(game, actor + at::ACTOR_FLAGS_58, flags58, controller_flags(flags58));
            let connection = game.ptr(actor + rg::PLAYER_CONTROLLER_NET_CONNECTION);
            budgeted(game, &budgets.controller, lines::kinds::CONTROLLER, || {
                lines::route_controller(actor, connection, flags58, after)
            });
            Branch::Controller
        }
        Some(class @ (Class::PlayerState | Class::PlayerRole)) => {
            let flags59 = game.u8_at(actor + at::ACTOR_FLAGS_59).unwrap_or(0);
            let (new58, new59) = global_flags(flags58, flags59);
            let after58 = set_byte(game, actor + at::ACTOR_FLAGS_58, flags58, new58);
            let after59 = set_byte(game, actor + at::ACTOR_FLAGS_59, flags59, new59);
            let settings = apply_settings(game, global, true, PLAYER_CHANNEL_GRACE_FRAMES);
            if class == Class::PlayerState {
                budgeted(game, &budgets.player_state, lines::kinds::PLAYER_STATE, || {
                    lines::route_player_state(
                        actor,
                        &short_class(game, actor),
                        (flags58, after58),
                        (flags59, after59),
                        &settings,
                    )
                });
                Branch::PlayerState
            } else {
                budgeted(game, &budgets.role, lines::kinds::ROLE, || {
                    lines::route_role(actor, (flags58, after58), (flags59, after59), &settings)
                });
                Branch::Role
            }
        }
        Some(Class::Weapon) => {
            weapon(game, budgets, actor, flags58, global_info);
            Branch::Weapon
        }
        Some(Class::Character) => {
            let settings = apply_settings(game, global, false, PLAYER_CHANNEL_GRACE_FRAMES);
            if settings.result != 0 {
                budgeted(game, &budgets.character, lines::kinds::CHARACTER, || {
                    lines::route_character(actor, &short_class(game, actor), &settings)
                });
            }
            Branch::Character
        }
        Some(Class::PlayerCharacter) | None => Branch::None,
    }
}

/// 6.5: patches 0015 and 0016 at registration.
fn weapon(game: &dyn Game, budgets: &Budgets, actor: usize, flags58: u8, global_info: usize) {
    let dormancy_at = actor + at::ACTOR_NET_DORMANCY;
    let dormancy = game.u8_at(dormancy_at).unwrap_or(0);
    let g = if global_info != 0 && game.readable(global_info, GLOBAL_DORMANCY_SPAN) {
        global_info
    } else {
        0
    };
    let wants = |g: usize| {
        if g == 0 {
            0
        } else {
            game.u8_at(g + at::GLOBAL_WANTS_DORMANT).unwrap_or(0)
        }
    };
    let wants_before = wants(g);
    let plan = weapon_plan(flags58, dormancy, g != 0);
    let woke =
        plan.wake && game.write(dormancy_at, &[at::DORM_AWAKE]) && game.write(g + at::GLOBAL_WANTS_DORMANT, &[0]);
    let cull = if plan.cull {
        apply_settings(game, Some(g), true, 0).result
    } else {
        -2
    };
    let wants_after = wants(g);
    let line = || lines::WeaponLine {
        actor: lines::name(actor, game.name(actor)),
        owner: {
            let owner = game.ptr(actor + at::ACTOR_OWNER);
            lines::name(owner, game.name(owner))
        },
        dormancy: (dormancy, game.u8_at(dormancy_at).unwrap_or(0)),
        wants_dormant: (wants_before, wants_after),
        woke,
        cull,
        route: weapon_route(flags58, wants_after),
    };
    budgeted(game, &budgets.weapon, lines::kinds::WEAPON, || {
        lines::route_weapon(&line())
    });
}
