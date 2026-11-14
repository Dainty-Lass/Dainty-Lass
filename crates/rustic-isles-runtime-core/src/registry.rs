//! The ownership registry (R0 7.5): every hook and feature id, its kind and its slice, and the slices landed in
//! this build. The AGPL runtime leaves alone whatever `owns` answers yes for. A slice's one edit here is adding its
//! name to `LANDED` (and, for a new id, its line).

/// The slices whose ids this build owns.
pub const LANDED: &[&str] = &[
    "R0", "R1", "R2", "R3", "R4", "R5", "R9", "R7", "R6", "R10", "R11", "R15", "R8", "R16", "R12",
];

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
    e("CL-5", Detour, "R4"),
    e("CL-7", Patch, "R4"),
    e("CL-8", Detour, "R4"),
    e("CL-9", Detour, "R4"),
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
    e("RG-3", Detour, "R14"),
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
    // R4's logging-only probes stay AGPL until R21 (R4 2)
    e("CL-3", Detour, "R19"),
    e("CL-4", Detour, "R19"),
    e("CL-6", Detour, "R19"),
    e("CL-10", Detour, "R19"),
    e("CL-11", Detour, "R19"),
    e("CL-12", Detour, "R19"),
    e("CL-13", Detour, "R19"),
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
    e("SP-2@R8", Feature, "R8"),
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

    const R4_PROBES: [&str; 7] = ["CL-3", "CL-4", "CL-6", "CL-10", "CL-11", "CL-12", "CL-13"];

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
        ids.extend(range("CL", 13).into_iter().map(|id| {
            let slice = if R4_PROBES.contains(&id.as_str()) { "R19" } else { "R4" };
            (id, slice)
        }));
        ids.extend(range("EXIT", 14).into_iter().map(|id| (id, "R3")));
        ids.extend(range("HL", 12).into_iter().map(|id| (id, "R5")));
        ids.extend(range("UI", 13).into_iter().map(|id| (id, "R6")));
        ids.extend(range("LD", 9).into_iter().map(|id| (id, "R7")));
        ids.extend(range("NETD", 8).into_iter().map(|id| (id, "R11")));
        ids.extend(range("RG", 7).into_iter().map(|id| {
            let slice = match id.as_str() {
                "RG-2" => "R13",
                "RG-3" => "R14",
                _ => "R12",
            };
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
            ("SP-2", &["R5", "R7", "R8", "R10", "R14", "R15", "R18", "R20"][..]),
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
            .filter(|id| (id.starts_with("SP-") || id.starts_with("EXIT-")) && !id.contains('@'))
            .count();
        assert_eq!(r0_r3, 17);
        // R2's chain and its feature id; HTTP-1@R10 stays the AGPL's until R10 lands
        assert!(owns(b"HTTP-1"));
        assert!(owns(b"HTTP-1@R2"));
        assert_eq!(owns(b"HTTP-1@R10"), LANDED.contains(&"R10"));
        // R4's six; its seven probes stay the AGPL's (R19)
        for id in ["CL-1", "CL-2", "CL-5", "CL-7", "CL-8", "CL-9"] {
            assert!(owns(id.as_bytes()), "{id}");
        }
        for id in R4_PROBES {
            assert!(!owns(id.as_bytes()), "{id}");
            assert_eq!(entry(id).map(|line| line.slice), Some("R19"));
        }
        if LANDED == ["R0", "R1", "R2", "R3", "R4"] {
            assert_eq!(owned_count(), 25);
        }
        assert_eq!(owns(b"SP-3@R9"), LANDED.contains(&"R9"));
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
    fn owns_r5() {
        for n in 1..=12 {
            assert!(owns(format!("HL-{n}").as_bytes()), "HL-{n}");
        }
        assert!(owns(b"SP-2@R5"));
        assert_eq!(owns(b"SP-2@R7"), LANDED.contains(&"R7"));
    }

    #[test]
    fn owns_r9() {
        // R9 lands alone (rulings R9): SRV-1 and its feature id, not R8's, R10's or R11's
        assert!(owns(b"SRV-1"));
        assert!(owns(b"SP-3@R9"));
        for id in ["SP-3@R8", "SP-3@R10", "SP-3@R11"] {
            let slice = entry(id).map(|line| line.slice).unwrap_or_default();
            assert_eq!(owns(id.as_bytes()), LANDED.contains(&slice), "{id}");
        }
        assert_eq!(entry("SRV-1").map(|line| line.kind), Some(Kind::Detour));
        assert_eq!(
            ENTRIES.iter().filter(|line| line.slice == "R9").count(),
            2,
            "R9 adds two"
        );
    }

    #[test]
    fn owns_r6() {
        for n in 1..=13 {
            let id = format!("UI-{n}");
            assert!(owns(id.as_bytes()), "{id}");
            assert_eq!(
                entry(&id).map(|line| (line.kind, line.slice)),
                Some((Kind::Detour, "R6"))
            );
        }
        assert!(!owns(b"UI-14"));
    }

    #[test]
    fn owns_r10() {
        // R10 lands with or after R9 and R2 (spec-runtime-r10 call 3)
        if LANDED.contains(&"R10") {
            assert!(LANDED.contains(&"R9") && LANDED.contains(&"R2"));
        }
        for id in ["SP-2@R10", "SP-3@R10", "HTTP-1@R10"] {
            assert_eq!(owns(id.as_bytes()), LANDED.contains(&"R10"), "{id}");
        }
        assert_eq!(ENTRIES.iter().filter(|line| line.slice == "R10").count(), 3);
    }

    #[test]
    fn r11_lands_with_or_after_r9() {
        // spec-runtime-r11 15.2: owning SP-3@R11 turns the AGPL's upkeep and manual tick off, and R11's own need R9
        if LANDED.contains(&"R11") {
            assert!(LANDED.contains(&"R9"), "R11 landed without R9");
            for n in 1..=8 {
                assert!(owns(format!("NETD-{n}").as_bytes()), "NETD-{n}");
            }
            assert!(owns(b"SP-3@R11"));
        }
        assert_eq!(ENTRIES.iter().filter(|line| line.slice == "R11").count(), 9);
    }

    #[test]
    fn owns_r15() {
        assert!(owns(b"SP-1@R15"));
        assert!(owns(b"SP-2@R15"));
    }

    #[test]
    fn r16_owns_travel() {
        assert!(owns(b"TR-1"));
        assert!(owns(b"SP-1@R16"));
    }

    #[test]
    fn r12_lands_with_or_after_r11() {
        // spec-runtime-r12 14.2: the drive reads R9's driver and runs before R11's manual tick flushes it
        if LANDED.contains(&"R12") {
            assert!(LANDED.contains(&"R11"), "R12 landed without R11");
            for id in ["RG-1", "RG-4", "RG-5", "RG-6", "RG-7", "SP-3@R12"] {
                assert!(owns(id.as_bytes()), "{id}");
            }
        }
        assert_eq!(
            entry("RG-3").map(|line| line.slice),
            Some("R14"),
            "RG-3 moved to R14 (12.2)"
        );
        assert_eq!(owns(b"RG-2"), LANDED.contains(&"R13"));
        assert_eq!(owns(b"RG-3"), LANDED.contains(&"R14"));
    }

    #[test]
    fn diagnostics_never_land() {
        assert!(!LANDED.contains(&"R19"));
    }
}
