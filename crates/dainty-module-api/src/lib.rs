//! Dainty Lass's module framework (DF2): the one seam every other repository builds on.
//!
//! A game module (DD3, DD4) is a [`GameModule`]: a [`ModuleManifest`] keyed by an exact [`GameVersion`],
//! a registration of what it needs from the core plugins ([`Registrar`]), and the calls the host makes.
//! The same trait runs compiled in ([`InProcessModule`]) or as its own process (DD5): [`child::main`] wraps
//! it as an executable and [`export_plugin!`] as a library the core loads ([`plugin`], RD27), and
//! [`ChildModule`] drives either over the frame protocol in [`frame`]. The host holds any of them as a
//! [`ModuleDriver`]. Compiled-in plugins (gateway, content, admin API, chat,
//! network) implement [`CorePlugin`].
//!
//! What a game module knows about its game ([`game`], MS1) is asked like admin and chat.
//!
//! The network ([`network`], DN2), chat ([`chat`], DN10), actor ([`actor`], DH1) and patch-set and theme
//! capabilities (DD25) are seams: types and stubs that later rows fill.

pub mod ids;

pub mod actor;
pub mod chat;
pub mod child;
pub mod driver;
pub mod frame;
pub mod game;
pub mod host;
pub mod manifest;
pub mod module;
pub mod network;
mod peer;
pub mod plugin;
pub mod registrar;
pub mod stub;
mod tree;
pub mod version;

pub use actor::{Actor, ActorContext};
pub use driver::{InProcessModule, ModuleDriver, ModuleDriverExt};
pub use game::{Availability, GameAnswer, GameCall, GameCallKind, GameServer, GameServerKind, UnavailableReason};
pub use host::{ChildModule, ChildOptions, ExternalModule, LoadError};
pub use ids::{ModuleId, Sha1Hex, Sha256Hex};
pub use manifest::{
    Capabilities, ClientProfile, EpicBuild, IniEdit, IniRoot, ModuleKind, ModuleManifest, PinnedFile, PortRange,
};
pub use module::{
    AdminCall, AdminRequest, AdminResponse, BoxFuture, CommandOutput, CommandRequest, CoreCapability, CorePlugin,
    ErrorKind, Event, EventName, GameModule, HostAnswer, HostCall, HostLink, LogLevel, LogRecord, ModuleConfig,
    ModuleContext, ModuleError, ModuleStatus, Phase, PlayerInfo, PlayerRestriction, RestrictionKind,
};
pub use network::{
    GlobalId, HomeApi, HomeBackend, HomeForward, HomeInvocation, IdKind, NodeId, NodeIdentity, OperationName, Region,
};
pub use registrar::{AdminAction, Registrar, Registration};
pub use version::{API_VERSION, ApiVersion, GameVersion};
