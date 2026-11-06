// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `GET /patchnotes/:language/:gameversion` (spec §3.6). The structure, titles, types and dates are the
//! contract; the prose is this project's own (spec §13, item 1).

use serde_json::{Value, json};

use crate::envelope;

const PERMALINK: &str = "/patch-notes/rusticisles/";

/// The notes for `language`, enveloped. 1.14.7's empty release version gets the welcome notes, since its runtime
/// collapses the `Update <version>` line.
pub fn payload(language: &str, release_version: &str) -> Value {
    if !release_version.is_empty() {
        return envelope(json!({
            "date": "2024-12-19T18:00:00.000+00:00",
            "description": "Welcome to Rustic Isles!",
            "language": language,
            "notes": [],
            "permalink": PERMALINK,
            "release_version": release_version,
            "title": "Rustic Isles",
        }));
    }
    envelope(json!({
        "date": "2026-10-08T00:00:00.000+00:00",
        "description": "Welcome back to the Shattered Isles, Slayer. Rustic Isles is a community-run server that \
                        keeps Dauntless going, with your Slayer, your gear and your loadouts right where you left \
                        them.",
        "language": language,
        "notes": [
            {
                "sections": [
                    {
                        "description": "Ramsgate is open again and the Behemoths are back on the islands. Everything \
                                        you earned before still belongs to you: your Slayer, your weapons and armour, \
                                        and every loadout you saved.",
                        "title": "The hunt continues",
                        "type": "featured",
                    },
                    {
                        "changes": [{
                            "list": [
                                "Press **PLAY** in the launcher to sign in and arrive in Ramsgate.",
                                "Not sure what to bring? Try any weapon on the dummies in the **Training Grounds** \
                                 first.",
                                "Gather a party, or join one, and set out after your first Behemoth.",
                            ]
                        }],
                        "title": "Getting started",
                        "type": "default",
                    },
                ],
                "title": "Welcome to Rustic Isles",
                "type": "new_to_dauntless",
            },
            {
                "sections": [{
                    "changes": [{
                        "list": [
                            "This is a volunteer project and some things will still break. We are fixing them as \
                             we go.",
                            "When something goes wrong, tell the team **what happened and when**, so we can find it \
                             in the logs.",
                            "Thank you for hunting with us and for helping Dauntless live on.",
                        ]
                    }],
                    "title": "Help and feedback",
                    "type": "default",
                }],
                "title": "Community",
                "type": "quality_of_life",
            },
        ],
        "permalink": PERMALINK,
        "release_version": release_version,
        "title": "Rustic Isles",
    }))
}
