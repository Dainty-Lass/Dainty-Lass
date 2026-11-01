//! `GET /patchnotes/:language/:gameversion` (`routes/system.ts`), the login screen's patch notes.
//!
//! RU3c lists only `release_version` as the version difference (`PATCHNOTES_RELEASE_VERSION`), but 1.12.0's handler
//! answers a different body too: a 2024 date, a one-line description and no notes. A version whose release version is
//! not blank gets 1.12.0's body; the 1.14.7 runtime collapses the "Update <version>" line, so its release version is
//! blank and it gets the welcome notes (RU3c's table gap, reported with RU5 game-data).

use serde_json::{Value, json};

use super::gauntlet::envelope;

/// The body for `language` and a version's `release_version`.
pub fn payload(language: &str, release_version: &str) -> Value {
    if release_version.is_empty() {
        envelope(welcome(language, release_version))
    } else {
        envelope(json!({
            "date": "2024-12-19T18:00:00.000+00:00",
            "description": "Welcome to Rustic Isles!",
            "language": language,
            "notes": [],
            "permalink": "/patch-notes/rusticisles/",
            "release_version": release_version,
            "title": "Rustic Isles"
        }))
    }
}

fn welcome(language: &str, release_version: &str) -> Value {
    json!({
        "date": "2026-10-08T00:00:00.000+00:00",
        "description": "Welcome, Slayer! The Shattered Isles are calling again. Gather your party in Ramsgate and return to the hunt.",
        "language": language,
        "notes": [
            {
                "sections": [
                    {
                        "description": "Rustic Isles is a community project that keeps Dauntless alive. Your Slayer, gear and loadouts are kept on Rustic Isles servers, so you can pick up right where you left off.",
                        "title": "The hunt continues",
                        "type": "featured"
                    },
                    {
                        "changes": [
                            {
                                "list": [
                                    "Press **PLAY!** to travel to **Ramsgate**, the heart of the Shattered Isles",
                                    "Visit the **Training Grounds** to try out weapons and omnicells before you head out",
                                    "Pick a hunt, form a party and bring down your first **Behemoth**"
                                ]
                            }
                        ],
                        "title": "Getting started",
                        "type": "default"
                    }
                ],
                "title": "Welcome to Rustic Isles",
                "type": "new_to_dauntless"
            },
            {
                "sections": [
                    {
                        "changes": [
                            {
                                "list": [
                                    "Rustic Isles is still growing, so you may run into the odd bug",
                                    "If something goes wrong, tell the Rustic Isles team what you were doing and roughly when it happened",
                                    "Thank you for hunting with us!"
                                ]
                            }
                        ],
                        "title": "Help and feedback",
                        "type": "default"
                    }
                ],
                "title": "Community",
                "type": "quality_of_life"
            }
        ],
        "permalink": "/patch-notes/rusticisles/",
        "release_version": release_version,
        "title": "Rustic Isles"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_versions_differ_in_more_than_the_release_version() {
        let new = payload("en", "");
        assert_eq!(new["payload"]["release_version"], "");
        assert_eq!(new["payload"]["notes"].as_array().map(Vec::len), Some(2));
        let old = payload("fr", "1.12.0");
        assert_eq!(old["payload"]["release_version"], "1.12.0");
        assert_eq!(old["payload"]["language"], "fr");
        assert_eq!(old["payload"]["notes"], json!([]));
    }
}
