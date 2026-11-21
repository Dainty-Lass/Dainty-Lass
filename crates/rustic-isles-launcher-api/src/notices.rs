// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Launcher notices (public spec §8): the active window, the public and admin views, the save's validation and
//! the two admin routes.

use std::sync::LazyLock;

use axum::response::Response;
use regex::Regex;
use rustic_isles_store::repo::reforged::{AdminAuditRecord, LauncherNoticesRecord};
use serde_json::{Map, Value};

use crate::admin::{AdminContext, admin_actor, fresh_id};
use crate::util::{self, iso, now_ms, object};

static INSTANT: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}(:[0-9]{2}(\.[0-9]{1,9})?)?(Z|[+-][0-9]{2}:[0-9]{2})$")
        .ok()
});

/// A JSON string in the instant shape that `Date.parse` reads; its ISO text.
pub fn parse_instant(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    let re = INSTANT.as_ref()?;
    let caps = re.captures(text)?;
    let with_seconds = if caps.get(1).is_some() {
        text.to_owned()
    } else {
        let zone = caps.get(3)?.as_str();
        format!("{}:00{zone}", &text[..text.len() - zone.len()])
    };
    dainty_service_kit::js::date_parse(&with_seconds).map(iso)
}

fn bound_ok(value: Option<&Value>, now_ms: i64, after: bool) -> bool {
    match value {
        Some(Value::String(text)) => match dainty_service_kit::js::date_parse(text) {
            Some(at) if after => now_ms >= at,
            Some(at) => now_ms < at,
            None => false,
        },
        _ => true,
    }
}

/// Whether an entry shows at `now`.
pub fn is_active(entry: &Value, now_ms: i64) -> bool {
    bound_ok(entry.get("showFrom"), now_ms, true) && bound_ok(entry.get("showUntil"), now_ms, false)
}

fn keep(entry: &Value, keys: &[&str]) -> Value {
    let mut map = Map::new();
    for key in keys {
        if let Some(value) = entry.get(*key) {
            map.insert((*key).to_owned(), value.clone());
        }
    }
    Value::Object(map)
}

/// The launcher's view: active entries only.
pub fn public_view(record: Option<&LauncherNoticesRecord>, now_ms: i64) -> Value {
    let pick = |list: Option<&Vec<Value>>, keys: &[&str]| -> Value {
        Value::Array(
            list.map(|l| {
                l.iter()
                    .filter(|e| is_active(e, now_ms))
                    .map(|e| keep(e, keys))
                    .collect()
            })
            .unwrap_or_default(),
        )
    };
    object(vec![
        (
            "announcements",
            pick(
                record.map(|r| &r.announcements),
                &["id", "text", "tone", "eventAt", "showFrom", "showUntil"],
            ),
        ),
        (
            "tags",
            pick(
                record.map(|r| &r.tags),
                &["id", "text", "tone", "showFrom", "showUntil"],
            ),
        ),
        ("serverTime", Value::from(iso(now_ms))),
    ])
}

/// The launcher notices as the dashboard edits them.
pub fn admin_view(record: Option<&LauncherNoticesRecord>, now_ms: i64) -> Value {
    let opt = |v: Option<&String>| v.cloned().map_or(Value::Null, Value::from);
    object(vec![
        ("version", Value::from(record.map_or(0, |r| r.version))),
        ("updatedAt", opt(record.and_then(|r| r.updated_at.as_ref()))),
        ("updatedBy", opt(record.and_then(|r| r.updated_by.as_ref()))),
        (
            "announcements",
            Value::Array(record.map(|r| r.announcements.clone()).unwrap_or_default()),
        ),
        ("tags", Value::Array(record.map(|r| r.tags.clone()).unwrap_or_default())),
        ("serverTime", Value::from(iso(now_ms))),
    ])
}

fn is_entry_id(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn count(text: &str) -> usize {
    text.chars().count()
}

struct Entry {
    id: Option<String>,
    fields: Vec<(&'static str, Value)>,
}

fn time_field(map: &Map<String, Value>, name: &str, label: &str) -> Result<Option<String>, String> {
    match map.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => parse_instant(value).map(Some).ok_or_else(|| {
            format!("{label}.{name} must be an ISO time in UTC (for example 2026-10-16T00:00:00.000Z).")
        }),
    }
}

fn text_field(map: &Map<String, Value>, label: &str, max: usize) -> Result<String, String> {
    let Some(text) = map.get("text").and_then(Value::as_str) else {
        return Err(format!("{label}.text must be text."));
    };
    let text = util::trim(text);
    if !(1..=max).contains(&count(text)) {
        return Err(format!("{label}.text must be 1 to {max} characters."));
    }
    if text.chars().any(util::is_control) {
        return Err(format!(
            "{label}.text must be one line of plain text (no tabs or line breaks)."
        ));
    }
    if text.contains(['<', '>']) {
        return Err(format!("{label}.text must be plain text (no < or > characters)."));
    }
    Ok(text.to_owned())
}

fn entry_id(map: &Map<String, Value>, label: &str) -> Result<Option<String>, String> {
    match map.get("id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) if is_entry_id(id) => Ok(Some(id.clone())),
        Some(_) => Err(format!("{label}.id must be 1 to 64 letters, digits, \"-\" or \"_\".")),
    }
}

fn window(map: &Map<String, Value>, label: &str) -> Result<(Option<String>, Option<String>), String> {
    let from = time_field(map, "showFrom", label)?;
    let until = time_field(map, "showUntil", label)?;
    if let (Some(f), Some(u)) = (&from, &until)
        && dainty_service_kit::js::date_parse(u) < dainty_service_kit::js::date_parse(f)
    {
        return Err(format!("{label}.showUntil is before showFrom."));
    }
    Ok((from, until))
}

fn announcement(map: &Map<String, Value>, label: &str) -> Result<Entry, String> {
    let id = entry_id(map, label)?;
    let text = text_field(map, label, 280)?;
    if text.matches("{time}").count() > 1 {
        return Err(format!("{label}.text may contain {{time}} once."));
    }
    if text.replace("{time}", "").contains(['{', '}']) {
        return Err(format!("{label}.text may only use the {{time}} placeholder."));
    }
    let tone = map.get("tone").and_then(Value::as_str);
    let Some(tone) = tone.filter(|t| matches!(*t, "info" | "warning" | "critical")) else {
        return Err(format!("{label}.tone must be one of info, warning, critical."));
    };
    let event = time_field(map, "eventAt", label)?;
    if text.contains("{time}") && event.is_none() {
        return Err(format!("{label}.eventAt is required when the text contains {{time}}."));
    }
    let (from, until) = window(map, label)?;
    let mut fields = vec![("text", Value::from(text)), ("tone", Value::from(tone))];
    for (name, value) in [("eventAt", event), ("showFrom", from), ("showUntil", until)] {
        if let Some(value) = value {
            fields.push((name, Value::from(value)));
        }
    }
    Ok(Entry { id, fields })
}

fn tag(map: &Map<String, Value>, label: &str) -> Result<Entry, String> {
    let id = entry_id(map, label)?;
    let text = text_field(map, label, 24)?;
    if text.contains(['{', '}']) {
        return Err(format!("{label}.text does not take placeholders."));
    }
    let tone = map.get("tone").and_then(Value::as_str);
    let Some(tone) = tone.filter(|t| matches!(*t, "info" | "warning" | "danger" | "success")) else {
        return Err(format!("{label}.tone must be one of info, warning, danger, success."));
    };
    if map.get("eventAt").is_some_and(|v| !v.is_null()) {
        return Err(format!("{label} does not take eventAt."));
    }
    let (from, until) = window(map, label)?;
    let mut fields = vec![("text", Value::from(text)), ("tone", Value::from(tone))];
    for (name, value) in [("showFrom", from), ("showUntil", until)] {
        if let Some(value) = value {
            fields.push((name, Value::from(value)));
        }
    }
    Ok(Entry { id, fields })
}

fn stored(entry: Entry, new_id: &mut dyn FnMut() -> String) -> (String, Value) {
    let mut map = Map::new();
    if let Some(id) = &entry.id {
        map.insert("id".into(), Value::from(id.clone()));
    }
    for (name, value) in entry.fields {
        map.insert(name.into(), value);
    }
    let id = match entry.id {
        Some(id) => id,
        None => {
            let id = new_id();
            map.insert("id".into(), Value::from(id.clone()));
            id
        }
    };
    (id, Value::Object(map))
}

/// The validated lists, or the first failure's message; `new_id` draws the generated ids in order.
pub fn validate_input(
    body: Option<&Value>,
    new_id: &mut dyn FnMut() -> String,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let Some(Value::Object(map)) = body else {
        return Err("The request body must be an object.".into());
    };
    let Some(announcements) = map.get("announcements").and_then(Value::as_array) else {
        return Err("announcements must be a list.".into());
    };
    if announcements.len() > 3 {
        return Err("announcements holds at most 3 entries.".into());
    }
    let Some(tags) = map.get("tags").and_then(Value::as_array) else {
        return Err("tags must be a list.".into());
    };
    if tags.len() > 4 {
        return Err("tags holds at most 4 entries.".into());
    }
    for (name, list) in [("announcements", announcements), ("tags", tags)] {
        for (i, entry) in list.iter().enumerate() {
            if !entry.is_object() {
                return Err(format!("{name}[{i}] must be an object."));
            }
        }
    }
    let mut parsed_a = Vec::new();
    for (i, entry) in announcements.iter().enumerate() {
        let map = entry.as_object().ok_or("announcements must be a list.")?;
        parsed_a.push(announcement(map, &format!("announcements[{i}]"))?);
    }
    let mut parsed_t = Vec::new();
    for (i, entry) in tags.iter().enumerate() {
        let map = entry.as_object().ok_or("tags must be a list.")?;
        parsed_t.push(tag(map, &format!("tags[{i}]"))?);
    }
    let mut ids: Vec<String> = Vec::new();
    let mut out_a = Vec::new();
    let mut out_t = Vec::new();
    for entry in parsed_a {
        let (id, value) = stored(entry, new_id);
        ids.push(id);
        out_a.push(value);
    }
    for entry in parsed_t {
        let (id, value) = stored(entry, new_id);
        ids.push(id);
        out_t.push(value);
    }
    for (i, id) in ids.iter().enumerate() {
        if ids[..i].contains(id) {
            return Err(format!("The id {id} is used by more than one notice or tag."));
        }
    }
    Ok((out_a, out_t))
}

/// The change reason, JS-trimmed, 8 to 200 code points.
pub fn validate_reason(value: Option<&Value>) -> Result<String, String> {
    let Some(text) = value.and_then(Value::as_str) else {
        return Err("A change reason of 8 to 200 characters is required.".into());
    };
    let text = util::trim(text);
    if !(8..=200).contains(&count(text)) {
        return Err("The change reason must be 8 to 200 characters.".into());
    }
    Ok(text.to_owned())
}

/// A JSON number that is a safe integer, 0 or more.
pub fn validate_expected_version(value: Option<&Value>) -> Result<u64, String> {
    match value {
        Some(Value::Number(_)) => dainty_service_kit::js::safe_integer(value)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or_else(|| "expectedVersion must be the version you loaded (a whole number, 0 or more).".into()),
        _ => Err("expectedVersion must be the version you loaded (a whole number, 0 or more).".into()),
    }
}

// ------------------------------------------------------------------ routes (§8.3)

/// `GET /admin/v1/launcher-notices`: the notices for editing.
pub async fn admin_read(ctx: AdminContext) -> Response {
    match ctx.store.read(|conn, repos| repos.launcher_notices.get(conn)).await {
        Ok(record) => util::ok(&admin_view(record.as_ref(), now_ms())),
        Err(e) => {
            tracing::error!("[LauncherNotices] admin read failed: {e}");
            util::launcher_error("INTERNAL", "The launcher notices could not be read. Please try again.")
        }
    }
}

enum Saved {
    Done(LauncherNoticesRecord),
    Conflict(Option<LauncherNoticesRecord>),
}

/// `PUT /admin/v1/launcher-notices`: saves the notices, refusing a stale version.
pub async fn admin_save(ctx: AdminContext) -> Response {
    let body = ctx.request.body.as_ref();
    let mut ids = || util::random_uuid();
    let checked = validate_input(body, &mut ids).and_then(|lists| {
        let reason = validate_reason(util::field(body, "reason"))?;
        let expected = validate_expected_version(util::field(body, "expectedVersion"))?;
        Ok((lists, reason, expected))
    });
    let ((announcements, tags), reason, expected) = match checked {
        Ok(ok) => ok,
        Err(message) => return util::launcher_error("NOTICES_INVALID", &message),
    };
    let Some(caller) = ctx.caller.clone() else {
        return crate::admin::admin_internal("the notices save had no caller");
    };
    let now = now_ms();
    let at = iso(now);
    let next = LauncherNoticesRecord {
        version: expected + 1,
        updated_at: Some(at.clone()),
        updated_by: Some(caller.user_id.clone()),
        announcements,
        tags,
    };
    let ip = ctx.request.client_ip.clone();
    let operator = caller.user_id.clone();
    let saved = ctx
        .store
        .write(&admin_actor(&caller), move |tx, repos| {
            let current = repos.launcher_notices.get(tx.conn())?;
            let version = current.as_ref().map_or(0, |r| r.version);
            if version != expected || !repos.launcher_notices.replace_if_version(tx, &next, expected)? {
                return Ok(Saved::Conflict(current));
            }
            let state = |r: Option<&LauncherNoticesRecord>| {
                object(vec![
                    ("version", Value::from(r.map_or(0, |r| r.version))),
                    (
                        "announcements",
                        Value::Array(r.map(|r| r.announcements.clone()).unwrap_or_default()),
                    ),
                    ("tags", Value::Array(r.map(|r| r.tags.clone()).unwrap_or_default())),
                ])
            };
            repos.admin.append_audit(
                tx,
                &AdminAuditRecord {
                    id: fresh_id(),
                    actor_user_id: operator,
                    target_user_id: None,
                    action: "launcher_notices.update".to_owned(),
                    old_state: Some(state(current.as_ref())),
                    new_state: Some(state(Some(&next))),
                    reason: Some(reason),
                    ip,
                    request_id: fresh_id(),
                    created_at: at,
                },
            )?;
            Ok(Saved::Done(next))
        })
        .await;
    match saved {
        Ok(Saved::Done(record)) => {
            tracing::info!(
                "[LauncherNotices] saved version {} by {}: {} announcement(s), {} tag(s)",
                record.version,
                caller.user_id,
                record.announcements.len(),
                record.tags.len()
            );
            util::ok(&admin_view(Some(&record), now))
        }
        Ok(Saved::Conflict(current)) => rustic_isles_auth::LauncherError::new(
            "NOTICES_VERSION_CONFLICT",
            "The launcher notices were saved by someone else. Reload them and apply your change again.",
        )
        .with_detail("current", admin_view(current.as_ref(), now))
        .response(&util::random_uuid()),
        Err(e) => {
            tracing::error!("[LauncherNotices] save failed: {e}");
            util::launcher_error("INTERNAL", "The launcher notices could not be saved. Please try again.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check(body: Value) -> String {
        let mut n = 0;
        let mut ids = || {
            n += 1;
            format!("gen-{n}")
        };
        validate_input(Some(&body), &mut ids).unwrap_err()
    }

    fn one(entry: Value) -> String {
        check(json!({"announcements":[entry],"tags":[]}))
    }

    #[test]
    fn instants() {
        assert_eq!(
            parse_instant(&json!("2026-10-16T02:00+02:00")).unwrap(),
            "2026-10-16T00:00:00.000Z"
        );
        assert_eq!(
            parse_instant(&json!("2026-10-16T00:00:00.5Z")).unwrap(),
            "2026-10-16T00:00:00.500Z"
        );
        for bad in [
            "2026-10-16",
            "2026-10-16T00:00",
            "2026-10-16 00:00:00Z",
            "2026-10-16T00:00:00",
            "2026-13-40T00:00:00Z",
        ] {
            assert_eq!(parse_instant(&json!(bad)), None, "{bad}");
        }
    }

    #[test]
    fn validation_messages() {
        assert_eq!(check(json!(null)), "The request body must be an object.");
        assert_eq!(check(json!([])), "The request body must be an object.");
        assert_eq!(check(json!({})), "announcements must be a list.");
        assert_eq!(check(json!({"announcements":[]})), "tags must be a list.");
        assert_eq!(
            check(json!({"announcements":[{},{},{},{}],"tags":[]})),
            "announcements holds at most 3 entries."
        );
        assert_eq!(
            check(json!({"announcements":[1],"tags":[]})),
            "announcements[0] must be an object."
        );
        assert_eq!(one(json!({"tone":"info"})), "announcements[0].text must be text.");
        assert_eq!(
            one(json!({"text":" ","tone":"info"})),
            "announcements[0].text must be 1 to 280 characters."
        );
        assert_eq!(
            one(json!({"text":"a\tb","tone":"info"})),
            "announcements[0].text must be one line of plain text (no tabs or line breaks)."
        );
        assert_eq!(
            one(json!({"text":"<b>","tone":"info"})),
            "announcements[0].text must be plain text (no < or > characters)."
        );
        assert_eq!(
            one(json!({"text":"{time}{time}","tone":"info","eventAt":"2026-10-16T00:00:00Z"})),
            "announcements[0].text may contain {time} once."
        );
        assert_eq!(
            one(json!({"text":"{name}","tone":"info"})),
            "announcements[0].text may only use the {time} placeholder."
        );
        assert_eq!(
            one(json!({"text":"at {time}","tone":"info"})),
            "announcements[0].eventAt is required when the text contains {time}."
        );
        assert_eq!(
            one(json!({"text":"x","tone":"loud"})),
            "announcements[0].tone must be one of info, warning, critical."
        );
        assert_eq!(
            one(json!({"id":"a b","text":"x","tone":"info"})),
            "announcements[0].id must be 1 to 64 letters, digits, \"-\" or \"_\"."
        );
        assert_eq!(
            one(json!({"text":"x","tone":"info","showFrom":"yesterday"})),
            "announcements[0].showFrom must be an ISO time in UTC (for example 2026-10-16T00:00:00.000Z)."
        );
        assert_eq!(
            one(json!({"text":"x","tone":"info","showFrom":"2026-10-17T00:00:00Z","showUntil":"2026-10-16T00:00:00Z"})),
            "announcements[0].showUntil is before showFrom."
        );
        let t = |entry: Value| check(json!({"announcements":[],"tags":[entry]}));
        assert_eq!(
            t(json!({"text":"x{y}","tone":"info"})),
            "tags[0].text does not take placeholders."
        );
        assert_eq!(
            t(json!({"text":"x","tone":"info","eventAt":"2026-10-16T00:00:00Z"})),
            "tags[0] does not take eventAt."
        );
        assert_eq!(
            t(json!({"text":"x".repeat(25),"tone":"info"})),
            "tags[0].text must be 1 to 24 characters."
        );
        assert_eq!(
            check(
                json!({"announcements":[{"id":"a","text":"x","tone":"info"}],"tags":[{"id":"a","text":"y","tone":"info"}]})
            ),
            "The id a is used by more than one notice or tag."
        );
    }

    #[test]
    fn a_good_save_and_the_views() {
        let mut ids = || "gen".to_owned();
        let body = json!({
            "announcements":[{"text":" Maintenance at {time} ","tone":"warning","eventAt":"2026-10-16T12:00:00Z","showUntil":"2026-10-17T00:00:00Z"}],
            "tags":[{"id":"kept","text":"Hello","tone":"info"}]
        });
        let (a, t) = validate_input(Some(&body), &mut ids).unwrap();
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&a[0]),
            r#"{"text":"Maintenance at {time}","tone":"warning","eventAt":"2026-10-16T12:00:00.000Z","showUntil":"2026-10-17T00:00:00.000Z","id":"gen"}"#
        );
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&t[0]),
            r#"{"id":"kept","text":"Hello","tone":"info"}"#
        );
        assert_eq!(
            validate_reason(Some(&json!("  fixing the date  "))).unwrap(),
            "fixing the date"
        );
        assert!(validate_reason(Some(&json!("short"))).is_err() && validate_reason(None).is_err());
        assert_eq!(validate_expected_version(Some(&json!(0))), Ok(0));
        assert_eq!(validate_expected_version(Some(&json!(4.0))), Ok(4));
        for bad in [json!(-1), json!(1.5), json!("1"), json!(null)] {
            assert!(validate_expected_version(Some(&bad)).is_err());
        }
        let now = dainty_service_kit::js::date_parse("2026-10-20T00:00:00Z").unwrap();
        let record = LauncherNoticesRecord {
            version: 1,
            updated_at: None,
            updated_by: None,
            announcements: vec![
                json!({"id":"now","text":"a","tone":"info","showFrom":"2026-10-01T00:00:00.000Z","showUntil":"2026-11-01T00:00:00.000Z"}),
                json!({"id":"later","text":"a","tone":"info","showFrom":"2026-12-01T00:00:00.000Z"}),
                json!({"id":"over","text":"a","tone":"info","showUntil":"2026-10-10T00:00:00.000Z"}),
            ],
            tags: vec![],
        };
        let view = public_view(Some(&record), now);
        assert_eq!(view["announcements"].as_array().unwrap().len(), 1);
        assert_eq!(view["announcements"][0]["id"], "now");
        let keys: Vec<&String> = view.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["announcements", "tags", "serverTime"]);
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&admin_view(None, now)),
            r#"{"version":0,"updatedAt":null,"updatedBy":null,"announcements":[],"tags":[],"serverTime":"2026-10-20T00:00:00.000Z"}"#
        );
    }
}
