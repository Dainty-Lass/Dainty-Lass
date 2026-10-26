//! The realm directory (`realms/realmsService.ts`): claiming a realm, membership, invites and presence.

use rifts_seam::{AccountId, BoxFuture};

use crate::RealmsError;
use crate::store::{JoinPolicy, RealmInviteRecord, RealmPresenceRecord, RealmRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRealm {
    pub name: String,
    pub join_policy: JoinPolicy,
    pub listed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinRealm {
    ByName(String),
    ByInvite(String),
}

/// `mine/settings`: the realm's policy and the hosting limits (`hosting.maxServers`, `maxPlayers`, `afkSeconds`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmSettings {
    pub join_policy: Option<JoinPolicy>,
    pub listed: Option<bool>,
    pub max_servers: Option<u8>,
    /// 1 to 64.
    pub max_players: Option<u8>,
    /// 0 to 7200.
    pub afk_seconds: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteOptions {
    pub max_uses: Option<u32>,
    pub lifetime_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberAction {
    Kick,
    Ban { reason: Option<String> },
    Unban,
}

/// One method per `realmsLauncher.ts` route; the route layer parses bodies and writes the views.
pub trait RealmDirectory: Send + Sync {
    /// `createRealm`.
    fn create_realm(&self, account: AccountId, realm: NewRealm) -> BoxFuture<'_, Result<RealmRecord, RealmsError>>;
    /// `getMine`.
    fn mine(&self, account: AccountId) -> BoxFuture<'_, Result<Option<RealmRecord>, RealmsError>>;
    /// `listListed`.
    fn listed(
        &self,
        account: AccountId,
        offset: u32,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<RealmRecord>, RealmsError>>;
    /// `getByName`; `known_host_key` is the joiner's pinned key.
    fn by_name(
        &self,
        account: AccountId,
        name: String,
        known_host_key: Option<String>,
    ) -> BoxFuture<'_, Result<Option<RealmRecord>, RealmsError>>;
    /// `previewInvite`.
    fn preview_invite(&self, account: AccountId, code: String) -> BoxFuture<'_, Result<RealmRecord, RealmsError>>;
    /// `join`.
    fn join(&self, account: AccountId, join: JoinRealm) -> BoxFuture<'_, Result<RealmRecord, RealmsError>>;
    /// `leave`.
    fn leave(&self, account: AccountId, name: String) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `updateSettings`.
    fn update_settings(
        &self,
        account: AccountId,
        settings: RealmSettings,
    ) -> BoxFuture<'_, Result<RealmRecord, RealmsError>>;
    /// `createInvite`.
    fn create_invite(
        &self,
        account: AccountId,
        options: InviteOptions,
    ) -> BoxFuture<'_, Result<RealmInviteRecord, RealmsError>>;
    /// `listInvites`.
    fn invites(&self, account: AccountId) -> BoxFuture<'_, Result<Vec<RealmInviteRecord>, RealmsError>>;
    /// `revokeInvite`.
    fn revoke_invite(&self, account: AccountId, code: String) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `kick`, `ban`, `unban`.
    fn moderate(
        &self,
        account: AccountId,
        target: AccountId,
        action: MemberAction,
    ) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `heartbeat`: the host keeps the realm online.
    fn host_heartbeat(&self, account: AccountId) -> BoxFuture<'_, Result<RealmRecord, RealmsError>>;
    /// `release`: the host gives up the name.
    fn release(&self, account: AccountId) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `listJoinRequests`.
    fn join_requests(&self, account: AccountId) -> BoxFuture<'_, Result<Vec<AccountId>, RealmsError>>;
    /// `approveJoinRequest`, `denyJoinRequest`.
    fn decide_join_request(
        &self,
        account: AccountId,
        target: AccountId,
        approve: bool,
    ) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `invitePlayerByUsername`; `addMemberByUsername` (launchers to 0.1.68) is the same call.
    fn invite_player(&self, account: AccountId, username: String) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `cancelPlayerInvite`.
    fn cancel_player_invite(&self, account: AccountId, target: AccountId) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `listMyPlayerInvites`.
    fn player_invites(&self, account: AccountId) -> BoxFuture<'_, Result<Vec<RealmRecord>, RealmsError>>;
    /// `acceptPlayerInvite`, `declinePlayerInvite`.
    fn answer_player_invite(
        &self,
        account: AccountId,
        realm_name: String,
        accept: bool,
    ) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `presenceHeartbeat` (also `TouchGameSession`).
    fn presence_heartbeat(&self, account: AccountId) -> BoxFuture<'_, Result<RealmPresenceRecord, RealmsError>>;
    /// `presenceLeave`.
    fn presence_leave(&self, account: AccountId) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `presenceStatus`.
    fn presence(&self, account: AccountId) -> BoxFuture<'_, Result<Option<RealmPresenceRecord>, RealmsError>>;
}
