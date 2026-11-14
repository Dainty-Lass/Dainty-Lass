//! LD-1, the loading gate (4): the server's compatibility answer and the client's two narrow overrides, with the
//! client's own `ClientRestart` capture (4.4), which R14 takes over.

use super::super::{flags, log};
use super::game::{self, read_i32, read_ptr, read_u8, readable};
use crate::handlers::PeCall;
use rustic_isles_runtime_core::loading::{FORCED_LOG_BUDGET, Possession, TriggerSigns, force_controller, lines};
use rustic_isles_runtime_core::offsets::r7;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

const TRIGGER: &str = "island_arrival_cinematic_trigger_bp_C";
/// Trigger objects whose release was logged, newest last.
const MAX_RELEASED: usize = 64;

static POSSESSION: Mutex<Possession> = Mutex::new(Possession::new());
static RELEASED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static SERVER_FORCED: AtomicU32 = AtomicU32::new(0);
static CLIENT_FORCED: AtomicU32 = AtomicU32::new(0);

fn possession() -> MutexGuard<'static, Possession> {
    POSSESSION.lock().unwrap_or_else(PoisonError::into_inner)
}

/// SP-1 after `ClientRestart`: the controller, the new pawn and the world (4.4). Never logs.
pub fn capture(call: &PeCall) {
    let pawn = if call.params == 0 {
        0
    } else {
        read_ptr(call.params).unwrap_or(0)
    };
    possession().capture(call.object, pawn, game::world(), game::now());
}

/// `pc` is live, readable, and its `Player` is a live `LocalPlayer` pointing back at it.
fn local(pc: usize) -> bool {
    if !readable(pc, r7::PC_SPAN_LOCAL) || !game::is_live(pc) {
        return false;
    }
    let player = read_ptr(pc + r7::PC_PLAYER).unwrap_or(0);
    player != 0
        && game::is_live(player)
        && readable(player, r7::PLAYER_SPAN)
        && game::is_a(player, "LocalPlayer")
        && read_ptr(player + r7::PLAYER_CONTROLLER) == Some(pc)
}

/// 4.4 steps 2 and 3: the pawn, when every read passes.
fn possessed_pawn(pc: usize) -> Option<usize> {
    if !local(pc) {
        return None;
    }
    let pawn = read_ptr(pc + r7::PC_PAWN).filter(|&pawn| pawn != 0)?;
    if !game::is_live(pawn) || !readable(pawn, r7::PAWN_SPAN) {
        return None;
    }
    let state = read_ptr(pawn + r7::PAWN_PLAYER_STATE).filter(|&state| state != 0)?;
    (game::is_live(state) && readable(state, r7::PLAYER_STATE_SPAN)).then_some(pawn)
}

fn stable_locked(possession: &mut Possession, pc: usize, now: u64) -> Option<usize> {
    let pawn = if pc != 0 && pc == possession.controller() {
        possessed_pawn(pc)
    } else {
        None
    };
    possession.stable(pc, pawn, now)
}

/// `possession_stable(pc)`: the pawn once the captured possession has settled (4.4). R14 reads it.
pub fn possession_stable(pc: usize) -> Option<usize> {
    let mut possession = possession();
    possession.check_world(game::world());
    stable_locked(&mut possession, pc, game::now())
}

/// `KEEP_ARRIVAL_TRIGGER_WAIT.flag` absent, read once.
fn trigger_release_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let on = !flags::flag("KEEP_ARRIVAL_TRIGGER_WAIT");
        log::line(lines::trigger_release(on));
        on
    })
}

fn trigger_signs(trigger: usize) -> Option<TriggerSigns> {
    Some(TriggerSigns {
        slayer_actors: read_i32(trigger + r7::TRIGGER_SLAYERS_NUM)?,
        intro_refs: read_i32(trigger + r7::TRIGGER_INTRO_NUM)?,
        client_started: read_u8(trigger + r7::TRIGGER_CLIENT_STARTED)?,
        loading_state: read_u8(trigger + r7::TRIGGER_LOADING_STATE)?,
        primary_cine_actor: read_ptr(trigger + r7::TRIGGER_PRIMARY_CINE)?,
        approach_actor: read_ptr(trigger + r7::TRIGGER_APPROACH)?,
    })
}

/// 4.3: the idle Training Grounds trigger, released once the possession is stable.
fn idle_trigger(trigger: usize, possession: &mut Possession, now: u64) -> bool {
    if !trigger_release_on() || !readable(trigger, r7::TRIGGER_SPAN) {
        return false;
    }
    if !trigger_signs(trigger).is_some_and(|signs| signs.idle()) {
        return false;
    }
    let pc = possession.controller();
    if pc == 0 || !game::is_live(pc) || stable_locked(possession, pc, now).is_none() {
        return false;
    }
    let mut released = RELEASED.lock().unwrap_or_else(PoisonError::into_inner);
    if !released.contains(&trigger) {
        if released.len() == MAX_RELEASED {
            released.remove(0);
        }
        released.push(trigger);
        log::line(&lines::trigger_released(possession.since_began(now).unwrap_or(0)));
    }
    true
}

/// 4.2: the local controller, last resort for a load that is really stuck.
fn local_controller(object: usize, classes: &[String], possession: &mut Possession, now: u64) -> bool {
    if !classes.iter().any(|name| name == "ArchonPlayerController") || !local(object) {
        return false;
    }
    if object != possession.controller() {
        return false;
    }
    let Some(pawn) = stable_locked(possession, object, now) else {
        return false;
    };
    let weapon = read_ptr(pawn + r7::CHARACTER_WEAPON).unwrap_or(0);
    let since = possession.since_began(now).unwrap_or(0);
    if !force_controller(true, weapon, since) {
        return false;
    }
    if CLIENT_FORCED.fetch_add(1, Ordering::Relaxed) < FORCED_LOG_BUDGET {
        log::line(&lines::client_forced(game::full_name(object).as_deref(), since));
    }
    true
}

fn client_ready(object: usize) -> bool {
    if object == 0 {
        return false;
    }
    let classes = game::class_names(object);
    let now = game::now();
    let mut possession = possession();
    possession.check_world(game::world());
    if classes.iter().any(|name| name == TRIGGER) {
        return idle_trigger(object, &mut possession, now);
    }
    local_controller(object, &classes, &mut possession, now)
}

/// LD-1's body (4.1): the original first; its true answer is never changed.
pub fn has_finished_loading(object: usize, server: bool, next: &mut dyn FnMut() -> u8) -> u8 {
    let answer = next();
    if answer != 0 {
        return answer;
    }
    if server {
        if !flags::production_log_profile() && SERVER_FORCED.fetch_add(1, Ordering::Relaxed) < FORCED_LOG_BUDGET {
            log::line(&lines::server_forced(game::full_name(object).as_deref()));
        }
        return 1;
    }
    u8::from(client_ready(object))
}
