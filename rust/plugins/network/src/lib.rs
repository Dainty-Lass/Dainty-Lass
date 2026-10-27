//! The network core plugin (DD16-DD18): regional nodes of one operator's cluster, each an ordinary
//! server.
//!
//! - [`cluster`]: the cluster CA and the node certificates it issues, and the mutual-TLS settings every
//!   node link uses (DN3).
//! - [`link`]: [`PeerLink`] and [`NodeService`], the two sides of a call between nodes. [`tls`] carries
//!   them over mutual TLS to a peer's gateway ([`TlsPeerLink`], with peer health and clock checks);
//!   [`endpoint`] is the answering side the gateway's `/__node` routes call; [`wire`] is what both put on
//!   the wire. [`inprocess`] carries them inside one process for tests, with partitions; [`NoPeers`] is
//!   the link of a server outside a network.
//! - [`directory`] (feature `directory`): account to home and username claims, one leader writing an
//!   append-only log that every other node pulls ([`Directory`]), on `store`.
//! - [`messages`]: what the directory's nodes send each other.
//! - [`home`]: the home API (DD17): session tickets an account's home signs, and a game module's operations on an
//!   account run at its home for another node ([`home::NetworkedHome`], [`home::HomeService`]).
//!
//! A server that isn't in a network runs the same directory with no peers (section 6): it is its own
//! leader, and nothing leaves it.

pub mod cluster;
#[cfg(feature = "directory")]
pub mod directory;
pub mod endpoint;
pub mod home;
pub mod inprocess;
pub mod link;
#[cfg(feature = "directory")]
pub mod messages;
pub mod relay;
pub mod tls;
#[cfg(feature = "directory")]
pub mod username;
pub mod wire;

#[cfg(feature = "directory")]
pub use directory::{AccountRecord, Directory, DirectoryConfig, DirectoryError, Role};
pub use endpoint::NodeEndpoint;
pub use link::{Clock, LinkError, NoPeers, NodeService, PeerLink, PeerRoster, Service, ServiceMux};
#[cfg(feature = "directory")]
pub use messages::{DirectoryChange, DirectoryEntry, Refusal};
pub use tls::{PeerHealth, PeerState, TlsLinkConfig, TlsPeerLink};
#[cfg(feature = "directory")]
pub use username::Username;
