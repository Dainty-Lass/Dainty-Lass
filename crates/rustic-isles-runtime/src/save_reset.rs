//! The help screen's "Reset My Save" button (client only). On the help screen's `Construct` a copy of its own
//! button class goes in under the others; a click asks the backend (`POST /dainty/v1/save-reset`, with the game's
//! own sign-in) and its answer opens the game's `ConfirmPopup_bpw_C`, whose confirm asks again. Every call into the
//! game is on the game thread: the backend is called on a worker, and its answer waits for the next `ProcessEvent`.

use crate::ffi::{engine, flags, log, reflect};
use crate::handlers::{PeCall, PeHandler, Point, Verdict, register_pe};
use rustic_isles_runtime_core::branding::{Lang, lang_for_culture};
use rustic_isles_runtime_core::save_reset::{
    self as core, ANSWERED, ANY, Answer, BUTTON_CLASS, CLICKED, CONSTRUCT, DISABLE_SAVE_RESET_BUTTON,
    HELP_SCREEN_CLASS, POPUP_CLASS, Step, keys, layout,
};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(15);
/// The buttons made, newest last: a screen built again gets one more, never a second in the same box.
const KEPT_BUTTONS: usize = 16;

static DISABLED: OnceLock<bool> = OnceLock::new();
static AUTHORIZATION: Mutex<Option<String>> = Mutex::new(None);
static BUTTONS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// The widget the popups are made from: the button last clicked.
static CONTEXT: Mutex<usize> = Mutex::new(0);
/// The open popup, and whether its confirm resets.
static POPUP: Mutex<Option<(usize, bool)>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
static ANSWER: Mutex<Option<Option<Answer>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Init, client only, before the freeze.
pub fn register() {
    let handlers = [
        PeHandler {
            slice: "SaveReset",
            order: 50,
            matcher: CONSTRUCT,
            before: None,
            after: Some(constructed),
        },
        PeHandler {
            slice: "SaveReset",
            order: 0,
            matcher: CLICKED,
            before: Some(clicked),
            after: None,
        },
        PeHandler {
            slice: "SaveReset",
            order: 0,
            matcher: ANSWERED,
            before: Some(answered),
            after: None,
        },
        PeHandler {
            slice: "SaveReset",
            order: 200,
            matcher: ANY,
            before: Some(deliver),
            after: None,
        },
    ];
    for handler in handlers {
        if let Err(error) = register_pe(Point::Sp1, handler) {
            log::defer(format!("[SaveReset] handler not registered: {error}"));
        }
    }
}

/// HTTP-1 lends the game's `Authorization` from its own backend requests; only a value fit for a header is kept.
pub fn remember_authorization(value: &[u16]) {
    let text = String::from_utf16_lossy(value);
    if core::header_safe(&text) {
        *lock(&AUTHORIZATION) = Some(text);
    }
}

fn disabled() -> bool {
    *DISABLED.get_or_init(|| flags::flag(DISABLE_SAVE_RESET_BUTTON))
}

fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| lang_for_culture(reflect::current_culture().as_deref()))
}

fn text(key: &str) -> &'static str {
    core::text(lang(), key)
}

/// An `FText` of any text, made on each call (a popup's backend message differs each time).
fn ftext(text: &str) -> Option<[u8; reflect::FTEXT_SIZE]> {
    let string = reflect::FStringParam::new(text);
    let mut params = reflect::Params::new(0x28);
    string.put(&mut params, 0);
    if !reflect::static_call("KismetTextLibrary", "Conv_StringToText", &mut params) {
        return None;
    }
    let mut made = [0u8; reflect::FTEXT_SIZE];
    made.copy_from_slice(&params.get(0x10, reflect::FTEXT_SIZE));
    (!made.iter().all(|&b| b == 0)).then_some(made)
}

fn children(panel: usize) -> Vec<usize> {
    let mut count = reflect::Params::new(4);
    if !reflect::invoke(panel, "GetChildrenCount", &mut count) {
        return Vec::new();
    }
    let n = i32::from_le_bytes(count.get(0, 4).try_into().unwrap_or_default()).clamp(0, 64);
    (0..n)
        .filter_map(|i| {
            let mut params = reflect::Params::new(0x10);
            params.put(0, &i.to_le_bytes());
            reflect::invoke(panel, "GetChildAt", &mut params)
                .then(|| params.get_ptr(8))
                .filter(|&child| child != 0)
        })
        .collect()
}

/// The panel holding the help screen's buttons, and one of them.
fn button_box(widget: usize, depth: u32) -> Option<(usize, usize)> {
    if depth > layout::MAX_DEPTH || !reflect::is_a(widget, "PanelWidget") {
        return None;
    }
    let kids = children(widget);
    if let Some(&sibling) = kids
        .iter()
        .find(|&&child| reflect::class_name_of(child).as_deref() == Some(BUTTON_CLASS))
    {
        return Some((widget, sibling));
    }
    kids.into_iter().find_map(|child| button_box(child, depth + 1))
}

fn create(context: usize, class_name: &str) -> Option<usize> {
    let class = reflect::find_class(class_name)?;
    let mut owner = reflect::Params::new(8);
    let player = if reflect::invoke(context, "GetOwningPlayer", &mut owner) {
        owner.get_ptr(0)
    } else {
        0
    };
    let mut params = reflect::Params::new(layout::CREATE_PARAMS);
    params.put_ptr(0, context);
    params.put_ptr(8, class);
    params.put_ptr(0x10, player);
    if !reflect::static_call("WidgetBlueprintLibrary", "Create", &mut params) {
        return None;
    }
    Some(params.get_ptr(0x18)).filter(|&made| made != 0 && engine::is_live_object(made))
}

/// The new slot laid out as the sibling's: its padding and alignment.
fn copy_layout(sibling: usize, slot: usize) {
    let Some(theirs) = reflect::live_ptr(sibling + layout::WIDGET_SLOT) else {
        return;
    };
    if !reflect::is_a(theirs, "VerticalBoxSlot") || !reflect::is_a(slot, "VerticalBoxSlot") {
        return;
    }
    if let Some(padding) = reflect::bytes_at(theirs + layout::VBOX_SLOT_PADDING, 16) {
        let mut params = reflect::Params::new(16);
        params.put(0, &padding);
        reflect::invoke(slot, "SetPadding", &mut params);
    }
    if let Some(align) = reflect::u8_at(theirs + layout::VBOX_SLOT_HALIGN) {
        let mut params = reflect::Params::new(1);
        params.put(0, &[align]);
        reflect::invoke(slot, "SetHorizontalAlignment", &mut params);
    }
}

fn constructed(call: &PeCall) {
    if disabled() || reflect::class_name_of(call.object).as_deref() != Some(HELP_SCREEN_CLASS) {
        return;
    }
    let Some(tree) = reflect::live_ptr(call.object + layout::USER_WIDGET_TREE) else {
        return;
    };
    let Some(root) = reflect::live_ptr(tree + layout::TREE_ROOT) else {
        return;
    };
    let Some((panel, sibling)) = button_box(root, 0) else {
        log::line("[SaveReset] the help screen's buttons were not found; no reset button");
        return;
    };
    let ours = lock(&BUTTONS).clone();
    if children(panel).iter().any(|child| ours.contains(child)) {
        return;
    }
    let Some(button) = create(call.object, BUTTON_CLASS) else {
        log::line("[SaveReset] the reset button could not be made");
        return;
    };
    let mut add = reflect::Params::new(0x10);
    add.put_ptr(0, button);
    if !reflect::invoke(panel, "AddChild", &mut add) {
        return;
    }
    copy_layout(sibling, add.get_ptr(8));
    if let Some(label) = reflect::text_for(text(keys::BUTTON)) {
        let mut params = reflect::Params::new(reflect::FTEXT_SIZE);
        params.put(0, &label);
        if !reflect::invoke(button, "Set Button Label", &mut params) {
            log::line("[SaveReset] the reset button's label could not be set");
        }
    }
    let mut kept = lock(&BUTTONS);
    kept.push(button);
    let over = kept.len().saturating_sub(KEPT_BUTTONS);
    kept.drain(..over);
    log::line("[SaveReset] reset button added to the help screen");
}

fn clicked(call: &PeCall) -> Verdict {
    if !lock(&BUTTONS).contains(&call.object) {
        return Verdict::Continue;
    }
    *lock(&CONTEXT) = call.object;
    start(Step::Ask);
    // Nothing is bound to our copy's own dispatcher
    Verdict::Absorb
}

fn answered(call: &PeCall) -> Verdict {
    let mut popup = lock(&POPUP);
    let Some((open, asks)) = *popup else {
        return Verdict::Continue;
    };
    if open != call.object {
        return Verdict::Continue;
    }
    *popup = None;
    drop(popup);
    let confirmed = call
        .entry
        .full_name()
        .is_some_and(|name| name.rsplit('.').next() == Some("Confirm"));
    if asks && confirmed {
        start(Step::Confirm);
    }
    Verdict::Continue
}

/// The backend asked on a worker; a second click while one is out is dropped.
fn start(step: Step) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("save-reset".into()).spawn(move || {
        let answer = ask_backend(step);
        *lock(&ANSWER) = Some(answer);
        READY.store(true, Ordering::SeqCst);
    });
    if spawned.is_err() {
        *lock(&ANSWER) = Some(None);
        READY.store(true, Ordering::SeqCst);
    }
}

/// The launcher's loopback backend over plain HTTP; `None` for no answer (no sign-in seen yet, a TLS address, a
/// network error or a reply the route doesn't write).
fn ask_backend(step: Step) -> Option<Answer> {
    let authorization = lock(&AUTHORIZATION).clone()?;
    let url = crate::config::backend_url();
    if url.secure {
        log::line("[SaveReset] the backend address is not plain HTTP; nothing asked");
        return None;
    }
    let host = if url.port == 80 {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let address = format!("{}:{}", url.host, url.port).to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).ok()?;
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    stream
        .write_all(core::request(&host, &authorization, step, lang()).as_bytes())
        .ok()?;
    let mut response = Vec::new();
    stream.take(64 * 1024).read_to_end(&mut response).ok()?;
    let answer = core::parse(&response);
    if answer.is_none() {
        let status = String::from_utf8_lossy(response.split(|&b| b == b'\r').next().unwrap_or_default());
        log::line(&format!(
            "[SaveReset] no answer from the backend ({})",
            status.chars().take(64).collect::<String>()
        ));
    }
    answer
}

thread_local! {
    static SHOWING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The next `ProcessEvent` after an answer came: the popup it opens.
fn deliver(_call: &PeCall) -> Verdict {
    if !READY.load(Ordering::Relaxed) || SHOWING.with(std::cell::Cell::get) {
        return Verdict::Continue;
    }
    READY.store(false, Ordering::SeqCst);
    let Some(answer) = lock(&ANSWER).take() else {
        return Verdict::Continue;
    };
    SHOWING.with(|s| s.set(true));
    show(answer.as_ref());
    SHOWING.with(|s| s.set(false));
    BUSY.store(false, Ordering::SeqCst);
    Verdict::Continue
}

fn show(answer: Option<&Answer>) {
    let context = *lock(&CONTEXT);
    if context == 0 || !engine::is_live_object(context) {
        return;
    }
    let (message, asks) = match answer {
        Some(answer) => (answer.message.as_str(), answer.asks()),
        None => (text(keys::UNREACHABLE), false),
    };
    let Some(popup) = create(context, POPUP_CLASS) else {
        log::line("[SaveReset] the popup could not be made");
        return;
    };
    reflect::write_byte(popup + layout::POPUP_REMOVE_ON_CLOSE, 1);
    let mut z = reflect::Params::new(4);
    z.put(0, &layout::POPUP_Z_ORDER.to_le_bytes());
    if !reflect::invoke(popup, "AddToViewport", &mut z) {
        return;
    }
    let (confirm, cancel) = if asks {
        (text(keys::CONFIRM), text(keys::CANCEL))
    } else {
        (text(keys::OK), text(keys::OK))
    };
    if let (Some(confirm), Some(cancel)) = (reflect::text_for(confirm), reflect::text_for(cancel)) {
        let mut params = reflect::Params::new(2 * reflect::FTEXT_SIZE);
        params.put(0, &confirm);
        params.put(reflect::FTEXT_SIZE, &cancel);
        reflect::invoke(popup, "Set_Button_Labels", &mut params);
    }
    if let Some(message) = ftext(message) {
        let mut params = reflect::Params::new(layout::SHOW_POPUP_PARAMS);
        params.put(0, &message);
        reflect::invoke(popup, "Show_Popup", &mut params);
    }
    *lock(&POPUP) = Some((popup, asks));
}
