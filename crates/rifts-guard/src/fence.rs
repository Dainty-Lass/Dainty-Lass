// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The fence an authorized inventory transaction carries: each grant is checked again against the hunt as it is
//! then, and the draft token's cap against the balance inside the transaction.

use std::sync::{Arc, Mutex, PoisonError};

use rifts_seam::{InventoryFence, InventoryTransaction, Json, Reply, RuntimeAuth, TransactionPhase, TransactionReads};
use serde_json::Value;

use crate::grants::{DraftPremiumDecision, HOST_DRAFT_PREMIUM_ID, decide_draft_premium, stacked_balance_of};
use crate::ports::{Clock, GuardLog, HuntSessions, iso_time};
use crate::writes::{inventory_body_denial, loot_refusal};

const DRAFT_PREFIX: &str = "P2P_LOOT_GRANT_DRAFT_PREMIUM_";

pub(crate) struct GuardFence {
    pub hunts: Arc<dyn HuntSessions>,
    pub log: Arc<dyn GuardLog>,
    pub clock: Arc<dyn Clock>,
    pub enforce: bool,
    pub runtime: RuntimeAuth,
    pub transaction: InventoryTransaction,
    pub body: Option<Value>,
    /// The body adds the draft token and is not an accepted replay.
    pub carries_draft: bool,
    pub early: Option<DraftPremiumDecision>,
    pub tags: String,
    pub allowed: Mutex<Option<(DraftPremiumDecision, String)>>,
}

impl GuardFence {
    fn refuse(&self, code: &str) -> Reply {
        self.log
            .warn(&format!("[P2PInventory] transaction denied code={code} {}", self.tags));
        if let Some(reason) = code.strip_prefix(DRAFT_PREFIX) {
            self.log.warn(&format!(
                "[HostGrantDeny] account={} session={} generation={} transaction={} item={HOST_DRAFT_PREMIUM_ID} quantity=1 reason={reason} at=transaction",
                self.transaction.user, self.runtime.session, self.runtime.generation, self.transaction.transaction_id
            ));
        }
        loot_refusal(code)
    }

    fn check(&self, grant: &Json, decide: impl FnOnce(&Value) -> Option<DraftPremiumDecision>) -> Result<(), Reply> {
        let Ok(hunt) = self.hunts.hunt(&self.runtime.session) else {
            return Err(self.refuse("P2P_LOOT_GRANT_DENIED"));
        };
        let presence = match &hunt {
            Some(h) if h.is_hub() => match self.hunts.hub_present_accounts(&h.session, h.generation) {
                Ok(accounts) => Some(accounts),
                Err(_) => return Err(self.refuse("P2P_LOOT_GRANT_DENIED")),
            },
            _ => None,
        };
        let parsed;
        let body = match &self.body {
            Some(body) => Some(body),
            None => {
                parsed = serde_json::from_str::<Value>(&grant.0).ok();
                parsed.as_ref()
            }
        };
        if let Some(reason) = inventory_body_denial(
            &self.runtime,
            hunt.as_ref(),
            &self.transaction.user,
            &self.transaction.character_id,
            body,
            presence.as_deref(),
        ) {
            return Err(self.refuse(&format!("P2P_LOOT_GRANT_{reason}")));
        }
        if !self.carries_draft {
            return Ok(());
        }
        let decision = body.and_then(decide);
        match decision {
            Some(decision) if decision.allowed => {
                let day = iso_time(self.clock.now())[..10].to_owned();
                *self.allowed.lock().unwrap_or_else(PoisonError::into_inner) = Some((decision, day));
                Ok(())
            }
            _ => {
                let reason = decision.map_or("NOT_A_GRANT", |d| d.reason);
                if self.enforce {
                    return Err(self.refuse(&format!("{DRAFT_PREFIX}{reason}")));
                }
                self.log.warn(&format!(
                    "[HostPolicyObserve] would refuse draft premium {} item={HOST_DRAFT_PREMIUM_ID} reason={reason} at=transaction{}",
                    self.tags,
                    decision.map(|d| d.balance_tags()).unwrap_or_default()
                ));
                Ok(())
            }
        }
    }
}

impl InventoryFence for GuardFence {
    fn on_grant(&self, grant: &Json) -> Result<(), Reply> {
        let early = self.early;
        self.check(grant, |_| early)
    }

    fn on_grant_in(&self, grant: &Json, reads: &dyn TransactionReads) -> Result<(), Reply> {
        self.check(grant, |body| {
            let balance = match reads.stacked_items(&self.transaction.character_id) {
                Ok(stored) => stacked_balance_of(stored.as_ref().map(|j| j.0.as_str()), HOST_DRAFT_PREMIUM_ID),
                Err(_) => None,
            };
            decide_draft_premium(body, balance)
        })
    }

    fn report(&self, phase: TransactionPhase) {
        match phase {
            TransactionPhase::Applying => self.log.info(&format!("[P2PInventory] applying {}", self.tags)),
            TransactionPhase::Accepted => {
                self.log
                    .info(&format!("[P2PInventory] transaction accepted {}", self.tags));
                let remembered = self.allowed.lock().unwrap_or_else(PoisonError::into_inner).take();
                if let Some((decision, day)) = remembered {
                    self.log.info(&format!(
                        "[HostGrantAllow] account={} session={} generation={} transaction={} item={HOST_DRAFT_PREMIUM_ID} quantity=1 reason={} day={day}{}",
                        self.transaction.user,
                        self.runtime.session,
                        self.runtime.generation,
                        self.transaction.transaction_id,
                        decision.reason,
                        decision.balance_tags()
                    ));
                }
            }
            TransactionPhase::Failed => self
                .log
                .error(&format!("[P2PInventory] transaction failed {}", self.tags)),
        }
    }
}
