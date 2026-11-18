//! The object census (spec-runtime-census): the walk over the object array, the size check, the name resolution and
//! every log text. The DLL decides when, and runs it on the game thread through its region-cached reader.

use crate::engine::Engine;
use crate::memory::Memory;
use crate::offsets::{census, layout};
use crate::reflect;
use crate::server_life::HubRole;
use std::collections::HashMap;

/// The flag file that turns the census on (4.1).
pub const FLAG: &str = "RUNTIME_CENSUS";
/// The watchdog poll of the first census (60 s at 2 s a poll).
pub const FIRST_POLL: u64 = 30;
/// Polls between censuses after the first (300 s).
pub const EVERY_POLLS: u64 = 150;
/// The longest outer chain walked.
pub const MAX_OUTERS: usize = 64;
/// Lines per section (5).
pub const TOP_CLASSES: usize = 25;
/// Actor lines.
pub const TOP_ACTORS: usize = 15;
/// Folder lines.
pub const TOP_FOLDERS: usize = 15;
/// Package lines.
pub const TOP_PACKAGES: usize = 10;
/// The largest class size believed (2.3).
pub const MAX_CLASS_SIZE: u32 = 16 * 1024 * 1024;

/// The watchdog's third start line (4.1).
pub const ON_LINE: &str = "[MitRuntime] [Census] on (RUNTIME_CENSUS.flag): 60 s after listen, then every 300 s";
/// Names off (5).
pub const NAMES_OFF_LINE: &str = "[MitRuntime] [Census] skipped: names are off (R0 6.2)";
/// The array did not read (5).
pub const ARRAY_LINE: &str = "[MitRuntime] [Census] skipped: the object array did not read";

/// Whether watchdog poll `iter` asks for a census (4.2).
pub fn due(iter: u64) -> bool {
    iter >= FIRST_POLL && (iter - FIRST_POLL).is_multiple_of(EVERY_POLLS)
}

/// One walk's tallies, by class pointer and by outermost pointer (3).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Walk {
    /// Objects counted.
    pub objects: u64,
    /// Items with an object pointer whose header or class did not read, and items of chunks that did not read.
    pub unreadable: u64,
    /// Objects per class.
    pub classes: HashMap<usize, u64>,
    /// Objects per outermost object.
    pub outermost: HashMap<usize, u64>,
}

fn word(bytes: &[u8], offset: usize) -> usize {
    bytes
        .get(offset..offset + 8)
        .and_then(|slice| <[u8; 8]>::try_from(slice).ok())
        .and_then(|raw| usize::try_from(u64::from_le_bytes(raw)).ok())
        .unwrap_or(0)
}

fn add(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// The walk (3): one read per chunk and one per object header, outers memoized.
pub fn walk(memory: &dyn Memory, objects: usize) -> Option<Walk> {
    let count = memory.i32_at(objects.checked_add(layout::OBJECTS_COUNT)?)?;
    if !(0..=layout::MAX_OBJECTS).contains(&count) {
        return None;
    }
    let count = usize::try_from(count).ok()?;
    let chunks = usize::try_from(memory.i32_at(objects.checked_add(layout::OBJECTS_CHUNK_COUNT)?)?).ok()?;
    let table = memory.ptr_at(objects.checked_add(layout::OBJECTS_CHUNKS)?)?;
    let mut walk = Walk::default();
    let mut memo: HashMap<usize, usize> = HashMap::new();
    let mut header = [0u8; layout::UOBJECT_SIZE];
    for chunk in 0..count.div_ceil(layout::OBJECTS_PER_CHUNK) {
        let items = (count - chunk * layout::OBJECTS_PER_CHUNK).min(layout::OBJECTS_PER_CHUNK);
        let base = (chunk < chunks)
            .then(|| table.checked_add(chunk * 8))
            .flatten()
            .and_then(|address| memory.ptr_at(address))
            .filter(|&base| base != 0);
        let mut bytes = vec![0u8; items * layout::OBJECT_ITEM_SIZE];
        if !base.is_some_and(|base| memory.read(base, &mut bytes)) {
            walk.unreadable += add(items);
            continue;
        }
        for item in bytes.as_chunks::<{ layout::OBJECT_ITEM_SIZE }>().0 {
            let object = word(item, 0);
            if object == 0 {
                continue;
            }
            if !memory.read(object, &mut header) {
                walk.unreadable += 1;
                continue;
            }
            let class = word(&header, layout::UOBJECT_CLASS);
            if class == 0 {
                walk.unreadable += 1;
                continue;
            }
            let outer = word(&header, layout::UOBJECT_OUTER);
            let top = if outer == 0 {
                object
            } else {
                outermost(memory, outer, &mut memo)
            };
            walk.objects += 1;
            *walk.classes.entry(class).or_default() += 1;
            *walk.outermost.entry(top).or_default() += 1;
        }
    }
    Some(walk)
}

/// The last object of `start`'s outer chain (3 step 4); every link visited is memoized with the answer.
fn outermost(memory: &dyn Memory, start: usize, memo: &mut HashMap<usize, usize>) -> usize {
    let mut path: Vec<usize> = Vec::new();
    let mut current = start;
    let top = loop {
        if let Some(&top) = memo.get(&current) {
            break top;
        }
        path.push(current);
        if path.len() >= MAX_OUTERS {
            break current;
        }
        match current
            .checked_add(layout::UOBJECT_OUTER)
            .and_then(|address| memory.ptr_at(address))
        {
            Some(next) if next != 0 && !path.contains(&next) => current = next,
            _ => break current,
        }
    };
    for link in path {
        memo.insert(link, top);
    }
    top
}

/// The size check (2.3) on the root of a class chain: its name and `PropertiesSize`.
pub fn size_check(root: Option<(&str, Option<i32>)>) -> Result<(), String> {
    let expected = layout::UOBJECT_SIZE;
    match root {
        None => Err("no class".to_owned()),
        Some(("Object", Some(size))) if usize::try_from(size).ok() == Some(expected) => Ok(()),
        Some((name, size)) => Err(format!(
            "root {name} has {} bytes, expected Object with {expected}",
            size.map_or_else(|| "?".to_owned(), |size| size.to_string())
        )),
    }
}

/// One class's size, when believable (2.3).
pub fn class_size(raw: Option<i32>) -> Option<u32> {
    let size = u32::try_from(raw?).ok()?;
    (usize::try_from(size).ok()? >= layout::UOBJECT_SIZE && size <= MAX_CLASS_SIZE).then_some(size)
}

/// A resolved class (3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassRow {
    /// The class's short name, `?` when it did not decode.
    pub name: String,
    /// Objects of the class.
    pub count: u64,
    /// `PropertiesSize`, with sizes on and believable.
    pub size: Option<u32>,
    /// The class is an `Actor`.
    pub actor: bool,
}

/// Everything the lines print (5).
#[derive(Debug, Clone)]
pub struct Report {
    /// Resolved classes.
    pub classes: Vec<ClassRow>,
    /// Outermost names with their object counts.
    pub packages: Vec<(String, u64)>,
    /// Objects counted.
    pub objects: u64,
    /// Unreadable items.
    pub unreadable: u64,
    /// The size check's verdict.
    pub sizes: Result<(), String>,
    /// How long the run took.
    pub ms: u64,
    /// R8's connection count.
    pub connections: i32,
    /// R8's stale mark.
    pub stale: bool,
    /// R8's role.
    pub role: HubRole,
}

/// A walk with its pointers resolved (3.1).
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Resolved classes.
    pub classes: Vec<ClassRow>,
    /// Outermost names with their object counts.
    pub packages: Vec<(String, u64)>,
    /// The size check's verdict.
    pub sizes: Result<(), String>,
}

/// Resolves a walk's pointers through the engine (3.1). The size check uses the most numerous class's root.
pub fn resolve(engine: &Engine, walk: &Walk) -> Resolved {
    let memory = engine.memory();
    let size_at = |class: usize| {
        class
            .checked_add(census::USTRUCT_PROPERTIES_SIZE)
            .and_then(|address| memory.i32_at(address))
    };
    let sample = walk
        .classes
        .iter()
        .max_by_key(|&(&class, &count)| (count, class))
        .map(|(&class, _)| class);
    let root = sample.and_then(|class| reflect::class_chain(memory, class).last().copied());
    let root_name = root.map(|root| engine.object_name(root).unwrap_or_else(|| "?".to_owned()));
    let sizes = size_check(root.zip(root_name.as_deref()).map(|(root, name)| (name, size_at(root))));
    let classes = walk
        .classes
        .iter()
        .map(|(&class, &count)| ClassRow {
            name: engine.object_name(class).unwrap_or_else(|| "?".to_owned()),
            count,
            size: sizes.is_ok().then(|| class_size(size_at(class))).flatten(),
            actor: reflect::class_is_a(engine, class, "Actor"),
        })
        .collect();
    let packages = walk
        .outermost
        .iter()
        .map(|(&object, &count)| (package_name(engine, object), count))
        .collect();
    Resolved {
        classes,
        packages,
        sizes,
    }
}

/// An outermost object's raw name: a package's is its whole path (`/Game/Maps/...`), which `object_name`'s short form
/// would cut to the last segment.
fn package_name(engine: &Engine, object: usize) -> String {
    object
        .checked_add(layout::UOBJECT_NAME)
        .and_then(|address| engine.fname_at(address))
        .unwrap_or_else(|| "?".to_owned())
}

/// A package path's first two segments (3.1); a name without a leading `/` is its own folder.
pub fn folder_of(package: &str) -> &str {
    if !package.starts_with('/') {
        return package;
    }
    match package[1..].find('/').map(|first| first + 1) {
        Some(first) => match package[first + 1..].find('/') {
            Some(second) => &package[..first + 1 + second],
            None => package,
        },
        None => package,
    }
}

/// `<n> KiB` below 1 MiB, else `<n.n> MiB` (5).
pub fn size_text(bytes: u64) -> String {
    if bytes < 1_048_576 {
        format!("{} KiB", bytes / 1024)
    } else {
        #[allow(clippy::cast_precision_loss, reason = "a display figure")]
        let mib = bytes as f64 / 1_048_576.0;
        format!("{mib:.1} MiB")
    }
}

/// What the next census compares against, by name.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Previous {
    /// Objects per class name.
    pub classes: HashMap<String, u64>,
    /// Objects per folder.
    pub folders: HashMap<String, u64>,
}

fn delta(previous: Option<&HashMap<String, u64>>, name: &str, count: u64) -> String {
    let Some(previous) = previous else {
        return String::new();
    };
    let before = previous.get(name).copied().unwrap_or(0);
    match count.cmp(&before) {
        std::cmp::Ordering::Greater => format!(" (+{})", count - before),
        std::cmp::Ordering::Less => format!(" (-{})", before - count),
        std::cmp::Ordering::Equal => " (=)".to_owned(),
    }
}

struct Merged {
    name: String,
    count: u64,
    shallow: Option<u64>,
    each: Option<u32>,
    actor: bool,
}

/// Classes sharing a short name are one row: counts and shallow bytes add, `each` stays only when every one agrees.
fn merge(classes: &[ClassRow]) -> Vec<Merged> {
    let mut rows: HashMap<&str, Merged> = HashMap::new();
    for class in classes {
        let shallow = class.size.map(|size| u64::from(size) * class.count);
        rows.entry(&class.name)
            .and_modify(|row| {
                row.count += class.count;
                row.shallow = row.shallow.zip(shallow).map(|(a, b)| a + b);
                row.each = row.each.filter(|&each| Some(each) == class.size);
                row.actor |= class.actor;
            })
            .or_insert(Merged {
                name: class.name.clone(),
                count: class.count,
                shallow,
                each: class.size,
                actor: class.actor,
            });
    }
    rows.into_values().collect()
}

fn opt(value: Option<impl ToString>) -> String {
    value.map_or_else(|| "?".to_owned(), |value| value.to_string())
}

/// Every line of one census (5), and what the next one compares against.
pub fn lines(report: &Report, previous: Option<&Previous>) -> (Vec<String>, Previous) {
    let mut rows = merge(&report.classes);
    let sized = report.sizes.is_ok();
    rows.sort_by(|a, b| {
        let by_size = if sized {
            b.shallow.unwrap_or(0).cmp(&a.shallow.unwrap_or(0))
        } else {
            std::cmp::Ordering::Equal
        };
        by_size.then(b.count.cmp(&a.count)).then(a.name.cmp(&b.name))
    });
    let shallow: u64 = rows.iter().filter_map(|row| row.shallow).sum();
    let actors: u64 = rows.iter().filter(|row| row.actor).map(|row| row.count).sum();
    let mut folders: HashMap<String, u64> = HashMap::new();
    for (package, count) in &report.packages {
        *folders.entry(folder_of(package).to_owned()).or_default() += count;
    }
    let mut out = Vec::new();
    out.push(format!(
        "[MitRuntime] [Census] {} objects in {} classes ({} unreadable), shallow {}, actors {actors}, connections {}{}, \
         role {}, took {} ms{}",
        report.objects,
        report.classes.len(),
        report.unreadable,
        size_text(shallow),
        report.connections,
        if report.stale { " (stale)" } else { "" },
        report.role.text(),
        report.ms,
        report
            .sizes
            .as_ref()
            .err()
            .map_or_else(String::new, |reason| format!(", sizes off: {reason}")),
    ));
    let prev_classes = previous.map(|previous| &previous.classes);
    let shown = rows.len().min(TOP_CLASSES);
    for (i, row) in rows.iter().take(shown).enumerate() {
        out.push(format!(
            "[MitRuntime] [Census] class {}/{shown} {}: {}{}, {} shallow, {} B each{}",
            i + 1,
            row.name,
            row.count,
            delta(prev_classes, &row.name, row.count),
            opt(row.shallow.map(size_text)),
            opt(row.each),
            if row.actor { ", actor" } else { "" },
        ));
    }
    let mut actor_rows: Vec<&Merged> = rows.iter().filter(|row| row.actor).collect();
    actor_rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    let shown = actor_rows.len().min(TOP_ACTORS);
    for (i, row) in actor_rows.iter().take(shown).enumerate() {
        out.push(format!(
            "[MitRuntime] [Census] actor {}/{shown} {}: {}{}",
            i + 1,
            row.name,
            row.count,
            delta(prev_classes, &row.name, row.count),
        ));
    }
    let mut folder_rows: Vec<(&String, &u64)> = folders.iter().collect();
    folder_rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let shown = folder_rows.len().min(TOP_FOLDERS);
    for (i, (folder, count)) in folder_rows.iter().take(shown).enumerate() {
        out.push(format!(
            "[MitRuntime] [Census] folder {}/{shown} {folder}: {count} objects{}",
            i + 1,
            delta(previous.map(|previous| &previous.folders), folder, **count),
        ));
    }
    let mut packages: Vec<&(String, u64)> = report.packages.iter().collect();
    packages.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let shown = packages.len().min(TOP_PACKAGES);
    for (i, (package, count)) in packages.iter().take(shown).enumerate() {
        out.push(format!(
            "[MitRuntime] [Census] package {}/{shown} {package}: {count} objects",
            i + 1
        ));
    }
    let next = Previous {
        classes: rows.into_iter().map(|row| (row.name, row.count)).collect(),
        folders,
    };
    (out, next)
}

#[cfg(test)]
mod tests;
