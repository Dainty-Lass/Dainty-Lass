// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Derived ids (DD18; spec §6). The `isles-db:` prefixes are values, not names: every id already derived in a
//! migrated database depends on them, so they stay byte for byte.

use std::collections::HashMap;

use dainty_module_api::GlobalId;
use rustic_isles::AccountId;
use sha2::{Digest as _, Sha256};

use crate::DbError;

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const INSTANCE: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// The ULID of SHA-256(`isles-db:<namespace>:<node>:<key>`)'s first 16 bytes.
pub fn deterministic_ulid(namespace: &str, node: &str, key: &str) -> String {
    let digest = Sha256::digest(format!("isles-db:{namespace}:{node}:{key}").as_bytes());
    let mut top = [0u8; 16];
    top.copy_from_slice(&digest[..16]);
    let n = u128::from_be_bytes(top);
    let mut out = String::with_capacity(26);
    out.push(char::from(CROCKFORD[(n >> 125) as usize & 7]));
    for i in 0..25 {
        out.push(char::from(CROCKFORD[(n >> (120 - 5 * i)) as usize & 31]));
    }
    out
}

/// The store's account id for a MongoDB userId on `node`: the same input always gives the same id (DD18).
pub fn derived_account_id(node: &str, user_id: &str) -> String {
    format!("account:{}@{node}", deterministic_ulid("account", node, user_id))
}

/// The store's party id for a MongoDB party id on `node`, derived like [`derived_account_id`].
pub fn derived_party_id(node: &str, party_id: &str) -> String {
    format!("party:{}@{node}", deterministic_ulid("party", node, party_id))
}

/// The id the RB11 starter repair gives an item (Node's `migrate_starter_instance_refs`).
pub fn repair_instance_id(user: &str, character: &str, catalog: &str, ordinal: usize) -> String {
    let digest = Sha256::digest(format!("isles-db:instance:{user}:{character}:{catalog}:{ordinal}").as_bytes());
    (0..26)
        .map(|i| char::from(INSTANCE[(usize::from(digest[i]) + 7 * i) % 36]))
        .collect()
}

/// Exactly 26 of `A`-`Z`, `0`-`9`.
pub fn is_instance_id(text: &str) -> bool {
    text.len() == 26 && text.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// A target account row read before extraction: its id and what an implicit row must not overwrite.
#[derive(Debug, Clone)]
pub(crate) struct Existing {
    pub id: String,
    pub name: String,
    pub notes: i64,
}

/// Node ids to global ids: the target's mappings first, then derived ones, remembered for the run.
pub(crate) struct Resolver {
    node: String,
    accounts: HashMap<String, String>,
    pub(crate) existing: HashMap<String, Existing>,
    parties: HashMap<String, String>,
}

impl Resolver {
    pub fn new(node: &str, accounts: Vec<(String, Existing)>, parties: Vec<(String, String)>) -> Self {
        let mut map = HashMap::new();
        let mut existing = HashMap::new();
        for (user, row) in accounts {
            if row.id.parse::<AccountId>().is_ok() {
                map.insert(user, row.id.clone());
                existing.insert(row.id.clone(), row);
            }
        }
        Self {
            node: node.to_owned(),
            accounts: map,
            existing,
            parties: parties
                .into_iter()
                .filter(|(_, id)| id.parse::<GlobalId>().is_ok())
                .collect(),
        }
    }

    /// The account of a user id (trimmed); an empty one aborts the run, `--skip-invalid` or not (P2).
    pub fn account(&mut self, user: &str) -> Result<String, DbError> {
        let user = user.trim();
        if user.is_empty() {
            return Err(DbError::Config("empty userId".into()));
        }
        if let Some(id) = self.accounts.get(user) {
            return Ok(id.clone());
        }
        let id = derived_account_id(&self.node, user);
        id.parse::<AccountId>()
            .map_err(|e| DbError::Config(format!("invalid derived account id {id:?}: {e}")))?;
        self.accounts.insert(user.to_owned(), id.clone());
        Ok(id)
    }

    pub fn party(&mut self, party: &str) -> Result<String, DbError> {
        let party = party.trim();
        if party.is_empty() {
            return Err(DbError::Config("empty partyId".into()));
        }
        if let Some(id) = self.parties.get(party) {
            return Ok(id.clone());
        }
        let id = derived_party_id(&self.node, party);
        id.parse::<GlobalId>()
            .map_err(|e| DbError::Config(format!("invalid derived party id {id:?}: {e}")))?;
        self.parties.insert(party.to_owned(), id.clone());
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Computed here from the formulas of spec §6.1 and compared with its table
    #[test]
    fn spec_vectors() {
        assert_eq!(
            derived_account_id("local", "u1"),
            "account:402D98CJC48J8KBFQ1526NZ6ZS@local"
        );
        assert_eq!(
            derived_account_id("eu-west", "u1"),
            "account:4QDCTFADD9TFQBS5FDCZT7TWSB@eu-west"
        );
        assert_eq!(
            derived_party_id("local", "p1"),
            "party:01Y36W9SXJ680YT5TTN54MAA3R@local"
        );
        assert_eq!(repair_instance_id("u1", "c1", "CAT_A", 0), "2UDZFBXSIH29XDDAMOVB4TBBMQ");
        assert_eq!(repair_instance_id("u1", "c1", "CAT_A", 1), "QRBAAJNT7N1UZ40DEGJA919MOQ");
    }

    #[test]
    fn existing_mappings_win_and_empty_ids_abort() {
        let row = Existing {
            id: "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@local".into(),
            name: "n".into(),
            notes: 0,
        };
        let mut r = Resolver::new("local", vec![("u1".into(), row)], Vec::new());
        assert_eq!(r.account(" u1 ").unwrap(), "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@local");
        assert_eq!(r.account("u2").unwrap(), derived_account_id("local", "u2"));
        assert_eq!(
            r.account("  ").unwrap_err().to_string(),
            "invalid argument: empty userId"
        );
        assert!(is_instance_id(&repair_instance_id("a", "b", "c", 9)));
        assert!(!is_instance_id("bad-id"));
    }
}
