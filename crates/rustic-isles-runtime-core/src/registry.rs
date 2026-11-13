//! The ownership registry (R0 7.5): every hook and feature id, its kind and its slice, and the slices landed in
//! this build. The AGPL runtime leaves alone whatever `owns` answers yes for. A slice's one edit here is adding its
//! name to `LANDED` (and, for a new id, its line).

/// The slices whose ids this build owns.
pub const LANDED: &[&str] = &["R0", "R1", "R2", "R3"];

/// The longest id `owns` accepts, in bytes.
pub const MAX_ID_BYTES: usize = 47;

/// What an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A dispatch point the broker chains (the AGPL always chains its wrapper on it).
    Chain,
    /// A MinHook detour (on an RVA or an export).
    Detour,
    /// A code or data write.
    Patch,
    /// A vectored exception handler.
    Veh,
    /// A UFunction native-pointer swap.
    Swap,
    /// A process setting with no hook.
    Setting,
    /// One dispatch point × one slice: every AGPL block of that slice there.
    Feature,
}

/// One registry line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The surface id (`SP-1`, `HL-12`, `SP-3@R9`).
    pub id: &'static str,
    /// Its kind.
    pub kind: Kind,
    /// The slice that ports it.
    pub slice: &'static str,
}

const fn e(id: &'static str, kind: Kind, slice: &'static str) -> Entry {
    Entry { id, kind, slice }
}

use Kind::{Chain, Detour, Feature, Patch, Setting, Swap, Veh};

/// Every id (surface section 3, R0 7.5).
pub const ENTRIES: &[Entry] = &[
    // Dispatch points
    e("SP-1", Chain, "R0"),
    e("SP-2", Chain, "R0"),
    e("SP-3", Chain, "R0"),
    e("HTTP-1", Chain, "R2"),
    // Login, EAC and the command line
    e("CL-1", Detour, "R4"),
    e("CL-2", Detour, "R4"),
    e("CL-3", Detour, "R4"),
    e("CL-4", Detour, "R4"),
    e("CL-5", Detour, "R4"),
    e("CL-6", Detour, "R4"),
    e("CL-7", Patch, "R4"),
    e("CL-8", Detour, "R4"),
    e("CL-9", Detour, "R4"),
    e("CL-10", Detour, "R4"),
    e("CL-11", Detour, "R4"),
    e("CL-12", Detour, "R4"),
    e("CL-13", Detour, "R4"),
    // Exit and crash
    e("EXIT-1", Detour, "R3"),
    e("EXIT-2", Detour, "R3"),
    e("EXIT-3", Detour, "R3"),
    e("EXIT-4", Detour, "R3"),
    e("EXIT-5", Detour, "R3"),
    e("EXIT-6", Detour, "R3"),
    e("EXIT-7", Detour, "R3"),
    e("EXIT-8", Detour, "R3"),
    e("EXIT-9", Detour, "R3"),
    e("EXIT-10", Detour, "R3"),
    e("EXIT-11", Detour, "R3"),
    e("EXIT-12", Veh, "R3"),
    e("EXIT-13", Detour, "R3"),
    e("EXIT-14", Patch, "R3"),
    // Headless server boot
    e("HL-1", Patch, "R5"),
    e("HL-2", Patch, "R5"),
    e("HL-3", Detour, "R5"),
    e("HL-4", Detour, "R5"),
    e("HL-5", Detour, "R5"),
    e("HL-6", Detour, "R5"),
    e("HL-7", Detour, "R5"),
    e("HL-8", Detour, "R5"),
    e("HL-9", Patch, "R5"),
    e("HL-10", Detour, "R5"),
    e("HL-11", Swap, "R5"),
    e("HL-12", Setting, "R5"),
    // Widget crash guards
    e("UI-1", Detour, "R6"),
    e("UI-2", Detour, "R6"),
    e("UI-3", Detour, "R6"),
    e("UI-4", Detour, "R6"),
    e("UI-5", Detour, "R6"),
    e("UI-6", Detour, "R6"),
    e("UI-7", Detour, "R6"),
    e("UI-8", Detour, "R6"),
    e("UI-9", Detour, "R6"),
    e("UI-10", Detour, "R6"),
    e("UI-11", Detour, "R6"),
    e("UI-12", Detour, "R6"),
    e("UI-13", Detour, "R6"),
    // Player data and loading
    e("LD-1", Detour, "R7"),
    e("LD-2", Detour, "R7"),
    e("LD-3", Detour, "R7"),
    e("LD-4", Detour, "R7"),
    e("LD-5", Detour, "R7"),
    e("LD-6", Detour, "R7"),
    e("LD-7", Detour, "R7"),
    e("LD-8", Detour, "R7"),
    e("LD-9", Detour, "R7"),
    // Net driver, replication graph and relevance
    e("NETD-1", Detour, "R11"),
    e("NETD-2", Detour, "R11"),
    e("NETD-3", Detour, "R11"),
    e("NETD-4", Detour, "R11"),
    e("NETD-5", Detour, "R11"),
    e("NETD-6", Detour, "R11"),
    e("NETD-7", Detour, "R11"),
    e("NETD-8", Detour, "R11"),
    e("RG-1", Detour, "R12"),
    e("RG-2", Detour, "R13"),
    e("RG-3", Detour, "R12"),
    e("RG-4", Detour, "R12"),
    e("RG-5", Detour, "R12"),
    e("RG-6", Detour, "R12"),
    e("RG-7", Detour, "R12"),
    // Game features
    e("PR-1", Detour, "R14"),
    e("ESC-1", Detour, "R18"),
    e("SRV-1", Detour, "R9"),
    e("TR-1", Detour, "R16"),
    // Diagnostics: R19 is not ported (RT-3), so these are never owned
    e("DX-1", Detour, "R19"),
    e("DX-2", Detour, "R19"),
    e("DX-3", Detour, "R19"),
    e("DX-4", Detour, "R19"),
    e("DX-5", Detour, "R19"),
    e("DX-6", Detour, "R19"),
    e("DX-7", Detour, "R19"),
    e("DX-8", Detour, "R19"),
    e("DX-9", Detour, "R19"),
    // Feature ids: one dispatch point × one slice
    e("SP-1@R14", Feature, "R14"),
    e("SP-1@R15", Feature, "R15"),
    e("SP-1@R16", Feature, "R16"),
    e("SP-1@R17", Feature, "R17"),
    e("SP-2@R5", Feature, "R5"),
    e("SP-2@R7", Feature, "R7"),
    e("SP-2@R10", Feature, "R10"),
    e("SP-2@R14", Feature, "R14"),
    e("SP-2@R15", Feature, "R15"),
    e("SP-2@R18", Feature, "R18"),
    e("SP-2@R20", Feature, "R20"),
    e("SP-3@R7", Feature, "R7"),
    e("SP-3@R8", Feature, "R8"),
    e("SP-3@R9", Feature, "R9"),
    e("SP-3@R10", Feature, "R10"),
    e("SP-3@R11", Feature, "R11"),
    e("SP-3@R12", Feature, "R12"),
    e("SP-3@R13", Feature, "R13"),
    e("SP-3@R14", Feature, "R14"),
    e("SP-3@R18", Feature, "R18"),
    e("SP-3@R20", Feature, "R20"),
    e("HTTP-1@R2", Feature, "R2"),
    e("HTTP-1@R10", Feature, "R10"),
];

/// The line for `id`.
pub fn entry(id: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.id == id)
}

/// `owns(id)`: the id is in the registry and its slice has landed. Bytes, as `rir_owns` receives them; empty,
/// unknown or over 47 bytes is no.
pub fn owns(id: &[u8]) -> bool {
    if id.is_empty() || id.len() > MAX_ID_BYTES {
        return false;
    }
    ENTRIES
        .iter()
        .any(|entry| entry.id.as_bytes() == id && LANDED.contains(&entry.slice))
}

/// How many ids this build owns (the status line's `owns=`).
pub fn owned_count() -> usize {
    ENTRIES.iter().filter(|entry| LANDED.contains(&entry.slice)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn range(prefix: &str, last: u32) -> Vec<String> {
        (1..=last).map(|n| format!("{prefix}-{n}")).collect()
    }

    /// Surface section 3, written out again independently of the table above.
    fn surface_ids() -> Vec<(String, &'static str)> {
        let mut ids: Vec<(String, &'static str)> = Vec::new();
        for id in ["SP-1", "SP-2", "SP-3"] {
            ids.push((id.into(), "R0"));
        }
        ids.push(("HTTP-1".into(), "R2"));
        ids.extend(range("CL", 13).into_iter().map(|id| (id, "R4")));
        ids.extend(range("EXIT", 14).into_iter().map(|id| (id, "R3")));
        ids.extend(range("HL", 12).into_iter().map(|id| (id, "R5")));
        ids.extend(range("UI", 13).into_iter().map(|id| (id, "R6")));
        ids.extend(range("LD", 9).into_iter().map(|id| (id, "R7")));
        ids.extend(range("NETD", 8).into_iter().map(|id| (id, "R11")));
        ids.extend(range("RG", 7).into_iter().map(|id| {
            let slice = if id == "RG-2" { "R13" } else { "R12" };
            (id, slice)
        }));
        ids.push(("PR-1".into(), "R14"));
        ids.push(("ESC-1".into(), "R18"));
        ids.push(("SRV-1".into(), "R9"));
        ids.push(("TR-1".into(), "R16"));
        ids.extend(range("DX", 9).into_iter().map(|id| (id, "R19")));
        ids
    }

    #[test]
    fn every_surface_id_once_with_its_slice() {
        let expected = surface_ids();
        assert_eq!(expected.len(), 93);
        for (id, slice) in &expected {
            let lines: Vec<_> = ENTRIES.iter().filter(|entry| entry.id == id).collect();
            assert_eq!(lines.len(), 1, "{id}");
            assert_eq!(lines[0].slice, *slice, "{id}");
        }
        let hooks = ENTRIES.iter().filter(|entry| entry.kind != Kind::Feature).count();
        assert_eq!(hooks, expected.len(), "no id outside surface section 3");
    }

    #[test]
    fn every_feature_id() {
        let features = [
            ("SP-1", &["R14", "R15", "R16", "R17"][..]),
            ("SP-2", &["R5", "R7", "R10", "R14", "R15", "R18", "R20"][..]),
            (
                "SP-3",
                &["R7", "R8", "R9", "R10", "R11", "R12", "R13", "R14", "R18", "R20"][..],
            ),
            ("HTTP-1", &["R2", "R10"][..]),
        ];
        let mut expected = HashSet::new();
        for (point, slices) in features {
            for slice in slices {
                let id = format!("{point}@{slice}");
                let line = entry(&id).unwrap_or_else(|| panic!("{id} missing"));
                assert_eq!((line.kind, line.slice), (Kind::Feature, *slice));
                expected.insert(id);
            }
        }
        let listed: HashSet<String> = ENTRIES
            .iter()
            .filter(|entry| entry.kind == Kind::Feature)
            .map(|entry| entry.id.to_owned())
            .collect();
        assert_eq!(listed, expected);
        let unique: HashSet<&str> = ENTRIES.iter().map(|entry| entry.id).collect();
        assert_eq!(unique.len(), ENTRIES.len());
    }

    #[test]
    fn owns_for_landed_slices() {
        let owned: Vec<&str> = ENTRIES
            .iter()
            .filter(|entry| owns(entry.id.as_bytes()))
            .map(|entry| entry.id)
            .collect();
        let expected: Vec<&str> = ENTRIES
            .iter()
            .filter(|entry| LANDED.contains(&entry.slice))
            .map(|entry| entry.id)
            .collect();
        assert_eq!(owned, expected);
        assert_eq!(owned_count(), owned.len());
        // R0's chains and R3's fourteen (spec-runtime-r3 10.1: 17 with R0, R1, R3)
        for id in ["SP-1", "SP-2", "SP-3"] {
            assert!(owns(id.as_bytes()), "{id}");
        }
        for n in 1..=14 {
            assert!(owns(format!("EXIT-{n}").as_bytes()), "EXIT-{n}");
        }
        assert!(!owns(b"EXIT-15"));
        let r0_r3 = owned
            .iter()
            .filter(|id| id.starts_with("SP-") || id.starts_with("EXIT-"))
            .count();
        assert_eq!(r0_r3, 17);
        // R2's chain and its feature id; HTTP-1@R10 stays the AGPL's until R10 lands
        assert!(owns(b"HTTP-1"));
        assert!(owns(b"HTTP-1@R2"));
        assert_eq!(owns(b"HTTP-1@R10"), LANDED.contains(&"R10"));
        if LANDED == ["R0", "R1", "R2", "R3"] {
            assert_eq!(owned_count(), 19);
        }
        assert!(!owns(b"SP-3@R9"));
        assert!(!owns(b"DX-8"));
        assert!(!owns(b"NOPE-1"));
        assert!(!owns(b""));
        assert!(!owns(b"sp-1"));
        let long = [b'S'; 48];
        assert!(!owns(&long));
        let mut padded = b"SP-1".to_vec();
        padded.resize(48, b' ');
        assert!(!owns(&padded));
    }

    #[test]
    fn diagnostics_never_land() {
        assert!(!LANDED.contains(&"R19"));
    }
}
