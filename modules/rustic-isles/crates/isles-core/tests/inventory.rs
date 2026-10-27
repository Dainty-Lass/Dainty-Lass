//! The route skeleton against RU3a's inventory and RU3c's diff: every route in Express's match order, the 1.12 set
//! as RU3c splits it (RU4's verify).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::BTreeSet;
use std::path::PathBuf;

use isles_core::routes::{RouteSet, Scope, Service, Side, Versions, When, all};
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
            field(entry, "path"),
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
            Side::Rifts if isles_guard(def.path) => {
                assert_ne!(def.when, When::Rifts, "{}", def.name());
                assert_eq!(def.versions, Versions::Both, "{}", def.name());
            }
            Side::Rifts => {
                assert_eq!(def.when, When::Rifts, "{}", def.name());
                assert_eq!(def.versions, Versions::Only1147, "{}", def.name());
            }
        }
    }
}

#[test]
fn the_112_set_is_ru3cs_split() {
    let diff = inventory("diff-1.12.0-1.14.7.json");
    let only: BTreeSet<(String, String)> = diff
        .iter()
        .filter(|e| field(e, "class") == "1.14.7-only")
        .map(|e| (field(e, "method").to_owned(), field(e, "path").to_owned()))
        .collect();
    assert_eq!(only.len(), 34);
    for def in all().iter().filter(|d| d.side == Side::Isles) {
        let key = (def.method.as_str().to_owned(), def.path.to_owned());
        // The Reforged API is the launcher's, not a game version's: RU3c left it out of the diff (seam 7)
        let version_only = only.contains(&key) && def.scope() == Scope::Version;
        assert_eq!(
            RouteSet::V1120.serves(def),
            !version_only,
            "{} (1.14.7-only in RU3c: {})",
            def.name(),
            only.contains(&key)
        );
    }
    // Everything 1.12.0's Node services register is in the 1.12 set (RU3b), by method and path
    let set: BTreeSet<(String, String)> = all()
        .iter()
        .filter(|d| RouteSet::V1120.serves(d))
        .map(|d| (d.method.as_str().to_owned(), d.path.to_owned()))
        .collect();
    for entry in inventory("routes-1.12.0.json") {
        let key = (field(&entry, "method").to_owned(), field(&entry, "path").to_owned());
        assert!(
            set.contains(&key),
            "1.12.0's {} {} is not served by the 1.12 module",
            key.0,
            key.1
        );
    }
    let backend_1120 = RouteSet::V1120.routes(Service::Backend).count();
    let backend_1147 = RouteSet::V1147.routes(Service::Backend).count();
    assert_eq!((backend_1147, backend_1120), (285, 285 - 25 - 58));
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
