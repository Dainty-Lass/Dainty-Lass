//! Supplies refilled at every game login (`sessionSupplies.ts`, Reforged): the host names, per account, stacked items
//! that go back to a fixed quantity each time that account starts a game session. `POST /login` is auth's route and
//! calls [`refill_session_supplies`]; the rule is here.

use std::collections::HashMap;

use module_api::{Actor, ActorContext};
use serde_json::{Map, Value, json};

use crate::AccountId;
use crate::http::Shared;
use crate::repo::RepoError;

use super::ports::StarterBootstrap;

pub const MAX_SESSION_SUPPLY_QUANTITY: i64 = 1_000_000;

/// Per user id, the catalog ids and the quantities they are set to (file order).
pub type SessionSupplies = HashMap<String, Vec<(String, i64)>>;

fn catalog_id_ok(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `ParseSessionSupplies`: fails with a readable reason on anything malformed, so a typo is logged, not half-applied.
pub fn parse_session_supplies(text: &str, is_instanced: &dyn Fn(&str) -> bool) -> Result<SessionSupplies, String> {
    let raw: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let Some(accounts) = raw.get("accounts").and_then(Value::as_object) else {
        return Err(r#"expected {"accounts": {"<user id>": {"<catalog id>": <quantity>}}}"#.into());
    };
    let mut result = SessionSupplies::new();
    for (user_id, items) in accounts {
        let Some(items) = items.as_object().filter(|_| !user_id.is_empty()) else {
            return Err(format!(
                "account {}: expected an object of catalog ids and quantities",
                Value::String(user_id.clone())
            ));
        };
        let mut targets = Vec::new();
        for (catalog_id, quantity) in items {
            if !catalog_id_ok(catalog_id) {
                return Err(format!(
                    "account {user_id}: invalid catalog id {}",
                    Value::String(catalog_id.clone())
                ));
            }
            if catalog_id.starts_with("CURRENCY_") {
                return Err(format!(
                    "account {user_id}: {catalog_id} is a currency (wallet), not a stacked item"
                ));
            }
            if is_instanced(catalog_id) {
                return Err(format!(
                    "account {user_id}: {catalog_id} is an instanced item, not a stacked one"
                ));
            }
            let whole = quantity
                .as_f64()
                .filter(|q| q.fract() == 0.0 && (0.0..=MAX_SESSION_SUPPLY_QUANTITY as f64).contains(q));
            let Some(quantity) = whole else {
                return Err(format!(
                    "account {user_id}: quantity of {catalog_id} must be a whole number from 0 to {MAX_SESSION_SUPPLY_QUANTITY}"
                ));
            };
            targets.push((catalog_id.clone(), quantity as i64));
        }
        result.insert(user_id.clone(), targets);
    }
    Ok(result)
}

/// `ApplySessionSupplies`: sets each target's quantity in a stacked-items list (0 removes the entry). `None` when it
/// already matches.
pub fn apply_session_supplies(stacked: &[Value], targets: &[(String, i64)]) -> Option<Vec<Value>> {
    let wanted: HashMap<&str, i64> = targets.iter().map(|(id, q)| (id.as_str(), *q)).collect();
    let mut changed = false;
    let mut next = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for item in stacked {
        let catalog_id = item.get("catalogId").and_then(Value::as_str);
        if let Some((id, quantity)) = catalog_id.and_then(|id| wanted.get_key_value(id)) {
            if seen.contains(id) {
                changed = true;
                continue;
            }
            seen.push(id);
            if *quantity == 0 {
                changed = true;
                continue;
            }
            if item.get("quantity").and_then(Value::as_f64) != Some(*quantity as f64) {
                changed = true;
            }
            let mut copy: Map<String, Value> = item.as_object().cloned().unwrap_or_default();
            copy.insert("quantity".into(), json!(quantity));
            next.push(Value::Object(copy));
            continue;
        }
        next.push(item.clone());
    }
    for (id, quantity) in targets {
        if !seen.contains(&id.as_str()) && *quantity > 0 {
            next.push(json!({ "catalogId": id, "quantity": quantity }));
            changed = true;
        }
    }
    changed.then_some(next)
}

fn targets_for(
    shared: &Shared,
    user_id: &str,
    is_instanced: &dyn Fn(&str) -> bool,
) -> Result<Option<Vec<(String, i64)>>, String> {
    let Some(file) = shared.settings.get("REFORGED_SESSION_SUPPLIES_FILE") else {
        return Ok(None);
    };
    if !std::path::Path::new(file).exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(file).map_err(|error| error.to_string())?;
    let mut supplies = parse_session_supplies(text.trim_start_matches('\u{feff}'), is_instanced)?;
    Ok(supplies.remove(user_id).filter(|targets| !targets.is_empty()))
}

/// `RefillSessionSupplies`, called at the game's login. Never fails: a failure is logged and the login goes on.
/// `is_instanced` is the storage policy's answer (`GetInventoryStoragePolicy(id)?.storage === "instanced"`).
pub async fn refill_session_supplies(
    shared: &Shared,
    starter: &dyn StarterBootstrap,
    account: &AccountId,
    user_id: &str,
    is_instanced: &dyn Fn(&str) -> bool,
) {
    let targets = match targets_for(shared, user_id, is_instanced) {
        Ok(Some(targets)) => targets,
        Ok(None) => return,
        Err(error) => {
            tracing::error!("[SessionSupplies] could not refill supplies for {user_id}: {error}");
            return;
        }
    };
    let Some(store) = &shared.store else { return };
    let owner = account.clone();
    let characters = match store
        .read(move |conn, repos| repos.characters.find_many_by_account(conn, &owner))
        .await
    {
        Ok(characters) => characters,
        Err(error) => {
            tracing::error!("[SessionSupplies] could not refill supplies for {user_id}: {error}");
            return;
        }
    };
    let actor = ActorContext::new(Actor::System);
    for character in characters {
        starter.ensure(shared, account, &character.character_id).await;
        for _ in 0..5 {
            let id = character.character_id.clone();
            let wanted = targets.clone();
            let outcome = store
                .write(&actor, move |tx, repos| {
                    let Some(inventory) = repos.inventories.find_by_character_id(tx.conn(), &id)? else {
                        return Ok(Attempt::Done);
                    };
                    let stacked: Vec<Value> = serde_json::from_str(&inventory.stacked_items)
                        .map_err(|error| RepoError::Data(format!("stacked items: {error}")))?;
                    let Some(next) = apply_session_supplies(&stacked, &wanted) else {
                        return Ok(Attempt::Done);
                    };
                    let updated = repos.inventories.update_both_if_revision_matches(
                        tx,
                        &id,
                        &inventory.instanced_items,
                        &Value::Array(next).to_string(),
                        inventory.revision.unwrap_or(0),
                    )?;
                    Ok(if updated.is_some() {
                        Attempt::Refilled
                    } else {
                        Attempt::Retry
                    })
                })
                .await;
            match outcome {
                Ok(Attempt::Refilled) => {
                    tracing::info!(
                        "[SessionSupplies] refilled {} item(s) for {user_id} (character {})",
                        targets.len(),
                        character.character_id
                    );
                    break;
                }
                Ok(Attempt::Done) => break,
                Ok(Attempt::Retry) => {}
                Err(error) => {
                    tracing::error!("[SessionSupplies] could not refill supplies for {user_id}: {error}");
                    return;
                }
            }
        }
    }
}

enum Attempt {
    Refilled,
    Done,
    Retry,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stacks(items: &[(&str, i64)]) -> Vec<Value> {
        items
            .iter()
            .map(|(id, q)| json!({ "catalogId": id, "quantity": q }))
            .collect()
    }

    #[test]
    fn a_valid_file_parses() {
        let text = r#"{ "accounts": { "u1": { "QI_GRENADE_ICE": 1000, "QI_STAMINA_POTION_00": 5 } } }"#;
        let parsed = parse_session_supplies(text, &|_| false).unwrap();
        assert_eq!(
            parsed["u1"],
            [
                ("QI_GRENADE_ICE".to_owned(), 1000),
                ("QI_STAMINA_POTION_00".to_owned(), 5)
            ]
        );
    }

    #[test]
    fn a_typo_is_refused_with_a_reason() {
        let bad = |text: &str| parse_session_supplies(text, &|id| id == "PART_X").unwrap_err();
        assert!(bad("not json").len() > 3);
        assert!(bad(r#"{"accounts": []}"#).contains("expected"));
        assert!(bad(r#"{"accounts": {"u": {"CURRENCY_NOTES": 5}}}"#).contains("is a currency"));
        assert!(bad(r#"{"accounts": {"u": {"PART_X": 5}}}"#).contains("is an instanced item"));
        assert!(bad(r#"{"accounts": {"u": {"A": 1.5}}}"#).contains("must be a whole number from 0 to 1000000"));
        assert!(bad(r#"{"accounts": {"u": {"A": 1000001}}}"#).contains("must be a whole number"));
        assert!(bad(r#"{"accounts": {"u": {"A B": 1}}}"#).contains("invalid catalog id"));
        assert!(bad(r#"{"accounts": {"u": 5}}"#).contains("expected an object"));
    }

    #[test]
    fn targets_are_set_added_and_removed_and_a_match_changes_nothing() {
        let targets = [("A".to_owned(), 10), ("B".to_owned(), 0), ("C".to_owned(), 3)];
        let held = stacks(&[("A", 4), ("B", 9), ("Z", 1), ("A", 7)]);
        let next = apply_session_supplies(&held, &targets).unwrap();
        assert_eq!(next, stacks(&[("A", 10), ("Z", 1), ("C", 3)]));
        assert_eq!(apply_session_supplies(&next, &targets), None);
        assert_eq!(apply_session_supplies(&[], &[("Q".to_owned(), 0)]), None);
    }
}
