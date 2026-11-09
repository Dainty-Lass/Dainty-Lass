// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The game calls other modules ask (MS3, core spec §7.9): whether players can get in, the game servers, and the
//! hub-status watch that puts their changes on the bus.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use dainty_module_api::game::{
    AVAILABILITY_CHANGED_EVENT, Availability, GameServer, GameServerKind, SERVER_DOWN_EVENT, SERVER_UP_EVENT,
    UnavailableReason,
};
use dainty_module_api::{Event, EventName, ModuleContext};

use crate::dedicated::{LiveHub, LiveHubStatus};
use crate::settings::Settings;
use crate::shared::Shared;

/// How often the watch asks the Director.
pub const WATCH_PERIOD: Duration = Duration::from_secs(10);
/// A Director slower than this counts as unreachable; its own client waits far longer for a launch
const ASK_TIMEOUT: Duration = Duration::from_secs(5);

/// What the Director answered on one ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Whether players can get in.
    pub availability: Availability,
    /// None when the Director did not answer
    pub servers: Option<Vec<GameServer>>,
}

/// `RAMSGATE_HUB` as the Director reads it (director spec §10): blank is on; `0`, `false`, `no`, `off` are off.
pub fn ramsgate_hub_off(settings: &Settings) -> bool {
    settings
        .raw("RAMSGATE_HUB")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"))
}

fn from_hub(hub: &LiveHub, kind: GameServerKind) -> GameServer {
    let port = u16::try_from(hub.port).unwrap_or(0);
    let (prefix, title) = match kind {
        GameServerKind::Dojo => ("dojo", "training_dojo"),
        _ => ("city", "ramsgate"),
    };
    GameServer {
        id: format!("{prefix}-{port}"),
        kind,
        title: title.to_owned(),
        map: String::new(),
        host: hub.host.clone(),
        port,
        players: 0,
        max_players: 0,
        ready: hub.ready,
        started_at: None,
    }
}

/// The hub decides (§7.9): Ramsgate up and ready is `up`. The Director's hub-status lists a hub only once it is
/// ready and alive, so a missing one is still starting or retrying unless the Director can start none.
pub fn availability(
    status: Option<&LiveHubStatus>,
    servers: Option<&[GameServer]>,
    hub_off: bool,
    can_launch: bool,
) -> Availability {
    let Some(status) = status else {
        return Availability::down(UnavailableReason::DirectorUnreachable);
    };
    match &status.ramsgate {
        Some(hub) if hub.ready => {
            let listed = servers
                .and_then(|list| list.iter().find(|s| s.kind == GameServerKind::Hub))
                .cloned();
            Availability::up(listed.unwrap_or_else(|| from_hub(hub, GameServerKind::Hub)))
        }
        Some(_) => Availability::down(UnavailableReason::HubNotReady),
        None if hub_off || !can_launch => Availability::down(UnavailableReason::NoHub),
        None => Availability::down(UnavailableReason::HubNotReady),
    }
}

/// The servers when the slot can't list them (`DEPLOYSERVER_URL`): the ready hubs from the hub-status.
fn hubs_only(status: &LiveHubStatus) -> Vec<GameServer> {
    let ramsgate = status.ramsgate.iter().map(|h| from_hub(h, GameServerKind::Hub));
    let training = status.training.iter().map(|h| from_hub(h, GameServerKind::Dojo));
    ramsgate.chain(training).filter(|s| s.ready).collect()
}

/// Asks the module's Director once.
pub async fn snapshot(shared: &Shared) -> Snapshot {
    let director = shared.dedicated_servers();
    let status = tokio::time::timeout(ASK_TIMEOUT, director.hub_status())
        .await
        .ok()
        .flatten();
    let servers = match &status {
        None => None,
        Some(status) => Some(
            tokio::time::timeout(ASK_TIMEOUT, director.servers())
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| hubs_only(status)),
        ),
    };
    let availability = availability(
        status.as_ref(),
        servers.as_deref(),
        ramsgate_hub_off(shared.settings()),
        director.configured(),
    );
    Snapshot { availability, servers }
}

/// What the watch last told the bus.
#[derive(Debug, Default)]
pub struct Watch {
    told: Option<(bool, Option<UnavailableReason>)>,
    servers: BTreeMap<String, GameServer>,
}

impl Watch {
    /// The events one snapshot makes: servers gone, servers new, then availability when `up` or the reason moved.
    /// An unanswered list keeps the last one, so a Director blip downs nothing
    pub fn step(&mut self, snapshot: &Snapshot) -> Vec<(&'static str, serde_json::Value)> {
        let mut out = Vec::new();
        if let Some(list) = &snapshot.servers {
            let now: BTreeMap<String, GameServer> = list.iter().map(|s| (s.id.clone(), s.clone())).collect();
            for (id, gone) in &self.servers {
                if !now.contains_key(id) {
                    out.push((SERVER_DOWN_EVENT, serde_json::to_value(gone).unwrap_or_default()));
                }
            }
            for (id, new) in &now {
                if !self.servers.contains_key(id) {
                    out.push((SERVER_UP_EVENT, serde_json::to_value(new).unwrap_or_default()));
                }
            }
            self.servers = now;
        }
        let state = (snapshot.availability.up, snapshot.availability.reason);
        if self.told != Some(state) {
            self.told = Some(state);
            let payload = serde_json::to_value(&snapshot.availability).unwrap_or_default();
            out.push((AVAILABILITY_CHANGED_EVENT, payload));
        }
        out
    }
}

/// Every [`WATCH_PERIOD`] from start: the first pass tells the bus where things stand.
pub async fn watch(ctx: ModuleContext, shared: Arc<Shared>) {
    let mut watch = Watch::default();
    let mut ticks = tokio::time::interval(WATCH_PERIOD);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let snapshot = snapshot(&shared).await;
        for (name, payload) in watch.step(&snapshot) {
            let Ok(name) = EventName::new(name) else { continue };
            let _ = ctx.emit(Event { name, payload }).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub(ready: bool) -> LiveHub {
        LiveHub {
            ready,
            host: "10.0.0.1".into(),
            port: 7777,
        }
    }

    fn status(ramsgate: Option<LiveHub>) -> LiveHubStatus {
        LiveHubStatus {
            ramsgate,
            training: None,
        }
    }

    #[test]
    fn reasons() {
        let reason = |s: Option<&LiveHubStatus>, off, launch| availability(s, None, off, launch).reason;
        assert_eq!(reason(None, false, true), Some(UnavailableReason::DirectorUnreachable));
        assert_eq!(
            reason(Some(&status(None)), false, true),
            Some(UnavailableReason::HubNotReady)
        );
        assert_eq!(reason(Some(&status(None)), true, true), Some(UnavailableReason::NoHub));
        assert_eq!(
            reason(Some(&status(None)), false, false),
            Some(UnavailableReason::NoHub)
        );
        let not_ready = status(Some(hub(false)));
        assert_eq!(
            reason(Some(&not_ready), false, true),
            Some(UnavailableReason::HubNotReady)
        );
        let up = availability(Some(&status(Some(hub(true)))), None, false, true);
        assert!(up.up && up.reason.is_none());
        let hub = up.hub.unwrap();
        assert_eq!(
            (hub.id.as_str(), hub.kind, hub.port),
            ("city-7777", GameServerKind::Hub, 7777)
        );
    }

    #[test]
    fn listed_hub_wins_over_the_status() {
        let mut listed = from_hub(&hub(true), GameServerKind::Hub);
        listed.players = 3;
        let up = availability(Some(&status(Some(hub(true)))), Some(&[listed.clone()]), false, true);
        assert_eq!(up.hub, Some(listed));
    }

    #[test]
    fn hub_switch() {
        for (value, off) in [
            ("off", true),
            (" No ", true),
            ("0", true),
            ("", false),
            ("on", false),
            ("x", false),
        ] {
            assert_eq!(
                ramsgate_hub_off(&Settings::from_pairs([("RAMSGATE_HUB", value)])),
                off,
                "{value}"
            );
        }
        assert!(!ramsgate_hub_off(&Settings::default()));
    }

    #[test]
    fn watch_events() {
        let names =
            |events: Vec<(&'static str, serde_json::Value)>| events.into_iter().map(|e| e.0).collect::<Vec<_>>();
        let mut watch = Watch::default();
        let down = Snapshot {
            availability: Availability::down(UnavailableReason::HubNotReady),
            servers: Some(Vec::new()),
        };
        assert_eq!(names(watch.step(&down)), [AVAILABILITY_CHANGED_EVENT]);
        assert!(watch.step(&down).is_empty());
        let city = from_hub(&hub(true), GameServerKind::Hub);
        let up = Snapshot {
            availability: Availability::up(city.clone()),
            servers: Some(vec![city.clone()]),
        };
        assert_eq!(names(watch.step(&up)), [SERVER_UP_EVENT, AVAILABILITY_CHANGED_EVENT]);
        // Players moving changes neither the set nor `up`
        let mut busier = up.clone();
        busier.servers = Some(vec![GameServer {
            players: 4,
            ..city.clone()
        }]);
        assert!(watch.step(&busier).is_empty());
        let blip = Snapshot {
            availability: Availability::down(UnavailableReason::DirectorUnreachable),
            servers: None,
        };
        assert_eq!(names(watch.step(&blip)), [AVAILABILITY_CHANGED_EVENT]);
        let events = watch.step(&down);
        assert_eq!(names(events.clone()), [SERVER_DOWN_EVENT, AVAILABILITY_CHANGED_EVENT]);
        assert_eq!(events[0].1["id"], "city-7777");
        assert_eq!(
            events[1].1,
            serde_json::json!({ "up": false, "reason": "hub_not_ready" })
        );
    }
}
