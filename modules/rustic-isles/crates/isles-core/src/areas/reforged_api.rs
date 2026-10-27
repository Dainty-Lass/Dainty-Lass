//! RU5's `ReforgedApi` group: The Reforged API with health, metrics, runtime and launcher updates and the admin panel, and Launcher Guard (RF1, decision 1). Node-scope routes: served from here whichever module is loaded, with its `TARGET_CHANGELIST` (RU3c, seam 7).

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
