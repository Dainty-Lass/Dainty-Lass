//! Every `[MitRelevance]` text (4.3): addresses `0x<HEX>` or `null`, flag bytes `0x<HH>`, floats without decimals.

use super::{Settings, WakeCall};

/// R13's tag.
pub const TAG: &str = "[MitRelevance]";

/// The relevance pass's first line (9.2).
pub const PASS_STARTED: &str = "[MitRelevance] relevance re-assert on change, else every 15 s";

/// Budget kinds.
pub mod kinds {
    /// 6.2.
    pub const CONTROLLER: &str = "route: controller";
    /// 6.3.
    pub const PLAYER_STATE: &str = "route: player-state";
    /// 6.4.
    pub const ROLE: &str = "route: role";
    /// 6.5.
    pub const WEAPON: &str = "route: weapon";
    /// 6.6.
    pub const CHARACTER: &str = "route: character";
    /// 7.2 step 8.
    pub const CULL_CLEARED: &str = "weapon: cull cleared";
    /// 7.2 step 9.
    pub const WOKE: &str = "weapon: woke";
    /// 9.3 step 1.
    pub const PAWN_GRACE: &str = "pawn grace: corrected";
    /// 9.3 step 2.
    pub const ROLE_CULL: &str = "role cull: corrected";
}

/// `[MitRelevance] <kind>: further lines suppressed`.
pub fn suppressed(kind: &str) -> String {
    format!("{TAG} {kind}: further lines suppressed")
}

/// An address: `0x<HEX>`, or `null` for 0.
pub fn addr(ptr: usize) -> String {
    if ptr == 0 {
        "null".to_owned()
    } else {
        format!("0x{ptr:X}")
    }
}

/// A flag byte.
pub fn byte(value: u8) -> String {
    format!("0x{value:02X}")
}

/// A name: `null` for a null object, `unreadable` when it cannot be read.
pub fn name(object: usize, name: Option<String>) -> String {
    match (object, name) {
        (0, _) => "null".to_owned(),
        (_, Some(name)) => name,
        (_, None) => "unreadable".to_owned(),
    }
}

fn settings_text(settings: &Settings) -> String {
    let (cull, cull_sq, grace) = settings.before;
    let (cull_after, cull_sq_after, grace_after) = settings.after;
    format!(
        "settings={} cull={cull:.0}->{cull_after:.0} cullSq={cull_sq:.0}->{cull_sq_after:.0} grace={grace}->{grace_after}",
        settings.result
    )
}

/// 6.2.
pub fn route_controller(actor: usize, connection: usize, before: u8, after: u8) -> String {
    format!(
        "{TAG} route: controller actor={} connection={} flags58={}->{} route=owner",
        addr(actor),
        addr(connection),
        byte(before),
        byte(after)
    )
}

/// 6.3.
pub fn route_player_state(
    actor: usize,
    class: &str,
    flags58: (u8, u8),
    flags59: (u8, u8),
    settings: &Settings,
) -> String {
    format!(
        "{TAG} route: player-state actor={} class={class} flags58={}->{} flags59={}->{} {} route=global",
        addr(actor),
        byte(flags58.0),
        byte(flags58.1),
        byte(flags59.0),
        byte(flags59.1),
        settings_text(settings)
    )
}

/// 6.4.
pub fn route_role(actor: usize, flags58: (u8, u8), flags59: (u8, u8), settings: &Settings) -> String {
    format!(
        "{TAG} route: role actor={} flags58={}->{} flags59={}->{} {} route=global",
        addr(actor),
        byte(flags58.0),
        byte(flags58.1),
        byte(flags59.0),
        byte(flags59.1),
        settings_text(settings)
    )
}

/// 6.5's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponLine {
    /// The weapon's name.
    pub actor: String,
    /// Its owner's name.
    pub owner: String,
    /// `NetDormancy` before and after.
    pub dormancy: (u8, u8),
    /// `bWantsToBeDormant` before and after.
    pub wants_dormant: (u8, u8),
    /// Both dormancy writes made.
    pub woke: bool,
    /// −2 when 0016 did not apply, else `apply_settings`'s answer.
    pub cull: i32,
    /// `global`, `owner`, `spatial-static` or `spatial-dynamic`.
    pub route: &'static str,
}

/// 6.5.
pub fn route_weapon(line: &WeaponLine) -> String {
    format!(
        "{TAG} route: weapon actor={} owner={} dormancy={}->{} wantsDormant={}->{} woke={} cull={} route={}",
        line.actor,
        line.owner,
        line.dormancy.0,
        line.dormancy.1,
        line.wants_dormant.0,
        line.wants_dormant.1,
        u8::from(line.woke),
        line.cull,
        line.route
    )
}

/// 6.6, when the answer is not 0.
pub fn route_character(actor: usize, class: &str, settings: &Settings) -> String {
    format!(
        "{TAG} route: character actor={} class={class} settings={} grace={}->{}",
        addr(actor),
        settings.result,
        settings.before.2,
        settings.after.2
    )
}

/// The line's `info=` text (7.2).
pub fn info_text(found: bool) -> &'static str {
    if found { "unsent" } else { "none" }
}

/// 7.2 step 8.
pub fn weapon_cull_cleared(
    weapon: &str,
    pawn: &str,
    connection: usize,
    flags58: u8,
    global: i32,
    cleared: u32,
    info: bool,
) -> String {
    format!(
        "{TAG} weapon: cull cleared weapon={weapon} pawn={pawn} connection={} flags58={} global={global} \
         snapshotsCleared={cleared} info={}",
        addr(connection),
        byte(flags58),
        info_text(info)
    )
}

/// 7.2 step 9.
pub fn weapon_woke(
    weapon: &str,
    pawn: &str,
    connection: usize,
    dormancy: (u8, u8),
    wants_dormant: (u8, u8),
    info: bool,
    call: WakeCall,
) -> String {
    format!(
        "{TAG} weapon: woke weapon={weapon} pawn={pawn} connection={} dormancy={}->{} wantsDormant={}->{} info={} \
         call={}",
        addr(connection),
        dormancy.0,
        dormancy.1,
        wants_dormant.0,
        wants_dormant.1,
        info_text(info),
        call.text()
    )
}

/// 9.3 step 1.
pub fn pawn_grace(pawn: usize, connection: usize, settings: &Settings) -> String {
    format!(
        "{TAG} pawn grace: corrected pawn={} connection={} grace={}->{}",
        addr(pawn),
        addr(connection),
        settings.before.2,
        settings.after.2
    )
}

/// 9.3 step 2.
pub fn role_cull(role: usize, pawn: usize, settings: &Settings, cleared: u32) -> String {
    let (cull, cull_sq, grace) = settings.before;
    let (cull_after, cull_sq_after, grace_after) = settings.after;
    format!(
        "{TAG} role cull: corrected role={} pawn={} global={} snapshotsCleared={cleared} cull={cull:.0}->{cull_after:.0} \
         cullSq={cull_sq:.0}->{cull_sq_after:.0} grace={grace}->{grace_after}",
        addr(role),
        addr(pawn),
        settings.result
    )
}
