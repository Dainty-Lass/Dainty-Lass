// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! External links: the fixed URL of every target the page may open, the check every URL passes before the OS
//! opens it, and the opening itself (spec §2.7.9, §2.4.10).

use std::io;
use std::path::Path;

use url::Url;

use crate::bridge::spec::fits;
use crate::bridge::types::ExternalTarget;

/// This project's repository; also the server source when a status names none.
pub const PROJECT_URL: &str = "https://github.com/SilentSpud/DaintyLass";
/// Longest URL opened, in UTF-16 units.
pub const MAX_URL: usize = 1024;
/// The one host any of whose pages may open: Tailscale share links are minted per server.
pub const TAILSCALE_LOGIN_HOST: &str = "login.tailscale.com";

const MYSTIC_URL: &str = "https://github.com/pranav158/Mystic-Paradox";
const DUMPER7_URL: &str = "https://github.com/Encryqed/Dumper-7";

// A file on the repository's main branch.
macro_rules! project_file {
    ($path:literal) => {
        concat!("https://github.com/SilentSpud/DaintyLass/blob/main/", $path)
    };
}

/// Every target's fixed URL; `None` for the two that come from the server.
#[must_use]
pub fn fixed_url(target: ExternalTarget) -> Option<&'static str> {
    use ExternalTarget as T;
    Some(match target {
        T::TailscaleShare | T::ServerSource => return None,
        T::TailscaleDownload => "https://tailscale.com/download",
        T::VcRedist => "https://aka.ms/vs/17/release/vc_redist.x64.exe",
        T::ProjectSource => PROJECT_URL,
        T::ProjectLicense => project_file!("README.md#credits-and-licence"),
        T::ProjectIssues => "https://github.com/SilentSpud/DaintyLass/issues",
        T::ProjectDocsPlayers => {
            project_file!("restricted/docs/players.md")
        }
        T::ProjectDocsHosting => {
            project_file!("restricted/docs/hosting.md")
        }
        T::ProjectDocsSdk => {
            project_file!("docs/sdk-and-game-data.md")
        }
        T::InstallingGuide => "https://github.com/pranav158/Mystic-Paradox/blob/main/docs/INSTALLING_DAUNTLESS.md",
        T::Legendary => "https://github.com/legendary-gl/legendary/releases/latest",
        T::Dumper7 => DUMPER7_URL,
        T::Dumper7Zip => "https://github.com/Encryqed/Dumper-7/archive/refs/heads/main.zip",
        T::VisualStudio => "https://visualstudio.microsoft.com/downloads/",
        T::VisualStudioInstaller => "https://aka.ms/vs/17/release/vs_community.exe",
        T::Rust => "https://www.rust-lang.org/tools/install",
        T::RustInstaller => "https://win.rustup.rs/x86_64",
        T::Nodejs => "https://nodejs.org/en/download",
        T::Mongodb => "https://www.mongodb.com/try/download/community",
        T::Git => "https://git-scm.com/download/win",
        T::RevivedSource => "https://github.com/mixutin/dauntless-revived",
        T::MysticSource => MYSTIC_URL,
        T::UndauntedSource => "https://github.com/SyST3MDeV/Undaunted",
    })
}

/// Every fixed URL.
pub fn fixed_urls() -> impl Iterator<Item = &'static str> {
    ALL_TARGETS.iter().filter_map(|t| fixed_url(*t))
}

const ALL_TARGETS: [ExternalTarget; 24] = {
    use ExternalTarget as T;
    [
        T::TailscaleDownload,
        T::TailscaleShare,
        T::VcRedist,
        T::ServerSource,
        T::ProjectSource,
        T::ProjectLicense,
        T::ProjectIssues,
        T::ProjectDocsPlayers,
        T::ProjectDocsHosting,
        T::ProjectDocsSdk,
        T::InstallingGuide,
        T::Legendary,
        T::Dumper7,
        T::VisualStudio,
        T::Rust,
        T::Nodejs,
        T::Mongodb,
        T::Git,
        T::VisualStudioInstaller,
        T::RustInstaller,
        T::Dumper7Zip,
        T::RevivedSource,
        T::MysticSource,
        T::UndauntedSource,
    ]
};

/// The URL `open_external(target)` opens, checked: the share link and source URL are the current server's.
#[must_use]
pub fn target_url(target: ExternalTarget, share: Option<&str>, source: Option<&str>) -> Option<Url> {
    let raw = match target {
        ExternalTarget::TailscaleShare => share?,
        ExternalTarget::ServerSource => source.unwrap_or(PROJECT_URL),
        other => fixed_url(other)?,
    };
    check(raw, source)
}

/// The link check: `text` may open when it is a fixed URL, a page on [`TAILSCALE_LOGIN_HOST`], or exactly the
/// status's `source` URL. Always `https`, never with a user or password, and a port only on the source URL.
#[must_use]
pub fn check(text: &str, source: Option<&str>) -> Option<Url> {
    if !fits(text, MAX_URL) {
        return None;
    }
    let url = Url::parse(text).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if source == Some(text) {
        return Some(url);
    }
    if has_explicit_port(text) {
        return None;
    }
    let allowed = fixed_urls().any(|fixed| fixed == text) || url.host_str() == Some(TAILSCALE_LOGIN_HOST);
    allowed.then_some(url)
}

/// The sign-in page `OAuthStart` named: `https`, no user or password, and for the Discord preset only
/// `discord.com` itself, so a server can't send the player's browser elsewhere in Discord's name.
#[must_use]
pub fn check_oauth(text: &str, provider: &str) -> Option<Url> {
    if !fits(text, MAX_URL) {
        return None;
    }
    let url = Url::parse(text).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.host_str().is_none() {
        return None;
    }
    if provider == "discord" && (url.host_str() != Some("discord.com") || has_explicit_port(text)) {
        return None;
    }
    Some(url)
}

// Url drops a default port, so `:443` is only visible in the text.
fn has_explicit_port(text: &str) -> bool {
    let rest = text.split_once("://").map_or(text, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let after_host = match host_port.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map_or("", |(_, tail)| tail),
        None => host_port,
    };
    after_host.contains(':')
}

/// Opens a checked URL with the OS's handler, without waiting for it.
pub fn open_url(url: &Url) -> io::Result<()> {
    if cfg!(windows) {
        // url.dll takes the URL as one argument, where a shell `start` would read `&` as a command separator.
        spawn_detached("rundll32.exe", &["url.dll,FileProtocolHandler", url.as_str()])
    } else if cfg!(target_os = "macos") {
        spawn_detached("open", &[url.as_str()])
    } else {
        spawn_detached("xdg-open", &[url.as_str()])
    }
}

/// Opens `dir` in the file manager; `false` when it is not a directory.
pub fn open_folder(dir: &Path) -> io::Result<bool> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let program = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    spawn_detached(program, &[dir.as_os_str()])?;
    Ok(true)
}

fn spawn_detached<S: AsRef<std::ffi::OsStr>>(program: &str, args: &[S]) -> io::Result<()> {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    // Reaped off-thread so a long-lived handler never leaves a zombie behind.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::spec::EXTERNAL_TARGETS;

    #[test]
    fn oauth_pages() {
        let discord = "https://discord.com/oauth2/authorize?client_id=1";
        assert!(check_oauth(discord, "discord").is_some());
        for refused in [
            "http://discord.com/oauth2/authorize",
            "https://discord.com.evil.example/oauth2/authorize",
            "https://evil.example/?discord.com",
            "https://user@discord.com/oauth2/authorize",
            "https://discord.com:8443/oauth2/authorize",
            "javascript:alert(1)",
        ] {
            assert!(check_oauth(refused, "discord").is_none(), "{refused}");
        }
        assert!(check_oauth("https://sso.example/authorize", "my-sso").is_some());
        assert!(check_oauth("http://sso.example/authorize", "my-sso").is_none());
    }

    fn target(name: &str) -> ExternalTarget {
        serde_json::from_value(serde_json::json!(name)).unwrap()
    }

    #[test]
    fn every_target_is_listed_once() {
        let names: Vec<String> = ALL_TARGETS
            .iter()
            .map(|t| serde_json::to_value(t).unwrap().as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, EXTERNAL_TARGETS);
    }

    #[test]
    fn every_fixed_url() {
        let expected = [
            ("tailscale_download", "https://tailscale.com/download"),
            ("vc_redist", "https://aka.ms/vs/17/release/vc_redist.x64.exe"),
            ("project_source", "https://github.com/SilentSpud/DaintyLass"),
            (
                "project_license",
                "https://github.com/SilentSpud/DaintyLass/blob/main/README.md#credits-and-licence",
            ),
            ("project_issues", "https://github.com/SilentSpud/DaintyLass/issues"),
            (
                "project_docs_players",
                "https://github.com/SilentSpud/DaintyLass/blob/main/restricted/docs/players.md",
            ),
            (
                "project_docs_hosting",
                "https://github.com/SilentSpud/DaintyLass/blob/main/restricted/docs/hosting.md",
            ),
            (
                "project_docs_sdk",
                "https://github.com/SilentSpud/DaintyLass/blob/main/docs/sdk-and-game-data.md",
            ),
            (
                "installing_guide",
                &format!("{MYSTIC_URL}/blob/main/docs/INSTALLING_DAUNTLESS.md"),
            ),
            ("legendary", "https://github.com/legendary-gl/legendary/releases/latest"),
            ("dumper7", DUMPER7_URL),
            ("dumper7_zip", &format!("{DUMPER7_URL}/archive/refs/heads/main.zip")),
            ("visual_studio", "https://visualstudio.microsoft.com/downloads/"),
            (
                "visual_studio_installer",
                "https://aka.ms/vs/17/release/vs_community.exe",
            ),
            ("rust", "https://www.rust-lang.org/tools/install"),
            ("rust_installer", "https://win.rustup.rs/x86_64"),
            ("nodejs", "https://nodejs.org/en/download"),
            ("mongodb", "https://www.mongodb.com/try/download/community"),
            ("git", "https://git-scm.com/download/win"),
            ("revived_source", "https://github.com/mixutin/dauntless-revived"),
            ("mystic_source", MYSTIC_URL),
            ("undaunted_source", "https://github.com/SyST3MDeV/Undaunted"),
        ];
        for (name, url) in expected {
            assert_eq!(fixed_url(target(name)), Some(url), "{name}");
            assert!(check(url, None).is_some(), "{name} passes the check");
            assert_eq!(
                target_url(target(name), None, None).unwrap().as_str(),
                Url::parse(url).unwrap().as_str()
            );
        }
        assert_eq!(fixed_urls().count(), expected.len());
        assert_eq!(fixed_url(ExternalTarget::TailscaleShare), None);
        assert_eq!(fixed_url(ExternalTarget::ServerSource), None);
    }

    #[test]
    fn server_side_targets() {
        let share = "https://login.tailscale.com/admin/invite/abc123";
        assert_eq!(
            target_url(ExternalTarget::TailscaleShare, Some(share), None)
                .unwrap()
                .as_str(),
            share
        );
        assert_eq!(target_url(ExternalTarget::TailscaleShare, None, None), None);
        assert_eq!(
            target_url(ExternalTarget::TailscaleShare, Some("https://evil.example/"), None),
            None
        );
        let source = "https://code.example.org:8443/server";
        assert_eq!(
            target_url(ExternalTarget::ServerSource, None, Some(source))
                .unwrap()
                .as_str(),
            source
        );
        assert_eq!(
            target_url(ExternalTarget::ServerSource, None, None).unwrap().as_str(),
            PROJECT_URL
        );
        assert_eq!(
            target_url(ExternalTarget::ServerSource, None, Some("http://code.example.org/")),
            None
        );
    }

    #[test]
    fn allowed_links() {
        assert!(check("https://login.tailscale.com/admin/machines", None).is_some());
        assert!(check("https://LOGIN.tailscale.com/x", None).is_some());
        assert!(check("https://login.tailscale.com/x?y=1#z", None).is_some());
        let source = "https://src.example.org:8443/a";
        assert!(check(source, Some(source)).is_some());
        assert!(check("https://src.example.org/a", Some("https://src.example.org/a")).is_some());
    }

    #[test]
    fn refused_links() {
        let refused = [
            "",
            "not a url",
            "http://tailscale.com/download",
            "ftp://login.tailscale.com/x",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "dauntless://join?v=1",
            "https://user@login.tailscale.com/x",
            "https://user:pw@login.tailscale.com/x",
            "https://:pw@login.tailscale.com/x",
            "https://login.tailscale.com:443/x",
            "https://login.tailscale.com:8443/x",
            "https://tailscale.com/download/",
            "https://tailscale.com/download?x=1",
            "https://login.tailscale.com.evil.example/x",
            "https://evil.example/login.tailscale.com",
            "https://example.com/",
            "https://src.example.org:8443/b",
        ];
        for url in refused {
            assert!(check(url, Some("https://src.example.org:8443/a")).is_none(), "{url}");
        }
        let long = format!("https://login.tailscale.com/{}", "a".repeat(MAX_URL));
        assert!(check(&long, None).is_none());
        let source = "http://src.example.org/a";
        assert!(
            check(source, Some(source)).is_none(),
            "a source URL is still https only"
        );
        let with_user = "https://u@src.example.org/a";
        assert!(check(with_user, Some(with_user)).is_none());
    }

    #[test]
    fn explicit_ports() {
        assert!(has_explicit_port("https://a.example:443/"));
        assert!(has_explicit_port("https://u:p@a.example:1/"));
        assert!(has_explicit_port("https://[::1]:8443/"));
        assert!(!has_explicit_port("https://[::1]/"));
        assert!(!has_explicit_port("https://u:p@a.example/x:y"));
        assert!(!has_explicit_port("https://a.example?x=:1"));
    }

    #[test]
    fn folders_open_only_when_they_are_directories() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(!open_folder(&file).unwrap());
        assert!(!open_folder(&dir.path().join("missing")).unwrap());
    }
}
