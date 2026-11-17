//! Staff kicks (spec-runtime-kick): once a second on the SP-3 tick, the Director's kick file for this port is read and
//! removed, and each named player connected here is sent back to the menu by a client call on their controller.
//! The pure half is the core crate's `kick`.

use crate::config;
use crate::ffi::memory::{ProcessMemory, read};
use crate::ffi::{engine, flags, log, reflect};
use crate::listen;
use rustic_isles_runtime_core::kick::{self as core, Pending};
use rustic_isles_runtime_core::offsets::{r7, r8};
use std::sync::Mutex;

struct Kicks {
    started: bool,
    disabled: bool,
    last_poll: Option<u64>,
    pending: Pending,
    ids_logged: bool,
}

static KICKS: Mutex<Kicks> = Mutex::new(Kicks {
    started: false,
    disabled: false,
    last_poll: None,
    pending: Pending::new(),
    ids_logged: false,
});

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

/// A remote player's controller on `connection`, with the player id on its player state when it reads.
fn player(connection: usize) -> Option<(usize, Option<String>)> {
    let controller = read::<usize>(connection + r8::CONNECTION_OWNING_ACTOR)?;
    if controller == 0 || !engine::is_live_object(controller) || !reflect::is_a(controller, "PlayerController") {
        return None;
    }
    let state = read::<usize>(controller + r7::PC_PLAYER_STATE).filter(|&s| s != 0 && engine::is_live_object(s));
    let id = state.and_then(|state| core::player_id(&ProcessMemory, state + r7::PS_NET_ID));
    Some((controller, id))
}

/// The first client call that answers, with the reason as an `FText` (calls 3 and 4).
fn send_to_menu(controller: usize) -> Option<&'static str> {
    let text = reflect::text_for(core::REASON)?;
    core::CLIENT_CALLS.into_iter().find(|call| {
        let mut params = reflect::Params::new(reflect::FTEXT_SIZE);
        params.put(0, &text);
        reflect::invoke(controller, call, &mut params)
    })
}

/// The step, from the SP-3 after handler while the server listens (`now` is the tick clock).
pub(super) fn step(driver: usize, now: u64) {
    let Ok(mut kicks) = KICKS.try_lock() else {
        return;
    };
    let port = config::port();
    if !kicks.started {
        kicks.started = true;
        kicks.disabled = flags::flag(core::DISABLE_FLAG);
        log::line(&core::config_line(kicks.disabled, port));
    }
    if kicks.disabled
        || kicks
            .last_poll
            .is_some_and(|last| now.saturating_sub(last) < core::POLL_MS)
    {
        return;
    }
    kicks.last_poll = Some(now);
    let mut read_now = None;
    if let Some(path) = flags::path_next_to_exe(&core::file_name(port))
        && path.is_file()
    {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        // Removed before it is acted on: a request is carried out once, and a stuck file can't repeat it
        _ = std::fs::remove_file(&path);
        let ids = core::parse(&text, unix_ms());
        read_now = Some(ids.len());
        kicks.pending.add(ids, now);
    }
    if kicks.pending.is_empty() {
        return;
    }
    let players: Vec<(usize, Option<String>)> = listen::client_connections(driver)
        .into_iter()
        .filter_map(player)
        .collect();
    let ids: Vec<Option<String>> = players.iter().map(|(_, id)| id.clone()).collect();
    if let Some(requested) = read_now {
        log::line(&core::read_line(requested, players.len()));
        if !kicks.ids_logged {
            kicks.ids_logged = true;
            log::line(&core::ids_line(&ids));
        }
    }
    let (kick, expired) = kicks.pending.take(&ids, now);
    for id in kick {
        for (controller, _) in players.iter().filter(|(_, read)| read.as_deref() == Some(id.as_str())) {
            log::line(&core::kick_line(&id, send_to_menu(*controller)));
        }
    }
    for id in expired {
        log::line(&core::expired_line(&id));
    }
}
