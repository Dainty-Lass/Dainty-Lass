//! The auth rules that are pure: who is admitted (`security/accountEligibility.ts`), consent and deactivation
//! (`security/accountConsent.ts`, `legal/legalPolicy.ts`, `legal/retentionPolicy.ts`), usernames
//! (`security/usernamePolicy.ts`), and the host's switches for email verification, registration approval, official
//! play and unofficial launchers (`config/*.ts`, `security/unofficialLauncherPolicy.ts`). Each switch is read from the
//! module's [`Settings`] at each call, as Node read `process.env`.

use std::collections::BTreeSet;
use std::sync::{LazyLock, Mutex, PoisonError};

use serde_json::Value;
use store::Engine;

use super::errors::LauncherApiError;
use crate::repo::auth::{ApprovalStatus, LauncherAccountRecord, LauncherConsent, OperationalStatus};
use crate::settings::Settings;

// ------------------------------------------------------------------ legal

pub const TERMS_VERSION: &str = "2026-10-14";
pub const PRIVACY_VERSION: &str = "2026-10-17";
pub const MINIMUM_AGE: u32 = 16;

/// Earlier privacy versions still counted as accepted, for the launchers that carry them.
pub const PRIVACY_VERSIONS_STILL_ACCEPTED: [&str; 3] = ["2026-10-16", "2026-10-15", "2026-10-14.2"];

/// A deletion request keeps the account deactivated and restorable for this many days.
pub const DELETION_HOLD_DAYS: i64 = 30;
pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegalRequirements {
    pub terms_version: String,
    pub privacy_version: String,
    pub minimum_age: u32,
}

pub fn current_legal_requirements() -> LegalRequirements {
    LegalRequirements {
        terms_version: TERMS_VERSION.to_owned(),
        privacy_version: PRIVACY_VERSION.to_owned(),
        minimum_age: MINIMUM_AGE,
    }
}

/// `IsAcceptedPrivacyVersion`: the current one, or one still accepted during its transition.
pub fn is_accepted_privacy_version(version: Option<&str>, required: &LegalRequirements) -> bool {
    let Some(version) = version else { return false };
    version == required.privacy_version
        || (required.privacy_version == PRIVACY_VERSION && PRIVACY_VERSIONS_STILL_ACCEPTED.contains(&version))
}

// ------------------------------------------------------------------ eligibility

/// Legacy launcher accounts default to approved.
pub fn effective_approval_status(account: &LauncherAccountRecord) -> ApprovalStatus {
    account.approval_status.unwrap_or(ApprovalStatus::Approved)
}

pub fn assert_account_admitted(account: &LauncherAccountRecord) -> Result<(), LauncherApiError> {
    match account.status {
        OperationalStatus::Banned => {
            return Err(LauncherApiError::new(
                "AUTH_ACCOUNT_BANNED",
                "This account has been banned.",
            ));
        }
        OperationalStatus::Disabled => {
            return Err(LauncherApiError::new(
                "AUTH_ACCOUNT_DISABLED",
                "This account has been disabled.",
            ));
        }
        OperationalStatus::Active => {}
    }
    match effective_approval_status(account) {
        ApprovalStatus::Pending => Err(LauncherApiError::new(
            "AUTH_APPROVAL_PENDING",
            "Your closed-test access request is waiting for approval.",
        )),
        ApprovalStatus::Rejected => Err(LauncherApiError::new(
            "AUTH_APPROVAL_REJECTED",
            "Your closed-test access request was not approved.",
        )),
        ApprovalStatus::Approved => Ok(()),
    }
}

/// May the account play: admitted, not timed out, with a username. A timed-out account still signs in to the
/// launcher (`assert_account_admitted`), so the player sees when they can play again.
pub fn assert_account_eligible(account: &LauncherAccountRecord) -> Result<(), LauncherApiError> {
    assert_account_eligible_at(account, &super::util::iso_now())
}

pub fn assert_account_eligible_at(account: &LauncherAccountRecord, now: &str) -> Result<(), LauncherApiError> {
    assert_account_admitted(account)?;
    assert_not_timed_out(account, now)?;
    if account.username_set == Some(false) {
        return Err(LauncherApiError::new(
            "AUTH_USERNAME_REQUIRED",
            "Choose a launcher username before playing.",
        ));
    }
    Ok(())
}

/// A timeout in force refuses play with its end (and reason) as details, for the launcher to show in the player's
/// locale.
pub fn assert_not_timed_out(account: &LauncherAccountRecord, now: &str) -> Result<(), LauncherApiError> {
    let Some(timeout) = account.timeout_at(now) else {
        return Ok(());
    };
    let mut error = LauncherApiError::new(
        "AUTH_ACCOUNT_TIMED_OUT",
        format!("This account is timed out until {}.", timeout.until),
    );
    error
        .details
        .push(("timeoutUntil".to_owned(), Value::String(timeout.until.clone())));
    if let Some(reason) = &timeout.reason {
        error
            .details
            .push(("timeoutReason".to_owned(), Value::String(reason.clone())));
    }
    Err(error)
}

pub fn is_account_eligible(account: &LauncherAccountRecord) -> bool {
    assert_account_eligible(account).is_ok()
}

// ------------------------------------------------------------------ consent

fn trimmed_lower(settings: &Settings, name: &str) -> String {
    service_kit::js::trim(settings.raw(name).unwrap_or_default()).to_lowercase()
}

/// `RUSTIC_ISLES_CONSENT_REQUIRED`: an explicit value wins (off/false/0/no turns it off, anything else on). Blank
/// follows the database: Node defaulted to on with MongoDB, the official backend, and off with its local SQLite.
/// Here a Postgres node is the shared one, so it defaults to on, and a SQLite server to off.
pub fn consent_required_at_sign_up(settings: &Settings, engine: Option<Engine>) -> bool {
    let raw = trimmed_lower(settings, "RUSTIC_ISLES_CONSENT_REQUIRED");
    if raw.is_empty() {
        return engine == Some(Engine::Postgres);
    }
    !matches!(raw.as_str(), "0" | "false" | "off" | "no")
}

/// `RUSTIC_ISLES_CONSENT_ENFORCE`: off unless on.
pub fn consent_enforced(settings: &Settings) -> bool {
    matches!(
        trimmed_lower(settings, "RUSTIC_ISLES_CONSENT_ENFORCE").as_str(),
        "1" | "true" | "on" | "yes"
    )
}

pub fn is_consent_current(consent: Option<&LauncherConsent>, required: &LegalRequirements) -> bool {
    consent.is_some_and(|consent| {
        consent.terms_version == required.terms_version
            && is_accepted_privacy_version(Some(&consent.privacy_version), required)
    })
}

pub fn consent_required_error() -> LauncherApiError {
    LauncherApiError::new(
        "CONSENT_REQUIRED",
        "Update the launcher to read and accept the current Terms of Use and Privacy Policy, then try again.",
    )
}

/// `ValidateRegistrationConsent`: the consent a registration carries. Absent: refused when consent is required,
/// otherwise nothing is recorded. A consent that does not confirm age is refused; one for other versions is refused
/// when consent is required and ignored otherwise.
pub fn validate_registration_consent(
    raw: Option<&Value>,
    now_iso: &str,
    settings: &Settings,
    engine: Option<Engine>,
    required: &LegalRequirements,
) -> Result<Option<LauncherConsent>, LauncherApiError> {
    let needed = consent_required_at_sign_up(settings, engine);
    let Some(body) = raw.and_then(Value::as_object) else {
        return if needed {
            Err(consent_required_error())
        } else {
            Ok(None)
        };
    };
    if body.get("ageConfirmed") != Some(&Value::Bool(true)) {
        return Err(LauncherApiError::new(
            "CONSENT_AGE_REQUIRED",
            format!(
                "You must be at least {} years old to create an account.",
                required.minimum_age
            ),
        ));
    }
    let terms_ok = body.get("termsVersion").and_then(Value::as_str) == Some(required.terms_version.as_str());
    let privacy = body.get("privacyVersion").and_then(Value::as_str);
    if !terms_ok || !is_accepted_privacy_version(privacy, required) {
        return if needed {
            Err(consent_required_error())
        } else {
            Ok(None)
        };
    }
    Ok(Some(LauncherConsent {
        terms_version: required.terms_version.clone(),
        // The version the player was shown (the current one, or one still accepted during its transition)
        privacy_version: privacy.unwrap_or_default().to_owned(),
        accepted_at: now_iso.to_owned(),
    }))
}

/// The refusal of an account with a deletion request: 403 ACCOUNT_DEACTIVATED with `scheduledFor` beside the code.
pub fn assert_account_not_deactivated(account: &LauncherAccountRecord) -> Result<(), LauncherApiError> {
    let Some(deletion) = &account.deletion else {
        return Ok(());
    };
    Err(LauncherApiError::new(
        "ACCOUNT_DEACTIVATED",
        "This account is scheduled for deletion and is deactivated. Restore it from the Account page to play again.",
    )
    .with_detail("scheduledFor", Value::String(deletion.scheduled_for.clone())))
}

/// Game sessions and realm hosting, when `RUSTIC_ISLES_CONSENT_ENFORCE` is on: 403 CONSENT_REQUIRED unless current.
pub fn assert_consent_current_when_enforced(
    account: &LauncherAccountRecord,
    settings: &Settings,
) -> Result<(), LauncherApiError> {
    if !consent_enforced(settings) || is_consent_current(account.consent.as_ref(), &current_legal_requirements()) {
        return Ok(());
    }
    Err(consent_required_error().with_status(403))
}

// ------------------------------------------------------------------ usernames

pub const USERNAME_MIN: usize = 3;
pub const USERNAME_MAX: usize = 16;
pub const USERNAME_RESERVED_MESSAGE: &str = "That username is reserved. Choose another.";
pub const USERNAME_NOT_ALLOWED_MESSAGE: &str = "That username isn't allowed. Choose another.";

/// Staff and official words refused anywhere in a name.
const IMPERSONATION_ANYWHERE: [&str; 12] = [
    "admin",
    "moderat",
    "owner",
    "staff",
    "support",
    "official",
    "developer",
    "gamemaster",
    "rusticisles",
    // The project's former name, still refused (crate::legacy)
    "mysticparadox",
    "mysticfox",
    "phoenixlabs",
];

/// Short or common words refused only as the whole name (trailing digits ignored).
const RESERVED_WHOLE: [&str; 18] = [
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

/// Vulgar, abusive and slur terms refused anywhere in a name.
const OFFENSIVE_ANYWHERE: [&str; 33] = [
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

/// Terms that also occur inside ordinary words or names: refused only as the whole name.
const OFFENSIVE_WHOLE: [&str; 23] = [
    "cock", "dick", "rape", "rapist", "coon", "spic", "pedo", "cum", "anal", "sex", "tits", "fag", "nig", "kms", "kys",
    "homo", "ass", "arse", "wank", "nazi", "dyke", "kike", "wtf",
];

/// Lower case, and digits read as the letters they imitate (0=o, 1=i or l, 3=e, 4=a, 5=s, 7=t, 8=b, 9=g).
fn name_forms(username: &str) -> Vec<String> {
    let lower = username.to_lowercase();
    let leet = |one: char| -> String {
        lower
            .chars()
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
    };
    let mut forms = vec![lower.clone(), leet('i'), leet('l')];
    forms.sort();
    forms.dedup();
    forms
}

/// Runs of three or more of one letter collapsed to one ("fuuuck" -> "fuck"); double letters stay.
fn collapse(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut j = i + 1;
        // JavaScript's `.` does not match a line terminator
        let breaks = matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
        while !breaks && j < chars.len() && chars[j] == c {
            j += 1;
        }
        if j - i >= 3 {
            out.push(c);
        } else {
            out.extend(&chars[i..j]);
        }
        i = j;
    }
    out
}

fn operator_blocklist(settings: &Settings) -> Vec<String> {
    settings
        .raw("RUSTIC_ISLES_USERNAME_BLOCKLIST")
        .unwrap_or_default()
        .split(|c: char| service_kit::js::is_space(c) || c == ',')
        .map(|term| term.trim().to_lowercase())
        .filter(|term| {
            (2..=32).contains(&term.len()) && term.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
        .collect()
}

fn contains_any(forms: &[String], terms: &[String]) -> bool {
    forms
        .iter()
        .any(|form| terms.iter().any(|term| form.contains(term.as_str())))
}

/// A collapsed spelling is compared only with terms whose own collapsed spelling keeps 4 or more letters, so "nigg"
/// (-> "nig") never matches "Night" through it.
fn contains_term(forms: &[String], terms: &[String]) -> bool {
    if contains_any(forms, terms) {
        return true;
    }
    let collapsed_terms: Vec<String> = terms
        .iter()
        .map(|t| collapse(t))
        .filter(|t| t.chars().count() >= 4)
        .collect();
    let collapsed_forms: Vec<String> = forms.iter().map(|f| collapse(f)).collect();
    contains_any(&collapsed_forms, &collapsed_terms)
}

/// A form, without its trailing digits, is one of the terms ("Dick69", "m0d").
fn is_whole(forms: &[String], terms: &[&str]) -> bool {
    let mut bare = BTreeSet::new();
    for form in forms {
        bare.insert(form.clone());
        bare.insert(form.trim_end_matches(|c: char| c.is_ascii_digit()).to_owned());
        bare.insert(collapse(form));
    }
    terms.iter().any(|term| bare.contains(*term))
}

fn strings(terms: &[&str]) -> Vec<String> {
    terms.iter().map(|t| (*t).to_owned()).collect()
}

/// `ValidateUsername`: trims edge whitespace, then the format rules. Returns the clean value.
pub fn validate_username(raw: Option<&Value>) -> Result<String, LauncherApiError> {
    let username = raw
        .and_then(Value::as_str)
        .map(service_kit::js::trim)
        .unwrap_or_default();
    let length = username.encode_utf16().count();
    if !(USERNAME_MIN..=USERNAME_MAX).contains(&length) {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            format!("Username must be {USERNAME_MIN}\u{2013}{USERNAME_MAX} characters."),
        ));
    }
    if !username.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "Username can only contain letters and numbers (no spaces or symbols).",
        ));
    }
    if !username.bytes().any(|b| b.is_ascii_alphabetic()) {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "Username needs at least one letter.",
        ));
    }
    Ok(username.to_owned())
}

/// `AssertUsernameAllowed`: refuses a name that impersonates staff or the project, or is vulgar, abusive or a slur,
/// including the operator's own terms (which message never says which part matched). Call it after [`validate_username`].
pub fn assert_username_allowed(username: &str, settings: &Settings) -> Result<(), LauncherApiError> {
    let forms = name_forms(service_kit::js::trim(username));
    if contains_term(&forms, &strings(&OFFENSIVE_ANYWHERE))
        || is_whole(&forms, &OFFENSIVE_WHOLE)
        || contains_term(&forms, &operator_blocklist(settings))
    {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            USERNAME_NOT_ALLOWED_MESSAGE,
        ));
    }
    if contains_term(&forms, &strings(&IMPERSONATION_ANYWHERE)) || is_whole(&forms, &RESERVED_WHOLE) {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            USERNAME_RESERVED_MESSAGE,
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------ switches

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmailVerificationPolicy {
    pub required: bool,
    /// The variable is unset or blank
    pub is_default: bool,
    /// Set when the value was not recognised (the mode is then required); the value is not repeated
    pub unrecognised: bool,
}

/// `ResolveEmailVerification`: only required and off are recognised; blank is off; anything else is required.
pub fn resolve_email_verification(settings: &Settings) -> EmailVerificationPolicy {
    match trimmed_lower(settings, "RUSTIC_ISLES_EMAIL_VERIFICATION").as_str() {
        "" => EmailVerificationPolicy {
            required: false,
            is_default: true,
            unrecognised: false,
        },
        "off" => EmailVerificationPolicy {
            required: false,
            is_default: false,
            unrecognised: false,
        },
        "required" => EmailVerificationPolicy {
            required: true,
            is_default: false,
            unrecognised: false,
        },
        _ => EmailVerificationPolicy {
            required: true,
            is_default: false,
            unrecognised: true,
        },
    }
}

pub fn email_verification_required(settings: &Settings) -> bool {
    resolve_email_verification(settings).required
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationApprovalPolicy {
    pub required: bool,
    pub approval_status: ApprovalStatus,
    pub is_default: bool,
    pub unrecognised: Option<String>,
    /// The replaced sqlite-only switch is still set; it has no effect
    pub legacy_warning: bool,
}

/// `ResolveRegistrationApproval`: what a new launcher account starts with, on either database.
pub fn resolve_registration_approval(settings: &Settings) -> RegistrationApprovalPolicy {
    let raw = trimmed_lower(settings, "RUSTIC_ISLES_REGISTRATION_APPROVAL");
    let legacy = !service_kit::js::trim(
        settings
            .raw("RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE")
            .unwrap_or_default(),
    )
    .is_empty();
    let mut policy = RegistrationApprovalPolicy {
        required: false,
        approval_status: ApprovalStatus::Approved,
        is_default: false,
        unrecognised: None,
        legacy_warning: legacy,
    };
    match raw.as_str() {
        "" => policy.is_default = true,
        "off" => {}
        "required" => {
            policy.required = true;
            policy.approval_status = ApprovalStatus::Pending;
        }
        other => {
            policy.required = true;
            policy.approval_status = ApprovalStatus::Pending;
            policy.unrecognised = Some(other.to_owned());
        }
    }
    policy
}

pub const OFFICIAL_PLAY_UNAVAILABLE_MESSAGE: &str =
    "The official server is currently unavailable. Join or host a realm.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialPlayStatus {
    pub available: bool,
    pub message: Option<&'static str>,
}

/// `DedicatedServersConfigured`: `DEPLOYSERVER_URL` holds something other than blank or whitespace.
pub fn dedicated_servers_configured(settings: &Settings) -> bool {
    !service_kit::js::trim(settings.raw("DEPLOYSERVER_URL").unwrap_or_default()).is_empty()
}

static REPORTED_OFFICIAL_PLAY: LazyLock<Mutex<BTreeSet<String>>> = LazyLock::new(Mutex::default);

/// `ReadOfficialPlay`: `RUSTIC_ISLES_OFFICIAL_PLAY=on|off`; blank is on with a Director to send hunts to (Node's
/// DeployServer), otherwise off; any other value fails closed to off, with one warning.
pub fn read_official_play(settings: &Settings) -> OfficialPlayStatus {
    let unavailable = OfficialPlayStatus {
        available: false,
        message: Some(OFFICIAL_PLAY_UNAVAILABLE_MESSAGE),
    };
    let available = OfficialPlayStatus {
        available: true,
        message: None,
    };
    match trimmed_lower(settings, "RUSTIC_ISLES_OFFICIAL_PLAY").as_str() {
        "on" => available,
        "off" => unavailable,
        "" if dedicated_servers_configured(settings) => available,
        "" => unavailable,
        other => {
            let refusal = format!(
                "RUSTIC_ISLES_OFFICIAL_PLAY must be on or off (got {other:?}); official play is treated as off"
            );
            let first = REPORTED_OFFICIAL_PLAY
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(refusal.clone());
            if first {
                tracing::warn!("[OfficialPlay] WARNING: {refusal}");
            }
            unavailable
        }
    }
}

pub const UNOFFICIAL_LAUNCHER_MESSAGE: &str =
    "This launcher isn't an official Rustic Isles build. Install the official launcher to play on Rustic Isles.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnofficialLauncherPolicy {
    pub refuse: bool,
    pub allowed_hashes: BTreeSet<String>,
    pub admins_exempt: bool,
    pub warnings: Vec<String>,
}

/// `ReadUnofficialLauncherPolicy`.
pub fn read_unofficial_launcher_policy(settings: &Settings) -> UnofficialLauncherPolicy {
    let mut warnings = Vec::new();
    let raw = trimmed_lower(settings, "RUSTIC_ISLES_UNOFFICIAL_LAUNCHER");
    let refuse = raw == "refuse";
    if !refuse && !raw.is_empty() && raw != "observe" {
        let shown: String = raw.chars().take(32).collect();
        warnings.push(format!(
            "RUSTIC_ISLES_UNOFFICIAL_LAUNCHER must be observe or refuse (got {shown:?}); observe applies"
        ));
    }
    let mut allowed = BTreeSet::new();
    let mut malformed = 0;
    for entry in settings
        .raw("RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES")
        .unwrap_or_default()
        .split(|c: char| service_kit::js::is_space(c) || c == ',')
    {
        let hash = entry.trim().to_lowercase();
        if hash.is_empty() {
            continue;
        }
        if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            allowed.insert(hash);
        } else {
            malformed += 1;
        }
    }
    if malformed > 0 {
        warnings.push(format!(
            "RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES: {malformed} entr{} not a SHA-256 and ignored",
            if malformed == 1 { "y is" } else { "ies are" }
        ));
    }
    let exempt_raw = trimmed_lower(settings, "RUSTIC_ISLES_UNOFFICIAL_LAUNCHER_TESTERS_EXEMPT");
    let admins_exempt = !matches!(exempt_raw.as_str(), "0" | "false" | "off" | "no");
    UnofficialLauncherPolicy {
        refuse,
        allowed_hashes: allowed,
        admins_exempt,
        warnings,
    }
}

/// What the launcher says about itself at Play: its version and the SHA-256 of its own `launcher.exe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherBuildReport {
    pub version: String,
    pub sha256: String,
}

/// official: the hash Guard pins; outdated: another hash on an older version; unofficial: another hash on the served
/// version or newer; unknown: no report, or nothing pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherBuildVerdict {
    Official,
    Outdated,
    Unofficial,
    Unknown,
}

/// `ParseLauncherBuildReport`: absent or malformed is `None`, never a refusal.
pub fn parse_launcher_build_report(value: Option<&Value>) -> Option<LauncherBuildReport> {
    let object = value?.as_object()?;
    let version = object.get("version")?.as_str()?;
    let sha256 = object.get("sha256")?.as_str()?;
    let digits = |part: &str| (1..=5).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit());
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let core_parts: Vec<&str> = core.split('.').collect();
    let version_ok = core_parts.len() == 3
        && core_parts.iter().all(|p| digits(p))
        && pre.is_none_or(|pre| {
            (1..=32).contains(&pre.len()) && pre.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        });
    let sha_ok = sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit());
    (version_ok && sha_ok).then(|| LauncherBuildReport {
        version: version.to_owned(),
        sha256: sha256.to_lowercase(),
    })
}

/// `ClassifyLauncherBuild`.
pub fn classify_launcher_build(
    report: Option<&LauncherBuildReport>,
    pinned_sha256: Option<&str>,
    served_version: Option<&str>,
) -> LauncherBuildVerdict {
    let (Some(report), Some(pinned)) = (report, pinned_sha256) else {
        return LauncherBuildVerdict::Unknown;
    };
    if report.sha256 == pinned.to_lowercase() {
        return LauncherBuildVerdict::Official;
    }
    if served_version.is_some_and(|served| !version_at_least(&report.version, served)) {
        return LauncherBuildVerdict::Outdated;
    }
    LauncherBuildVerdict::Unofficial
}

/// `UnofficialLauncherRefuses`: only an unofficial verdict can be refused.
pub fn unofficial_launcher_refuses(
    verdict: LauncherBuildVerdict,
    report: Option<&LauncherBuildReport>,
    roles: &[String],
    policy: &UnofficialLauncherPolicy,
) -> bool {
    if !policy.refuse || verdict != LauncherBuildVerdict::Unofficial {
        return false;
    }
    if report.is_some_and(|report| policy.allowed_hashes.contains(&report.sha256.to_lowercase())) {
        return false;
    }
    !(policy.admins_exempt && roles.iter().any(|role| role == "admin"))
}

struct ParsedVersion {
    core: [u128; 3],
    prerelease: Option<Vec<String>>,
}

fn parse_version(text: &str) -> Option<ParsedVersion> {
    let text = service_kit::js::trim(text);
    let text = text.split_once('+').map_or(text, |(version, build)| {
        let build_ok = !build.is_empty()
            && build
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if build_ok { version } else { text }
    });
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || !parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if pre
        .is_some_and(|pre| pre.is_empty() || !pre.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    {
        return None;
    }
    // BigInt in Node: a part too large for u128 is treated as the largest
    let number = |p: &str| p.parse::<u128>().unwrap_or(u128::MAX);
    Some(ParsedVersion {
        core: [number(parts[0]), number(parts[1]), number(parts[2])],
        prerelease: pre.map(|pre| pre.split('.').map(str::to_owned).collect()),
    })
}

/// `VersionAtLeast` (`security/launcherVersion.ts`): semver precedence with prereleases.
pub fn version_at_least(actual: &str, minimum: &str) -> bool {
    let (Some(left), Some(right)) = (parse_version(actual), parse_version(minimum)) else {
        return false;
    };
    for index in 0..3 {
        if left.core[index] != right.core[index] {
            return left.core[index] > right.core[index];
        }
    }
    let Some(left_pre) = left.prerelease else { return true };
    let Some(right_pre) = right.prerelease else {
        return false;
    };
    for index in 0..left_pre.len().max(right_pre.len()) {
        let (Some(a), b) = (left_pre.get(index), right_pre.get(index)) else {
            return true;
        };
        let Some(b) = b else { return false };
        if a == b {
            continue;
        }
        let numeric = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        return match (numeric(a), numeric(b)) {
            (true, true) => a.parse::<u128>().unwrap_or(u128::MAX) > b.parse::<u128>().unwrap_or(u128::MAX),
            (x, y) if x != y => !x,
            _ => a > b,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::repo::auth::LauncherAccountRecord;
    use crate::{AccountId, UserId};

    const NOW: &str = "2026-10-14T12:00:00.000Z";

    fn account(update: impl FnOnce(&mut LauncherAccountRecord)) -> LauncherAccountRecord {
        let node = module_api::NodeIdentity::standalone().node;
        let mut record = LauncherAccountRecord {
            account: AccountId::new(&node),
            user_id: UserId("user-1".into()),
            email: None,
            display_name_normalized: "slayer".into(),
            display_name: "Slayer".into(),
            password_hash: None,
            status: OperationalStatus::Active,
            approval_status: None,
            approval_updated_at: None,
            approval_updated_by: None,
            approval_reason: None,
            roles: vec!["player".into()],
            roles_updated_at: None,
            roles_updated_by: None,
            created_at: NOW.into(),
            last_login_at: None,
            username_set: Some(true),
            consent: None,
            deletion: None,
            email_verified_at: None,
            email_verification_pending_since: None,
            timeout: None,
        };
        update(&mut record);
        record
    }

    fn consent(terms: &str, privacy: &str) -> LauncherConsent {
        LauncherConsent {
            terms_version: terms.into(),
            privacy_version: privacy.into(),
            accepted_at: NOW.into(),
        }
    }

    fn code(result: Result<impl std::fmt::Debug, LauncherApiError>) -> String {
        result.unwrap_err().code
    }

    #[test]
    fn legacy_accounts_remain_approved_and_the_others_are_ineligible() {
        assert_eq!(effective_approval_status(&account(|_| {})), ApprovalStatus::Approved);
        assert!(is_account_eligible(&account(|_| {})));
        assert!(!is_account_eligible(&account(
            |a| a.approval_status = Some(ApprovalStatus::Pending)
        )));
        assert!(!is_account_eligible(&account(
            |a| a.approval_status = Some(ApprovalStatus::Rejected)
        )));
        assert!(!is_account_eligible(&account(
            |a| a.status = OperationalStatus::Disabled
        )));
        assert!(!is_account_eligible(&account(|a| a.status = OperationalStatus::Banned)));
        assert!(!is_account_eligible(&account(|a| a.username_set = Some(false))));
    }

    #[test]
    fn a_timeout_stops_play_until_it_ends_but_not_sign_in() {
        let timed_out = account(|a| {
            a.timeout = Some(crate::repo::auth::AccountTimeout {
                until: "2026-10-14T13:00:00.000Z".into(),
                since: NOW.into(),
                reason: Some("spam".into()),
            })
        });
        assert!(assert_account_admitted(&timed_out).is_ok());
        let error = assert_account_eligible_at(&timed_out, NOW).unwrap_err();
        assert_eq!(error.code, "AUTH_ACCOUNT_TIMED_OUT");
        assert_eq!(
            error.details,
            [
                ("timeoutUntil".to_owned(), Value::from("2026-10-14T13:00:00.000Z")),
                ("timeoutReason".to_owned(), Value::from("spam")),
            ]
        );
        assert!(
            assert_account_eligible_at(&timed_out, "2026-10-14T13:00:00.000Z").is_ok(),
            "it ends on time"
        );
        assert!(assert_account_eligible_at(&timed_out, "2026-10-15T00:00:00.000Z").is_ok());
    }

    #[test]
    fn consent_required_follows_the_database_unless_a_value_is_set() {
        let sqlite = Some(Engine::Sqlite);
        let postgres = Some(Engine::Postgres);
        let none = Settings::default();
        assert!(consent_required_at_sign_up(&none, postgres));
        assert!(!consent_required_at_sign_up(&none, sqlite));
        assert!(!consent_required_at_sign_up(&none, None));
        let on = Settings::from_pairs([("RUSTIC_ISLES_CONSENT_REQUIRED", "on")]);
        assert!(consent_required_at_sign_up(&on, sqlite));
        let off = Settings::from_pairs([("RUSTIC_ISLES_CONSENT_REQUIRED", "off")]);
        assert!(!consent_required_at_sign_up(&off, postgres));
        let blank = Settings::from_pairs([("RUSTIC_ISLES_CONSENT_REQUIRED", "   ")]);
        assert!(!consent_required_at_sign_up(&blank, sqlite));
        assert!(!consent_enforced(&none));
        assert!(!consent_enforced(&Settings::from_pairs([(
            "RUSTIC_ISLES_CONSENT_ENFORCE",
            "off"
        )])));
        assert!(consent_enforced(&Settings::from_pairs([(
            "RUSTIC_ISLES_CONSENT_ENFORCE",
            "ON"
        )])));
    }

    #[test]
    fn a_consent_is_current_only_for_the_current_versions() {
        let required = current_legal_requirements();
        assert!(!is_consent_current(None, &required));
        assert!(is_consent_current(
            Some(&consent(TERMS_VERSION, PRIVACY_VERSION)),
            &required
        ));
        assert!(!is_consent_current(
            Some(&consent("2026-01-01", PRIVACY_VERSION)),
            &required
        ));
        assert!(!is_consent_current(
            Some(&consent(TERMS_VERSION, "2026-01-01")),
            &required
        ));
    }

    #[test]
    fn registration_consent_is_refused_or_recorded_as_node_does() {
        let required = current_legal_requirements();
        let postgres = Some(Engine::Postgres);
        let sqlite = Some(Engine::Sqlite);
        let none = Settings::default();
        let current = json!({"termsVersion": TERMS_VERSION, "privacyVersion": PRIVACY_VERSION, "ageConfirmed": true});
        assert_eq!(
            code(validate_registration_consent(None, NOW, &none, postgres, &required)),
            "CONSENT_REQUIRED"
        );
        assert_eq!(
            validate_registration_consent(None, NOW, &none, sqlite, &required).unwrap(),
            None
        );
        assert_eq!(
            validate_registration_consent(Some(&Value::Null), NOW, &none, sqlite, &required).unwrap(),
            None
        );
        let no_age = json!({"termsVersion": TERMS_VERSION, "privacyVersion": PRIVACY_VERSION, "ageConfirmed": false});
        assert_eq!(
            code(validate_registration_consent(
                Some(&no_age),
                NOW,
                &none,
                sqlite,
                &required
            )),
            "CONSENT_AGE_REQUIRED"
        );
        let yes = json!({"termsVersion": TERMS_VERSION, "privacyVersion": PRIVACY_VERSION, "ageConfirmed": "yes"});
        assert_eq!(
            code(validate_registration_consent(
                Some(&yes),
                NOW,
                &none,
                postgres,
                &required
            )),
            "CONSENT_AGE_REQUIRED"
        );
        let old = json!({"termsVersion": "2026-01-01", "privacyVersion": PRIVACY_VERSION, "ageConfirmed": true});
        assert_eq!(
            code(validate_registration_consent(
                Some(&old),
                NOW,
                &none,
                postgres,
                &required
            )),
            "CONSENT_REQUIRED"
        );
        assert_eq!(
            validate_registration_consent(Some(&old), NOW, &none, sqlite, &required).unwrap(),
            None
        );
        assert_eq!(
            validate_registration_consent(Some(&current), NOW, &none, postgres, &required).unwrap(),
            Some(consent(TERMS_VERSION, PRIVACY_VERSION))
        );
        // The version the player was shown is the one kept
        let older = json!({"termsVersion": TERMS_VERSION, "privacyVersion": "2026-10-15", "ageConfirmed": true});
        assert_eq!(
            validate_registration_consent(Some(&older), NOW, &none, postgres, &required)
                .unwrap()
                .unwrap()
                .privacy_version,
            "2026-10-15"
        );
    }

    #[test]
    fn a_deactivated_account_carries_its_scheduled_erasure_time() {
        assert!(assert_account_not_deactivated(&account(|_| {})).is_ok());
        let deactivated = account(|a| {
            a.deletion = Some(crate::repo::auth::LauncherDeletion {
                requested_at: NOW.into(),
                scheduled_for: "2026-11-13T12:00:00.000Z".into(),
            });
        });
        let refused = assert_account_not_deactivated(&deactivated).unwrap_err();
        assert_eq!(refused.code, "ACCOUNT_DEACTIVATED");
        assert_eq!(
            refused.details,
            vec![("scheduledFor".to_owned(), json!("2026-11-13T12:00:00.000Z"))]
        );
    }

    #[test]
    fn enforced_consent_refuses_with_403() {
        let enforced = Settings::from_pairs([("RUSTIC_ISLES_CONSENT_ENFORCE", "on")]);
        assert!(assert_consent_current_when_enforced(&account(|_| {}), &Settings::default()).is_ok());
        let refused = assert_consent_current_when_enforced(&account(|_| {}), &enforced).unwrap_err();
        assert_eq!((refused.code.as_str(), refused.status()), ("CONSENT_REQUIRED", 403));
        let ok = account(|a| a.consent = Some(consent(TERMS_VERSION, PRIVACY_VERSION)));
        assert!(assert_consent_current_when_enforced(&ok, &enforced).is_ok());
    }

    #[test]
    fn privacy_versions_in_transition_count_as_accepted() {
        let required = current_legal_requirements();
        assert!(is_accepted_privacy_version(Some(PRIVACY_VERSION), &required));
        for version in ["2026-10-16", "2026-10-15", "2026-10-14.2"] {
            assert!(is_accepted_privacy_version(Some(version), &required), "{version}");
        }
        assert!(!is_accepted_privacy_version(Some("2026-10-14"), &required));
        assert!(!is_accepted_privacy_version(None, &required));
        assert_eq!(
            (TERMS_VERSION, PRIVACY_VERSION, MINIMUM_AGE, DELETION_HOLD_DAYS),
            ("2026-10-14", "2026-10-17", 16, 30)
        );
    }

    fn rejected(name: &str) -> String {
        validate_username(Some(&json!(name))).unwrap_err().message
    }

    fn refusal(name: &str, settings: &Settings) -> Option<String> {
        assert_username_allowed(name, settings).err().map(|e| e.message)
    }

    #[test]
    fn username_format_is_letters_and_digits_three_to_sixteen_with_a_letter() {
        for name in ["Fox", "MysticFox", "Fox99", "x12", "Sixteen16Chars16"] {
            assert_eq!(validate_username(Some(&json!(name))).unwrap(), name);
        }
        assert_eq!(validate_username(Some(&json!("  MysticFox  "))).unwrap(), "MysticFox");
        for name in [
            "Mystic_Fox",
            "fox!",
            "fox-x",
            "fox x",
            "fox.x",
            "fóx",
            "fox@home",
            "<b>fox",
        ] {
            assert!(rejected(name).contains("only contain letters and numbers"), "{name}");
        }
        assert!(rejected("123").contains("at least one letter"));
        assert!(rejected("ab").contains("3\u{2013}16 characters"));
        assert!(rejected(&"A".repeat(17)).contains("3\u{2013}16 characters"));
        assert!(validate_username(None).is_err());
        assert!(validate_username(Some(&json!(5))).is_err());
    }

    #[test]
    fn staff_and_project_names_are_reserved() {
        let none = Settings::default();
        for name in [
            "Admin",
            "AdminBob",
            "TheAdmin7",
            "Moderator",
            "ModeratorX",
            "TheOwner",
            "Owner1",
            "StaffFox",
            "Support",
            "FoxSupport",
            "Official",
            "Developer",
            "GameMaster",
            "RusticIsles",
            "MysticParadox",
            "MysticFox",
            "MysticFox2",
            "PhoenixLabs",
            "Mod",
            "mod7",
            "Dev",
            "GM",
            "Root",
            "Paradox",
            "Paradox99",
            "Dauntless",
            "Player1",
            "Host",
            "Guest",
            "Null",
        ] {
            assert_eq!(
                refusal(name, &none).as_deref(),
                Some(USERNAME_RESERVED_MESSAGE),
                "{name}"
            );
        }
    }

    #[test]
    fn offensive_names_are_refused_in_digit_and_stretched_spellings() {
        let none = Settings::default();
        let (f, s, n) = (["fu", "ck"].concat(), ["sh", "it"].concat(), ["ni", "gg"].concat());
        let assembled = [format!("{f}er"), format!("Big{s}"), format!("{n}a"), format!("{n}er7")];
        for name in assembled.iter().map(String::as_str).chain([
            "Fuuuuck",
            "Shiiiit",
            "Sh1t",
            "B1tch",
            "N1gg4",
            "Pen15",
            "Wh0re",
            "S3xy",
            "Dick",
            "Dick69",
            "D1ck",
            "Cock",
            "Rape",
            "Nazi",
            "Hitler",
            "KKKfox",
            "Cum",
            "Tits",
            "Wanker",
            "Rapist",
            "Kys",
            "Killyourself",
            "Pussy",
            "Porn",
            "Faggot",
            "Retard",
            "Kike",
            "Coon",
        ]) {
            assert_eq!(
                refusal(name, &none).as_deref(),
                Some(USERNAME_NOT_ALLOWED_MESSAGE),
                "{name}"
            );
        }
    }

    #[test]
    fn ordinary_names_that_only_contain_such_letters_pass() {
        let none = Settings::default();
        for name in [
            "Peacock",
            "Hancock",
            "Grapefruit",
            "Therapist",
            "Raccoon",
            "Torpedo",
            "Sussex",
            "Essex",
            "Sextant",
            "Skyshard",
            "Swanky",
            "Nazir",
            "VanDyke",
            "Kikeo",
            "Assassin",
            "Classic",
            "Cassandra",
            "Bassist",
            "Cumulus",
            "Analyst",
            "Dickens",
            "Titan",
            "Shiitake",
            "Night",
            "Nigel",
            "Niger",
            "Homer",
            "Assault",
            "Moderna",
            "ParadoxHunter",
            "Hostage",
            "Embermane",
            "Mysticzzz",
            "Tempest7",
            "QuietFox",
            "Kestrel",
            "Brimvane",
            "R4in",
            "Fox99",
            "Glass",
            "Scrapes",
            "Drape",
            "Spice",
            "Cockpit",
        ] {
            assert_eq!(refusal(name, &none), None, "{name}");
        }
    }

    #[test]
    fn the_operators_blocklist_adds_terms() {
        let env = Settings::from_pairs([("RUSTIC_ISLES_USERNAME_BLOCKLIST", "Grief, toxic  b@d x")]);
        assert_eq!(
            refusal("GriefLord", &env).as_deref(),
            Some(USERNAME_NOT_ALLOWED_MESSAGE)
        );
        assert_eq!(refusal("ToXiCfox", &env).as_deref(), Some(USERNAME_NOT_ALLOWED_MESSAGE));
        assert_eq!(refusal("Toxic7", &env).as_deref(), Some(USERNAME_NOT_ALLOWED_MESSAGE));
        assert_eq!(refusal("Badger", &env), None);
        assert_eq!(refusal("Xenon", &env), None);
        assert_eq!(refusal("GriefLord", &Settings::default()), None);
    }

    #[test]
    fn email_verification_reads_like_node() {
        let read = |value: &str| {
            resolve_email_verification(&Settings::from_pairs([("RUSTIC_ISLES_EMAIL_VERIFICATION", value)]))
        };
        assert_eq!(
            resolve_email_verification(&Settings::default()),
            EmailVerificationPolicy {
                required: false,
                is_default: true,
                unrecognised: false
            }
        );
        assert!(read("  ").is_default);
        assert!(!read("OFF").required);
        assert!(read(" Required ").required);
        for value in ["on", "true", "maybe", "requried"] {
            let policy = read(value);
            assert!(policy.required && policy.unrecognised, "{value}");
        }
    }

    #[test]
    fn registration_approval_reads_like_node() {
        let read = |value: &str| {
            resolve_registration_approval(&Settings::from_pairs([("RUSTIC_ISLES_REGISTRATION_APPROVAL", value)]))
        };
        let default = resolve_registration_approval(&Settings::default());
        assert!(default.is_default && default.approval_status == ApprovalStatus::Approved);
        assert!(!read(" OFF ").is_default && read("off").approval_status == ApprovalStatus::Approved);
        assert_eq!(read(" Required ").approval_status, ApprovalStatus::Pending);
        let typo = read("requried");
        assert!(typo.required && typo.unrecognised.as_deref() == Some("requried"));
        let legacy = resolve_registration_approval(&Settings::from_pairs([
            ("RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE", "on"),
            ("RUSTIC_ISLES_REGISTRATION_APPROVAL", "required"),
        ]));
        assert!(legacy.legacy_warning && legacy.required);
    }

    #[test]
    fn official_play_defaults_follow_the_director_and_other_values_fail_closed() {
        let with = |pairs: &[(&str, &str)]| read_official_play(&Settings::from_pairs(pairs.iter().copied()));
        let deploy = ("DEPLOYSERVER_URL", "deployserver.invalid:8080");
        let off = OfficialPlayStatus {
            available: false,
            message: Some(OFFICIAL_PLAY_UNAVAILABLE_MESSAGE),
        };
        let on = OfficialPlayStatus {
            available: true,
            message: None,
        };
        assert_eq!(with(&[deploy]), on);
        assert_eq!(with(&[]), off);
        assert_eq!(with(&[("DEPLOYSERVER_URL", "  ")]), off);
        assert_eq!(with(&[("RUSTIC_ISLES_OFFICIAL_PLAY", "   "), deploy]), on);
        assert_eq!(with(&[("RUSTIC_ISLES_OFFICIAL_PLAY", " ON ")]), on);
        assert_eq!(with(&[deploy, ("RUSTIC_ISLES_OFFICIAL_PLAY", "Off")]), off);
        for value in ["1", "0", "yes", "true", "disabled", "onn", "of"] {
            assert_eq!(with(&[deploy, ("RUSTIC_ISLES_OFFICIAL_PLAY", value)]), off, "{value}");
        }
    }

    const OFFICIAL: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

    fn report(sha: &str, version: &str) -> LauncherBuildReport {
        LauncherBuildReport {
            version: version.into(),
            sha256: sha.into(),
        }
    }

    #[test]
    fn the_unofficial_launcher_policy_is_observe_unless_set_to_refuse() {
        let read =
            |pairs: &[(&str, &str)]| read_unofficial_launcher_policy(&Settings::from_pairs(pairs.iter().copied()));
        assert!(!read(&[]).refuse);
        assert!(read(&[("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER", " Refuse ")]).refuse);
        let typo = read(&[("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER", "refuze")]);
        assert!(!typo.refuse);
        assert_eq!(typo.warnings.len(), 1);
        assert!(read(&[]).admins_exempt);
        assert!(!read(&[("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER_TESTERS_EXEMPT", "off")]).admins_exempt);
        let test_build = "c".repeat(64);
        let hashes = format!("{}, not-a-hash  {}", test_build.to_uppercase(), "d".repeat(64));
        let policy = read(&[("RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES", &hashes)]);
        assert_eq!(
            policy.allowed_hashes.iter().cloned().collect::<Vec<_>>(),
            vec![test_build, "d".repeat(64)]
        );
        assert_eq!(policy.warnings.len(), 1);
    }

    #[test]
    fn with_refuse_only_an_unofficial_build_is_refused() {
        let test_build = "c".repeat(64);
        let self_built = "b".repeat(64);
        let refuse = read_unofficial_launcher_policy(&Settings::from_pairs([
            ("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER", "refuse"),
            ("RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES", test_build.as_str()),
        ]));
        let player = vec!["player".to_owned()];
        use LauncherBuildVerdict::*;
        assert!(unofficial_launcher_refuses(
            Unofficial,
            Some(&report(&self_built, "0.1.68")),
            &player,
            &refuse
        ));
        assert!(!unofficial_launcher_refuses(
            Official,
            Some(&report(OFFICIAL, "0.1.68")),
            &player,
            &refuse
        ));
        assert!(!unofficial_launcher_refuses(
            Outdated,
            Some(&report(&self_built, "0.1.60")),
            &player,
            &refuse
        ));
        assert!(!unofficial_launcher_refuses(Unknown, None, &player, &refuse));
        assert!(!unofficial_launcher_refuses(
            Unofficial,
            Some(&report(&test_build, "0.1.69")),
            &player,
            &refuse
        ));
        let admin = vec!["admin".to_owned()];
        assert!(!unofficial_launcher_refuses(
            Unofficial,
            Some(&report(&self_built, "0.1.68")),
            &admin,
            &refuse
        ));
        let strict = UnofficialLauncherPolicy {
            admins_exempt: false,
            ..refuse
        };
        assert!(unofficial_launcher_refuses(
            Unofficial,
            Some(&report(&self_built, "0.1.68")),
            &admin,
            &strict
        ));
        assert!(!unofficial_launcher_refuses(
            Unofficial,
            Some(&report(&self_built, "0.1.68")),
            &player,
            &read_unofficial_launcher_policy(&Settings::default())
        ));
    }

    #[test]
    fn the_play_verdict_follows_the_pinned_hash_and_the_served_version() {
        let self_built = "b".repeat(64);
        let verdict = |r: Option<LauncherBuildReport>, pinned: Option<&str>| {
            classify_launcher_build(r.as_ref(), pinned, Some("0.1.68"))
        };
        assert_eq!(
            verdict(Some(report(OFFICIAL, "0.1.68")), Some(OFFICIAL)),
            LauncherBuildVerdict::Official
        );
        assert_eq!(
            verdict(Some(report(&self_built, "0.1.67")), Some(OFFICIAL)),
            LauncherBuildVerdict::Outdated
        );
        assert_eq!(
            verdict(Some(report(&self_built, "0.1.68")), Some(OFFICIAL)),
            LauncherBuildVerdict::Unofficial
        );
        assert_eq!(
            verdict(Some(report(&self_built, "0.1.69")), Some(OFFICIAL)),
            LauncherBuildVerdict::Unofficial
        );
        assert_eq!(verdict(None, Some(OFFICIAL)), LauncherBuildVerdict::Unknown);
        assert_eq!(
            verdict(Some(report(&self_built, "0.1.68")), None),
            LauncherBuildVerdict::Unknown
        );
    }

    #[test]
    fn a_launcher_report_is_parsed_or_dropped() {
        let ok = json!({"version": "0.1.68", "sha256": "A".repeat(64)});
        assert_eq!(
            parse_launcher_build_report(Some(&ok)),
            Some(report(&"a".repeat(64), "0.1.68"))
        );
        assert!(
            parse_launcher_build_report(Some(&json!({"version": "0.1.68-rc.1", "sha256": "a".repeat(64)}))).is_some()
        );
        for bad in [
            json!({"version": "0.1", "sha256": "a".repeat(64)}),
            json!({"version": "0.1.68", "sha256": "xyz"}),
            json!([1]),
            json!("x"),
            json!({"version": 1, "sha256": "a".repeat(64)}),
        ] {
            assert_eq!(parse_launcher_build_report(Some(&bad)), None, "{bad}");
        }
        assert_eq!(parse_launcher_build_report(None), None);
    }

    #[test]
    fn launcher_version_comparison_follows_prerelease_precedence() {
        assert!(version_at_least("0.1.35", "0.1.35"));
        assert!(version_at_least("0.1.35+build.7", "0.1.35"));
        assert!(!version_at_least("0.1.35-rc.1", "0.1.35"));
        assert!(version_at_least("0.1.35-rc.2", "0.1.35-rc.1"));
        assert!(!version_at_least("0.1.35-alpha", "0.1.35-rc.1"));
        assert!(version_at_least("999999999999999999.0.0", "2.0.0"));
        assert!(!version_at_least("not-a-version", "0.1.35"));
    }
}
