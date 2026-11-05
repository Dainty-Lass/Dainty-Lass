//! What a host's runtime posts to `POST /inventory` (RF6): the body through the host grant policy and the draft
//! premium decision, and the same bytes as a character's stored `stackedItems`. It checks that a denial names what it
//! found, an exempt token is the one whole add, and an allowed token keeps the balance at the cap.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rifts_guard::grants::{
    Allowances, GrantSession, HOST_DRAFT_PREMIUM_BALANCE_CAP, HOST_DRAFT_PREMIUM_ID, NoGameData, classify,
    decide_draft_premium, grant_denial, stacked_balance_of, violations,
};
use rifts_guard::json::parse_p2p_body;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = classify(text, &NoGameData);
        if let Some(balance) = stacked_balance_of(Some(text), HOST_DRAFT_PREMIUM_ID) {
            assert!(balance >= 0);
        }
    }
    let headers = [("content-type".to_owned(), "application/json".to_owned())];
    let Ok(Some(body)) = parse_p2p_body(&headers, data) else {
        return;
    };
    let strict = violations(&body, &NoGameData, Allowances::default());
    let allowed = violations(&body, &NoGameData, Allowances { draft_premium: true });
    // The allowance only ever drops the one draft token
    assert!(allowed.len() <= strict.len());
    assert!(strict.len() - allowed.len() <= 1);
    assert!(
        strict
            .iter()
            .filter(|entry| !allowed.contains(entry))
            .all(|entry| entry.catalog_id == HOST_DRAFT_PREMIUM_ID)
    );
    for session in [
        GrantSession::default(),
        GrantSession {
            no_completion: true,
            realm_hub: false,
        },
        GrantSession {
            no_completion: true,
            realm_hub: true,
        },
    ] {
        match grant_denial(session, &body, &NoGameData, Allowances::default()) {
            Some(denial) if denial.classes == ["NO_REWARD_SESSION"] => {
                assert!(session.no_completion && !session.realm_hub);
            }
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
