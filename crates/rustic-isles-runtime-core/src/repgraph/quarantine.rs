//! The controller channel quarantine (6): channels refused once stay refused until the sweep finds them stale.

use super::identity::chan;
use super::{Budgets, Game, TABLE, budgeted, kinds, lines, once_a_second};
use crate::offsets::{r9, repgraph as at};

/// Who queued an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// RG-4's cached channel on the wrong connection.
    #[default]
    ConnActorInfoMismatch,
    /// RG-4's remote controller.
    RemoteControllerPreChannel,
    /// RG-5.
    SetChannelActor,
    /// RG-6.
    ReplicateActorFinal,
}

impl Source {
    /// Its log text.
    pub fn text(self) -> &'static str {
        match self {
            Self::ConnActorInfoMismatch => "conn-actor-info-mismatch",
            Self::RemoteControllerPreChannel => "remote-controller-pre-channel",
            Self::SetChannelActor => "set-channel-actor",
            Self::ReplicateActorFinal => "replicate-actor-final",
        }
    }
}

/// One quarantined channel; `channel != 0` marks the entry used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Entry {
    /// The channel.
    pub channel: usize,
    /// The controller it carried or was to carry.
    pub actor: usize,
    /// The connection it must not serve.
    pub wrong_connection: usize,
    /// The controller's own connection, or 0.
    pub correct_connection: usize,
    /// The caller's RVA (RG-5 only).
    pub caller: usize,
    /// `GetTickCount64` at queueing.
    pub queued_ms: u64,
    /// Who queued it.
    pub source: Source,
}

/// What `queue` is asked to record.
#[derive(Debug, Clone, Copy)]
pub struct Queued {
    /// The channel.
    pub channel: usize,
    /// The controller.
    pub actor: usize,
    /// The connection it must not serve.
    pub wrong: usize,
    /// The controller's own connection, or 0.
    pub correct: usize,
    /// Who queues it.
    pub source: Source,
    /// The caller's RVA, or 0.
    pub caller: usize,
}

/// The table: 64 entries and a count kept current, so the empty table answers `contains` with one compare.
pub struct Quarantine {
    entries: [Entry; TABLE],
    used: usize,
    full_ms: Option<u64>,
}

impl Default for Quarantine {
    fn default() -> Self {
        Self::new()
    }
}

const EMPTY: Entry = Entry {
    channel: 0,
    actor: 0,
    wrong_connection: 0,
    correct_connection: 0,
    caller: 0,
    queued_ms: 0,
    source: Source::ConnActorInfoMismatch,
};

fn fill(field: &mut usize, value: usize) {
    if *field == 0 {
        *field = value;
    }
}

impl Quarantine {
    /// Empty.
    pub const fn new() -> Self {
        Self {
            entries: [EMPTY; TABLE],
            used: 0,
            full_ms: None,
        }
    }

    /// Used entries.
    pub fn len(&self) -> usize {
        self.used
    }

    /// Whether no entry is used.
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// The entry for `channel`.
    pub fn entry(&self, channel: usize) -> Option<&Entry> {
        if self.used == 0 || channel == 0 {
            return None;
        }
        self.entries.iter().find(|entry| entry.channel == channel)
    }

    /// `contains(channel)`.
    pub fn contains(&self, channel: usize) -> bool {
        self.entry(channel).is_some()
    }

    /// `queue(...)`: false for a null channel or a full table.
    pub fn queue(&mut self, game: &dyn Game, budgets: &Budgets, queued: Queued) -> bool {
        if queued.channel == 0 {
            return false;
        }
        if self.used != 0
            && let Some(entry) = self.entries.iter_mut().find(|entry| entry.channel == queued.channel)
        {
            fill(&mut entry.actor, queued.actor);
            fill(&mut entry.wrong_connection, queued.wrong);
            fill(&mut entry.correct_connection, queued.correct);
            fill(&mut entry.caller, queued.caller);
            return true;
        }
        let now = game.now_ms();
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.channel == 0) else {
            if once_a_second(&mut self.full_ms, now) {
                game.log(&lines::table_full(
                    queued.channel,
                    queued.actor,
                    queued.wrong,
                    queued.correct,
                ));
            }
            return false;
        };
        *entry = Entry {
            channel: queued.channel,
            actor: queued.actor,
            wrong_connection: queued.wrong,
            correct_connection: queued.correct,
            caller: queued.caller,
            queued_ms: now,
            source: queued.source,
        };
        self.used += 1;
        budgeted(game, &budgets.queued, kinds::QUEUED, || {
            lines::queued(
                queued.channel,
                queued.actor,
                queued.wrong,
                queued.correct,
                queued.source,
                queued.caller,
            )
        });
        true
    }

    /// The stale sweep (11.2 step 1): clears every entry `stale` says outlived its channel.
    pub fn sweep(&mut self, game: &dyn Game, budgets: &Budgets) {
        if self.used == 0 {
            return;
        }
        for entry in self.entries.iter_mut().filter(|entry| entry.channel != 0) {
            let Some((current_connection, current_actor)) = stale(game, entry) else {
                continue;
            };
            let gone = *entry;
            *entry = EMPTY;
            self.used -= 1;
            budgeted(game, &budgets.stale, kinds::STALE, || {
                lines::dropped_stale(
                    gone.channel,
                    gone.actor,
                    current_actor,
                    gone.wrong_connection,
                    current_connection,
                )
            });
        }
    }
}

/// `stale(entry)`: `Some((current connection, current actor))` when the entry no longer describes its channel.
pub fn stale(game: &dyn Game, entry: &Entry) -> Option<(usize, usize)> {
    let live = chan(game, entry.channel);
    let (connection, actor) = if live {
        (
            game.ptr(entry.channel + at::CHANNEL_CONNECTION),
            game.ptr(entry.channel + r9::CHANNEL_ACTOR),
        )
    } else {
        (0, 0)
    };
    let moved = entry.wrong_connection != 0 && connection != entry.wrong_connection;
    let rebound = actor != 0 && entry.actor != 0 && actor != entry.actor;
    (!live || moved || rebound).then_some((connection, actor))
}
