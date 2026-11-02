//! The launcher's region ping (dainty-lass `launcher-kit`) and the gateway answer the same routes and check the
//! same region ids (DN7); until DN2 gives both one definition, they are compared here.

use launcher_kit::net::regionping::{self, parse};

#[test]
fn the_gateway_serves_the_launchers_routes() {
    assert_eq!(gateway::network::NODES_PATH, regionping::NODES_PATH);
    assert_eq!(gateway::network::PING_PATH, regionping::PING_PATH);
    assert_eq!(gateway::network::REGIONS_PATH, regionping::REPORT_PATH);
    assert_eq!(gateway::network::MAX_NODES, parse::MAX_NODES);
}

#[test]
fn both_sides_accept_the_same_region_ids() {
    let long = "a".repeat(65);
    for id in [
        "uk", "at", "oregon", "us-east", "eu_west", "", "UK", "a b", "é", "-x", "x-", &long, "1", "a.b",
    ] {
        assert_eq!(gateway::network::is_region_id(id), parse::is_region_id(id), "{id:?}");
    }
}
