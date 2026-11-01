//! The Rust backend under Rustic Isles' game module (DaintyLass plan section 2, DD6, DD8): written from Mystic
//! Paradox's 1.14.7 Node services, with what a module supplies as hooks and tables (RU3c).
//!
//! - [`routes`]: every route of RU3a's inventory in Express's match order.
//! - [`http`]: the pipeline that serves them, 501 in the launcher envelope until an area ports a route.
//! - [`areas`]: one handler file per RU5 area group.
//! - [`hooks`], [`tables`], [`settings`], [`game_data`]: what differs between the versions.
//! - [`repo`], [`db`]: one trait per Node repository on `store`'s two engines and its change log, with account
//!   writes through the home API (DN2) and global account ids (DD18, [`ids`]).
//! - [`module`]: the `GameModule` a version crate fills in.

pub mod areas;
pub mod bootstrap;
pub mod db;
pub mod game_data;
pub mod hooks;
pub mod http;
pub mod ids;
pub mod module;
pub mod replay;
pub mod repo;
pub mod routes;
pub mod settings;
pub mod tables;
#[doc(hidden)]
pub mod testing;
pub mod version;

pub use game_data::generators as gamedata;
pub use hooks::VersionHooks;
pub use ids::{AccountId, UserId};
pub use module::{IslesModule, ModuleSpec, PortService};
pub use settings::Settings;
pub use tables::VersionTables;
pub use version::IslesVersion;
