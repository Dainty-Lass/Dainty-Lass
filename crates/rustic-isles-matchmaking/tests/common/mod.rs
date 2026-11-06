// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Accounts on a temporary store, a home that records the ring calls, and an activity recorder.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::sync::{Arc, Mutex};

use module_api::network::{HomeError, InstanceRecord, JoinDecision, PublishedTicket, SessionTicket, TicketState};
use module_api::{Actor, ActorContext, BoxFuture, GameVersion, GlobalId, HomeApi, NodeId, NodeIdentity};
use rustic_isles::Settings;
use rustic_isles_matchmaking::test_support::{FakeDirector, TestModule};
use rustic_isles_matchmaking::{Ctx, HuntActivity, JoinParty, JoinRequest};
use rustic_isles_store::repo::auth::AccountRecord;
use rustic_isles_store::{AccountId, IslesStore, UserId};

pub fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

/// A game account on this node named `<user> name`.
pub async fn account(store: &IslesStore, user_id: &str) -> AccountId {
    let id = AccountId::new(store.node());
    let record = AccountRecord {
        id: id.clone(),
        user_id: UserId::new(user_id),
        name: format!("{user_id} name"),
        notes: 0,
    };
    store
        .write(&system(), move |tx, repos| repos.accounts.create(tx, &record))
        .await
        .unwrap();
    id
}

pub fn ctx(module: &TestModule, client: Option<&str>) -> Ctx {
    Ctx {
        shared: Arc::clone(&module.shared),
        state: Arc::clone(&module.state),
        request_id: Some("test-request".into()),
        client: client.map(str::to_owned),
    }
}

/// A module with the fake Director in its slot.
pub fn module_with(store: Option<IslesStore>, settings: Settings, director: &Arc<FakeDirector>) -> TestModule {
    let module = TestModule::new(store, settings);
    module.shared.set_dedicated_servers(Arc::clone(director) as _);
    module
}

pub fn solo(player: &str, mode: Option<&str>, hunt: Option<&str>) -> JoinRequest {
    JoinRequest {
        player: player.into(),
        mode: mode.map(str::to_owned),
        hunt: hunt.map(str::to_owned),
        ..JoinRequest::default()
    }
}

pub fn with_party(
    player: &str,
    mode: &str,
    hunt: Option<&str>,
    party_id: &str,
    members: &[&str],
    id: Option<GlobalId>,
) -> JoinRequest {
    JoinRequest {
        party: JoinParty {
            party_id: Some(party_id.into()),
            members: Some(members.iter().map(|m| (*m).to_owned()).collect()),
            revision: Some(1),
            global_id: id,
        },
        ..solo(player, Some(mode), hunt)
    }
}

#[derive(Default)]
pub struct Activity {
    pub starts: Mutex<Vec<(Vec<String>, String, String)>>,
    pub ends: Mutex<Vec<(Vec<String>, String)>>,
}

impl HuntActivity for Activity {
    fn dedicated_hunt_starts(&self, users: &[String], hunt_id: &str, hunt_ref: &str) {
        self.starts
            .lock()
            .unwrap()
            .push((users.to_vec(), hunt_id.into(), hunt_ref.into()));
    }

    fn dedicated_hunt_ends(&self, users: &[String], end_reason: &str) {
        self.ends.lock().unwrap().push((users.to_vec(), end_reason.into()));
    }
}

/// A standalone home that writes down every ring call and answers each as not in a network.
pub struct RecordingHome {
    pub identity: NodeIdentity,
    pub calls: Mutex<Vec<String>>,
}

impl RecordingHome {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            identity: NodeIdentity::standalone(),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn note(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl HomeApi for RecordingHome {
    fn resolve_home<'a>(&'a self, _account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        Box::pin(async move { Ok(self.identity.clone()) })
    }

    fn session_ticket<'a>(
        &'a self,
        _account: &'a GlobalId,
        _audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    fn ring_submit_ticket<'a>(
        &'a self,
        _account: &'a GlobalId,
        queue: &'a str,
        _game: &'a GameVersion,
        _party_size: u16,
        client_ip: &'a str,
    ) -> BoxFuture<'a, Result<PublishedTicket, HomeError>> {
        self.note(format!("submit {queue} {client_ip}"));
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    fn ring_ticket_state<'a>(&'a self, _ticket: &'a GlobalId) -> BoxFuture<'a, Result<Option<TicketState>, HomeError>> {
        self.note("ticket_state".into());
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    fn ring_register_instance<'a>(&'a self, record: &'a InstanceRecord) -> BoxFuture<'a, Result<(), HomeError>> {
        self.note(format!("register {}", record.queue));
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    fn ring_resolve_join<'a>(
        &'a self,
        _account: &'a GlobalId,
        client_ip: &'a str,
        _instance: &'a GlobalId,
    ) -> BoxFuture<'a, Result<JoinDecision, HomeError>> {
        self.note(format!("resolve_join {client_ip}"));
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    fn ring_assign_ramsgate<'a>(
        &'a self,
        _account: &'a GlobalId,
        client_ip: &'a str,
        _game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<(InstanceRecord, Option<SessionTicket>), HomeError>> {
        self.note(format!("assign_ramsgate {client_ip}"));
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }
}
