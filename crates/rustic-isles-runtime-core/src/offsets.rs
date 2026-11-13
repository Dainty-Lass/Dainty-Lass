//! Facts about Dauntless-Win64-Shipping.exe 1.14.7 (CL 647472), written by hand (RT-2). Not a generated SDK: RVAs,
//! offsets, sizes, prologue bytes and UE names only. Each slice adds its own lines, naming itself.

/// One hook target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The surface id that hooks it.
    pub id: &'static str,
    /// The RVA from the image base (for `Export` and `UFunction`, 0).
    pub rva: u32,
    /// How the runtime finds it.
    pub found_by: FoundBy,
    /// What it is.
    pub what: &'static str,
}

/// How a target is found (surface section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundBy {
    /// A fixed RVA.
    Rva,
    /// An RVA that must start a `.pdata` function with these bytes, else no hook.
    RvaAndBytes(&'static [u8]),
    /// The function holding this RVA, from `.pdata`.
    PdataOf(u32),
    /// `GetProcAddress(module, procedure)`.
    Export(&'static str, &'static str),
    /// A UFunction by class and name at run time.
    UFunction(&'static str, &'static str),
}

/// Every hook target this build knows.
pub const TARGETS: &[Target] = &[
    // R0: the dispatch chains (one target, two roles for SP-1/SP-2)
    Target {
        id: "SP-1",
        rva: 0x0286_1880,
        found_by: FoundBy::Rva,
        what: "UObject::ProcessEvent (client)",
    },
    Target {
        id: "SP-2",
        rva: 0x0286_1880,
        found_by: FoundBy::Rva,
        what: "UObject::ProcessEvent (server)",
    },
    Target {
        id: "SP-3",
        rva: 0x03E1_0AB0,
        found_by: FoundBy::Rva,
        what: "UEngine::Tick (base UGameEngine tick)",
    },
];

/// The target line for `id`.
pub fn target(id: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|target| target.id == id)
}

/// Engine layout offsets (UE 4.26 as built into 1.14.7).
pub mod layout {
    /// `UObject` object flags (`u32`).
    pub const UOBJECT_FLAGS: usize = 0x08;
    /// `UObject` index into the object array (`i32`).
    pub const UOBJECT_INDEX: usize = 0x0C;
    /// `UObject` class pointer.
    pub const UOBJECT_CLASS: usize = 0x10;
    /// `UObject` name: comparison index `i32` here, number `u32` at +4.
    pub const UOBJECT_NAME: usize = 0x18;
    /// `UObject` outer pointer.
    pub const UOBJECT_OUTER: usize = 0x20;
    /// Bytes `is_live_object` checks readable.
    pub const UOBJECT_SIZE: usize = 0x28;
    /// `RF_BeginDestroyed`.
    pub const RF_BEGIN_DESTROYED: u32 = 0x8000;
    /// `RF_FinishDestroyed`.
    pub const RF_FINISH_DESTROYED: u32 = 0x1_0000;
    /// The actor flag byte holding "being destroyed".
    pub const ACTOR_FLAGS_BYTE: usize = 0x5C;
    /// "Being destroyed" in that byte.
    pub const ACTOR_BEING_DESTROYED: u8 = 0x08;
    /// Bytes `is_actor_alive_for_replication` checks readable.
    pub const ACTOR_SIZE: usize = 0x60;

    /// `FUObjectArray`'s chunk table pointer.
    pub const OBJECTS_CHUNKS: usize = 0x00;
    /// Max elements (`i32`).
    pub const OBJECTS_MAX: usize = 0x10;
    /// Element count (`i32`).
    pub const OBJECTS_COUNT: usize = 0x14;
    /// Max chunks (`i32`).
    pub const OBJECTS_MAX_CHUNKS: usize = 0x18;
    /// Chunk count (`i32`).
    pub const OBJECTS_CHUNK_COUNT: usize = 0x1C;
    /// Items per chunk.
    pub const OBJECTS_PER_CHUNK: usize = 65_536;
    /// One `FUObjectItem`; the object pointer is at +0.
    pub const OBJECT_ITEM_SIZE: usize = 0x18;
    /// The exporter's engine-model bound.
    pub const MAX_OBJECTS: i32 = 4_000_000;

    /// `FNamePool`'s block table.
    pub const NAME_POOL_BLOCKS: usize = 0x10;
    /// Entry stride.
    pub const NAME_STRIDE: usize = 2;
    /// Offset bits of a comparison index.
    pub const NAME_OFFSET_BITS: u32 = 16;
    /// Blocks in the pool.
    pub const NAME_BLOCKS: u32 = 8192;
}

/// Data RVAs.
pub mod data {
    /// `GWorld`, a `UWorld*` (R9's world wait).
    pub const GWORLD: u32 = 0x06FC_7F58;
    /// `GObjects` (`FUObjectArray`).
    pub const GOBJECTS: u32 = 0x06E5_9ED0;
    /// `GNames` (`FNamePool`).
    pub const GNAMES: u32 = 0x06E3_F280;
    /// `FName::AppendString`, the names fallback (R0 6.2; not called by R0).
    pub const FNAME_APPEND_STRING: u32 = 0x0265_6410;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{self, Kind};
    use std::collections::HashMap;

    #[test]
    fn table_rules() {
        let mut by_rva: HashMap<u32, Vec<&str>> = HashMap::new();
        for target in TARGETS {
            let entry = registry::entry(target.id).unwrap_or_else(|| panic!("{} is not a registry id", target.id));
            assert!(
                matches!(entry.kind, Kind::Chain | Kind::Detour),
                "{} is not a detour",
                target.id
            );
            assert_eq!(
                TARGETS.iter().filter(|other| other.id == target.id).count(),
                1,
                "{} twice",
                target.id
            );
            if let FoundBy::RvaAndBytes(prologue) = target.found_by {
                assert!(!prologue.is_empty(), "{} has an empty prologue", target.id);
            }
            if matches!(
                target.found_by,
                FoundBy::Rva | FoundBy::RvaAndBytes(_) | FoundBy::PdataOf(_)
            ) {
                by_rva.entry(target.rva).or_default().push(target.id);
            }
        }
        for (rva, ids) in by_rva {
            assert!(
                ids.len() == 1 || (ids == ["SP-1", "SP-2"]),
                "{rva:#x} shared by {ids:?}"
            );
        }
        // Every detour of a landed slice has its line (RT-2; a later slice adds lines as it lands)
        for entry in registry::ENTRIES {
            if matches!(entry.kind, Kind::Chain | Kind::Detour) && registry::LANDED.contains(&entry.slice) {
                assert!(target(entry.id).is_some(), "{} has no target line", entry.id);
            }
        }
    }
}
