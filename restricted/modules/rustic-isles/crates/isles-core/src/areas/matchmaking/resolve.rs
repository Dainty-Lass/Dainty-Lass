//! Which party a join travels with (`matchmakingParty.ts`) and the Gauntlet refusal (`matchmakingHuntPolicy.ts`).
//!
//! - ISLAND requests carry the partyId of the party the client is actively travelling with. It must match the
//!   authoritative party, and only members who are online go along, so a stale server-side party can never turn
//!   "Private Hunt / solo" into a two-player server whose airship waits forever.
//! - CITY/ReturnToRamsgate requests omit partyId in the captured 1.12 protocol, so the authoritative party is kept and
//!   returning members still fan out to the shared Ramsgate.
//! - An optional module may ask for the ISLAND member filter on another request (`filter_offline`): a Training request
//!   it may host on a player's PC freezes its roster from this list.

use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartySnapshot {
    pub party_id: String,
    pub members: Vec<String>,
    pub revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartyResolution {
    pub party_id: Option<String>,
    pub party_members: Option<Vec<String>>,
    pub party_revision: Option<i64>,
    pub excluded_members: Vec<String>,
    pub party_id_mismatch: bool,
}

/// `ResolveMatchmakingParty`.
pub fn resolve_matchmaking_party(
    game_mode: Option<&str>,
    user_id: &str,
    requested_party_id: Option<&str>,
    party: Option<&PartySnapshot>,
    is_online: impl Fn(&str) -> bool,
    filter_offline_members: bool,
) -> PartyResolution {
    let Some(party) = party else {
        return PartyResolution::default();
    };
    let island = game_mode == Some("ISLAND");
    if island && requested_party_id != Some(party.party_id.as_str()) {
        return PartyResolution {
            excluded_members: party.members.iter().filter(|m| *m != user_id).cloned().collect(),
            party_id_mismatch: true,
            ..PartyResolution::default()
        };
    }
    let online_only = island || filter_offline_members;
    let members: Vec<String> = party
        .members
        .iter()
        .filter(|m| *m == user_id || !online_only || is_online(m))
        .cloned()
        .collect();
    PartyResolution {
        party_id: Some(party.party_id.clone()),
        excluded_members: party.members.iter().filter(|m| !members.contains(m)).cloned().collect(),
        party_members: Some(members),
        party_revision: party.revision,
        party_id_mismatch: false,
    }
}

pub const GAUNTLET_DISABLED_REASON: &str = "GAUNTLET_DISABLED";

/// `GauntletMatchmakingEnabled`: only the literal `on` (case and spaces aside).
pub fn gauntlet_enabled(settings: &Settings) -> bool {
    settings
        .raw("RUSTIC_ISLES_GAUNTLET")
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("on")
}

/// `IsGauntletHuntId`: every Gauntlet hunt id in the 1.14.7 hunt tables contains "Gauntlet", and no other does.
pub fn is_gauntlet_hunt_id(hunt_id: Option<&str>) -> bool {
    hunt_id.is_some_and(|id| id.to_ascii_lowercase().contains("gauntlet"))
}

/// `RefusesGauntletRequest`: a Gauntlet request is cancelled with `GAUNTLET_DISABLED` until the switch is on.
pub fn refuses_gauntlet_request(hunt_id: Option<&str>, settings: &Settings) -> bool {
    is_gauntlet_hunt_id(hunt_id) && !gauntlet_enabled(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn party() -> PartySnapshot {
        PartySnapshot {
            party_id: "party-live".into(),
            members: vec!["leader".into(), "online-member".into(), "offline-member".into()],
            revision: Some(7),
        }
    }

    fn strings(list: &[&str]) -> Option<Vec<String>> {
        Some(list.iter().map(|s| (*s).to_owned()).collect())
    }

    #[test]
    fn island_travel_excludes_disconnected_party_members() {
        let result = resolve_matchmaking_party(
            Some("ISLAND"),
            "leader",
            Some("party-live"),
            Some(&party()),
            |id| id == "online-member",
            false,
        );
        assert_eq!(result.party_id.as_deref(), Some("party-live"));
        assert_eq!(result.party_members, strings(&["leader", "online-member"]));
        assert_eq!(result.party_revision, Some(7));
        assert_eq!(result.excluded_members, vec!["offline-member".to_owned()]);
        assert!(!result.party_id_mismatch);
    }

    #[test]
    fn island_travel_without_the_active_party_id_falls_back_to_solo() {
        let result = resolve_matchmaking_party(Some("ISLAND"), "leader", None, Some(&party()), |_| true, false);
        assert_eq!(result.party_id, None);
        assert_eq!(result.party_members, None);
        assert_eq!(result.party_revision, None);
        assert!(result.party_id_mismatch);
        assert_eq!(
            result.excluded_members,
            vec!["online-member".to_owned(), "offline-member".to_owned()]
        );
    }

    #[test]
    fn city_travel_keeps_the_authoritative_party_because_1_12_omits_party_id() {
        let result = resolve_matchmaking_party(Some("CITY"), "leader", None, Some(&party()), |_| false, false);
        assert_eq!(result.party_id.as_deref(), Some("party-live"));
        assert_eq!(result.party_members, Some(party().members));
        assert!(result.excluded_members.is_empty());
    }

    #[test]
    fn a_player_without_a_party_travels_solo() {
        let result = resolve_matchmaking_party(Some("ISLAND"), "leader", Some("party-live"), None, |_| true, false);
        assert_eq!(result.party_id, None);
        assert_eq!(result.party_members, None);
        assert!(!result.party_id_mismatch);
    }

    #[test]
    fn shared_keeps_offline_members_unless_a_module_asks_for_the_online_filter() {
        let online = |id: &str| id == "online-member";
        let default = resolve_matchmaking_party(Some("SHARED"), "leader", None, Some(&party()), online, false);
        assert_eq!(default.party_members, Some(party().members));
        assert!(default.excluded_members.is_empty());

        let filtered = resolve_matchmaking_party(Some("SHARED"), "leader", None, Some(&party()), online, true);
        assert_eq!(filtered.party_id.as_deref(), Some("party-live"));
        assert_eq!(filtered.party_revision, Some(7));
        assert!(!filtered.party_id_mismatch);
        assert_eq!(filtered.party_members, strings(&["leader", "online-member"]));
        assert_eq!(filtered.excluded_members, vec!["offline-member".to_owned()]);

        let city = resolve_matchmaking_party(Some("CITY"), "leader", None, Some(&party()), online, false);
        assert_eq!(city.party_members, Some(party().members));
        let island = resolve_matchmaking_party(Some("ISLAND"), "leader", None, Some(&party()), online, true);
        assert!(island.party_id_mismatch);
    }

    #[test]
    fn gauntlet_is_enabled_only_by_the_literal_on() {
        for value in ["on", " ON ", "On"] {
            assert!(
                gauntlet_enabled(&Settings::from_pairs([("RUSTIC_ISLES_GAUNTLET", value)])),
                "{value}"
            );
        }
        for value in ["", "off", "true", "1", "yes", "onn"] {
            assert!(
                !gauntlet_enabled(&Settings::from_pairs([("RUSTIC_ISLES_GAUNTLET", value)])),
                "{value}"
            );
        }
        assert!(!gauntlet_enabled(&Settings::default()));
    }

    #[test]
    fn gauntlet_hunt_ids_are_the_ids_that_contain_gauntlet() {
        for id in [
            "CR19_PlayerHunt_Gauntlet",
            "CR19_MatchmakerHunt_Gauntlet",
            "CR19_PlayerHunt_gauntlet_Extra",
        ] {
            assert!(is_gauntlet_hunt_id(Some(id)), "{id}");
        }
        for id in [
            "ShatteredIsles_IslandA",
            "CR19_PlayerHunt_Arena_Elite",
            "Trials_Island_01",
            "ShatteredIsles_ReturnToRamsgate",
        ] {
            assert!(!is_gauntlet_hunt_id(Some(id)), "{id}");
        }
        assert!(!is_gauntlet_hunt_id(None));
    }

    #[test]
    fn a_gauntlet_request_is_refused_while_the_switch_is_off() {
        assert_eq!(GAUNTLET_DISABLED_REASON, "GAUNTLET_DISABLED");
        let off = Settings::default();
        let on = Settings::from_pairs([("RUSTIC_ISLES_GAUNTLET", "on")]);
        assert!(refuses_gauntlet_request(Some("CR19_PlayerHunt_Gauntlet"), &off));
        assert!(!refuses_gauntlet_request(Some("CR19_MatchmakerHunt_Gauntlet"), &on));
        assert!(!refuses_gauntlet_request(Some("ShatteredIsles_IslandA"), &off));
    }
}
