// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The inventory and weapon-reforge area (inventory spec): the eight `inventory-reforge` routes, the inventory
//! transaction and its ledger, the item rules, the storage policy, the currency half of a transaction and the
//! reforge's reset and banked restore.

mod currency;
pub mod items;
mod ports;
pub mod prestige;
pub mod reforge;
mod routes;
pub mod storage_policy;
mod transaction;
pub mod validation;
pub mod weapon_level;

pub use ports::{
    AuthRequest, Authentication, Authenticator, BountiesWallet, Caller, CharacterOwnership, Characters, CharactersList,
    CurrencyChange, ExtensionAuthenticator, HostRuntime, InventoryActivity, InventoryActivitySlot, InventoryPorts,
    LedgerObserver, NoRing, Observer, RingInstances, RingInstancesSlot, StarterBootstrap, StarterRecordsCheck, Wallet,
    WalletError,
};
pub use routes::{register, routes};
pub use transaction::{
    Annotator, CommittedTransaction, Economy, GrantHook, ItemUpdate, TxContext, TxError, TxOptions, TxOutcome,
    TxRequest, get_inventory, inventory_record, player_actor, run_inventory_transaction, system_actor,
    update_instanced_item,
};

pub(crate) use ports::ring_instance;
pub(crate) use routes::module_id;
