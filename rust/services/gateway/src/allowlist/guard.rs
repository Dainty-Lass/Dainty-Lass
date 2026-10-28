//! The UDP ports the allowlist guards (DF4d): the game ports `ALLOWLIST_PORTS` names, plus the ranges the loaded
//! game modules registered. Nothing else is ever added, and a server with no module guards exactly the first.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use module_api::{ModuleId, PortRange};

use super::ports::{MAX_RANGES, check_ports};

/// A module's range never starts below this: the privileged ports are the host's own services' (DNS, SSH...).
pub const FIRST_MODULE_PORT: u16 = 1024;

type Changed = Arc<dyn Fn() + Send + Sync>;

pub struct Guard {
    base: PortRange,
    modules: Mutex<BTreeMap<ModuleId, Vec<PortRange>>>,
    changed: Mutex<Option<Changed>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn show(range: PortRange) -> String {
    if range.first == range.last {
        range.first.to_string()
    } else {
        format!("{}-{}", range.first, range.last)
    }
}

/// Sorted, with overlapping and adjacent ranges joined.
fn merged(mut ranges: Vec<PortRange>) -> Vec<PortRange> {
    ranges.sort_by_key(|range| (range.first, range.last));
    let mut out: Vec<PortRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match out.last_mut() {
            Some(last) if u32::from(range.first) <= u32::from(last.last) + 1 => last.last = last.last.max(range.last),
            _ => out.push(range),
        }
    }
    out
}

impl Guard {
    /// Guards `ports` (`ALLOWLIST_PORTS`, one port or range) and nothing else yet.
    pub fn new(ports: &str) -> Result<Self, String> {
        let text = check_ports(ports)?;
        let number = |text: &str| text.parse::<u16>().map_err(|e| e.to_string());
        let base = match text.split_once('-') {
            Some((first, last)) => PortRange {
                first: number(first)?,
                last: number(last)?,
            },
            None => PortRange::single(number(&text)?),
        };
        Ok(Self {
            base,
            modules: Mutex::default(),
            changed: Mutex::default(),
        })
    }

    /// Called after every change to the set (the helper re-applies its rule).
    pub fn on_change(&self, changed: Changed) {
        *lock(&self.changed) = Some(changed);
    }

    fn notify(&self) {
        let changed = lock(&self.changed).clone();
        if let Some(changed) = changed {
            changed();
        }
    }

    /// The firewall's local ports: `ALLOWLIST_PORTS` alone, or with the modules' ranges, sorted and joined.
    pub fn text(&self) -> String {
        merged(self.all(&lock(&self.modules), None))
            .into_iter()
            .map(show)
            .collect::<Vec<_>>()
            .join(",")
    }

    fn all(&self, modules: &BTreeMap<ModuleId, Vec<PortRange>>, without: Option<&ModuleId>) -> Vec<PortRange> {
        std::iter::once(self.base)
            .chain(
                modules
                    .iter()
                    .filter(|(id, _)| Some(*id) != without)
                    .flat_map(|(_, ranges)| ranges.iter().copied()),
            )
            .collect()
    }

    /// What each module has guarded, as text.
    pub fn modules(&self) -> BTreeMap<String, Vec<String>> {
        lock(&self.modules)
            .iter()
            .map(|(id, ranges)| (id.to_string(), ranges.iter().copied().map(show).collect()))
            .collect()
    }

    /// Whether `port` is inside `ALLOWLIST_PORTS` or any range registered by a loaded game module (DF4d).
    pub fn covers_port(&self, port: u16) -> bool {
        self.all(&lock(&self.modules), None)
            .into_iter()
            .any(|range| (range.first..=range.last).contains(&port))
    }

    /// Adds `module`'s ranges, or refuses all of them: each must be a valid range of ports from
    /// [`FIRST_MODULE_PORT`] up, overlapping neither `ALLOWLIST_PORTS`, nor another module's ranges, nor its own.
    pub fn attach(&self, module: &ModuleId, ranges: &[PortRange]) -> Result<(), String> {
        {
            let mut modules = lock(&self.modules);
            let others = self.all(&modules, Some(module));
            for (at, range) in ranges.iter().enumerate() {
                let shown = show(*range);
                if !range.is_valid() {
                    return Err(format!("{shown} is not a valid range of ports"));
                }
                if range.first < FIRST_MODULE_PORT {
                    return Err(format!(
                        "{shown} is in the privileged ports (below {FIRST_MODULE_PORT})"
                    ));
                }
                if range.overlaps(self.base) {
                    return Err(format!(
                        "{shown} overlaps the server's own game ports {}",
                        show(self.base)
                    ));
                }
                if let Some(other) = others.iter().chain(&ranges[..at]).find(|other| other.overlaps(*range)) {
                    return Err(format!("{shown} overlaps {}, which is guarded already", show(*other)));
                }
            }
            if merged([others, ranges.to_vec()].concat()).len() > MAX_RANGES {
                return Err(format!("the firewall rule holds at most {MAX_RANGES} ranges"));
            }
            if ranges.is_empty() {
                modules.remove(module);
            } else {
                modules.insert(module.clone(), ranges.to_vec());
            }
        }
        self.notify();
        Ok(())
    }

    /// Takes `module`'s ranges back out.
    pub fn detach(&self, module: &ModuleId) {
        let removed = lock(&self.modules).remove(module).is_some();
        if removed {
            self.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn id(text: &str) -> ModuleId {
        text.parse().unwrap()
    }

    fn range(first: u16, last: u16) -> PortRange {
        PortRange { first, last }
    }

    #[test]
    fn no_module_guards_exactly_the_game_ports() {
        for ports in ["8770-8777", "8777", "8770-8801"] {
            assert_eq!(Guard::new(ports).unwrap().text(), ports);
        }
        assert_eq!(Guard::new("08770-08777").unwrap().text(), "8770-8777");
        assert!(Guard::new("8770,8771").is_err());
    }

    #[test]
    fn the_union_is_sorted_and_adjacent_ranges_join() {
        let guard = Guard::new("8770-8777").unwrap();
        guard
            .attach(&id("b"), &[range(64020, 64020), range(64000, 64009)])
            .unwrap();
        guard.attach(&id("a"), &[range(64010, 64019)]).unwrap();
        assert_eq!(guard.text(), "8770-8777,64000-64020");
        assert_eq!(guard.modules()["a"], ["64010-64019"]);
        guard.detach(&id("a"));
        assert_eq!(guard.text(), "8770-8777,64000-64009,64020");
        guard.detach(&id("b"));
        assert_eq!(guard.text(), "8770-8777");
    }

    #[test]
    fn refuses_what_the_allowlist_would_not_open() {
        let guard = Guard::new("8770-8777").unwrap();
        guard.attach(&id("a"), &[range(64000, 64009)]).unwrap();
        for (bad, why) in [
            (range(0, 5), "not a valid"),
            (range(9, 8), "not a valid"),
            (range(2000, 4000), "not a valid"),
            (range(53, 53), "privileged"),
            (range(1, 1023), "privileged"),
            (range(1000, 1010), "privileged"),
            (range(8777, 8780), "game ports"),
            (range(8700, 8770), "game ports"),
            (range(64009, 64012), "guarded already"),
        ] {
            let error = guard.attach(&id("b"), &[bad]).unwrap_err();
            assert!(error.contains(why), "{bad:?}: {error}");
        }
        assert!(
            guard
                .attach(&id("b"), &[range(64100, 64109), range(64105, 64110)])
                .is_err()
        );
        // Nothing of a refused module stays
        assert_eq!(guard.text(), "8770-8777,64000-64009");
        assert!(!guard.modules().contains_key("b"));
    }

    #[test]
    fn a_module_s_ranges_are_replaced_not_doubled_and_the_rule_stays_small() {
        let guard = Guard::new("8770-8777").unwrap();
        guard.attach(&id("a"), &[range(64000, 64009)]).unwrap();
        guard.attach(&id("a"), &[range(64005, 64015)]).unwrap();
        assert_eq!(guard.text(), "8770-8777,64005-64015");
        let wide: Vec<PortRange> = (0..MAX_RANGES as u16)
            .map(|n| range(20000 + n * 10, 20000 + n * 10))
            .collect();
        assert!(guard.attach(&id("b"), &wide).unwrap_err().contains("at most"));
        assert!(guard.attach(&id("b"), &wide[..MAX_RANGES - 2]).is_ok());
    }

    #[test]
    fn every_change_is_announced_once() {
        let guard = Guard::new("8770-8777").unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        guard.on_change(Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
        }));
        guard.attach(&id("a"), &[range(64000, 64009)]).unwrap();
        guard.attach(&id("b"), &[range(8777, 8777)]).unwrap_err();
        guard.detach(&id("nobody"));
        guard.detach(&id("a"));
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
