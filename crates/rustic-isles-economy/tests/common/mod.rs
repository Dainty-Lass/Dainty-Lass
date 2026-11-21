// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A module over a temporary store with the three economy parts and the characters area, driven through core's
//! `app()`; synthetic game data written by the test (nothing from the game).

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use dainty_module_api::{Actor, ActorContext, BoxFuture as ApiFuture};
use dainty_service_kit::serve::ConnectionInfo;
use http_body_util::BodyExt;
use rustic_isles::handlers::auth_handlers;
use rustic_isles::testing::{SAMPLE_TABLES, SampleHooks};
use rustic_isles::{
    Destination, Entitlement, GameDataRoot, IslesVersion, MaskedJourneyNode, Method, NoPlayerPolicy, ObjectiveRow,
    ProgressionConfigRule, RouteSet, SeasonalCoin, Service, Settings, Shared, SharedParts, Slots, StarterManifest,
    StubReply, VersionHooks, VersionTables, When, routes,
};
use rustic_isles_auth::test_support::throwaway_signing_keys;
use rustic_isles_auth::{AuthData, AuthStore, Realms, RuntimeCaller, RuntimeRequest};
use rustic_isles_characters::{CharacterData, Characters, StarterKit};
use rustic_isles_economy::EconomyCaches;
use rustic_isles_store::repo::auth::AccountRecord;
use rustic_isles_store::repo::inventory::InventoryRecord;
use rustic_isles_store::repo::progression::ProgressionTrackRecord;
use rustic_isles_store::test_support::TempStore;
use rustic_isles_store::{AccountId, IslesAuthStore, IslesStore, UserId};
use rustic_rifts_seam::{
    BoxFuture, CapabilityScope, Component, DedicatedOnly, GuardHooks, HostRouteSample, HostWrite, HostWriteKind,
    HostWriteOptions, InventoryDecision, InventoryFence, InventoryTransaction, Json, Lifecycle, Reply, RequestInfo,
    RiftsParts, RuntimeAuth, SeamError, StubRoutes, TableSpec, TransactionPhase,
};
use serde_json::{Map, Value, json};
use tower::ServiceExt;

pub const NOW_MS: i64 = 1_792_000_000_000;
pub const GS_KEY: &str = "gs-key-for-tests";
pub const PASS: &str = "season43";
const SECRET: &str = "a-test-secret-that-is-long-enough-1234";

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("not an object"),
    }
}

pub fn character_data() -> CharacterData {
    CharacterData {
        fill: vec![("PlayerAccountProgressStep".into(), json!("New"))],
        default_appearance: object(json!({"Data": [], "StringData": []})),
        forced_character_flags: vec![],
        forced_login_flag: ("Login_Seen".into(), json!("true")),
        dojo_series_key: "SERIE_dojo".into(),
        dojo_series: object(json!({"Series": "DOJO_TEST"})),
        dojo_node_id: "Dojo".into(),
        trigger_series_key: "SERIE_trigger".into(),
        trigger_node: "Node_T".into(),
        trigger_status: 3.0,
    }
}

pub const WEAPON: &str = "WP_TEST_STARTER";

pub fn kit() -> StarterKit {
    StarterKit {
        instanced: vec![
            WEAPON.into(),
            "PART_TEST_A".into(),
            "FLASK_TEST".into(),
            "BANNER_TEST".into(),
            "GLIDER_TEST".into(),
        ],
        default_weapon_parts: vec!["PART_TEST_A".into()],
        equipment: vec![("weapon".into(), WEAPON.into())],
        instance_data: vec![],
        flask: "FLASK_TEST".into(),
        banner: "BANNER_TEST".into(),
        glider: "GLIDER_TEST".into(),
        banner_customization: Map::new(),
        intro_emote: "EMOTE_TEST".into(),
        flare: "FLARE_TEST".into(),
        default_quick_chats: (1..=9).map(|i| format!("chat_{i}")).collect(),
        wallet: vec![("CURRENCY_NOTES".into(), 5000.0), ("CURRENCY_PJM_WEAPON".into(), 50.0)],
        default_appearance: Map::new(),
    }
}

pub static TABLES: VersionTables = VersionTables {
    active_huntpass_id: PASS,
    store_sections: &[
        "exchange_vendor_slot_2",
        "exchange_vendor_slot_3",
        "ladyluckstore",
        "season_store",
        "webstore",
        "dyes",
        "hp_level_skip_rank",
        "huntpass_store",
        "fountain_daily_free_bundle",
    ],
    seasonal_coin: Some(SeasonalCoin {
        catalog_id: "CURRENCY_SEASONAL_COIN",
        max_per_claim: 1000,
    }),
    masked_journey_nodes: &[MaskedJourneyNode {
        node: "Gauntlet_Unlock",
        switch: "RUSTIC_ISLES_GAUNTLET",
    }],
    starter_manifest: StarterManifest {
        bootstrap_version: "test-1",
        stacked_item_ids: &["STACK_STARTER"],
        quick_emoji_wheel: &[],
    },
    ..SAMPLE_TABLES
};

pub struct Hooks {
    pub policy: NoPlayerPolicy,
    pub base_level: bool,
}

impl VersionHooks for Hooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        self.policy
    }
    fn weapons_at_base_level(&self, settings: &Settings) -> bool {
        self.base_level && settings.raw("KEEP_WEAPON_ITEM_LEVELS").map(str::trim) != Some("true")
    }
    fn reforge_rules(&self) -> bool {
        true
    }
    fn mastery_persistence(&self) -> bool {
        true
    }
    fn huntpass_payload(&self, id: &str) -> Value {
        json!({"huntpass_id": id, "season": id})
    }
    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value {
        json!({
            "phx_account_id": user_id,
            "objective_id": row.objective_id,
            "progress": rustic_isles_economy::number_value(row.value),
            "completed_count": rustic_isles_economy::number_value(row.completed_count),
            "created_date": row.created,
            "last_modified_date": row.last_modified,
        })
    }
    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::AsIs
    }
    fn survey_config(&self) -> StubReply {
        SampleHooks.survey_config()
    }
    fn entitlements_payload(&self, list: &[Entitlement]) -> Value {
        let list: Vec<Value> = list
            .iter()
            .map(|e| json!({"activatedDate": e.activated, "duration": rustic_isles_economy::number_value(e.duration), "name": e.name}))
            .collect();
        json!({ "entitlements": list })
    }
    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination {
        SampleHooks.candidate_destination(hunt_id, realm_hub)
    }
}

pub fn runtime_auth() -> RuntimeAuth {
    RuntimeAuth {
        capability_id: "cap-1".into(),
        session: rustic_rifts_seam::SessionId("01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap()),
        generation: 1,
        host_account: rustic_rifts_seam::AccountId("host".into()),
        scopes: vec![CapabilityScope::GameServerRuntime],
        roster: vec![],
        host_peer: None,
        bridge_instance: None,
        wire_session: None,
    }
}

/// `cap-ok` is a runtime with rifts' handle and the roster `player-1`, `player-2`; `cap-bare` one without a handle.
struct HostRealms;

impl Realms for HostRealms {
    fn authenticate_runtime<'a>(
        &'a self,
        request: RuntimeRequest<'a>,
    ) -> ApiFuture<'a, Result<rustic_isles_auth::RuntimeAuth, String>> {
        let capability = request
            .headers
            .get(rustic_rifts_seam::HOST_CAPABILITY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        Box::pin(async move {
            let roster = vec!["player-1".to_owned(), "player-2".to_owned()];
            Ok(match capability.as_str() {
                "cap-ok" => rustic_isles_auth::RuntimeAuth::Authenticated(
                    RuntimeCaller::new(roster).with_handle(Arc::new(runtime_auth())),
                ),
                "cap-bare" => rustic_isles_auth::RuntimeAuth::Authenticated(RuntimeCaller::new(roster)),
                _ => rustic_isles_auth::RuntimeAuth::NotMine,
            })
        })
    }
}

/// The fence the guard hands out; it keeps what it heard.
pub struct RecordingFence {
    pub journal: Arc<Mutex<Vec<String>>>,
    pub refusal: Option<Reply>,
}

impl InventoryFence for RecordingFence {
    fn on_grant(&self, grant: &Json) -> Result<(), Reply> {
        self.journal.lock().unwrap().push(format!("grant {}", grant.0));
        match &self.refusal {
            Some(reply) => Err(reply.clone()),
            None => Ok(()),
        }
    }
    fn report(&self, phase: TransactionPhase) {
        self.journal.lock().unwrap().push(format!("{phase:?}"));
    }
}

#[derive(Default)]
pub struct FakeGuard {
    /// Refuses every host write with this reply
    pub refusal: Mutex<Option<Reply>>,
    /// Refuses these kinds on a probe only
    pub probe_refusals: Mutex<Vec<HostWriteKind>>,
    /// Refuses these kinds always
    pub kind_refusals: Mutex<Vec<HostWriteKind>>,
    /// Observe mode (the policy does not refuse)
    pub observe: Mutex<bool>,
    pub fail_authorize: Mutex<bool>,
    pub asked: Mutex<Vec<(HostWriteKind, String, HostWriteOptions)>>,
    pub recorded: Mutex<Vec<HostWrite>>,
    pub notes: Mutex<Vec<(String, String)>>,
    /// The inventory decision: none authorizes with a recording fence
    pub inventory_refusal: Mutex<Option<Reply>>,
    pub grant_refusal: Mutex<Option<Reply>>,
    pub journal: Arc<Mutex<Vec<String>>>,
}

fn forbidden(message: &str) -> Reply {
    Reply {
        status: 403,
        body: Json(format!(
            r#"{{"code":"forbidden","message":"{message}","payload":null}}"#
        )),
    }
}

impl FakeGuard {
    pub fn refuse_all(&self) {
        *self.refusal.lock().unwrap() = Some(forbidden("This runtime cannot write player state."));
    }
    pub fn tags(&self) -> Vec<(HostWriteKind, String, Value)> {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .map(|w| (w.kind, w.target.0.clone(), serde_json::from_str(&w.summary.0).unwrap()))
            .collect()
    }
}

impl Lifecycle for FakeGuard {
    fn start(&self, role: &str) -> Result<(), SeamError> {
        DedicatedOnly.start(role)
    }
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async { Ok(()) })
    }
    fn startup_summary(&self, role: &str) -> Result<Option<Json>, SeamError> {
        DedicatedOnly.startup_summary(role)
    }
    fn runtime_safety_issues(&self, settings: &dyn rustic_rifts_seam::Settings) -> Result<Vec<String>, SeamError> {
        DedicatedOnly.runtime_safety_issues(settings)
    }
    fn tables(&self) -> &'static [TableSpec] {
        &[]
    }
    fn account_deletion_steps(&self) -> Vec<rustic_rifts_seam::DeletionStep> {
        vec![]
    }
}

impl GuardHooks for FakeGuard {
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(!*self.observe.lock().unwrap())
    }
    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError> {
        self.notes.lock().unwrap().push((what.to_owned(), detail.to_owned()));
        Ok(())
    }
    fn census(&self, _: &HostRouteSample) -> Result<(), SeamError> {
        Ok(())
    }
    fn rate_limit_bucket(&self, _: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        Box::pin(async { Ok(None) })
    }
    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        kind: HostWriteKind,
        target: rustic_rifts_seam::AccountId,
        options: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        self.asked.lock().unwrap().push((kind, target.0, options));
        if *self.fail_authorize.lock().unwrap() {
            return Box::pin(async { Err(SeamError::unimplemented(Component::Guard, "authorize_host_write")) });
        }
        let mut answer = self.refusal.lock().unwrap().clone();
        if self.kind_refusals.lock().unwrap().contains(&kind)
            || (options.probe && self.probe_refusals.lock().unwrap().contains(&kind))
        {
            answer = Some(forbidden("This runtime cannot write player state."));
        }
        Box::pin(async move { Ok(answer) })
    }
    fn record_host_write(&self, _: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        self.recorded.lock().unwrap().push(write);
        Box::pin(async { Ok(()) })
    }
    fn authorize_inventory(
        &self,
        _: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        self.journal.lock().unwrap().push(format!(
            "authorize_inventory {} {} {}",
            transaction.user.0, transaction.character_id, transaction.transaction_id
        ));
        let decision = match self.inventory_refusal.lock().unwrap().clone() {
            Some(reply) => InventoryDecision::Refused(reply),
            None => InventoryDecision::Authorized(Box::new(RecordingFence {
                journal: Arc::clone(&self.journal),
                refusal: self.grant_refusal.lock().unwrap().clone(),
            })),
        };
        Box::pin(async move { Ok(decision) })
    }
}

fn rifts(guard: Arc<FakeGuard>) -> rustic_rifts_seam::Rifts {
    let routes: Vec<rustic_rifts_seam::Route> = routes::all()
        .iter()
        .filter(|r| r.when == When::Rifts)
        .map(|r| rustic_rifts_seam::Route {
            method: match r.method {
                Method::Get => rustic_rifts_seam::Method::Get,
                Method::Patch => rustic_rifts_seam::Method::Patch,
                _ => rustic_rifts_seam::Method::Post,
            },
            path: r.path,
            mount: rustic_rifts_seam::Mount::Normal,
            handler: "stub.route",
        })
        .collect();
    let routes: &'static [rustic_rifts_seam::Route] = Box::leak(routes.into_boxed_slice());
    let dedicated = Arc::new(DedicatedOnly);
    rustic_rifts_seam::Rifts::new(RiftsParts {
        realms: dedicated.clone(),
        entry: dedicated,
        guard,
        routes: vec![Arc::new(StubRoutes {
            component: Component::Guard,
            routes,
        })],
    })
}

pub struct Options {
    pub policy: NoPlayerPolicy,
    pub settings: Vec<(&'static str, String)>,
    pub store: bool,
    pub game_data: bool,
    /// Extra files in the backend folder
    pub files: Vec<(&'static str, Value)>,
    /// Leave out the storage policy
    pub no_policy: bool,
    pub base_level: bool,
    pub dedicated_guard: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            policy: NoPlayerPolicy::ConfiguredAccount,
            settings: Vec::new(),
            store: true,
            game_data: true,
            files: Vec::new(),
            no_policy: false,
            base_level: true,
            dedicated_guard: false,
        }
    }
}

pub struct Answer {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {} {}", self.status, self.body))
    }

    #[track_caller]
    pub fn ok(&self) -> Value {
        assert_eq!(self.status, 200, "{}", self.body);
        self.json()
    }

    #[track_caller]
    pub fn assert_empty(&self, status: u16) {
        assert_eq!(
            (self.status, self.body.as_str()),
            (status, ""),
            "{:?}",
            self.content_type
        );
        assert_eq!(self.content_type, None);
    }

    #[track_caller]
    pub fn assert(&self, status: u16, body: &str) {
        assert_eq!((self.status, self.body.as_str()), (status, body));
    }
}

#[derive(Clone)]
pub enum Who {
    Nobody,
    Player(&'static str),
    Gameserver,
    /// A game server with a player's bearer
    GameserverAs(&'static str),
    Host(&'static str),
    Data(AuthData),
}

/// A host runtime caller with `user` as its user id (`AuthData` put in the extensions).
pub fn host_as(user: Option<&str>, handle: bool) -> Who {
    let mut caller = RuntimeCaller::new(vec!["player-1".to_owned(), "player-2".to_owned()]);
    if handle {
        caller = caller.with_handle(Arc::new(runtime_auth()));
    }
    Who::Data(AuthData {
        user_id: user.map(str::to_owned),
        is_gameserver: false,
        claims: Map::new(),
        runtime: Some(caller),
    })
}

pub struct Harness {
    pub app: axum::Router,
    pub shared: Arc<Shared>,
    pub store: IslesStore,
    pub guard: Arc<FakeGuard>,
    pub data: tempfile::TempDir,
    temp: TempStore,
}

impl Harness {
    pub async fn new(options: Options) -> Self {
        let temp = TempStore::open().unwrap();
        let store = temp.store.clone();
        let version = IslesVersion {
            hooks: Arc::new(Hooks {
                policy: options.policy,
                base_level: options.base_level,
            }),
            tables: &TABLES,
            routes: RouteSet::V1147,
        };
        let (private, public) = throwaway_signing_keys();
        let mut pairs: Vec<(String, String)> = vec![
            ("AUTH_SIGNING_PRIVKEY_B64".into(), private),
            ("AUTH_SIGNING_PUBKEY_B64".into(), public),
            ("API_KEY_HASH_SECRET".into(), SECRET.into()),
            ("RUSTIC_ISLES_REPLAY_HEADERS".into(), "on".into()),
            ("RUSTIC_ISLES_RATE_LIMIT".into(), "off".into()),
        ];
        pairs.extend(options.settings.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
        let auth_store = Arc::new(IslesAuthStore::new(store.clone()));
        let hash = rustic_isles_auth::hashes::gameserver_key_hash(SECRET, GS_KEY);
        auth_store
            .replace_gameserver_keys(&ActorContext::new(Actor::System), &[hash])
            .await
            .unwrap();
        let guard = Arc::new(FakeGuard::default());
        let data = tempfile::tempdir().unwrap();
        if !options.no_policy {
            rustic_isles_economy::test_support::write_json(
                data.path(),
                "inventory_storage_policy.json",
                &rustic_isles_economy::test_support::storage_policy(),
            )
            .unwrap();
        }
        rustic_isles_economy::test_support::write_json(
            data.path(),
            "progression_config.json",
            &rustic_isles_economy::test_support::progression_config(PASS),
        )
        .unwrap();
        for (name, value) in &options.files {
            rustic_isles_economy::test_support::write_json(data.path(), name, value).unwrap();
        }
        let mut parts = SharedParts::bare(version);
        parts.settings = Settings::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        parts.auth_store = Some(auth_store);
        parts.auth_seams.realms = Arc::new(HostRealms);
        parts.rifts = if options.dedicated_guard {
            rustic_rifts_seam::Rifts::dedicated_only()
        } else {
            rifts(Arc::clone(&guard))
        };
        if options.game_data {
            parts.game_data = GameDataRoot::new(Some(data.path().to_path_buf()), None);
        }
        let mut slots = Slots::default();
        if options.store {
            slots.insert(Arc::new(store.clone()));
        }
        slots.insert(Arc::new(Characters::new(character_data(), kit())));
        slots.insert(Arc::new(EconomyCaches::default()));
        parts.slots = slots;
        let mut handlers = auth_handlers().unwrap();
        rustic_isles_characters::register(&mut handlers);
        rustic_isles_economy::register(&mut handlers);
        parts.handlers = handlers.clone();
        let shared = Shared::new(parts).unwrap();
        let app = rustic_isles::app(Arc::clone(&shared), Service::Backend, &handlers).unwrap();
        Self {
            app,
            shared,
            store,
            guard,
            data,
            temp,
        }
    }

    pub async fn bare() -> Self {
        Self::new(Options::default()).await
    }

    pub fn token(&self, user: &str) -> String {
        format!("Bearer {}", self.shared.auth().sign_game_token(user).unwrap())
    }

    pub async fn send_with(
        &self,
        method: &str,
        uri: &str,
        who: &Who,
        body: Option<&str>,
        extra: &[(&str, &str)],
    ) -> Answer {
        let mut builder = Request::builder().method(method).uri(uri).header("host", "localhost");
        if !extra.iter().any(|(n, _)| *n == "x-isles-replay-now-ms") {
            builder = builder.header("x-isles-replay-now-ms", NOW_MS.to_string());
        }
        match who {
            Who::Nobody | Who::Data(_) => {}
            Who::Player(user) => builder = builder.header("authorization", self.token(user)),
            Who::Gameserver => builder = builder.header("x-rustic-isles-gameserver-apikey", GS_KEY),
            Who::GameserverAs(user) => {
                builder = builder
                    .header("x-rustic-isles-gameserver-apikey", GS_KEY)
                    .header("authorization", self.token(user));
            }
            Who::Host(capability) => {
                builder = builder
                    .header("authorization", "Host runtime")
                    .header(rustic_rifts_seam::HOST_CAPABILITY_HEADER, *capability);
            }
        }
        for (name, value) in extra {
            builder = builder.header(*name, *value);
        }
        if body.is_some() && !extra.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-type")) {
            builder = builder.header("content-type", "application/json");
        }
        let text = body.unwrap_or("");
        let mut request = builder
            .header("content-length", text.len().to_string())
            .body(Body::from(text.to_owned()))
            .unwrap();
        if let Who::Data(data) = who {
            request.extensions_mut().insert(data.clone());
        }
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(ConnectionInfo {
            peer,
            local: "127.0.0.1:61000".parse().unwrap(),
        }));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_owned());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            content_type,
            body: String::from_utf8_lossy(&body).into_owned(),
        }
    }

    pub async fn send(&self, method: &str, uri: &str, who: &Who, body: Option<&str>) -> Answer {
        self.send_with(method, uri, who, body, &[]).await
    }

    pub async fn get(&self, uri: &str, who: &Who) -> Answer {
        self.send("GET", uri, who, None).await
    }

    pub async fn post(&self, uri: &str, who: &Who, body: &Value) -> Answer {
        self.send("POST", uri, who, Some(&body.to_string())).await
    }

    /// The player's first character (made by `GET /character`).
    pub async fn character(&self, user: &'static str) -> String {
        let answer = self.get("/character", &Who::Player(user)).await;
        let list = answer.ok();
        list[0]["id"].as_str().unwrap().to_owned()
    }

    pub async fn account(&self, user: &str) -> Option<AccountRecord> {
        let user = UserId::new(user);
        self.store
            .read(move |c, r| r.accounts.find_by_user_id(c, &user))
            .await
            .unwrap()
    }

    pub async fn account_id(&self, user: &str) -> AccountId {
        self.account(user).await.unwrap().id
    }

    pub async fn inventory(&self, character: &str) -> InventoryRecord {
        let character = character.to_owned();
        self.store
            .read(move |c, r| r.inventories.find_by_character_id(c, &character))
            .await
            .unwrap()
            .unwrap()
    }

    pub async fn stacked(&self, character: &str) -> Value {
        serde_json::from_str(&self.inventory(character).await.stacked_items).unwrap()
    }

    pub async fn instanced(&self, character: &str) -> Value {
        serde_json::from_str(&self.inventory(character).await.instanced_items).unwrap()
    }

    pub async fn set_inventory(&self, character: &str, instanced: &Value, stacked: &Value) {
        let row = self.inventory(character).await;
        let (instanced, stacked) = (instanced.to_string(), stacked.to_string());
        let character = character.to_owned();
        let updated = self
            .store
            .write(&system(), move |tx, r| {
                r.inventories.update_both_if_revision_matches(
                    tx,
                    &character,
                    &instanced,
                    &stacked,
                    row.revision.unwrap_or(0),
                )
            })
            .await
            .unwrap();
        assert!(updated.is_some());
    }

    pub async fn balances(&self, user: &str) -> Vec<(String, f64)> {
        let id = self.account_id(user).await;
        self.store
            .read(move |c, r| r.wallets.find(c, &id))
            .await
            .unwrap()
            .map(|w| w.balances)
            .unwrap_or_default()
    }

    pub async fn balance(&self, user: &str, catalog: &str) -> Option<f64> {
        self.balances(user)
            .await
            .into_iter()
            .find(|(c, _)| c == catalog)
            .map(|(_, b)| b)
    }

    pub async fn credit(&self, user: &str, catalog: &'static str, amount: f64) {
        let id = self.account_id(user).await;
        let starter = rustic_isles_economy::wallet::starter_balances(&self.shared);
        self.store
            .write(&system(), move |tx, r| {
                rustic_isles_economy::wallet::add_currency(tx, r, &id, &starter, catalog, amount)
                    .map_err(|e| rustic_isles_store::RepoError::Data(e.to_string()))
            })
            .await
            .unwrap();
    }

    pub async fn track(&self, user: &str, track: &str) -> Option<ProgressionTrackRecord> {
        let id = self.account_id(user).await;
        let track = track.to_owned();
        self.store
            .read(move |c, r| r.progression_tracks.get(c, &id, &track))
            .await
            .unwrap()
    }

    pub async fn set_progress(&self, user: &str, track: &'static str, amount: i64) {
        let id = self.account_id(user).await;
        self.store
            .write(&system(), move |tx, r| {
                r.progression_tracks.increment(tx, &id, track, amount)
            })
            .await
            .unwrap();
    }

    pub async fn ledger_rows(&self, user: &str) -> Vec<Value> {
        let id = self.account_id(user).await;
        self.store
            .read(move |c, r| r.inventory_transactions.list_recent_for_account(c, &id, 100))
            .await
            .unwrap()
    }
}

pub fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

pub fn settings(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
}
