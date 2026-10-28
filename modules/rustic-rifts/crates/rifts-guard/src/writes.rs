//! Who a hosting runtime may write for (`hostWrites.ts`, `hubAuthorization.ts`, `p2pInventoryTransactions.ts`). Pure:
//! the hooks in `lib.rs` read the session and the hub's presence and log.

use rifts_seam::{AccountId, HostWriteKind, HubKind, Json, Reply, RuntimeAuth};

use crate::json::js_string;
use crate::ports::HuntView;

pub const fn kind_name(kind: HostWriteKind) -> &'static str {
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

/// Why a host write is refused (`HostWriteDenialReason`). `PLAYER_HOST_AUTH_REQUIRED` cannot happen here: the seam
/// hands Guard a checked [`RuntimeAuth`] only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteDenial {
    HuntNotFound,
    HuntNotRunning,
    WriteKindNotAllowedInHub,
    WriteKindNotAllowed,
    NotAMember,
    MemberNotAdmitted,
    MemberNotPresent,
}

impl WriteDenial {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HuntNotFound => "HUNT_NOT_FOUND",
            Self::HuntNotRunning => "HUNT_NOT_RUNNING",
            Self::WriteKindNotAllowedInHub => "WRITE_KIND_NOT_ALLOWED_IN_HUB",
            Self::WriteKindNotAllowed => "WRITE_KIND_NOT_ALLOWED",
            Self::NotAMember => "NOT_A_MEMBER",
            Self::MemberNotAdmitted => "MEMBER_NOT_ADMITTED",
            Self::MemberNotPresent => "MEMBER_NOT_PRESENT",
        }
    }

    /// `HOST_WRITE_KIND_POLICY_REASONS`: the host policy's, which observe mode only logs. Every other reason is about
    /// who may write and always refuses.
    pub const fn is_policy(self) -> bool {
        matches!(self, Self::WriteKindNotAllowedInHub | Self::WriteKindNotAllowed)
    }
}

/// `HUB_WRITE_KINDS`: what the dedicated Ramsgate does for a player, never XP (a hub has no XP source, decision D4);
/// the Training hub the same but the reforge, or its tutorial's quest step (a bounty write) could not progress.
pub fn hub_allows(hub: HubKind, kind: Option<HostWriteKind>) -> bool {
    use HostWriteKind::{
        Bounty, Cooldown, Escalation, HuntPassXp, ItemSave, LoadoutUnlock, Mastery, Objectives, RankConfirm, Reforge,
        SlayersPath,
    };
    match (hub, kind) {
        (_, None) => false,
        (HubKind::Ramsgate, Some(kind)) => matches!(
            kind,
            Bounty
                | Cooldown
                | SlayersPath
                | Escalation
                | Objectives
                | Mastery
                | Reforge
                | RankConfirm
                | LoadoutUnlock
                | HuntPassXp
                | ItemSave
        ),
        (HubKind::Training, Some(kind)) => matches!(
            kind,
            Bounty
                | Cooldown
                | SlayersPath
                | Escalation
                | Objectives
                | Mastery
                | RankConfirm
                | LoadoutUnlock
                | HuntPassXp
                | ItemSave
        ),
    }
}

/// `HuntAllowsWriteKind`: every kind but the reforge, a hub action.
pub fn hunt_allows(kind: Option<HostWriteKind>) -> bool {
    kind != Some(HostWriteKind::Reforge)
}

/// `IsHubRuntimeAccount`: the host and the members present.
pub fn is_hub_runtime_account(host: Option<&AccountId>, present: &[AccountId], account: &AccountId) -> bool {
    host == Some(account) || present.contains(account)
}

/// A drained hunt still serves its players until its server ends, so their saves keep working.
fn running(hunt: &HuntView) -> bool {
    matches!(hunt.state.as_str(), "RUNNING" | "DRAINING")
        || (hunt.state == "SUSPECT" && hunt.pre_suspect_state.as_deref() == Some("RUNNING"))
}

/// `HostWriteDenialReason`. `presence` is read for a hub only (without it only the host is present); `check_kind:
/// false` checks only who may write.
pub fn host_write_denial(
    runtime: &RuntimeAuth,
    hunt: Option<&HuntView>,
    target: &AccountId,
    kind: Option<HostWriteKind>,
    presence: Option<&[AccountId]>,
    check_kind: bool,
) -> Option<WriteDenial> {
    let Some(hunt) = hunt.filter(|hunt| hunt.session == runtime.session && hunt.generation == runtime.generation)
    else {
        return Some(WriteDenial::HuntNotFound);
    };
    if !running(hunt) {
        return Some(WriteDenial::HuntNotRunning);
    }
    if let Some(hub) = hunt.hub {
        if check_kind && !hub_allows(hub, kind) {
            return Some(WriteDenial::WriteKindNotAllowedInHub);
        }
        if !is_hub_runtime_account(hunt.host.as_ref(), presence.unwrap_or_default(), target) {
            return Some(WriteDenial::MemberNotPresent);
        }
        return None;
    }
    if check_kind && !hunt_allows(kind) {
        return Some(WriteDenial::WriteKindNotAllowed);
    }
    let Some(participant) = hunt.roster.iter().find(|entry| &entry.account == target) else {
        return Some(WriteDenial::NotAMember);
    };
    if Some(target) != hunt.host.as_ref() && !participant.admitted {
        return Some(WriteDenial::MemberNotAdmitted);
    }
    None
}

/// The 403 the core sends for a refused host write.
pub fn write_refusal(message: &str) -> Reply {
    Reply {
        status: 403,
        body: Json(format!(
            r#"{{"code":"P2P_HOST_WRITE_DENIED","message":{},"payload":null}}"#,
            js_string(message)
        )),
    }
}

pub const LOOT_DENIED: &str = "Player-hosted hunt loot grant is not authorized.";

pub fn loot_refusal(code: &str) -> Reply {
    Reply {
        status: 403,
        body: Json(format!(
            r#"{{"code":{},"message":{},"payload":null}}"#,
            js_string(code),
            js_string(LOOT_DENIED)
        )),
    }
}

/// `P2PInventoryTransactionDenialReason` for the account the transaction is for.
pub fn inventory_denial(
    runtime: &RuntimeAuth,
    hunt: Option<&HuntView>,
    account: &AccountId,
    character_id: &str,
    presence: Option<&[AccountId]>,
) -> Option<&'static str> {
    inventory_body_denial(runtime, hunt, account, character_id, None, presence)
}

/// `P2PInventoryTransactionDenialReason` including the body's `accountId` check when the body is available.
pub fn inventory_body_denial(
    runtime: &RuntimeAuth,
    hunt: Option<&HuntView>,
    account: &AccountId,
    character_id: &str,
    body: Option<&serde_json::Value>,
    presence: Option<&[AccountId]>,
) -> Option<&'static str> {
    let body_account_mismatch = || {
        body.and_then(|v| v.get("accountId"))
            .is_some_and(|val| !val.is_null() && val.as_str() != Some(account.0.as_str()))
    };
    let hunt = hunt.filter(|hunt| {
        hunt.session == runtime.session
            && hunt.generation == runtime.generation
            && matches!(hunt.state.as_str(), "RUNNING" | "DRAINING")
    });
    let Some(hunt) = hunt else {
        return Some("HUNT_NOT_RUNNING");
    };
    if hunt.is_hub() {
        if !is_hub_runtime_account(hunt.host.as_ref(), presence.unwrap_or_default(), account) {
            return Some("PARTICIPANT_NOT_ADMITTED");
        }
        if body_account_mismatch() {
            return Some("ACCOUNT_MISMATCH");
        }
        return None;
    }
    if !hunt
        .roster
        .iter()
        .any(|entry| &entry.account == account && entry.admitted)
    {
        return Some("PARTICIPANT_NOT_ADMITTED");
    }
    if body_account_mismatch() {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ports::RosterEntry;
    use rifts_seam::{CapabilityScope, SessionId};

    pub(crate) fn account(name: &str) -> AccountId {
        AccountId(name.to_owned())
    }

    pub(crate) fn session() -> SessionId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap()
    }

    pub(crate) fn hunt() -> HuntView {
        HuntView {
            session: session(),
            generation: 2,
            state: "RUNNING".to_owned(),
            pre_suspect_state: None,
            host: Some(account("host")),
            realm: None,
            hub: None,
            roster: vec![
                RosterEntry {
                    account: account("host"),
                    admitted: true,
                },
                RosterEntry {
                    account: account("friend"),
                    admitted: true,
                },
                RosterEntry {
                    account: account("late"),
                    admitted: false,
                },
            ],
            frozen_characters: vec![(account("friend"), "char-friend".to_owned())],
        }
    }

    pub(crate) fn hub(kind: HubKind) -> HuntView {
        HuntView {
            hub: Some(kind),
            roster: Vec::new(),
            frozen_characters: Vec::new(),
            realm: Some("01ARZ3NDEKTSV4RRFFQ69G5FAW@local".parse().unwrap()),
            ..hunt()
        }
    }

    pub(crate) fn runtime() -> RuntimeAuth {
        RuntimeAuth {
            capability_id: "cap-1".to_owned(),
            session: session(),
            generation: 2,
            host_account: account("host"),
            scopes: vec![CapabilityScope::GameServerRuntime],
            roster: vec![account("host"), account("friend")],
            host_peer: None,
            bridge_instance: None,
            wire_session: None,
        }
    }

    fn with_state(mut view: HuntView, state: &str, pre: Option<&str>) -> HuntView {
        view.state = state.to_owned();
        view.pre_suspect_state = pre.map(str::to_owned);
        view
    }

    fn denial(
        view: &HuntView,
        target: &str,
        kind: Option<HostWriteKind>,
        present: Option<&[AccountId]>,
    ) -> Option<&'static str> {
        host_write_denial(&runtime(), Some(view), &account(target), kind, present, true).map(WriteDenial::as_str)
    }

    const ALL: [HostWriteKind; 11] = [
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
    ];

    // hostWrites.test.ts
    #[test]
    fn a_host_may_write_for_itself_and_its_admitted_members_of_a_running_hunt() {
        assert_eq!(denial(&hunt(), "host", None, None), None);
        assert_eq!(denial(&hunt(), "friend", None, None), None);
        assert_eq!(
            denial(&with_state(hunt(), "SUSPECT", Some("RUNNING")), "friend", None, None),
            None
        );
    }

    #[test]
    fn a_drained_hunt_keeps_its_members_saves() {
        assert_eq!(
            denial(&with_state(hunt(), "DRAINING", None), "friend", None, None),
            None
        );
        assert_eq!(
            denial(&with_state(hunt(), "DRAINING", None), "stranger", None, None),
            Some("NOT_A_MEMBER")
        );
        assert_eq!(
            denial(&with_state(hunt(), "FENCED", None), "friend", None, None),
            Some("HUNT_NOT_RUNNING")
        );
    }

    #[test]
    fn a_host_write_is_refused_outside_its_running_hunt_and_its_admitted_members() {
        assert_eq!(
            host_write_denial(&runtime(), None, &account("host"), None, None, true),
            Some(WriteDenial::HuntNotFound)
        );
        let other_generation = HuntView {
            generation: 3,
            ..hunt()
        };
        assert_eq!(denial(&other_generation, "host", None, None), Some("HUNT_NOT_FOUND"));
        assert_eq!(
            denial(&with_state(hunt(), "ADMITTING", None), "host", None, None),
            Some("HUNT_NOT_RUNNING")
        );
        assert_eq!(
            denial(&with_state(hunt(), "COMPLETED", None), "host", None, None),
            Some("HUNT_NOT_RUNNING")
        );
        assert_eq!(denial(&hunt(), "stranger", None, None), Some("NOT_A_MEMBER"));
        assert_eq!(denial(&hunt(), "late", None, None), Some("MEMBER_NOT_ADMITTED"));
    }

    #[test]
    fn a_ramsgate_hub_takes_its_write_kinds_for_a_present_member_and_for_the_host() {
        let present = [account("friend")];
        for kind in ALL.into_iter().filter(|kind| *kind != HostWriteKind::Xp) {
            assert_eq!(
                denial(&hub(HubKind::Ramsgate), "friend", Some(kind), Some(&present)),
                None,
                "{kind:?}"
            );
            assert_eq!(
                denial(&hub(HubKind::Ramsgate), "host", Some(kind), Some(&present)),
                None,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn hubs_refuse_xp_and_training_also_refuses_the_reforge() {
        let present = [account("friend")];
        assert_eq!(
            denial(
                &hub(HubKind::Ramsgate),
                "friend",
                Some(HostWriteKind::Xp),
                Some(&present)
            ),
            Some("WRITE_KIND_NOT_ALLOWED_IN_HUB")
        );
        for kind in ALL {
            let refused = matches!(kind, HostWriteKind::Xp | HostWriteKind::Reforge);
            assert_eq!(
                denial(&hub(HubKind::Training), "friend", Some(kind), Some(&present)),
                refused.then_some("WRITE_KIND_NOT_ALLOWED_IN_HUB"),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_hub_write_needs_a_running_hub_of_this_generation_and_a_target_present_in_it() {
        let present = [account("friend")];
        let bounty = Some(HostWriteKind::Bounty);
        let ramsgate = hub(HubKind::Ramsgate);
        assert_eq!(
            denial(
                &with_state(ramsgate.clone(), "SUSPECT", Some("RUNNING")),
                "friend",
                bounty,
                Some(&present)
            ),
            None
        );
        assert_eq!(
            denial(
                &with_state(ramsgate.clone(), "SUSPECT", Some("ADMITTING")),
                "friend",
                bounty,
                Some(&present)
            ),
            Some("HUNT_NOT_RUNNING")
        );
        assert_eq!(
            denial(
                &with_state(ramsgate.clone(), "ADMITTING", None),
                "friend",
                bounty,
                Some(&present)
            ),
            Some("HUNT_NOT_RUNNING")
        );
        let other_generation = HuntView {
            generation: 3,
            ..ramsgate.clone()
        };
        assert_eq!(
            denial(&other_generation, "friend", bounty, Some(&present)),
            Some("HUNT_NOT_FOUND")
        );
        assert_eq!(
            denial(&ramsgate, "stranger", bounty, Some(&present)),
            Some("MEMBER_NOT_PRESENT")
        );
        assert_eq!(denial(&ramsgate, "friend", bounty, None), Some("MEMBER_NOT_PRESENT"));
        assert_eq!(denial(&ramsgate, "host", bounty, None), None);
    }

    #[test]
    fn a_hunt_refuses_the_reforge_and_keeps_every_other_kind_on_its_roster() {
        let present = [account("friend")];
        assert_eq!(
            denial(&hunt(), "friend", Some(HostWriteKind::Reforge), None),
            Some("WRITE_KIND_NOT_ALLOWED")
        );
        assert_eq!(denial(&hunt(), "friend", Some(HostWriteKind::Xp), None), None);
        assert_eq!(denial(&hunt(), "friend", Some(HostWriteKind::Xp), Some(&present)), None);
        assert_eq!(
            denial(&hunt(), "late", Some(HostWriteKind::Bounty), None),
            Some("MEMBER_NOT_ADMITTED")
        );
    }

    #[test]
    fn rank_confirms_and_loadout_unlocks_need_an_admitted_roster_member() {
        for kind in [HostWriteKind::RankConfirm, HostWriteKind::LoadoutUnlock] {
            assert_eq!(denial(&hunt(), "friend", Some(kind), None), None);
            assert_eq!(denial(&hunt(), "host", Some(kind), None), None);
            assert_eq!(denial(&hunt(), "late", Some(kind), None), Some("MEMBER_NOT_ADMITTED"));
            assert_eq!(denial(&hunt(), "stranger", Some(kind), None), Some("NOT_A_MEMBER"));
        }
        assert_eq!(
            denial(
                &with_state(hunt(), "COMPLETED", None),
                "friend",
                Some(HostWriteKind::LoadoutUnlock),
                None
            ),
            Some("HUNT_NOT_RUNNING")
        );
    }

    // hubAuthorization.test.ts
    #[test]
    fn hub_write_kinds_by_kind() {
        let ramsgate: Vec<_> = ALL
            .into_iter()
            .filter(|kind| hub_allows(HubKind::Ramsgate, Some(*kind)))
            .collect();
        assert!(!ramsgate.contains(&HostWriteKind::Xp));
        assert_eq!(ramsgate.len(), ALL.len() - 1);
        assert!(hub_allows(HubKind::Ramsgate, Some(HostWriteKind::ItemSave)));
        assert!(hub_allows(HubKind::Training, Some(HostWriteKind::ItemSave)));
        assert!(!hub_allows(HubKind::Training, Some(HostWriteKind::Reforge)));
        assert!(!hub_allows(HubKind::Ramsgate, None));
        assert!(hunt_allows(None));
        assert!(!hunt_allows(Some(HostWriteKind::Reforge)));
        assert!(hunt_allows(Some(HostWriteKind::LoadoutUnlock)));
    }

    #[test]
    fn the_hub_runtime_account_is_the_host_or_a_present_member() {
        let present = [account("friend")];
        assert!(is_hub_runtime_account(
            Some(&account("host")),
            &present,
            &account("host")
        ));
        assert!(is_hub_runtime_account(
            Some(&account("host")),
            &present,
            &account("friend")
        ));
        assert!(!is_hub_runtime_account(
            Some(&account("host")),
            &present,
            &account("stranger")
        ));
        assert!(is_hub_runtime_account(Some(&account("host")), &[], &account("host")));
    }

    // p2pInventoryTransactions.test.ts
    #[test]
    fn native_inventory_binds_the_running_generation_admitted_account_and_frozen_character() {
        let deny = |view: &HuntView, who: &str, character: &str| {
            inventory_denial(&runtime(), Some(view), &account(who), character, None)
        };
        assert_eq!(deny(&hunt(), "friend", "char-friend"), None);
        assert_eq!(deny(&hunt(), "friend", "other"), Some("CHARACTER_MISMATCH"));
        assert_eq!(deny(&hunt(), "late", "char-friend"), Some("PARTICIPANT_NOT_ADMITTED"));
        assert_eq!(deny(&hunt(), "stranger", "x"), Some("PARTICIPANT_NOT_ADMITTED"));
        assert_eq!(
            deny(&with_state(hunt(), "ADMITTING", None), "friend", "char-friend"),
            Some("HUNT_NOT_RUNNING")
        );
        let fenced = HuntView {
            generation: 3,
            ..hunt()
        };
        assert_eq!(deny(&fenced, "friend", "char-friend"), Some("HUNT_NOT_RUNNING"));
        assert_eq!(
            inventory_denial(&runtime(), None, &account("friend"), "char-friend", None),
            Some("HUNT_NOT_RUNNING")
        );
        let mismatched_body = serde_json::json!({ "accountId": "host" });
        assert_eq!(
            inventory_body_denial(
                &runtime(),
                Some(&hunt()),
                &account("friend"),
                "char-friend",
                Some(&mismatched_body),
                None,
            ),
            Some("ACCOUNT_MISMATCH")
        );
    }

    #[test]
    fn a_drained_hunts_native_loot_still_saves() {
        let drained = with_state(hunt(), "DRAINING", None);
        assert_eq!(
            inventory_denial(&runtime(), Some(&drained), &account("friend"), "char-friend", None),
            None
        );
        let fenced = with_state(hunt(), "FENCED", None);
        assert_eq!(
            inventory_denial(&runtime(), Some(&fenced), &account("friend"), "char-friend", None),
            Some("HUNT_NOT_RUNNING")
        );
    }

    #[test]
    fn a_hub_transaction_needs_a_running_hub_and_an_account_present_in_it() {
        let present = [account("friend")];
        let ramsgate = hub(HubKind::Ramsgate);
        assert_eq!(
            inventory_denial(&runtime(), Some(&ramsgate), &account("friend"), "any", Some(&present)),
            None
        );
        assert_eq!(
            inventory_denial(&runtime(), Some(&ramsgate), &account("host"), "any", None),
            None
        );
        assert_eq!(
            inventory_denial(&runtime(), Some(&ramsgate), &account("stranger"), "any", Some(&present)),
            Some("PARTICIPANT_NOT_ADMITTED")
        );
        let training = hub(HubKind::Training);
        assert_eq!(
            inventory_denial(&runtime(), Some(&training), &account("friend"), "any", Some(&present)),
            None
        );
    }

    #[test]
    fn refusals_use_the_core_shape() {
        assert_eq!(
            write_refusal("Host writes are switched off.").body.0,
            r#"{"code":"P2P_HOST_WRITE_DENIED","message":"Host writes are switched off.","payload":null}"#
        );
        assert_eq!(loot_refusal("P2P_LOOT_GRANT_DENIED").status, 403);
    }
}
