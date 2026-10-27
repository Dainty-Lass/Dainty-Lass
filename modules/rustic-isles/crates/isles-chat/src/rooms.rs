//! `ChatQuery::Jid` and `ChatQuery::Room` (`XMPPAuth.ts`, `RoomService.ts#authorizeJoin`).
//!
//! Authorization is by the authenticated account, never the nickname. The rules, by room:
//!
//! - **Party**: only the members of that party (`GetPartyForPlayer(account).partyId`). The plugin re-asks every minute
//!   and removes whoever is no longer admitted, which is `evictPartyMember` on a leave, kick or disband.
//! - **Region** (Paradox's `City-*` rooms): the players matchmaking sent to a hub in that region ([`HubSessions`],
//!   patch 0025), or anyone with `REALTIME_XMPP_DEV_CITY_MUC`.
//! - **Hunt**: anyone with `REALTIME_XMPP_DEV_HUNT_MUC`, which is on unless switched off, as Paradox's was.
//! - **Network** (`General`): refused. Paradox has no such room ("unknown room type"); DD21's network-wide room is
//!   for games that have one.
//!
//! History is kept nowhere: Paradox's rooms were in memory and a joiner saw only what was said after they joined.

use std::time::{SystemTime, UNIX_EPOCH};

use module_api::chat::{Jid, RoomKind, RoomRules};
use module_api::{ErrorKind, GlobalId, IdKind, ModuleError};

use crate::XMPP_DOMAIN;
use crate::hubs::HubSessions;
use crate::ids::instance_id;
use crate::source::ChatSource;

pub fn now_millis() -> u64 {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// The player's JID: their `userId` on [`XMPP_DOMAIN`], so the SASL authcid, which must equal the token's `userId`,
/// is the local part (the plugin refuses a login whose authcid is not the JID's local part).
pub async fn jid_of(source: &dyn ChatSource, account: &GlobalId) -> Result<Jid, ModuleError> {
    let player = source
        .player(account)
        .await?
        .ok_or_else(|| ModuleError::new(ErrorKind::NotFound, format!("no account {account}")))?;
    Jid::new(&player.user_id, XMPP_DOMAIN, None).map_err(|error| {
        ModuleError::new(
            ErrorKind::Invalid,
            format!("the userId is not a JID local part ({error})"),
        )
    })
}

const REFUSED: RoomRules = RoomRules {
    may_join: false,
    may_speak: false,
    history: false,
};

const OPEN: RoomRules = RoomRules {
    may_join: true,
    may_speak: true,
    history: false,
};

/// What `account` may do in `room`, at `now_ms`.
pub async fn rules_of(
    source: &dyn ChatSource,
    hubs: &HubSessions,
    now_ms: u64,
    room: &RoomKind,
    account: &GlobalId,
) -> Result<RoomRules, ModuleError> {
    let settings = source.settings();
    let allowed = match room {
        RoomKind::Network => false,
        RoomKind::Region(region) => settings.dev_city_muc || hubs.may_join_region(account, region, now_ms),
        RoomKind::Hunt(_) => settings.dev_hunt_muc,
        RoomKind::Party(party) => match source.party_of(account).await? {
            Some(current) => instance_id(&current, IdKind::Party, &source.node().node).as_ref() == Some(party),
            None => false,
        },
    };
    Ok(if allowed { OPEN } else { REFUSED })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;
    use crate::ids::room_id_text;
    use crate::source::ChatSettings;
    use module_api::{NodeId, Region};

    fn region() -> Region {
        "local".parse().unwrap()
    }

    #[tokio::test]
    async fn a_jid_is_the_user_id_on_the_captured_domain() {
        let fake = Fake::default();
        let alice = fake.add("alice-uuid", "Alice");
        let jid = jid_of(&fake, &alice).await.unwrap();
        assert_eq!(jid.to_string(), "alice-uuid@prod.ol.epicgames.com");
        assert_eq!(
            (jid.local(), jid.domain(), jid.resource()),
            ("alice-uuid", XMPP_DOMAIN, None)
        );
        let unknown = GlobalId::new(IdKind::Account, &"local".parse::<NodeId>().unwrap());
        assert_eq!(jid_of(&fake, &unknown).await.unwrap_err().kind, ErrorKind::NotFound);
        // a userId that is not a JID part is refused rather than rewritten
        let odd = fake.add("has space", "Odd");
        assert_eq!(jid_of(&fake, &odd).await.unwrap_err().kind, ErrorKind::Invalid);
    }

    /// RoomService.test.ts and the Party MUC rule of `authorizeJoin`: the authenticated account decides.
    #[tokio::test]
    async fn only_a_partys_members_join_its_room() {
        let fake = Fake::default();
        let hubs = HubSessions::new();
        let (alice, bob) = (fake.add("alice", "Alice"), fake.add("bob", "Bob"));
        let party = GlobalId::new(IdKind::Party, &fake.identity.node);
        // the party area writes the id the client sees: room spelling, with the build suffix
        fake.put_party(&alice, &format!("{}_QnVpbGQ=", room_id_text(&party)));
        let room = RoomKind::Party(party.clone());
        assert_eq!(rules_of(&fake, &hubs, 0, &room, &alice).await.unwrap(), OPEN);
        assert_eq!(
            rules_of(&fake, &hubs, 0, &room, &bob).await.unwrap(),
            REFUSED,
            "not a member"
        );
        let other = RoomKind::Party(GlobalId::new(IdKind::Party, &fake.identity.node));
        assert_eq!(
            rules_of(&fake, &hubs, 0, &other, &alice).await.unwrap(),
            REFUSED,
            "another party"
        );
        fake.put_party(&bob, &party.to_string());
        assert_eq!(
            rules_of(&fake, &hubs, 0, &room, &bob).await.unwrap(),
            OPEN,
            "the full global id is read too"
        );
        // A source that cannot be asked refuses the join: the error reaches the plugin as it is
        fake.fail_party_lookup();
        assert_eq!(
            rules_of(&fake, &hubs, 0, &room, &alice).await.unwrap_err().kind,
            ErrorKind::Unavailable
        );
    }

    /// cityRooms.test.ts: "with the dev switch off, players sent to the hub chat in its room and others are refused",
    /// and RoomService.test.ts: "City MUC can be disabled outside development", "dev City MUC".
    #[tokio::test]
    async fn region_rooms_follow_the_hub_rule_and_the_dev_switch() {
        let mut fake = Fake::default();
        let hubs = HubSessions::new();
        let (alice, eve) = (fake.add("alice", "Alice"), fake.add("eve", "Eve"));
        let room = RoomKind::Region(region());
        hubs.record(&alice, "179cf2a8-0716-49f5-9027-6e4876ba0d76", &region(), 0);
        assert_eq!(rules_of(&fake, &hubs, 1, &room, &alice).await.unwrap(), OPEN);
        assert_eq!(rules_of(&fake, &hubs, 1, &room, &eve).await.unwrap(), REFUSED);
        let germany = RoomKind::Region("germany".parse().unwrap());
        assert_eq!(rules_of(&fake, &hubs, 1, &germany, &alice).await.unwrap(), REFUSED);
        fake.settings = ChatSettings {
            dev_city_muc: true,
            ..ChatSettings::default()
        };
        assert_eq!(
            rules_of(&fake, &hubs, 1, &room, &eve).await.unwrap(),
            OPEN,
            "the dev switch opens it to anyone"
        );
        assert_eq!(rules_of(&fake, &hubs, 1, &germany, &eve).await.unwrap(), OPEN);
    }

    #[tokio::test]
    async fn hunt_rooms_are_the_dev_switch_and_the_network_room_is_refused() {
        let mut fake = Fake::default();
        let hubs = HubSessions::new();
        let alice = fake.add("alice", "Alice");
        let hunt = RoomKind::Hunt(GlobalId::new(IdKind::Hunt, &fake.identity.node));
        assert_eq!(
            rules_of(&fake, &hubs, 0, &hunt, &alice).await.unwrap(),
            OPEN,
            "on unless switched off"
        );
        assert_eq!(
            rules_of(&fake, &hubs, 0, &RoomKind::Network, &alice).await.unwrap(),
            REFUSED
        );
        fake.settings.dev_hunt_muc = false;
        assert_eq!(rules_of(&fake, &hubs, 0, &hunt, &alice).await.unwrap(), REFUSED);
    }
}
