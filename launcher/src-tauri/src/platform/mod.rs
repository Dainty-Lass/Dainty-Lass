//! The platform layer (rows L26 to L37): folders, settings, the key store, the log, the game folder,
//! launching, links, dialogs, the clipboard and the art protocol, and the Host page's host kit, server,
//! requirements and the runtime loader's list (RD9).

pub mod art;
pub mod clipboard;
pub mod dialogs;
pub mod epic_library;
pub mod epic_login;
pub mod epic_session;
pub mod game_config;
pub mod game_console;
pub mod game_folder;
#[cfg(feature = "host")]
pub mod hostkit;
#[cfg(feature = "host")]
pub mod hostserver;
pub mod invite_link;
pub mod keystore;
pub mod launch;
pub mod links;
pub mod linux_runtime;
pub mod loader_session;
pub mod log;
pub mod paths;
#[cfg(feature = "host")]
pub mod requirements;
pub mod settings;
pub mod system;
pub mod updater;

#[cfg(test)]
mod game_config_tests;
