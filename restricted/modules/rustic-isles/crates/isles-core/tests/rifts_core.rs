//! `rifts_seam::IslesCore` as matchmaking answers it: what rustic-rifts calls back into isles.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

mod support;

use std::sync::Arc;

use isles_core::areas::matchmaking::rifts_core::MatchmakingCore;
use isles_core::areas::matchmaking::state::{MatchmakingResult, MatchmakingState, Source};
use isles_core::areas::matchmaking::testkit::FakeDeploy;
use rifts_seam::{AccountId, Candidate, IslesCore, Json, NoCore, SeamError};

fn core(target: &support::Target, state: Arc<MatchmakingState>) -> MatchmakingCore {
    MatchmakingCore {
        store: Some(target.store.clone()),
        node: target.node.node.clone(),
        state,
        fallback: Arc::new(NoCore),
    }
}

fn candidate(result: &MatchmakingResult) -> Candidate {
    Candidate(Json(result.to_json().to_string()))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_realms_publish_and_end_travel_and_touch_the_game_session() {
    let target = support::targets().remove(0);
    let state = Arc::new(MatchmakingState::with_deploy(FakeDeploy::online("h", 1)));
    let core = core(&target, state.clone());

    let mut hub = MatchmakingResult::matching("ShatteredIsles_TrainingDojo", "c1".into());
    hub.ready = true;
    hub.host = "127.0.0.1".into();
    hub.port = 56001;
    hub.source = Some(Source::RealmHub);
    core.set_candidate(AccountId("alice".into()), candidate(&hub)).unwrap();
    assert_eq!(state.candidate("alice"), Some(hub));
    assert!(
        core.set_candidate(AccountId("alice".into()), Candidate(Json("[1]".into())))
            .is_err()
    );

    assert!(
        core.cancel_travel(
            "ShatteredIsles_TrainingDojo",
            &[AccountId("alice".into())],
            "REALM_FULL"
        )
        .unwrap()
    );
    let canceled = state.candidate("alice").unwrap();
    assert!(canceled.canceled && !canceled.ready);
    assert_eq!(canceled.status_reason.as_deref(), Some("REALM_FULL"));

    assert!(core.dedicated_servers_configured());
    core.touch_game_session(AccountId("alice".into())).await.unwrap();
    let now = state.clock.now_ms();
    assert!(state.presence.is_recently_active("alice", 75_000, now));
    assert!(!state.presence.is_recently_active("bob", 75_000, now));
    let unconfigured = MatchmakingCore {
        state: Arc::new(MatchmakingState::default()),
        ..core
    };
    assert!(!unconfigured.dedicated_servers_configured());
    // accounts, Guard and operations are other areas': the fallback answers
    let error = unconfigured.display_name(AccountId("alice".into())).await.unwrap_err();
    assert!(matches!(error, SeamError::Unimplemented(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_realms_read_and_create_parties_by_their_global_id() {
    for target in support::targets() {
        target.account("alice").await;
        let core = core(&target, Arc::default());
        let created = core.get_or_create_party(AccountId("alice".into())).await.unwrap();
        assert_eq!(created.leader, AccountId("alice".into()), "{}", target.engine);
        assert_eq!(created.members, [AccountId("alice".into())]);
        assert_eq!(created.revision, 1);
        let again = core.get_or_create_party(AccountId("alice".into())).await.unwrap();
        assert_eq!(again.id, created.id);
        assert_eq!(core.party(created.id.clone()).await.unwrap(), Some(created));
        let other: rifts_seam::PartyId = "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap();
        assert_eq!(core.party(other).await.unwrap(), None);
        assert!(core.get_or_create_party(AccountId("nobody".into())).await.is_err());
    }
}
