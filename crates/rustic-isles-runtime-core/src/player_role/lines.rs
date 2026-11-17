//! Every R14 line (4.3): `[MitPlayerRole]` 5 to 10, `[MitInput]` 11, `[MitHud]` 12. No account id or key.

/// The role tag.
pub const ROLE: &str = "[MitPlayerRole]";
/// The input tag.
pub const INPUT: &str = "[MitInput]";
/// The HUD tag.
pub const HUD: &str = "[MitHud]";

/// A pointer: `0x` and 16 lowercase hex digits, `null` for 0 (R7's form).
pub fn p(ptr: usize) -> String {
    if ptr == 0 {
        "null".to_owned()
    } else {
        format!("0x{ptr:016x}")
    }
}

/// An id, `-` when empty.
fn id(text: &str) -> &str {
    if text.is_empty() { "-" } else { text }
}

fn bit(value: bool) -> u8 {
    u8::from(value)
}

/// A budget's last line (the tag is the kind's).
pub fn suppressed(kind: &str) -> String {
    format!("{kind}: further lines suppressed")
}

/// The apply line's budget kind.
pub const APPLY_KIND: &str = "[MitPlayerRole] apply";
/// The ability line's budget kind.
pub const ABILITY_KIND: &str = "[MitPlayerRole] ability";
/// The input mode line's budget kind.
pub const INPUT_MODE_KIND: &str = "[MitInput] input mode";
/// The hide line's budget kind.
pub const HIDE_KIND: &str = "[MitHud] hide";

/// 5.2 step 4.
pub fn apply(loadout: usize, item: &str, pawn_role: usize, pawn_role_item: &str, applied: bool) -> String {
    format!(
        "{ROLE} apply loadout={} item={} pawnRole={} pawnRoleItem={} applied={}",
        p(loadout),
        id(item),
        p(pawn_role),
        id(pawn_role_item),
        bit(applied)
    )
}

/// 5.4 step 1.
pub fn retry_gave_up(loadout: usize, ms: u64, retries: u32) -> String {
    format!(
        "{ROLE} retry: gave up loadout={} after {ms} ms retries={retries}",
        p(loadout)
    )
}

/// 5.4 step 4.
pub fn retry_gone(loadout: usize) -> String {
    format!(
        "{ROLE} retry: loadout={} is gone (world torn down), dropped",
        p(loadout)
    )
}

/// 5.4 step 6.
pub fn retry_waiting(loadout: usize, item: usize, slot: usize, pawn: usize) -> String {
    format!(
        "{ROLE} retry: waiting loadout={} item={} slot={} pawn={}",
        p(loadout),
        p(item),
        p(slot),
        p(pawn)
    )
}

/// 5.4 step 7.
pub fn retry_attempt(attempt: u32, loadout: usize, elapsed_ms: u64, applied: bool) -> String {
    format!(
        "{ROLE} retry: attempt={attempt} loadout={} elapsedMs={elapsed_ms} applied={}",
        p(loadout),
        bit(applied)
    )
}

/// 7.1.
pub fn role_table_full(role: usize) -> String {
    format!("{ROLE} role table full, role={} not watched", p(role))
}

/// 7.3.
pub fn connections(old: i64, new: i64) -> String {
    format!("{ROLE} connections {old} -> {new}: re-arming the role refresh")
}

/// 7.4 step 3.
pub fn refresh_wait(role: usize, item: &str) -> String {
    format!("{ROLE} refresh: wait role={} item={}", p(role), id(item))
}

/// 7.4 step 5.
pub fn refresh_done(role: usize, item: &str, pushes: u32) -> String {
    format!(
        "{ROLE} refresh: done role={} item={} pushes={pushes}",
        p(role),
        id(item)
    )
}

/// 7.4 step 6.
pub fn refresh_push(push: u32, role: usize, item: &str, dormancy: u8) -> String {
    format!(
        "{ROLE} refresh: push={push} role={} item={} dormancy={dormancy}",
        p(role),
        id(item)
    )
}

/// Why a role left the tables (9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeReason {
    /// RG-3.
    RouteRemove,
    /// 7.4 step 1.
    TickInvalidActor,
}

impl PurgeReason {
    /// The line's text.
    pub fn text(self) -> &'static str {
        match self {
            Self::RouteRemove => "route-remove",
            Self::TickInvalidActor => "tick-invalid-actor",
        }
    }
}

/// 9.
pub fn purge(role: usize, reason: PurgeReason, roles: usize, tempest: usize) -> String {
    format!(
        "{ROLE} purge role={} reason={} roles={roles} tempest={tempest}",
        p(role),
        reason.text()
    )
}

/// 8.1.
pub fn tempest_full(role: usize) -> String {
    format!("{ROLE} tempest watch full, role={} not watched", p(role))
}

/// 8.1.
pub fn tempest_watching(role: usize) -> String {
    format!("{ROLE} tempest: watching role={}", p(role))
}

/// 8.3 step 3.
pub fn tempest_active(role: usize) -> String {
    format!("{ROLE} tempest: active, waiting role={}", p(role))
}

/// 8.3 step 5.
pub fn tempest_unreadable(role: usize) -> String {
    format!("{ROLE} tempest: unreadable role={}", p(role))
}

/// 8.3 step 6.
pub fn tempest_healthy(role: usize) -> String {
    format!("{ROLE} tempest: healthy role={}", p(role))
}

/// 8.3 step 7.
pub fn tempest_catalog_empty(role: usize) -> String {
    format!("{ROLE} tempest: catalog empty role={}", p(role))
}

/// 8.3 step 8.
pub fn tempest_pending(role: usize, active_for_ms: u64) -> String {
    format!("{ROLE} tempest: pending role={} activeForMs={active_for_ms}", p(role))
}

/// 8.3 step 10.
pub fn tempest_gave_up(role: usize, repairs: u32) -> String {
    format!("{ROLE} tempest: gave up role={} repairs={repairs}", p(role))
}

/// 8.3 step 11: `(requested, dispatched)` for the buffs and the abilities.
pub fn tempest_repair(repair: u32, role: usize, buffs: (bool, bool), abilities: (bool, bool)) -> String {
    format!(
        "{ROLE} tempest: repair={repair} role={} buffs={}/{} abilities={}/{}",
        p(role),
        bit(buffs.0),
        bit(buffs.1),
        bit(abilities.0),
        bit(abilities.1)
    )
}

/// 10 step 4.
pub fn ability(handle: u32, pressed: bool, activated: bool, owner: Option<&str>) -> String {
    format!(
        "{ROLE} ability: handle={handle} inputPressed={} activated={} owner={}",
        bit(pressed),
        bit(activated),
        owner.unwrap_or("null")
    )
}

/// 10 step 2: the first bytes did not check.
pub const ABILITY_DISABLED: &str =
    "[MitPlayerRole] ability: InternalTryActivateAbility's first bytes did not match; the route is off";

/// 11.5 step 2.
pub const CLIENT_ID_EMPTY: &str = "[MitPlayerRole] client: PlayerRoleId empty, waiting for replication";
/// 11.5 step 2.
pub const CLIENT_PAUSE_OVER: &str = "[MitPlayerRole] client: retrying the role after a 30 s pause";
/// 11.5 step 2.
pub const CLIENT_NO_LOADOUT: &str = "[MitPlayerRole] client: no live loadout to re-apply; will retry";

/// 11.5 step 2.
pub fn client_reapplied(attempt: u32, role_id: &str, role_after: usize) -> String {
    format!(
        "{ROLE} client: re-applied the role attempt={attempt} id={} roleAfter={}",
        id(role_id),
        p(role_after)
    )
}

/// The three gameplay bytes before and after (11.5 step 4).
pub type Bytes3 = [u8; 3];

/// 11.5 step 4.
pub fn client_replayed(trigger: &str, before: Bytes3, after: Bytes3) -> String {
    format!(
        "{ROLE} client: lifecycle replayed trigger={trigger} gameplayEquipped={}->{} roleActive={}->{} \
         bpEquipCalled={}->{}",
        before[0], after[0], before[1], after[1], before[2], after[2]
    )
}

/// 11.1.
pub fn new_possession(pc: usize, pawn: usize) -> String {
    format!("{INPUT} new possession pc={} pawn={}", p(pc), p(pawn))
}

/// 11.2 step 4.
pub const WAITING_STABLE: &str = "[MitInput] waiting for a stable possession; pump skipped";

/// 11.2 step 5.
pub fn deferred_replayed(ms: u64) -> String {
    format!("{INPUT} activate: deferred event replayed after {ms} ms")
}

/// 11.2 step 5.
pub fn activation_timeout(ms: u64) -> String {
    format!("{INPUT} activate: timeout after {ms} ms, restoring input without the cinematic clear")
}

/// 11.3 step 3.
pub fn input_mode(game_input: bool, from_runtime: bool) -> String {
    format!(
        "{INPUT} input mode gameInput={} source={}",
        bit(game_input),
        if from_runtime { "runtime" } else { "game" }
    )
}

/// 11.3 step 4.
pub const IGNORED_OTHER: &str = "[MitInput] ignored an input mode change from another controller or pawn";

/// 11.3 step 4.
pub fn activation_deferred(ms: u64) -> String {
    format!("{INPUT} activate: deferred, {ms} ms since the possession (settle 3000 ms)")
}

/// 11.4 step 1.
pub fn online_data_ready(trigger: &str, was: u8, now: u8) -> String {
    format!("{INPUT} activate trigger={trigger}: bOnlineDataReady was {was}, now {now}")
}

/// 11.4 step 4.
pub const SLOT_INVALID: &str = "[MitInput] active loadout slot was invalid; slot 0 applied";

/// 11.4 step 4.
pub fn slot_kept(slot: i32) -> String {
    format!("{INPUT} kept the active loadout slot {slot}")
}

/// 11.4 step 4.
pub fn role_reapplied(slot: i32) -> String {
    format!("{INPUT} re-applied the role for slot {slot}")
}

/// 11.4 step 4.
pub const LIFECYCLE_DEFERRED: &str = "[MitInput] role lifecycle deferred until the possession settles";
/// 11.4 step 4.
pub const NO_LOADOUT: &str = "[MitInput] no loadout; slot and role skipped";
/// 11.4 step 6.
pub const CINEMATIC_CLEARED: &str = "[MitInput] cinematic mode cleared";

/// 11.4 step 6.
pub fn cinematic_skipped(trigger: &str, allow: bool, edge: bool, game_input: i8, stable: bool) -> String {
    format!(
        "{INPUT} cinematic clear skipped trigger={trigger} allow={} edge={} gameInput={game_input} stable={}",
        bit(allow),
        bit(edge),
        bit(stable)
    )
}

/// 11.4 step 7: each return byte, `-` when the call was not made.
pub fn input_configuration(hunter: Option<u8>, default: Option<u8>) -> String {
    let text = |value: Option<u8>| value.map_or_else(|| "-".to_owned(), |value| value.to_string());
    format!(
        "{INPUT} input configuration Hunter -> {}, Default -> {}",
        text(hunter),
        text(default)
    )
}

/// 11.4: a function not found, once per name.
pub fn not_found(tag: &str, label: &str) -> String {
    format!("{tag} {label} not found")
}

/// 11.6.
pub fn movement_repair(field: &str, old: u8, new: u8) -> String {
    format!("{INPUT} movement repair: {field} {old} -> {new}")
}

/// 12.3 step 4.
pub fn hud_hide(trigger: &str, before: u8, after: u8) -> String {
    format!("{HUD} hide trigger={trigger} visibility={before}->{after}")
}

/// 12.3 step 5.
pub fn airship_ended(trigger: &str) -> String {
    format!("{HUD} airship ended trigger={trigger}; the combat HUD stays hidden until the player has control")
}

/// Why the combat HUD came back (12.3 step 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreReason {
    /// The player has had control for a second.
    PlayerControl,
    /// 45 s after the airship.
    Timeout,
}

/// 12.3 step 5.
pub fn hud_restore(trigger: &str, reason: RestoreReason, since_ms: u64, before: u8, after: u8) -> String {
    let reason = match reason {
        RestoreReason::PlayerControl => "player-control",
        RestoreReason::Timeout => "timeout",
    };
    format!("{HUD} restore trigger={trigger} reason={reason} sinceAirshipEndMs={since_ms} visibility={before}->{after}")
}

/// The sheathe line's budget kind (r14-omnicell-fix 3).
pub const SHEATHE_KIND: &str = "[MitPlayerRole] client: sheathe";

/// What the sheathe watch reads of the pawn and its role (r14-omnicell-fix 3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoleSign {
    /// The pawn's `PlayerRole`.
    pub role: usize,
    /// The role is live (7.1's `role_live`).
    pub live: bool,
    /// The character's `bIsActiveGameplay`.
    pub character_active: u8,
    /// The character's role anim-state extension id is set.
    pub extension: bool,
    /// The role's four lifecycle bytes (`0x2F4..=0x2F7`), 0 when no live role.
    pub bytes: [u8; 4],
}

fn sign_text(sign: &RoleSign) -> String {
    format!(
        "role={} live={} characterActive={} extension={} roleBytes={}{}{}{}",
        p(sign.role),
        bit(sign.live),
        sign.character_active,
        bit(sign.extension),
        sign.bytes[0],
        sign.bytes[1],
        sign.bytes[2],
        sign.bytes[3]
    )
}

/// r14-omnicell-fix 3.2.
pub fn sheathe(previous: u8, new: u8, sign: &RoleSign) -> String {
    format!("{ROLE} client: sheathe {previous}->{new} {}", sign_text(sign))
}

/// r14-omnicell-fix 3.3: the first unsheathed check of a pawn.
pub fn sheathe_baseline(sign: &RoleSign) -> String {
    format!("{ROLE} client: role baseline after unsheathe {}", sign_text(sign))
}

/// r14-omnicell-fix 3.3: nothing lost since the baseline.
pub fn sheathe_intact(sign: &RoleSign) -> String {
    format!("{ROLE} client: role intact after unsheathe {}", sign_text(sign))
}

/// r14-omnicell-fix 3.4: what was lost, and the state after the repair.
pub fn sheathe_repaired(repair: u32, reasons: &[&str], after: &RoleSign) -> String {
    format!(
        "{ROLE} client: role repaired after unsheathe repair={repair} lost={} after: {}",
        reasons.join(","),
        sign_text(after)
    )
}

/// r14-omnicell-fix 3.4: the per-pawn budget ran out.
pub fn sheathe_gave_up(reasons: &[&str]) -> String {
    format!(
        "{ROLE} client: role lost after unsheathe lost={}; repairs exhausted for this pawn",
        reasons.join(",")
    )
}
