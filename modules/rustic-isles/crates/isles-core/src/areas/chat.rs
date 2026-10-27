//! RU5's `Chat` group: Friends, guild and the voice stubs; the XMPP rules are `isles-chat`'s for DaintyLass's chat plugin (DD21).

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
