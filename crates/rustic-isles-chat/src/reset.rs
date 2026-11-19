// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A player starts their save over, from the help screen's button (the runtime's confirm popup, through
//! `POST /dainty/v1/save-reset`) or by typing `/reset` in chat. Either asks first, which only warns and opens a
//! two-minute window; only a confirm inside it resets (`rustic_isles_store::repo::resets`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use dainty_module_api::{Actor, ActorContext, GlobalId};
use rustic_isles::{AccountId, replay};
use rustic_isles_store::repo::resets::{self, PlayerReset};
use rustic_isles_store::{IslesStore, RepoResult};

/// How long a `/reset` waits for its confirm.
pub const CONFIRM_WINDOW_MS: i64 = 120_000;

/// What the player typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetStep {
    /// `/reset`: warn and wait for the confirm.
    Ask,
    /// `/reset confirm` (or `confirmar`).
    Confirm,
}

/// The step a line is, or None when it is not `/reset`.
pub fn parse(line: &str) -> Option<ResetStep> {
    let mut words = line.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("/reset") {
        return None;
    }
    match words.next().map(str::to_ascii_lowercase).as_deref() {
        Some("confirm" | "confirmar") => Some(ResetStep::Confirm),
        _ => Some(ResetStep::Ask),
    }
}

/// What the source did with a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetAnswer {
    /// The warning stands; a confirm is taken until this unix ms.
    Asked,
    /// A confirm with no open `/reset` before it.
    NothingToConfirm,
    /// Refused: the next reset is allowed at this unix ms.
    TooSoon(i64),
    /// The save was reset.
    Done,
}

/// The open `/reset`s, by account: in memory, as a two-minute window needs nothing more.
#[derive(Debug, Default)]
pub struct Confirmations {
    open: Mutex<HashMap<GlobalId, i64>>,
}

impl Confirmations {
    /// Opens the window for `account` at `now`.
    pub fn open(&self, account: &GlobalId, now: i64) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.retain(|_, until| *until > now);
        open.insert(account.clone(), now.saturating_add(CONFIRM_WINDOW_MS));
    }

    /// Closes the window; whether it was still open at `now`.
    pub fn take(&self, account: &GlobalId, now: i64) -> bool {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.remove(account).is_some_and(|until| until > now)
    }
}

/// The open windows of this process, shared by the chat command and the help screen's button.
pub fn confirmations() -> &'static Confirmations {
    static OPEN: OnceLock<Confirmations> = OnceLock::new();
    OPEN.get_or_init(Confirmations::default)
}

/// One step of `account`'s reset at `now`, on its home store.
pub async fn step(store: &IslesStore, account: &AccountId, step: ResetStep, now: i64) -> RepoResult<ResetAnswer> {
    let global = account.global().clone();
    match step {
        ResetStep::Ask => {
            let key = account.clone();
            let next = store
                .read(move |conn, _| resets::next_player_reset(conn, &key, now))
                .await?;
            if let Some(next) = next {
                return Ok(ResetAnswer::TooSoon(next));
            }
            confirmations().open(&global, now);
            Ok(ResetAnswer::Asked)
        }
        ResetStep::Confirm => {
            if !confirmations().take(&global, now) {
                return Ok(ResetAnswer::NothingToConfirm);
            }
            let actor = ActorContext::new(Actor::Player { account: global });
            let snapshot = replay::random_uuid();
            let key = account.clone();
            let done = store
                .account_write(account, &actor, move |tx, _| {
                    resets::player_reset(tx, &key, &snapshot, now)
                })
                .await?;
            Ok(match done {
                PlayerReset::Done(rows) => {
                    tracing::info!("save reset account={account} rows={rows}");
                    ResetAnswer::Done
                }
                PlayerReset::TooSoon(next) => ResetAnswer::TooSoon(next),
            })
        }
    }
}

/// The answer's name in the route's JSON.
pub fn status(answer: ResetAnswer) -> &'static str {
    match answer {
        ResetAnswer::Asked => "asked",
        ResetAnswer::NothingToConfirm => "nothingToConfirm",
        ResetAnswer::TooSoon(_) => "tooSoon",
        ResetAnswer::Done => "done",
    }
}

/// The game's language as the client named it: `pt-BR` for any Portuguese, else `en`.
fn is_portuguese(lang: Option<&str>) -> bool {
    lang.is_some_and(|l| l.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("pt")))
}

/// `2026-11-19 14:05 UTC`.
fn utc_minute(ms: i64) -> String {
    let iso = dainty_service_kit::js::iso_string(ms);
    format!("{} UTC", iso.get(..16).unwrap_or(&iso).replace('T', " "))
}

/// The catalog: `(en, pt-BR)` per reply. `{time}` is the only placeholder.
const ASKED: (&str, &str) = (
    "Reset your save? This erases all your characters, gear, currency and progress, and you start over as a new \
     slayer. Your account, name and friends stay. Type /reset confirm within 2 minutes to go ahead. After the reset \
     you must quit to the main menu and rejoin.",
    "Reiniciar seu progresso? Isso apaga todos os seus personagens, equipamentos, moedas e progresso, e você \
     recomeça como um novo caçador. Sua conta, seu nome e seus amigos continuam. Digite /reset confirmar em até 2 \
     minutos para continuar. Depois do reinício, você precisa voltar ao menu principal e entrar de novo.",
);
/// The popup's warning: the same, with the popup's own button to go ahead.
const ASKED_POPUP: (&str, &str) = (
    "Reset your save? This erases all your characters, gear, currency and progress, and you start over as a new      slayer. Your account, name and friends stay. You can do this once every 24 hours. After the reset you must quit      to the main menu and rejoin.",
    "Reiniciar seu progresso? Isso apaga todos os seus personagens, equipamentos, moedas e progresso, e você      recomeça como um novo caçador. Sua conta, seu nome e seus amigos continuam. Você pode fazer isso uma vez a      cada 24 horas. Depois do reinício, você precisa voltar ao menu principal e entrar de novo.",
);
const NOTHING_TO_CONFIRM: (&str, &str) = (
    "There is no reset to confirm. Type /reset first.",
    "Não há reinício para confirmar. Digite /reset primeiro.",
);
const TOO_SOON: (&str, &str) = (
    "You can reset your save once every 24 hours. Your next reset is allowed at {time}.",
    "Você pode reiniciar seu progresso uma vez a cada 24 horas. O próximo reinício é permitido em {time}.",
);
const DONE: (&str, &str) = (
    "Your save was reset. Quit to the main menu and rejoin to start over.",
    "Seu progresso foi reiniciado. Volte ao menu principal e entre de novo para recomeçar.",
);
const FAILED: (&str, &str) = (
    "The reset could not be done right now, and nothing was changed. Please try again later.",
    "Não foi possível reiniciar agora, e nada foi alterado. Tente de novo mais tarde.",
);

fn pick(text: (&'static str, &'static str), lang: Option<&str>) -> &'static str {
    if is_portuguese(lang) { text.1 } else { text.0 }
}

/// The reply to an answer, in the client's language.
pub fn reply(answer: ResetAnswer, lang: Option<&str>) -> String {
    match answer {
        ResetAnswer::Asked => pick(ASKED, lang).to_owned(),
        ResetAnswer::NothingToConfirm => pick(NOTHING_TO_CONFIRM, lang).to_owned(),
        ResetAnswer::TooSoon(next) => pick(TOO_SOON, lang).replace("{time}", &utc_minute(next)),
        ResetAnswer::Done => pick(DONE, lang).to_owned(),
    }
}

/// The popup's text for an answer: as [`reply`], but the warning names no chat command.
pub fn popup(answer: ResetAnswer, lang: Option<&str>) -> String {
    match answer {
        ResetAnswer::Asked => pick(ASKED_POPUP, lang).to_owned(),
        other => reply(other, lang),
    }
}

/// The reply when the source could not answer.
pub fn failed(lang: Option<&str>) -> String {
    pick(FAILED, lang).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_reset() {
        assert_eq!(parse("/reset"), Some(ResetStep::Ask));
        assert_eq!(parse("  /RESET  "), Some(ResetStep::Ask));
        assert_eq!(parse("/reset now"), Some(ResetStep::Ask));
        assert_eq!(parse("/reset confirm"), Some(ResetStep::Confirm));
        assert_eq!(parse("/reset Confirmar"), Some(ResetStep::Confirm));
        assert_eq!(parse("/resets"), None);
        assert_eq!(parse("reset"), None);
        assert_eq!(parse("/report x"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn window_closes_after_two_minutes_and_on_take() {
        let account: GlobalId = "account:01J00000000000000000000000@oregon-1".parse().unwrap();
        let c = Confirmations::default();
        assert!(!c.take(&account, 0));
        c.open(&account, 0);
        assert!(c.take(&account, CONFIRM_WINDOW_MS - 1));
        assert!(!c.take(&account, CONFIRM_WINDOW_MS - 1));
        c.open(&account, 0);
        assert!(!c.take(&account, CONFIRM_WINDOW_MS));
    }

    #[test]
    fn replies_follow_the_language_and_say_when() {
        assert!(reply(ResetAnswer::Asked, None).contains("rejoin"));
        assert!(reply(ResetAnswer::Asked, Some("pt-BR")).contains("entrar de novo"));
        assert!(reply(ResetAnswer::Asked, Some("en-US")).starts_with("Reset"));
        let at = 1_795_000_000_000;
        let text = reply(ResetAnswer::TooSoon(at), Some("pt"));
        assert!(text.ends_with(&format!("{}.", utc_minute(at))), "{text}");
        assert!(!text.contains("{time}"));
        assert!(!popup(ResetAnswer::Asked, None).contains("/reset"));
        assert!(popup(ResetAnswer::Asked, Some("pt-BR")).contains("24 horas"));
        assert_eq!(popup(ResetAnswer::Done, None), reply(ResetAnswer::Done, None));
        assert_eq!(utc_minute(0), "1970-01-01 00:00 UTC");
    }
}
