//! The types that cross the bridge (rows L6, L7): launcher-kit's `bridge::types` (RL8), the one source the
//! generated TypeScript comes from.

pub use launcher_kit::bridge::types::*;

pub use crate::core::i18n::Language;

use protocol::GameVersion;

/// The shell's names on the MIT graphics setting.
pub trait GraphicsPresetExt: Sized {
    /// Leave graphics to the in-game menu.
    #[cfg_attr(not(test), allow(dead_code, reason = "the game-config tests name it"))]
    const GAME_MENU: Self;
    const DEFAULT: Self;
    fn value(self) -> i8;
}

impl GraphicsPresetExt for GraphicsPreset {
    const GAME_MENU: Self = Self::UNSET;
    const DEFAULT: Self = Self::UNSET;

    fn value(self) -> i8 {
        self.get()
    }
}

/// The edition a version plays and its wire name.
pub trait GameExt: Sized {
    fn of(version: &GameVersion) -> Self;
    fn as_str(&self) -> &'static str;
}

impl GameExt for Game {
    fn of(version: &GameVersion) -> Self {
        if protocol::invite::is_revived(version) {
            Self::Legacy
        } else {
            Self::Reborn
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Reborn => "reborn",
            Self::Legacy => "legacy",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn guarded_outcomes_read_as_the_page_expects() {
        let info = GuardedActionInfo {
            id: "0123456789abcdef".into(),
            kind: "ban".into(),
            summary: "ban Ada: griefing".into(),
            state: GuardedState::Pending,
            mode: GuardedMode::Delay,
            initiated_by: "58ef5419".into(),
            needs_role: "moderator".into(),
            created_at: "2026-10-27T12:00:00Z".into(),
            release_at: Some("2026-10-27T12:10:00Z".into()),
            expires_at: "2026-10-29T12:10:00Z".into(),
            approved_by: None,
            canceled_by: None,
            ended_at: None,
            result: None,
        };
        let listed = serde_json::to_value(GuardedList::Ok(vec![info.clone()])).unwrap();
        assert_eq!(listed["ok"], true);
        assert_eq!(listed["actions"][0]["initiatedBy"], "58ef5419");
        assert_eq!(listed["actions"][0]["state"], "pending");
        assert_eq!(listed["actions"][0]["releaseAt"], "2026-10-27T12:10:00Z");
        let decided = serde_json::to_value(GuardedOutcome::Ok(info)).unwrap();
        assert_eq!(decided["action"]["id"], "0123456789abcdef");
        assert_eq!(
            serde_json::to_value(GuardedOutcome::Err(LauncherError::new(ErrorCode::GuardedNotFound))).unwrap(),
            json!({ "ok": false, "error": { "code": "guarded_not_found" } })
        );
        assert_eq!(
            serde_json::to_value(GuardedList::Err(LauncherError::new(ErrorCode::GuardedNotWaiting))).unwrap(),
            json!({ "ok": false, "error": { "code": "guarded_not_waiting" } })
        );
    }

    #[test]
    fn install_outcomes_read_as_the_page_expects() {
        let info = InstallInfo {
            version: "1.14.7".into(),
            state: InstallState::Downloading,
            folder: Some("D:/Games/Dauntless/1.14.7".into()),
            progress: None,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(InstallOutcome::Ok(info)).unwrap(),
            json!({ "ok": true, "install": {
                "version": "1.14.7", "state": "downloading", "folder": "D:/Games/Dauntless/1.14.7",
                "progress": null, "error": null,
            } })
        );
        assert_eq!(
            serde_json::to_value(InstallOutcome::Err(LauncherError::new(ErrorCode::EpicNotOwned))).unwrap(),
            json!({ "ok": false, "error": { "code": "epic_not_owned" } })
        );
        assert_eq!(serde_json::to_value(TaskKind::Install).unwrap(), json!("install"));
    }

    #[test]
    fn results_read_as_the_page_expects() {
        assert_eq!(serde_json::to_value(ActionResult::Ok).unwrap(), json!({ "ok": true }));
        assert_eq!(
            serde_json::to_value(ActionResult::err(ErrorCode::KeyInvalid)).unwrap(),
            json!({ "ok": false, "error": { "code": "key_invalid" } })
        );
        let check = InviteCheck::Ok {
            certificate_changed: true,
            previous_fingerprint: None,
            moved_from: Some("100.64.0.1:61000".into()),
        };
        assert_eq!(
            serde_json::to_value(check).unwrap(),
            json!({ "ok": true, "certificateChanged": true, "previousFingerprint": null, "movedFrom": "100.64.0.1:61000" })
        );
        let registered = RegisterOutcome::Ok {
            username: "Slayer".into(),
        };
        assert_eq!(
            serde_json::to_value(registered).unwrap(),
            json!({ "ok": true, "username": "Slayer" })
        );
        let pinged = RegionPing::Ok {
            regions: vec![
                RegionRtt {
                    region: "uk".into(),
                    rtt_ms: Some(18.4),
                    samples: 5,
                    problem: None,
                },
                RegionRtt {
                    region: "oregon".into(),
                    rtt_ms: None,
                    samples: 0,
                    problem: Some(RegionProblem::CertMismatch),
                },
            ],
            reported: true,
        };
        assert_eq!(
            serde_json::to_value(pinged).unwrap(),
            json!({ "ok": true, "regions": [
                { "region": "uk", "rttMs": 18.4, "samples": 5, "problem": null },
                { "region": "oregon", "rttMs": null, "samples": 0, "problem": "cert_mismatch" },
            ], "reported": true })
        );
        assert_eq!(
            serde_json::to_value(RegionPing::Err(LauncherError::new(ErrorCode::ServerUnreachable))).unwrap(),
            json!({ "ok": false, "error": { "code": "server_unreachable" } })
        );
        let listed = GameVersionList {
            versions: vec![
                GameVersionEntry {
                    version: "1.14.7".into(),
                    display_name: "Rustic Isles 1.14.7".into(),
                    origin: GameVersionOrigin::Launcher,
                    install: GameVersionInstall::Epic,
                    epic: Some(EpicVersionInfo {
                        build_version: "1.14.7.647472".into(),
                        manifest: EpicManifestState::Ready,
                        download_bytes: Some(15_985_792_608),
                        install_bytes: Some(16_521_629_423),
                    }),
                },
                GameVersionEntry {
                    version: "1.4.4".into(),
                    display_name: "Dauntless Revived".into(),
                    origin: GameVersionOrigin::Server,
                    install: GameVersionInstall::Locate,
                    epic: None,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(listed).unwrap(),
            json!({ "ok": true, "versions": [
                { "version": "1.14.7", "displayName": "Rustic Isles 1.14.7", "origin": "launcher", "install": "epic",
                  "epic": { "buildVersion": "1.14.7.647472", "manifest": "ready", "downloadBytes": 15_985_792_608_u64,
                            "installBytes": 16_521_629_423_u64 } },
                { "version": "1.4.4", "displayName": "Dauntless Revived", "origin": "server", "install": "locate",
                  "epic": null },
            ] })
        );
        let epic = EpicView {
            signed_in: true,
            display_name: Some("Slayer".into()),
            signing_in: false,
        };
        assert_eq!(
            serde_json::to_value(epic).unwrap(),
            json!({ "signedIn": true, "displayName": "Slayer", "signingIn": false })
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::EpicCodeInvalid).unwrap(),
            json!("epic_code_invalid")
        );
        let error = LauncherError::with_detail(ErrorCode::HostFailed, "setup_form");
        assert_eq!(
            serde_json::to_value(RegisterOutcome::Err(error)).unwrap(),
            json!({ "ok": false, "error": { "code": "host_failed", "detail": "setup_form" } })
        );
    }

    #[test]
    fn module_outcomes_read_as_the_page_expects() {
        let error = LauncherError::with_detail(ErrorCode::ModuleBadState, "loaded");
        assert_eq!(
            serde_json::to_value(ModuleOutcome::Err(error.clone())).unwrap(),
            json!({ "ok": false, "error": { "code": "module_bad_state", "detail": "loaded" } })
        );
        assert_eq!(
            serde_json::to_value(ModuleListOutcome::Ok(Vec::new())).unwrap(),
            json!({ "ok": true, "modules": [] })
        );
        assert_eq!(
            serde_json::to_value(ModuleCommandOutcome::Ok {
                text: "hi".into(),
                data: Some("{\"a\":1}".into()),
            })
            .unwrap(),
            json!({ "ok": true, "text": "hi", "data": "{\"a\":1}" })
        );
        assert_eq!(
            serde_json::to_value(ModuleCommandOutcome::Ok {
                text: String::new(),
                data: None,
            })
            .unwrap(),
            json!({ "ok": true, "text": "", "data": null })
        );
        assert_eq!(
            serde_json::to_value([
                ModuleSettingValue::Flag(true),
                ModuleSettingValue::Number(2.5),
                ModuleSettingValue::Text("x".into())
            ])
            .unwrap(),
            json!([true, 2.5, "x"])
        );
        assert_eq!(
            serde_json::to_value(ModuleRunState::NotLoaded).unwrap(),
            json!("not_loaded")
        );
        assert_eq!(
            serde_json::to_value(ModuleOrigin::CompiledIn).unwrap(),
            json!("compiled_in")
        );
    }

    #[test]
    fn names_match_the_typescript_ones() {
        assert_eq!(
            serde_json::to_value(ExternalTarget::Dumper7Zip).unwrap(),
            json!("dumper7_zip")
        );
        assert_eq!(serde_json::to_value(ExternalTarget::Dumper7).unwrap(), json!("dumper7"));
        assert_eq!(
            serde_json::to_value(ExternalTarget::ProjectDocsSdk).unwrap(),
            json!("project_docs_sdk")
        );
        assert_eq!(serde_json::to_value(GameLanguage::PtBr).unwrap(), json!("pt-BR"));
        assert_eq!(
            serde_json::to_value(ErrorCode::RuntimeDifferentOnServer).unwrap(),
            json!("runtime_different_on_server")
        );
        assert_eq!(
            serde_json::to_value(NoticeCode::RuntimeInstalledRestart).unwrap(),
            json!("runtime_installed_restart")
        );
        assert_eq!(
            serde_json::to_value(HostFolder::GamedataPacks).unwrap(),
            json!("gamedata_packs")
        );
        assert_eq!(
            serde_json::to_value(HostUpdateState::UpToDate).unwrap(),
            json!("up_to_date")
        );
        let invite = HostInvite {
            link: "x".into(),
            for_name: None,
        };
        assert_eq!(
            serde_json::to_value(invite).unwrap(),
            json!({ "link": "x", "for": null })
        );
    }

    #[test]
    fn a_settings_patch_is_checked_whole() {
        let patch: SettingsPatch =
            serde_json::from_value(json!({ "runtimeProfile": "development", "graphics": -1, "hostTools": true }))
                .unwrap();
        assert_eq!(patch.runtime_profile, Some(RuntimeProfile::Development));
        assert_eq!(patch.graphics, Some(GraphicsPreset::GAME_MENU));
        assert_eq!(patch.host_tools, Some(true));
        for bad in [
            json!({ "graphics": 5 }),
            json!({ "unexpected": true }),
            json!({ "huntRegion": "us" }),
            json!({ "exposure": "basic" }),
            json!({ "language": "xx" }),
            json!({ "windowed": "yes" }),
            json!({ "runtimeProfile": "debug" }),
            json!([]),
        ] {
            assert!(serde_json::from_value::<SettingsPatch>(bad.clone()).is_err(), "{bad}");
        }
    }
}
