//! Which program runs each service. The backend and the Director are Node until RB14. The gateway,
//! the content server and the allowlist helper move to Rust one row at a time: [`rust_ported`] is the
//! one switch each row flips. A ported service runs inside this binary (RD26): in-process under `start`,
//! or as `<this binary> <service>` where it needs a process of its own (the legacy plan, `--close-ports`).

use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::paths::Paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontService {
    Gateway,
    Content,
    Allowlist,
}

/// Whether the service's Rust build replaces its Node build (MIGRATION_PLAN.md section 4.3). Flipping
/// the gateway or the content server is the front-door cutover (RS5), the maintainer's step.
pub fn rust_ported(service: FrontService) -> bool {
    match service {
        // RS3 flips this.
        FrontService::Gateway => false,
        // RS2 flips this.
        FrontService::Content => false,
        // RS4: DR's allowlist with Reforged's port range and nftables variant.
        FrontService::Allowlist => true,
    }
}

impl FrontService {
    pub fn bin_name(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Content => "content",
            Self::Allowlist => "allowlist",
        }
    }

    /// Whether this binary was built with the service's Rust code.
    pub fn built_in(self) -> bool {
        crate::BUILT_IN_SERVICES.contains(&self.bin_name())
    }

    /// The service's arguments after `<this binary>`: its name, then its env file (DR X7).
    fn rust_args(self, env_file: &Path) -> Vec<String> {
        let env_file = env_file.display().to_string();
        match self {
            // The content service takes the file on its serve command (`content --env-file` is refused).
            Self::Gateway | Self::Content => {
                vec![self.bin_name().into(), "serve".into(), "--env-file".into(), env_file]
            }
            Self::Allowlist => vec![self.bin_name().into(), "--env-file".into(), env_file],
        }
    }

    fn node_dir(self, paths: &Paths) -> PathBuf {
        match self {
            Self::Gateway | Self::Allowlist => paths.gateway_dir.clone(),
            Self::Content => paths.content_dir.clone(),
        }
    }

    fn node_script(self) -> PathBuf {
        match self {
            Self::Gateway | Self::Content => ["dist", "server.js"].iter().collect(),
            Self::Allowlist => ["dist", "allowlist", "server.js"].iter().collect(),
        }
    }
}

/// How to start one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The env file is in `args` (a Rust service); otherwise it is read into the environment.
    pub env_file_in_args: bool,
}

/// Where the programs are: Node, this binary (which runs the Rust services) and the folders searched
/// for the Rust binaries that are not part of it yet (a legacy server's metagame and deploy server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Programs {
    pub node: PathBuf,
    pub host: PathBuf,
    pub rust_dirs: Vec<PathBuf>,
}

impl Programs {
    /// Node from PATH (or where its Windows installer and nvm-windows put it); the rest as [`Self::find_rust`].
    pub fn find(paths: &Paths) -> Result<Self, HostError> {
        let node = find_node(&std::env::vars().collect::<Vec<_>>(), Path::exists).ok_or_else(|| {
            HostError::message(
                "Node.js (node) was not found: the backend and the Director still run on it. Install Node.js 22 or newer from https://nodejs.org",
            )
        })?;
        Ok(Self {
            node,
            ..Self::find_rust(paths)
        })
    }

    /// This binary, and the folders with other Rust binaries: next to it (the server release), then the
    /// workspace's `rust/target/release` and `debug`. `node` is left as the bare name.
    pub fn find_rust(paths: &Paths) -> Self {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("reforged-host"));
        let mut rust_dirs = Vec::new();
        if let Some(dir) = exe.parent() {
            rust_dirs.push(dir.to_path_buf());
        }
        for profile in ["release", "debug"] {
            let dir = paths.repo.join("rust").join("target").join(profile);
            if !rust_dirs.contains(&dir) {
                rust_dirs.push(dir);
            }
        }
        let host = host_cli(&exe, paths, &rust_dirs);
        Self {
            node: PathBuf::from("node"),
            host,
            rust_dirs,
        }
    }

    pub fn rust_bin(&self, name: &str) -> Option<PathBuf> {
        let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        self.rust_dirs.iter().map(|d| d.join(&file)).find(|f| f.is_file())
    }

    /// The service's Rust build whether or not [`rust_ported`] says so yet: what a legacy server runs,
    /// since only the Rust services carry the `legacy` features. `None` when this binary lacks it.
    pub fn launch_rust(&self, service: FrontService, paths: &Paths, env_file: &Path) -> Option<Launch> {
        service.built_in().then(|| Launch {
            command: self.host.clone(),
            args: service.rust_args(env_file),
            cwd: paths.workspace.clone(),
            env_file_in_args: true,
        })
    }

    /// The program for `service`, given its env file.
    pub fn launch(&self, service: FrontService, paths: &Paths, env_file: &Path) -> Launch {
        if rust_ported(service)
            && let Some(launch) = self.launch_rust(service, paths, env_file)
        {
            return launch;
        }
        Launch {
            command: self.node.clone(),
            args: vec![service.node_script().display().to_string()],
            cwd: service.node_dir(paths),
            env_file_in_args: false,
        }
    }
}

/// The binary a service runs from on its own (`<it> gateway serve ...`): this one when it has the host
/// kit's command line, else `reforged-host` (or `reforged-server-console`) in the release's `bin/` or a
/// build. `reforged-server-minimal` and the server GUI have no such commands.
fn host_cli(exe: &Path, paths: &Paths, rust_dirs: &[PathBuf]) -> PathBuf {
    const CLIS: [&str; 2] = ["reforged-host", "reforged-server-console"];
    let stem = exe
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if CLIS.contains(&stem.as_str()) {
        return exe.to_path_buf();
    }
    let dirs = std::iter::once(paths.repo.join("bin")).chain(rust_dirs.iter().cloned());
    dirs.flat_map(|dir| CLIS.map(|name| dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))))
        .find(|file| file.is_file())
        .unwrap_or_else(|| exe.to_path_buf())
}

/// `node` on PATH, then in `%ProgramFiles%\nodejs` and nvm-windows' link (the launcher's `findNode`).
pub fn find_node(env: &[(String, String)], exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let windows = cfg!(windows);
    let get = |name: &str| {
        env.iter()
            .find(|(k, _)| {
                if windows {
                    k.eq_ignore_ascii_case(name)
                } else {
                    k == name
                }
            })
            .map(|(_, v)| v.clone())
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(path) = get("PATH") {
        for dir in std::env::split_paths(&path) {
            let text = dir.to_string_lossy();
            let text = text.trim().trim_matches('"');
            if !text.is_empty() {
                dirs.push(PathBuf::from(text));
            }
        }
    }
    if windows {
        for base in [
            get("ProgramW6432"),
            get("ProgramFiles"),
            Some("C:\\Program Files".into()),
        ]
        .into_iter()
        .flatten()
        {
            dirs.push(Path::new(&base).join("nodejs"));
        }
        if let Some(link) = get("NVM_SYMLINK") {
            dirs.push(PathBuf::from(link));
        }
    }
    let exe = if windows { "node.exe" } else { "node" };
    dirs.into_iter().map(|d| d.join(exe)).find(|f| exists(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn programs(dir: &Path) -> Programs {
        Programs {
            node: PathBuf::from("node"),
            host: dir.join("reforged-host"),
            rust_dirs: vec![dir.join("release"), dir.join("debug")],
        }
    }

    #[test]
    fn runs_a_ported_service_from_this_binary_when_it_is_built_in_and_node_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let env_file = paths.config.join("allowlist.env");
        let p = programs(dir.path());
        let launch = p.launch(FrontService::Allowlist, &paths, &env_file);
        if FrontService::Allowlist.built_in() {
            assert_eq!(launch.command, dir.path().join("reforged-host"));
            assert_eq!(
                launch.args,
                [
                    "allowlist".to_string(),
                    "--env-file".into(),
                    env_file.display().to_string()
                ]
            );
            assert!(launch.env_file_in_args);
        } else {
            assert_eq!(launch.command, PathBuf::from("node"));
            assert_eq!(launch.cwd, paths.gateway_dir);
            assert!(!launch.env_file_in_args);
        }
    }

    #[test]
    fn keeps_a_service_that_is_not_ported_yet_on_node_even_when_it_is_built_in() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let p = programs(dir.path());
        assert!(!rust_ported(FrontService::Gateway) && !rust_ported(FrontService::Content));
        let gateway = p.launch(FrontService::Gateway, &paths, &paths.config.join("gateway.env"));
        assert_eq!(gateway.command, PathBuf::from("node"));
        assert_eq!(gateway.cwd, paths.gateway_dir);
        let content = p.launch(FrontService::Content, &paths, &paths.config.join("content.env"));
        assert_eq!(content.cwd, paths.content_dir);
        assert_eq!(
            content.args,
            vec![Path::new("dist").join("server.js").display().to_string()]
        );
    }

    #[test]
    fn gives_the_rust_gateway_and_content_their_serve_command() {
        let args = FrontService::Gateway.rust_args(Path::new("g.env"));
        assert_eq!(args, ["gateway", "serve", "--env-file", "g.env"]);
        let args = FrontService::Content.rust_args(Path::new("c.env"));
        assert_eq!(args, ["content", "serve", "--env-file", "c.env"]);
    }

    #[test]
    fn finds_node_on_path_then_in_the_usual_places() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let exe = if cfg!(windows) { "node.exe" } else { "node" };
        let env = vec![("PATH".to_string(), format!("/a{sep}\"/b\"{sep}"))];
        let found = find_node(&env, |p| p == Path::new("/b").join(exe));
        assert_eq!(found, Some(Path::new("/b").join(exe)));
        assert_eq!(find_node(&env, |_| false), None);
        if cfg!(windows) {
            let env = vec![("ProgramFiles".to_string(), "D:\\PF".to_string())];
            let want = Path::new("D:\\PF").join("nodejs").join(exe);
            assert_eq!(find_node(&env, |p| p == want), Some(want.clone()));
        }
    }
}
