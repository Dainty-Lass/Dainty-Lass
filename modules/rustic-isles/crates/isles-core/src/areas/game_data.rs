//! RU5's `GameData` group: Game data and scripts. RU3c: hooks `progression_config`, `survey_config`; tables `ACTIVE_HUNTPASS_ID`, `RECENT_HUNTPASS_SEASONS`, `PATCHNOTES_RELEASE_VERSION`, `BOUNTY_ITEM_GRANT_DATA`, `GAME_DATA`.

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
