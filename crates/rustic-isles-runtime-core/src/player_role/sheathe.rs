//! The sheathe watch (spec-runtime-r14-omnicell-fix 3): the glider and the end of combat sheathe the weapon, and the
//! omnicell was lost there for the rest of the hunt. After each unsheathe the role is compared with the pawn's first
//! unsheathed state and, when something was lost, re-applied and replayed as at arrival.

use super::lines::{self, RoleSign, SHEATHE_KIND};
use super::{
    Game, Invoke, Route, SHEATHE_REPAIRS, SHEATHE_SETTLE_MS, State, budgeted, client::get_loadout, role_live, with,
};
use crate::offsets::r7;
use crate::offsets::r14 as at;

/// `EWeaponSheatheState::SheatheState_UNSHEATHED`.
pub const UNSHEATHED: u8 = 3;

/// The watch's per-pawn state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sheathe {
    /// The pawn the rest belongs to.
    pub pawn: usize,
    /// The first unsheathed state seen, updated by every intact check.
    pub baseline: Option<RoleSign>,
    /// When the next check is due (an unsheathe plus the settle).
    pub check_at_ms: Option<u64>,
    /// Repairs made for this pawn.
    pub repairs: u32,
}

/// The pawn's role state (3.1); `None` when the pawn is unreadable.
pub fn sign(game: &dyn Game, pawn: usize) -> Option<RoleSign> {
    if pawn == 0 || !game.readable(pawn, at::CHARACTER_SHEATHE_SPAN) {
        return None;
    }
    let role = game.ptr(pawn + at::CHARACTER_PLAYER_ROLE);
    let live = role_live(game, role) && game.readable(role, at::ROLE_SPAN);
    let bytes = if live {
        [
            at::ROLE_EQUIPPED,
            at::ROLE_GAMEPLAY_EQUIPPED,
            at::ROLE_ACTIVE_GAMEPLAY,
            at::ROLE_BP_EQUIP_CALLED,
        ]
        .map(|offset| game.u8_at(role + offset).unwrap_or(0))
    } else {
        [0; 4]
    };
    let mut guid = [0u8; 16];
    let extension = game.read(pawn + at::CHARACTER_ROLE_EXTENSION_ID, &mut guid) && guid != [0; 16];
    Some(RoleSign {
        role,
        live,
        character_active: game.u8_at(pawn + at::CHARACTER_ACTIVE_GAMEPLAY).unwrap_or(0),
        extension,
        bytes,
    })
}

/// What `now` lost against `baseline` (3.3). A different but live role is not a loss.
pub fn lost(baseline: &RoleSign, now: &RoleSign) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if baseline.live && !now.live {
        reasons.push("role");
    }
    if baseline.character_active == 1 && now.character_active != 1 {
        reasons.push("character-active");
    }
    if baseline.extension && !now.extension {
        reasons.push("anim-extension");
    }
    if now.live
        && baseline
            .bytes
            .iter()
            .zip(now.bytes)
            .any(|(&was, is)| was == 1 && is != 1)
    {
        reasons.push("role-lifecycle");
    }
    reasons
}

/// The watch's state for `pawn`, reset when the pawn changed.
fn for_pawn(state: &State, pawn: usize, body: impl FnOnce(&mut Sheathe)) {
    with(&state.sheathe, |watch| {
        if watch.pawn != pawn {
            *watch = Sheathe {
                pawn,
                ..Sheathe::default()
            };
        }
        body(watch);
    });
}

/// After `BP_PlayerCharacter_C.OnSheathedStateChanged` (3.2): params `+0x00` `PreviousState`, `+0x01` `NewState`.
pub fn sheathe_changed(game: &dyn Game, state: &State, pawn: usize, params: usize) {
    let Some(captured) = game.captured() else {
        return;
    };
    if params == 0 || game.chain_for(captured.pc, Some(pawn)).is_none() {
        return;
    }
    let (Some(previous), Some(new)) = (game.u8_at(params), game.u8_at(params + 1)) else {
        return;
    };
    let sign = sign(game, pawn).unwrap_or_default();
    budgeted(game, &state.budgets.sheathe, SHEATHE_KIND, false, || {
        lines::sheathe(previous, new, &sign)
    });
    let due = game.now_ms() + SHEATHE_SETTLE_MS;
    for_pawn(state, pawn, |watch| {
        watch.check_at_ms = (new == UNSHEATHED).then_some(due);
    });
}

/// From the client pump on a stable possession (3.3): the check an unsheathe armed, once it is due.
pub fn sheathe_check(game: &dyn Game, state: &State, pc: usize) {
    let pawn = game.chain_for(pc, None).unwrap_or(0);
    if pawn == 0 {
        return;
    }
    let now_ms = game.now_ms();
    let mut armed = None;
    for_pawn(state, pawn, |watch| {
        if watch.check_at_ms.is_some_and(|at| now_ms >= at) {
            watch.check_at_ms = None;
            armed = Some((watch.baseline, watch.repairs));
        }
    });
    let Some((baseline, repairs)) = armed else {
        return;
    };
    let Some(now) = sign(game, pawn) else {
        return;
    };
    let Some(baseline) = baseline else {
        if now.live {
            for_pawn(state, pawn, |watch| watch.baseline = Some(now));
            game.log(&lines::sheathe_baseline(&now));
        }
        return;
    };
    let reasons = lost(&baseline, &now);
    if reasons.is_empty() {
        for_pawn(state, pawn, |watch| watch.baseline = Some(now));
        game.log(&lines::sheathe_intact(&now));
        return;
    }
    if repairs >= SHEATHE_REPAIRS {
        game.log(&lines::sheathe_gave_up(&reasons));
        return;
    }
    let repair = repairs + 1;
    for_pawn(state, pawn, |watch| watch.repairs = repair);
    repair_role(game, state, pc, pawn);
    let after = sign(game, pawn).unwrap_or_default();
    game.log(&lines::sheathe_repaired(repair, &reasons, &after));
}

/// The arrival's two steps again (3.4): PR-1 through R14's wrapper, then `HandleActiveGameplayStateChanged(true, true)`.
fn repair_role(game: &dyn Game, state: &State, pc: usize, pawn: usize) {
    let loadout = get_loadout(game, state, pc);
    if loadout != 0 && game.is_live_object(loadout) {
        game.apply(loadout, true);
    }
    if game.readable(pc, r7::PC_SPAN_LOCAL) && game.ptr(pc + r7::PC_PAWN) == pawn && game.is_live_object(pawn) {
        let outcome = game.invoke(pawn, "HandleActiveGameplayStateChanged", Route::Original, &mut [1, 1]);
        if outcome == Invoke::NotFound {
            state.once(game, "ArchonCharacter.HandleActiveGameplayStateChanged", || {
                lines::not_found(lines::ROLE, "ArchonCharacter.HandleActiveGameplayStateChanged")
            });
        }
    }
}
