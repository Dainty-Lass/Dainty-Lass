// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Whom a hosted runtime may write player state for, and the refusals it gets.

use rifts_seam::{AccountId, HostWriteKind, HubKind, Json, Reply, RuntimeAuth};
use serde_json::Value;

use crate::{json::js_string, ports::HuntView};

/// Every write kind with its stored and logged name.
const KIND_NAMES: [(HostWriteKind, &str); 12] = [
    (HostWriteKind::Xp, "XP"),
    (HostWriteKind::Mastery, "MASTERY"),
    (HostWriteKind::Objectives, "OBJECTIVES"),
    (HostWriteKind::Escalation, "ESCALATION"),
    (HostWriteKind::Cooldown, "COOLDOWN"),
    (HostWriteKind::Bounty, "BOUNTY"),
    (HostWriteKind::SlayersPath, "SLAYERS_PATH"),
    (HostWriteKind::Reforge, "REFORGE"),
    (HostWriteKind::RankConfirm, "RANK_CONFIRM"),
    (HostWriteKind::LoadoutUnlock, "LOADOUT_UNLOCK"),
    (HostWriteKind::HuntPassXp, "HUNT_PASS_XP"),
    (HostWriteKind::ItemSave, "ITEM_SAVE"),
];

#[cfg(test)]
pub(crate) const ALL_KINDS: [HostWriteKind; 12] = {
    let mut kinds = [HostWriteKind::Xp; 12];
    let mut at = 0;
    while at < KIND_NAMES.len() {
        kinds[at] = KIND_NAMES[at].0;
        at += 1;
    }
    kinds
};

/// The kind's name in logs and the store, such as `xp`.
pub fn kind_name(kind: HostWriteKind) -> &'static str {
    KIND_NAMES
        .iter()
        .find(|(known, _)| *known == kind)
        .map_or("", |(_, name)| name)
}

pub(crate) fn parse_kind(text: &str) -> Option<HostWriteKind> {
    KIND_NAMES.iter().find(|(_, name)| *name == text).map(|(kind, _)| *kind)
}

/// Why a write is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteDenial {
    /// No hunt for this runtime's session and generation.
    HuntNotFound,
    /// The hunt is not `RUNNING` or `DRAINING`, nor `SUSPECT` after `RUNNING`.
    HuntNotRunning,
    /// A hub refuses this kind of write.
    WriteKindNotAllowedInHub,
    /// The target is neither the hub's host nor present in it.
    MemberNotPresent,
    /// A hunt refuses this kind of write.
    WriteKindNotAllowed,
    /// The target is not on the hunt's roster.
    NotAMember,
    /// The target is on the roster but was not admitted.
    MemberNotAdmitted,
}

impl WriteDenial {
    /// The denial's code, such as `NOT_A_MEMBER`.
    pub fn as_str(self) -> &'static str {
        use WriteDenial::*;
        match self {
            HuntNotFound => "HUNT_NOT_FOUND",
            HuntNotRunning => "HUNT_NOT_RUNNING",
            WriteKindNotAllowedInHub => "WRITE_KIND_NOT_ALLOWED_IN_HUB",
            MemberNotPresent => "MEMBER_NOT_PRESENT",
            WriteKindNotAllowed => "WRITE_KIND_NOT_ALLOWED",
            NotAMember => "NOT_A_MEMBER",
            MemberNotAdmitted => "MEMBER_NOT_ADMITTED",
        }
    }

    /// The write kind's own refusals, which observe mode only logs.
    pub fn is_policy(self) -> bool {
        self == Self::WriteKindNotAllowed || self == Self::WriteKindNotAllowedInHub
    }
}

/// A hunt never reforges; hubs grant no hunt XP, and the Training hub no reforge either.
fn kind_allowed(hub: Option<HubKind>, kind: Option<HostWriteKind>) -> bool {
    let barred: &[HostWriteKind] = match hub {
        None => &[HostWriteKind::Reforge],
        Some(HubKind::Ramsgate) => &[HostWriteKind::Xp],
        Some(HubKind::Training) => &[HostWriteKind::Xp, HostWriteKind::Reforge],
    };
    // A hub write must name its kind; a hunt's identity check may leave it out.
    match kind {
        Some(kind) => !barred.contains(&kind),
        None => hub.is_none(),
    }
}

/// A suspect hunt that was running still takes host writes (inventory does not).
fn is_running(hunt: &HuntView) -> bool {
    match hunt.state.as_str() {
        "RUNNING" | "DRAINING" => true,
        "SUSPECT" => hunt.pre_suspect_state.as_deref() == Some("RUNNING"),
        _ => false,
    }
}

fn is_host(hunt: &HuntView, account: &AccountId) -> bool {
    hunt.host.as_ref().is_some_and(|host| host == account)
}

/// The host, or an account the hub says is present.
fn in_hub(hunt: &HuntView, account: &AccountId, presence: Option<&[AccountId]>) -> bool {
    is_host(hunt, account) || presence.is_some_and(|present| present.contains(account))
}

fn this_generation<'a>(hunt: Option<&'a HuntView>, runtime: &RuntimeAuth) -> Option<&'a HuntView> {
    hunt.filter(|h| h.generation == runtime.generation && h.session == runtime.session)
}

/// Why the runtime may not write `target`'s state, `None` when it may.
/// The kind is checked only with `check_kind`; a hub write must name its kind.
pub fn host_write_denial(
    runtime: &RuntimeAuth,
    hunt: Option<&HuntView>,
    target: &AccountId,
    kind: Option<HostWriteKind>,
    presence: Option<&[AccountId]>,
    check_kind: bool,
) -> Option<WriteDenial> {
    use WriteDenial::*;
    let Some(hunt) = this_generation(hunt, runtime) else {
        return Some(HuntNotFound);
    };
    if !is_running(hunt) {
        return Some(HuntNotRunning);
    }
    let kind_refused = check_kind && !kind_allowed(hunt.hub, kind);
    if hunt.is_hub() {
        return match (kind_refused, in_hub(hunt, target, presence)) {
            (true, _) => Some(WriteKindNotAllowedInHub),
            (false, false) => Some(MemberNotPresent),
            (false, true) => None,
        };
    }
    if kind_refused {
        return Some(WriteKindNotAllowed);
    }
    match hunt.roster.iter().find(|entry| entry.account == *target) {
        None => Some(NotAMember),
        Some(entry) if !entry.admitted && !is_host(hunt, target) => Some(MemberNotAdmitted),
        Some(_) => None,
    }
}

/// A 403 in the module's `{code, message, payload}` envelope.
fn forbidden(code_json: &str, message_json: &str) -> Reply {
    let body = format!(r#"{{"code":{code_json},"message":{message_json},"payload":null}}"#);
    Reply {
        status: 403,
        body: Json(body),
    }
}

/// The 403 `P2P_HOST_WRITE_DENIED` reply with this message.
pub fn write_refusal(message: &str) -> Reply {
    forbidden(r#""P2P_HOST_WRITE_DENIED""#, &js_string(message))
}

/// Why an inventory transaction for `account` is refused; a suspect hunt is refused here, unlike a write.
pub fn inventory_body_denial(
    runtime: &RuntimeAuth,
    hunt: Option<&HuntView>,
    account: &AccountId,
    character_id: &str,
    body: Option<&Value>,
    presence: Option<&[AccountId]>,
) -> Option<&'static str> {
    let running = this_generation(hunt, runtime).filter(|h| h.state == "RUNNING" || h.state == "DRAINING");
    let Some(hunt) = running else {
        return Some("HUNT_NOT_RUNNING");
    };
    // A body may name the account; `null` names none.
    let named = body.and_then(|b| b.get("accountId")).filter(|id| !id.is_null());
    let mismatch = named.is_some_and(|id| id.as_str() != Some(account.0.as_str()));
    let admitted = if hunt.is_hub() {
        in_hub(hunt, account, presence)
    } else {
        hunt.roster
            .iter()
            .any(|entry| entry.admitted && entry.account == *account)
    };
    if !admitted {
        return Some("PARTICIPANT_NOT_ADMITTED");
    }
    if mismatch {
        return Some("ACCOUNT_MISMATCH");
    }
    if hunt.is_hub() {
        return None;
    }
    let frozen = hunt
        .frozen_characters
        .iter()
        .find_map(|(member, character)| (member == account).then_some(character));
    (frozen.map(String::as_str) != Some(character_id)).then_some("CHARACTER_MISMATCH")
}

/// Why an inventory transaction for `account` is refused, without a body to check.
pub fn inventory_denial(
    runtime: &RuntimeAuth,
    hunt: Option<&HuntView>,
    account: &AccountId,
    character_id: &str,
    presence: Option<&[AccountId]>,
) -> Option<&'static str> {
    inventory_body_denial(runtime, hunt, account, character_id, None, presence)
}

/// The 403 loot refusal reply with this code.
pub fn loot_refusal(code: &str) -> Reply {
    forbidden(
        &js_string(code),
        r#""Player-hosted hunt loot grant is not authorized.""#,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testkit::{account, hub, hunt, runtime};

    fn accounts(names: &[&str]) -> Vec<AccountId> {
        names.iter().map(|name| account(name)).collect()
    }

    fn deny(hunt: &HuntView, target: &str, kind: HostWriteKind, presence: &[&str]) -> Option<&'static str> {
        let present = accounts(presence);
        let denial = host_write_denial(
            &runtime(),
            Some(hunt),
            &account(target),
            Some(kind),
            Some(&present),
            true,
        );
        denial.map(WriteDenial::as_str)
    }

    #[test]
    fn hunt_writes() {
        use HostWriteKind::*;
        let mut running = hunt();
        let cases = [
            ("member", Xp, None),
            ("host", Mastery, None),
            ("member", RankConfirm, None),
            ("member", LoadoutUnlock, None),
            ("member", Reforge, Some("WRITE_KIND_NOT_ALLOWED")),
            ("stranger", Xp, Some("NOT_A_MEMBER")),
            ("pending", Xp, Some("MEMBER_NOT_ADMITTED")),
        ];
        for (target, kind, denial) in cases {
            assert_eq!(deny(&running, target, kind, &[]), denial, "{target} {kind:?}");
        }
        assert!(host_write_denial(&runtime(), Some(&running), &account("member"), None, None, true).is_none());

        let states = [
            ("DRAINING", None, None),
            ("SUSPECT", Some("RUNNING"), None),
            ("SUSPECT", Some("STARTING"), Some("HUNT_NOT_RUNNING")),
            ("ENDED", Some("STARTING"), Some("HUNT_NOT_RUNNING")),
        ];
        for (state, before, denial) in states {
            running.state = state.into();
            running.pre_suspect_state = before.map(String::from);
            assert_eq!(deny(&running, "member", Xp, &[]), denial, "{state}");
        }

        let fenced = HuntView {
            generation: 2,
            ..hunt()
        };
        assert_eq!(deny(&fenced, "member", Xp, &[]), Some("HUNT_NOT_FOUND"));
        let missing = host_write_denial(&runtime(), None, &account("member"), None, None, true);
        assert_eq!(missing, Some(WriteDenial::HuntNotFound));
    }

    #[test]
    fn hub_writes() {
        use HostWriteKind::*;
        let ramsgate = hub(HubKind::Ramsgate);
        let training = hub(HubKind::Training);
        let cases = [
            (&ramsgate, "guest", Mastery, &["guest"][..], None),
            (&ramsgate, "guest", Reforge, &["guest"][..], None),
            (
                &ramsgate,
                "guest",
                Xp,
                &["guest"][..],
                Some("WRITE_KIND_NOT_ALLOWED_IN_HUB"),
            ),
            (&ramsgate, "guest", Mastery, &[][..], Some("MEMBER_NOT_PRESENT")),
            (&ramsgate, "host", Mastery, &[][..], None),
            (
                &training,
                "guest",
                Reforge,
                &["guest"][..],
                Some("WRITE_KIND_NOT_ALLOWED_IN_HUB"),
            ),
            (&training, "guest", ItemSave, &["guest"][..], None),
        ];
        for (hub, target, kind, presence, denial) in cases {
            assert_eq!(
                deny(hub, target, kind, presence),
                denial,
                "{:?} {target} {kind:?}",
                hub.hub
            );
        }
        let unnamed = host_write_denial(&runtime(), Some(&ramsgate), &account("host"), None, None, true);
        assert!(unnamed.is_some_and(WriteDenial::is_policy));
        // The identity check alone ignores the kind.
        let identity = host_write_denial(&runtime(), Some(&training), &account("host"), Some(Xp), None, false);
        assert!(identity.is_none());
        assert!(!WriteDenial::MemberNotPresent.is_policy());
    }

    #[test]
    fn inventory_denials() {
        let rt = runtime();
        let check = |hunt: &HuntView, who: &str, character: &str, body: Option<Value>, presence: &[&str]| {
            let present = accounts(presence);
            inventory_body_denial(&rt, Some(hunt), &account(who), character, body.as_ref(), Some(&present))
        };
        let running = hunt();
        let cases = [
            ("member", "char-m", None, None),
            ("member", "char-x", None, Some("CHARACTER_MISMATCH")),
            ("host", "char-h", None, Some("CHARACTER_MISMATCH")),
            ("pending", "char-p", None, Some("PARTICIPANT_NOT_ADMITTED")),
            (
                "member",
                "char-m",
                Some(json!({"accountId": "other"})),
                Some("ACCOUNT_MISMATCH"),
            ),
            ("member", "char-m", Some(json!({"accountId": null})), None),
            (
                "member",
                "char-m",
                Some(json!({"accountId": 5})),
                Some("ACCOUNT_MISMATCH"),
            ),
        ];
        for (who, character, body, denial) in cases {
            assert_eq!(
                check(&running, who, character, body.clone(), &[]),
                denial,
                "{who} {character} {body:?}"
            );
        }
        let suspect = HuntView {
            state: "SUSPECT".into(),
            pre_suspect_state: Some("RUNNING".into()),
            ..hunt()
        };
        assert_eq!(check(&suspect, "member", "char-m", None, &[]), Some("HUNT_NOT_RUNNING"));
        let ramsgate = hub(HubKind::Ramsgate);
        let hub_cases = [
            ("guest", &["guest"][..], None),
            ("guest", &[][..], Some("PARTICIPANT_NOT_ADMITTED")),
            ("host", &[][..], None),
        ];
        for (who, presence, denial) in hub_cases {
            assert_eq!(check(&ramsgate, who, "any", None, presence), denial, "{who}");
        }
        assert_eq!(
            inventory_denial(&rt, None, &account("member"), "char-m", None),
            Some("HUNT_NOT_RUNNING")
        );
    }

    #[test]
    fn refusal_bodies() {
        let write = write_refusal("This host may not write this player's state.");
        let want = concat!(
            r#"{"code":"P2P_HOST_WRITE_DENIED","#,
            r#""message":"This host may not write this player's state.","payload":null}"#
        );
        assert_eq!(write.body.0, want);
        let loot = loot_refusal("P2P_LOOT_GRANT_DENIED");
        assert_eq!(loot.status, 403);
        let want = concat!(
            r#"{"code":"P2P_LOOT_GRANT_DENIED","#,
            r#""message":"Player-hosted hunt loot grant is not authorized.","payload":null}"#
        );
        assert_eq!(loot.body.0, want);
        let names = ALL_KINDS.map(kind_name);
        let want = [
            "XP",
            "MASTERY",
            "OBJECTIVES",
            "ESCALATION",
            "COOLDOWN",
            "BOUNTY",
            "SLAYERS_PATH",
            "REFORGE",
            "RANK_CONFIRM",
            "LOADOUT_UNLOCK",
            "HUNT_PASS_XP",
            "ITEM_SAVE",
        ];
        assert_eq!(names, want);
        for (kind, name) in ALL_KINDS.into_iter().zip(want) {
            assert_eq!(parse_kind(name), Some(kind));
        }
        assert_eq!(parse_kind("xp"), None);
    }
}
