//! Server stubs (spec-runtime-stub): which particle components the effects sweep stops, its schedule, the function
//! layouts it checks and its log texts. The DLL walks the object array and makes the calls.

use crate::engine::Engine;
use crate::memory::Memory;
use crate::offsets::{layout, sim};
use crate::reflect;
use std::collections::HashMap;

/// The flag file that turns the effects sweep on (call 1).
pub const EFFECTS_FLAG: &str = "RUNTIME_STUB_EFFECTS";
/// The first sweep, after the start step.
pub const FIRST_MS: u64 = 5_000;
/// Between sweeps.
pub const EVERY_MS: u64 = 10_000;
/// `RF_ClassDefaultObject | RF_ArchetypeObject`: templates, never stopped.
pub const TEMPLATE_FLAGS: u32 = 0x10 | 0x20;
/// The longest outer chain walked.
pub const MAX_OUTERS: usize = 64;

/// The start line.
pub const ON_LINE: &str = "[MitRuntime] [Stub] effects on (RUNTIME_STUB_EFFECTS.flag): particle components in a level are \
                           deactivated and stop ticking, 5 s after listen and every 10 s";

/// Whether a sweep is due at `now`, given the start and the last sweep (2.2).
pub fn due(start: u64, last: Option<u64>, now: u64) -> bool {
    match last {
        None => now.saturating_sub(start) >= FIRST_MS,
        Some(last) => now.saturating_sub(last) >= EVERY_MS,
    }
}

/// Whether `outer`'s chain holds an object whose class `is_level` says yes (2.1); every link is memoized.
pub fn in_level(
    memory: &dyn Memory,
    outer: usize,
    is_level: &mut dyn FnMut(usize) -> bool,
    memo: &mut HashMap<usize, bool>,
) -> bool {
    let mut path = Vec::new();
    let mut current = outer;
    let answer = loop {
        if current == 0 || path.len() >= MAX_OUTERS || path.contains(&current) {
            break false;
        }
        if let Some(&known) = memo.get(&current) {
            break known;
        }
        path.push(current);
        let class = current
            .checked_add(layout::UOBJECT_CLASS)
            .and_then(|address| memory.ptr_at(address))
            .unwrap_or(0);
        if class != 0 && is_level(class) {
            break true;
        }
        current = current
            .checked_add(layout::UOBJECT_OUTER)
            .and_then(|address| memory.ptr_at(address))
            .unwrap_or(0);
    };
    for link in path {
        memo.insert(link, answer);
    }
    answer
}

/// A bitfield `bool` property's byte offset and mask, found by name in `class` or its supers (2.3). Only a
/// `BoolProperty` whose four layout bytes agree is believed: one byte wide, at byte 0, one bit, both masks equal.
pub fn bool_property(engine: &Engine, class: usize, name: &str) -> Option<(usize, u8)> {
    let memory = engine.memory();
    for link in reflect::class_chain(memory, class) {
        let mut field = memory.ptr_at(link.checked_add(sim::USTRUCT_CHILD_PROPERTIES)?)?;
        let mut walked = 0;
        while field != 0 && walked < reflect::MAX_FIELDS {
            walked += 1;
            if engine.fname_at(field.checked_add(sim::FFIELD_NAME)?)?.as_str() == name {
                let kind = memory.ptr_at(field.checked_add(sim::FFIELD_CLASS)?)?;
                if engine.fname_at(kind)?.as_str() != "BoolProperty" {
                    return None;
                }
                let offset = usize::try_from(memory.i32_at(field.checked_add(sim::FPROPERTY_OFFSET)?)?).ok()?;
                let mut bytes = [0u8; 4];
                if !memory.read(field.checked_add(sim::FBOOL_FIELD_SIZE)?, &mut bytes) {
                    return None;
                }
                return bool_layout(offset, bytes);
            }
            field = memory.ptr_at(field.checked_add(sim::FFIELD_NEXT)?)?;
        }
    }
    None
}

/// `FieldSize`, `ByteOffset`, `ByteMask`, `FieldMask` as one believable bitfield bool.
pub fn bool_layout(offset: usize, [size, byte_offset, byte_mask, field_mask]: [u8; 4]) -> Option<(usize, u8)> {
    (size == 1 && byte_offset == 0 && byte_mask == field_mask && byte_mask.is_power_of_two())
        .then_some((offset, byte_mask))
}

/// The replication flag's line, once.
pub fn replicates_line(found: Option<(usize, u8)>) -> String {
    found.map_or_else(
        || "[MitRuntime] [Stub] ActorComponent.bReplicates did not read: effects sweep off".to_owned(),
        |(offset, mask)| format!("[MitRuntime] [Stub] ActorComponent.bReplicates at +0x{offset:X} mask 0x{mask:02X}"),
    )
}

/// A function the sweep needs and did not find.
pub fn missing_line(name: &str) -> String {
    format!("[MitRuntime] [Stub] ActorComponent.{name} not found: effects sweep off")
}

/// One sweep's figures.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Sweep {
    /// Particle components found in a level.
    pub components: usize,
    /// Deactivated this sweep.
    pub stopped: usize,
    /// Ticking turned off this sweep.
    pub unticked: usize,
    /// Left alone because they replicate.
    pub replicated: usize,
    /// How long the sweep took.
    pub ms: u64,
}

/// The sweep's line, logged on the first sweep and whenever it changed something (2.3).
pub fn sweep_line(sweep: &Sweep, first: bool) -> Option<String> {
    (first || sweep.stopped > 0 || sweep.unticked > 0).then(|| {
        format!(
            "[MitRuntime] [Stub] effects: {} particle components in levels, {} deactivated, {} unticked, {} left \
             (replicated), took {} ms",
            sweep.components, sweep.stopped, sweep.unticked, sweep.replicated, sweep.ms
        )
    })
}

// ---- 3: the package filter ----

/// The flag file that installs the package filter (3.2).
pub const PACKAGES_FLAG: &str = "RUNTIME_STUB_PACKAGES";
/// The optional list beside the executable (3.3).
pub const PACKAGES_FILE: &str = "rustic_isles_stub_packages.txt";
/// The prefixes refused when the list is absent: none, since refusing whole folders stops a hub booting (3.5).
pub const DEFAULT_REFUSED: [&str; 0] = [];
/// Distinct refused names logged one by one; after that only the totals.
pub const NAMED_REFUSALS: usize = 40;
/// The longest name the filter reads, in UTF-16 units.
pub const MAX_NAME: usize = 1024;

/// Which package names are refused: a `keep` prefix wins over a `refuse` one. Compared without case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    /// Refused prefixes, lower case.
    pub refuse: Vec<String>,
    /// Kept prefixes, lower case.
    pub keep: Vec<String>,
    /// Kept package names (a path's last segment), lower case.
    pub keep_names: Vec<String>,
}

impl Rules {
    /// The defaults (3.3).
    pub fn defaults() -> Self {
        Self {
            refuse: DEFAULT_REFUSED
                .iter()
                .map(|prefix| prefix.to_ascii_lowercase())
                .collect(),
            keep: Vec::new(),
            keep_names: Vec::new(),
        }
    }

    /// The list's grammar: one prefix a line, `!` before a kept one, `#` comments and blank lines skipped. A kept
    /// line without `/` is a package name, kept wherever it is; any other line not starting with `/` is ignored.
    pub fn parse(text: &str) -> Self {
        let mut rules = Self {
            refuse: Vec::new(),
            keep: Vec::new(),
            keep_names: Vec::new(),
        };
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (keep, prefix) = line.strip_prefix('!').map_or((false, line), |rest| (true, rest.trim()));
            if keep && !prefix.is_empty() && !prefix.contains('/') {
                rules.keep_names.push(prefix.to_ascii_lowercase());
                continue;
            }
            if !prefix.starts_with('/') {
                continue;
            }
            let prefix = prefix.to_ascii_lowercase();
            if keep {
                rules.keep.push(prefix);
            } else {
                rules.refuse.push(prefix);
            }
        }
        rules
    }

    /// Whether the long package name is refused.
    pub fn refuses(&self, long_name: &str) -> bool {
        let name = long_name.to_ascii_lowercase();
        let short = name.rsplit('/').next().unwrap_or(&name);
        self.refuse.iter().any(|prefix| name.starts_with(prefix.as_str()))
            && !self.keep.iter().any(|prefix| name.starts_with(prefix.as_str()))
            && !self.keep_names.iter().any(|kept| kept == short)
    }
}

/// The long package name for what `DoesPackageExist` was given: a long name as it is, or a game content filename
/// (`../../../Archon/Content/UI/X.uasset`) as `/Game/UI/X`. `None` for anything else.
pub fn long_name(given: &str) -> Option<String> {
    let given = given.replace('\\', "/");
    if given.starts_with('/') {
        return Some(given);
    }
    let lower = given.to_ascii_lowercase();
    let at = lower.find("/content/")?;
    let rest = &given[at + "/content/".len()..];
    let rest = rest.rsplit_once('.').map_or(rest, |(stem, _)| stem);
    Some(format!("/Game/{rest}"))
}

/// The install's line (3.2).
pub fn packages_on_line(rules: &Rules, from_file: bool) -> String {
    format!(
        "[MitRuntime] [Stub] packages: refusing {} (keeping {}) from {}",
        if rules.refuse.is_empty() {
            "nothing".to_owned()
        } else {
            rules.refuse.join(", ")
        },
        if rules.keep.is_empty() && rules.keep_names.is_empty() {
            "none".to_owned()
        } else {
            let mut kept = rules.keep.clone();
            kept.extend(rules.keep_names.iter().cloned());
            kept.join(", ")
        },
        if from_file { PACKAGES_FILE } else { "the defaults" },
    )
}

/// One refused package, logged for the first `NAMED_REFUSALS` distinct names.
pub fn refused_line(name: &str) -> String {
    format!("[MitRuntime] [Stub] package refused: {name}")
}

/// The running totals, logged from the after handler when they changed (3.4).
pub fn refusals_line(refusals: u64, distinct: usize) -> String {
    format!("[MitRuntime] [Stub] packages refused so far: {refusals} lookups of {distinct} packages")
}

/// The functions the sweep calls and their parameter layouts (2.3).
pub mod layouts {
    /// `ActorComponent.IsActive`.
    pub const RETURN: &[(&str, usize)] = &[("ReturnValue", 0x00)];
    /// `ActorComponent.Deactivate`.
    pub const NONE: &[(&str, usize)] = &[];
    /// `ActorComponent.SetComponentTickEnabled`.
    pub const TICK: &[(&str, usize)] = &[("bEnabled", 0x00)];
}

#[cfg(test)]
mod tests;
