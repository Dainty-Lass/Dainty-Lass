// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The route table in Express's registration order, which is the match order (core spec Appendix A). A new
//! upstream route goes in at its registration position, never at the end.

use super::Group::*;
use super::Method::*;
use super::RouteDef;
use super::Service::{Backend as B, Director as D};
use super::Side::*;
use super::When::{Admin, Always, Gauntlet, Rifts as RiftsOnly};

const fn r(
    service: super::Service,
    method: super::Method,
    path: &'static str,
    group: super::Group,
    side: super::Side,
    when: super::When,
) -> RouteDef {
    RouteDef {
        service,
        method,
        path,
        group,
        side,
        when,
    }
}

#[rustfmt::skip]
pub(super) static TABLE: [RouteDef; 299] = [
    r(B, Get, r"/", LauncherApi, Isles, Always), // B1
    r(B, Get, r"/health/live", LauncherApi, Isles, Always), // B2
    r(B, Get, r"/health/ready", LauncherApi, Isles, Always), // B3
    r(B, Get, r"/health/release-gates", LauncherApi, Isles, Admin), // B4
    r(B, Get, r"/internal/v1/metrics", LauncherApi, Isles, Always), // B5
    r(B, Get, r"/features/platform/win", Auth, Isles, Always), // B6
    r(B, Get, r"/account/link/epic/:AccId", Auth, Isles, Always), // B7
    r(B, Post, r"/login", Auth, Isles, Always), // B8
    r(B, Get, r"/accountinfo", Auth, Isles, Always), // B9
    r(B, Get, r"/tags", Auth, Isles, Always), // B10
    r(B, Put, r"/gamesession/epic", Auth, Isles, Always), // B11
    r(B, Put, r"/gamesession/epiceos", Auth, Isles, Always), // B12
    r(B, Post, r"/accountinfo/public", Auth, Isles, Always), // B13
    r(B, Post, r"/migration/trigger", Auth, Isles, Always), // B14
    r(B, Get, r"/migration/status", Auth, Isles, Always), // B15
    r(B, Get, r"/isbanned", Auth, Isles, Always), // B16
    r(B, Post, r"/account/api/oauth/token", Auth, Isles, Always), // B17
    r(B, Get, r"/account/api/oauth/verify", Auth, Isles, Always), // B18
    r(B, Get, r"/account/api/public/account/:AccId", Auth, Isles, Always), // B19
    r(B, Get, r"/account/api/public/account/:AccId/externalAuths", Auth, Isles, Always), // B20
    r(B, Get, r"/account/api/public/account/displayName/:displayName", Auth, Isles, Always), // B21
    r(B, Delete, r"/account/api/oauth/sessions/kill", Auth, Isles, Always), // B22
    r(B, Delete, r"/account/api/oauth/sessions/kill/:AuthToken", Auth, Isles, Always), // B23
    r(B, Get, r"/account/api/public/account", Auth, Isles, Always), // B24
    r(B, Get, r"/dauntless-status", LauncherApi, Isles, Always), // B25
    r(B, Post, r"/heartbeat", GameData, Isles, Always), // B26
    r(B, Post, r"/event", GameData, Isles, Always), // B27
    r(B, Get, r"/crash/CrashReporter/Ping", GameData, Isles, Always), // B28
    r(B, Post, r"/crash/CrashReporter/CheckReport", GameData, Isles, Always), // B29
    r(B, Get, r"/survey/config", GameData, Isles, Always), // B30
    r(B, Post, r"/account/migrate", GameData, Isles, Always), // B31
    r(B, Post, r"/profile/update", GameData, Isles, Always), // B32
    r(B, Post, r"/trials/leaderboards/all", GameData, Isles, Always), // B33
    r(B, Post, r"/trials/leaderboards/solo/individual", GameData, Isles, Always), // B34
    r(B, Get, r"/vivox/login", Chat, Isles, Always), // B35
    r(B, Post, r"/motd/", GameData, Isles, Always), // B36
    r(B, Get, r"/motd/trigger", GameData, Isles, Always), // B37
    r(B, Get, r"/entitlementsv2", BountiesStore, Isles, Always), // B38
    r(B, Post, r"/entitlementv2/:userId", BountiesStore, Isles, Always), // B39
    r(B, Get, r"/playertreatments", GameData, Isles, Always), // B40
    r(B, Get, r"/playertreatments/", GameData, Isles, Always), // B41
    r(B, Get, r"/playertreatments/:userId", GameData, Isles, Always), // B42
    r(B, Get, r"/escalation/:escalationSeason/:userId", Progression, Isles, Always), // B43
    r(B, Post, r"/escalation/:escalationSeason/:userId", Progression, Isles, Always), // B44
    r(B, Get, r"/eventstats/", GameData, Isles, Always), // B45
    r(B, Get, r"/progression/config", GameData, Isles, Always), // B46
    r(B, Get, r"/config", GameData, Isles, Gauntlet), // B47
    r(B, Get, r"/^\/progression\/([^/]*)\/([^/]+)$/i", GameData, Isles, Gauntlet), // B48
    r(B, Get, r"/^\/rewards\/personal\/([^/]*)\/([^/]+)$/i", GameData, Isles, Gauntlet), // B49
    r(B, Get, r"/^\/rewards\/guild\/([^/]*)\/([^/]+)$/i", GameData, Isles, Gauntlet), // B50
    r(B, Get, r"/^\/leaderboard\/get_leaderboard\//i", GameData, Isles, Gauntlet), // B51
    r(B, Get, r"/huntpass/:userId", Progression, Isles, Always), // B52
    r(B, Get, r"/cooldown/:userId", BountiesStore, Isles, Always), // B53
    r(B, Put, r"/cooldown/batch/:userId", BountiesStore, Isles, Always), // B54
    r(B, Put, r"/cooldown/:userId/:cooldownId", BountiesStore, Isles, Always), // B55
    r(B, Post, r"/cooldown/:userId/:cooldownId", BountiesStore, Isles, Always), // B56
    r(B, Get, r"/bounty/game-data", BountiesStore, Isles, Always), // B57
    r(B, Get, r"/bounty/:userId", BountiesStore, Isles, Always), // B58
    r(B, Post, r"/bounty/:userId", BountiesStore, Isles, Always), // B59
    r(B, Post, r"/bounty/delete/:userId", BountiesStore, Isles, Always), // B60
    r(B, Delete, r"/bounty/delete/:userId", BountiesStore, Isles, Always), // B61
    r(B, Get, r"/all/", GameData, Isles, Always), // B62
    r(B, Get, r"/patchnotes/:language/:gameversion", GameData, Isles, Always), // B63
    r(B, Get, r"/maintenance/status", LauncherApi, Isles, Always), // B64
    r(B, Get, r"/character", CharactersLoadouts, Isles, Always), // B65
    r(B, Put, r"/character", CharactersLoadouts, Isles, Always), // B66
    r(B, Post, r"/character", CharactersLoadouts, Isles, Always), // B67
    r(B, Post, r"/^\/inventory\/([^/]*)\/(.+)$/", InventoryReforge, Isles, Always), // B68
    r(B, Post, r"/inventory/:characterId/:changeList", InventoryReforge, Isles, Always), // B69
    r(B, Get, r"/inventory/:userId/:characterId", InventoryReforge, Isles, Always), // B70
    r(B, Get, r"/inventory/:userId/", InventoryReforge, Isles, Always), // B71
    r(B, Post, r"/inventory", InventoryReforge, Isles, Always), // B72
    r(B, Post, r"/inventory/instanceditem", InventoryReforge, Isles, Always), // B73
    r(B, Post, r"/reconcile", BountiesStore, Isles, Always), // B74
    r(B, Get, r"/creator", BountiesStore, Isles, Always), // B75
    r(B, Get, r"/balance", BountiesStore, Isles, Always), // B76
    r(B, Get, r"/product/skus/public", BountiesStore, Isles, Always), // B77
    r(B, Get, r"/token/:currency/:catalogId", BountiesStore, Isles, Always), // B78
    r(B, Post, r"/notification/:currency", BountiesStore, Isles, Always), // B79
    r(B, Get, r"/guild/invite/player", Chat, Isles, Always), // B80
    r(B, Get, r"/guild", Chat, Isles, Always), // B81
    r(B, Get, r"/game_tuning/seasonal_event_schedule", GameData, Isles, Always), // B82
    r(B, Get, r"/game_tuning/huntpass_xp_config", GameData, Isles, Always), // B83
    r(B, Get, r"/game_tuning/island_content_config", GameData, Isles, Always), // B84
    r(B, Get, r"/game_tuning/bounty_game_data", GameData, Isles, Always), // B85
    r(B, Get, r"/game_tuning/bounty_game_data_daily", GameData, Isles, Always), // B86
    r(B, Get, r"/game_tuning/bounty_game_data_weekly", GameData, Isles, Always), // B87
    r(B, Post, r"/candidate/player/register", MatchmakingParties, Isles, Always), // B88
    r(B, Post, r"/candidate/player/alive", MatchmakingParties, Isles, Always), // B89
    r(B, Get, r"/candidate/regions", MatchmakingParties, Isles, Always), // B90
    r(B, Post, r"/key/generate", Auth, Isles, Always), // B91
    r(B, Post, r"/key/consume", Auth, Isles, Always), // B92
    r(B, Get, r"/candidate/status", MatchmakingParties, Isles, Always), // B93
    r(B, Post, r"/candidate/join", MatchmakingParties, Isles, Always), // B94
    r(B, Delete, r"/candidate/leave", MatchmakingParties, Isles, Always), // B95
    r(B, Post, r"/candidate/leave", MatchmakingParties, Isles, Always), // B96
    r(B, Put, r"/candidate/leave", MatchmakingParties, Isles, Always), // B97
    r(B, Delete, r"/candidate", MatchmakingParties, Isles, Always), // B98
    r(B, Delete, r"/candidate/:candidateId", MatchmakingParties, Isles, Always), // B99
    r(B, Get, r"/QoS", MatchmakingParties, Isles, Always), // B100
    r(B, Post, r"/party", MatchmakingParties, Isles, Always), // B101
    r(B, Put, r"/party/invite", MatchmakingParties, Isles, Always), // B102
    r(B, Get, r"/party/invites", MatchmakingParties, Isles, Always), // B103
    r(B, Put, r"/party/invite/accept/:inviterId", MatchmakingParties, Isles, Always), // B104
    r(B, Delete, r"/party/member", MatchmakingParties, Isles, Always), // B105
    r(B, Delete, r"/party/member/:targetId", MatchmakingParties, Isles, Always), // B106
    r(B, Put, r"/party/member/promote/:targetId", MatchmakingParties, Isles, Always), // B107
    r(B, Get, r"/encountered-content/:characterId/:contentType", Progression, Isles, Always), // B108
    r(B, Post, r"/encountered-content/query/:characterId", Progression, Isles, Always), // B109
    r(B, Post, r"/encountered-content/query", Progression, Isles, Always), // B110
    r(B, Post, r"/encountered-content/:characterId/query", Progression, Isles, Always), // B111
    r(B, Post, r"/encountered-content/:characterId", Progression, Isles, Always), // B112
    r(B, Post, r"/encountered-content", Progression, Isles, Always), // B113
    r(B, Post, r"/encountered-content/", Progression, Isles, Always), // B114
    r(B, Get, r"/progression/objectives/:userId", Progression, Isles, Always), // B115
    r(B, Get, r"/breadcrumbs/", Progression, Isles, Always), // B116
    r(B, Get, r"/breadcrumbs/:characterId", Progression, Isles, Always), // B117
    r(B, Post, r"/breadcrumbs/:characterId", Progression, Isles, Always), // B118
    r(B, Post, r"/progression/:userId/:progressionId/:amount", Progression, Isles, Always), // B119
    r(B, Post, r"/progression/:userId", Progression, Isles, Always), // B120
    r(B, Post, r"/progression/:userId/:progressionId/:rank/confirm/public", Progression, Isles, Always), // B121
    r(B, Get, r"/progression/:userId", Progression, Isles, Always), // B122
    r(B, Get, r"/pjm/:userId", Progression, Isles, Always), // B123
    r(B, Get, r"/weapon_tracker/:userId/:itemId", Progression, Isles, Always), // B124
    r(B, Post, r"/pjm/:userId", Progression, Isles, Always), // B125
    r(B, Get, r"/pjm", Progression, Isles, Always), // B126
    r(B, Get, r"/progression/tracked_objectives/:phxAccountId", Progression, Isles, Always), // B127
    r(B, Post, r"/progression/tracked_objectives/:phxAccountId", Progression, Isles, Always), // B128
    r(B, Get, r"/progression/:userId/:progressionId", Progression, Isles, Always), // B129
    r(B, Delete, r"/progression/:userId/:progressionId", InventoryReforge, Isles, Always), // B130
    r(B, Post, r"/progression/purchase/:userId/:progressionId/:amount", InventoryReforge, Isles, Always), // B131
    r(B, Get, r"/loadout/:userId//all", CharactersLoadouts, Isles, Always), // B132
    r(B, Get, r"/loadout/:userId/:characterId/all", CharactersLoadouts, Isles, Always), // B133
    r(B, Post, r"/loadout/:userId/:characterId/unlock/:numSlots", CharactersLoadouts, Isles, Always), // B134
    r(B, Post, r"/loadout/:userId/:characterId/active/:index", CharactersLoadouts, Isles, Always), // B135
    r(B, Post, r"/loadout/:userId/:characterId/:index", CharactersLoadouts, Isles, Always), // B136
    r(B, Get, r"/friends/api/public/friends/:accountId", Chat, Isles, Always), // B137
    r(B, Post, r"/friends/api/public/friends/:accountId/:friendId", Chat, Isles, Always), // B138
    r(B, Delete, r"/friends/api/public/friends/:accountId/:friendId", Chat, Isles, Always), // B139
    r(B, Get, r"/friends/api/public/blocklist/:accountId", Chat, Isles, Always), // B140
    r(B, Get, r"/friends/api/public/list/:namespace/:accountId/recentPlayers", Chat, Isles, Always), // B141
    r(B, Get, r"/friends/api/v1/:accountId/settings", Chat, Isles, Always), // B142
    r(B, Post, r"/slayerlink/availability", Chat, Isles, Always), // B143
    r(B, Get, r"/slayerlink/status_good", Chat, Isles, Always), // B144
    r(B, Post, r"/dainty/v1/save-reset", Chat, Isles, Always), // B144a: the help screen's save reset, not in Node
    r(B, Post, r"/launcher/v1/auth/register", Auth, Isles, Always), // B145
    r(B, Post, r"/launcher/v1/auth/login", Auth, Isles, Always), // B146
    r(B, Post, r"/launcher/v1/auth/refresh", Auth, Isles, Always), // B147
    r(B, Post, r"/launcher/v1/auth/logout", Auth, Isles, Always), // B148
    r(B, Post, r"/launcher/v1/auth/logout-all", Auth, Isles, Always), // B149
    r(B, Post, r"/launcher/v1/auth/discord/start", Auth, Isles, Always), // B150
    r(B, Get, r"/launcher/v1/auth/discord/callback", Auth, Isles, Always), // B151
    r(B, Post, r"/launcher/v1/auth/discord/complete", Auth, Isles, Always), // B152
    r(B, Post, r"/launcher/v1/auth/email/verify", Auth, Isles, Always), // B153
    r(B, Post, r"/launcher/v1/auth/email/resend", Auth, Isles, Always), // B154
    r(B, Post, r"/launcher/v1/auth/password/forgot", Auth, Isles, Always), // B155
    r(B, Post, r"/launcher/v1/auth/password/reset", Auth, Isles, Always), // B156
    r(B, Post, r"/launcher/v1/account/discord/link/start", Auth, Isles, Always), // B157
    r(B, Post, r"/launcher/v1/account/discord/unlink/start", Auth, Isles, Always), // B158
    r(B, Post, r"/launcher/v1/account/discord/unlink", Auth, Isles, Always), // B159
    r(B, Get, r"/launcher/v1/me", Auth, Isles, Always), // B160
    r(B, Get, r"/launcher/v1/username", Auth, Isles, Always), // B161
    r(B, Post, r"/launcher/v1/username", Auth, Isles, Always), // B162
    r(B, Post, r"/launcher/v1/game-sessions", Auth, Isles, Always), // B163
    r(B, Get, r"/launcher/v1/policy", Auth, Isles, Always), // B164
    r(B, Get, r"/launcher/v1/status", Auth, Isles, Always), // B165
    r(B, Get, r"/launcher/v1/legal", Auth, Isles, Always), // B166
    r(B, Get, r"/launcher/v1/notices", Auth, Isles, Always), // B167
    r(B, Get, r"/launcher/v1/account/consent", Auth, Isles, Always), // B168
    r(B, Post, r"/launcher/v1/account/consent", Auth, Isles, Always), // B169
    r(B, Get, r"/launcher/v1/account/deletion", Auth, Isles, Always), // B170
    r(B, Post, r"/launcher/v1/account/deletion", Auth, Isles, Always), // B171
    r(B, Delete, r"/launcher/v1/account/deletion", Auth, Isles, Always), // B172
    r(B, Post, r"/dainty/api/Register", LauncherApi, Isles, Always), // B173
    r(B, Get, r"/dainty/api/GetUserInfo", LauncherApi, Isles, Always), // B174
    r(B, Get, r"/dainty/api/ServerStatus", LauncherApi, Isles, Always), // B175
    r(B, Get, r"/dainty/api/RegistrationStatus", LauncherApi, Isles, Always), // B176
    r(B, Get, r"/dainty/api/Playtime", LauncherApi, Isles, Always), // no Node route: the key holder's playtime
    r(B, Post, r"/dainty/api/LoginQueue", LauncherApi, Isles, Always), // B176a: the login queue, not in Node
    r(B, Delete, r"/dainty/api/LoginQueue", LauncherApi, Isles, Always), // B176b
    r(B, Post, r"/dainty/api/CreateInvite", LauncherApi, Isles, Always), // B177
    r(B, Get, r"/dainty/api/InviteCodes", LauncherApi, Isles, Always), // B178
    r(B, Delete, r"/dainty/api/InviteCode/:code", LauncherApi, Isles, Always), // B179
    r(B, Post, r"/dainty/api/SetAccountStatus", LauncherApi, Isles, Always), // B180
    r(B, Post, r"/dainty/api/Kick", LauncherApi, Isles, Always), // B181
    r(B, Get, r"/launcher/v1/runtime/:target/:channel/:platform", LauncherApi, Isles, Always), // B182
    r(B, Get, r"/launcher/v1/runtime/:channel/:platform", LauncherApi, Isles, Always), // B183
    r(B, Get, r"/launcher/v1/runtime/:target/:channel/:platform/download", LauncherApi, Isles, Always), // B184
    r(B, Get, r"/launcher/v1/runtime/:channel/:platform/download", LauncherApi, Isles, Always), // B185
    r(B, Get, r"/launcher/v1/guard-manifests/:channel/:platform", Guard, Rifts, Always), // B186
    r(B, Post, r"/launcher/v1/admin/guard-manifests/:channel/:platform", Guard, Rifts, Always), // B187
    r(B, Post, r"/launcher/v1/admin/updates/runtime/:target/:channel/:platform", LauncherApi, Isles, Always), // B188
    r(B, Post, r"/launcher/v1/admin/updates/runtime/:target/:channel/:platform/extra", LauncherApi, Isles, Always), // B189
    r(B, Get, r"/launcher/v1/runtime/:target/:channel/:platform/extra/:filename", LauncherApi, Isles, Always), // B190
    r(B, Get, r"/launcher/v1/admin/updates/serving", LauncherApi, Isles, Always), // B191
    r(B, Post, r"/launcher/v1/admin/updates/serving", LauncherApi, Isles, Always), // B192
    r(B, Get, r"/launcher/v1/updates/:target/:arch/download", LauncherApi, Isles, Always), // B193
    r(B, Get, r"/launcher/v1/updates/:target/:arch/files/:version/:file", LauncherApi, Isles, Always), // B194
    r(B, Get, r"/launcher/v1/updates/:target/:arch/:currentVersion", LauncherApi, Isles, Always), // B195
    r(B, Put, r"/launcher/v1/logs/sessions/:launchSessionId/:fileName", LauncherApi, Isles, Always), // B196
    r(B, Get, r"/admin/v1/launcher-logs", LauncherApi, Isles, Admin), // B197
    r(B, Get, r"/admin/v1/launcher-logs/:userId/:launchSessionId/:fileName", LauncherApi, Isles, Admin), // B198
    r(B, Post, r"/admin/v1/auth/login", LauncherApi, Isles, Admin), // B199
    r(B, Post, r"/admin/v1/auth/logout", LauncherApi, Isles, Admin), // B200
    r(B, Get, r"/admin/v1/auth/discord/start", LauncherApi, Isles, Admin), // B201
    r(B, Get, r"/admin/v1/auth/discord/callback", LauncherApi, Isles, Admin), // B202
    r(B, Get, r"/admin/v1/me", LauncherApi, Isles, Admin), // B203
    r(B, Get, r"/admin/v1/online-players", LauncherApi, Isles, Admin), // B204
    r(B, Get, r"/admin/v1/players", LauncherApi, Isles, Admin), // B205
    r(B, Patch, r"/admin/v1/players/:userId/access", LauncherApi, Isles, Admin), // B206
    r(B, Patch, r"/admin/v1/players/:userId/roles", LauncherApi, Isles, Admin), // B207
    r(B, Delete, r"/admin/v1/players/:userId", LauncherApi, Isles, Admin), // B208
    r(B, Get, r"/admin/v1/audit", LauncherApi, Isles, Admin), // B209
    r(B, Get, r"/admin/v1/operations/overview", LauncherApi, Isles, Admin), // B210
    r(B, Patch, r"/admin/v1/operations/policy", LauncherApi, Isles, Admin), // B211
    r(B, Get, r"/admin/v1/launcher-notices", LauncherApi, Isles, Admin), // B212
    r(B, Put, r"/admin/v1/launcher-notices", LauncherApi, Isles, Admin), // B213
    r(B, Get, r"/admin/v1/guard/sessions", Guard, Rifts, Admin), // B214
    r(B, Get, r"/admin/v1/guard/sessions/:guardSessionId", Guard, Rifts, Admin), // B215
    r(B, Get, r"/admin/v1/guard/findings", Guard, Rifts, Admin), // B216
    r(B, Get, r"/admin/v1/activity/players/:userId", LauncherApi, Isles, Admin), // B217
    r(B, Get, r"/admin/v1/activity/overview", LauncherApi, Isles, Admin), // B218
    r(B, Get, r"/admin/v1/activity/flags", LauncherApi, Isles, Admin), // B219
    r(B, Patch, r"/admin/v1/activity/flags/:id", LauncherApi, Isles, Admin), // B220
    r(B, Post, r"/admin/v1/players/:userId/tester-max", LauncherApi, Isles, Admin), // B221
    r(B, Get, r"/admin/v1/grant-catalog", LauncherApi, Isles, Admin), // B221a: staff grants, not in Node
    r(B, Post, r"/admin/v1/players/:userId/grants", LauncherApi, Isles, Admin), // B221b
    r(B, Get, r"/admin/v1/players/:userId/save-snapshots", LauncherApi, Isles, Admin), // B221c: /reset snapshots
    r(B, Post, r"/admin/v1/players/:userId/save-snapshots/:snapshotId/restore", LauncherApi, Isles, Admin), // B221d
    r(B, Get, r"/admin/v1/player-logs", LauncherApi, Isles, Admin), // B222
    r(B, Get, r"/admin/v1/player-logs/:userId/:launchSessionId/:fileName", LauncherApi, Isles, Admin), // B223
    r(B, Delete, r"/admin/v1/player-logs/:userId/:launchSessionId", LauncherApi, Isles, Admin), // B224
    r(B, Post, r"/launcher/v1/guard/sessions", Guard, Rifts, Always), // B225
    r(B, Post, r"/launcher/v1/guard/heartbeat", Guard, Rifts, Always), // B226
    r(B, Get, r"/launcher/v1/guard/sessions/:guardSessionId", Guard, Rifts, Always), // B227
    r(B, Post, r"/rustic/v1/hunts/:sessionId/tickets/self", P2p, Rifts, RiftsOnly), // B228
    r(B, Post, r"/rustic/v1/hunts/:sessionId/host-ready", P2p, Rifts, RiftsOnly), // B229
    r(B, Post, r"/rustic/v1/hunts/:sessionId/event-batches", Guard, Rifts, RiftsOnly), // B230
    r(B, Post, r"/rustic/v1/hunts/:sessionId/completion", P2p, Rifts, RiftsOnly), // B231
    r(B, Post, r"/rustic/v1/hunts/:sessionId/abort", P2p, Rifts, RiftsOnly), // B232
    r(B, Post, r"/rustic/v1/hunts/:sessionId/end", P2p, Rifts, RiftsOnly), // B233
    r(B, Get, r"/rustic/v1/hunts/:sessionId/status", P2p, Rifts, RiftsOnly), // B234
    r(B, Get, r"/rustic/v1/hunts/:sessionId/reward/self", P2p, Rifts, RiftsOnly), // B235
    r(B, Post, r"/internal/v1/hunt-guard/shadow-comparisons", Guard, Rifts, RiftsOnly), // B236
    r(B, Post, r"/rustic/v1/hunts/:sessionId/lease", P2p, Rifts, RiftsOnly), // B237
    r(B, Post, r"/rustic/v1/hunts/:sessionId/tickets/consume", P2p, Rifts, RiftsOnly), // B238
    r(B, Post, r"/rustic/v1/hunts/:sessionId/peers/:admissionId/disconnect", P2p, Rifts, RiftsOnly), // B239
    r(B, Patch, r"/admin/v1/operations/p2p-emergency-stop", P2p, Rifts, RiftsOnly), // B240
    r(B, Patch, r"/admin/v1/operations/reward-rollout", P2p, Rifts, RiftsOnly), // B241
    r(B, Patch, r"/admin/v1/operations/coop-transport", P2p, Rifts, RiftsOnly), // B242
    r(B, Patch, r"/admin/v1/operations/alerts/:alertId/ack", P2p, Rifts, RiftsOnly), // B243
    r(B, Post, r"/launcher/v1/p2p/hunts/:sessionId/host-bootstrap", P2p, Rifts, RiftsOnly), // B244
    r(B, Post, r"/launcher/v1/p2p/hunts/:sessionId/join-bootstrap", P2p, Rifts, RiftsOnly), // B245
    r(B, Post, r"/launcher/v1/p2p/hunts/:sessionId/route-ready", P2p, Rifts, RiftsOnly), // B246
    r(B, Post, r"/launcher/v1/p2p/hunts/:sessionId/transport-config", P2p, Rifts, RiftsOnly), // B247
    r(B, Post, r"/launcher/v1/p2p/hunts/:sessionId/signal", P2p, Rifts, RiftsOnly), // B248
    r(B, Get, r"/launcher/v1/p2p/control", P2p, Rifts, RiftsOnly), // B249
    r(B, Post, r"/launcher/v1/p2p/control/:messageId/ack", P2p, Rifts, RiftsOnly), // B250
    r(B, Post, r"/launcher/v1/p2p/host-presence", P2p, Rifts, RiftsOnly), // B251
    r(B, Post, r"/launcher/v1/p2p/host-offers/:offerId/respond", P2p, Rifts, RiftsOnly), // B252
    r(B, Post, r"/launcher/v1/p2p/hunts/:sessionId/observation", P2p, Rifts, RiftsOnly), // B253
    r(B, Post, r"/launcher/v1/realm-hosting/hubs/start", Realms, Rifts, RiftsOnly), // B254
    r(B, Post, r"/launcher/v1/realm-hosting/hubs/stop", Realms, Rifts, RiftsOnly), // B255
    r(B, Post, r"/launcher/v1/p2p/hunts/:sessionId/peers", Realms, Rifts, RiftsOnly), // B256
    r(B, Get, r"/launcher/v1/p2p/admission-key", P2p, Rifts, RiftsOnly), // no Node route: the realms admission key
    r(B, Get, r"/launcher/v1/realm-hosting/status", Realms, Rifts, RiftsOnly), // B257
    r(B, Post, r"/launcher/v1/realms", Realms, Rifts, RiftsOnly), // B258
    r(B, Get, r"/launcher/v1/realms/mine", Realms, Rifts, RiftsOnly), // B259
    r(B, Get, r"/launcher/v1/realms/listed", Realms, Rifts, RiftsOnly), // B260
    r(B, Get, r"/launcher/v1/realms/by-name/:name", Realms, Rifts, RiftsOnly), // B261
    r(B, Get, r"/launcher/v1/realms/invite/:code", Realms, Rifts, RiftsOnly), // B262
    r(B, Post, r"/launcher/v1/realms/join", Realms, Rifts, RiftsOnly), // B263
    r(B, Post, r"/launcher/v1/realms/by-name/:name/leave", Realms, Rifts, RiftsOnly), // B264
    r(B, Post, r"/launcher/v1/realms/mine/settings", Realms, Rifts, RiftsOnly), // B265
    r(B, Post, r"/launcher/v1/realms/mine/invites", Realms, Rifts, RiftsOnly), // B266
    r(B, Get, r"/launcher/v1/realms/mine/invites", Realms, Rifts, RiftsOnly), // B267
    r(B, Post, r"/launcher/v1/realms/mine/invites/:code/revoke", Realms, Rifts, RiftsOnly), // B268
    r(B, Post, r"/launcher/v1/realms/mine/members/:accountId/kick", Realms, Rifts, RiftsOnly), // B269
    r(B, Post, r"/launcher/v1/realms/mine/members/:accountId/ban", Realms, Rifts, RiftsOnly), // B270
    r(B, Post, r"/launcher/v1/realms/mine/members/:accountId/unban", Realms, Rifts, RiftsOnly), // B271
    r(B, Post, r"/launcher/v1/realms/mine/heartbeat", Realms, Rifts, RiftsOnly), // B272
    r(B, Post, r"/launcher/v1/realms/mine/release", Realms, Rifts, RiftsOnly), // B273
    r(B, Get, r"/launcher/v1/realms/mine/requests", Realms, Rifts, RiftsOnly), // B274
    r(B, Post, r"/launcher/v1/realms/mine/requests/:accountId/approve", Realms, Rifts, RiftsOnly), // B275
    r(B, Post, r"/launcher/v1/realms/mine/requests/:accountId/deny", Realms, Rifts, RiftsOnly), // B276
    r(B, Post, r"/launcher/v1/realms/mine/members/add", Realms, Rifts, RiftsOnly), // B277
    r(B, Post, r"/launcher/v1/realms/mine/player-invites", Realms, Rifts, RiftsOnly), // B278
    r(B, Post, r"/launcher/v1/realms/mine/player-invites/:accountId/cancel", Realms, Rifts, RiftsOnly), // B279
    r(B, Get, r"/launcher/v1/realms/player-invites", Realms, Rifts, RiftsOnly), // B280
    r(B, Post, r"/launcher/v1/realms/player-invites/:name/accept", Realms, Rifts, RiftsOnly), // B281
    r(B, Post, r"/launcher/v1/realms/player-invites/:name/decline", Realms, Rifts, RiftsOnly), // B282
    r(B, Post, r"/launcher/v1/realms/presence/heartbeat", Realms, Rifts, RiftsOnly), // B283
    r(B, Post, r"/launcher/v1/realms/presence/leave", Realms, Rifts, RiftsOnly), // B284
    r(B, Get, r"/launcher/v1/realms/presence", Realms, Rifts, RiftsOnly), // B285
    r(D, Get, r"/health/ready", Director, Isles, Always), // D1
    r(D, Get, r"/api/hub-status", Director, Isles, Always), // D2
    r(D, Get, r"/api/instances", Director, Isles, Always), // D3
    r(D, Post, r"/api/matchmaker/handle-matchmaking-for-player", Director, Isles, Always), // D4
    r(D, Post, r"/api/matchmaker/resolve-hunt-launch-manifest", P2p, Rifts, RiftsOnly), // D5
];
