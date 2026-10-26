//! The Rust services this binary was built with, as modules: each reads the env file the host kit
//! wrote for it (`hostconfig.rs`), through its own `Config::from_env`, so a service in this process
//! sees exactly what it saw as a process of its own.

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "admin"))]
use std::sync::Arc;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "admin"))]
use service_kit::env::Env;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "admin"))]
use crate::Module;

/// The modules built into this binary, for the message an empty build prints.
pub const BUILT_IN: &[&str] = &[
    #[cfg(feature = "gateway")]
    "gateway",
    #[cfg(feature = "allowlist")]
    "allowlist",
    #[cfg(feature = "content")]
    "content",
    #[cfg(feature = "admin")]
    "admin",
];

/// Every module the crate can build, for the same message.
pub const ALL: &[&str] = &["gateway", "allowlist", "content", "admin"];

/// Where the allowlist module leaves its helper for the gateway module's in-process feed.
#[cfg(feature = "allowlist")]
pub type AllowlistSlot = Arc<std::sync::Mutex<Option<Arc<gateway::allowlist::helper::Helper>>>>;

#[cfg(feature = "allowlist")]
pub fn allowlist_slot() -> AllowlistSlot {
    Arc::new(std::sync::Mutex::new(None))
}

/// The allowlist helper (`allowlist.env`). Started before the Director in public mode, so the game
/// ports are closed before any game server listens (what `--close-ports` did for a process).
#[cfg(feature = "allowlist")]
pub fn allowlist(env: &Env, slot: AllowlistSlot) -> Result<Module, String> {
    use gateway::allowlist::config::AllowlistConfig;
    let config = AllowlistConfig::from_env(env)?;
    Ok(Module::new(
        "allowlist",
        Arc::new(move |ctx| {
            let (config, slot) = (config.clone(), slot.clone());
            Box::pin(async move { gateway::allowlist::helper::run_with_context(config, ctx, Some(slot)).await })
        }),
    )
    .ready_within(std::time::Duration::from_secs(60)))
}

/// The gateway (`gateway.env`). With `slot`, logged-in players' addresses go to the allowlist
/// module directly; without it, over loopback HTTP as `GATEWAY_ALLOWLIST` says (an external helper).
#[cfg(feature = "gateway")]
pub fn gateway(env: &Env, slot: Option<AllowlistSlot>) -> Result<Module, String> {
    use gateway::config::Config;
    let config = Config::from_env(env)?;
    let env = env.clone();
    Ok(Module::new(
        "gateway",
        Arc::new(move |ctx| {
            let (config, env, slot) = (config.clone(), env.clone(), slot.clone());
            Box::pin(async move {
                let capture = gateway::server::load_capture(&env)?;
                let sink: Option<gateway::feed::InProcessSink> = slot.map(|slot| {
                    Arc::new(move |ip: &str| {
                        let guard = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.as_ref().is_some_and(|helper| helper.allow_ip(ip))
                    }) as gateway::feed::InProcessSink
                });
                gateway::server::run_with_context(config, capture, sink, ctx).await
            })
        }),
    )
    .ready_within(std::time::Duration::from_secs(60)))
}

#[cfg(all(feature = "gateway", not(feature = "allowlist")))]
pub type AllowlistSlot = Arc<std::sync::Mutex<Option<Arc<gateway::allowlist::helper::Helper>>>>;

/// The content server (`content.env`).
#[cfg(feature = "content")]
pub fn content(env: &Env) -> Result<Module, String> {
    let config = content::config::Config::from_env(env)?;
    Ok(Module::new(
        "content",
        Arc::new(move |ctx| {
            let config = config.clone();
            Box::pin(async move { content::server::run_with_context(config, ctx).await })
        }),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(60)))
}

/// The admin API (`admin.env`).
#[cfg(feature = "admin")]
pub fn admin(env: &Env) -> Result<Module, String> {
    let config = admin::config::Config::from_env(env)?;
    Ok(Module::new(
        "admin",
        Arc::new(move |ctx| {
            let config = config.clone();
            Box::pin(async move { admin::server::run_with_context(config, ctx).await })
        }),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(60)))
}
