// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The module over a bare composer (core spec §7, §13.2): drive, start, settings, admin and commands.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::Path;
use std::sync::Arc;

use dainty_module_api::chat::{PRESENCE_EVENT, PresenceEvent};
use dainty_module_api::network::HomeInvocation;
use dainty_module_api::stub::RecordingHost;
use dainty_module_api::{
    Actor, ActorContext, AdminCall, AdminRequest, Capabilities, CommandRequest, ErrorKind, Event, EventName,
    GameModule, GlobalId, HostCall, IdKind, InProcessModule, LogLevel, ModuleConfig, ModuleContext, ModuleId, NodeId,
    NodeIdentity, OperationName, Phase, PortRange,
};
use rustic_isles::testing::{self, Mounts, sample_version};
use rustic_isles::{BackendService, BareComposer, DefaultPorts, IslesModule, ModuleSpec};

fn spec() -> ModuleSpec {
    ModuleSpec {
        id: ModuleId::new("isles-1-14-7").unwrap(),
        display_name: "Rustic Isles 1.14.7".to_owned(),
        invite_aliases: vec!["reforged".to_owned()],
        capabilities: Capabilities::default(),
        unverified: false,
        ports: DefaultPorts {
            backend: 61000,
            director: 61001,
            qos: 61010,
            game: PortRange {
                first: 8770,
                last: 8777,
            },
        },
        version: sample_version(),
        services: vec![Arc::new(BackendService)],
        composer: Arc::new(BareComposer),
    }
}

fn module() -> Arc<IslesModule> {
    Arc::new(IslesModule::new(spec()).unwrap())
}

fn context(host: &Arc<RecordingHost>) -> ModuleContext {
    ModuleContext::new(
        ModuleId::new("isles-1-14-7").unwrap(),
        NodeIdentity::standalone(),
        host.clone(),
    )
}

/// Every port 0 and the env file, as `drive` configures them.
fn config(module: &IslesModule, env_file: Option<&Path>) -> ModuleConfig {
    let mut registrar = dainty_module_api::Registrar::new(module.manifest());
    module.register(&mut registrar).unwrap();
    let registration = registrar.finish().unwrap();
    let mut config = ModuleConfig::defaults(&module.manifest(), &registration);
    for range in config.ports.values_mut() {
        *range = PortRange::single(0);
    }
    if let Some(file) = env_file {
        config
            .sections
            .get_mut("backend")
            .unwrap()
            .insert("env_file".into(), file.display().to_string().into());
    }
    config
}

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[tokio::test]
async fn drive_a_bare_module() {
    let host = dainty_module_api::stub::recording_host();
    let driver = InProcessModule::new(module(), NodeIdentity::standalone(), host.clone()).unwrap();
    testing::drive(&driver, &host, None, &Mounts::default()).await.unwrap();
    let logs: Vec<String> = host
        .calls()
        .into_iter()
        .filter_map(|(_, call)| match call {
            HostCall::Log(record) if record.level == LogLevel::Info => Some(record.message),
            _ => None,
        })
        .collect();
    assert!(
        logs.iter()
            .any(|l| l.starts_with("Rustic Isles 1.14.7 listening: backend 127.0.0.1:"))
    );
    assert!(logs.iter().any(|l| l == "Rustic Isles 1.14.7 stopped"));
    let listening = testing::listening(&host);
    assert_eq!(listening.len(), 1);
    assert_eq!(listening[0].0, "backend");
}

#[tokio::test]
async fn the_manifest_and_registration() {
    let module = module();
    let manifest = module.manifest();
    manifest.validate().unwrap();
    let names: Vec<&str> = manifest.ports.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["backend", "director", "qos", "game"]);
    assert_eq!(manifest.game_version.to_string(), "1.14.7");
    let mut registrar = dainty_module_api::Registrar::new(manifest.clone());
    module.register(&mut registrar).unwrap();
    let registration = registrar.finish().unwrap();
    registration.check(&manifest).unwrap();
    assert_eq!(registration.routes.len(), 6);
    assert!(registration.routes[1..].iter().all(|r| r.host_only));
    let sections: Vec<&str> = registration.config.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(sections, ["backend", "switches", "game_data"]);
    let switches = &registration.config[1].keys;
    assert_eq!(switches.len(), 11);
    assert_eq!(switches[0].default, serde_json::json!("80"));
    assert_eq!(switches[5].help, "Optional host switch (RUSTIC_ISLES_ACCOUNT_ERASURE)");
}

#[tokio::test]
async fn start_twice_and_bad_settings() {
    let host = dainty_module_api::stub::recording_host();
    let module = module();
    let ctx = context(&host);
    module.start(ctx.clone(), config(&module, None)).await.unwrap();
    assert_eq!(module.status().phase, Phase::Ready);
    let again = module.start(ctx.clone(), config(&module, None)).await.unwrap_err();
    assert_eq!(
        (again.kind, again.message.as_str()),
        (ErrorKind::Invalid, "already started")
    );
    module.stop(ctx.clone()).await.unwrap();
    assert_eq!(module.status().phase, Phase::Stopped);

    let dir = tempfile::tempdir().unwrap();
    let env = write(dir.path(), "x.env", "RUSTIC_ISLES_ADMIN_API=yes\n");
    let failing = self::module();
    let error = failing.start(ctx, config(&failing, Some(&env))).await.unwrap_err();
    let text = "RUSTIC_ISLES_ADMIN_API: must be on or off (blank follows RUSTIC_ISLES_MANAGER_ADMIN)";
    assert_eq!(error.message, text);
    let status = failing.status();
    assert_eq!((status.phase, status.detail.as_deref()), (Phase::Failed, Some(text)));
}

#[test]
fn env_files_merge() {
    let dir = tempfile::tempdir().unwrap();
    let backend = write(
        dir.path(),
        "backend.env",
        "PORT=61000\nDEPLOYSERVER_URL=http://127.0.0.1:61001\nREGISTRATION_MODE=open\n",
    );
    write(
        dir.path(),
        "admin.env",
        "REFORGED_DB_ENGINE=sqlite\nREFORGED_DB_FILE=reforged.db\nPORT=61003\n",
    );
    write(
        dir.path(),
        "director.env",
        "PORT=61001\nMY_IP=127.0.0.1\nPORT_RANGE_BEGIN=8766\nPORT_RANGE_END=8777\nGAMESERVER_BINARY_PATH=x\nMETAGAME_API_KEY=k\n",
    );
    let module = module();
    let mut config = config(&module, Some(&backend));
    let settings = |config: &ModuleConfig| module.settings(config).unwrap();
    let with_range = |config: &mut ModuleConfig, first, last| {
        config.ports.insert("game".into(), PortRange { first, last });
    };
    with_range(&mut config, 8770, 8777);
    let s = settings(&config);
    assert_eq!(s.raw("PORT"), Some("61000"));
    assert_eq!(s.raw("REFORGED_DB_ENGINE"), Some("sqlite"));
    assert_eq!(s.raw("METAGAME_API_KEY"), Some("k"));
    assert!(s.database_wanted());
    assert_eq!(
        (s.raw("PORT_RANGE_BEGIN"), s.raw("PORT_RANGE_END")),
        (Some("8766"), Some("8777"))
    );
    with_range(&mut config, 8778, 8785);
    let s = settings(&config);
    assert_eq!(
        (s.raw("PORT_RANGE_BEGIN"), s.raw("PORT_RANGE_END")),
        (Some("8778"), Some("8785"))
    );

    let mut plain = self::config(&module, None);
    with_range(&mut plain, 8770, 8777);
    let s = settings(&plain);
    assert_eq!(
        (s.raw("PORT_RANGE_BEGIN"), s.raw("PORT_RANGE_END")),
        (Some("8770"), Some("8777"))
    );
    assert_eq!(s.raw("QOS_PORT"), Some("80"));
    assert_eq!(s.raw("RUSTIC_ISLES_RATE_LIMIT"), None);

    let other = write(dir.path(), "other.env", "PORT=1\n");
    let s = settings(&self::config(&module, Some(&other)));
    assert_eq!((s.raw("PORT"), s.raw("REFORGED_DB_ENGINE")), (Some("1"), None));
    let missing = self::config(&module, Some(&dir.path().join("none.env")));
    assert!(module.settings(&missing).unwrap_err().message.contains("none.env"));
}

#[tokio::test]
async fn admin_actions() {
    let host = dainty_module_api::stub::recording_host();
    let module = module();
    let ctx = context(&host);
    let call = |request| AdminCall {
        actor: ActorContext::new(Actor::Console),
        request,
    };
    let ban = |until: &str| {
        call(AdminRequest::Ban {
            player: "p".into(),
            reason: None,
            until: Some(until.into()),
        })
    };
    let past = module
        .admin(ctx.clone(), ban("2001-01-01T00:00:00Z"))
        .await
        .unwrap_err();
    assert_eq!(
        (past.kind, past.message.as_str()),
        (ErrorKind::Invalid, "a timeout ends in the future")
    );
    let bad = module.admin(ctx.clone(), ban("tomorrow")).await.unwrap_err();
    assert_eq!(
        (bad.kind, bad.message.as_str()),
        (ErrorKind::Invalid, "a timeout ends at an RFC 3339 time")
    );
    let kick = module
        .admin(
            ctx,
            call(AdminRequest::Kick {
                player: "p".into(),
                reason: None,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (kick.kind, kick.message.as_str()),
        (ErrorKind::Unavailable, "the module has no database")
    );
}

fn home_command(account: &GlobalId) -> CommandRequest {
    HomeInvocation {
        from: NodeId::new("peer").unwrap(),
        account: account.clone(),
        actor: ActorContext::new(Actor::System),
        operation: OperationName::new("progression.read").unwrap(),
        body: serde_json::Value::Null,
    }
    .to_command()
}

#[tokio::test]
async fn commands_and_events() {
    let host = dainty_module_api::stub::recording_host();
    let module = module();
    let ctx = context(&host);
    let unknown = || CommandRequest {
        actor: ActorContext::new(Actor::Console),
        name: "nope".into(),
        args: vec![],
    };
    let idle = module.command(ctx.clone(), unknown()).await.unwrap_err();
    assert_eq!(
        (idle.kind, idle.message.as_str()),
        (ErrorKind::Unavailable, "the module is not running")
    );

    let dir = tempfile::tempdir().unwrap();
    let env = write(dir.path(), "db.env", "REFORGED_DB_FILE=isles.db\n");
    module.start(ctx.clone(), config(&module, Some(&env))).await.unwrap();
    let unsupported = module.command(ctx.clone(), unknown()).await.unwrap_err();
    assert_eq!(
        (unsupported.kind, unsupported.message.as_str()),
        (ErrorKind::Unsupported, "command nope")
    );

    let elsewhere = GlobalId::new(IdKind::Account, &NodeId::new("faraway").unwrap());
    let refused = module.command(ctx.clone(), home_command(&elsewhere)).await.unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Invalid);
    assert_eq!(
        refused.message,
        format!("account {elsewhere} is homed on faraway, not local")
    );
    let hunt = GlobalId::new(IdKind::Hunt, &NodeId::new("local").unwrap());
    let not_account = module.command(ctx.clone(), home_command(&hunt)).await.unwrap_err();
    assert_eq!(
        (not_account.kind, not_account.message.as_str()),
        (ErrorKind::Invalid, "an account id is account:<ULID>@<node>")
    );
    let local = GlobalId::new(IdKind::Account, &NodeId::new("local").unwrap());
    let bare = module.command(ctx.clone(), home_command(&local)).await.unwrap_err();
    assert_eq!(bare.kind, ErrorKind::Unavailable);

    let director = dainty_module_api::network::DirectorInvocation::PlaceHunt {
        match_id: hunt.clone(),
        queue: "q".into(),
        game: "1.14.7".parse().unwrap(),
        players: vec![],
    };
    let unconfigured = module.command(ctx.clone(), director.to_command()).await.unwrap_err();
    assert_eq!(
        (unconfigured.kind, unconfigured.message.as_str()),
        (
            ErrorKind::Unavailable,
            "dedicated servers are not configured on this node"
        )
    );

    let presence = |online| Event {
        name: EventName::new(PRESENCE_EVENT).unwrap(),
        payload: serde_json::to_value(PresenceEvent {
            account: local.clone(),
            user_id: "u1".into(),
            online,
        })
        .unwrap(),
    };
    module.event(ctx.clone(), presence(true)).await.unwrap();
    let garbage = Event {
        name: EventName::new(PRESENCE_EVENT).unwrap(),
        payload: serde_json::json!({"nope": 1}),
    };
    module.event(ctx.clone(), garbage).await.unwrap();
    module.event(ctx.clone(), presence(false)).await.unwrap();
    module.stop(ctx).await.unwrap();
}
