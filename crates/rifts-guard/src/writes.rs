// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Whom a hosted runtime may write player state for, and the refusals it gets.

use rifts_seam::{AccountId, HostWriteKind, HubKind, Json, Reply, RuntimeAuth};
use serde_json::Value;

use crate::json::js_string;
use crate::ports::HuntView;

pub fn kind_name(kind: HostWriteKind) -> &'static str {
    match kind {
        HostWriteKind::Xp => "XP",
        HostWriteKind::Mastery => "MASTERY",
        HostWriteKind::Objectives => "OBJECTIVES",
        HostWriteKind::Escalation => "ESCALATION",
        HostWriteKind::Cooldown => "COOLDOWN",
        HostWriteKind::Bounty => "BOUNTY",
        HostWriteKind::SlayersPath => "SLAYERS_PATH",
        HostWriteKind::Reforge => "REFORGE",
        HostWriteKind::RankConfirm => "RANK_CONFIRM",
        HostWriteKind::LoadoutUnlock => "LOADOUT_UNLOCK",
        HostWriteKind::HuntPassXp => "HUNT_PASS_XP",
        HostWriteKind::ItemSave => "ITEM_SAVE",
    }
}

pub(crate) const ALL_KINDS: [HostWriteKind; 12] = [
    HostWriteKind::Xp,
    HostWriteKind::Mastery,
    HostWriteKind::Objectives,
    HostWriteKind::Escalation,
    HostWriteKind::Cooldown,
    HostWriteKind::Bounty,
    HostWriteKind::SlayersPath,
    HostWriteKind::Reforge,
    HostWriteKind::RankConfirm,
    HostWriteKind::LoadoutUnlock,
    HostWriteKind::HuntPassXp,
    HostWriteKind::ItemSave,
];

pub(crate) fn parse_kind(text: &str) -> Option<HostWriteKind> {
    ALL_KINDS.into_iter().find(|kind| kind_name(*kind) == text)
}

/// Why a write is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteDenial {
    HuntNotFound,
    HuntNotRunning,
    WriteKindNotAllowedInHub,
    MemberNotPresent,
    WriteKindNotAllowed,
    NotAMember,
    MemberNotAdmitted,
}

impl WriteDenial {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HuntNotFound => "HUNT_NOT_FOUND",
            Self::HuntNotRunning => "HUNT_NOT_RUNNING",
            Self::WriteKindNotAllowedInHub => "WRITE_KIND_NOT_ALLOWED_IN_HUB",
            Self::MemberNotPresent => "MEMBER_NOT_PRESENT",
            Self::WriteKindNotAllowed => "WRITE_KIND_NOT_ALLOWED",
            Self::NotAMember => "NOT_A_MEMBER",
            Self::MemberNotAdmitted => "MEMBER_NOT_ADMITTED",
        }
    }

    /// The write kind's own refusals, which observe mode only logs.
    pub fn is_policy(self) -> bool {
        matches!(self, Self::WriteKindNotAllowedInHub | Self::WriteKindNotAllowed)
    }
}

/// A hunt never reforges; hubs grant no hunt XP, and the Training hub no reforge either.
fn kind_allowed(hub: Option<HubKind>, kind: Option<HostWriteKind>) -> bool {
    match (hub, kind) {
        (None, kind) => kind != Some(HostWriteKind::Reforge),
        (Some(_), None) => false,
        (Some(HubKind::Ramsgate), Some(kind)) => kind != HostWriteKind::Xp,
        (Some(HubKind::Training), Some(kind)) => !matches!(kind, HostWriteKind::Xp | HostWriteKind::Reforge),
    }
}

/// A suspect hunt that was running still takes host writes (inventory does not).
fn is_running(hunt: &HuntView) -> bool {
    matches!(hunt.state.as_str(), "RUNNING" | "DRAINING")
        || (hunt.state == "SUSPECT" && hunt.pre_suspect_state.as_deref() == Some("RUNNING"))
}

fn is_host(hunt: &HuntView, account: &AccountId) -> bool {
    hunt.host.as_ref() == Some(account)
}

pub fn host_write_denial(
    runtime: &RuntimeAuth,
    hunt: Option<&HuntView>,
    target: &AccountId,
    kind: Option<HostWriteKind>,
    presence: Option<&[AccountId]>,
    check_kind: bool,
) -> Option<WriteDenial> {
    let Some(hunt) = hunt.filter(|h| h.session == runtime.session && h.generation == runtime.generation) else {
        return Some(WriteDenial::HuntNotFound);
    };
    if !is_running(hunt) {
        return Some(WriteDenial::HuntNotRunning);
    }
    if hunt.is_hub() {
        if check_kind && !kind_allowed(hunt.hub, kind) {
            return Some(WriteDenial::WriteKindNotAllowedInHub);
        }
        let present = presence.unwrap_or(&[]);
        if !is_host(hunt, target) && !present.contains(target) {
            return Some(WriteDenial::MemberNotPresent);
        }
        return None;
    }
    if check_kind && !kind_allowed(None, kind) {
        return Some(WriteDenial::WriteKindNotAllowed);
    }
    let Some(entry) = hunt.roster.iter().find(|entry| &entry.account == target) else {
        return Some(WriteDenial::NotAMember);
    };
    if !is_host(hunt, target) && !entry.admitted {
        return Some(WriteDenial::MemberNotAdmitted);
    }
    None
}

pub fn write_refusal(message: &str) -> Reply {
    Reply {
        status: 403,
        body: Json(format!(
            r#"{{"code":"P2P_HOST_WRITE_DENIED","message":{},"payload":null}}"#,
            js_string(message)
        )),
    }
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
    let Some(hunt) = hunt.filter(|h| {
        h.session == runtime.session
            && h.generation == runtime.generation
            && matches!(h.state.as_str(), "RUNNING" | "DRAINING")
    }) else {
        return Some("HUNT_NOT_RUNNING");
    };
    let account_mismatch = body
        .and_then(|b| b.get("accountId"))
        .is_some_and(|id| !id.is_null() && id.as_str() != Some(account.0.as_str()));
    if hunt.is_hub() {
        if !is_host(hunt, account) && !presence.unwrap_or(&[]).contains(account) {
            return Some("PARTICIPANT_NOT_ADMITTED");
        }
        return account_mismatch.then_some("ACCOUNT_MISMATCH");
    }
    if !hunt
        .roster
        .iter()
        .any(|entry| &entry.account == account && entry.admitted)
    {
        return Some("PARTICIPANT_NOT_ADMITTED");
    }
    if account_mismatch {
        return Some("ACCOUNT_MISMATCH");
    }
    let frozen = hunt
        .frozen_characters
        .iter()
        .find(|(member, _)| member == account)
        .map(|(_, character)| character.as_str());
    if frozen != Some(character_id) {
        return Some("CHARACTER_MISMATCH");
    }
    None
}

pub fn inventory_denial(
    runtime: &RuntimeAuth,
    hunt: Option<&HuntView>,
    account: &AccountId,
    character_id: &str,
    presence: Option<&[AccountId]>,
) -> Option<&'static str> {
    inventory_body_denial(runtime, hunt, account, character_id, None, presence)
}

pub fn loot_refusal(code: &str) -> Reply {
    Reply {
        status: 403,
        body: Json(format!(
            r#"{{"code":{},"message":"Player-hosted hunt loot grant is not authorized.","payload":null}}"#,
            js_string(code)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{account, hub, hunt, runtime};
    use serde_json::json;

    fn deny(hunt: &HuntView, target: &str, kind: HostWriteKind, presence: &[&str]) -> Option<&'static str> {
        let presence: Vec<AccountId> = presence.iter().map(|a| account(a)).collect();
        host_write_denial(
            &runtime(),
            Some(hunt),
            &account(target),
            Some(kind),
            Some(&presence),
            true,
        )
        .map(WriteDenial::as_str)
    }

    #[test]
    fn hunt_writes() {
        let mut running = hunt();
        assert_eq!(deny(&running, "member", HostWriteKind::Xp, &[]), None);
        assert_eq!(deny(&running, "host", HostWriteKind::Mastery, &[]), None);
        assert_eq!(deny(&running, "member", HostWriteKind::RankConfirm, &[]), None);
        assert_eq!(deny(&running, "member", HostWriteKind::LoadoutUnlock, &[]), None);
        assert_eq!(
            deny(&running, "member", HostWriteKind::Reforge, &[]),
            Some("WRITE_KIND_NOT_ALLOWED")
        );
        assert_eq!(deny(&running, "stranger", HostWriteKind::Xp, &[]), Some("NOT_A_MEMBER"));
        assert_eq!(
            deny(&running, "pending", HostWriteKind::Xp, &[]),
            Some("MEMBER_NOT_ADMITTED")
        );
        assert!(host_write_denial(&runtime(), Some(&running), &account("member"), None, None, true).is_none());
        running.state = "DRAINING".into();
        assert_eq!(deny(&running, "member", HostWriteKind::Xp, &[]), None);
        running.state = "SUSPECT".into();
        running.pre_suspect_state = Some("RUNNING".into());
        assert_eq!(deny(&running, "member", HostWriteKind::Xp, &[]), None);
        running.pre_suspect_state = Some("STARTING".into());
        assert_eq!(
            deny(&running, "member", HostWriteKind::Xp, &[]),
            Some("HUNT_NOT_RUNNING")
        );
        running.state = "ENDED".into();
        assert_eq!(
            deny(&running, "member", HostWriteKind::Xp, &[]),
            Some("HUNT_NOT_RUNNING")
        );
        let mut fenced = hunt();
        fenced.generation = 2;
        assert_eq!(deny(&fenced, "member", HostWriteKind::Xp, &[]), Some("HUNT_NOT_FOUND"));
        assert_eq!(
            host_write_denial(&runtime(), None, &account("member"), None, None, true).map(WriteDenial::as_str),
            Some("HUNT_NOT_FOUND")
        );
    }

    #[test]
    fn hub_writes() {
        let ramsgate = hub(HubKind::Ramsgate);
        assert_eq!(deny(&ramsgate, "guest", HostWriteKind::Mastery, &["guest"]), None);
        assert_eq!(deny(&ramsgate, "guest", HostWriteKind::Reforge, &["guest"]), None);
        assert_eq!(
            deny(&ramsgate, "guest", HostWriteKind::Xp, &["guest"]),
            Some("WRITE_KIND_NOT_ALLOWED_IN_HUB")
        );
        assert_eq!(
            deny(&ramsgate, "guest", HostWriteKind::Mastery, &[]),
            Some("MEMBER_NOT_PRESENT")
        );
        assert_eq!(deny(&ramsgate, "host", HostWriteKind::Mastery, &[]), None);
        assert!(
            host_write_denial(&runtime(), Some(&ramsgate), &account("host"), None, None, true)
                .is_some_and(WriteDenial::is_policy)
        );
        let training = hub(HubKind::Training);
        assert_eq!(
            deny(&training, "guest", HostWriteKind::Reforge, &["guest"]),
            Some("WRITE_KIND_NOT_ALLOWED_IN_HUB")
        );
        assert_eq!(deny(&training, "guest", HostWriteKind::ItemSave, &["guest"]), None);
        // the identity check alone ignores the kind
        assert!(
            host_write_denial(
                &runtime(),
                Some(&training),
                &account("host"),
                Some(HostWriteKind::Xp),
                None,
                false
            )
            .is_none()
        );
        assert!(!WriteDenial::MemberNotPresent.is_policy());
    }

    #[test]
    fn inventory_denials() {
        let rt = runtime();
        let running = hunt();
        let check = |hunt: &HuntView, who: &str, character: &str, body: Option<Value>, presence: &[&str]| {
            let presence: Vec<AccountId> = presence.iter().map(|a| account(a)).collect();
            inventory_body_denial(
                &rt,
                Some(hunt),
                &account(who),
                character,
                body.as_ref(),
                Some(&presence),
            )
        };
        assert_eq!(check(&running, "member", "char-m", None, &[]), None);
        assert_eq!(
            check(&running, "member", "char-x", None, &[]),
            Some("CHARACTER_MISMATCH")
        );
        assert_eq!(check(&running, "host", "char-h", None, &[]), Some("CHARACTER_MISMATCH"));
        assert_eq!(
            check(&running, "pending", "char-p", None, &[]),
            Some("PARTICIPANT_NOT_ADMITTED")
        );
        assert_eq!(
            check(&running, "member", "char-m", Some(json!({"accountId": "other"})), &[]),
            Some("ACCOUNT_MISMATCH")
        );
        assert_eq!(
            check(&running, "member", "char-m", Some(json!({"accountId": null})), &[]),
            None
        );
        assert_eq!(
            check(&running, "member", "char-m", Some(json!({"accountId": 5})), &[]),
            Some("ACCOUNT_MISMATCH")
        );
        let mut suspect = hunt();
        suspect.state = "SUSPECT".into();
        suspect.pre_suspect_state = Some("RUNNING".into());
        assert_eq!(check(&suspect, "member", "char-m", None, &[]), Some("HUNT_NOT_RUNNING"));
        let ramsgate = hub(HubKind::Ramsgate);
        assert_eq!(check(&ramsgate, "guest", "any", None, &["guest"]), None);
        assert_eq!(
            check(&ramsgate, "guest", "any", None, &[]),
            Some("PARTICIPANT_NOT_ADMITTED")
        );
        assert_eq!(check(&ramsgate, "host", "any", None, &[]), None);
        assert_eq!(
            inventory_denial(&rt, None, &account("member"), "char-m", None),
            Some("HUNT_NOT_RUNNING")
        );
    }

    #[test]
    fn refusal_bodies() {
        assert_eq!(
            write_refusal("This host may not write this player's state.").body.0,
            r#"{"code":"P2P_HOST_WRITE_DENIED","message":"This host may not write this player's state.","payload":null}"#
        );
        let loot = loot_refusal("P2P_LOOT_GRANT_DENIED");
        assert_eq!(loot.status, 403);
        assert_eq!(
            loot.body.0,
            r#"{"code":"P2P_LOOT_GRANT_DENIED","message":"Player-hosted hunt loot grant is not authorized.","payload":null}"#
        );
        let names: Vec<&str> = ALL_KINDS.map(kind_name).to_vec();
        assert_eq!(
            names,
            [
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
                "ITEM_SAVE"
            ]
        );
        assert_eq!(parse_kind("SLAYERS_PATH"), Some(HostWriteKind::SlayersPath));
        assert_eq!(parse_kind("xp"), None);
    }
}
