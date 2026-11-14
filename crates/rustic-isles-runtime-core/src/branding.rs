//! Client branding and the F2 console (spec-runtime-r17): the texts and their culture rule, the matchers, the
//! credits slot reordering, the scripted login's counters and the console's schedule. The DLL's
//! `branding` handlers apply these to the game.

use crate::names::{Kind, Matcher};

/// R17's flag names (RT-14 for R17).
pub mod flags {
    /// Leaves the login screen's version lines alone (read once).
    pub const DISABLE_LOGIN_BRANDING: &str = "DISABLE_LOGIN_BRANDING";
    /// Leaves the credits alone (read once).
    pub const DISABLE_CREDITS_BRANDING: &str = "DISABLE_CREDITS_BRANDING";
    /// Leaves the press-start text alone (read once).
    pub const DISABLE_PRESS_START_BRANDING: &str = "DISABLE_PRESS_START_BRANDING";
    /// Scripted runs: press start, Play and travel (probed per matching call).
    pub const AUTO_PRESS_START: &str = "AUTO_PRESS_START";
    /// Leaves `FExec::Exec`'s fatal stub alone, so a console command kills the game again (CON-2).
    pub const DISABLE_CONSOLE_EXEC_GUARD: &str = "DISABLE_CONSOLE_EXEC_GUARD";
}

/// Login screen (4.2): every function whose full name holds `LoginScreen`; the body filters by object.
pub const LOGIN: Matcher = &[(Kind::Contains, "LoginScreen")];
/// Credits (4.5): any `OnInitialized`, and the credits widget's `PreConstruct`; the body filters by class.
pub const CREDITS: Matcher = &[
    (Kind::FunctionName, "OnInitialized"),
    (Kind::Contains, "bpw_Credits_C.PreConstruct"),
];
/// Press-start popup (4.6): the screen's, its panel's and the native screen's functions.
pub const PRESS_START: Matcher = &[(Kind::Contains, "PressStart")];
/// The scripted login (5.1); the body also requires `LoginScreen` in the full name.
pub const AUTO_LOGIN: Matcher = &[
    (Kind::FunctionName, "OnStartupMovieFinished"),
    (Kind::FunctionName, "OnPlayScreenPollTimer"),
];
/// The console (6.2): every UFunction; it returns at once when done.
pub const CONSOLE: Matcher = &[(Kind::Prefix, "Function ")];

/// The handlers' places on SP-1 (4.1, 5.1, 6.2).
pub mod order {
    /// After: the login screen's version lines.
    pub const LOGIN: i32 = 0;
    /// After: the credits.
    pub const CREDITS: i32 = 10;
    /// After: the press-start text.
    pub const PRESS_START: i32 = 20;
    /// After: the console.
    pub const CONSOLE: i32 = 100;
    /// Before: the scripted login.
    pub const AUTO_LOGIN: i32 = 0;
}

/// "Function short name" in log lines: the text after the last `.`.
pub fn function_short_name(full_name: &str) -> &str {
    full_name.rsplit('.').next().unwrap_or(full_name)
}

/// The language of R17's texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// English.
    En,
    /// Brazilian Portuguese.
    PtBr,
}

/// The culture rule: a culture starting `pt` (any case) takes `pt-BR`; anything else, or none, `en`.
pub fn lang_for_culture(culture: Option<&str>) -> Lang {
    match culture {
        Some(culture) if culture.get(..2).is_some_and(|prefix| prefix.eq_ignore_ascii_case("pt")) => Lang::PtBr,
        _ => Lang::En,
    }
}

/// The text keys (section 7).
pub mod keys {
    /// The press-start popup's welcome.
    pub const WELCOME: &str = "press_start.welcome";
    /// The credits block's title.
    pub const CREDITS_TITLE: &str = "credits.title";
    /// Its subheading.
    pub const CREDITS_SUBHEADING: &str = "credits.subheading";
    /// Its disclaimer.
    pub const CREDITS_DISCLAIMER: &str = "credits.disclaimer";
    /// Its names column.
    pub const CREDITS_NAMES: &str = "credits.names";
    /// Its roles column.
    pub const CREDITS_ROLES: &str = "credits.roles";
}

/// English (DU2).
pub const EN: &[(&str, &str)] = &[
    (
        keys::WELCOME,
        "Welcome to Dainty Lass, Slayer.\nA community server that keeps Dauntless playable.\n\nNot affiliated with Phoenix Labs.",
    ),
    (keys::CREDITS_TITLE, "Dainty Lass"),
    (keys::CREDITS_SUBHEADING, "Community server\n"),
    (
        keys::CREDITS_DISCLAIMER,
        "Dainty Lass is a community preservation project\nand is not affiliated with Phoenix Labs.\n",
    ),
    (
        keys::CREDITS_NAMES,
        "Silent Spud\nThe Undaunted and Mystic Paradox communities\n\n",
    ),
    (keys::CREDITS_ROLES, "Dainty Lass\nThe servers it grew from\n\n"),
];

/// Brazilian Portuguese (DU2; "Slayer" stays the game's English term).
pub const PT_BR: &[(&str, &str)] = &[
    (
        keys::WELCOME,
        "Bem-vindo ao Dainty Lass, Slayer.\nUm servidor da comunidade que mantém Dauntless jogável.\n\nSem vínculo com a Phoenix Labs.",
    ),
    (keys::CREDITS_TITLE, "Dainty Lass"),
    (keys::CREDITS_SUBHEADING, "Servidor da comunidade\n"),
    (
        keys::CREDITS_DISCLAIMER,
        "Dainty Lass é um projeto de preservação da comunidade\ne não tem vínculo com a Phoenix Labs.\n",
    ),
    (
        keys::CREDITS_NAMES,
        "Silent Spud\nAs comunidades Undaunted e Mystic Paradox\n\n",
    ),
    (keys::CREDITS_ROLES, "Dainty Lass\nOs servidores de onde ele veio\n\n"),
];

/// The text for `key` in `lang`; empty for an unknown key.
pub fn text(lang: Lang, key: &str) -> &'static str {
    let table = match lang {
        Lang::En => EN,
        Lang::PtBr => PT_BR,
    };
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map_or("", |(_, value)| value)
}

/// Moves `slots[from]` to `to`, shifting the slots between by one (4.5 step 7). Out of range is refused.
pub fn reorder<T: Copy>(slots: &mut [T], from: usize, to: usize) -> bool {
    if from >= slots.len() || to >= slots.len() {
        return false;
    }
    if from > to {
        slots[to..=from].rotate_right(1);
    } else {
        slots[from..=to].rotate_left(1);
    }
    true
}

/// What one poll of the Play screen should try.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PollPlan {
    /// Try the Play click (its poll count).
    pub play: Option<u32>,
    /// Try the travel (its poll count).
    pub travel: Option<u32>,
}

/// The scripted login's once-per-process steps (5.2).
#[derive(Debug, Default)]
pub struct AutoLogin {
    pressed: bool,
    play_polls: u32,
    play_done: bool,
    travel_polls: u32,
    travel_done: bool,
}

/// Polls to let the Play screen settle before acting.
pub const SETTLE_POLLS: u32 = 4;
/// A miss is logged every this many polls.
pub const MISS_LOG_EVERY: u32 = 10;

impl AutoLogin {
    /// A fresh process.
    pub const fn new() -> Self {
        Self {
            pressed: false,
            play_polls: 0,
            play_done: false,
            travel_polls: 0,
            travel_done: false,
        }
    }

    /// `OnStartupMovieFinished`: true the first time only (step 1 is tried once).
    pub fn press_start(&mut self) -> bool {
        !std::mem::replace(&mut self.pressed, true)
    }

    /// `OnPlayScreenPollTimer`: counts the poll for each open step and says which to try.
    pub fn poll(&mut self) -> PollPlan {
        let mut plan = PollPlan::default();
        if !self.play_done {
            self.play_polls += 1;
            plan.play = (self.play_polls >= SETTLE_POLLS).then_some(self.play_polls);
        }
        if !self.travel_done {
            self.travel_polls += 1;
            plan.travel = (self.travel_polls >= SETTLE_POLLS).then_some(self.travel_polls);
        }
        plan
    }

    /// Step 2 succeeded.
    pub fn play_done(&mut self) {
        self.play_done = true;
    }

    /// Step 3 succeeded, or its function is missing: stop it for the process.
    pub fn travel_done(&mut self) {
        self.travel_done = true;
    }

    /// Whether a miss on poll `count` is logged.
    pub fn log_miss(count: u32) -> bool {
        count.is_multiple_of(MISS_LOG_EVERY)
    }
}

/// How often the console handler looks.
pub const CONSOLE_THROTTLE_MS: u64 = 250;
/// How long after the viewport appears the console is made.
pub const CONSOLE_DELAY_MS: u64 = 3000;

/// The console's schedule (6.2 steps 1, 3 and 4).
#[derive(Debug, Default)]
pub struct ConsoleSchedule {
    last_check: Option<u64>,
    viewport_since: Option<u64>,
    done: bool,
}

impl ConsoleSchedule {
    /// Nothing seen yet.
    pub const fn new() -> Self {
        Self {
            last_check: None,
            viewport_since: None,
            done: false,
        }
    }

    /// Step 1: whether to look now (at most every 250 ms, never once done).
    pub fn check(&mut self, now: u64) -> bool {
        if self.done {
            return false;
        }
        if self
            .last_check
            .is_some_and(|last| now.saturating_sub(last) < CONSOLE_THROTTLE_MS)
        {
            return false;
        }
        self.last_check = Some(now);
        true
    }

    /// Steps 3 and 4: the viewport is up; true once 3 000 ms have passed since it was first seen.
    pub fn viewport_seen(&mut self, now: u64) -> bool {
        let since = *self.viewport_since.get_or_insert(now);
        !self.done && now.saturating_sub(since) >= CONSOLE_DELAY_MS
    }

    /// Step 4 or 5 ended it.
    pub fn finish(&mut self) {
        self.done = true;
    }

    /// Whether it ended.
    pub fn done(&self) -> bool {
        self.done
    }
}

/// The console's one line.
pub fn console_line(key_set: bool, spawned: bool) -> String {
    format!(
        "[MitConsole] console on F2 (key {}, console {})",
        if key_set { "set" } else { "left: no console keys" },
        if spawned { "spawned" } else { "kept" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::matches;
    use std::collections::HashSet;

    #[test]
    fn tables_match() {
        let en: HashSet<&str> = EN.iter().map(|(key, _)| *key).collect();
        let pt: HashSet<&str> = PT_BR.iter().map(|(key, _)| *key).collect();
        assert_eq!(en, pt);
        assert_eq!(en.len(), EN.len());
        assert_eq!(pt.len(), PT_BR.len());
        for key in [
            keys::WELCOME,
            keys::CREDITS_TITLE,
            keys::CREDITS_SUBHEADING,
            keys::CREDITS_DISCLAIMER,
            keys::CREDITS_NAMES,
            keys::CREDITS_ROLES,
        ] {
            assert!(en.contains(key), "{key}");
        }
        for (_, value) in EN.iter().chain(PT_BR) {
            assert!(!value.is_empty());
        }
        assert_eq!(text(Lang::PtBr, keys::CREDITS_SUBHEADING), "Servidor da comunidade\n");
        assert_eq!(text(Lang::En, "nope"), "");
        for lang in [Lang::En, Lang::PtBr] {
            let names = text(lang, keys::CREDITS_NAMES);
            let roles = text(lang, keys::CREDITS_ROLES);
            assert_eq!(names.lines().count(), roles.lines().count(), "rows stay aligned");
            assert!(names.ends_with("\n\n") && roles.ends_with("\n\n"));
        }
    }

    #[test]
    fn culture_rule() {
        assert_eq!(lang_for_culture(Some("pt-BR")), Lang::PtBr);
        assert_eq!(lang_for_culture(Some("pt-PT")), Lang::PtBr);
        assert_eq!(lang_for_culture(Some("PT")), Lang::PtBr);
        assert_eq!(lang_for_culture(Some("en-US")), Lang::En);
        assert_eq!(lang_for_culture(Some("")), Lang::En);
        assert_eq!(lang_for_culture(Some("p")), Lang::En);
        assert_eq!(lang_for_culture(None), Lang::En, "unreadable");
    }

    const MOVIE: &str = "Function LoginScreen_bps.LoginScreen_bps_C.OnStartupMovieFinished";
    const POLL: &str = "Function LoginScreen_bps.LoginScreen_bps_C.OnPlayScreenPollTimer";
    const INITIALIZED: &str = "Function UMG.UserWidget.OnInitialized";
    const PRECONSTRUCT: &str = "Function bpw_Credits.bpw_Credits_C.PreConstruct";
    const CONSTRUCT: &str = "Function PressStartScreen_bps.PressStartScreen_bps_C.Construct";

    /// Every function R17 calls (8.2).
    const CALLED: &[&str] = &[
        "Function UMG.TextBlock.SetText",
        "Function UMG.Widget.SetVisibility",
        "Function UMG.PanelWidget.AddChild",
        "Function Engine.GameplayStatics.SpawnObject",
        "Function UMG.TextBlock.SetFont",
        "Function UMG.TextBlock.SetColorAndOpacity",
        "Function UMG.TextBlock.SetShadowOffset",
        "Function UMG.TextBlock.SetShadowColorAndOpacity",
        "Function UMG.TextLayoutWidget.SetJustification",
        "Function UMG.ScrollBoxSlot.SetPadding",
        "Function UMG.ScrollBoxSlot.SetHorizontalAlignment",
        "Function UMG.ScrollBoxSlot.SetVerticalAlignment",
        "Function UMG.WidgetBlueprintLibrary.Create",
        "Function Engine.KismetTextLibrary.Conv_StringToText",
        "Function Engine.KismetStringLibrary.Conv_StringToName",
        "Function Engine.KismetInternationalizationLibrary.GetCurrentCulture",
        "Function Archon.ArchonPartyClient.StartTravelToCity",
    ];
    /// The two LoginScreen functions the scripted login calls.
    const CALLED_ON_LOGIN: &[&str] = &[
        "Function LoginScreen_bps.LoginScreen_bps_C.OnPressToStartDismissed",
        "Function LoginScreen_bps.LoginScreen_bps_C.BndEvt__w_loginbutton_K2Node_ComponentBoundEvent_3_OnButtonClickedEvent__DelegateSignature",
    ];

    #[test]
    fn matchers_catch() {
        assert!(matches(AUTO_LOGIN, Some(MOVIE)));
        assert!(matches(AUTO_LOGIN, Some(POLL)));
        assert!(matches(LOGIN, Some(MOVIE)));
        assert!(matches(LOGIN, Some(POLL)));
        assert!(matches(CREDITS, Some(INITIALIZED)));
        assert!(matches(CREDITS, Some(PRECONSTRUCT)));
        assert!(!matches(
            CREDITS,
            Some("Function bpw_Credits.bpw_Credits_C.OnInitializedLate")
        ));
        assert!(matches(PRESS_START, Some(CONSTRUCT)));
        assert!(matches(
            PRESS_START,
            Some("Function PressStartPanel_bpw.PressStartPanel_bpw_C.Construct")
        ));
        assert!(!matches(AUTO_LOGIN, Some(CONSTRUCT)));
        assert!(!matches(CREDITS, Some(MOVIE)));
        assert!(!matches(PRESS_START, Some(PRECONSTRUCT)));
        assert!(matches(CONSOLE, Some(MOVIE)));
        assert!(!matches(CONSOLE, None));
    }

    #[test]
    fn nothing_r17_calls_reenters_it() {
        for called in CALLED.iter().chain(CALLED_ON_LOGIN) {
            for (name, matcher) in [
                ("auto login", AUTO_LOGIN),
                ("credits", CREDITS),
                ("press start", PRESS_START),
            ] {
                assert!(!matches(matcher, Some(called)), "{name} matches {called}");
            }
            assert!(matches(CONSOLE, Some(called)), "the console's catch-all");
        }
        for called in CALLED {
            assert!(!matches(LOGIN, Some(called)), "login matches {called}");
        }
        // The login handler sees the two LoginScreen calls: it only collapses version lines, calling neither
        for called in CALLED_ON_LOGIN {
            assert!(matches(LOGIN, Some(called)));
        }
    }

    #[test]
    fn short_names() {
        assert_eq!(function_short_name(MOVIE), "OnStartupMovieFinished");
        assert_eq!(function_short_name("plain"), "plain");
    }

    #[test]
    fn reordering() {
        let mut slots = [1, 2, 3, 4];
        assert!(reorder(&mut slots, 3, 0));
        assert_eq!(slots, [4, 1, 2, 3], "last to first");
        let mut slots = [1, 2, 3, 4, 5];
        assert!(reorder(&mut slots, 4, 2));
        assert_eq!(slots, [1, 2, 5, 3, 4], "last to middle");
        assert!(reorder(&mut slots, 2, 2));
        assert_eq!(slots, [1, 2, 5, 3, 4], "in place");
        assert!(!reorder(&mut slots, 5, 0));
        assert!(!reorder(&mut slots, 4, 5));
        assert_eq!(slots, [1, 2, 5, 3, 4]);
        let mut empty: [u8; 0] = [];
        assert!(!reorder(&mut empty, 0, 0));
    }

    #[test]
    fn auto_login_counters() {
        let mut auto = AutoLogin::new();
        assert!(auto.press_start());
        assert!(!auto.press_start(), "once");
        for _ in 1..SETTLE_POLLS {
            assert_eq!(auto.poll(), PollPlan::default());
        }
        assert_eq!(
            auto.poll(),
            PollPlan {
                play: Some(4),
                travel: Some(4)
            }
        );
        auto.play_done();
        assert_eq!(
            auto.poll(),
            PollPlan {
                play: None,
                travel: Some(5)
            }
        );
        auto.travel_done();
        assert_eq!(auto.poll(), PollPlan::default());
        assert!(AutoLogin::log_miss(10));
        assert!(AutoLogin::log_miss(20));
        assert!(!AutoLogin::log_miss(4));
        assert!(!AutoLogin::log_miss(11));
    }

    #[test]
    fn console_schedule() {
        let mut schedule = ConsoleSchedule::new();
        assert!(schedule.check(1000));
        assert!(!schedule.check(1249), "throttled");
        assert!(schedule.check(1250));
        assert!(!schedule.viewport_seen(1250));
        assert!(schedule.check(4249));
        assert!(!schedule.viewport_seen(4249), "2 999 ms");
        assert!(schedule.check(4500));
        assert!(schedule.viewport_seen(4250 + 250));
        let mut schedule = ConsoleSchedule::new();
        assert!(schedule.check(0));
        assert!(!schedule.viewport_seen(0));
        assert!(schedule.viewport_seen(3000), "3 000 ms");
        schedule.finish();
        assert!(schedule.done());
        assert!(!schedule.check(10_000), "done stays done");
        assert!(!schedule.viewport_seen(10_000));
        assert_eq!(
            console_line(true, false),
            "[MitConsole] console on F2 (key set, console kept)"
        );
        assert_eq!(
            console_line(false, true),
            "[MitConsole] console on F2 (key left: no console keys, console spawned)"
        );
    }
}
