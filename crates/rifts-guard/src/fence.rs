// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The fence an authorized inventory transaction carries: each grant is checked again against the hunt as it is
//! then, and the draft token's cap against the balance inside the transaction.

use std::sync::{Arc, Mutex, PoisonError};

use rifts_seam::{InventoryFence, InventoryTransaction, Json, Reply, RuntimeAuth, TransactionPhase, TransactionReads};
use serde_json::Value;

use crate::{
    grants::{DraftPremiumDecision, HOST_DRAFT_PREMIUM_ID, decide_draft_premium, stacked_balance_of},
    ports::{Clock, GuardLog, HuntSessions, iso_time},
    writes::{inventory_body_denial, loot_refusal},
};

const DRAFT_PREFIX: &str = "P2P_LOOT_GRANT_DRAFT_PREMIUM_";
const DENIED: &str = "P2P_LOOT_GRANT_DENIED";

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
    /// `account=.. session=.. generation=.. transaction=..`, for the draft token's own lines.
    fn draft_tags(&self) -> String {
        let (user, id) = (&self.transaction.user, &self.transaction.transaction_id);
        let (session, generation) = (&self.runtime.session, self.runtime.generation);
        format!("account={user} session={session} generation={generation} transaction={id}")
    }

    fn refuse(&self, code: &str) -> Reply {
        self.log
            .warn(&format!("[P2PInventory] transaction denied code={code} {}", self.tags));
        if let Some(reason) = code.strip_prefix(DRAFT_PREFIX) {
            let tags = self.draft_tags();
            self.log.warn(&format!(
                "[HostGrantDeny] {tags} item={HOST_DRAFT_PREMIUM_ID} quantity=1 reason={reason} at=transaction"
            ));
        }
        loot_refusal(code)
    }

    /// The hub's present accounts, read now; `Err` when the read fails.
    fn presence_now(&self, hunt: Option<&crate::ports::HuntView>) -> Result<Option<Vec<rifts_seam::AccountId>>, ()> {
        match hunt {
            Some(hub) if hub.is_hub() => self
                .hunts
                .hub_present_accounts(&hub.session, hub.generation)
                .map(Some)
                .map_err(drop),
            _ => Ok(None),
        }
    }

    fn remember(&self, decision: DraftPremiumDecision) {
        let day = iso_time(self.clock.now())[..10].to_owned();
        let mut allowed = self.allowed.lock().unwrap_or_else(PoisonError::into_inner);
        *allowed = Some((decision, day));
    }

    fn check(&self, grant: &Json, decide: impl FnOnce(&Value) -> Option<DraftPremiumDecision>) -> Result<(), Reply> {
        let hunt = self
            .hunts
            .hunt(&self.runtime.session)
            .map_err(|_| self.refuse(DENIED))?;
        let presence = self.presence_now(hunt.as_ref()).map_err(|()| self.refuse(DENIED))?;
        // The authorized body, or else this grant's own.
        let own = self
            .body
            .is_none()
            .then(|| serde_json::from_str::<Value>(&grant.0).ok())
            .flatten();
        let body = self.body.as_ref().or(own.as_ref());
        let denial = inventory_body_denial(
            &self.runtime,
            hunt.as_ref(),
            &self.transaction.user,
            &self.transaction.character_id,
            body,
            presence.as_deref(),
        );
        if let Some(reason) = denial {
            return Err(self.refuse(&format!("P2P_LOOT_GRANT_{reason}")));
        }
        if !self.carries_draft {
            return Ok(());
        }
        let decision = body.and_then(decide);
        if let Some(allowed) = decision.filter(|d| d.allowed) {
            self.remember(allowed);
            return Ok(());
        }
        let reason = decision.map_or("NOT_A_GRANT", |d| d.reason);
        if self.enforce {
            return Err(self.refuse(&[DRAFT_PREFIX, reason].concat()));
        }
        let balance = decision.map(|d| d.balance_tags()).unwrap_or_default();
        self.log.warn(&format!(
            "[HostPolicyObserve] would refuse draft premium {} item={HOST_DRAFT_PREMIUM_ID} reason={reason} \
             at=transaction{balance}",
            self.tags
        ));
        Ok(())
    }
}

impl InventoryFence for GuardFence {
    fn on_grant(&self, grant: &Json) -> Result<(), Reply> {
        let early = self.early;
        self.check(grant, move |_| early)
    }

    fn on_grant_in(&self, grant: &Json, reads: &dyn TransactionReads) -> Result<(), Reply> {
        let balance = || {
            let stored = reads.stacked_items(&self.transaction.character_id).ok()?;
            stacked_balance_of(stored.as_ref().map(|j| j.0.as_str()), HOST_DRAFT_PREMIUM_ID)
        };
        self.check(grant, |body| decide_draft_premium(body, balance()))
    }

    fn report(&self, phase: TransactionPhase) {
        let tags = &self.tags;
        match phase {
            TransactionPhase::Applying => self.log.info(&format!("[P2PInventory] applying {tags}")),
            TransactionPhase::Failed => self.log.error(&format!("[P2PInventory] transaction failed {tags}")),
            TransactionPhase::Accepted => {
                self.log.info(&format!("[P2PInventory] transaction accepted {tags}"));
                let remembered = self.allowed.lock().unwrap_or_else(PoisonError::into_inner).take();
                let Some((decision, day)) = remembered else {
                    return;
                };
                let (draft, reason, balance) = (self.draft_tags(), decision.reason, decision.balance_tags());
                self.log.info(&format!(
                    "[HostGrantAllow] {draft} item={HOST_DRAFT_PREMIUM_ID} quantity=1 reason={reason} day={day}{balance}"
                ));
            }
        }
    }
}
