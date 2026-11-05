// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Each part's rows are exactly core's table rows of its group, in the table's order.

use rustic_isles::{Group, Method, Service, routes};

fn rows(group: Group) -> Vec<(Method, &'static str)> {
    routes::all()
        .iter()
        .filter(|r| r.group == group && r.service == Service::Backend)
        .map(|r| (r.method, r.path))
        .collect()
}

#[test]
fn parts_match_the_table() {
    assert_eq!(
        rustic_isles_economy::inventory::routes().to_vec(),
        rows(Group::InventoryReforge)
    );
    assert_eq!(
        rustic_isles_economy::progression::routes().to_vec(),
        rows(Group::Progression)
    );
    assert_eq!(
        rustic_isles_economy::bounties::routes().to_vec(),
        rows(Group::BountiesStore)
    );
}
