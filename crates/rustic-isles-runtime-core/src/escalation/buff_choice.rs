//! The escalation buff-choice spawn (R18 4): SP-2 notes the round-finished transition, SP-3 later calls the game's
//! own `SpawnPlayerBuffChoicesForFinishedRound` on the mode component, once per component and round.

use super::pointer;
use crate::memory::Memory;
use crate::names::{Kind, Matcher};
use crate::offsets::r18;

/// SP-2's matcher (RT-18 for R18: catches exactly one UFunction in 1.14.7).
pub const MATCHER: Matcher = &[(Kind::Contains, "EscalationGameModeComponent.AuthOnGameStateChanged")];
/// Both handlers' order on their chains (R18 call 7).
pub const ORDER: i32 = 1800;
/// Its A/B flag: no request is queued.
pub const NO_SPAWN_FLAG: &str = "ESCALATION_NO_BUFF_CHOICE_SPAWN";
/// The class that declares the spawn function.
pub const DECLARING_CLASS: &str = "EscalationGameModeComponent";
/// The game's spawn function (BlueprintCallable, no parameters).
pub const SPAWN_FUNCTION: &str = "SpawnPlayerBuffChoicesForFinishedRound";
/// The SP-3 body runs on every 64th tick (R18 call 9).
pub const CADENCE_MASK: u64 = 0x3F;

/// One request to spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// The mode component.
    pub component: usize,
    /// Its state component's `CurrentRound`, `i32::MIN` when unknown.
    pub round: i32,
}

/// The process's state (one value, behind a mutex in the DLL).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BuffChoice {
    pending: Option<Request>,
    last: Option<(usize, i32)>,
}

impl BuffChoice {
    /// An empty state.
    pub const fn new() -> Self {
        Self {
            pending: None,
            last: None,
        }
    }

    /// Queues a request unless that component and round were already dispatched; a newer note replaces an
    /// undispatched one. The `queued` line when it queued.
    pub fn note(&mut self, request: Request) -> Option<String> {
        if self.last == Some((request.component, request.round)) {
            return None;
        }
        self.pending = Some(request);
        Some(queued_line(request))
    }

    /// Takes the pending request, clearing it.
    pub fn take(&mut self) -> Option<Request> {
        self.pending.take()
    }

    /// Records the component and round last dispatched.
    pub fn mark_dispatched(&mut self, request: Request) {
        self.last = Some((request.component, request.round));
    }
}

/// Steps 1 to 3 of R18 4.2 past the flag: the call is the round-finished transition (`EventEnding`, `Begin`), and
/// the request it makes. `None` for anything else or anything unreadable.
pub fn transition(memory: &dyn Memory, object: usize, params: usize) -> Option<Request> {
    if object == 0 || params == 0 {
        return None;
    }
    // The params block is the caller's stack buffer: the region rule only, no alignment rule
    let mut state = [0u8; 2];
    if !memory.read(params, &mut state) {
        return None;
    }
    if state != [r18::ESC_STATE_EVENT_ENDING, r18::TRANSITION_BEGIN] {
        return None;
    }
    if !memory.readable(object, r18::ESC_MODE_READ) {
        return None;
    }
    let state_component = memory.ptr_at(object.checked_add(r18::ESC_MODE_STATE_COMPONENT)?)?;
    let round = if state_component != 0 && memory.readable(state_component, r18::ESC_STATE_READ) {
        state_component
            .checked_add(r18::ESC_STATE_CURRENT_ROUND)
            .and_then(|at| memory.i32_at(at))
            .unwrap_or(i32::MIN)
    } else {
        i32::MIN
    };
    Some(Request {
        component: object,
        round,
    })
}

/// Whether the SP-3 body runs on the `count`th call (1-based).
pub fn cadence_due(count: u64) -> bool {
    count & CADENCE_MASK == 0
}

/// `queued`.
pub fn queued_line(request: Request) -> String {
    format!(
        "[EscalationBuffChoiceSpawn] queued mode={} round={} trigger=EventEnding(7) entry",
        pointer(request.component),
        request.round
    )
}

/// The component is gone by the tick.
pub fn skipped_line(request: Request) -> String {
    format!(
        "[EscalationBuffChoiceSpawn] skipped: mode component no longer live mode={} round={}",
        pointer(request.component),
        request.round
    )
}

/// The function was not found in the component's class chain.
pub fn not_found_line(request: Request) -> String {
    format!(
        "[EscalationBuffChoiceSpawn] {SPAWN_FUNCTION} UFunction not found on mode={}",
        pointer(request.component)
    )
}

/// Before the call.
pub fn dispatching_line(request: Request) -> String {
    format!(
        "[EscalationBuffChoiceSpawn] dispatching mode={} round={}",
        pointer(request.component),
        request.round
    )
}

/// After the call; two spaces so it aligns with `dispatching`.
pub fn dispatched_line(request: Request) -> String {
    format!(
        "[EscalationBuffChoiceSpawn] dispatched  mode={} round={}",
        pointer(request.component),
        request.round
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::BufferMemory;
    use crate::names::matches;

    const BASE: usize = 0x10_0000;
    const MODE: usize = BASE;
    const STATE: usize = BASE + 0x100;
    const PARAMS: usize = BASE + 0x300;

    fn world(params: [u8; 2], state: usize, round: i32) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x302];
        bytes[0xB8..0xC0].copy_from_slice(&(state as u64).to_le_bytes());
        bytes[0x100 + 0x134..0x100 + 0x138].copy_from_slice(&round.to_le_bytes());
        bytes[0x300..0x302].copy_from_slice(&params);
        bytes
    }

    fn read(bytes: &[u8]) -> Option<Request> {
        transition(&BufferMemory { base: BASE, bytes }, MODE, PARAMS)
    }

    #[test]
    fn transitions() {
        assert_eq!(
            read(&world([7, 0], STATE, 3)),
            Some(Request {
                component: MODE,
                round: 3
            })
        );
        for params in [[7, 1], [6, 0], [8, 0]] {
            assert_eq!(read(&world(params, STATE, 3)), None, "{params:?}");
        }
        // Params shorter than 2 bytes: the block's last byte is the end of memory
        let bytes = world([7, 0], STATE, 3);
        assert_eq!(
            transition(
                &BufferMemory {
                    base: BASE,
                    bytes: &bytes
                },
                MODE,
                BASE + 0x301
            ),
            None
        );
        assert_eq!(
            transition(
                &BufferMemory {
                    base: BASE,
                    bytes: &bytes
                },
                0,
                PARAMS
            ),
            None
        );
        assert_eq!(
            transition(
                &BufferMemory {
                    base: BASE,
                    bytes: &bytes
                },
                MODE,
                0
            ),
            None
        );
        assert_eq!(
            read(&world([7, 0], 0, 3)).map(|r| r.round),
            Some(i32::MIN),
            "null state"
        );
        // The state component's last bytes run off memory
        assert_eq!(
            read(&world([7, 0], BASE + 0x200, 3)).map(|r| r.round),
            Some(i32::MIN),
            "short state"
        );
        // The mode component itself short of 0xC0
        let bytes = world([7, 0], STATE, 3);
        assert_eq!(
            transition(
                &BufferMemory {
                    base: BASE,
                    bytes: &bytes
                },
                BASE + 0x250,
                PARAMS
            ),
            None
        );
    }

    #[test]
    fn state_machine() {
        let mut state = BuffChoice::new();
        let a = Request {
            component: 10,
            round: 1,
        };
        let b = Request {
            component: 20,
            round: 1,
        };
        assert!(state.note(a).is_some());
        assert!(state.note(b).is_some());
        assert_eq!(state.take(), Some(b), "a second note replaces the first");
        assert_eq!(state.take(), None, "take clears");
        state.mark_dispatched(b);
        assert_eq!(state.note(b), None, "already served");
        assert_eq!(state.take(), None);
        let next = Request {
            component: 20,
            round: 2,
        };
        assert!(state.note(next).is_some());
        assert_eq!(state.take(), Some(next));
    }

    #[test]
    fn cadence() {
        assert!((1..64).all(|count| !cadence_due(count)));
        assert!(cadence_due(64) && cadence_due(128));
        assert!(!cadence_due(65));
    }

    #[test]
    fn matcher() {
        assert!(matches(
            MATCHER,
            Some("Function Archon.EscalationGameModeComponent.AuthOnGameStateChanged")
        ));
        assert!(!matches(
            MATCHER,
            Some("Function BP_Mode.BP_Mode_C.AuthOnGameStateChanged")
        ));
    }

    #[test]
    fn texts() {
        let known = Request {
            component: 2_199_023_255_552,
            round: 0,
        };
        let unknown = Request {
            component: 0,
            round: i32::MIN,
        };
        assert_eq!(
            queued_line(known),
            "[EscalationBuffChoiceSpawn] queued mode=2199023255552 round=0 trigger=EventEnding(7) entry"
        );
        assert_eq!(
            queued_line(unknown),
            "[EscalationBuffChoiceSpawn] queued mode=null round=-2147483648 trigger=EventEnding(7) entry"
        );
        assert_eq!(
            skipped_line(known),
            "[EscalationBuffChoiceSpawn] skipped: mode component no longer live mode=2199023255552 round=0"
        );
        assert_eq!(
            not_found_line(known),
            "[EscalationBuffChoiceSpawn] SpawnPlayerBuffChoicesForFinishedRound UFunction not found on \
             mode=2199023255552"
        );
        assert_eq!(
            dispatching_line(known),
            "[EscalationBuffChoiceSpawn] dispatching mode=2199023255552 round=0"
        );
        assert_eq!(
            dispatched_line(known),
            "[EscalationBuffChoiceSpawn] dispatched  mode=2199023255552 round=0"
        );
    }
}
