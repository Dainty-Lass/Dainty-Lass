// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Admission, play eligibility, consent, usernames and the host switches (spec §6.1-§6.5).

use serde_json::Value;

use crate::errors::LauncherError;
use crate::store::{Approval, Consent, LauncherAccount, Status};
use crate::util;

/// The Terms of Use version a consent must name.
pub const TERMS_VERSION: &str = "2026-10-14";
/// The current Privacy Policy version.
pub const PRIVACY_VERSION: &str = "2026-10-17";
/// Privacy versions an earlier consent may still name
pub const ACCEPTED_PRIVACY_VERSIONS: [&str; 3] = ["2026-10-16", "2026-10-15", "2026-10-14.2"];
/// The youngest age a registration may declare, in years.
pub const MINIMUM_AGE: u32 = 16;
/// Days between a deletion request and the erasure.
pub const DELETION_HOLD_DAYS: i64 = 30;

/// `AUTH_ACCOUNT_BANNED`'s text.
pub const MSG_BANNED: &str = "This account has been banned.";
/// `AUTH_ACCOUNT_DISABLED`'s text.
pub const MSG_DISABLED: &str = "This account has been disabled.";
/// `AUTH_APPROVAL_PENDING`'s text.
pub const MSG_PENDING: &str = "Your closed-test access request is waiting for approval.";
/// `AUTH_APPROVAL_REJECTED`'s text.
pub const MSG_REJECTED: &str = "Your closed-test access request was not approved.";
/// `CONSENT_REQUIRED`'s text.
pub const MSG_CONSENT_REQUIRED: &str =
    "Update the launcher to read and accept the current Terms of Use and Privacy Policy, then try again.";
/// `ACCOUNT_DEACTIVATED`'s text.
pub const MSG_DEACTIVATED: &str =
    "This account is scheduled for deletion and is deactivated. Restore it from the Account page to play again.";

/// Banned and disabled only (before codes are mailed).
pub fn assert_not_blocked(account: &LauncherAccount) -> Result<(), LauncherError> {
    match account.status {
        Status::Banned => Err(LauncherError::new("AUTH_ACCOUNT_BANNED", MSG_BANNED)),
        Status::Disabled => Err(LauncherError::new("AUTH_ACCOUNT_DISABLED", MSG_DISABLED)),
        Status::Active => Ok(()),
    }
}

/// Sign-in: not blocked, and the effective approval (`null` reads as approved).
pub fn assert_account_admitted(account: &LauncherAccount) -> Result<(), LauncherError> {
    assert_not_blocked(account)?;
    match account.effective_approval() {
        Approval::Pending => Err(LauncherError::new("AUTH_APPROVAL_PENDING", MSG_PENDING)),
        Approval::Rejected => Err(LauncherError::new("AUTH_APPROVAL_REJECTED", MSG_REJECTED)),
        Approval::Approved => Ok(()),
    }
}

/// Play: admitted, not timed out, a username chosen. A timeout is in force while its `until` sorts after now.
pub fn assert_account_eligible(account: &LauncherAccount, now_iso: &str) -> Result<(), LauncherError> {
    assert_account_admitted(account)?;
    if let Some(timeout) = &account.timeout
        && timeout.until.as_str() > now_iso
    {
        let mut error = LauncherError::new(
            "AUTH_ACCOUNT_TIMED_OUT",
            format!("This account is timed out until {}.", timeout.until),
        )
        .with_detail("timeoutUntil", Value::String(timeout.until.clone()));
        if let Some(reason) = &timeout.reason {
            error = error.with_detail("timeoutReason", Value::String(reason.clone()));
        }
        return Err(error);
    }
    if account.username_set == Some(false) {
        return Err(LauncherError::new(
            "AUTH_USERNAME_REQUIRED",
            "Choose a launcher username before playing.",
        ));
    }
    Ok(())
}

/// Whether the account may play now, as [`assert_account_eligible`] decides.
pub fn is_account_eligible(account: &LauncherAccount, now_iso: &str) -> bool {
    assert_account_eligible(account, now_iso).is_ok()
}

/// Whether a consent's privacy version is current or still accepted.
pub fn is_privacy_accepted(version: &str) -> bool {
    version == PRIVACY_VERSION || ACCEPTED_PRIVACY_VERSIONS.contains(&version)
}

/// Terms current, privacy current or still accepted.
pub fn has_current_consent(account: &LauncherAccount) -> bool {
    account
        .consent
        .as_ref()
        .is_some_and(|c| c.terms_version == TERMS_VERSION && is_privacy_accepted(&c.privacy_version))
}

/// `RUSTIC_ISLES_CONSENT_REQUIRED`: off for `0 false off no`, on for anything else, blank follows the engine.
pub fn consent_required_at_signup(raw: Option<&str>, engine_is_postgres: bool) -> bool {
    match util::normalize_lower(raw.unwrap_or("")).as_str() {
        "" => engine_is_postgres,
        "0" | "false" | "off" | "no" => false,
        _ => true,
    }
}

/// `RUSTIC_ISLES_CONSENT_ENFORCE`: on only for `1 true on yes`.
pub fn consent_enforced(raw: Option<&str>) -> bool {
    is_on(raw)
}

/// A setting reads as on: `1`, `true`, `on` or `yes`, in any case.
pub fn is_on(raw: Option<&str>) -> bool {
    matches!(
        util::normalize_lower(raw.unwrap_or("")).as_str(),
        "1" | "true" | "on" | "yes"
    )
}

/// `ACCOUNT_DEACTIVATED` (with `scheduledFor`) while a deletion is scheduled.
pub fn assert_not_deactivated(account: &LauncherAccount) -> Result<(), LauncherError> {
    match &account.deletion {
        Some(deletion) => Err(LauncherError::new("ACCOUNT_DEACTIVATED", MSG_DEACTIVATED)
            .with_detail("scheduledFor", Value::String(deletion.scheduled_for.clone()))),
        None => Ok(()),
    }
}

/// `CONSENT_REQUIRED` (403) when consent is enforced and the account's is not current.
pub fn assert_consent_if_enforced(account: &LauncherAccount, enforced: bool) -> Result<(), LauncherError> {
    if enforced && !has_current_consent(account) {
        // 403 here; the table's 400 is the registration refusal
        return Err(LauncherError::new("CONSENT_REQUIRED", MSG_CONSENT_REQUIRED).with_status(403));
    }
    Ok(())
}

/// The body's `consent` at registration: `Ok(None)` records none.
pub fn registration_consent(
    value: Option<&Value>,
    required: bool,
    now_iso: &str,
) -> Result<Option<Consent>, LauncherError> {
    let refuse = || LauncherError::new("CONSENT_REQUIRED", MSG_CONSENT_REQUIRED);
    let Some(Value::Object(consent)) = value else {
        return if required { Err(refuse()) } else { Ok(None) };
    };
    if consent.get("ageConfirmed") != Some(&Value::Bool(true)) {
        return Err(LauncherError::new(
            "CONSENT_AGE_REQUIRED",
            "You must be at least 16 years old to create an account.",
        ));
    }
    let terms = consent.get("termsVersion").and_then(Value::as_str);
    let privacy = consent.get("privacyVersion").and_then(Value::as_str);
    match (terms, privacy) {
        (Some(TERMS_VERSION), Some(privacy)) if is_privacy_accepted(privacy) => Ok(Some(Consent {
            terms_version: TERMS_VERSION.to_owned(),
            privacy_version: privacy.to_owned(),
            accepted_at: now_iso.to_owned(),
        })),
        _ if required => Err(refuse()),
        _ => Ok(None),
    }
}

// Usernames (§6.3)

const NOT_ALLOWED_ANYWHERE: &[&str] = &[
    "fuck",
    "shit",
    "cunt",
    "bitch",
    "whore",
    "slut",
    "nigg",
    "faggot",
    "fagg",
    "retard",
    "penis",
    "vagina",
    "pussy",
    "dildo",
    "porn",
    "jizz",
    "twat",
    "wanker",
    "bollock",
    "asshole",
    "arsehole",
    "motherf",
    "chink",
    "tranny",
    "hitler",
    "kkk",
    "molest",
    "incest",
    "pedophil",
    "paedo",
    "sexy",
    "boob",
    "killyourself",
];
const NOT_ALLOWED_WHOLE: &[&str] = &[
    "cock", "dick", "rape", "rapist", "coon", "spic", "pedo", "cum", "anal", "sex", "tits", "fag", "nig", "kms", "kys",
    "homo", "ass", "arse", "wank", "nazi", "dyke", "kike", "wtf",
];
const RESERVED_ANYWHERE: &[&str] = &[
    "admin",
    "moderat",
    "owner",
    "staff",
    "support",
    "official",
    "developer",
    "gamemaster",
    "rusticisles",
    "mysticparadox",
    "mysticfox",
    "phoenixlabs",
];
const RESERVED_WHOLE: &[&str] = &[
    "mod",
    "mods",
    "dev",
    "devs",
    "gm",
    "root",
    "system",
    "server",
    "paradox",
    "dauntless",
    "null",
    "undefined",
    "everyone",
    "guest",
    "anonymous",
    "host",
    "player",
    "unknown",
];

/// A username outside 3 to 16 characters.
pub const MSG_USERNAME_LENGTH: &str = "Username must be 3\u{2013}16 characters.";
/// A username with anything but letters and digits.
pub const MSG_USERNAME_SYMBOLS: &str = "Username can only contain letters and numbers (no spaces or symbols).";
/// A username with no letter.
pub const MSG_USERNAME_LETTER: &str = "Username needs at least one letter.";
/// A username on a word list or the operator's blocklist.
pub const MSG_USERNAME_NOT_ALLOWED: &str = "That username isn't allowed. Choose another.";
/// A username the service keeps for itself.
pub const MSG_USERNAME_RESERVED: &str = "That username is reserved. Choose another.";

/// The format rules on the JS-trimmed text (a non-string is `""`); the trimmed name when they pass.
pub fn validate_username_format(value: Option<&Value>) -> Result<String, LauncherError> {
    let text = match value {
        Some(Value::String(s)) => service_kit::js::trim(s),
        _ => "",
    };
    let len = util::utf16_len(text);
    if !(3..=16).contains(&len) {
        return Err(LauncherError::validation(MSG_USERNAME_LENGTH));
    }
    if !text.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(LauncherError::validation(MSG_USERNAME_SYMBOLS));
    }
    if !text.bytes().any(|b| b.is_ascii_alphabetic()) {
        return Err(LauncherError::validation(MSG_USERNAME_LETTER));
    }
    Ok(text.to_owned())
}

/// `RUSTIC_ISLES_USERNAME_BLOCKLIST`: JS white space and commas split it; terms of 2 to 32 `a-z0-9` are kept.
pub fn parse_blocklist(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(|c: char| c == ',' || service_kit::js::is_space(c))
        .map(|t| t.trim().to_lowercase())
        .filter(|t| (2..=32).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
        .collect()
}

fn leet(text: &str, one: char) -> String {
    text.chars()
        .map(|c| match c {
            '0' => 'o',
            '1' => one,
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '7' => 't',
            '8' => 'b',
            '9' => 'g',
            other => other,
        })
        .collect()
}

fn forms(name: &str) -> Vec<String> {
    let lower = name.to_lowercase();
    let mut out = vec![lower.clone()];
    for form in [leet(&lower, 'i'), leet(&lower, 'l')] {
        if !out.contains(&form) {
            out.push(form);
        }
    }
    out
}

/// A run of three or more of one character becomes one; doubles stay.
pub fn collapse(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let mut j = i;
        while j < chars.len() && chars[j] == chars[i] {
            j += 1;
        }
        let run = j - i;
        let keep = if run >= 3 { 1 } else { run };
        for _ in 0..keep {
            out.push(chars[i]);
        }
        i = j;
    }
    out
}

fn matches_anywhere<S: AsRef<str>>(forms: &[String], terms: &[S]) -> bool {
    forms.iter().any(|form| {
        let collapsed = collapse(form);
        terms.iter().any(|term| {
            let term = term.as_ref();
            let collapsed_term = collapse(term);
            form.contains(term) || (collapsed_term.chars().count() >= 4 && collapsed.contains(&collapsed_term))
        })
    })
}

fn matches_whole(forms: &[String], terms: &[&str]) -> bool {
    forms.iter().any(|form| {
        let stripped = form.trim_end_matches(|c: char| c.is_ascii_digit());
        let collapsed = collapse(form);
        terms
            .iter()
            .any(|&term| form == term || stripped == term || collapsed == term)
    })
}

/// The word lists, then the operator's blocklist, then the reserved names; the message never names the term.
pub fn check_username_allowed(name: &str, blocklist: &[String]) -> Result<(), LauncherError> {
    let forms = forms(name);
    if matches_anywhere(&forms, NOT_ALLOWED_ANYWHERE)
        || matches_whole(&forms, NOT_ALLOWED_WHOLE)
        || matches_anywhere(&forms, blocklist)
    {
        return Err(LauncherError::validation(MSG_USERNAME_NOT_ALLOWED));
    }
    if matches_anywhere(&forms, RESERVED_ANYWHERE) || matches_whole(&forms, RESERVED_WHOLE) {
        return Err(LauncherError::validation(MSG_USERNAME_RESERVED));
    }
    Ok(())
}

// Host switches (§6.4, §6.5)

/// `RUSTIC_ISLES_EMAIL_VERIFICATION`: `(required, unrecognised)`.
pub fn email_verification(raw: Option<&str>) -> (bool, bool) {
    match util::normalize_lower(raw.unwrap_or("")).as_str() {
        "" | "off" => (false, false),
        "required" => (true, false),
        _ => (true, true),
    }
}

/// `RUSTIC_ISLES_REGISTRATION_APPROVAL`: `(required, the unrecognised value)`.
pub fn registration_approval(raw: Option<&str>) -> (bool, Option<String>) {
    let value = util::normalize_lower(raw.unwrap_or(""));
    match value.as_str() {
        "" | "off" => (false, None),
        "required" => (true, None),
        _ => (true, Some(service_kit::js::trim(raw.unwrap_or("")).to_owned())),
    }
}

/// The answer while official play is off.
pub const MSG_OFFICIAL_UNAVAILABLE: &str = "The official server is currently unavailable. Join or host a realm.";

/// `RUSTIC_ISLES_OFFICIAL_PLAY`: `(available, the unrecognised value)`.
pub fn official_play(raw: Option<&str>, deployserver_url: Option<&str>) -> (bool, Option<String>) {
    let value = util::normalize_lower(raw.unwrap_or(""));
    match value.as_str() {
        "on" => (true, None),
        "off" => (false, None),
        "" => (!service_kit::js::trim(deployserver_url.unwrap_or("")).is_empty(), None),
        _ => (false, Some(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Deletion, Timeout};
    use serde_json::json;

    pub(crate) fn account() -> LauncherAccount {
        LauncherAccount {
            account_id: "account:01J00000000000000000000000@local".into(),
            user_id: "u1".into(),
            email: Some("fox@example.test".into()),
            display_name: "Fox".into(),
            display_name_normalized: "fox".into(),
            password_hash: None,
            status: Status::Active,
            approval: None,
            roles: vec!["player".into()],
            roles_updated_at: None,
            created_at: "2026-01-01T00:00:00.000Z".into(),
            last_login_at: None,
            username_set: None,
            consent: None,
            deletion: None,
            email_verified_at: None,
            email_verification_pending_since: None,
            timeout: None,
        }
    }

    #[test]
    fn eligibility_follows_status_approval_timeout_and_name() {
        let now = "2026-11-05T12:00:00.000Z";
        let legacy = account();
        assert!(assert_account_admitted(&legacy).is_ok());
        assert!(is_account_eligible(&legacy, now));
        for (change, code) in [
            (Some(Approval::Pending), "AUTH_APPROVAL_PENDING"),
            (Some(Approval::Rejected), "AUTH_APPROVAL_REJECTED"),
        ] {
            let mut a = account();
            a.approval = change;
            assert_eq!(assert_account_eligible(&a, now).unwrap_err().code, code);
        }
        for (status, code) in [
            (Status::Disabled, "AUTH_ACCOUNT_DISABLED"),
            (Status::Banned, "AUTH_ACCOUNT_BANNED"),
        ] {
            let mut a = account();
            a.status = status;
            assert_eq!(assert_account_eligible(&a, now).unwrap_err().code, code);
        }
        let mut a = account();
        a.username_set = Some(false);
        assert_eq!(
            assert_account_eligible(&a, now).unwrap_err().code,
            "AUTH_USERNAME_REQUIRED"
        );
        let mut a = account();
        a.timeout = Some(Timeout {
            until: "2026-11-05T13:00:00.000Z".into(),
            since: "2026-11-05T11:00:00.000Z".into(),
            reason: Some("spam".into()),
        });
        let error = assert_account_eligible(&a, now).unwrap_err();
        assert_eq!(error.code, "AUTH_ACCOUNT_TIMED_OUT");
        assert_eq!(error.status(), 500);
        assert_eq!(
            error.message,
            "This account is timed out until 2026-11-05T13:00:00.000Z."
        );
        assert_eq!(error.details[0].0, "timeoutUntil");
        assert_eq!(error.details[1], ("timeoutReason".into(), json!("spam")));
        assert!(assert_account_admitted(&a).is_ok());
        assert!(is_account_eligible(&a, "2026-11-05T13:00:00.000Z"));
    }

    #[test]
    fn consent_defaults_and_rules() {
        assert!(consent_required_at_signup(None, true));
        assert!(!consent_required_at_signup(None, false));
        assert!(!consent_required_at_signup(Some("  "), false));
        assert!(consent_required_at_signup(Some("on"), false));
        assert!(!consent_required_at_signup(Some("off"), true));
        assert!(!consent_enforced(None));
        assert!(consent_enforced(Some("ON")));
        let now = "2026-11-05T00:00:00.000Z";
        assert_eq!(
            registration_consent(None, true, now).unwrap_err().code,
            "CONSENT_REQUIRED"
        );
        assert_eq!(registration_consent(None, false, now), Ok(None));
        assert_eq!(registration_consent(Some(&Value::Null), false, now), Ok(None));
        for age in [json!(false), json!("yes")] {
            let consent =
                json!({"ageConfirmed": age, "termsVersion": TERMS_VERSION, "privacyVersion": PRIVACY_VERSION});
            assert_eq!(
                registration_consent(Some(&consent), false, now).unwrap_err().code,
                "CONSENT_AGE_REQUIRED"
            );
        }
        let old = json!({"ageConfirmed": true, "termsVersion": "2020-01-01", "privacyVersion": PRIVACY_VERSION});
        assert_eq!(
            registration_consent(Some(&old), true, now).unwrap_err().code,
            "CONSENT_REQUIRED"
        );
        assert_eq!(registration_consent(Some(&old), false, now), Ok(None));
        let kept = json!({"ageConfirmed": true, "termsVersion": TERMS_VERSION, "privacyVersion": "2026-10-15"});
        assert_eq!(
            registration_consent(Some(&kept), true, now)
                .unwrap()
                .unwrap()
                .privacy_version,
            "2026-10-15"
        );
        let mut a = account();
        a.deletion = Some(Deletion {
            requested_at: now.into(),
            scheduled_for: "2026-12-05T00:00:00.000Z".into(),
        });
        assert_eq!(assert_not_deactivated(&a).unwrap_err().details[0].0, "scheduledFor");
        let mut a = account();
        assert_eq!(assert_consent_if_enforced(&a, true).unwrap_err().status(), 403);
        assert!(assert_consent_if_enforced(&a, false).is_ok());
        for (privacy, ok) in [
            ("2026-10-16", true),
            ("2026-10-15", true),
            ("2026-10-14.2", true),
            ("2026-10-14", false),
        ] {
            a.consent = Some(Consent {
                terms_version: TERMS_VERSION.into(),
                privacy_version: privacy.into(),
                accepted_at: now.into(),
            });
            assert_eq!(has_current_consent(&a), ok, "{privacy}");
        }
    }

    fn allowed(name: &str, blocklist: &[String]) -> Result<(), String> {
        check_username_allowed(name, blocklist).map_err(|e| e.message)
    }

    #[test]
    fn username_formats() {
        for ok in ["Fox", "MysticFox", "Fox99", "x12", "Sixteen16Chars16"] {
            assert_eq!(validate_username_format(Some(&json!(ok))).unwrap(), ok);
        }
        assert_eq!(
            validate_username_format(Some(&json!("  MysticFox  "))).unwrap(),
            "MysticFox"
        );
        for symbol in [
            "Mystic_Fox",
            "fox!",
            "fox-x",
            "fox x",
            "fox.x",
            "f\u{f3}x",
            "fox@home",
            "<b>fox",
        ] {
            assert_eq!(
                validate_username_format(Some(&json!(symbol))).unwrap_err().message,
                MSG_USERNAME_SYMBOLS
            );
        }
        assert_eq!(
            validate_username_format(Some(&json!("123"))).unwrap_err().message,
            MSG_USERNAME_LETTER
        );
        for short in [json!("ab"), json!("A".repeat(17)), json!(5)] {
            assert_eq!(
                validate_username_format(Some(&short)).unwrap_err().message,
                MSG_USERNAME_LENGTH
            );
        }
        assert_eq!(validate_username_format(None).unwrap_err().message, MSG_USERNAME_LENGTH);
    }

    #[test]
    fn username_word_lists() {
        let reserved = "Admin AdminBob TheAdmin7 Moderator ModeratorX TheOwner Owner1 StaffFox Support FoxSupport \
            Official Developer GameMaster RusticIsles MysticParadox MysticFox MysticFox2 PhoenixLabs Mod mod7 Dev GM \
            Root Paradox Paradox99 Dauntless Player1 Host Guest Null";
        for name in reserved.split_whitespace() {
            assert_eq!(allowed(name, &[]), Err(MSG_USERNAME_RESERVED.into()), "{name}");
        }
        let refused = "Fuuuuck Shiiiit Sh1t B1tch N1gg4 Pen15 Wh0re S3xy Dick Dick69 D1ck Cock Rape Nazi Hitler \
            KKKfox Cum Tits Wanker Rapist Kys Killyourself Pussy Porn Faggot Retard Kike Coon fucker Bigshit nigga nigger7";
        for name in refused.split_whitespace() {
            assert_eq!(allowed(name, &[]), Err(MSG_USERNAME_NOT_ALLOWED.into()), "{name}");
        }
        let pass = "Peacock Hancock Grapefruit Therapist Raccoon Torpedo Sussex Essex Sextant Skyshard Swanky Nazir \
            VanDyke Kikeo Assassin Classic Cassandra Bassist Cumulus Analyst Dickens Titan Shiitake Night Nigel Niger \
            Homer Assault Moderna ParadoxHunter Hostage Embermane Mysticzzz Tempest7 QuietFox Kestrel Brimvane R4in \
            Fox99 Glass Scrapes Drape Spice Cockpit";
        for name in pass.split_whitespace() {
            assert_eq!(allowed(name, &[]), Ok(()), "{name}");
        }
        let blocklist = parse_blocklist(Some("Grief, toxic  b@d x"));
        assert_eq!(blocklist, ["grief", "toxic"]);
        for name in ["GriefLord", "ToXiCfox", "Toxic7"] {
            assert_eq!(
                allowed(name, &blocklist),
                Err(MSG_USERNAME_NOT_ALLOWED.into()),
                "{name}"
            );
        }
        for name in ["Badger", "Xenon"] {
            assert_eq!(allowed(name, &blocklist), Ok(()));
        }
        assert_eq!(allowed("GriefLord", &[]), Ok(()));
    }

    #[test]
    fn switches() {
        assert_eq!(email_verification(None), (false, false));
        assert_eq!(email_verification(Some("  ")), (false, false));
        assert_eq!(email_verification(Some("OFF")), (false, false));
        assert_eq!(email_verification(Some(" Required ")), (true, false));
        for odd in ["on", "true", "maybe", "requried"] {
            assert_eq!(email_verification(Some(odd)), (true, true));
        }
        assert_eq!(registration_approval(None), (false, None));
        assert_eq!(registration_approval(Some(" OFF ")), (false, None));
        assert_eq!(registration_approval(Some(" Required ")), (true, None));
        assert_eq!(registration_approval(Some("requried")), (true, Some("requried".into())));
        assert!(official_play(None, Some("https://deploy")).0);
        assert!(!official_play(None, None).0);
        assert!(!official_play(None, Some(" ")).0);
        assert!(official_play(Some(""), Some("https://deploy")).0);
        assert!(official_play(Some(" ON "), None).0);
        assert!(!official_play(Some("Off"), Some("https://deploy")).0);
        for odd in ["1", "0", "yes", "true", "disabled", "onn", "of"] {
            let (available, warned) = official_play(Some(odd), Some("https://deploy"));
            assert!(!available && warned.is_some(), "{odd}");
        }
    }
}
