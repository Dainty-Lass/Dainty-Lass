// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hunt tables and the resolver (spec §9): which matchmaker row, map and Behemoth a player hunt starts, tag
//! routing as Unreal's `FGameplayTagQuery` evaluates it, the Trials rotation, the missing-hunt registry and the
//! start-up audits.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rand::RngExt as _;
use serde_json::{Map, Value};
use service_kit::js;

use crate::hub::Level;

pub const PLAYER_TABLE: &str = "player_hunts_table";
/// In lookup order; the main table first wins a name collision. The `_new` Trials tables are reached only through
/// tag routing.
pub const MATCHMAKER_TABLES: [&str; 6] = [
    "matchmaker_hunts_table",
    "arena_easy_matchmaker_hunts",
    "arena_hard_matchmaker_hunts",
    "arena_elite_matchmaker_hunts",
    "arena_hard_matchmaker_hunts_new",
    "arena_elite_matchmaker_hunts_new",
];
/// The rotation's length, as the runtime counts it.
pub const TRIALS_ROTATION_ROW_COUNT: i64 = 181;
pub const DEFAULT_TRIALS_ROTATION_MINUTES: u64 = 10_080;

/// Keys that are array indexes first in ascending order, then the rest in insertion order (JavaScript's).
pub(crate) fn js_entries(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    fn index(key: &str) -> Option<u32> {
        let canonical =
            key == "0" || (!key.starts_with('0') && !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()));
        canonical
            .then(|| key.parse::<u32>().ok())
            .flatten()
            .filter(|n| *n < u32::MAX)
    }
    let mut indexed: Vec<(u32, &String, &Value)> = Vec::new();
    let mut rest = Vec::new();
    for (k, v) in map {
        match index(k) {
            Some(i) => indexed.push((i, k, v)),
            None => rest.push((k, v)),
        }
    }
    indexed.sort_by_key(|(i, _, _)| *i);
    indexed.into_iter().map(|(_, k, v)| (k, v)).chain(rest).collect()
}

#[derive(Debug, Clone, Default)]
pub struct Tables {
    pub player_rows: Map<String, Value>,
    /// The matchmaker tables that were loaded, in lookup order
    pub matchmaker_sources: Vec<(String, Map<String, Value>)>,
}

/// The file names for a version, as core's `director_files(trials)` lists them.
pub fn table_files(trials: bool) -> Vec<String> {
    let count = if trials { 6 } else { 4 };
    std::iter::once(PLAYER_TABLE)
        .chain(MATCHMAKER_TABLES.iter().copied().take(count))
        .map(|name| format!("{name}.json"))
        .collect()
}

fn load_rows(path: &Path) -> Result<Map<String, Value>, String> {
    let bytes = std::fs::read(path)
        .map_err(|_| format!("[hunt-manifest] Required hunt table is missing: {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value = service_kit::jsjson::parse(text).map_err(|e| format!("{}: {e}", path.display()))?;
    match value.get(0).and_then(|first| first.get("Rows")) {
        Some(Value::Object(rows)) => Ok(rows.clone()),
        _ => Err(format!(
            "[hunt-manifest] {} is not a DataTable export (no Rows)",
            path.display()
        )),
    }
}

/// All seven tables (1.14.7 has the Trials tables).
pub fn load_tables(dir: &Path) -> Result<Tables, String> {
    load_tables_for(dir, true)
}

pub fn load_tables_for(dir: &Path, trials: bool) -> Result<Tables, String> {
    let files = table_files(trials);
    let mut tables = Tables::default();
    for (at, file) in files.iter().enumerate() {
        let rows = load_rows(&dir.join(file))?;
        if at == 0 {
            tables.player_rows = rows;
        } else {
            let name = file.trim_end_matches(".json").to_owned();
            tables.matchmaker_sources.push((name, rows));
        }
    }
    Ok(tables)
}

/// The arena week of `Arena_MatchmakerHunt_{Easy|Hard|Elite}_{www}`.
pub fn arena_week(id: &str) -> Option<&str> {
    let rest = id.strip_prefix("Arena_MatchmakerHunt_")?;
    let (tier, week) = rest.split_once('_')?;
    let tier_ok = matches!(tier, "Easy" | "Hard" | "Elite");
    (tier_ok && week.len() == 3 && week.bytes().all(|b| b.is_ascii_digit())).then_some(week)
}

/// `TRIALS_ROTATION_MINUTES`: unset is a week; anything but a whole number in [1, 525 600] warns and is a week.
pub fn parse_trials_rotation_minutes(raw: Option<&str>) -> (u64, Option<String>) {
    let Some(raw) = raw else {
        return (DEFAULT_TRIALS_ROTATION_MINUTES, None);
    };
    let n = js::number(Some(&Value::String(raw.to_owned())));
    if n.fract() == 0.0 && (1.0..=525_600.0).contains(&n) {
        return (n as u64, None);
    }
    let warning = format!("[TrialsRotation] invalid TRIALS_ROTATION_MINUTES='{raw}'; using 10080");
    (DEFAULT_TRIALS_ROTATION_MINUTES, Some(warning))
}

/// The featured week at `now_ms`, three digits (the runtime's formula).
pub fn current_trials_week(now_ms: i64, minutes: u64) -> String {
    let span = (minutes.max(1) as i64).saturating_mul(60_000);
    let week = now_ms.div_euclid(span).rem_euclid(TRIALS_ROTATION_ROW_COUNT) + 1;
    format!("{week:03}")
}

fn is_tag_match(query_tag: &str, row_tag: &str) -> bool {
    row_tag == query_tag
        || row_tag
            .strip_prefix(query_tag)
            .is_some_and(|rest| rest.starts_with('.'))
}

fn first_present<'a>(object: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| object.get(*k).filter(|v| !v.is_null()))
}

// Legitimate queries nest two or three deep; anything deeper counts as an overflow (§16 item 14)
const MAX_DEPTH: usize = 64;

struct Query<'a> {
    stream: &'a [Value],
    dictionary: Vec<String>,
    tags: &'a [&'a str],
    at: usize,
}

impl Query<'_> {
    fn number(&self, at: usize) -> f64 {
        match self.stream.get(at) {
            Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }

    fn count(&mut self) -> usize {
        let n = self.number(self.at);
        self.at += 1;
        if n > 0.0 {
            // Never past what is left of the stream: the result is the same as reading past its end
            let left = self.stream.len().saturating_sub(self.at);
            (n.ceil().min(left as f64 + 1.0)) as usize
        } else {
            0
        }
    }

    /// None is an overflow: the whole query does not match.
    fn expression(&mut self, depth: usize) -> Option<bool> {
        if self.at >= self.stream.len() || depth > MAX_DEPTH {
            return None;
        }
        let kind = self.number(self.at);
        self.at += 1;
        if (1.0..=3.0).contains(&kind) {
            let count = self.count();
            let mut contributed = Vec::new();
            for _ in 0..count {
                if self.at >= self.stream.len() {
                    break;
                }
                let index = self.number(self.at);
                self.at += 1;
                if index >= 0.0 && index.fract() == 0.0 {
                    let tag = self.dictionary.get(index as usize).filter(|t| !t.is_empty());
                    contributed.extend(tag.cloned());
                }
            }
            let matches = |tag: &String| self.tags.iter().any(|row| is_tag_match(tag, row));
            return Some(if kind == 1.0 {
                contributed.iter().any(matches)
            } else if kind == 2.0 {
                contributed.iter().all(matches)
            } else {
                !contributed.iter().any(matches)
            });
        }
        if (4.0..=6.0).contains(&kind) {
            let count = self.count();
            let mut results = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                results.push(self.expression(depth + 1)?);
            }
            return Some(if kind == 4.0 {
                results.iter().any(|r| *r)
            } else if kind == 5.0 {
                results.iter().all(|r| *r)
            } else {
                !results.iter().any(|r| *r)
            });
        }
        None
    }
}

/// A tag query (`QueryTokenStream`, `TagDictionary` or their capture names) against a row's tags.
pub fn evaluate_tag_query(query: &Value, row_tags: &[&str]) -> bool {
    let stream = match first_present(query, &["QueryTokenStream", "queryTokenStream", "tokenStream"]) {
        Some(Value::Array(items)) => items.as_slice(),
        _ => &[],
    };
    let dictionary = match first_present(query, &["TagDictionary", "tagDictionary"]) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|entry| match entry {
                Value::String(s) => s.clone(),
                other => match other.get("TagName").filter(|v| !v.is_null()) {
                    Some(name) => js::string(Some(name)),
                    None => String::new(),
                },
            })
            .collect(),
        _ => Vec::new(),
    };
    if stream.len() < 3 || !js::truthy(stream.get(1)) {
        return false;
    }
    let mut query = Query {
        stream,
        dictionary,
        tags: row_tags,
        at: 2,
    };
    query.expression(0).unwrap_or(false)
}

fn row_tags(row: &Value) -> Vec<&str> {
    match row.get("HuntTags") {
        Some(Value::Array(tags)) => tags.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// `{id}:{hunt id}` for each id, joined with `,`.
pub fn expected_player_args(ids: &[Value], hunt_id: &str) -> String {
    ids.iter()
        .map(|id| format!("{}:{hunt_id}", js::string(Some(id))))
        .collect::<Vec<_>>()
        .join(",")
}

pub type HuntLog = Arc<dyn Fn(Level, &str) + Send + Sync>;
pub type MissingHuntHook = Arc<dyn Fn(&str) + Send + Sync>;

pub fn tracing_hunt_log() -> HuntLog {
    Arc::new(|level, message| match level {
        Level::Info => tracing::info!("{message}"),
        Level::Warn => tracing::warn!("{message}"),
        Level::Error => tracing::error!("{message}"),
    })
}

pub struct ResolverOptions {
    pub tables: Tables,
    pub log: HuntLog,
    pub trials_rotation_minutes: u64,
    pub now: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// A value in `[min, max)`
    pub random_int: Arc<dyn Fn(u64, u64) -> u64 + Send + Sync>,
    pub on_missing_hunt: Option<MissingHuntHook>,
}

impl ResolverOptions {
    /// The system clock and generator, `tracing`, a week's rotation, no registry.
    pub fn new(tables: Tables) -> Self {
        Self {
            tables,
            log: tracing_hunt_log(),
            trials_rotation_minutes: DEFAULT_TRIALS_ROTATION_MINUTES,
            now: Arc::new(|| chrono::Utc::now().timestamp_millis()),
            random_int: Arc::new(|min, max| {
                if max > min {
                    rand::rng().random_range(min..max)
                } else {
                    min
                }
            }),
            on_missing_hunt: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableAudit {
    pub player_rows: usize,
    pub direct_rows: usize,
    pub tag_routed_rows: usize,
    pub tag_routed_refs: usize,
    pub matchmaker_rows: usize,
    pub tables: usize,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageAudit {
    pub total: usize,
    pub unresolved: Vec<String>,
    pub referenced: BTreeSet<String>,
    pub maps: BTreeSet<String>,
    pub game_modes: BTreeSet<String>,
    pub max_players: Vec<i64>,
}

impl CoverageAudit {
    pub fn resolvable(&self) -> usize {
        self.total - self.unresolved.len()
    }
}

pub struct HuntResolver {
    options: ResolverOptions,
    quiet: AtomicBool,
}

impl HuntResolver {
    pub fn new(options: ResolverOptions) -> Self {
        Self {
            options,
            quiet: AtomicBool::new(false),
        }
    }

    pub fn tables(&self) -> &Tables {
        &self.options.tables
    }

    pub fn set_quiet(&self, quiet: bool) {
        self.quiet.store(quiet, Ordering::SeqCst);
    }

    fn log(&self, level: Level, message: &str) {
        if !self.quiet.load(Ordering::SeqCst) {
            (self.options.log)(level, message);
        }
    }

    fn pick<'a>(&self, candidates: &'a [String]) -> &'a str {
        let n = candidates.len() as u64;
        let at = (self.options.random_int)(0, n).min(n.saturating_sub(1));
        candidates.get(at as usize).map_or("", String::as_str)
    }

    fn player_row(&self, id: &str) -> Option<&Value> {
        self.options.tables.player_rows.get(id).filter(|v| !v.is_null())
    }

    pub fn find_matchmaker_row(&self, id: &str) -> Option<&Value> {
        self.options
            .tables
            .matchmaker_sources
            .iter()
            .find_map(|(_, rows)| rows.get(id).filter(|v| !v.is_null()))
    }

    fn source(&self, name: &str) -> Option<&Map<String, Value>> {
        let sources = &self.options.tables.matchmaker_sources;
        sources.iter().find(|(n, _)| n == name).map(|(_, rows)| rows)
    }

    fn direct_candidates(&self, row: &Value) -> Vec<String> {
        let Some(Value::Array(handles)) = row.get("MatchmakerHuntIDs") else {
            return Vec::new();
        };
        handles
            .iter()
            .filter_map(|h| h.get("RowName").and_then(Value::as_str))
            .filter(|name| !name.is_empty() && self.find_matchmaker_row(name).is_some())
            .map(str::to_owned)
            .collect()
    }

    /// The rows of the tables a player row's tag lists name that every query of the list matches.
    pub fn tag_routed_matchmaker_hunt_ids(&self, player_hunt_id: &str, row: &Value) -> Vec<String> {
        let Some(Value::Array(lists)) = row.get("MatchmakerHuntsByTag") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for list in lists {
            let table = first_present(list, &["MatchmakerTable"])
                .and_then(|t| t.get("ObjectName").filter(|v| !v.is_null()))
                .or_else(|| list.get("matchmakerTable").filter(|v| !v.is_null()));
            let mut name = table.and_then(Value::as_str).unwrap_or("").to_owned();
            if let Some((_, rest)) = name.split_once('\'') {
                name = rest.split('\'').next().unwrap_or("").to_owned();
            }
            if !MATCHMAKER_TABLES.contains(&name.as_str()) {
                self.log(
                    Level::Warn,
                    &format!(
                        "[HuntTagRouting] {player_hunt_id} references matchmaker table '{name}' which is not \
                         registered; add it to HUNT_TABLE_FILES in shared/hunt-manifest"
                    ),
                );
                continue;
            }
            let queries = match first_present(list, &["HuntTags", "queries"]) {
                Some(Value::Array(q)) if !q.is_empty() => q,
                _ => continue,
            };
            let Some(rows) = self.source(&name) else {
                continue;
            };
            for (id, candidate) in js_entries(rows) {
                let tags = row_tags(candidate);
                let has_tags = matches!(candidate.get("HuntTags"), Some(Value::Array(t)) if !t.is_empty());
                if has_tags && queries.iter().all(|q| evaluate_tag_query(q, &tags)) {
                    out.push(id.clone());
                }
            }
        }
        out
    }

    /// The matchmaker hunt id a player hunt starts.
    pub fn matchmaker_hunt_id_for(&self, hunt_id: &str) -> Result<String, String> {
        let Some(row) = self.player_row(hunt_id) else {
            if let Some(report) = &self.options.on_missing_hunt {
                report(hunt_id);
            }
            return Err(format!(
                "player_hunts_table has no usable row for HuntId '{hunt_id}' (missing row or empty MatchmakerHuntIDs)"
            ));
        };
        let direct = self.direct_candidates(row);
        let routed = direct.is_empty();
        let candidates = if routed {
            let routed = self.tag_routed_matchmaker_hunt_ids(hunt_id, row);
            if !routed.is_empty() {
                self.log(
                    Level::Info,
                    &format!(
                        "[HuntTagRouting] {hunt_id} resolved to {} matchmaker row(s) via tag query",
                        routed.len()
                    ),
                );
            }
            routed
        } else {
            direct
        };
        if candidates.is_empty() {
            return Err(format!(
                "player_hunts_table row '{hunt_id}' has no resolvable 1.12 MatchmakerHuntIDs"
            ));
        }
        let arena: Vec<String> = candidates.iter().filter(|c| arena_week(c).is_some()).cloned().collect();
        if !routed || arena.is_empty() {
            return Ok(self.pick(&candidates).to_owned());
        }
        let minutes = self.options.trials_rotation_minutes;
        let week = current_trials_week((self.options.now)(), minutes);
        let suffix = format!("_{week}");
        let featured: Vec<String> = arena.iter().filter(|c| c.ends_with(&suffix)).cloned().collect();
        if featured.is_empty() {
            self.log(
                Level::Warn,
                &format!(
                    "[TrialsRotation] no arena row matched featured week _{week}; falling back to a numbered arena \
                     candidate"
                ),
            );
            return Ok(self.pick(&arena).to_owned());
        }
        let chosen = self.pick(&featured).to_owned();
        self.log(
            Level::Info,
            &format!("[TrialsRotation] interval={minutes}m featured=_{week} -> {chosen}"),
        );
        Ok(chosen)
    }

    fn matchmaker_row(&self, id: &str) -> Result<&Value, String> {
        self.find_matchmaker_row(id)
            .ok_or_else(|| format!("matchmaker_hunts_table has no row for MatchmakerHuntId '{id}'"))
    }

    /// The Behemoth class path, or none (Hunting Grounds generate their encounters).
    pub fn behemoth_path_for(&self, matchmaker_hunt_id: &str) -> Result<Option<Value>, String> {
        let row = self.matchmaker_row(matchmaker_hunt_id)?;
        let path = row
            .get("SpecificBehemoth")
            .and_then(|b| b.get("BehemothAsset"))
            .and_then(|a| a.get("AssetPathName"))
            .filter(|v| !v.is_null() && v.as_str() != Some("None"))
            .filter(|v| !js::trim(&js::string(Some(v))).is_empty());
        Ok(path.cloned())
    }

    /// Every usable map of a matchmaker row, cut before the object name; an error names what is missing.
    pub fn usable_maps(&self, matchmaker_hunt_id: &str) -> Result<Vec<String>, String> {
        let row = self.matchmaker_row(matchmaker_hunt_id)?;
        let Some(Value::Array(list)) = row
            .get("MapList")
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        else {
            return Err(format!(
                "matchmaker_hunts_table row '{matchmaker_hunt_id}' has no MapList"
            ));
        };
        let maps: Vec<String> = list
            .iter()
            .filter_map(|entry| entry.get("MapAssetName").and_then(Value::as_str))
            .filter(|name| name.starts_with("/Game/") && name.contains('.'))
            .map(|name| name[..name.rfind('.').unwrap_or(name.len())].to_owned())
            .collect();
        if maps.is_empty() {
            return Err(format!(
                "matchmaker_hunts_table row '{matchmaker_hunt_id}' has no valid /Game/ map asset path"
            ));
        }
        Ok(maps)
    }

    pub fn map_path_for(&self, matchmaker_hunt_id: &str) -> Result<String, String> {
        let maps = self.usable_maps(matchmaker_hunt_id)?;
        Ok(self.pick(&maps).to_owned())
    }

    /// `{huntId, matchmakerHuntId, mapPath, behemothPath, expectedPlayerString}` for a hunt id and its party.
    pub fn resolve_manifest(&self, hunt_id: Option<&Value>, expected: Option<&Value>) -> Result<Value, String> {
        let party = match expected {
            Some(Value::Array(ids))
                if (1..=4).contains(&ids.len()) && ids.iter().all(|id| id.as_str().is_some_and(|s| !s.is_empty())) =>
            {
                ids
            }
            _ => return Err("ExpectedPlayers must contain between one and four account IDs".to_owned()),
        };
        let key = js::string(hunt_id);
        let matchmaker = self.matchmaker_hunt_id_for(&key)?;
        let behemoth = self.behemoth_path_for(&matchmaker)?;
        let map = self.map_path_for(&matchmaker)?;
        let mut out = Map::new();
        if let Some(id) = hunt_id {
            out.insert("huntId".into(), id.clone());
        }
        out.insert("matchmakerHuntId".into(), Value::String(matchmaker));
        out.insert("mapPath".into(), Value::String(map));
        out.insert(
            "behemothPath".into(),
            behemoth.unwrap_or_else(|| Value::from("NO_BEHEMOTH")),
        );
        out.insert(
            "expectedPlayerString".into(),
            Value::String(expected_player_args(party, &key)),
        );
        Ok(Value::Object(out))
    }

    /// The table audit (changes nothing).
    pub fn validate_tables(&self) -> TableAudit {
        let tables = &self.options.tables;
        let mut audit = TableAudit {
            player_rows: tables.player_rows.len(),
            matchmaker_rows: tables.matchmaker_sources.first().map_or(0, |(_, rows)| rows.len()),
            tables: tables.matchmaker_sources.len(),
            ..TableAudit::default()
        };
        for (id, row) in js_entries(&tables.player_rows) {
            let direct = match row.get("MatchmakerHuntIDs") {
                Some(Value::Array(handles)) if !handles.is_empty() => handles,
                _ => {
                    let routed = self.tag_routed_matchmaker_hunt_ids(id, row);
                    if routed.is_empty() {
                        continue;
                    }
                    audit.tag_routed_rows += 1;
                    let bad = routed.iter().filter(|r| self.usable_maps(r).is_err()).count();
                    if bad > 0 {
                        audit.problems.push(format!(
                            "{id} -> {bad}/{} tag-routed rows have no valid map",
                            routed.len()
                        ));
                    }
                    continue;
                }
            };
            audit.direct_rows += 1;
            for handle in direct {
                let name = handle.get("RowName");
                if name.and_then(Value::as_str) == Some("None") {
                    audit.tag_routed_refs += 1;
                    continue;
                }
                let text = js::string(name);
                if self.find_matchmaker_row(&text).is_none() {
                    audit.problems.push(format!("{id} -> missing MatchmakerHunt '{text}'"));
                } else if self.usable_maps(&text).is_err() {
                    audit.problems.push(format!("{id} -> {text} has no valid map"));
                }
            }
        }
        audit
    }

    /// The coverage audit, quiet.
    pub fn audit_coverage(&self) -> CoverageAudit {
        let was_quiet = self.quiet.swap(true, Ordering::SeqCst);
        let tables = &self.options.tables;
        let mut audit = CoverageAudit {
            total: tables.player_rows.len(),
            ..CoverageAudit::default()
        };
        let mut max_players = BTreeSet::new();
        for (id, row) in js_entries(&tables.player_rows) {
            let direct: Vec<String> = self
                .direct_candidates(row)
                .into_iter()
                .filter(|name| name != "None")
                .collect();
            let candidates = if direct.is_empty() {
                self.tag_routed_matchmaker_hunt_ids(id, row)
            } else {
                direct
            };
            if candidates.is_empty() {
                audit.unresolved.push(id.clone());
                continue;
            }
            for candidate in candidates {
                let Some(matchmaker) = self.find_matchmaker_row(&candidate) else {
                    continue;
                };
                let mode = match matchmaker.get("GameModeOverride") {
                    Some(Value::String(mode)) if !mode.is_empty() => mode.clone(),
                    _ => "(map/default)".to_owned(),
                };
                audit.game_modes.insert(mode);
                if let Some(max) = js::safe_integer(matchmaker.get("MaxPlayers")) {
                    max_players.insert(max);
                }
                audit.maps.extend(self.usable_maps(&candidate).unwrap_or_default());
                audit.referenced.insert(candidate);
            }
        }
        audit.unresolved.sort();
        audit.max_players = max_players.into_iter().collect();
        self.quiet.store(was_quiet, Ordering::SeqCst);
        audit
    }

    /// The audits' lines, at their levels (§9.7).
    pub fn audit_lines(&self) -> Vec<(Level, String)> {
        let table = self.validate_tables();
        let coverage = self.audit_coverage();
        let mut lines = vec![
            (
                Level::Info,
                format!(
                    "[HuntTableAudit] playerRows={} directRows={} tagRoutedRows={} tagRoutedRefs={} matchmakerRows={} \
                     (+{} aux tables) problems={}",
                    table.player_rows,
                    table.direct_rows,
                    table.tag_routed_rows,
                    table.tag_routed_refs,
                    table.matchmaker_rows,
                    table.tables.saturating_sub(1),
                    table.problems.len()
                ),
            ),
            (
                Level::Info,
                format!(
                    "[HuntLaunchCoverage] resolvable={}/{} referencedMatchmakerRows={} uniqueMaps={} gameModes={} \
                     unresolved={}",
                    coverage.resolvable(),
                    coverage.total,
                    coverage.referenced.len(),
                    coverage.maps.len(),
                    coverage.game_modes.len(),
                    coverage.unresolved.len()
                ),
            ),
        ];
        if !coverage.unresolved.is_empty() {
            lines.push((
                Level::Warn,
                format!(
                    "[HuntLaunchCoverage] unsupported player hunt rows: {}",
                    coverage.unresolved.join(", ")
                ),
            ));
        }
        let ids: Vec<String> = self.options.tables.player_rows.keys().cloned().collect();
        lines.extend(family_coverage(&ids).into_iter().map(|line| (Level::Info, line)));
        for problem in table.problems.iter().take(20) {
            lines.push((Level::Warn, format!("[HuntTableAudit] {problem}")));
        }
        if table.problems.len() > 20 {
            lines.push((
                Level::Warn,
                format!(
                    "[HuntTableAudit] {} additional problems omitted",
                    table.problems.len() - 20
                ),
            ));
        }
        lines
    }
}

fn js_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Families of ids ending in a capital letter (`Hunt_A` ... `Hunt_R`): a family ending early is the cue that newer
/// content was never exported.
pub fn family_coverage(ids: &[String]) -> Vec<String> {
    let mut groups: Vec<(String, Vec<char>)> = Vec::new();
    for id in ids {
        let Some(letter) = id.chars().last().filter(char::is_ascii_uppercase) else {
            continue;
        };
        let prefix = &id[..id.len() - 1];
        match groups.iter_mut().find(|(p, _)| p == prefix) {
            Some((_, letters)) => letters.push(letter),
            None => groups.push((prefix.to_owned(), vec![letter])),
        }
    }
    let text = |(prefix, letters): &(String, Vec<char>)| {
        std::iter::once(prefix.clone())
            .chain(letters.iter().map(char::to_string))
            .collect::<Vec<_>>()
            .join(",")
    };
    groups.sort_by(|a, b| js_cmp(&text(a), &text(b)));
    groups
        .into_iter()
        .filter(|(_, letters)| letters.len() >= 3)
        .map(|(prefix, mut letters)| {
            letters.sort_unstable();
            let (first, last) = (letters[0], letters[letters.len() - 1]);
            let n = letters.len();
            let gap = (last as usize - first as usize + 1) > n;
            let tail = if gap { " (NON-CONTIGUOUS)" } else { "" };
            format!("[HuntTableCoverage] {prefix}: {first}-{last} ({n}){tail}")
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MissingHunt {
    id: String,
    count: u64,
    first_ms: i64,
    last_ms: i64,
}

/// Absent player rows clients asked for (unbounded, one entry per id; §15).
#[derive(Debug, Default)]
pub struct MissingHuntRegistry(Mutex<Vec<MissingHunt>>);

impl MissingHuntRegistry {
    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<MissingHunt>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Counts a sighting; the first one only is logged at error.
    pub fn record(&self, id: &str, now_ms: i64) {
        let mut entries = self.entries();
        if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
            entry.count += 1;
            entry.last_ms = now_ms;
            return;
        }
        entries.push(MissingHunt {
            id: id.to_owned(),
            count: 1,
            first_ms: now_ms,
            last_ms: now_ms,
        });
        drop(entries);
        tracing::error!(
            "[HuntTableGap] Client requested HuntId '{id}' which is absent from player_hunts_table. This hunt CANNOT \
             start. The vendored tables predate this content — re-export with CatalogExporter EXPORT_HUNTS=1 and \
             import the new rows."
        );
    }

    fn sorted(&self) -> Vec<MissingHunt> {
        let mut entries = self.entries().clone();
        entries.sort_by_key(|e| std::cmp::Reverse(e.count));
        entries
    }

    /// The watchdog's warning, when there are entries.
    pub fn summary(&self) -> Option<String> {
        let entries = self.sorted();
        if entries.is_empty() {
            return None;
        }
        let list: Vec<String> = entries.iter().map(|e| format!("{} x{}", e.id, e.count)).collect();
        Some(format!(
            "[HuntTableGap] {} hunt id(s) requested but missing from player_hunts_table: {}",
            entries.len(),
            list.join(", ")
        ))
    }

    pub fn to_json(&self) -> Value {
        let iso = rustic_isles::season::to_iso_string;
        Value::Array(
            self.sorted()
                .into_iter()
                .map(|e| {
                    let mut entry = Map::new();
                    entry.insert("huntId".into(), Value::String(e.id));
                    entry.insert("count".into(), Value::from(e.count));
                    entry.insert("firstSeen".into(), Value::String(iso(e.first_ms)));
                    entry.insert("lastSeen".into(), Value::String(iso(e.last_ms)));
                    Value::Object(entry)
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn query(stream: Value, dictionary: &[&str]) -> Value {
        json!({ "QueryTokenStream": stream, "TagDictionary": dictionary.iter().map(|t| json!({"TagName": t})).collect::<Vec<_>>() })
    }

    #[test]
    fn tag_queries() {
        let easy = query(json!([0, 1, 1, 1, 0]), &["Hunt.Arena.Easy"]);
        let cases: [(&[&str], bool); 4] = [
            (&["Hunt.Arena.Easy"], true),
            (&["Hunt.Arena.Easy.Week"], true),
            (&["Hunt.Arena.Hard"], false),
            (&["Hunt.Arena.EasyX"], false),
        ];
        for (tags, want) in cases {
            assert_eq!(evaluate_tag_query(&easy, tags), want, "{tags:?}");
        }
        let all = query(json!([0, 1, 2, 2, 0, 1]), &["A", "B"]);
        assert!(!evaluate_tag_query(&all, &["A"]));
        assert!(evaluate_tag_query(&all, &["A", "B"]));
        let none = query(json!([0, 1, 3, 1, 0]), &["Hunt.Arena.Easy"]);
        assert!(evaluate_tag_query(&none, &["Hunt.Arena.Hard"]));
        assert!(!evaluate_tag_query(&none, &["Hunt.Arena.Easy"]));
        let nested = query(json!([0, 1, 5, 2, 1, 1, 0, 3, 1, 1]), &["A", "B"]);
        assert!(evaluate_tag_query(&nested, &["A"]));
        assert!(!evaluate_tag_query(&nested, &["A", "B"]));
        for bad in [
            query(json!([0, 0, 1, 1, 0]), &["A"]),
            query(json!([0, 1, 9, 1, 0]), &["A"]),
            query(json!([0, 1, 1, 1]), &["A"]),
            json!({}),
        ] {
            assert!(!evaluate_tag_query(&bad, &["A"]));
        }
        // A huge count stops at the end of the stream (§16 item 14)
        let huge = json!({"tokenStream": [0, 1, 4, 1e18, 1, 1, 0], "tagDictionary": ["A"]});
        assert!(!evaluate_tag_query(&huge, &["A"]));
        let huge_tags = json!({"tokenStream": [0, 1, 1, 1e18, 0], "tagDictionary": ["A"]});
        assert!(evaluate_tag_query(&huge_tags, &["A"]));
        let deep: Vec<Value> = [0, 1]
            .into_iter()
            .chain(std::iter::repeat_n(4, 200_000))
            .map(Value::from)
            .collect();
        assert!(!evaluate_tag_query(&json!({ "tokenStream": deep }), &["A"]));
    }

    #[test]
    fn trials() {
        let week = 10_080 * 60_000;
        assert_eq!(current_trials_week(0, 10_080), "001");
        assert_eq!(current_trials_week(week, 10_080), "002");
        assert_eq!(current_trials_week(181 * week, 10_080), "001");
        assert_eq!(current_trials_week(30 * 60_000, 30), "002");
        assert_eq!(parse_trials_rotation_minutes(None), (10_080, None));
        assert_eq!(parse_trials_rotation_minutes(Some("30")), (30, None));
        let warnings: Vec<String> = ["0", "600000", "soon"]
            .iter()
            .map(|raw| {
                let (minutes, warning) = parse_trials_rotation_minutes(Some(raw));
                assert_eq!(minutes, 10_080);
                warning.unwrap()
            })
            .collect();
        assert_eq!(
            warnings[0],
            "[TrialsRotation] invalid TRIALS_ROTATION_MINUTES='0'; using 10080"
        );
        assert_eq!(arena_week("Arena_MatchmakerHunt_Hard_042"), Some("042"));
        assert_eq!(arena_week("Arena_MatchmakerHunt_Test_042"), None);
    }

    #[test]
    fn families() {
        let ids: Vec<String> = [
            "Hunt_A", "Hunt_B", "Hunt_D", "Other_A", "Other_B", "Zed_A", "Zed_B", "Zed_C", "x",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(
            family_coverage(&ids),
            [
                "[HuntTableCoverage] Hunt_: A-D (3) (NON-CONTIGUOUS)",
                "[HuntTableCoverage] Zed_: A-C (3)"
            ]
        );
    }

    #[test]
    fn registry() {
        let registry = MissingHuntRegistry::default();
        assert_eq!(registry.summary(), None);
        registry.record("A", 0);
        registry.record("B", 1000);
        registry.record("B", 2000);
        assert_eq!(
            registry.summary().unwrap(),
            "[HuntTableGap] 2 hunt id(s) requested but missing from player_hunts_table: B x2, A x1"
        );
        assert_eq!(
            service_kit::jsjson::stringify(&registry.to_json()),
            r#"[{"huntId":"B","count":2,"firstSeen":"1970-01-01T00:00:01.000Z","lastSeen":"1970-01-01T00:00:02.000Z"},{"huntId":"A","count":1,"firstSeen":"1970-01-01T00:00:00.000Z","lastSeen":"1970-01-01T00:00:00.000Z"}]"#
        );
    }
}
