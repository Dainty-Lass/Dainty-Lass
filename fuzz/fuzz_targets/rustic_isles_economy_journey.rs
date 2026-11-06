//! What Rustic Isles' Slayer's Path reads (spec-rustic-isles-economy-progression §14): arbitrary stored and incoming
//! node maps, versions and a definitions graph through the merge, the mask and a host's plan. Nothing panics, and a
//! node unlocked in the stored map is never locked by a host's save.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_economy::progression::journey::{
    Graph, as_map, is_unlocked, mask_nodes, merge_nodes, plan_host_journey, restore_masked,
};
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(3, '\u{1}');
    let parse = |t: Option<&str>| {
        t.and_then(|t| serde_json::from_str::<Value>(t).ok())
            .unwrap_or(Value::Null)
    };
    let (stored, incoming, defs) = (parse(parts.next()), parse(parts.next()), parse(parts.next()));
    let (stored, incoming) = (as_map(&stored), as_map(&incoming));
    let version = (data.first().copied().unwrap_or(0) % 4) as f64;
    let (mut merged, _, _) = merge_nodes(&stored, 2.0, &incoming, version);
    let masked = vec!["Gauntlet_Unlock".to_owned()];
    restore_masked(&mut merged, &stored, &masked);
    let _ = mask_nodes(&merged, &masked);
    let graph = Graph::build(defs.as_object(), &[]);
    let plan = plan_host_journey(&stored, &merged, &graph);
    for (key, node) in &stored {
        if is_unlocked(node) {
            assert!(plan.nodes.get(key).is_some_and(is_unlocked), "{key}");
        }
    }
});
