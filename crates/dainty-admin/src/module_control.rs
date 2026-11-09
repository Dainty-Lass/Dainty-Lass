//! The registry's own list, load and unload (DF3), apart from `modules.rs` (DF4c's kick, ban and lookup
//! on the modules). The game modules and plugins `[modules]` lists, for the host's tools: `modules` and `module`
//! to read them, `loadModule`, `unloadModule` and `restartModule` to act on one (`schema.rs` roots). Only a listed module can
//! be loaded, never a path (DD5). The controller hands the registry in as a [`ModuleControl`]; a
//! standalone admin API (`dainty-host admin`) has none and says so.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_graphql::{Context, ErrorExtensions, SimpleObject};
use dainty_module_api::{BoxFuture, GameAnswer, GameCall, GameVersion, ModuleError};

/// What a [`ModuleControl`] action returns: the module after it, or why it was refused.
pub type Pending<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// A port a module was given: its default, or where the controller moved it.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct ModulePort {
    /// What the module calls the port.
    pub name: String,
    /// The first port of its range.
    pub first: u16,
    /// The last port of its range, inclusive.
    pub last: u16,
}

/// A module `[modules]` lists, as the registry sees it now.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct Module {
    /// The module's id.
    pub id: String,
    /// not_loaded, starting, ready or failed
    pub state: String,
    /// compiled_in or external
    pub source: String,
    /// Whether the server loads it at start.
    pub autoload: bool,
    /// Known once it loaded, or at once for a compiled-in one
    pub display_name: Option<String>,
    /// The game version it serves, once known.
    pub game_version: Option<String>,
    /// game or plugin
    pub kind: Option<String>,
    /// Built but not checked against recorded traffic (DD9).
    pub unverified: bool,
    /// Why it failed or was unloaded.
    pub detail: Option<String>,
    /// The ports it was given.
    pub ports: Vec<ModulePort>,
    /// The commands it registered.
    pub commands: Vec<String>,
}

/// What the admin API may do with the registry.
pub trait ModuleControl: Send + Sync {
    /// Every listed module.
    fn list(&self) -> Vec<Module>;
    /// One listed module, or why there is none.
    fn get(&self, id: &str) -> Result<Module, String>;
    /// Loads a listed module.
    fn load<'a>(&'a self, id: &'a str) -> Pending<'a, Module>;
    /// Unloads a module.
    fn unload<'a>(&'a self, id: &'a str) -> Pending<'a, Module>;
    /// Unloads and loads a module again.
    fn restart<'a>(&'a self, id: &'a str) -> Pending<'a, Module>;
    /// host.toml's settings the dashboard edits, when the host kit handed the file in
    fn host_settings(&self) -> Option<Arc<dyn crate::host_settings::HostSettings>> {
        None
    }
    /// One game call to the module serving `version` (`ModuleRegistry::game`, MS2).
    fn game<'a>(&'a self, version: &'a GameVersion, call: GameCall) -> BoxFuture<'a, Result<GameAnswer, ModuleError>> {
        let _ = (version, call);
        Box::pin(async { Err(ModuleError::unsupported("game calls")) })
    }
    /// Every ready game module's servers (`ModuleRegistry::game_servers`, MS2).
    fn game_servers(&self) -> BoxFuture<'_, Vec<crate::game::GameServers>> {
        Box::pin(async { Vec::new() })
    }
    /// The recorder's events after `after`, oldest first, only `names` when given (A5, MS5).
    fn events(
        &self,
        after: i64,
        names: Vec<String>,
        limit: i64,
    ) -> BoxFuture<'_, Result<Vec<crate::events::KeptEvent>, ModuleError>> {
        let _ = (after, names, limit);
        Box::pin(async { Err(ModuleError::unsupported("kept events")) })
    }
}

/// The registry, when the admin API runs inside the server.
#[derive(Clone, Default)]
pub struct Modules(pub Option<Arc<dyn ModuleControl>>);

fn fail(code: &str, message: impl Into<String>) -> async_graphql::Error {
    async_graphql::Error::new(message).extend_with(|_, e| e.set("code", code))
}

pub(crate) fn control<'a>(ctx: &'a Context<'_>) -> async_graphql::Result<&'a Arc<dyn ModuleControl>> {
    ctx.data::<Modules>()?.0.as_ref().ok_or_else(|| {
        fail(
            "MODULES_UNAVAILABLE",
            "this admin API runs outside the server, so it has no modules: run it through dainty-host serve",
        )
    })
}

/// The `modules` query; a coded error when the admin API runs outside the server.
pub fn list(ctx: &Context<'_>) -> async_graphql::Result<Vec<Module>> {
    Ok(control(ctx)?.list())
}

/// The `module` query: the module, or null when none has that id.
pub fn get(ctx: &Context<'_>, id: &str) -> async_graphql::Result<Option<Module>> {
    Ok(control(ctx)?.get(id).ok())
}

/// The `loadModule` mutation.
pub async fn load(ctx: &Context<'_>, id: &str) -> async_graphql::Result<Module> {
    control(ctx)?.load(id).await.map_err(|e| fail("MODULE_REFUSED", e))
}

/// The `unloadModule` mutation.
pub async fn unload(ctx: &Context<'_>, id: &str) -> async_graphql::Result<Module> {
    control(ctx)?.unload(id).await.map_err(|e| fail("MODULE_REFUSED", e))
}

/// The `restartModule` mutation.
pub async fn restart(ctx: &Context<'_>, id: &str) -> async_graphql::Result<Module> {
    control(ctx)?.restart(id).await.map_err(|e| fail("MODULE_REFUSED", e))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::sync::Mutex;

    use async_graphql::{EmptySubscription, Object, Schema};
    use serde_json::json;

    use super::*;

    struct Fake(Mutex<String>);

    impl Fake {
        fn module(&self) -> Module {
            Module {
                id: "stub".into(),
                state: self.0.lock().unwrap().clone(),
                source: "compiled_in".into(),
                autoload: true,
                display_name: Some("Stub".into()),
                game_version: Some("0.0.1".into()),
                kind: Some("game".into()),
                unverified: true,
                detail: None,
                ports: vec![ModulePort {
                    name: "http".into(),
                    first: 65000,
                    last: 65000,
                }],
                commands: vec!["echo".into()],
            }
        }
    }

    impl ModuleControl for Fake {
        fn list(&self) -> Vec<Module> {
            vec![self.module()]
        }

        fn get(&self, id: &str) -> Result<Module, String> {
            (id == "stub")
                .then(|| self.module())
                .ok_or_else(|| format!("no module {id}"))
        }

        fn load<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
            Box::pin(async move {
                if id != "stub" {
                    return Err(format!("no module {id} is listed in host.toml ([modules])"));
                }
                *self.0.lock().unwrap() = "ready".into();
                Ok(self.module())
            })
        }

        fn unload<'a>(&'a self, _id: &'a str) -> Pending<'a, Module> {
            Box::pin(async move {
                *self.0.lock().unwrap() = "not_loaded".into();
                Ok(self.module())
            })
        }

        fn restart<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
            self.load(id)
        }
    }

    struct Q;

    #[Object]
    impl Q {
        async fn modules(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Module>> {
            list(ctx)
        }

        async fn module(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<Option<Module>> {
            get(ctx, &id)
        }
    }

    struct M;

    #[Object]
    impl M {
        async fn load_module(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<Module> {
            load(ctx, &id).await
        }
    }

    async fn run(modules: Modules, query: &str) -> serde_json::Value {
        let schema = Schema::build(Q, M, EmptySubscription).data(modules).finish();
        serde_json::to_value(schema.execute(query).await).unwrap()
    }

    #[tokio::test]
    async fn the_host_reads_and_drives_the_listed_modules() {
        let fake = Modules(Some(Arc::new(Fake(Mutex::new("not_loaded".into())))));
        let listed = run(
            fake.clone(),
            "{ modules { id state gameVersion ports { name first } } }",
        )
        .await;
        assert_eq!(
            listed["data"]["modules"],
            json!([{"id": "stub", "state": "not_loaded", "gameVersion": "0.0.1", "ports": [{"name": "http", "first": 65000}]}])
        );
        let loaded = run(fake.clone(), "mutation { loadModule(id: \"stub\") { state } }").await;
        assert_eq!(loaded["data"]["loadModule"]["state"], "ready");
        let refused = run(fake.clone(), "mutation { loadModule(id: \"../x\") { state } }").await;
        assert_eq!(refused["errors"][0]["extensions"]["code"], "MODULE_REFUSED");
        assert_eq!(
            run(fake, "{ module(id: \"nobody\") { id } }").await["data"]["module"],
            json!(null)
        );
        // Outside the server there is no registry, and the API says why
        let alone = run(Modules::default(), "{ modules { id } }").await;
        assert_eq!(alone["errors"][0]["extensions"]["code"], "MODULES_UNAVAILABLE");
    }
}
