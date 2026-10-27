//! The network core plugin (DD16-DD18): regional nodes of one operator's cluster, each an ordinary
//! server. This row (DN4) is the directory and what it needs from node links:
//!
//! - [`link`]: [`PeerLink`] and [`NodeService`], the two sides of a call between nodes. DN3 carries them
//!   over mutual TLS on host-only gateway routes; [`inprocess`] carries them inside one process for tests,
//!   with partitions; [`NoPeers`] is the link of a server outside a network.
//! - [`directory`]: account to home and username claims, one leader writing an append-only log that
//!   every other node pulls ([`Directory`]), on `store`.
//! - [`messages`]: what the directory's nodes send each other.
//!
//! A server that isn't in a network runs the same directory with no peers (section 6): it is its own
//! leader, and nothing leaves it.

pub mod directory;
pub mod inprocess;
pub mod link;
pub mod messages;
pub mod username;

pub use directory::{AccountRecord, Directory, DirectoryConfig, DirectoryError, Role};
pub use link::{LinkError, NoPeers, NodeService, PeerLink, Service};
pub use messages::{DirectoryChange, DirectoryEntry, Refusal};
pub use username::Username;
