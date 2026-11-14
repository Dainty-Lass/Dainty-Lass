//! Player data and loading (spec-runtime-r7), the pure part: the retry budget, the restart gate, the progression
//! bookkeeping, the possession timing, the idle-trigger and hub rules, the hunt id and the log texts. The DLL reads
//! the game and feeds these values.

use std::collections::HashSet;

/// Retry slots (6.3).
pub const RETRY_SLOTS: usize = 32;
/// A slot this stale is reused before the oldest (6.3).
pub const RETRY_STALE_MS: u64 = 300_000;
/// Retries per manager and controller pair (6.3).
pub const RETRY_BUDGET: u32 = 1;
/// The most load manager entries read (6.1).
pub const MAX_ENTRIES: i32 = 128;
/// Restart gate ring entries (7.2).
pub const GATE_SLOTS: usize = 32;
/// The gate's fallback (7.2).
pub const GATE_FALLBACK_MS: u64 = 90_000;
/// Distinct parked components remembered before the walk goes over every object (8.2).
pub const MAX_CANDIDATES: usize = 1024;
/// Completed components remembered before the set is emptied (8.2).
pub const MAX_COMPLETED: usize = 4096;
/// Stranded components resumed from the predicate in one process (8.3).
pub const MAX_RESUMES: u32 = 8;
/// The resume poll's interval (8.4).
pub const POLL_INTERVAL_MS: u64 = 200;
/// The resume poll gives the waiters up to the retry after this (8.4).
pub const POLL_GIVE_UP_MS: u64 = 90_000;
/// A possession settles this long before it is stable (4.4).
pub const POSSESSION_SETTLE_MS: u64 = 3_000;
/// The same controller and pawn for this long (4.4).
pub const STABLE_CHAIN_MS: u64 = 750;
/// The local controller is never forced before this; the client's own readiness takes about 1.5 s (4.2).
pub const CONTROLLER_FORCE_MS: u64 = 20_000;
/// Forced-answer lines per kind per process (11).
pub const FORCED_LOG_BUDGET: u32 = 20;

/// `0x` and 16 lowercase hex digits (11).
pub fn ptr(value: usize) -> String {
    format!("0x{value:016x}")
}

// --- The hunt id (5.3) ---

/// The launch's `accountId:hunterId, ...`: the text after its first `:` up to the next `,`, verbatim.
pub fn shared_hunt_id(expected_players: Option<&str>) -> String {
    let Some(list) = expected_players else {
        return String::new();
    };
    match list.split_once(':') {
        Some((_, rest)) => rest.split(',').next().unwrap_or_default().to_owned(),
        None => String::new(),
    }
}

// --- The restart gate (7) ---

/// A hub game mode: Ramsgate's `BPGM_City_C`, the Training Grounds' `BPGM_ArchonTrainingGrounds_C` (7.1).
pub fn is_hub_class(name: &str) -> bool {
    name.contains("City") || name.contains("TrainingGrounds")
}

#[derive(Debug, Clone, Copy, Default)]
struct GateEntry {
    controller: usize,
    first_held: Option<u64>,
    loaded: bool,
}

/// What `should_hold` decided, with the line it logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// The data loaded (or the fallback already ran): restart.
    Allow,
    /// Held for the first time (logs).
    First,
    /// Still held.
    Held,
    /// 90 s passed: allowed from now on (logs).
    GaveUp,
}

impl Hold {
    /// Whether the restart is refused.
    pub fn holds(self) -> bool {
        matches!(self, Hold::First | Hold::Held)
    }
}

/// The ring of 32 controllers (7.2).
#[derive(Debug, Default)]
pub struct RestartGate {
    entries: [GateEntry; GATE_SLOTS],
    cursor: usize,
}

impl RestartGate {
    /// An empty ring.
    pub const fn new() -> Self {
        Self {
            entries: [GateEntry {
                controller: 0,
                first_held: None,
                loaded: false,
            }; GATE_SLOTS],
            cursor: 0,
        }
    }

    fn entry_for(&mut self, controller: usize) -> &mut GateEntry {
        let index = match self.entries.iter().position(|entry| entry.controller == controller) {
            Some(index) => index,
            None => {
                let index = self.cursor;
                self.cursor = (self.cursor + 1) % GATE_SLOTS;
                self.entries[index] = GateEntry {
                    controller,
                    first_held: None,
                    loaded: false,
                };
                index
            }
        };
        &mut self.entries[index]
    }

    /// LD-2: the controller's data loaded. A null controller is ignored.
    pub fn mark_loaded(&mut self, controller: usize) {
        if controller != 0 {
            self.entry_for(controller).loaded = true;
        }
    }

    /// Whether the controller's data loaded, as far as the gate knows (R14 reads it).
    pub fn loaded(&self, controller: usize) -> bool {
        controller != 0
            && self
                .entries
                .iter()
                .any(|entry| entry.controller == controller && entry.loaded)
    }

    /// `should_hold(controller, now)`.
    pub fn should_hold(&mut self, controller: usize, now: u64) -> Hold {
        if controller == 0 {
            return Hold::Allow;
        }
        let entry = self.entry_for(controller);
        if entry.loaded {
            return Hold::Allow;
        }
        let Some(first) = entry.first_held else {
            entry.first_held = Some(now);
            return Hold::First;
        };
        if now.saturating_sub(first) >= GATE_FALLBACK_MS {
            entry.loaded = true;
            return Hold::GaveUp;
        }
        Hold::Held
    }
}

// --- The retry budget (6.3) ---

#[derive(Debug, Clone, Copy)]
struct RetrySlot {
    manager: usize,
    controller: usize,
    last_failure: u64,
    retries: u32,
}

/// 32 slots keyed by manager and controller, so a recycled address starts with a fresh budget.
#[derive(Debug, Default)]
pub struct RetryTable {
    slots: [Option<RetrySlot>; RETRY_SLOTS],
}

impl RetryTable {
    /// An empty table.
    pub const fn new() -> Self {
        Self {
            slots: [None; RETRY_SLOTS],
        }
    }

    /// `claim`: the retry number when granted, `None` when refused.
    pub fn claim(&mut self, manager: usize, controller: usize, now: u64) -> Option<u32> {
        if manager == 0 || controller == 0 {
            return None;
        }
        let found = self
            .slots
            .iter()
            .position(|slot| slot.is_some_and(|slot| slot.manager == manager && slot.controller == controller));
        let index = match found {
            Some(index) => index,
            None => {
                let index = self
                    .slots
                    .iter()
                    .position(Option::is_none)
                    .or_else(|| {
                        self.slots.iter().position(|slot| {
                            slot.is_some_and(|slot| now.saturating_sub(slot.last_failure) >= RETRY_STALE_MS)
                        })
                    })
                    .or_else(|| {
                        (0..RETRY_SLOTS).min_by_key(|&index| self.slots[index].map_or(0, |slot| slot.last_failure))
                    })?;
                self.slots[index] = Some(RetrySlot {
                    manager,
                    controller,
                    last_failure: now,
                    retries: 0,
                });
                index
            }
        };
        let slot = self.slots[index].as_mut()?;
        slot.last_failure = now;
        if slot.retries < RETRY_BUDGET {
            slot.retries += 1;
            Some(slot.retries)
        } else {
            None
        }
    }

    /// Empties every slot matching either (a null argument matches nothing); how many were emptied.
    pub fn clear(&mut self, manager: usize, controller: usize) -> usize {
        let mut cleared = 0;
        for slot in &mut self.slots {
            if slot.is_some_and(|slot| {
                (manager != 0 && slot.manager == manager) || (controller != 0 && slot.controller == controller)
            }) {
                *slot = None;
                cleared += 1;
            }
        }
        cleared
    }
}

// --- The load failure's snapshot and rearm (6.2, 6.4) ---

/// One load manager entry as read: its resolved owner, its loader and the loader's two bytes (`None` unreadable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoaderEntry {
    /// The weak owner, resolved by the game.
    pub owner: usize,
    /// The loader interface.
    pub loader: usize,
    /// `bLoaded`.
    pub loaded: Option<u8>,
    /// `bLoading`.
    pub loading: Option<u8>,
}

/// Entries whose `bLoaded` is 0.
pub fn incomplete(entries: &[LoaderEntry]) -> usize {
    entries.iter().filter(|entry| entry.loaded == Some(0)).count()
}

/// Recoverable: the controller passed its checks, and every entry has an owner, a loader and two bytes of 0 or 1.
pub fn recoverable(controller_ok: bool, entries: &[LoaderEntry]) -> bool {
    let bit = |value: Option<u8>| matches!(value, Some(0 | 1));
    controller_ok
        && entries
            .iter()
            .all(|entry| entry.owner != 0 && entry.loader != 0 && bit(entry.loaded) && bit(entry.loading))
}

/// The entries whose `bLoading` is cleared, or `None` when the rearm fails (a null loader, nothing to rearm, or a
/// count other than `incomplete`).
pub fn rearm_targets(entries: &[LoaderEntry], incomplete: usize) -> Option<Vec<usize>> {
    if entries.iter().any(|entry| entry.loader == 0) {
        return None;
    }
    let targets: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.loaded == Some(0))
        .map(|(index, _)| index)
        .collect();
    (!targets.is_empty() && targets.len() == incomplete).then_some(targets)
}

// --- Progression (8.2 to 8.4) ---

/// Parked and completed progression components (8.2). The DLL keeps one under a lock.
#[derive(Debug, Default)]
pub struct Progression {
    waiters: Vec<usize>,
    wait_since: Option<u64>,
    candidates: Vec<usize>,
    overflow: bool,
    completed: HashSet<usize>,
}

impl Progression {
    /// Empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// LD-6: `CompleteLoadInternal` ran for `component`.
    pub fn complete(&mut self, component: usize) {
        if component == 0 {
            return;
        }
        if self.completed.len() >= MAX_COMPLETED && !self.completed.contains(&component) {
            self.completed.clear();
        }
        self.completed.insert(component);
        self.waiters.retain(|&waiter| waiter != component);
        self.candidates.retain(|&candidate| candidate != component);
    }

    /// LD-7: `component` parked; returns the waiter count.
    pub fn park(&mut self, component: usize, now: u64) -> usize {
        self.completed.remove(&component);
        if self.waiters.is_empty() {
            self.wait_since = Some(now);
        }
        if !self.waiters.contains(&component) {
            self.waiters.push(component);
        }
        if !self.candidates.contains(&component) {
            if self.candidates.len() < MAX_CANDIDATES {
                self.candidates.push(component);
            } else {
                self.overflow = true;
            }
        }
        self.waiters.len()
    }

    /// Takes every waiter, with when the first of them parked.
    pub fn take_waiters(&mut self) -> (Vec<usize>, Option<u64>) {
        (std::mem::take(&mut self.waiters), self.wait_since.take())
    }

    /// Empties the waiters; how many there were.
    pub fn clear_waiters(&mut self) -> usize {
        self.wait_since = None;
        std::mem::take(&mut self.waiters).len()
    }

    /// The waiter count.
    pub fn waiter_count(&self) -> usize {
        self.waiters.len()
    }

    /// When the first current waiter parked.
    pub fn wait_since(&self) -> Option<u64> {
        self.wait_since
    }

    /// Whether `component` completed since it last parked.
    pub fn is_completed(&self, component: usize) -> bool {
        self.completed.contains(&component)
    }

    /// A copy of the candidates.
    pub fn candidates(&self) -> Vec<usize> {
        self.candidates.clone()
    }

    /// Set once more than 1024 distinct components parked: the stranded walk covers every object.
    pub fn overflow(&self) -> bool {
        self.overflow
    }

    /// Forgets a candidate that is gone or not a component.
    pub fn remove_candidate(&mut self, component: usize) {
        self.candidates.retain(|&candidate| candidate != component);
    }
}

/// The stranded-component resumes left in the process (8.3).
#[derive(Debug, Default)]
pub struct ResumeBudget {
    used: u32,
}

impl ResumeBudget {
    /// None used.
    pub const fn new() -> Self {
        Self { used: 0 }
    }

    /// Whether any is left.
    pub fn left(&self) -> bool {
        self.used < MAX_RESUMES
    }

    /// Counts one; false once eight were counted.
    pub fn take(&mut self) -> bool {
        if !self.left() {
            return false;
        }
        self.used += 1;
        true
    }
}

/// The resume poll's throttle (8.4).
#[derive(Debug, Default)]
pub struct Poll {
    last: Option<u64>,
}

impl Poll {
    /// Never checked.
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Whether the poll checks now: at most every 200 ms from the last check.
    pub fn due(&mut self, now: u64) -> bool {
        if self
            .last
            .is_some_and(|last| now.saturating_sub(last) < POLL_INTERVAL_MS)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// The poll leaves the waiters to the retry: more than 90 s since the first parked.
pub fn poll_gives_up(wait_since: Option<u64>, now: u64) -> bool {
    wait_since.is_some_and(|since| now.saturating_sub(since) > POLL_GIVE_UP_MS)
}

// --- The client's possession (4.2 to 4.4) ---

/// The last `ClientRestart` and the stability tracking (4.4).
#[derive(Debug, Default)]
pub struct Possession {
    controller: usize,
    pawn: usize,
    world: usize,
    began: Option<u64>,
    tracked: Option<(usize, usize)>,
    tracked_since: u64,
    generation: u64,
}

impl Possession {
    /// Nothing captured.
    pub const fn new() -> Self {
        Self {
            controller: 0,
            pawn: 0,
            world: 0,
            began: None,
            tracked: None,
            tracked_since: 0,
            generation: 0,
        }
    }

    /// A `ClientRestart` ran on `controller` with `pawn` (0 when unreadable) in `world`.
    pub fn capture(&mut self, controller: usize, pawn: usize, world: usize, now: u64) {
        if controller != self.controller || pawn != self.pawn {
            self.began = Some(now);
            self.tracked = None;
            self.generation += 1;
        }
        self.controller = controller;
        self.pawn = pawn;
        self.world = world;
    }

    /// Before any use: a different world forgets the pair (its controller is freed); the time stays.
    pub fn check_world(&mut self, world: usize) {
        if (self.controller != 0 || self.pawn != 0) && world != self.world {
            self.controller = 0;
            self.pawn = 0;
            self.tracked = None;
        }
    }

    /// The captured controller (0 when none).
    pub fn controller(&self) -> usize {
        self.controller
    }

    /// The captured pawn (0 when none).
    pub fn pawn(&self) -> usize {
        self.pawn
    }

    /// Counts new possessions: a repeated `ClientRestart` for the same pair keeps it (R14 3.1).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// When the captured possession began (`GetTickCount64`).
    pub fn began(&self) -> Option<u64> {
        self.began
    }

    /// Milliseconds since the captured possession began.
    pub fn since_began(&self, now: u64) -> Option<u64> {
        self.began.map(|began| now.saturating_sub(began))
    }

    /// Resets the stability tracking (a failed read).
    pub fn reset_tracking(&mut self) {
        self.tracked = None;
    }

    /// `possession_stable`'s decision over the pawn the DLL read for `pc` (`None` when a read in steps 2 and 3
    /// failed). A failed step resets the tracking.
    pub fn stable(&mut self, pc: usize, pawn: Option<usize>, now: u64) -> Option<usize> {
        let pawn = pawn.filter(|&pawn| pawn != 0 && (self.pawn == 0 || pawn == self.pawn));
        let (true, Some(pawn)) = (self.controller != 0 && pc == self.controller, pawn) else {
            self.tracked = None;
            return None;
        };
        if self.tracked != Some((pc, pawn)) {
            self.tracked = Some((pc, pawn));
            self.tracked_since = now;
        }
        let settled = self.since_began(now)? >= POSSESSION_SETTLE_MS;
        let chained = now.saturating_sub(self.tracked_since) >= STABLE_CHAIN_MS;
        (settled && chained).then_some(pawn)
    }
}

/// 4.2 steps 4 to 6 over values: a stable possession, a weapon, and 20 s since the possession began.
pub fn force_controller(stable: bool, weapon: usize, since_ms: u64) -> bool {
    stable && weapon != 0 && since_ms >= CONTROLLER_FORCE_MS
}

/// The arrival trigger's six signs of a real cinematic (4.3 step 3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TriggerSigns {
    /// `SlayerActors.Num`.
    pub slayer_actors: i32,
    /// `IntroAnimSequenceRefs.Num`.
    pub intro_refs: i32,
    /// `bClientStarted`.
    pub client_started: u8,
    /// `LoadingState`.
    pub loading_state: u8,
    /// `PrimaryCineActor`.
    pub primary_cine_actor: usize,
    /// `IslandApproachLevelSequenceActor`.
    pub approach_actor: usize,
}

impl TriggerSigns {
    /// No sign of a real cinematic.
    pub fn idle(&self) -> bool {
        *self == Self::default()
    }
}

// --- Log texts (11) ---

/// The log lines, as sections 4 to 8 give them.
pub mod lines {
    use super::ptr;

    /// 4.1.
    pub fn server_forced(name: Option<&str>) -> String {
        format!("[Readiness] server: forced ready obj={}", name.unwrap_or("null"))
    }

    /// 4.2.
    pub fn client_forced(name: Option<&str>, since_ms: u64) -> String {
        format!(
            "[Readiness] client: forced ready local controller={} sinceClientRestartMs={since_ms}",
            name.unwrap_or("null")
        )
    }

    /// 4.3 step 1.
    pub fn trigger_release(on: bool) -> &'static str {
        if on {
            "[Readiness] idle arrival trigger release on (default)"
        } else {
            "[Readiness] idle arrival trigger release off (KEEP_ARRIVAL_TRIGGER_WAIT.flag)"
        }
    }

    /// 4.3.
    pub fn trigger_released(since_ms: u64) -> String {
        format!(
            "[Readiness] client: released the idle arrival trigger sinceClientRestartMs={since_ms} (its own timeout is 18 s)"
        )
    }

    /// 5.1 step 1.
    pub fn completed_after_retry(success: bool) -> String {
        format!(
            "[PlayerData] load completed after a retry success={}",
            u8::from(success)
        )
    }

    /// 5.2 step 3.
    pub const HUNT_ID_NONE: &str = "[PlayerData] hunt id empty and the launch names none; not set";
    /// 5.2 step 4.
    pub const HUNT_ID_FUNCTION_MISSING: &str =
        "[PlayerData] HuntSystemComponent.DebugSetPlayerHuntID not found; hunt ids are not backfilled";

    /// 5.2 step 6.
    pub fn hunt_id_set(hunt_id: &str) -> String {
        format!("[PlayerData] hunt id was empty: set '{hunt_id}'")
    }

    /// 6.1 step 3.
    pub const DROPPED_NO_NET_ID: &str =
        "[PlayerData] load failure dropped: its owner has no net id; the server keeps listening";
    /// 6.1 step 6.
    pub const RETRY_DISABLED: &str = "[PlayerData] retry disabled by flag; the game handles the failure";

    /// 6.1 step 7.
    pub fn not_recoverable(incomplete: usize) -> String {
        format!("[PlayerData] load failure not recoverable incomplete={incomplete}; the game handles it")
    }

    /// 6.1 step 8.
    pub const RETRY_USED: &str = "[PlayerData] retry already used; the game handles the failure";
    /// 6.1 step 9.
    pub const REARM_FAILED: &str = "[PlayerData] retry could not rearm; the game handles the failure";

    /// 6.1 step 10.
    pub fn retried(retry: u32, rearmed: usize) -> String {
        format!("[PlayerData] retry {retry}/{} rearmed={rearmed}", super::RETRY_BUDGET)
    }

    /// 7.2, a first hold.
    pub fn restart_held(controller: usize) -> String {
        format!(
            "[PlayerData] restart held until the player's data loads: controller={}",
            ptr(controller)
        )
    }

    /// 7.2, the fallback.
    pub fn restart_released(controller: usize) -> String {
        format!(
            "[PlayerData] data still not loaded after 90 s; restart allowed: controller={}",
            ptr(controller)
        )
    }

    /// 8.2, LD-6's first call.
    pub const COMPLETE_RUNS: &str = "[Progression] CompleteLoadInternal runs";

    /// 8.2, LD-7.
    pub fn parked(component: usize, waiters: usize) -> String {
        format!("[Progression] parked component={} waiters={waiters}", ptr(component))
    }

    /// 8.3 step 2.
    pub fn stranded(component: usize) -> String {
        format!(
            "[Progression] stranded component={}: calling CompleteLoadInternal",
            ptr(component)
        )
    }

    /// 8.3 step 4.
    pub fn querying(instance: usize) -> String {
        format!(
            "[Progression] querying the hunt pass season data instance={}",
            ptr(instance)
        )
    }

    /// 8.3 step 4.
    pub const QUERY_RETURNED: &str = "[Progression] query returned";
    /// 8.3 step 4.
    pub const QUERY_MISSING: &str =
        "[Progression] ArchonGameInstance.QueryCurrentHuntPassSeasonData not found; the configuration is not queried";

    /// 8.3 step 4, when the parameter guard refuses the call (R17 3.3).
    pub fn query_skipped(size: Option<u16>, buffer_len: usize) -> String {
        let size = size.map_or_else(|| "unreadable".to_owned(), |size| size.to_string());
        format!(
            "[Progression] ArchonGameInstance.QueryCurrentHuntPassSeasonData: parameter size {size} exceeds {buffer_len}, call skipped"
        )
    }

    /// 8.3 step 5.
    pub fn configuration(loaded: bool) -> &'static str {
        if loaded {
            "[Progression] configuration loaded"
        } else {
            "[Progression] configuration not loaded yet: progression reads will park"
        }
    }

    /// 8.4 step 2.
    pub fn resume_mode(on: bool) -> &'static str {
        if on {
            "[Progression] resume of parked components on (no phantom local player)"
        } else {
            "[Progression] resume of parked components off (this server has a phantom local player, whose own load covers it)"
        }
    }

    /// 8.4 step 6.
    pub fn poll_gave_up(waiters: usize) -> String {
        format!(
            "[Progression] configuration still not loaded after 90 s; leaving {waiters} parked component(s) to the load-failure retry"
        )
    }

    /// 8.4 step 7.
    pub fn already_completed(component: usize) -> String {
        format!("[Progression] component={} already completed", ptr(component))
    }

    /// 8.4 step 7.
    pub fn gone(component: usize) -> String {
        format!("[Progression] component={} is gone; skipped", ptr(component))
    }

    /// 8.4 step 7.
    pub fn resumed(after_ms: u64, component: usize) -> String {
        format!(
            "[Progression] configuration loaded {after_ms} ms after the first park: CompleteLoadInternal component={}",
            ptr(component)
        )
    }

    /// RT-3: an id R7 owns and does not install (3.5).
    pub const SKIPPED_LOGGING_ONLY: &str = "skipped: logging only (RT-3)";

    /// A wrapper's panic, once per id (3.5).
    pub fn panicked(id: &str) -> String {
        format!("[Panic] {id} handler panicked; the call goes on to the chain")
    }
}

#[cfg(test)]
mod tests;
