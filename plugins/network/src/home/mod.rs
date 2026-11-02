//! The home API and session tickets (DD17, DN5): the account operations other nodes need, over node links.
//!
//! An account's data lives only at its home. Another node that needs it (a hunt it hosts) gets, from the
//! home, a short-lived ticket the home signed ([`ticket`], [`keys`]); with it the node runs the game module's
//! operations on the account at the home ([`protocol`], [`service`]); and [`client::NetworkedHome`] is the
//! `HomeApi` modules reach all of it through. The plugin never reads an operation: a module's name, an
//! operation name and a JSON body travel to the module's [`module_api::HomeBackend`] at the home.
//!
//! The account key (`RRK_...`) never crosses a node link: [`protocol`] refuses it in every field.

pub mod client;
pub mod keys;
pub mod protocol;
pub mod service;
pub mod ticket;

pub use client::{NetworkedConfig, NetworkedHome};
pub use keys::{KeyError, SigningKey, VerifyingKey};
pub use protocol::{HOME_VERSION, HomeBody, HomeCall, HomeRefusal, HomeReply, HomeRequest, HomeRequestCall};
pub use service::{DriverBackend, HomeConfig, HomeLocator, HomeRouter, HomeService};
pub use ticket::{Claims, Expect, RevocationPersistError, TicketConfig, TicketError, Tickets};
