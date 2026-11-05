//! What carries from a 1.4.4 database at the final schema: accounts, their bans and Discord links, and the invite
//! codes. Everything else is counted and left behind: characters, inventory and progression don't map across game
//! versions, and the account keys are SHA-256 hashes where 1.14.7 keeps HMACs, so they can't carry either.

use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Nullable, Text};
use store::Connection;

use crate::error::Result;
use crate::report::Report;
use crate::target::{Ban, Batch, DiscordLink, Invite, LegacyAccount};

/// The final schema's tables an import carries, in full or in part.
const CARRIED: [&str; 4] = ["users", "account_bans", "discordlinks", "invitecodes"];

/// Credentials and bookkeeping that are left behind on purpose, and so aren't reported as player data lost.
const NOT_PLAYER_DATA: [&str; 12] = [
    "userapikeys",
    "userapikeystoregister",
    "gameserverapikeys",
    "gameserverapikeystoregister",
    "accountkeyrecovery",
    "recoveryproofs",
    "recoverychallenges",
    "recoveryevents",
    "authepochs",
    "onetimejobs",
    "player_addresses",
    "moderation_events",
];

#[derive(QueryableByName)]
struct User {
    #[diesel(sql_type = Text)]
    user_id: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    notes: i64,
    #[diesel(sql_type = Bool)]
    is_admin: bool,
}

#[derive(QueryableByName)]
struct BanRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Nullable<BigInt>)]
    expires_at: Option<i64>,
    #[diesel(sql_type = Text)]
    reason: String,
}

#[derive(QueryableByName)]
struct LinkRow {
    #[diesel(sql_type = Text)]
    discord_id: String,
    #[diesel(sql_type = Text)]
    user_id: String,
    #[diesel(sql_type = Text)]
    linked_at: String,
}

#[derive(QueryableByName)]
struct InviteRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = BigInt)]
    uses_remaining: i64,
    #[diesel(sql_type = Bool)]
    infinite_uses: bool,
}

#[derive(QueryableByName)]
struct Table {
    #[diesel(sql_type = Text)]
    name: String,
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

/// `linkedAt` as 1.14.7 keeps it (RFC 3339), or `now` when 1.4.4 kept something else.
fn rfc3339_or(text: &str, now: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(text) {
        Ok(time) => time
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string(),
        Err(_) => now.to_owned(),
    }
}

/// The accounts and invites to carry. `now_ms` decides which bans are still in force; `now` stands in for a
/// Discord link time 1.4.4 kept in another form.
pub fn batch(conn: &mut Connection, now_ms: i64, now: &str, with_invites: bool) -> Result<Batch> {
    let mut bans: BTreeMap<String, Ban> = BTreeMap::new();
    let rows = diesel::sql_query(
        "SELECT account_id, expires_at, reason FROM account_bans WHERE active <> 0 AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind::<BigInt, _>(now_ms)
    .load::<BanRow>(conn)?;
    for row in rows {
        bans.insert(
            row.account_id,
            Ban {
                expires_at: row.expires_at,
                reason: row.reason,
            },
        );
    }
    let mut links: BTreeMap<String, DiscordLink> = BTreeMap::new();
    let rows = diesel::sql_query(
        "SELECT discordId AS discord_id, userId AS user_id, linkedAt AS linked_at FROM discordlinks ORDER BY discordId",
    )
    .load::<LinkRow>(conn)?;
    for row in rows {
        links.insert(
            row.user_id,
            DiscordLink {
                subject: row.discord_id,
                linked_at: rfc3339_or(&row.linked_at, now),
            },
        );
    }
    let users =
        diesel::sql_query("SELECT userId AS user_id, name, notes, isAdmin AS is_admin FROM users ORDER BY userId")
            .load::<User>(conn)?;
    let accounts = users
        .into_iter()
        .map(|user| LegacyAccount {
            ban: bans.remove(&user.user_id),
            discord: links.remove(&user.user_id),
            user_id: user.user_id,
            name: user.name,
            notes: user.notes,
            admin: user.is_admin,
        })
        .collect();
    let invites = if with_invites {
        diesel::sql_query(
            "SELECT invitecode AS code, usesRemaining AS uses_remaining, infiniteUses AS infinite_uses \
             FROM invitecodes ORDER BY invitecode",
        )
        .load::<InviteRow>(conn)?
        .into_iter()
        .map(|row| Invite {
            code: row.code,
            uses_remaining: row.uses_remaining,
            infinite_uses: row.infinite_uses,
        })
        .collect()
    } else {
        Vec::new()
    };
    Ok(Batch { accounts, invites })
}

/// Counts the player data no 1.14.7 table takes, by table, into `report.dropped`.
pub fn count_dropped(conn: &mut Connection, report: &mut Report) -> Result<()> {
    let tables = diesel::sql_query(
        "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite!_%' ESCAPE '!' \
         AND name NOT LIKE '!_!_%' ESCAPE '!' ORDER BY name",
    )
    .load::<Table>(conn)?;
    for table in tables {
        if CARRIED.contains(&table.name.as_str()) || NOT_PLAYER_DATA.contains(&table.name.as_str()) {
            continue;
        }
        let rows = diesel::sql_query(format!(
            "SELECT count(*) AS n FROM {}",
            store::portable::quote_ident(&table.name)
        ))
        .get_result::<Count>(conn)?
        .n;
        if rows > 0 {
            report
                .dropped
                .insert(table.name, usize::try_from(rows).unwrap_or(usize::MAX));
        }
    }
    Ok(())
}
