//! The help screen's "Reset my save" button, its pure parts: the 1.14.7 names it hooks, its texts and the backend
//! call it makes (`POST /dainty/v1/save-reset`). Names and offsets are from the maintainer's Dumper-7 dump of this
//! build and the `Help_bps` asset; the backend decides and words every answer.

use crate::branding::Lang;
use crate::names::{Kind, Matcher};

/// Leaves the help screen as the game built it (read once).
pub const DISABLE_SAVE_RESET_BUTTON: &str = "DISABLE_SAVE_RESET_BUTTON";

/// The help screen's Blueprint class (`/Game/UI/MainMenu/Help_bps`).
pub const HELP_SCREEN_CLASS: &str = "help_bps_C";
/// The class of the help screen's buttons ("Report a Player", "Send Feedback" and the rest).
pub const BUTTON_CLASS: &str = "w_Button_CenteredBox_sm_C";
/// The game's own two-button popup.
pub const POPUP_CLASS: &str = "ConfirmPopup_bpw_C";

/// After the help screen's `Construct`: the button goes in.
pub const CONSTRUCT: Matcher = &[(Kind::FunctionName, "Construct")];
/// The button's inner `UButton` clicked.
pub const CLICKED: Matcher = &[(
    Kind::FunctionName,
    "BndEvt__HairButton_K2Node_ComponentBoundEvent_11_OnButtonClickedEvent__DelegateSignature",
)];
/// The popup's two answers, from its buttons, keys and gamepad alike.
pub const ANSWERED: Matcher = &[(Kind::FunctionName, "Confirm"), (Kind::FunctionName, "Cancel")];
/// Any call: a finished request is shown on the game thread at the next one.
pub const ANY: Matcher = &[(Kind::Prefix, "Function ")];

/// Offsets and parameter blocks (UMG, 4.26.2-647472).
pub mod layout {
    /// `UUserWidget.WidgetTree`.
    pub const USER_WIDGET_TREE: usize = 0x260;
    /// `UWidgetTree.RootWidget`.
    pub const TREE_ROOT: usize = 0x28;
    /// `UWidget.Slot`.
    pub const WIDGET_SLOT: usize = 0x28;
    /// `UVerticalBoxSlot.Padding` (`FMargin`, 16 bytes).
    pub const VBOX_SLOT_PADDING: usize = 0x40;
    /// `UVerticalBoxSlot.HorizontalAlignment`.
    pub const VBOX_SLOT_HALIGN: usize = 0x58;
    /// `ConfirmPopup_bpw_C.RemoveOnClose`.
    pub const POPUP_REMOVE_ON_CLOSE: usize = 0x402;
    /// `WidgetBlueprintLibrary.Create`'s block: context, class, owning player, return.
    pub const CREATE_PARAMS: usize = 0x20;
    /// `ConfirmPopup_bpw_C.Show_Popup`'s block, its one local included.
    pub const SHOW_POPUP_PARAMS: usize = 0x20;
    /// Above the help screen.
    pub const POPUP_Z_ORDER: i32 = 1000;
    /// The deepest widget tree walked looking for the buttons' box.
    pub const MAX_DEPTH: u32 = 8;
}

/// The texts the runtime shows itself; everything else is the backend's message.
pub mod keys {
    /// The help screen's new button.
    pub const BUTTON: &str = "save_reset.button";
    /// The confirm popup's button that resets.
    pub const CONFIRM: &str = "save_reset.confirm";
    /// The confirm popup's button that doesn't.
    pub const CANCEL: &str = "save_reset.cancel";
    /// An answer popup's buttons.
    pub const OK: &str = "save_reset.ok";
    /// The backend gave no answer.
    pub const UNREACHABLE: &str = "save_reset.unreachable";
}

const EN: &[(&str, &str)] = &[
    (keys::BUTTON, "Reset My Save"),
    (keys::CONFIRM, "Reset My Save"),
    (keys::CANCEL, "Cancel"),
    (keys::OK, "OK"),
    (
        keys::UNREACHABLE,
        "The server could not be reached, and nothing was changed. Please try again later.",
    ),
];

const PT_BR: &[(&str, &str)] = &[
    (keys::BUTTON, "Reiniciar Meu Progresso"),
    (keys::CONFIRM, "Reiniciar Meu Progresso"),
    (keys::CANCEL, "Cancelar"),
    (keys::OK, "OK"),
    (
        keys::UNREACHABLE,
        "Não foi possível falar com o servidor, e nada foi alterado. Tente de novo mais tarde.",
    ),
];

/// The text of `key` in `lang`; empty for an unknown key.
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

/// The language the backend is asked to answer in.
pub fn lang_tag(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::PtBr => "pt-BR",
    }
}

/// Which step the backend is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The button: warn, or say when the next reset is allowed.
    Ask,
    /// The popup's confirm.
    Confirm,
}

impl Step {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Confirm => "confirm",
        }
    }
}

/// The backend's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// `asked`, `tooSoon`, `done`, `nothingToConfirm` or `failed`.
    pub status: String,
    /// What the popup says, in the asked language.
    pub message: String,
}

impl Answer {
    /// Whether the popup asks to confirm, rather than only saying something.
    pub fn asks(&self) -> bool {
        self.status == "asked"
    }
}

/// The HTTP/1.1 request, `authorization` as the game sent it. `host` is the backend's `host[:port]`.
pub fn request(host: &str, authorization: &str, step: Step, lang: Lang) -> String {
    let body = format!(r#"{{"step":"{}","lang":"{}"}}"#, step.as_str(), lang_tag(lang));
    format!(
        "POST /dainty/v1/save-reset HTTP/1.1\r\nHost: {host}\r\nAuthorization: {authorization}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Whether a header value can go in a request line as is (no line break smuggled in).
pub fn header_safe(value: &str) -> bool {
    !value.is_empty() && value.len() <= 8192 && value.bytes().all(|b| b == b'\t' || (0x20..0x7f).contains(&b))
}

/// The answer in a whole response: a 200 or 503 with the JSON body the route writes.
pub fn parse(response: &[u8]) -> Option<Answer> {
    let text = std::str::from_utf8(response).ok()?;
    let (head, body) = text.split_once("\r\n\r\n")?;
    let code = head.split(' ').nth(1)?;
    if code != "200" && code != "503" {
        return None;
    }
    if head
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"))
    {
        return None;
    }
    Some(Answer {
        status: string_field(body, "status")?,
        message: string_field(body, "message")?,
    })
}

/// The string value of `"name":` in a flat JSON object, escapes decoded.
fn string_field(json: &str, name: &str) -> Option<String> {
    let key = format!("\"{name}\"");
    let at = json.find(&key)? + key.len();
    let rest = json[at..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' | 'f' => {}
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let unit = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(unit).unwrap_or('\u{fffd}'));
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_in_both_languages() {
        for key in [keys::BUTTON, keys::CONFIRM, keys::CANCEL, keys::OK, keys::UNREACHABLE] {
            assert!(!text(Lang::En, key).is_empty(), "{key}");
            assert!(!text(Lang::PtBr, key).is_empty(), "{key}");
        }
        assert_eq!(text(Lang::En, "nope"), "");
    }

    #[test]
    fn the_request_is_one_post() {
        let r = request("127.0.0.1:41000", "Bearer abc", Step::Confirm, Lang::PtBr);
        assert!(r.starts_with("POST /dainty/v1/save-reset HTTP/1.1\r\nHost: 127.0.0.1:41000\r\n"));
        assert!(r.contains("\r\nAuthorization: Bearer abc\r\n"));
        assert!(r.ends_with("\r\n\r\n{\"step\":\"confirm\",\"lang\":\"pt-BR\"}"));
        assert!(r.contains("Content-Length: 33\r\n"));
        assert!(header_safe("Bearer a.b-c_d"));
        assert!(!header_safe("Bearer a\r\nX: y"));
        assert!(!header_safe(""));
    }

    #[test]
    fn answers_are_read_from_the_route_s_json() {
        let ok = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n\
                   {\"status\":\"tooSoon\",\"message\":\"Voc\\u00ea \\\"pode\\\" em 2026-11-19 14:05 UTC.\"}";
        let answer = parse(ok).unwrap();
        assert_eq!(answer.status, "tooSoon");
        assert_eq!(answer.message, "Você \"pode\" em 2026-11-19 14:05 UTC.");
        assert!(!answer.asks());
        let failed = b"HTTP/1.1 503 Service Unavailable\r\n\r\n{\"status\":\"failed\",\"message\":\"x\"}";
        assert_eq!(parse(failed).unwrap().status, "failed");
        assert!(parse(b"HTTP/1.1 401 Unauthorized\r\n\r\n").is_none());
        assert!(parse(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1a\r\n{}").is_none());
        assert!(parse(b"HTTP/1.1 200 OK\r\n\r\n{\"status\":\"asked\"}").is_none());
        assert!(parse(b"garbage").is_none());
    }
}
