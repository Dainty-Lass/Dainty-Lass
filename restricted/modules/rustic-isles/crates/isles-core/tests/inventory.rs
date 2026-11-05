//! The route skeleton against RU3a's inventory: every route in Express's match order (RU4's verify).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::path::PathBuf;

use isles_core::routes::{RouteSet, Service, Side, When, all};
use serde_json::Value;

fn inventory(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/inventory")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str::<Value>(&text)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn field<'a>(entry: &'a Value, name: &str) -> &'a str {
    entry[name].as_str().unwrap()
}

#[test]
fn the_table_is_ru3as_inventory_in_its_order() {
    let routes = inventory("routes-1.14.7.json");
    let table = all();
    assert_eq!(table.len(), routes.len());
    for (def, entry) in table.iter().zip(&routes) {
        let want = format!(
            "{} {} {} {} {}",
            field(entry, "service"),
            field(entry, "method"),
            // The inventory is Node's; D3 (spec-rd27-rifts) renamed Guard's and admission's prefix
            field(entry, "path").replace("/mystic/v1/", "/rustic/v1/"),
            field(entry, "group"),
            field(entry, "side")
        );
        let have = format!(
            "{} {} {} {} {}",
            def.service.as_str(),
            def.method.as_str(),
            def.path,
            def.group.as_str(),
            def.side.as_str()
        );
        assert_eq!(have, want);
    }
}

/// Launcher Guard and the Guard manifests: RU3a's rifts rows that isles keeps (RF1, decision 1).
fn isles_guard(path: &str) -> bool {
    path.contains("/guard/sessions")
        || path.contains("/guard/heartbeat")
        || path.contains("guard-manifests")
        || path.starts_with("/admin/v1/guard/")
}

#[test]
fn rifts_rows_are_rifts_but_launcher_guard() {
    for def in all() {
        match def.side {
            Side::Isles => assert_ne!(def.when, When::Rifts, "{}", def.name()),
            Side::Rifts if isles_guard(def.path) => assert_ne!(def.when, When::Rifts, "{}", def.name()),
            Side::Rifts => assert_eq!(def.when, When::Rifts, "{}", def.name()),
        }
    }
}

#[test]
fn the_backend_serves_every_backend_route() {
    assert_eq!(RouteSet::V1147.routes(Service::Backend).count(), 285);
}

#[test]
fn mount_conditions_follow_appts() {
    let admin: Vec<_> = all().iter().filter(|d| d.when == When::Admin).collect();
    // AdminApi.enabled: the admin routers and the release-gates preflight
    assert_eq!(admin.len(), 29);
    assert!(
        admin
            .iter()
            .all(|d| d.path.starts_with("/admin/") || d.path == "/health/release-gates")
    );
    let gauntlet: Vec<_> = all()
        .iter()
        .filter(|d| d.when == When::Gauntlet)
        .map(|d| d.path)
        .collect();
    assert_eq!(gauntlet.len(), 5);
    assert!(gauntlet.contains(&"/config"));
}
