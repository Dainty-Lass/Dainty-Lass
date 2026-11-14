//! 6 to 10: the role table and refresh, the Tempest repair, RG-3's purge and the ability route.

use super::*;
use crate::player_role::server::{add_role, array_header, modifiers};

fn pump(fake: &Fake) {
    fake.advance(1000);
    server_pump(fake, &fake.state);
}

fn pushes(fake: &Fake) -> usize {
    fake.calls_of("ForceNetUpdate").len()
}

#[test]
fn the_role_table_keeps_one_entry_per_role_and_logs_full_once() {
    let fake = Fake::new();
    let role = fake.role("PR_ICEBORNE");
    add_role(&fake, &fake.state, role);
    add_role(&fake, &fake.state, role);
    assert_eq!(fake.state.roles.borrow().len(), 1);
    for n in 1..256 {
        add_role(&fake, &fake.state, n * 0x10_0000_0000);
    }
    assert_eq!(fake.state.roles.borrow().len(), 256);
    add_role(&fake, &fake.state, 0x7777_0000_0000);
    add_role(&fake, &fake.state, 0x7778_0000_0000);
    assert_eq!(fake.count_lines("role table full, role="), 1);
}

#[test]
fn the_refresh_cycle() {
    let fake = Fake::new();
    let role = fake.role("PR_ICEBORNE");
    add_role(&fake, &fake.state, role);
    fake.set_active(role, false);
    pump(&fake);
    assert!(!fake.state.roles.borrow()[0].was_active);
    fake.set_active(role, true);
    pump(&fake);
    assert!(fake.has_line("refresh: wait role="));
    pump(&fake);
    assert_eq!(pushes(&fake), 0, "1 000 ms is not settled");
    pump(&fake);
    assert_eq!(pushes(&fake), 1);
    assert_eq!(fake.call_names(), ["FlushNetDormancy", "ForceNetUpdate"]);
    assert!(fake.calls.borrow().iter().all(|call| call.route == Route::Original));
    assert!(fake.has_line("refresh: push=1 role="));
    pump(&fake);
    assert_eq!(pushes(&fake), 1, "1 000 ms after a push waits");
    pump(&fake);
    assert_eq!(pushes(&fake), 2);
    fake.advance(1500);
    server_pump(&fake, &fake.state);
    assert_eq!(pushes(&fake), 3);
    fake.advance(1500);
    server_pump(&fake, &fake.state);
    assert!(fake.has_line("refresh: done role="));
    for _ in 0..5 {
        fake.advance(1500);
        server_pump(&fake, &fake.state);
    }
    assert_eq!(pushes(&fake), 3, "done");
    // Inactive, then active again: another cycle
    fake.set_active(role, false);
    pump(&fake);
    fake.set_active(role, true);
    for _ in 0..8 {
        fake.advance(1500);
        server_pump(&fake, &fake.state);
    }
    assert_eq!(pushes(&fake), 6);
}

#[test]
fn a_connection_count_change_rearms_every_role() {
    let fake = Fake::new();
    let role = fake.role("PR_ICEBORNE");
    add_role(&fake, &fake.state, role);
    fake.set_active(role, true);
    fake.connections.set(Some(1));
    for _ in 0..10 {
        fake.advance(1500);
        server_pump(&fake, &fake.state);
    }
    assert!(fake.has_line("connections -1 -> 1: re-arming the role refresh"));
    assert_eq!(pushes(&fake), 3);
    fake.connections.set(Some(2));
    for _ in 0..10 {
        fake.advance(1500);
        server_pump(&fake, &fake.state);
    }
    assert!(fake.has_line("connections 1 -> 2: re-arming the role refresh"));
    assert_eq!(pushes(&fake), 6);
}

#[test]
fn role_live_refuses_each_destroying_mark() {
    let fake = Fake::new();
    let role = fake.role("PR_ICEBORNE");
    assert!(role_live(&fake, role));
    fake.put_u8(role + 0x5C, 0x08);
    assert!(!role_live(&fake, role), "the destroying bit");
    fake.put_u8(role + 0x5C, 0);
    fake.put_u32(role + 0x08, 0x8000);
    assert!(!role_live(&fake, role), "RF_BeginDestroyed");
    fake.put_u32(role + 0x08, 0x10000);
    assert!(!role_live(&fake, role), "RF_FinishDestroyed");
    fake.put_u32(role + 0x08, 0);
    fake.kill(role);
    assert!(!role_live(&fake, role));
}

#[test]
fn a_dead_role_is_purged_by_the_tick() {
    let fake = Fake::new();
    let role = fake.role("PR_TEMPEST");
    add_role(&fake, &fake.state, role);
    fake.put_u8(role + 0x5C, 0x08);
    pump(&fake);
    assert!(fake.has_line("reason=tick-invalid-actor roles=1 tempest=1"));
    assert!(fake.state.roles.borrow().is_empty());
}

#[test]
fn rg3_purges_a_role_in_both_tables_with_one_line() {
    let fake = Fake::new();
    let role = fake.role("PR_TEMPEST");
    add_role(&fake, &fake.state, role);
    assert!(fake.has_line("tempest: watching role="));
    let info = fake.block(0x10);
    fake.put(info, role);
    route_remove(&fake, &fake.state, info);
    assert_eq!(fake.count_lines("purge role="), 1);
    assert!(fake.has_line("reason=route-remove roles=1 tempest=1"));
    assert!(fake.state.roles.borrow().is_empty() && fake.state.tempest.borrow().is_empty());
    let unknown = fake.role("PR_ICEBORNE");
    fake.put(info, unknown);
    fake.take_lines();
    route_remove(&fake, &fake.state, info);
    assert!(fake.take_lines().is_empty(), "an unknown role logs nothing");
    let not_role = fake.block(0x100);
    fake.put(info, not_role);
    route_remove(&fake, &fake.state, info);
    route_remove(&fake, &fake.state, 0);
    assert!(fake.take_lines().is_empty());
}

/// A Tempest role with its catalog and modifier group.
fn tempest_role(fake: &Fake, desired: (i32, i32), applied: (i32, i32), pending: (i32, i32)) -> (usize, usize) {
    let role = fake.role("PR_TEMPEST");
    let buffs = fake.block(0x40);
    let abilities = fake.block(0x40);
    fake.put(role + 0x2A8, buffs);
    fake.put_i32(role + 0x2B0, desired.0);
    fake.put_i32(role + 0x2B4, desired.0);
    fake.put(role + 0x2C8, abilities);
    fake.put_i32(role + 0x2D0, desired.1);
    fake.put_i32(role + 0x2D4, desired.1);
    let group = fake.block(0xD0);
    fake.put(role + 0x2F8, group);
    fake.put_i32(group + 0x88, applied.0);
    fake.put_i32(group + 0x78, applied.1);
    fake.put_i32(group + 0xA8, pending.0);
    fake.put_i32(group + 0xC8, pending.1);
    fake.set_active(role, true);
    (role, group)
}

fn settle(fake: &Fake, role: usize) {
    add_role(fake, &fake.state, role);
    pump(fake);
    assert!(fake.has_line("tempest: active, waiting role="));
    fake.advance(3000);
}

#[test]
fn a_non_tempest_role_is_not_watched() {
    let fake = Fake::new();
    let role = fake.role("PR_ICEBORNE");
    add_role(&fake, &fake.state, role);
    assert!(fake.state.tempest.borrow().is_empty());
}

#[test]
fn healthy_is_checked_before_the_catalog() {
    let fake = Fake::new();
    let (role, _) = tempest_role(&fake, (0, 0), (1, 1), (0, 0));
    settle(&fake, role);
    server_pump(&fake, &fake.state);
    assert!(fake.has_line("tempest: healthy role="));
    assert!(!fake.has_line("catalog empty"));
    assert!(fake.state.tempest.borrow().is_empty());
}

#[test]
fn pending_waits_and_the_repair_asks_only_for_what_is_missing() {
    let fake = Fake::new();
    let (role, group) = tempest_role(&fake, (1, 1), (1, 0), (0, 1));
    settle(&fake, role);
    server_pump(&fake, &fake.state);
    assert!(
        fake.calls_of("ApplyGameplayAbilitiesToOwner").is_empty(),
        "pending waits"
    );
    fake.put_i32(group + 0xC8, 0);
    pump(&fake);
    let calls = fake.calls_of("ApplyGameplayAbilitiesToOwner");
    assert_eq!(calls.len(), 1);
    assert!(
        fake.calls_of("ApplyGameplayBuffsToOwner").is_empty(),
        "the buffs are applied"
    );
    let header = fake.ptr(role + 0x2C8);
    assert_eq!(
        calls[0].params[..8],
        (header as u64).to_le_bytes(),
        "a copy of the array header"
    );
    assert_eq!(calls[0].params[8..12], 1i32.to_le_bytes());
    assert!(fake.has_line("tempest: repair=1 role="));
    assert!(fake.has_line("buffs=0/0 abilities=1/1"));
    // 3 500 ms between repairs, the fourth gives up
    pump(&fake);
    pump(&fake);
    pump(&fake);
    assert_eq!(fake.calls_of("ApplyGameplayAbilitiesToOwner").len(), 1);
    pump(&fake);
    assert_eq!(fake.calls_of("ApplyGameplayAbilitiesToOwner").len(), 2);
    for _ in 0..3 {
        fake.advance(3500);
        server_pump(&fake, &fake.state);
    }
    assert_eq!(fake.calls_of("ApplyGameplayAbilitiesToOwner").len(), 3);
    assert!(fake.has_line("tempest: gave up role="));
    assert!(fake.state.tempest.borrow().is_empty());
}

#[test]
fn modifier_counts_must_be_in_range() {
    let fake = Fake::new();
    let (role, group) = tempest_role(&fake, (1, 1), (0, 0), (0, 0));
    assert!(modifiers(&fake, role).is_some());
    fake.put_i32(group + 0x88, 257);
    assert!(modifiers(&fake, role).is_none());
    fake.put_i32(group + 0x88, -1);
    assert!(modifiers(&fake, role).is_none());
    fake.put_i32(group + 0x88, 256);
    assert!(modifiers(&fake, role).is_some());
    fake.put(role + 0x2F8, 0);
    assert_eq!(
        modifiers(&fake, role).map(|snap| snap.applied),
        Some([0, 0, 0]),
        "a null group"
    );
}

#[test]
fn the_array_guard() {
    let fake = Fake::new();
    let (role, _) = tempest_role(&fake, (1, 1), (0, 0), (0, 0));
    let array = role + 0x2A8;
    assert!(array_header(&fake, array).is_some());
    fake.put_i32(array + 8, 0);
    assert!(array_header(&fake, array).is_none(), "Num 0");
    fake.put_i32(array + 8, 257);
    fake.put_i32(array + 12, 300);
    assert!(array_header(&fake, array).is_none(), "Num > 256");
    fake.put_i32(array + 8, 2);
    fake.put_i32(array + 12, 1);
    assert!(array_header(&fake, array).is_none(), "Max < Num");
    fake.put_i32(array + 12, 2);
    fake.put(array, 0);
    assert!(array_header(&fake, array).is_none(), "null data");
}

/// A component and an ability block.
fn ability_call(fake: &Fake, pressed: bool, with_event_data: bool) -> (usize, usize) {
    let component = fake.block(0x400);
    let params = fake.block(0xC8);
    fake.put_u32(params, 7);
    fake.put_u8(params + 4, u8::from(pressed));
    fake.put_bytes(params + 8, &[0xAB; 16]);
    ability(fake, &fake.state, component, params, with_event_data);
    (component, params)
}

#[test]
fn pressed_and_activated() {
    let fake = Fake::new();
    let (component, params) = ability_call(&fake, true, false);
    assert_eq!(fake.call_names(), ["ServerSetInputPressed"]);
    let call = &fake.calls_of("ServerSetInputPressed")[0];
    assert_eq!((call.object, call.route), (component, Route::Hooked));
    assert_eq!(call.params, 7u32.to_le_bytes());
    let activates = fake.activates.borrow().clone();
    assert_eq!(
        activates,
        [(component, 7, [0xAB; 16], 0)],
        "no event data for the short form"
    );
    let mut key = [0u8; 16];
    assert!(fake.read(params + 8, &mut key));
    assert_eq!(key, [0xAB; 16], "the callee wrote to a copy");
    assert!(fake.has_line("ability: handle=7 inputPressed=1 activated=1 owner=null"));
}

#[test]
fn pressed_and_not_activated() {
    let fake = Fake::new();
    fake.activated.set(false);
    let (_, params) = ability_call(&fake, true, true);
    assert_eq!(fake.call_names(), ["ServerSetInputPressed", "ServerSetInputReleased"]);
    assert_eq!(
        fake.activates.borrow()[0].3,
        params + 0x18,
        "the long form's event data"
    );
}

#[test]
fn not_pressed() {
    let fake = Fake::new();
    fake.activated.set(false);
    ability_call(&fake, false, false);
    assert!(fake.calls.borrow().is_empty());
    assert_eq!(fake.activates.borrow().len(), 1);
}

#[test]
fn the_route_is_off_when_the_bytes_did_not_check() {
    let fake = Fake::new();
    fake.ability_ready.set(false);
    ability_call(&fake, true, false);
    assert!(fake.calls.borrow().is_empty() && fake.activates.borrow().is_empty());
}

#[test]
fn the_ability_line_names_the_owner_and_is_quiet() {
    let fake = Fake::new();
    let owner = fake.block(0x40);
    fake.names
        .borrow_mut()
        .insert(owner, "BP_PlayerCharacter_C_1".to_owned());
    let component = fake.block(0x400);
    fake.put(component + 0x3D0, owner);
    let params = fake.block(0x18);
    ability(&fake, &fake.state, component, params, false);
    assert!(fake.has_line("owner=BP_PlayerCharacter_C_1"));
    fake.take_lines();
    fake.quiet.set(true);
    ability(&fake, &fake.state, component, params, false);
    assert!(fake.take_lines().is_empty());
}

#[test]
fn the_pump_does_nothing_with_empty_tables() {
    let fake = Fake::new();
    fake.connections.set(Some(1));
    pump(&fake);
    assert!(fake.take_lines().is_empty());
    assert_eq!(fake.state.connections.get(), -1);
}
