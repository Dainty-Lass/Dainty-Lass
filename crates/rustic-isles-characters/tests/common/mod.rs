// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Synthetic fixtures (spec §13.1) and a module over a temporary store, driven through core's `app()`.

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
use dainty_service_kit::jsjson;
use dainty_service_kit::serve::ConnectionInfo;
use http_body_util::BodyExt;
use rustic_isles::handlers::auth_handlers;
use rustic_isles::testing::{SAMPLE_TABLES, SampleHooks};
use rustic_isles::{
    Destination, Entitlement, IslesVersion, Method, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule, RouteSet,
    Service, Settings, Shared, SharedParts, Slots, StarterManifest, StubReply, VersionHooks, VersionTables, When,
    routes,
};
use rustic_isles_auth::test_support::throwaway_signing_keys;
use rustic_isles_auth::{AuthData, AuthStore, Realms, RuntimeCaller, RuntimeRequest};
use rustic_isles_characters::{CharacterData, Characters, StarterKit};
use rustic_isles_store::repo::auth::{AccountRecord, LauncherAccountRecord, OperationalStatus};
use rustic_isles_store::repo::characters::LoadoutRecord;
use rustic_isles_store::repo::inventory::InventoryRecord;
use rustic_isles_store::repo::progression::PlayerJourneyRecord;
use rustic_isles_store::test_support::TempStore;
use rustic_isles_store::{AccountId, IslesAuthStore, IslesStore, UserId};
use rustic_rifts_seam::{
    BoxFuture, CapabilityScope, Component, DedicatedOnly, GuardHooks, HostRouteSample, HostWrite, HostWriteKind,
    HostWriteOptions, InventoryDecision, InventoryTransaction, Json, Lifecycle, Reply, RequestInfo, RiftsParts,
    RuntimeAuth, SeamError, StubRoutes, TableSpec,
};
use serde_json::{Map, Value, json};
use tower::ServiceExt;

pub const NOW_MS: &str = "1792000000000";
pub const GS_KEY: &str = "gs-key-for-tests";
const SECRET: &str = "a-test-secret-that-is-long-enough-1234";

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("not an object"),
    }
}

pub fn appearance() -> Map<String, Value> {
    object(json!({
        "CreationState": "EArchonCharacterCreationState::None",
        "Data": [7, 8],
        "StringData": [{"Key": "Hair", "Value": "Short"}],
        "AssetReferences": [],
    }))
}

pub fn dojo_series() -> Map<String, Value> {
    object(json!({"Series": "DOJO_TEST", "Steps": [1, 2]}))
}

pub fn character_data() -> CharacterData {
    CharacterData {
        fill: vec![
            ("RecentPlayers".into(), json!("[]")),
            (
                "AppearanceData".into(),
                Value::from(jsjson::stringify(&Value::Object(appearance()))),
            ),
            ("PlayerAccountProgressStep".into(), json!("New")),
            ("CharacterFlagData".into(), json!("{\"Flags\":[]}")),
            ("RepairData".into(), json!("{}")),
        ],
        default_appearance: appearance(),
        forced_character_flags: vec![("Flag_A".into(), json!("True")), ("Flag_B".into(), json!("True"))],
        forced_login_flag: ("Login_Seen".into(), json!("true")),
        dojo_series_key: "SERIE_dojo".into(),
        dojo_series: dojo_series(),
        dojo_node_id: "Dojo".into(),
        trigger_series_key: "SERIE_trigger".into(),
        trigger_node: "Node_T".into(),
        trigger_status: 3.0,
    }
}

pub const WEAPON: &str = "WP_TEST_00";
pub const FLASK: &str = "FLASK_TEST";
pub const PARTS: [&str; 2] = ["PART_TEST_A", "PART_TEST_B"];

pub fn kit() -> StarterKit {
    let instanced = [
        WEAPON,
        "PART_TEST_A",
        "PART_TEST_B",
        "HELM_TEST",
        "CHEST_TEST",
        "LANTERN_TEST",
        FLASK,
        "BANNER_TEST",
        "GLIDER_TEST",
    ];
    StarterKit {
        instanced: instanced.iter().map(|s| (*s).to_owned()).collect(),
        default_weapon_parts: PARTS.iter().map(|s| (*s).to_owned()).collect(),
        equipment: vec![
            ("weapon".into(), WEAPON.into()),
            ("helmet".into(), "HELM_TEST".into()),
            ("chest".into(), "CHEST_TEST".into()),
            ("lantern".into(), "LANTERN_TEST".into()),
        ],
        instance_data: vec![(WEAPON.into(), object(json!({"Level": 1})))],
        flask: FLASK.into(),
        banner: "BANNER_TEST".into(),
        glider: "GLIDER_TEST".into(),
        banner_customization: object(json!({"Color": "Red"})),
        intro_emote: "EMOTE_TEST".into(),
        flare: "FLARE_TEST".into(),
        default_quick_chats: (1..=9).map(|i| format!("chat_{i}")).collect(),
        wallet: vec![("COIN_TEST".into(), 100.0), ("GEM_TEST".into(), 5.0)],
        default_appearance: Map::new(),
    }
}

pub const EMOJIS: [&str; 9] = ["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9"];

pub static TABLES: VersionTables = VersionTables {
    starter_manifest: StarterManifest {
        bootstrap_version: "test-1",
        stacked_item_ids: &["STACK_A", "STACK_B"],
        quick_emoji_wheel: &EMOJIS,
    },
    ..SAMPLE_TABLES
};

pub static NO_EMOJI_TABLES: VersionTables = VersionTables {
    starter_manifest: StarterManifest {
        bootstrap_version: "test-1",
        stacked_item_ids: &["STACK_A", "STACK_B"],
        quick_emoji_wheel: &[],
    },
    ..SAMPLE_TABLES
};

struct Hooks(NoPlayerPolicy);

impl VersionHooks for Hooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        self.0
    }
    fn weapons_at_base_level(&self, settings: &Settings) -> bool {
        SampleHooks.weapons_at_base_level(settings)
    }
    fn reforge_rules(&self) -> bool {
        true
    }
    fn mastery_persistence(&self) -> bool {
        true
    }
    fn huntpass_payload(&self, id: &str) -> Value {
        SampleHooks.huntpass_payload(id)
    }
    fn objective_row(&self, user_id: &str, row: &ObjectiveRow) -> Value {
        SampleHooks.objective_row(user_id, row)
    }
    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::AsIs
    }
    fn survey_config(&self) -> StubReply {
        SampleHooks.survey_config()
    }
    fn entitlements_payload(&self, list: &[Entitlement]) -> Value {
        SampleHooks.entitlements_payload(list)
    }
    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination {
        SampleHooks.candidate_destination(hunt_id, realm_hub)
    }
}

/// What the realms seam hands back: `cap-ok` carries rifts' runtime, `cap-bare` none, anything else is not a runtime.
struct HostRealms;

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
            Ok(match capability.as_str() {
                "cap-ok" => rustic_isles_auth::RuntimeAuth::Authenticated(
                    RuntimeCaller::new(Vec::new()).with_handle(Arc::new(runtime_auth())),
                ),
                "cap-bare" => rustic_isles_auth::RuntimeAuth::Authenticated(RuntimeCaller::new(Vec::new())),
                _ => rustic_isles_auth::RuntimeAuth::NotMine,
            })
        })
    }
}

/// Admits unless given a refusal; keeps what it was asked and what it recorded.
#[derive(Default)]
pub struct FakeGuard {
    pub refusal: Mutex<Option<Reply>>,
    pub asked: Mutex<Vec<(HostWriteKind, String)>>,
    pub recorded: Mutex<Vec<HostWrite>>,
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
        Ok(true)
    }
    fn note_would_refuse(&self, _: &str, _: &str) -> Result<(), SeamError> {
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
        _: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        self.asked.lock().unwrap().push((kind, target.0));
        let refusal = self.refusal.lock().unwrap().clone();
        Box::pin(async move { Ok(refusal) })
    }
    fn record_host_write(&self, _: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        self.recorded.lock().unwrap().push(write);
        Box::pin(async { Ok(()) })
    }
    fn authorize_inventory(
        &self,
        _: RuntimeAuth,
        _: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        rustic_rifts_seam::stub(Component::Guard, "authorize_inventory")
    }
}

/// Rifts loaded with the fake guard, a stub answering every `rifts` row.
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
    pub settings: Vec<(&'static str, &'static str)>,
    pub tables: &'static VersionTables,
    pub store: bool,
    pub area: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            policy: NoPlayerPolicy::ConfiguredAccount,
            settings: Vec::new(),
            tables: &TABLES,
            store: true,
            area: true,
        }
    }
}

pub struct Answer {
    pub status: u16,
    pub content_type: Option<String>,
    pub request_id: Option<String>,
    pub body: String,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {} {}", self.status, self.body))
    }

    pub fn ok(&self) -> Value {
        assert_eq!(self.status, 200, "{}", self.body);
        self.json()
    }

    pub fn assert_empty(&self, status: u16) {
        assert_eq!(
            (self.status, self.body.as_str()),
            (status, ""),
            "{:?}",
            self.content_type
        );
        assert_eq!(self.content_type, None);
    }

    /// The loadout answer's payload.
    pub fn payload(&self) -> Value {
        self.ok()["payload"].clone()
    }
}

/// Who a request is from.
#[derive(Clone)]
pub enum Who {
    Nobody,
    Player(&'static str),
    BadBearer,
    Gameserver,
    GameserverAs(&'static str),
    Host(&'static str),
    /// `AuthData` already in the extensions, as another area's middleware leaves it
    Data(AuthData),
}

pub struct Harness {
    pub app: axum::Router,
    pub shared: Arc<Shared>,
    pub store: IslesStore,
    pub guard: Arc<FakeGuard>,
    temp: TempStore,
}

impl Harness {
    pub async fn new(options: Options) -> Self {
        let temp = TempStore::open().unwrap();
        let store = temp.store.clone();
        let version = IslesVersion {
            hooks: Arc::new(Hooks(options.policy)),
            tables: options.tables,
            routes: RouteSet::V1147,
        };
        let (private, public) = throwaway_signing_keys();
        let mut pairs: Vec<(&str, &str)> = vec![
            ("AUTH_SIGNING_PRIVKEY_B64", &private),
            ("AUTH_SIGNING_PUBKEY_B64", &public),
            ("API_KEY_HASH_SECRET", SECRET),
            ("RUSTIC_ISLES_REPLAY_HEADERS", "on"),
            ("RUSTIC_ISLES_RATE_LIMIT", "off"),
        ];
        pairs.extend(options.settings.iter().copied());
        let auth_store = Arc::new(IslesAuthStore::new(store.clone()));
        let hash = rustic_isles_auth::hashes::gameserver_key_hash(SECRET, GS_KEY);
        auth_store
            .replace_gameserver_keys(&ActorContext::new(Actor::System), &[hash])
            .await
            .unwrap();
        let guard = Arc::new(FakeGuard::default());
        let mut parts = SharedParts::bare(version);
        parts.settings = Settings::from_pairs(pairs);
        parts.auth_store = Some(auth_store);
        parts.auth_seams.realms = Arc::new(HostRealms);
        parts.rifts = rifts(Arc::clone(&guard));
        let mut slots = Slots::default();
        if options.store {
            slots.insert(Arc::new(store.clone()));
        }
        if options.area {
            slots.insert(Arc::new(Characters::new(character_data(), kit())));
        }
        parts.slots = slots;
        let mut handlers = auth_handlers().unwrap();
        rustic_isles_characters::register(&mut handlers);
        parts.handlers = handlers.clone();
        let shared = Shared::new(parts).unwrap();
        let app = rustic_isles::app(Arc::clone(&shared), Service::Backend, &handlers).unwrap();
        Self {
            app,
            shared,
            store,
            guard,
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
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "localhost")
            .header("x-isles-replay-now-ms", NOW_MS);
        match who {
            Who::Nobody | Who::Data(_) => {}
            Who::Player(user) => builder = builder.header("authorization", self.token(user)),
            Who::BadBearer => builder = builder.header("authorization", "Bearer nope"),
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
        let header = |name: &str| response.headers().get(name).map(|v| v.to_str().unwrap().to_owned());
        let content_type = header("content-type");
        let request_id = header("x-rustic-isles-request-id");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            content_type,
            request_id,
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

    /// A player's first character (made by `GET /character`); its id.
    pub async fn first_character(&self, user: &'static str) -> String {
        let list = self.get("/character", &Who::Player(user)).await.ok();
        list[0]["id"].as_str().unwrap().to_owned()
    }

    pub async fn account(&self, user: &str) -> Option<AccountRecord> {
        let user = UserId::new(user);
        self.store
            .read(move |c, r| r.accounts.find_by_user_id(c, &user))
            .await
            .unwrap()
    }

    pub async fn make_account(&self, user: &str) -> AccountId {
        let id = AccountId::new(self.store.node());
        let record = AccountRecord {
            id: id.clone(),
            user_id: UserId::new(user),
            name: user.to_owned(),
            notes: 0,
        };
        self.store
            .write(&system(), move |tx, r| r.accounts.create(tx, &record))
            .await
            .unwrap();
        id
    }

    pub async fn launcher_account(&self, account: &AccountId, user: &str, name: &str) {
        let record = launcher(account, user, name);
        self.store
            .write(&system(), move |tx, r| {
                r.launcher_accounts.create(tx, &record).map(|_| ())
            })
            .await
            .unwrap();
    }

    pub async fn journey(&self, user: &str, nodes: Value) {
        let account = match self.account(user).await {
            Some(account) => account.id,
            None => self.make_account(user).await,
        };
        let text = nodes.to_string();
        let found = {
            let id = account.clone();
            self.store
                .read(move |c, r| r.player_journey.find(c, &id))
                .await
                .unwrap()
        };
        self.store
            .write(&system(), move |tx, r| match found {
                None => r
                    .player_journey
                    .create_if_absent(
                        tx,
                        &PlayerJourneyRecord {
                            account,
                            nodes: text,
                            update_version: 0,
                            revision: 0,
                        },
                    )
                    .map(|_| ()),
                Some(row) => r
                    .player_journey
                    .update_if_revision(tx, &account, &text, row.update_version + 1, row.revision)
                    .map(|_| ()),
            })
            .await
            .unwrap();
    }

    pub async fn inventory(&self, character: &str) -> Option<InventoryRecord> {
        let character = character.to_owned();
        self.store
            .read(move |c, r| r.inventories.find_by_character_id(c, &character))
            .await
            .unwrap()
    }

    pub async fn loadout(&self, user: &str, character: &str) -> Option<LoadoutRecord> {
        let account = self.account(user).await?.id;
        let character = character.to_owned();
        self.store
            .read(move |c, r| r.loadouts.find(c, &character, &account))
            .await
            .unwrap()
    }

    pub async fn stored_data(&self, user: &str, character: &str) -> String {
        let account = self.account(user).await.unwrap().id;
        let character = character.to_owned();
        self.store
            .read(move |c, r| r.characters.find(c, &character, &account))
            .await
            .unwrap()
            .unwrap()
            .data
    }

    /// Removes a row outright (a soft-deleted key would block the re-insert, store spec §16 item 3).
    pub async fn remove_row(&self, table: &'static str, character: &str) {
        use diesel::RunQueryDsl;
        let sql = format!("DELETE FROM {table} WHERE character_id = '{character}'");
        self.store
            .write(&system(), move |tx, _| {
                diesel::sql_query(sql).execute(tx.conn())?;
                Ok(())
            })
            .await
            .unwrap();
    }

    pub async fn set_inventory(&self, character: &str, instanced: &Value, stacked: &Value) {
        let row = self.inventory(character).await.unwrap();
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

    /// `(actor, table, op)` of every change-log entry after the migrations.
    pub async fn log(&self) -> Vec<(String, String, String)> {
        self.store
            .database()
            .changes(0, 100_000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.op != "migrate")
            .map(|e| (e.actor, e.table_name, e.op))
            .collect()
    }
}

pub fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

pub fn launcher(id: &AccountId, user_id: &str, name: &str) -> LauncherAccountRecord {
    LauncherAccountRecord {
        account: id.clone(),
        user_id: UserId::new(user_id),
        email: None,
        display_name_normalized: name.to_lowercase(),
        display_name: name.to_owned(),
        password_hash: None,
        status: OperationalStatus::Active,
        approval_status: None,
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: vec!["player".into()],
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: "2026-10-01T00:00:00.000Z".into(),
        last_login_at: None,
        username_set: Some(true),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
        timeout: None,
    }
}

/// The text inside a JSON string member, parsed.
pub fn inner(value: &Value, key: &str) -> Value {
    serde_json::from_str(value[key].as_str().unwrap()).unwrap()
}

pub fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}
