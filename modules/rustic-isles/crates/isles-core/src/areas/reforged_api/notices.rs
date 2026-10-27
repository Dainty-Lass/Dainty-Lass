//! The launcher notices (`services/launcherNotices.ts`, `controllers/launcherNotices.ts`): validation of a manager
//! save, the active-window rule and the two views. The public read (`GET /launcher/v1/notices`, which the auth
//! area's launcher router answers) uses [`public_view`]; the dashboard's read and versioned save are here.

use serde_json::{Map, Value, json};

use super::admin::{Ctx, route};
use super::util::{self, date_parse, iso, now_ms};
use crate::http::Handlers;
use crate::repo::reforged::{AdminAuditRecord, LauncherNoticesRecord};
use crate::routes::{Method, Service};

pub const ANNOUNCEMENTS_MAX: usize = 3;
pub const TAGS_MAX: usize = 4;
const ANNOUNCEMENT_TEXT_MAX: usize = 280;
const TAG_TEXT_MAX: usize = 24;
const ID_MAX: usize = 64;
const REASON_MIN: usize = 8;
const REASON_MAX: usize = 200;
pub const TIME_TOKEN: &str = "{time}";

type Check<T> = Result<T, String>;

fn code_points(text: &str) -> usize {
    text.chars().count()
}

/// `ParseLauncherNoticeInstant`: a full ISO-8601 instant, normalised to UTC milliseconds; anything else is refused.
pub fn parse_instant(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    // /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d{1,9})?)?(?:Z|[+-]\d{2}:\d{2})$/
    let b = text.as_bytes();
    let digits = |from: usize, n: usize| b.get(from..from + n).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    let at = |i: usize, c: u8| b.get(i) == Some(&c);
    if !(digits(0, 4)
        && at(4, b'-')
        && digits(5, 2)
        && at(7, b'-')
        && digits(8, 2)
        && at(10, b'T')
        && digits(11, 2)
        && at(13, b':')
        && digits(14, 2))
    {
        return None;
    }
    let mut i = 16;
    if at(i, b':') {
        if !digits(i + 1, 2) {
            return None;
        }
        i += 3;
        if at(i, b'.') {
            let fraction = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if !(1..=9).contains(&fraction) {
                return None;
            }
            i += 1 + fraction;
        }
    }
    let zone_ok = match b.get(i..) {
        Some([b'Z']) => true,
        Some([sign, rest @ ..]) if (*sign == b'+' || *sign == b'-') && rest.len() == 5 => {
            rest[..2].iter().all(u8::is_ascii_digit) && rest[2] == b':' && rest[3..].iter().all(u8::is_ascii_digit)
        }
        _ => false,
    };
    if !zone_ok {
        return None;
    }
    // chrono wants the seconds that the pattern lets a minute-only instant leave out
    let full = if at(16, b':') {
        text.to_owned()
    } else {
        format!("{}:00{}", &text[..16], &text[16..])
    };
    date_parse(&full).map(iso)
}

fn is_control(c: char) -> bool {
    c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c)
}

fn check_text(raw: Option<&Value>, label: &str, max: usize, allow_time: bool) -> Check<String> {
    let Some(Value::String(raw)) = raw else {
        return Err(format!("{label} must be text."));
    };
    let text = service_kit::js::trim(raw);
    let length = code_points(text);
    if !(1..=max).contains(&length) {
        return Err(format!("{label} must be 1 to {max} characters."));
    }
    if text.chars().any(is_control) {
        return Err(format!(
            "{label} must be one line of plain text (no tabs or line breaks)."
        ));
    }
    if text.contains(['<', '>']) {
        return Err(format!("{label} must be plain text (no < or > characters)."));
    }
    if !allow_time {
        if text.contains(['{', '}']) {
            return Err(format!("{label} does not take placeholders."));
        }
        return Ok(text.to_owned());
    }
    if text.matches(TIME_TOKEN).count() > 1 {
        return Err(format!("{label} may contain {{time}} once."));
    }
    if text.replace(TIME_TOKEN, "").contains(['{', '}']) {
        return Err(format!("{label} may only use the {{time}} placeholder."));
    }
    Ok(text.to_owned())
}

fn check_id(raw: Option<&Value>, label: &str) -> Check<Option<String>> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id))
            if (1..=ID_MAX).contains(&id.len())
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
        {
            Ok(Some(id.clone()))
        }
        _ => Err(format!(
            "{label} must be 1 to {ID_MAX} letters, digits, \"-\" or \"_\"."
        )),
    }
}

fn check_tone(raw: Option<&Value>, label: &str, allowed: &[&str]) -> Check<String> {
    match raw {
        Some(Value::String(tone)) if allowed.contains(&tone.as_str()) => Ok(tone.clone()),
        _ => Err(format!("{label} must be one of {}.", allowed.join(", "))),
    }
}

fn check_instant(raw: Option<&Value>, label: &str) -> Check<Option<String>> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        some => parse_instant(some)
            .map(Some)
            .ok_or_else(|| format!("{label} must be an ISO time in UTC (for example 2026-10-16T00:00:00.000Z).")),
    }
}

fn check_window(item: &Map<String, Value>, label: &str) -> Check<(Option<String>, Option<String>)> {
    let from = check_instant(item.get("showFrom"), &format!("{label}.showFrom"))?;
    let until = check_instant(item.get("showUntil"), &format!("{label}.showUntil"))?;
    if let (Some(from), Some(until)) = (&from, &until)
        && date_parse(until) < date_parse(from)
    {
        return Err(format!("{label}.showUntil is before showFrom."));
    }
    Ok((from, until))
}

fn check_list<'a>(raw: Option<&'a Value>, label: &str, max: usize) -> Check<Vec<&'a Map<String, Value>>> {
    let Some(Value::Array(items)) = raw else {
        return Err(format!("{label} must be a list."));
    };
    if items.len() > max {
        return Err(format!("{label} holds at most {max} entries."));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            entry
                .as_object()
                .ok_or_else(|| format!("{label}[{index}] must be an object."))
        })
        .collect()
}

/// The fields in the order Node builds them: id first when given, then the rest; a made-up id comes last.
fn build(id: Option<String>, new_id: &mut dyn FnMut() -> String, mut fields: Vec<(&str, Value)>) -> Value {
    let mut out = Map::new();
    let made = id.is_none();
    if let Some(id) = id.clone() {
        out.insert("id".into(), json!(id));
    }
    for (key, value) in fields.drain(..) {
        out.insert(key.to_owned(), value);
    }
    if made {
        out.insert("id".into(), json!(new_id()));
    }
    Value::Object(out)
}

fn check_announcement(item: &Map<String, Value>, index: usize, new_id: &mut dyn FnMut() -> String) -> Check<Value> {
    let label = format!("announcements[{index}]");
    let id = check_id(item.get("id"), &format!("{label}.id"))?;
    let text = check_text(item.get("text"), &format!("{label}.text"), ANNOUNCEMENT_TEXT_MAX, true)?;
    let tone = check_tone(
        item.get("tone"),
        &format!("{label}.tone"),
        &["info", "warning", "critical"],
    )?;
    let event_at = check_instant(item.get("eventAt"), &format!("{label}.eventAt"))?;
    if text.contains(TIME_TOKEN) && event_at.is_none() {
        return Err(format!("{label}.eventAt is required when the text contains {{time}}."));
    }
    let (from, until) = check_window(item, &label)?;
    let mut fields = vec![("text", json!(text)), ("tone", json!(tone))];
    for (key, value) in [("eventAt", event_at), ("showFrom", from), ("showUntil", until)] {
        if let Some(value) = value {
            fields.push((key, json!(value)));
        }
    }
    Ok(build(id, new_id, fields))
}

fn check_tag(item: &Map<String, Value>, index: usize, new_id: &mut dyn FnMut() -> String) -> Check<Value> {
    let label = format!("tags[{index}]");
    let id = check_id(item.get("id"), &format!("{label}.id"))?;
    let text = check_text(item.get("text"), &format!("{label}.text"), TAG_TEXT_MAX, false)?;
    let tone = check_tone(
        item.get("tone"),
        &format!("{label}.tone"),
        &["info", "warning", "danger", "success"],
    )?;
    if item.get("eventAt").is_some_and(|v| !v.is_null()) {
        return Err(format!("{label} does not take eventAt."));
    }
    let (from, until) = check_window(item, &label)?;
    let mut fields = vec![("text", json!(text)), ("tone", json!(tone))];
    for (key, value) in [("showFrom", from), ("showUntil", until)] {
        if let Some(value) = value {
            fields.push((key, json!(value)));
        }
    }
    Ok(build(id, new_id, fields))
}

/// `ValidateLauncherNoticesInput`: the announcements and the tags as they will be stored.
pub fn validate_input(body: Option<&Value>, new_id: &mut dyn FnMut() -> String) -> Check<(Vec<Value>, Vec<Value>)> {
    let Some(Value::Object(source)) = body else {
        return Err("The request body must be an object.".into());
    };
    let announcement_list = check_list(source.get("announcements"), "announcements", ANNOUNCEMENTS_MAX)?;
    let tag_list = check_list(source.get("tags"), "tags", TAGS_MAX)?;
    let announcements = announcement_list
        .iter()
        .enumerate()
        .map(|(i, item)| check_announcement(item, i, new_id))
        .collect::<Check<Vec<_>>>()?;
    let tags = tag_list
        .iter()
        .enumerate()
        .map(|(i, item)| check_tag(item, i, new_id))
        .collect::<Check<Vec<_>>>()?;
    let mut seen = std::collections::HashSet::new();
    for item in announcements.iter().chain(&tags) {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        if !seen.insert(id.to_owned()) {
            return Err(format!("The id {id} is used by more than one notice or tag."));
        }
    }
    Ok((announcements, tags))
}

/// `ValidateLauncherNoticesReason`.
pub fn validate_reason(raw: Option<&Value>) -> Check<String> {
    let Some(Value::String(raw)) = raw else {
        return Err("A change reason of 8 to 200 characters is required.".into());
    };
    let reason = service_kit::js::trim(raw);
    let length = code_points(reason);
    if !(REASON_MIN..=REASON_MAX).contains(&length) {
        return Err(format!(
            "The change reason must be {REASON_MIN} to {REASON_MAX} characters."
        ));
    }
    Ok(reason.to_owned())
}

/// `ValidateLauncherNoticesExpectedVersion`.
pub fn validate_expected_version(raw: Option<&Value>) -> Check<u64> {
    match raw {
        Some(Value::Number(n)) => service_kit::js::safe_integer(Some(&Value::Number(n.clone())))
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(version_message),
        _ => Err(version_message()),
    }
}

fn version_message() -> String {
    "expectedVersion must be the version you loaded (a whole number, 0 or more).".into()
}

/// `IsLauncherNoticeActive`: inside its window at `now`.
pub fn is_active(item: &Value, now: i64) -> bool {
    let at = |key: &str| item.get(key).and_then(Value::as_str).and_then(date_parse);
    if let Some(from) = item.get("showFrom").and_then(Value::as_str)
        && !date_parse(from).is_some_and(|from| now >= from)
    {
        return false;
    }
    if let Some(until) = item.get("showUntil").and_then(Value::as_str)
        && !date_parse(until).is_some_and(|until| now < until)
    {
        return false;
    }
    let _ = at;
    true
}

fn pick(item: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for key in keys {
        if let Some(value) = item.get(*key) {
            out.insert((*key).to_owned(), value.clone());
        }
    }
    Value::Object(out)
}

/// `PublicLauncherNoticesView`: what the launcher polls, only the notices inside their window.
pub fn public_view(record: Option<&LauncherNoticesRecord>, now: i64, server_time: &str) -> Value {
    let (announcements, tags): (&[Value], &[Value]) = record.map_or((&[], &[]), |r| (&r.announcements, &r.tags));
    json!({
        "announcements": announcements.iter().filter(|i| is_active(i, now))
            .map(|i| pick(i, &["id", "text", "tone", "eventAt", "showFrom", "showUntil"])).collect::<Vec<_>>(),
        "tags": tags.iter().filter(|i| is_active(i, now))
            .map(|i| pick(i, &["id", "text", "tone", "showFrom", "showUntil"])).collect::<Vec<_>>(),
        "serverTime": server_time,
    })
}

/// `AdminLauncherNoticesView`.
pub fn admin_view(record: Option<&LauncherNoticesRecord>, server_time: &str) -> Value {
    json!({
        "version": record.map_or(0, |r| r.version),
        "updatedAt": record.and_then(|r| r.updated_at.clone()),
        "updatedBy": record.and_then(|r| r.updated_by.clone()),
        "announcements": record.map_or_else(Vec::new, |r| r.announcements.clone()),
        "tags": record.map_or_else(Vec::new, |r| r.tags.clone()),
        "serverTime": server_time,
    })
}

fn launcher_error(status: u16, code: &str, message: &str, details: Option<Value>) -> axum::response::Response {
    let mut error = Map::new();
    if let Some(Value::Object(details)) = details {
        error.extend(details);
    }
    error.insert("code".into(), json!(code));
    error.insert("message".into(), json!(message));
    error.insert("requestId".into(), json!(uuid::Uuid::new_v4().to_string()));
    util::json(status, &json!({ "error": Value::Object(error) }))
}

async fn get_admin_notices(ctx: Ctx) -> axum::response::Response {
    match ctx.store.read(|conn, repos| repos.launcher_notices.get(conn)).await {
        Ok(record) => util::json(200, &admin_view(record.as_ref(), &util::now_iso())),
        Err(error) => {
            tracing::error!("[LauncherNotices] admin read failed: {error}");
            launcher_error(
                500,
                "INTERNAL",
                "The launcher notices could not be read. Please try again.",
                None,
            )
        }
    }
}

async fn update_notices(ctx: Ctx) -> axum::response::Response {
    let caller = ctx.caller().clone();
    let body = ctx.req.body.clone();
    let invalid = |message: &str| launcher_error(400, "NOTICES_INVALID", message, None);
    let (announcements, tags) = match validate_input(body.as_ref(), &mut || uuid::Uuid::new_v4().to_string()) {
        Ok(content) => content,
        Err(message) => return invalid(&message),
    };
    let object = body.as_ref().and_then(Value::as_object);
    let reason = match validate_reason(object.and_then(|o| o.get("reason"))) {
        Ok(reason) => reason,
        Err(message) => return invalid(&message),
    };
    let expected = match validate_expected_version(object.and_then(|o| o.get("expectedVersion"))) {
        Ok(version) => version,
        Err(message) => return invalid(&message),
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let saved_at = util::now_iso();
    let next = LauncherNoticesRecord {
        version: expected + 1,
        updated_at: Some(saved_at.clone()),
        updated_by: Some(caller.user_id.clone()),
        announcements,
        tags,
    };
    let state =
        |r: &LauncherNoticesRecord| json!({ "version": r.version, "announcements": r.announcements, "tags": r.tags });
    let (ip, actor_id, rid, at, saved) = (ctx.req.ip(), caller.user_id.clone(), request_id, saved_at, next.clone());
    let actor = module_api::ActorContext::new(module_api::Actor::Admin {
        key: caller.user_id.clone(),
    });
    let outcome = ctx
        .store
        .write(&actor, move |tx, repos| {
            let before = repos.launcher_notices.get(tx.conn())?;
            let current = before.clone().unwrap_or(LauncherNoticesRecord {
                version: 0,
                updated_at: None,
                updated_by: None,
                announcements: vec![],
                tags: vec![],
            });
            if current.version != expected || !repos.launcher_notices.replace_if_version(tx, &saved, expected)? {
                return Ok(Err(current));
            }
            repos.admin.append_audit(
                tx,
                &AdminAuditRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    actor_user_id: actor_id,
                    target_user_id: None,
                    action: "launcher_notices.update".into(),
                    old_state: Some(state(&current)),
                    new_state: Some(state(&saved)),
                    reason: Some(reason),
                    ip,
                    request_id: rid,
                    created_at: at,
                },
            )?;
            Ok(Ok(saved))
        })
        .await;
    match outcome {
        Ok(Ok(saved)) => {
            tracing::info!(
                "[LauncherNotices] saved version {} by {}: {} announcement(s), {} tag(s)",
                saved.version,
                caller.user_id,
                saved.announcements.len(),
                saved.tags.len()
            );
            util::json(200, &admin_view(Some(&saved), &util::now_iso()))
        }
        Ok(Err(current)) => launcher_error(
            409,
            "NOTICES_VERSION_CONFLICT",
            "The launcher notices were saved by someone else. Reload them and apply your change again.",
            Some(json!({ "current": admin_view(Some(&current), &util::now_iso()) })),
        ),
        Err(error) => {
            tracing::error!("[LauncherNotices] save failed: {error}");
            launcher_error(
                500,
                "INTERNAL",
                "The launcher notices could not be saved. Please try again.",
                None,
            )
        }
    }
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Get, "/admin/v1/launcher-notices", |call| {
        Box::pin(route(call, true, get_admin_notices))
    });
    handlers.on(b, Method::Put, "/admin/v1/launcher-notices", |call| {
        Box::pin(route(call, true, update_notices))
    });
}

/// `GET /launcher/v1/notices` for the auth area: the cache header and the view.
pub fn public_response(record: Option<&LauncherNoticesRecord>) -> axum::response::Response {
    let mut response = util::json(200, &public_view(record, now_ms(), &util::now_iso()));
    util::cache_control(&mut response, "public, max-age=30");
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("made-{n}")
        }
    }

    fn validate(body: Value) -> Check<(Vec<Value>, Vec<Value>)> {
        validate_input(Some(&body), &mut ids())
    }

    // services/launcherNotices.test.ts
    #[test]
    fn instants_are_full_iso_or_refused() {
        let parse = |v: &str| parse_instant(Some(&json!(v)));
        assert_eq!(
            parse("2026-10-16T00:00:00.000Z").as_deref(),
            Some("2026-10-16T00:00:00.000Z")
        );
        assert_eq!(
            parse("2026-10-16T02:00+02:00").as_deref(),
            Some("2026-10-16T00:00:00.000Z")
        );
        assert_eq!(
            parse("2026-10-16T00:00:00.5Z").as_deref(),
            Some("2026-10-16T00:00:00.500Z")
        );
        for bad in [
            "2026-10-16",
            "2026-10-16T00:00",
            "2026-10-16 00:00:00Z",
            "2026-10-16T00:00:00",
            "soon",
            "2026-13-40T00:00:00Z",
            "",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(parse_instant(Some(&json!(5))), None);
        assert_eq!(parse_instant(None), None);
    }

    #[test]
    fn a_save_is_checked_field_by_field() {
        let (announcements, tags) = validate(json!({
            "announcements": [
                {"text": " Maintenance at {time} ", "tone": "warning", "eventAt": "2026-10-16T12:00:00Z", "showUntil": "2026-10-17T00:00:00Z"},
                {"id": "kept", "text": "Hello", "tone": "info"}
            ],
            "tags": [{"text": "NEW", "tone": "success", "showFrom": "2026-10-01T00:00:00Z"}]
        }))
        .unwrap();
        assert_eq!(
            announcements[0].to_string(),
            r#"{"text":"Maintenance at {time}","tone":"warning","eventAt":"2026-10-16T12:00:00.000Z","showUntil":"2026-10-17T00:00:00.000Z","id":"made-1"}"#
        );
        assert_eq!(
            announcements[1].to_string(),
            r#"{"id":"kept","text":"Hello","tone":"info"}"#
        );
        assert_eq!(
            tags[0].to_string(),
            r#"{"text":"NEW","tone":"success","showFrom":"2026-10-01T00:00:00.000Z","id":"made-2"}"#
        );
        let fails = |body: Value, message: &str| assert_eq!(validate(body.clone()).unwrap_err(), message, "{body}");
        fails(json!(null), "The request body must be an object.");
        fails(json!([]), "The request body must be an object.");
        fails(json!({"tags": []}), "announcements must be a list.");
        fails(json!({"announcements": []}), "tags must be a list.");
        fails(
            json!({"announcements": [{},{},{},{}], "tags": []}),
            "announcements holds at most 3 entries.",
        );
        fails(
            json!({"announcements": [5], "tags": []}),
            "announcements[0] must be an object.",
        );
        fails(
            json!({"announcements": [{"tone": "info"}], "tags": []}),
            "announcements[0].text must be text.",
        );
        fails(
            json!({"announcements": [{"text": "", "tone": "info"}], "tags": []}),
            "announcements[0].text must be 1 to 280 characters.",
        );
        fails(
            json!({"announcements": [{"text": "a\tb", "tone": "info"}], "tags": []}),
            "announcements[0].text must be one line of plain text (no tabs or line breaks).",
        );
        fails(
            json!({"announcements": [{"text": "<b>", "tone": "info"}], "tags": []}),
            "announcements[0].text must be plain text (no < or > characters).",
        );
        fails(
            json!({"announcements": [{"text": "{time}{time}", "tone": "info", "eventAt": "2026-10-16T12:00:00Z"}], "tags": []}),
            "announcements[0].text may contain {time} once.",
        );
        fails(
            json!({"announcements": [{"text": "{name}", "tone": "info"}], "tags": []}),
            "announcements[0].text may only use the {time} placeholder.",
        );
        fails(
            json!({"announcements": [{"text": "at {time}", "tone": "info"}], "tags": []}),
            "announcements[0].eventAt is required when the text contains {time}.",
        );
        fails(
            json!({"announcements": [{"text": "x", "tone": "loud"}], "tags": []}),
            "announcements[0].tone must be one of info, warning, critical.",
        );
        fails(
            json!({"announcements": [{"text": "x", "tone": "info", "id": "a b"}], "tags": []}),
            "announcements[0].id must be 1 to 64 letters, digits, \"-\" or \"_\".",
        );
        fails(
            json!({"announcements": [{"text": "x", "tone": "info", "showFrom": "yesterday"}], "tags": []}),
            "announcements[0].showFrom must be an ISO time in UTC (for example 2026-10-16T00:00:00.000Z).",
        );
        fails(
            json!({"announcements": [{"text": "x", "tone": "info", "showFrom": "2026-10-02T00:00:00Z", "showUntil": "2026-10-01T00:00:00Z"}], "tags": []}),
            "announcements[0].showUntil is before showFrom.",
        );
        fails(
            json!({"announcements": [], "tags": [{"text": "x{y}", "tone": "info"}]}),
            "tags[0].text does not take placeholders.",
        );
        fails(
            json!({"announcements": [], "tags": [{"text": "x", "tone": "info", "eventAt": "2026-10-16T12:00:00Z"}]}),
            "tags[0] does not take eventAt.",
        );
        fails(
            json!({"announcements": [], "tags": [{"text": "x".repeat(25), "tone": "info"}]}),
            "tags[0].text must be 1 to 24 characters.",
        );
        fails(
            json!({"announcements": [{"id": "same", "text": "x", "tone": "info"}], "tags": [{"id": "same", "text": "y", "tone": "info"}]}),
            "The id same is used by more than one notice or tag.",
        );
    }

    #[test]
    fn reasons_and_versions_are_checked() {
        assert_eq!(
            validate_reason(Some(&json!("  fixing the date  "))).unwrap(),
            "fixing the date"
        );
        assert_eq!(
            validate_reason(Some(&json!("short"))).unwrap_err(),
            "The change reason must be 8 to 200 characters."
        );
        assert_eq!(
            validate_reason(None).unwrap_err(),
            "A change reason of 8 to 200 characters is required."
        );
        assert_eq!(validate_expected_version(Some(&json!(0))), Ok(0));
        assert_eq!(validate_expected_version(Some(&json!(4.0))), Ok(4));
        for bad in [json!(-1), json!(1.5), json!("1"), json!(null)] {
            assert!(validate_expected_version(Some(&bad)).is_err());
        }
        assert!(validate_expected_version(None).is_err());
    }

    #[test]
    fn the_public_view_shows_what_is_in_its_window() {
        let record = LauncherNoticesRecord {
            version: 2,
            updated_at: Some("t".into()),
            updated_by: Some("u".into()),
            announcements: vec![
                json!({"id": "now", "text": "a", "tone": "info", "showFrom": "2026-10-01T00:00:00.000Z", "showUntil": "2026-11-01T00:00:00.000Z"}),
                json!({"id": "later", "text": "b", "tone": "info", "showFrom": "2026-12-01T00:00:00.000Z"}),
                json!({"id": "over", "text": "c", "tone": "info", "showUntil": "2026-10-10T00:00:00.000Z"}),
            ],
            tags: vec![json!({"id": "t", "text": "NEW", "tone": "success"})],
        };
        let now = date_parse("2026-10-20T00:00:00Z").unwrap();
        let view = public_view(Some(&record), now, "2026-10-20T00:00:00.000Z");
        assert_eq!(view["announcements"].as_array().map(Vec::len), Some(1));
        assert_eq!(view["announcements"][0]["id"], "now");
        assert_eq!(view["tags"][0]["text"], "NEW");
        assert_eq!(
            view.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["announcements", "tags", "serverTime"]
        );
        assert_eq!(
            public_view(None, now, "x"),
            json!({"announcements": [], "tags": [], "serverTime": "x"})
        );
        let admin = admin_view(None, "x");
        assert_eq!(
            admin,
            json!({"version": 0, "updatedAt": null, "updatedBy": null, "announcements": [], "tags": [], "serverTime": "x"})
        );
        assert_eq!(admin_view(Some(&record), "x")["updatedBy"], "u");
    }
}
