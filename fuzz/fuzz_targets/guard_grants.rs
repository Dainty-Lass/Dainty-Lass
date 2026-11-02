// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What a player host's inventory transaction may grant: item classes, the stored balance and the daily draft
//! token's cap (RX3, docs/security/rifts-guard.md).
#![no_main]

use libfuzzer_sys::fuzz_target;
use rifts_guard::grants::{
    Allowances, GrantSession, HOST_DRAFT_PREMIUM_BALANCE_CAP, HOST_DRAFT_PREMIUM_ID, NoGameData, classify,
    decide_draft_premium, grant_denial, stacked_balance_of, violations,
};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = classify(text, &NoGameData);
    if let Some(balance) = stacked_balance_of(Some(text), HOST_DRAFT_PREMIUM_ID) {
        assert!(balance >= 0);
    }
    let Ok(body) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };

    let strict = violations(&body, &NoGameData, Allowances::default());
    let allowed = violations(&body, &NoGameData, Allowances { draft_premium: true });
    // the allowance lifts at most the one draft token, and nothing else
    assert!(strict.len() - allowed.len() <= 1);
    assert!(allowed.iter().all(|v| strict.contains(v)));
    for lifted in strict.iter().filter(|v| !allowed.contains(v)) {
        assert_eq!(lifted.catalog_id, HOST_DRAFT_PREMIUM_ID);
    }

    let shapes = [(false, false), (true, false), (true, true), (false, true)];
    for (no_completion, realm_hub) in shapes {
        let session = GrantSession {
            no_completion,
            realm_hub,
        };
        let denial = grant_denial(session, &body, &NoGameData, Allowances::default());
        let no_reward = denial.as_ref().is_some_and(|d| d.classes == ["NO_REWARD_SESSION"]);
        if no_completion && !realm_hub && no_reward {
            continue;
        }
        assert!(!no_reward);
        match denial {
            Some(denial) => {
                assert_eq!(denial.violations, strict);
                assert!(!denial.classes.is_empty());
            }
            None => assert!(strict.is_empty()),
        }
    }

    for balance in [None, Some(0), Some(1), Some(i128::from(u32::MAX))] {
        if let Some(decision) = decide_draft_premium(&body, balance) {
            if decision.allowed {
                let (_, after) = decision.balance.unwrap_or((0, i128::MAX));
                assert!(after <= HOST_DRAFT_PREMIUM_BALANCE_CAP);
            }
        }
    }
});
