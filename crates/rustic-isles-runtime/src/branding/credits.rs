//! The Dainty Lass block above Phoenix Labs in the credits (spec-runtime-r17 4.5). A section made without a table
//! keeps the text set on it, so a new section of Phoenix's own class carries the block, styled like it.

use super::{Budget, collapse, read_once, text};
use crate::ffi::{engine, log, reflect};
use crate::handlers::PeCall;
use rustic_isles_runtime_core::branding::{flags, function_short_name, keys, reorder};
use rustic_isles_runtime_core::offsets::r17;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

static DISABLED: OnceLock<bool> = OnceLock::new();
static CLASS: AtomicUsize = AtomicUsize::new(0);
static PLACED_ON: AtomicUsize = AtomicUsize::new(0);
static SECTION: AtomicUsize = AtomicUsize::new(0);
static BUSY: Mutex<()> = Mutex::new(());
static ROLES_FALLBACK: Budget = Budget::new();
static CONSTRUCTED: Budget = Budget::new();

const NOT_AS_EXPECTED: &str = "[MitCredits] Phoenix Labs section not as expected - credits left unchanged";

pub(super) fn after(call: &PeCall) {
    if read_once(&DISABLED, flags::DISABLE_CREDITS_BRANDING) {
        return;
    }
    let credits = call.object;
    if !is_credits(credits) {
        return;
    }
    let Some(full_name) = call.entry.full_name() else {
        return;
    };
    // try_lock: the block's own calls into the game must not place it twice
    let Ok(_busy) = BUSY.try_lock() else {
        return;
    };
    if function_short_name(&full_name) == "OnInitialized" {
        if PLACED_ON.swap(credits, Ordering::Relaxed) != credits {
            place(credits);
        }
    } else if full_name.contains("bpw_Credits_C.PreConstruct") {
        recheck();
    }
}

/// The object's class is exactly `bpw_Credits_C` (its class pointer cached once seen).
fn is_credits(object: usize) -> bool {
    if !engine::is_live_object(object) {
        return false;
    }
    let Some(class) = reflect::class_of(object) else {
        return false;
    };
    let known = CLASS.load(Ordering::Relaxed);
    if known != 0 {
        return class == known;
    }
    let found = reflect::class_name_of(object).as_deref() == Some("bpw_Credits_C");
    if found {
        CLASS.store(class, Ordering::Relaxed);
    }
    found
}

fn set_columns(section: usize) -> bool {
    let Some(columns) = reflect::live_ptr(section + r17::SECTION_COLUMNS) else {
        return false;
    };
    let names = child(columns, 0).is_some_and(|names| reflect::set_text(names, text(keys::CREDITS_NAMES)));
    let roles = child(columns, 1).is_some_and(|roles| reflect::set_text(roles, text(keys::CREDITS_ROLES)));
    names && roles
}

/// A panel's slots: the array's data pointer and its slot pointers.
fn slots(panel: usize) -> Option<(usize, Vec<usize>)> {
    let array = panel + r17::UPANELWIDGET_SLOTS;
    let data = reflect::ptr(array)?;
    let count = reflect::bytes_at(array + 8, 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(i32::from_le_bytes)?;
    let count = usize::try_from(count).ok().filter(|&count| count <= 4096)?;
    let slots = (0..count)
        .map(|i| reflect::ptr(data + i * 8))
        .collect::<Option<Vec<usize>>>()?;
    Some((data, slots))
}

/// The panel's child `index` (its slot's content), live.
fn child(panel: usize, index: usize) -> Option<usize> {
    let (_, slots) = slots(panel)?;
    reflect::live_ptr(*slots.get(index)? + r17::UPANELSLOT_CONTENT)
}

/// Spawns a text block of the style's class in the credits' widget tree, styled from it, holding `line`.
fn text_line(credits: usize, style: usize, line: &'static str) -> Option<usize> {
    let tree = reflect::live_ptr(credits + r17::UUSERWIDGET_WIDGET_TREE)?;
    let mut params = reflect::Params::new(0x18);
    params.put_ptr(0, reflect::class_of(style)?);
    params.put_ptr(8, tree);
    if !reflect::static_call("GameplayStatics", "SpawnObject", &mut params) {
        return None;
    }
    let block = params.get_ptr(0x10);
    if block == 0 {
        return None;
    }
    // Byte copies of the style's members, passed to the engine's setters, which copy them properly
    let setters = [
        ("SetFont", r17::UTEXTBLOCK_FONT, 0x58),
        ("SetColorAndOpacity", r17::UTEXTBLOCK_COLOR, 0x28),
        ("SetShadowOffset", r17::UTEXTBLOCK_SHADOW_OFFSET, 0x08),
        ("SetShadowColorAndOpacity", r17::UTEXTBLOCK_SHADOW_COLOR, 0x10),
    ];
    for (setter, member, size) in setters {
        let mut params = reflect::Params::new(size);
        params.put(0, &reflect::bytes_at(style + member, size)?);
        reflect::invoke(block, setter, &mut params);
    }
    let mut params = reflect::Params::new(1);
    params.put(0, &[1]);
    reflect::invoke(block, "SetJustification", &mut params);
    reflect::set_text(block, line);
    Some(block)
}

/// Adds `content` to the panel and moves its slot to `target`. Copies Phoenix's slot alignment (and, for the
/// first line, its padding) through the slot's own setters.
fn add_at(panel: usize, content: usize, phoenix_slot: usize, target: usize, first: bool) -> bool {
    let mut params = reflect::Params::new(0x10);
    params.put_ptr(0, content);
    if !reflect::invoke(panel, "AddChild", &mut params) {
        return false;
    }
    let slot = params.get_ptr(8);
    if slot == 0 {
        return false;
    }
    if reflect::class_of(slot).is_some() && reflect::class_of(slot) == reflect::class_of(phoenix_slot) {
        let mut setters = vec![
            ("SetHorizontalAlignment", r17::SLOT_HALIGN, 1),
            ("SetVerticalAlignment", r17::SLOT_VALIGN, 1),
        ];
        if first {
            setters.push(("SetPadding", r17::SLOT_PADDING, 16));
        }
        for (setter, member, size) in setters {
            if let Some(value) = reflect::bytes_at(phoenix_slot + member, size) {
                let mut params = reflect::Params::new(size);
                params.put(0, &value);
                reflect::invoke(slot, setter, &mut params);
            }
        }
    }
    let Some((data, mut slots)) = slots(panel) else {
        return false;
    };
    let Some(last) = slots.len().checked_sub(1) else {
        return false;
    };
    if slots[last] != slot || !reflect::writable(data, slots.len() * 8) || !reorder(&mut slots, last, target) {
        return false;
    }
    // Slate builds the box's children in Slots order and the panel has no reflected insert-at
    slots
        .iter()
        .enumerate()
        .skip(target)
        .all(|(i, &value)| reflect::write(data + i * 8, value))
}

fn place(credits: usize) {
    let Some(placed) = build(credits) else {
        return;
    };
    let (count, index, section) = placed;
    if count == 4 {
        SECTION.store(section, Ordering::Relaxed);
    }
    log::line(&format!(
        "[MitCredits] Dainty Lass block above Phoenix Labs: placed {count}/4 at index {index}"
    ));
}

/// Steps 1 to 7: (widgets placed, Phoenix's index, the new section).
fn build(credits: usize) -> Option<(usize, usize, usize)> {
    let found = phoenix(credits);
    let Some((phoenix, phoenix_slot, panel, index)) = found else {
        log::line(NOT_AS_EXPECTED);
        return None;
    };
    let Some(title_style) = reflect::live_ptr(phoenix + r17::SECTION_TITLE_TEXT) else {
        log::line(NOT_AS_EXPECTED);
        return None;
    };
    let roles_style = reflect::live_ptr(phoenix + r17::SECTION_COLUMNS)
        .and_then(|columns| child(columns, 1))
        .unwrap_or_else(|| {
            if ROLES_FALLBACK.take(1) {
                log::line("[MitCredits] Phoenix Labs has no roles column; its title style is used");
            }
            title_style
        });

    let mut params = reflect::Params::new(0x20);
    params.put_ptr(0, credits);
    params.put_ptr(8, reflect::class_of(phoenix)?);
    let section = if reflect::static_call("WidgetBlueprintLibrary", "Create", &mut params) {
        params.get_ptr(0x18)
    } else {
        0
    };
    let section_title = reflect::live_ptr(section + r17::SECTION_TITLE_TEXT);
    let section_columns = reflect::live_ptr(section + r17::SECTION_COLUMNS);
    let (Some(section_title), Some(_)) = (section_title, section_columns) else {
        log::line("[MitCredits] the Dainty Lass section could not be made - credits left unchanged");
        return None;
    };
    if let Some(show) = reflect::u8_at(phoenix + r17::SECTION_SHOW_TITLE) {
        reflect::write(section + r17::SECTION_SHOW_TITLE, show);
    }
    collapse(section_title);
    set_columns(section);

    let lines = [
        (title_style, keys::CREDITS_TITLE),
        (roles_style, keys::CREDITS_SUBHEADING),
        (roles_style, keys::CREDITS_DISCLAIMER),
    ];
    let mut widgets = Vec::with_capacity(4);
    for (style, key) in lines {
        let Some(line) = text_line(credits, style, text(key)) else {
            log::line("[MitCredits] a Dainty Lass text line could not be made - credits left unchanged");
            return None;
        };
        widgets.push(line);
    }
    widgets.push(section);

    let mut placed = 0;
    for (i, &widget) in widgets.iter().enumerate() {
        if !add_at(panel, widget, phoenix_slot, index + placed, i == 0) {
            break;
        }
        placed += 1;
    }
    Some((placed, index, section))
}

/// Step 1: Phoenix's section, its slot, the panel holding it and its position there.
fn phoenix(credits: usize) -> Option<(usize, usize, usize, usize)> {
    let phoenix = reflect::live_ptr(credits + r17::CREDITS_PHOENIX)?;
    let slot = reflect::live_ptr(phoenix + r17::UWIDGET_SLOT)?;
    let panel = reflect::live_ptr(slot + r17::UPANELSLOT_PARENT)?;
    let (_, slots) = slots(panel)?;
    let index = slots
        .iter()
        .position(|&each| reflect::ptr(each + r17::UPANELSLOT_CONTENT) == Some(phoenix))?;
    Some((phoenix, slot, panel, index))
}

/// `PreConstruct`: the section's column texts again, if it is still live.
fn recheck() {
    let section = SECTION.load(Ordering::Relaxed);
    let present = section != 0 && engine::is_live_object(section) && set_columns(section);
    if CONSTRUCTED.take(4) {
        log::line(&format!(
            "[MitCredits] credits constructed; Dainty Lass section {}",
            if present { "present" } else { "missing" }
        ));
    }
}
