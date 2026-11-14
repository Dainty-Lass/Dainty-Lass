//! 4.3: the texts for fixed values.

use crate::player_role::lines::*;

const P: usize = 0x1234;
const PS: &str = "0x0000000000001234";

#[test]
fn role_lines() {
    assert_eq!(
        apply(P, "PR_TEMPEST", 0, "", true),
        format!("[MitPlayerRole] apply loadout={PS} item=PR_TEMPEST pawnRole=null pawnRoleItem=- applied=1")
    );
    assert_eq!(
        retry_gave_up(P, 30_000, 3),
        format!("[MitPlayerRole] retry: gave up loadout={PS} after 30000 ms retries=3")
    );
    assert_eq!(
        retry_gone(P),
        format!("[MitPlayerRole] retry: loadout={PS} is gone (world torn down), dropped")
    );
    assert_eq!(
        retry_waiting(P, 0, P, 0),
        format!("[MitPlayerRole] retry: waiting loadout={PS} item=null slot={PS} pawn=null")
    );
    assert_eq!(
        retry_attempt(2, P, 3000, false),
        format!("[MitPlayerRole] retry: attempt=2 loadout={PS} elapsedMs=3000 applied=0")
    );
    assert_eq!(
        connections(-1, 1),
        "[MitPlayerRole] connections -1 -> 1: re-arming the role refresh"
    );
    assert_eq!(
        refresh_push(1, P, "PR_X", 1),
        format!("[MitPlayerRole] refresh: push=1 role={PS} item=PR_X dormancy=1")
    );
    assert_eq!(
        purge(P, PurgeReason::RouteRemove, 1, 0),
        format!("[MitPlayerRole] purge role={PS} reason=route-remove roles=1 tempest=0")
    );
    assert_eq!(
        tempest_repair(1, P, (true, true), (false, false)),
        format!("[MitPlayerRole] tempest: repair=1 role={PS} buffs=1/1 abilities=0/0")
    );
    assert_eq!(
        ability(9, true, false, Some("Owner")),
        "[MitPlayerRole] ability: handle=9 inputPressed=1 activated=0 owner=Owner"
    );
    assert_eq!(
        client_replayed("pump", [0, 0, 1], [1, 1, 1]),
        "[MitPlayerRole] client: lifecycle replayed trigger=pump gameplayEquipped=0->1 roleActive=0->1 \
         bpEquipCalled=1->1"
    );
    assert_eq!(
        suppressed(APPLY_KIND),
        "[MitPlayerRole] apply: further lines suppressed"
    );
}

#[test]
fn input_and_hud_lines() {
    assert_eq!(
        new_possession(P, 0),
        format!("[MitInput] new possession pc={PS} pawn=null")
    );
    assert_eq!(
        cinematic_skipped("timeout", false, true, -1, true),
        "[MitInput] cinematic clear skipped trigger=timeout allow=0 edge=1 gameInput=-1 stable=1"
    );
    assert_eq!(
        input_configuration(Some(0), Some(1)),
        "[MitInput] input configuration Hunter -> 0, Default -> 1"
    );
    assert_eq!(
        online_data_ready("event", 0, 1),
        "[MitInput] activate trigger=event: bOnlineDataReady was 0, now 1"
    );
    assert_eq!(
        not_found(INPUT, "Controller.ResetIgnoreMoveInput"),
        "[MitInput] Controller.ResetIgnoreMoveInput not found"
    );
    assert_eq!(
        hud_restore("pump", RestoreReason::Timeout, 45_000, 1, 0),
        "[MitHud] restore trigger=pump reason=timeout sinceAirshipEndMs=45000 visibility=1->0"
    );
    assert_eq!(
        airship_ended("state-update"),
        "[MitHud] airship ended trigger=state-update; the combat HUD stays hidden until the player has control"
    );
}
