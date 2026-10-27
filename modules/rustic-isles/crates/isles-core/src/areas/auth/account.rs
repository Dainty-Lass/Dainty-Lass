//! The launcher's account routes beside sign-in: consent to the Terms of Use and Privacy Policy and the account
//! deletion hold (`controllers/launcherAccount.ts`), the signed-in policy (`controllers/launcherPolicy.ts`), the public
//! status, legal and notices reads (`routes/launcherAuth.ts`, `controllers/launcherNotices.ts`).

use serde_json::{Map, Value, json};

use super::data::{account_write, launcher_by_user};
use super::errors::LauncherApiError;
use super::policy::{
    DAY_MS, DELETION_HOLD_DAYS, LegalRequirements, current_legal_requirements, is_accepted_privacy_version,
    is_consent_current, read_official_play,
};
use super::util::{iso_from_ms, iso_now, now_ms, parse_ms};
use crate::http::Shared;
use crate::repo::auth::{LauncherAccountRecord, LauncherConsent, LauncherDeletion};
use crate::repo::reforged::{LauncherNoticeTag, LauncherNoticesRecord, NoticeTone, TagTone};

fn legal_json(required: &LegalRequirements) -> Value {
    json!({
        "termsVersion": required.terms_version,
        "privacyVersion": required.privacy_version,
        "minimumAge": required.minimum_age,
    })
}

/// `CurrentLegalRequirements()` as `GET /launcher/v1/legal` answers it.
pub fn legal_requirements() -> Value {
    legal_json(&current_legal_requirements())
}

async fn require_account(shared: &Shared, user_id: &str) -> Result<LauncherAccountRecord, LauncherApiError> {
    launcher_by_user(shared, user_id)
        .await?
        .ok_or_else(|| LauncherApiError::new("NOT_FOUND", "Account not found."))
}

fn consent_state(consent: Option<&LauncherConsent>) -> Value {
    let required = current_legal_requirements();
    let accepted = consent.map_or(Value::Null, |consent| {
        json!({
            "termsVersion": consent.terms_version,
            "privacyVersion": consent.privacy_version,
            "ageConfirmed": true,
            "acceptedAt": consent.accepted_at,
        })
    });
    json!({
        "required": legal_json(&required),
        "accepted": accepted,
        "upToDate": is_consent_current(consent, &required),
    })
}

/// `GetConsentState`.
pub async fn get_consent_state(shared: &Shared, user_id: &str) -> Result<Value, LauncherApiError> {
    Ok(consent_state(require_account(shared, user_id).await?.consent.as_ref()))
}

/// `AcceptConsent`: records the player's acceptance of the documents the launcher shows. The age confirmation must be
/// true; the versions must be the current ones (a launcher with older texts gets LEGAL_VERSION_OUTDATED).
pub async fn accept_consent(
    shared: &Shared,
    user_id: &str,
    body: Option<&Value>,
    now: i64,
) -> Result<Value, LauncherApiError> {
    let empty = Map::new();
    let fields = body.and_then(Value::as_object).unwrap_or(&empty);
    let required = current_legal_requirements();
    if fields.get("ageConfirmed") != Some(&Value::Bool(true)) {
        return Err(LauncherApiError::new(
            "CONSENT_AGE_REQUIRED",
            format!(
                "You must be at least {} years old to play Mystic Paradox.",
                required.minimum_age
            ),
        ));
    }
    let (Some(terms), Some(privacy)) = (
        fields.get("termsVersion").and_then(Value::as_str),
        fields.get("privacyVersion").and_then(Value::as_str),
    ) else {
        return Err(LauncherApiError::new(
            "AUTH_VALIDATION_FAILED",
            "termsVersion and privacyVersion are required.",
        ));
    };
    if terms != required.terms_version || !is_accepted_privacy_version(Some(privacy), &required) {
        return Err(LauncherApiError::new(
            "LEGAL_VERSION_OUTDATED",
            "These Terms of Use or this Privacy Policy have changed. Update the launcher, read them again and accept.",
        )
        .with_detail("termsVersion", json!(required.terms_version))
        .with_detail("privacyVersion", json!(required.privacy_version)));
    }
    let account = require_account(shared, user_id).await?;
    let consent = LauncherConsent {
        terms_version: required.terms_version.clone(),
        // The version the player was shown (the current one, or one still accepted during its transition)
        privacy_version: privacy.to_owned(),
        accepted_at: iso_from_ms(now),
    };
    let (id, record) = (account.account.clone(), consent.clone());
    account_write(shared, &account.account, move |tx, repos| {
        repos.launcher_accounts.set_consent(tx, &id, &record)
    })
    .await?;
    Ok(consent_state(Some(&consent)))
}

fn deletion_state(request: Option<&LauncherDeletion>) -> Value {
    let request = request.map_or(
        Value::Null,
        |request| json!({ "requestedAt": request.requested_at, "scheduledFor": request.scheduled_for }),
    );
    json!({ "request": request, "holdDays": DELETION_HOLD_DAYS })
}

/// `GetDeletionState`.
pub async fn get_deletion_state(shared: &Shared, user_id: &str) -> Result<Value, LauncherApiError> {
    Ok(deletion_state(
        require_account(shared, user_id).await?.deletion.as_ref(),
    ))
}

/// `RequestAccountDeletion`: asks for the account to be deleted after the hold. The player types the username to
/// confirm (case-insensitive). The account is deactivated at once: it cannot play, host or join, but it can sign in and
/// restore it. Asking again changes nothing.
pub async fn request_account_deletion(
    shared: &Shared,
    user_id: &str,
    body: Option<&Value>,
    now: i64,
) -> Result<Value, LauncherApiError> {
    let account = require_account(shared, user_id).await?;
    let typed = body
        .and_then(|b| b.get("confirmUsername"))
        .and_then(Value::as_str)
        .map(service_kit::js::trim)
        .unwrap_or_default();
    if typed.is_empty() || typed.to_lowercase() != account.display_name.to_lowercase() {
        return Err(LauncherApiError::new(
            "DELETION_CONFIRMATION_MISMATCH",
            "Type your username exactly as it is shown to confirm the deletion.",
        ));
    }
    if account.deletion.is_some() {
        return Ok(deletion_state(account.deletion.as_ref()));
    }
    let requested_at = iso_from_ms(now);
    let deletion = LauncherDeletion {
        requested_at: requested_at.clone(),
        scheduled_for: iso_from_ms(parse_ms(&requested_at).unwrap_or(now) + DELETION_HOLD_DAYS * DAY_MS),
    };
    let (id, record) = (account.account.clone(), deletion.clone());
    account_write(shared, &account.account, move |tx, repos| {
        repos.launcher_accounts.set_deletion(tx, &id, Some(&record))
    })
    .await?;
    Ok(deletion_state(Some(&deletion)))
}

/// `CancelAccountDeletion`: restores a deactivated account while its hold lasts. Restoring an account with no request
/// changes nothing.
pub async fn cancel_account_deletion(shared: &Shared, user_id: &str, now: i64) -> Result<Value, LauncherApiError> {
    let account = require_account(shared, user_id).await?;
    let Some(deletion) = &account.deletion else {
        return Ok(deletion_state(None));
    };
    if parse_ms(&deletion.scheduled_for).is_some_and(|scheduled| scheduled <= now) {
        return Err(LauncherApiError::new(
            "DELETION_HOLD_EXPIRED",
            format!("The {DELETION_HOLD_DAYS}-day hold has ended, so this account can no longer be restored."),
        ));
    }
    let id = account.account.clone();
    account_write(shared, &account.account, move |tx, repos| {
        repos.launcher_accounts.set_deletion(tx, &id, None)
    })
    .await?;
    Ok(deletion_state(None))
}

/// `GetLauncherPolicy`.
pub async fn get_launcher_policy(shared: &Shared, user_id: &str) -> Result<Value, LauncherApiError> {
    let account = require_account(shared, user_id).await?;
    // The tester role unlocks the log upload only: a tester gets the same stable channel and production diagnostics as
    // everyone. The development profile's verbose diagnostics are for admin accounts.
    let operations = shared.auth.operations.policy().await;
    let development = operations.diagnostics_profile == "DEVELOPMENT" && account.roles.iter().any(|r| r == "admin");
    let version_base = account
        .roles_updated_at
        .clone()
        .unwrap_or_else(|| account.created_at.clone());
    let mut policy = Map::new();
    policy.insert(
        "policyVersion".into(),
        json!(format!("{version_base}:ops-{}", operations.version)),
    );
    policy.insert("roles".into(), json!(account.roles));
    policy.insert("channel".into(), json!("stable"));
    policy.insert(
        "managedFeatureIds".into(),
        if development {
            json!(["diagnostics.verbose"])
        } else {
            json!([])
        },
    );
    policy.insert("logUpload".into(), json!({ "auto": development }));
    policy.insert("guardEnforcement".into(), json!(operations.guard_enforcement));
    policy.insert(
        "diagnosticsProfile".into(),
        json!(if development { "DEVELOPMENT" } else { "PRODUCTION" }),
    );
    // The fields of an optional module (rustic-rifts): `p2pEmergencyStop`, `coopHunts`
    for (name, text) in shared
        .rifts
        .realms()
        .launcher_policy_fields()
        .map_err(super::launcher::seam_error)?
    {
        let value = serde_json::from_str(&text.0).map_err(|_| super::errors::internal())?;
        policy.insert(name, value);
    }
    Ok(Value::Object(policy))
}

/// `GET /launcher/v1/status`.
pub async fn launcher_status(shared: &Shared) -> Result<Value, LauncherApiError> {
    let online = match &shared.store {
        Some(store) => store
            .read(|conn, _| {
                use diesel::RunQueryDsl;
                diesel::sql_query("SELECT 1").execute(conn)?;
                Ok(())
            })
            .await
            .is_ok(),
        None => false,
    };
    let official = read_official_play(&shared.settings);
    let mut official_play = Map::new();
    official_play.insert("available".into(), json!(official.available));
    if let Some(message) = official.message {
        official_play.insert("message".into(), json!(message));
    }
    Ok(json!({
        "online": online,
        "supportedBuildChangelist": shared.version.tables.target_changelist,
        "officialPlay": official_play,
    }))
}

// ------------------------------------------------------------------ notices

fn notice_active(show_from: Option<&str>, show_until: Option<&str>, now: i64) -> bool {
    if show_from.is_some_and(|from| parse_ms(from).is_none_or(|from| now < from)) {
        return false;
    }
    if show_until.is_some_and(|until| parse_ms(until).is_none_or(|until| now >= until)) {
        return false;
    }
    true
}

fn tone_text(tone: NoticeTone) -> &'static str {
    match tone {
        NoticeTone::Info => "info",
        NoticeTone::Warning => "warning",
        NoticeTone::Critical => "critical",
    }
}

fn tag_tone_text(tone: TagTone) -> &'static str {
    match tone {
        TagTone::Info => "info",
        TagTone::Warning => "warning",
        TagTone::Danger => "danger",
        TagTone::Success => "success",
    }
}

fn with_window(object: &mut Map<String, Value>, from: &Option<String>, until: &Option<String>) {
    if let Some(from) = from {
        object.insert("showFrom".into(), json!(from));
    }
    if let Some(until) = until {
        object.insert("showUntil".into(), json!(until));
    }
}

/// `PublicLauncherNoticesView`: the items active now.
pub fn public_notices_view(record: Option<&LauncherNoticesRecord>, now: i64, server_time: &str) -> Value {
    let mut announcements = Vec::new();
    let mut tags = Vec::new();
    if let Some(record) = record {
        for item in &record.announcements {
            if !notice_active(item.show_from.as_deref(), item.show_until.as_deref(), now) {
                continue;
            }
            let mut object = Map::new();
            object.insert("id".into(), json!(item.id));
            object.insert("text".into(), json!(item.text));
            object.insert("tone".into(), json!(tone_text(item.tone)));
            if let Some(event_at) = &item.event_at {
                object.insert("eventAt".into(), json!(event_at));
            }
            with_window(&mut object, &item.show_from, &item.show_until);
            announcements.push(Value::Object(object));
        }
        for item in &record.tags {
            let LauncherNoticeTag {
                id,
                text,
                tone,
                show_from,
                show_until,
            } = item;
            if !notice_active(show_from.as_deref(), show_until.as_deref(), now) {
                continue;
            }
            let mut object = Map::new();
            object.insert("id".into(), json!(id));
            object.insert("text".into(), json!(text));
            object.insert("tone".into(), json!(tag_tone_text(*tone)));
            with_window(&mut object, show_from, show_until);
            tags.push(Value::Object(object));
        }
    }
    json!({ "announcements": announcements, "tags": tags, "serverTime": server_time })
}

/// `GetPublicLauncherNotices`: the items active now. A store read error answers the empty set, never an error the
/// launcher must show; one log line per streak of failed reads, so a broken store does not flood the log every poll.
pub async fn public_launcher_notices(shared: &Shared) -> Value {
    use std::sync::atomic::Ordering;
    let read = match &shared.store {
        Some(store) => store.read(|conn, repos| repos.launcher_notices.get(conn)).await,
        None => Err(crate::repo::RepoError::Data("no database is configured".into())),
    };
    let record = match read {
        Ok(record) => {
            shared.auth.notices_failure_logged.store(false, Ordering::Relaxed);
            record
        }
        Err(error) => {
            if !shared.auth.notices_failure_logged.swap(true, Ordering::Relaxed) {
                tracing::error!("[LauncherNotices] could not read the notices, answering none: {error}");
            }
            None
        }
    };
    public_notices_view(record.as_ref(), now_ms(), &iso_now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::reforged::LauncherNotice;

    #[test]
    fn the_public_view_keeps_only_the_items_active_now() {
        let record = LauncherNoticesRecord {
            version: 3,
            updated_at: None,
            updated_by: None,
            announcements: vec![
                LauncherNotice {
                    id: "a".into(),
                    text: "Maintenance at {time}".into(),
                    tone: NoticeTone::Warning,
                    event_at: Some("2026-10-20T10:00:00.000Z".into()),
                    show_from: None,
                    show_until: Some("2026-10-21T00:00:00.000Z".into()),
                },
                LauncherNotice {
                    id: "later".into(),
                    text: "Not yet".into(),
                    tone: NoticeTone::Info,
                    event_at: None,
                    show_from: Some("2026-12-01T00:00:00.000Z".into()),
                    show_until: None,
                },
            ],
            tags: vec![LauncherNoticeTag {
                id: "t".into(),
                text: "Beta".into(),
                tone: TagTone::Success,
                show_from: None,
                show_until: None,
            }],
        };
        let now = parse_ms("2026-10-20T00:00:00.000Z").unwrap();
        let view = public_notices_view(Some(&record), now, "2026-10-20T00:00:00.000Z");
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            concat!(
                r#"{"announcements":[{"id":"a","text":"Maintenance at {time}","tone":"warning","#,
                r#""eventAt":"2026-10-20T10:00:00.000Z","showUntil":"2026-10-21T00:00:00.000Z"}],"#,
                r#""tags":[{"id":"t","text":"Beta","tone":"success"}],"serverTime":"2026-10-20T00:00:00.000Z"}"#
            )
        );
        let empty = public_notices_view(None, now, "x");
        assert_eq!(
            serde_json::to_string(&empty).unwrap(),
            r#"{"announcements":[],"tags":[],"serverTime":"x"}"#
        );
    }

    #[test]
    fn legal_requirements_answer_the_current_versions() {
        assert_eq!(
            serde_json::to_string(&legal_requirements()).unwrap(),
            r#"{"termsVersion":"2026-10-14","privacyVersion":"2026-10-17","minimumAge":16}"#
        );
    }
}
