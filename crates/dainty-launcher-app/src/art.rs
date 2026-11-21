// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The `rr-art` scheme (RD27 spec §2.7.10): the server's branding images, cached by the controller, served to the
//! page by id so it never sees a file path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use dainty_launcher_kit::bridge::spec::is_lower_hex;
use dainty_launcher_kit::net::hostapi::ImageType;
use tauri::http::{Response, StatusCode, header};

/// The scheme's name.
pub const SCHEME: &str = "rr-art";
const ID_LEN: usize = 64;
// WebView2 cannot serve a custom scheme directly, so Tauri maps it to this host
const WEBVIEW2_HOST: &str = "rr-art.localhost";

/// Registered art files by id.
#[derive(Debug, Default)]
pub struct ArtStore {
    files: Mutex<HashMap<String, (PathBuf, ImageType)>>,
}

impl ArtStore {
    /// Serves `file` as `id`; an id that is not 64 lower-case hex is ignored.
    pub fn insert(&self, id: &str, file: &Path, kind: ImageType) {
        if is_lower_hex(id, ID_LEN) {
            self.lock().insert(id.to_owned(), (file.to_path_buf(), kind));
        }
    }

    fn get(&self, id: &str) -> Option<(PathBuf, ImageType)> {
        self.lock().get(id).cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (PathBuf, ImageType)>> {
        self.files.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The id a request names: `rr-art://bg/<id>` (WebKitGTK) or `http://rr-art.localhost/bg/<id>` (WebView2).
pub fn requested_id(uri: &str) -> Option<String> {
    let url = url::Url::parse(uri).ok()?;
    let path = match (url.scheme(), url.host_str()) {
        (SCHEME, Some("bg")) => url.path().strip_prefix('/')?,
        ("http" | "https", Some(WEBVIEW2_HOST)) => url.path().strip_prefix("/bg/")?,
        _ => return None,
    };
    is_lower_hex(path, ID_LEN).then(|| path.to_owned())
}

/// What the scheme answers: the file with its type, else 404 with an empty body.
pub async fn answer(store: &ArtStore, uri: &str) -> (StatusCode, Option<&'static str>, Vec<u8>) {
    let Some((file, kind)) = requested_id(uri).and_then(|id| store.get(&id)) else {
        return (StatusCode::NOT_FOUND, None, Vec::new());
    };
    match tokio::fs::read(&file).await {
        Ok(bytes) => (StatusCode::OK, Some(kind.content_type()), bytes),
        Err(_) => (StatusCode::NOT_FOUND, None, Vec::new()),
    }
}

/// [`answer`] as the HTTP response the webview gets.
pub async fn respond(store: &ArtStore, uri: &str) -> Response<Vec<u8>> {
    let (status, content_type, body) = answer(store, uri).await;
    let mut response = Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if let Some(content_type) = content_type {
        response = response.header(header::CONTENT_TYPE, content_type);
    }
    response.body(body).unwrap_or_else(|_| {
        let mut fallback = Response::new(Vec::new());
        *fallback.status_mut() = StatusCode::NOT_FOUND;
        fallback
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn both_webview_forms_name_the_id() {
        assert_eq!(requested_id(&format!("rr-art://bg/{ID}")).as_deref(), Some(ID));
        assert_eq!(
            requested_id(&format!("http://rr-art.localhost/bg/{ID}")).as_deref(),
            Some(ID)
        );
        assert_eq!(requested_id(&format!("rr-art://fg/{ID}")), None);
        assert_eq!(requested_id(&format!("http://rr-art.localhost/fg/{ID}")), None);
        assert_eq!(requested_id(&format!("http://evil.localhost/bg/{ID}")), None);
        assert_eq!(requested_id(&format!("rr-art://bg/{}", ID.to_uppercase())), None);
        assert_eq!(requested_id("rr-art://bg/0123"), None);
        assert_eq!(requested_id(&format!("rr-art://bg/{ID}/x")), None);
    }

    #[tokio::test]
    async fn serves_registered_files_and_404s_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bg.png");
        std::fs::write(&file, b"png bytes").unwrap();
        let store = ArtStore::default();
        store.insert(ID, &file, ImageType::Png);
        store.insert("not-an-id", &file, ImageType::Png);

        let found = respond(&store, &format!("rr-art://bg/{ID}")).await;
        assert_eq!(found.status(), StatusCode::OK);
        assert_eq!(found.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(found.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(found.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(found.body(), b"png bytes");

        let other = "f".repeat(64);
        let missing = respond(&store, &format!("rr-art://bg/{other}")).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert!(missing.body().is_empty());
        assert!(missing.headers().get(header::CONTENT_TYPE).is_none());
        assert_eq!(missing.headers()[header::CACHE_CONTROL], "no-store");

        std::fs::remove_file(&file).unwrap();
        let gone = respond(&store, &format!("http://rr-art.localhost/bg/{ID}")).await;
        assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    }
}
