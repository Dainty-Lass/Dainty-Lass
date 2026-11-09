//! [`ModuleDriver`]: the host's one view of a loaded module, whether it is compiled in
//! ([`InProcessModule`]) or a child process (`host::ChildModule`). The controller (DF3) holds drivers and
//! never needs to know which.

use std::sync::Arc;

use crate::chat::{ChatAnswer, ChatQuery};
use crate::frame::{ModuleAnswer, ModuleCall};
use crate::game::{GameAnswer, GameCall};
use crate::manifest::ModuleManifest;
use crate::module::{
    AdminCall, AdminResponse, BoxFuture, CommandOutput, CommandRequest, ErrorKind, Event, GameModule, HostLink,
    ModuleConfig, ModuleContext, ModuleError, ModuleStatus,
};
use crate::network::NodeIdentity;
use crate::registrar::{Registrar, Registration};

/// A loaded module as the host drives it.
pub trait ModuleDriver: Send + Sync {
    /// The module's manifest, as loaded.
    fn manifest(&self) -> &ModuleManifest;

    /// Runs one call. Everything else on this trait is a typed wrapper around it.
    fn call(&self, call: ModuleCall) -> BoxFuture<'_, Result<ModuleAnswer, ModuleError>>;

    /// Stops the module for good: an in-process module is stopped, a child is stopped, given its grace
    /// period and then killed (DD5).
    fn unload(&self) -> BoxFuture<'_, Result<(), ModuleError>>;
}

fn wrong(answer: &ModuleAnswer) -> ModuleError {
    ModuleError::new(ErrorKind::Protocol, format!("the module answered {answer:?}"))
}

/// The typed calls, for any driver. The registration is checked against the manifest here, so a
/// compiled-in module is held to the same rules as an external one.
pub trait ModuleDriverExt: ModuleDriver {
    /// What the module registers, checked against its manifest; a mismatch is [`ErrorKind::Invalid`].
    fn register(&self) -> impl Future<Output = Result<Registration, ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Register).await? {
                ModuleAnswer::Registered(registration) => {
                    registration
                        .check(self.manifest())
                        .map_err(|error| ModuleError::new(ErrorKind::Invalid, error.to_string()))?;
                    Ok(registration)
                }
                other => Err(wrong(&other)),
            }
        }
    }

    /// Starts the module with its configuration.
    fn start(&self, config: ModuleConfig) -> impl Future<Output = Result<(), ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Start(config)).await? {
                ModuleAnswer::Started => Ok(()),
                other => Err(wrong(&other)),
            }
        }
    }

    /// Stops the module; it can be started again.
    fn stop(&self) -> impl Future<Output = Result<(), ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Stop).await? {
                ModuleAnswer::Stopped => Ok(()),
                other => Err(wrong(&other)),
            }
        }
    }

    /// The module's current status.
    fn status(&self) -> impl Future<Output = Result<ModuleStatus, ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Status).await? {
                ModuleAnswer::Status(status) => Ok(status),
                other => Err(wrong(&other)),
            }
        }
    }

    /// Runs a console command the module registered.
    fn command(&self, request: CommandRequest) -> impl Future<Output = Result<CommandOutput, ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Command(request)).await? {
                ModuleAnswer::Command(output) => Ok(output),
                other => Err(wrong(&other)),
            }
        }
    }

    /// Runs an admin action the module registered.
    fn admin(&self, call: AdminCall) -> impl Future<Output = Result<AdminResponse, ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Admin(call)).await? {
                ModuleAnswer::Admin(response) => Ok(response),
                other => Err(wrong(&other)),
            }
        }
    }

    /// Delivers an event the module subscribed to.
    fn event(&self, event: Event) -> impl Future<Output = Result<(), ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Event(event)).await? {
                ModuleAnswer::EventHandled => Ok(()),
                other => Err(wrong(&other)),
            }
        }
    }

    /// Asks the module's chat hooks about a message or channel.
    fn chat(&self, query: ChatQuery) -> impl Future<Output = Result<ChatAnswer, ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Chat(query)).await? {
                ModuleAnswer::Chat(answer) => Ok(answer),
                other => Err(wrong(&other)),
            }
        }
    }

    /// Asks the module about its game (MS1).
    fn game(&self, call: GameCall) -> impl Future<Output = Result<GameAnswer, ModuleError>> + Send {
        async move {
            match self.call(ModuleCall::Game(call)).await? {
                ModuleAnswer::Game(answer) => Ok(answer),
                other => Err(wrong(&other)),
            }
        }
    }
}

impl<D: ModuleDriver + ?Sized> ModuleDriverExt for D {}

/// Runs one call against a module: the same dispatch serves a compiled-in module and the child adapter,
/// which is what makes their answers the same.
pub async fn dispatch(
    module: &dyn GameModule,
    ctx: ModuleContext,
    call: ModuleCall,
) -> Result<ModuleAnswer, ModuleError> {
    Ok(match call {
        ModuleCall::Register => {
            let mut registrar = Registrar::new(module.manifest());
            module.register(&mut registrar)?;
            let registration = registrar
                .finish()
                .map_err(|error| ModuleError::new(ErrorKind::Invalid, error.to_string()))?;
            ModuleAnswer::Registered(registration)
        }
        ModuleCall::Start(config) => {
            module.start(ctx, config).await?;
            ModuleAnswer::Started
        }
        ModuleCall::Stop => {
            module.stop(ctx).await?;
            ModuleAnswer::Stopped
        }
        ModuleCall::Status => ModuleAnswer::Status(module.status()),
        ModuleCall::Command(request) => ModuleAnswer::Command(module.command(ctx, request).await?),
        ModuleCall::Admin(call) => ModuleAnswer::Admin(module.admin(ctx, call).await?),
        ModuleCall::Event(event) => {
            module.event(ctx, event).await?;
            ModuleAnswer::EventHandled
        }
        ModuleCall::Chat(query) => ModuleAnswer::Chat(module.chat(ctx, query).await?),
        ModuleCall::Game(call) => ModuleAnswer::Game(module.game(ctx, call).await?),
    })
}

/// A module compiled into this binary.
pub struct InProcessModule {
    module: Arc<dyn GameModule>,
    manifest: ModuleManifest,
    ctx: ModuleContext,
}

impl InProcessModule {
    /// Loads `module`, refusing a manifest that doesn't validate, as the handshake would.
    pub fn new(module: Arc<dyn GameModule>, node: NodeIdentity, link: Arc<dyn HostLink>) -> Result<Self, ModuleError> {
        let manifest = module.manifest();
        manifest
            .validate()
            .map_err(|error| ModuleError::new(ErrorKind::Invalid, error.to_string()))?;
        let ctx = ModuleContext::new(manifest.id.clone(), node, link);
        Ok(Self { module, manifest, ctx })
    }
}

impl ModuleDriver for InProcessModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn call(&self, call: ModuleCall) -> BoxFuture<'_, Result<ModuleAnswer, ModuleError>> {
        Box::pin(dispatch(self.module.as_ref(), self.ctx.clone(), call))
    }

    fn unload(&self) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move { self.module.stop(self.ctx.clone()).await })
    }
}
