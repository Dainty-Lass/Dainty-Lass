// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The art pack the launcher shows behind the server's page (`CONTENT_BRANDING_DIR`): background images with
//! their credits and an accent colour, re-scanned every 10 s.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value, json};

use crate::input::{clean_accent, clean_credit, cut_utf16, image_content_type, is_safe_image_name, signature_matches};
use crate::{Clock, log};

pub const RESCAN_MS: i64 = 10_000;
pub const MAX_IMAGES: usize = 50;
pub const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
pub const URL_PREFIX: &str = "/content/v1/branding/";
const CONFIG: &str = "branding.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub name: String,
    pub credit: Option<String>,
    pub content_type: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub images: Vec<Image>,
    pub accent: Option<String>,
    pub body: String,
}

impl Scan {
    fn new(images: Vec<Image>, accent: Option<String>) -> Self {
        let backgrounds: Vec<Value> = images
            .iter()
            .map(|image| json!({"url": format!("{URL_PREFIX}{}", image.name), "credit": image.credit}))
            .collect();
        let body = service_kit::jsjson::stringify(&json!({"backgrounds": backgrounds, "accent": accent}));
        Self { images, accent, body }
    }

    pub fn image(&self, name: &str) -> Option<&Image> {
        self.images.iter().find(|image| image.name == name)
    }
}

pub struct Branding {
    dir: Option<PathBuf>,
    clock: Clock,
    state: Mutex<Option<(i64, Arc<Scan>)>>,
}

impl Branding {
    pub fn new(dir: Option<PathBuf>, clock: Clock) -> Self {
        Self {
            dir,
            clock,
            state: Mutex::new(None),
        }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The current scan, redone when there is none or it is [`RESCAN_MS`] old. Blocking.
    pub fn current(&self) -> Arc<Scan> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let now = (self.clock)();
        if let Some((at, scan)) = state.as_ref()
            && now - at < RESCAN_MS
        {
            return scan.clone();
        }
        let scan = Arc::new(match &self.dir {
            Some(dir) => scan(dir),
            None => Scan::new(Vec::new(), None),
        });
        *state = Some((now, scan.clone()));
        scan
    }
}

fn shown(value: Option<&Value>) -> String {
    cut_utf16(&service_kit::js::string(value), 120)
}

/// Scans the folder: `branding.json`'s list (or every safe image by name), each image checked.
pub fn scan(dir: &Path) -> Scan {
    let dir_text = dir.display().to_string();
    let listing: BTreeSet<String> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .collect(),
        Err(error) => {
            log::warn(
                "branding folder unreadable",
                json!({"dir": dir_text, "error": error.to_string()}),
            );
            return Scan::new(Vec::new(), None);
        }
    };
    let config = read_config(dir, &dir_text);
    let wanted: Vec<(Option<Value>, Option<Value>)> = match config.as_ref().and_then(|c| c.get("backgrounds")) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(_) => (Some(item.clone()), None),
                Value::Object(entry) => (entry.get("file").cloned(), entry.get("credit").cloned()),
                _ => (None, None),
            })
            .collect(),
        // BTreeSet: sorted by bytes
        _ => listing
            .iter()
            .filter(|name| is_safe_image_name(name))
            .map(|name| (Some(Value::String(name.clone())), None))
            .collect(),
    };
    let mut images: Vec<Image> = Vec::new();
    for (file, credit) in wanted {
        if images.len() == MAX_IMAGES {
            break;
        }
        let name = match &file {
            Some(Value::String(name))
                if is_safe_image_name(name) && listing.contains(name) && !images.iter().any(|i| &i.name == name) =>
            {
                name.clone()
            }
            _ => {
                log::warn(
                    "branding image skipped (bad name, missing or listed twice)",
                    json!({"file": shown(file.as_ref())}),
                );
                continue;
            }
        };
        let content_type = image_content_type(&name);
        match check_image(&dir.join(&name), content_type) {
            Ok(()) => images.push(Image {
                credit: clean_credit(credit.as_ref()),
                name,
                content_type,
            }),
            Err(Skip::Unreadable(error)) => log::warn(
                "branding image skipped (unreadable)",
                json!({"file": name, "error": error}),
            ),
            Err(Skip::NotAFile) => log::warn(
                "branding image skipped (not a regular file, empty or over 25 MB)",
                json!({"file": name}),
            ),
            Err(Skip::Mismatch) => log::warn(
                "branding image skipped (contents don't match the extension)",
                json!({"file": name}),
            ),
        }
    }
    let accent_value = config.as_ref().and_then(|c| c.get("accent"));
    let accent = clean_accent(accent_value);
    if accent_value.is_some() && accent.is_none() {
        log::warn("branding.json accent ignored: use #rgb or #rrggbb", json!({}));
    }
    Scan::new(images, accent)
}

fn read_config(dir: &Path, dir_text: &str) -> Option<Map<String, Value>> {
    let bytes = match fs::read(dir.join(CONFIG)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn(
                "branding.json is not valid JSON; ignoring it",
                json!({"dir": dir_text, "error": error.to_string()}),
            );
            return None;
        }
    };
    let parsed = String::from_utf8(bytes).map_err(|e| e.to_string()).and_then(|text| {
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        serde_json::from_str::<Value>(text).map_err(|e| e.to_string())
    });
    match parsed {
        Ok(Value::Object(config)) => Some(config),
        Ok(_) => {
            log::warn("branding.json is not an object; ignoring it", json!({"dir": dir_text}));
            None
        }
        Err(error) => {
            log::warn(
                "branding.json is not valid JSON; ignoring it",
                json!({"dir": dir_text, "error": error}),
            );
            None
        }
    }
}

enum Skip {
    Unreadable(String),
    NotAFile,
    Mismatch,
}

fn check_image(path: &Path, content_type: &str) -> Result<(), Skip> {
    // Not followed: an image linked in from elsewhere is not served
    let meta = fs::symlink_metadata(path).map_err(|e| Skip::Unreadable(e.to_string()))?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > MAX_IMAGE_BYTES {
        return Err(Skip::NotAFile);
    }
    let mut head = Vec::with_capacity(12);
    fs::File::open(path)
        .and_then(|file| file.take(12).read_to_end(&mut head))
        .map_err(|e| Skip::Unreadable(e.to_string()))?;
    if !signature_matches(content_type, &head) {
        return Err(Skip::Mismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0];

    #[test]
    fn no_folder_or_a_missing_one_means_no_art() {
        let empty = r#"{"backgrounds":[],"accent":null}"#;
        assert_eq!(Branding::new(None, Arc::new(|| 0)).current().body, empty);
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(scan(&tmp.path().join("nope")).body, empty);
    }

    #[test]
    fn without_branding_json_valid_images_in_name_order() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        fs::write(d.join("b.png"), PNG).unwrap();
        fs::write(d.join("a.jpg"), JPEG).unwrap();
        fs::write(d.join("fake.png"), b"not a png").unwrap();
        fs::write(d.join("notes.txt"), b"x").unwrap();
        fs::write(d.join("empty.jpg"), b"").unwrap();
        fs::create_dir(d.join("sub.jpg")).unwrap();
        let scan = scan(d);
        assert_eq!(
            scan.body,
            r#"{"backgrounds":[{"url":"/content/v1/branding/a.jpg","credit":null},{"url":"/content/v1/branding/b.png","credit":null}],"accent":null}"#
        );
    }

    #[test]
    fn branding_json_order_credits_and_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        fs::write(d.join("a.jpg"), JPEG).unwrap();
        fs::write(d.join("b.png"), PNG).unwrap();
        let config = json!({
            "backgrounds": [
                {"file": "b.png", "credit": "  Photo\nby   a friend "},
                "a.jpg",
                "../../secret.jpg",
                "missing.jpg",
                "a.jpg",
                5,
            ],
            "accent": "#C8A24A",
        });
        fs::write(d.join(CONFIG), format!("\u{feff}{config}")).unwrap();
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = lines.clone();
        let _guard = crate::log_capture(move |line| seen.lock().unwrap().push(line.to_owned()));
        let scan = scan(d);
        assert_eq!(
            scan.body,
            r##"{"backgrounds":[{"url":"/content/v1/branding/b.png","credit":"Photo by a friend"},{"url":"/content/v1/branding/a.jpg","credit":null}],"accent":"#c8a24a"}"##
        );
        let skipped = lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("bad name, missing or listed twice"))
            .count();
        assert!(skipped >= 3, "{skipped}");
    }

    #[test]
    fn broken_branding_json_still_lists_folder_images() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.jpg"), JPEG).unwrap();
        fs::write(tmp.path().join(CONFIG), "{nope").unwrap();
        assert_eq!(scan(tmp.path()).images.len(), 1);
        fs::write(tmp.path().join(CONFIG), "[1]").unwrap();
        assert_eq!(scan(tmp.path()).images.len(), 1);
    }

    #[test]
    fn a_new_image_appears_after_10_s() {
        let tmp = tempfile::tempdir().unwrap();
        let now = Arc::new(AtomicI64::new(0));
        let clock = now.clone();
        let branding = Branding::new(
            Some(tmp.path().to_owned()),
            Arc::new(move || clock.load(Ordering::SeqCst)),
        );
        assert!(branding.current().images.is_empty());
        fs::write(tmp.path().join("a.jpg"), JPEG).unwrap();
        now.store(9_999, Ordering::SeqCst);
        assert!(branding.current().images.is_empty());
        now.store(10_000, Ordering::SeqCst);
        assert_eq!(branding.current().images.len(), 1);
    }
}
