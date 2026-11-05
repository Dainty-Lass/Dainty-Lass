// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The private stored-text decoders the fuzz target drives (spec §14); not an API.

use crate::RepoResult;
use crate::repo::progression::ProgressionTrackRecord;

pub fn decode_roles(text: &str) -> RepoResult<Vec<String>> {
    crate::repo::auth::decode_roles(text)
}

pub fn decode_grant_result(text: &str) -> RepoResult<ProgressionTrackRecord> {
    crate::repo::progression::grant_result(text)
}
