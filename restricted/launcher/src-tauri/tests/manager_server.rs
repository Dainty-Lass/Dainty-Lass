//! The oracle for the manager's MIT embedded server (`dainty-manager-server`, spec-tauri-shell §12.1): the module
//! panel and history tests the restricted `server_gui` and `server_history` carried, run against the MIT crate.
//! Differences the clean-room review accepted are in `docs/port/clean-room/manager-server.md`.

#![cfg(feature = "server-gui")]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod modules {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use dainty_launcher_kit::bridge::modules::ModulePanel;
    use dainty_launcher_kit::bridge::types::{
        ErrorCode, LauncherError, ModuleCommandOutcome, ModuleListOutcome, ModuleOutcome, ModuleRunState,
        ModuleSettingValue, ModuleView,
    };
    use dainty_manager_server::ServerModules;
    use dainty_module_api::GameModule;
    use dainty_module_api::stub::StubModule;
    use dainty_service_kit::context::StatusRegistry;
    use serde_json::json;
    use server_controller::{ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions};

    const HOST_TOML: &str = "version = 1\nmode = \"private\"\nname = \"Friday Hunts\"\nhost = \"100.101.102.103\"\n\
        game_address = \"100.101.102.103\"\nregistration = \"INVITECODE\"\nmax_hunt_servers = 3\n";

    struct Fixture {
        panel: ServerModules,
        registry: ModuleRegistry,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn host_toml(&self) -> std::path::PathBuf {
            dainty_host::paths::Paths::new(self.dir.path()).host_toml
        }
    }

    fn running() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let registry = ModuleRegistry::new(
            RegistryOptions::standalone(),
            StatusRegistry::new("private", Vec::new()),
        );
        registry
            .add_entry(ModuleEntry {
                id: "stub".parse().unwrap(),
                source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(StubModule::new()) as Arc<dyn GameModule>)),
                autoload: false,
                settings: BTreeMap::new(),
            })
            .unwrap();
        let panel = ServerModules::new();
        panel.attach_registry(Some(registry.clone()));
        panel.set_workspace(Some(dir.path().to_path_buf()));
        let fixture = Fixture { panel, registry, dir };
        let file = fixture.host_toml();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, HOST_TOML).unwrap();
        fixture
    }

    fn shared(panel: &ServerModules) -> Arc<dyn ModulePanel> {
        Arc::new(panel.clone())
    }
    fn module(outcome: ModuleOutcome) -> ModuleView {
        match outcome {
            ModuleOutcome::Ok(view) => *view,
            ModuleOutcome::Err(error) => panic!("{error:?}"),
        }
    }

    fn refused(outcome: ModuleOutcome) -> (ErrorCode, String) {
        match outcome {
            ModuleOutcome::Err(error) => (error.code, error.detail.unwrap_or_default()),
            ModuleOutcome::Ok(view) => panic!("{view:?}"),
        }
    }

    fn said(outcome: ModuleCommandOutcome) -> Result<(String, Option<String>), (ErrorCode, String)> {
        match outcome {
            ModuleCommandOutcome::Ok { text, data } => Ok((text, data)),
            ModuleCommandOutcome::Err(error) => Err((error.code, error.detail.unwrap_or_default())),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stopped_server_has_no_modules() {
        let panel: Arc<dyn ModulePanel> = Arc::new(ServerModules::new());
        assert_eq!(
            panel.list().await,
            ModuleListOutcome::Err(LauncherError::new(ErrorCode::ServerStopped))
        );
        assert_eq!(refused(panel.load("stub".into()).await).0, ErrorCode::ServerStopped);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_listed_modules_load_run_a_command_and_unload() {
        let f = running();
        let panel = shared(&f.panel);
        let ModuleListOutcome::Ok(listed) = panel.list().await else {
            panic!("the list")
        };
        assert_eq!(listed.len(), 1);
        // A compiled-in module's manifest is known before it loads; its commands and settings are not
        let before = &listed[0];
        assert_eq!(before.state, ModuleRunState::NotLoaded);
        assert_eq!(before.display_name.as_deref(), Some("Stub"));
        assert!(before.unverified);
        assert!(before.commands.is_empty() && before.settings.is_empty());

        let ready = module(panel.load("stub".into()).await);
        assert_eq!(ready.state, ModuleRunState::Ready);
        assert!(ready.commands.iter().any(|c| c.name == "echo"));
        assert_eq!(ready.settings[0].keys[0].name, "greeting");
        assert_eq!(
            refused(panel.load("stub".into()).await),
            (ErrorCode::ModuleBadState, "loaded".into())
        );

        let echo = |input: serde_json::Value| panel.run("stub".into(), "echo".into(), input.to_string());
        assert_eq!(said(echo(json!({ "words": "there" })).await).unwrap().0, "hello there");
        assert_eq!(said(echo(json!(null)).await).unwrap().0, "hello");
        assert_eq!(
            said(echo(json!({ "words": 7 })).await).unwrap_err(),
            (ErrorCode::ModuleInputInvalid, "argument_text:words".into())
        );
        assert_eq!(
            said(panel.run("stub".into(), "nope".into(), "{}".into()).await).unwrap_err(),
            (ErrorCode::ModuleInputInvalid, "command".into())
        );
        assert_eq!(
            said(panel.run("stub".into(), "fail".into(), "null".into()).await)
                .unwrap_err()
                .0,
            ErrorCode::ModuleFailed
        );

        let stopped = module(panel.unload("stub".into()).await);
        assert_eq!(stopped.state, ModuleRunState::NotLoaded);
        assert_eq!(
            refused(panel.unload("stub".into()).await),
            (ErrorCode::ModuleBadState, "not_loaded".into())
        );
        assert_eq!(
            said(echo(json!(null)).await).unwrap_err(),
            (ErrorCode::ModuleBadState, "not_loaded".into())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_listed_ids_are_modules() {
        let f = running();
        let panel = shared(&f.panel);
        for id in ["nope", "../stub", "C:\\modules\\stub.exe", "Stub", ""] {
            let (code, _) = refused(panel.load(id.into()).await);
            assert_eq!(code, ErrorCode::ModuleUnknown, "{id}");
            assert_eq!(refused(panel.get(id.into()).await).0, ErrorCode::ModuleUnknown, "{id}");
            assert_eq!(
                said(panel.run(id.into(), "echo".into(), "null".into()).await)
                    .unwrap_err()
                    .0,
                ErrorCode::ModuleUnknown,
                "{id}"
            );
        }
        assert_eq!(f.registry.list().len(), 1, "nothing was added");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_are_saved_to_host_toml_and_taken_by_the_next_load() {
        let f = running();
        let panel = shared(&f.panel);
        module(panel.load("stub".into()).await);
        let saved = module(
            panel
                .set_settings("stub".into(), json!({ "stub": { "greeting": "hey" } }).to_string())
                .await,
        );
        assert_eq!(saved.settings[0].keys[0].value, ModuleSettingValue::Text("hey".into()));
        assert_eq!(
            saved.settings[0].keys[0].default,
            ModuleSettingValue::Text("hello".into())
        );
        let text = std::fs::read_to_string(f.host_toml()).unwrap();
        assert!(
            text.contains("[modules.stub.stub]") && text.contains("greeting = \"hey\""),
            "{text}"
        );
        assert!(
            text.contains("name = \"Friday Hunts\""),
            "the rest of the file stays: {text}"
        );

        // A running module keeps what it started with; a restart takes the new value
        let echo = || panel.run("stub".into(), "echo".into(), "null".into());
        assert_eq!(said(echo().await).unwrap().0, "hello");
        module(panel.unload("stub".into()).await);
        module(panel.load("stub".into()).await);
        assert_eq!(said(echo().await).unwrap().0, "hey");

        // What does not fit is refused and the file is as it was
        let before = std::fs::read_to_string(f.host_toml()).unwrap();
        for (values, why) in [
            (json!({ "stub": { "greeting": 3 } }), "type:stub.greeting"),
            (json!({ "stub": { "nope": "x" } }), "unknown_key:stub.nope"),
            (json!({ "other": { "greeting": "x" } }), "unknown_section"),
        ] {
            assert_eq!(
                refused(panel.set_settings("stub".into(), values.to_string()).await),
                (ErrorCode::ModuleSettingsInvalid, why.into())
            );
        }
        assert_eq!(std::fs::read_to_string(f.host_toml()).unwrap(), before);
        // And a module that has not loaded has registered nothing to set
        module(panel.unload("stub".into()).await);
        assert_eq!(
            refused(
                panel
                    .set_settings("stub".into(), json!({ "stub": { "greeting": "x" } }).to_string())
                    .await
            )
            .0,
            ErrorCode::ModuleSettingsInvalid
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_need_a_host_toml() {
        let f = running();
        std::fs::remove_file(f.host_toml()).unwrap();
        let panel = shared(&f.panel);
        module(panel.load("stub".into()).await);
        assert_eq!(
            refused(
                panel
                    .set_settings("stub".into(), json!({ "stub": { "greeting": "x" } }).to_string())
                    .await
            ),
            (ErrorCode::ModuleFailed, "host_toml".into())
        );
    }
}

mod history {
    use std::path::Path;

    use dainty_admin::history::{self, Actor};
    use dainty_admin::keys::{Keys, Role};
    use dainty_launcher_kit::bridge::checks::HistoryFilter as HistoryArgs;
    use dainty_launcher_kit::bridge::types::ErrorCode;
    use dainty_manager_server::history::{apply, dry_run, query};

    /// A workspace with the admin API's database: two moderators, and a partner who revoked both.
    async fn workspace() -> (tempfile::TempDir, Vec<String>, String) {
        let dir = tempfile::tempdir().unwrap();
        let paths = dainty_host::paths::Paths::new(dir.path());
        std::fs::create_dir_all(&paths.config).unwrap();
        let slashed = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
        let key = "0123456789abcdef0123456789abcdef";
        let env = format!(
            "DAINTY_ADMIN_API_KEY={key}\nDAINTY_ADMIN_KEY={key}\nRUSTIC_ISLES_MANAGER_OPERATOR_SECRET={key}\n\
             DAINTY_ADMIN_API_TIMEOUTS_FILE={}\nDAINTY_DB_ENGINE=sqlite\nDAINTY_DB_FILE={}\n",
            slashed(&paths.config.join("timeouts.json")),
            slashed(&paths.config.join("dainty-lass.db")),
        );
        std::fs::write(paths.config.join("admin.env"), env).unwrap();
        let node = history::node_database(Some(&paths.config.join("admin.env"))).unwrap();
        let keys = Keys::open(&Keys::database_for(&node), key).unwrap();
        let host = Actor::Host("test".into());
        let (maya, _) = keys.issue("Maya", Role::Moderator, host.clone(), 1).await.unwrap();
        let (lee, _) = keys.issue("Lee", Role::Moderator, host.clone(), 1).await.unwrap();
        let (partner, _) = keys.issue("Partner", Role::Owner, host, 1).await.unwrap();
        for id in [&maya.id, &lee.id] {
            keys.revoke(id, Actor::Admin(partner.id.clone()), 2).await.unwrap();
        }
        (dir, vec![maya.id, lee.id], partner.id)
    }

    fn by(actor: &str) -> HistoryArgs {
        HistoryArgs {
            actor: Some(actor.to_owned()),
            ..HistoryArgs::default()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_page_reverts_a_partners_changes_with_the_plan_it_was_shown() {
        let (dir, revoked, partner) = workspace().await;
        let Ok(page) = query(dir.path(), &by(&format!("admin:{partner}"))).await else {
            panic!("a page");
        };
        assert_eq!(page.entries.len(), 2);
        assert!(
            page.entries
                .iter()
                .all(|e| e.table == "admin_keys" && e.database == "admin")
        );
        assert!(
            page.entries
                .iter()
                .all(|e| !e.after.as_deref().unwrap_or("").contains("key_hash\":\"0"))
        );

        let Ok(plan) = dry_run(dir.path(), &by(&format!("admin:{partner}"))).await else {
            panic!("a plan");
        };
        assert_eq!((plan.action_count, plan.conflict_count), (2, 0));
        assert!(
            plan.actions
                .iter()
                .all(|a| a.kind == "update" && revoked.iter().any(|id| a.key.contains(id)))
        );

        // Only a plan this page was shown is applied
        let Err(unknown) = apply(dir.path(), &"f".repeat(64)).await else {
            panic!("refused");
        };
        assert_eq!(unknown.code, ErrorCode::HistoryPlanChanged);
        let Ok(done) = apply(dir.path(), &plan.id).await else {
            panic!("applied");
        };
        assert_eq!((done.rows_changed, done.conflict_count), (2, 0));
        assert_eq!(done.databases[0].database, "admin");
        // The plan is spent
        assert!(apply(dir.path(), &plan.id).await.is_err());
        // The revert is in the history as the server GUI
        let Ok(page) = query(dir.path(), &by("host:server-gui")).await else {
            panic!("a page");
        };
        assert_eq!(page.entries.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bad_filter_and_a_workspace_without_a_server_are_refused_with_codes() {
        let (dir, _, _) = workspace().await;
        let Err(bad) = query(dir.path(), &by("nobody-knows")).await else {
            panic!("refused");
        };
        assert_eq!(bad.code, ErrorCode::HistoryBadFilter);
        let Err(nothing) = dry_run(dir.path(), &HistoryArgs::default()).await else {
            panic!("refused");
        };
        assert_eq!(nothing.code, ErrorCode::HistoryBadFilter);

        let empty = tempfile::tempdir().unwrap();
        let Err(unset) = query(empty.path(), &HistoryArgs::default()).await else {
            panic!("refused");
        };
        assert_eq!(unset.code, ErrorCode::HistoryUnavailable);
    }
}
