//! The worker's run (spec §3.1): flags, the wait for the catalog, the exports in their fixed order, the manifest.

use std::path::PathBuf;
use std::time::Duration;

use crate::clock::Utc;
use crate::engine::Engine;
use crate::export::{self, Ctx, TextFile};
use crate::flags::{Flag, Flags};
use crate::status::Status;

pub const MANIFEST: &str = "export_manifest_1_14_7.json";
const WAIT_TRIES: u32 = 240;
const WAIT_STEP: Duration = Duration::from_millis(500);

/// Each export's return, as the manifest lists them (0 when off or not carried).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Results {
    pub catalog_general: i64,
    pub catalog_equipment: i64,
    pub live_weapon_slots: i64,
    pub progression: i64,
    pub combat: i64,
    pub skins: i64,
    pub slayers_path: i64,
    pub hunts: i64,
    pub table_inventory: i64,
    pub store_runtime: i64,
    pub mastery: i64,
}

/// What the run needs from its host: where the output is (resolved again at every use, spec §3.3), the debugger
/// sink, and how to wait.
pub struct Host<'h> {
    pub out: &'h dyn Fn() -> PathBuf,
    pub debug: fn(&str),
    pub sleep: &'h dyn Fn(Duration),
    pub now: &'h dyn Fn() -> Utc,
}

enum Step {
    /// Carried: runs when its flag is on.
    Export(fn(&Ctx, &mut Results)),
    /// Not in phase 1 (spec §9): refused when its flag is on.
    Refused,
}

/// The exports in their fixed order (spec §3.1 step 5), with each one's skip text (spec §6.17).
const ORDER: [(Flag, &str, Step); 13] = [
    (Flag::Catalog, "catalog export", Step::Export(catalog)),
    (Flag::WeaponSlots, "live weapon-slot export", Step::Refused),
    (Flag::Cells, "live cell-slot export", Step::Refused),
    (
        Flag::Progression,
        "progression export",
        Step::Export(|c, r| r.progression = export::progression::export(c)),
    ),
    (Flag::Combat, "combat export", Step::Refused),
    (Flag::Skins, "weapon skins export", Step::Refused),
    (
        Flag::SlayersPath,
        "Slayer's Path node export",
        Step::Export(|c, r| r.slayers_path = export::slayers::export(c)),
    ),
    (
        Flag::Hunts,
        "live hunt table export",
        Step::Export(|c, r| r.hunts = export::hunts::export(c)),
    ),
    (Flag::DropTables, "drop-table export", Step::Refused),
    (Flag::CurrencyNames, "currency name resolution", Step::Refused),
    (
        Flag::StoreRuntime,
        "store runtime capture",
        Step::Export(|c, r| r.store_runtime = export::store::export(c)),
    ),
    (
        Flag::Mastery,
        "mastery/achievement objective export",
        Step::Export(|c, r| r.mastery = export::mastery::export(c)),
    ),
    (Flag::TableInventory, "DataTable census", Step::Refused),
];

fn catalog(ctx: &Ctx, results: &mut Results) {
    results.catalog_general = export::catalog::catalog(ctx);
    results.catalog_equipment = export::catalog::equipment(ctx);
    ctx.status.line(
        "",
        &format!(
            "catalog DONE general={} equipment={}",
            results.catalog_general, results.catalog_equipment
        ),
    );
}

/// The whole run; `None` when the catalog never appeared.
pub fn run(engine: &Engine, flags: &Flags, host: &Host) -> Option<Results> {
    let status = Status::new(&(host.out)(), host.debug);
    status.line("", &flags.status_line());
    for flag in [Flag::SkinsResolveStringTable, Flag::CurrencyNames] {
        if flags.get(flag) {
            status.line(
                "",
                &format!(
                    "WARNING: {}=1 resolves names through string-table calls off the game thread; this build does \
                     not carry it",
                    flag.key()
                ),
            );
        }
    }
    status.line("", "waiting for GObjects + catalog...");
    if !wait_for_catalog(engine, host) {
        status.line(
            "",
            "TIMEOUT: catalog never appeared. Reach the main menu/Ramsgate, then re-inject.",
        );
        return None;
    }

    let mut results = Results::default();
    for (flag, what, step) in &ORDER {
        let ctx = Ctx {
            engine,
            out: (host.out)(),
            status: &Status::new(&(host.out)(), host.debug),
        };
        match (flags.get(*flag), step) {
            (false, _) => ctx.status.line("", &format!("{}=0, skipping {what}", flag.key())),
            (true, Step::Refused) => {
                ctx.status.line(
                    "",
                    &format!("{}=1 is not supported by this build, skipping", flag.key()),
                );
            }
            (true, Step::Export(export)) => export(&ctx, &mut results),
        }
    }

    let out = (host.out)();
    if let Some(mut file) = TextFile::create(&out.join(MANIFEST)) {
        file.text(&manifest(flags, &results, &(host.now)()));
        file.finish();
    }
    status.line(
        "",
        &format!(
            "ALL DONE catalog={} equipment={} progression={} slayersPath={} hunts={} storeRuntime={} mastery={}",
            results.catalog_general,
            results.catalog_equipment,
            results.progression,
            results.slayers_path,
            results.hunts,
            results.store_runtime,
            results.mastery
        ),
    );
    Some(results)
}

/// Up to 240 tries, 500 ms apart: objects exist and a live catalog is among them.
fn wait_for_catalog(engine: &Engine, host: &Host) -> bool {
    for attempt in 0..WAIT_TRIES {
        if engine.object_count() > 0 && export::catalog::live_catalog(engine) != 0 {
            return true;
        }
        engine.take_fault();
        if attempt + 1 < WAIT_TRIES {
            (host.sleep)(WAIT_STEP);
        }
    }
    false
}

/// `export_manifest_1_14_7.json` (spec §6.15): two-space indent; the caller writes it CRLF.
pub fn manifest(flags: &Flags, results: &Results, now: &Utc) -> String {
    let listed = [
        Flag::Catalog,
        Flag::Progression,
        Flag::Combat,
        Flag::Skins,
        Flag::SkinsResolveStringTable,
        Flag::SlayersPath,
        Flag::WeaponSlots,
        Flag::Hunts,
        Flag::TableInventory,
        Flag::StoreRuntime,
        Flag::Mastery,
    ];
    let flag_lines: Vec<String> = listed
        .iter()
        .map(|flag| format!("    \"{}\": {}", flag.key(), flags.get(*flag)))
        .collect();
    let result_lines: Vec<String> = [
        ("catalogGeneralItems", results.catalog_general),
        ("catalogEquipmentItems", results.catalog_equipment),
        ("liveWeaponSlotItems", results.live_weapon_slots),
        ("progressionRows", results.progression),
        ("combatRows", results.combat),
        ("skinsRows", results.skins),
        ("slayersPathNodes", results.slayers_path),
        ("huntRows", results.hunts),
        ("tableInventoryRows", results.table_inventory),
        ("storeRuntimeRows", results.store_runtime),
        ("masteryRows", results.mastery),
    ]
    .iter()
    .map(|(key, value)| format!("    \"{key}\": {value}"))
    .collect();
    format!(
        "{{\n  \"gameVersion\": \"1.14.7\",\n  \"changelist\": 647472,\n  \"engineVersion\": \"4.26.2\",\n  \
         \"exportedAt\": \"{}\",\n  \"flags\": {{\n{}\n  }},\n  \"results\": {{\n{}\n  }}\n}}\n",
        now.timestamp(),
        flag_lines.join(",\n"),
        result_lines.join(",\n")
    )
}
