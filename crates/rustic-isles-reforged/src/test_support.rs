// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Test support: a fixed clock, a recording matchmaking presence, a tester funding fake and the operator signer.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use module_api::BoxFuture;
use rustic_isles::Shared;

use crate::admin::tester_max::{RankRequirement, StackOutcome, TesterFunding};
use crate::state::MatchmakingPresence;

/// Pins core's clock (none releases it); the clock is process-wide, so tests that pin it run serially.
pub fn pin_now(ms: Option<i64>) {
    rustic_isles::replay::pin_now_for_tests(ms);
}

/// Keeps every touch.
#[derive(Debug, Default)]
pub struct RecordingPresence(pub Mutex<Vec<(String, i64)>>);

impl MatchmakingPresence for RecordingPresence {
    fn touch(&self, user_id: &str, at_ms: i64) {
        self.0.lock().unwrap().push((user_id.to_owned(), at_ms));
    }
}

/// Signs as `crates/admin` does: the operator headers for a request.
pub fn operator_headers(
    secret: &str,
    method: &str,
    target: &str,
    id: &str,
    now_ms: i64,
    body: &[u8],
) -> Vec<(String, String)> {
    let timestamp = now_ms.to_string();
    let signature = crate::admin::auth::operator_signature(secret, method, target, &timestamp, id, body);
    vec![
        ("x-paradox-operator-id".into(), id.into()),
        ("x-paradox-operator-name".into(), "admin-api".into()),
        ("x-paradox-operator-timestamp".into(), timestamp),
        ("x-paradox-operator-signature".into(), signature),
    ]
}

/// `(character, transaction id, stacks)` of one applied run.
pub type Applied = (String, String, Vec<(String, i64)>);

/// Tracks with a linear curve of `per_rank` XP for `ranks` ranks, catalog ids it knows, and stacks it records.
#[derive(Debug, Default)]
pub struct FakeFunding {
    pub tracks: HashMap<String, (i64, i64)>,
    pub catalog: Vec<String>,
    pub applied: Mutex<Vec<Applied>>,
    pub refuse_stacks: bool,
}

impl FakeFunding {
    pub fn with_track(mut self, id: &str, per_rank: i64, ranks: i64) -> Self {
        self.tracks.insert(id.to_owned(), (per_rank, ranks));
        self
    }

    pub fn with_catalog(mut self, ids: &[&str]) -> Self {
        self.catalog.extend(ids.iter().map(|s| (*s).to_owned()));
        self
    }
}

impl TesterFunding for FakeFunding {
    fn is_approved_xp_track(&self, _: &Shared, id: &str) -> bool {
        self.tracks.contains_key(id)
    }

    fn track_cap_xp(&self, _: &Shared, id: &str) -> Option<i64> {
        self.tracks.get(id).map(|(per, ranks)| per * ranks)
    }

    fn rank_for_progress(&self, _: &Shared, id: &str, progress: i64) -> Option<i64> {
        let (per, ranks) = self.tracks.get(id)?;
        Some((progress / per).min(*ranks))
    }

    fn required_xp_for_rank(&self, _: &Shared, id: &str, rank: i64) -> RankRequirement {
        match self.tracks.get(id) {
            None => RankRequirement::UnknownTrack,
            Some((_, ranks)) if rank > *ranks || rank < 0 => RankRequirement::OutOfRange,
            #[allow(clippy::cast_precision_loss)]
            Some((per, _)) => RankRequirement::Xp((per * rank) as f64),
        }
    }

    fn is_known_catalog_id(&self, _: &Shared, id: &str) -> bool {
        self.catalog.iter().any(|c| c == id)
    }

    fn apply_operator_test_stacks<'a>(
        &'a self,
        _: &'a Arc<Shared>,
        _: &'a str,
        character_id: &'a str,
        transaction_id: &'a str,
        stacks: &'a [(String, i64)],
    ) -> BoxFuture<'a, StackOutcome> {
        Box::pin(async move {
            if self.refuse_stacks {
                return StackOutcome::NotOwned;
            }
            self.applied
                .lock()
                .unwrap()
                .push((character_id.to_owned(), transaction_id.to_owned(), stacks.to_vec()));
            StackOutcome::Applied
        })
    }
}
