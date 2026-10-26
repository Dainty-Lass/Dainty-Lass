//! [`Registrar`]: what a module asks of the core plugins (DD3) as data, so it crosses the pipe as it is
//! and the host checks it the same way whichever side built it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ids::check_name;
use crate::manifest::{ModuleManifest, Transport, is_relative_path, is_text};
use crate::module::{CoreCapability, EventName};

/// Bounds on a registration, which arrives from an external module's process.
pub mod limits {
    pub const ROUTES: usize = 64;
    pub const PREFIX: usize = 256;
    pub const CONTENT_FILES: usize = 1024;
    pub const COMMANDS: usize = 64;
    pub const ARGS: usize = 16;
    pub const HELP: usize = 256;
    pub const EVENTS: usize = 128;
    pub const SECTIONS: usize = 32;
    pub const KEYS: usize = 64;
}

/// A path prefix the gateway sends to one of the module's loopback ports (DF4a). The gateway still
/// refuses host-only routes from outside, whatever a module registers (AGENTS.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSpec {
    pub prefix: String,
    /// A TCP port named in the manifest
    pub port: String,
    #[serde(default)]
    pub host_only: bool,
}

/// A file content serves for the module's game version (DF4b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentFile {
    /// Where content serves it, relative to the version's folder
    pub path: String,
    /// Where it is, relative to the module's folder
    pub source: String,
    pub media_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgSpec {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    pub help: String,
}

/// A command the console, TUI, server GUI and admin API offer (DF3). The UI draws it from this (DD11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub args: Vec<ArgSpec>,
}

impl CommandSpec {
    pub fn new(name: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, name: impl Into<String>, required: bool, help: impl Into<String>) -> Self {
        self.args.push(ArgSpec {
            name: name.into(),
            required,
            help: help.into(),
        });
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminAction {
    Kick,
    Ban,
    Lookup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigKey {
    pub name: String,
    pub default: serde_json::Value,
    pub help: String,
}

/// A `host.toml` section the module reads, `[modules.<id>.<name>]` (DF3), with its defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSection {
    pub name: String,
    #[serde(default)]
    pub keys: Vec<ConfigKey>,
}

impl ConfigSection {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            keys: Vec::new(),
        }
    }

    pub fn key(mut self, name: impl Into<String>, default: serde_json::Value, help: impl Into<String>) -> Self {
        self.keys.push(ConfigKey {
            name: name.into(),
            default,
            help: help.into(),
        });
        self
    }
}

/// Everything a module registered.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    #[serde(default)]
    pub routes: Vec<RouteSpec>,
    /// UDP ports named in the manifest that the allowlist opens to admitted players
    #[serde(default)]
    pub udp_ports: Vec<String>,
    #[serde(default)]
    pub content: Vec<ContentFile>,
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    #[serde(default)]
    pub emits: Vec<EventName>,
    #[serde(default)]
    pub subscribes: Vec<EventName>,
    #[serde(default)]
    pub admin_actions: Vec<AdminAction>,
    #[serde(default)]
    pub config: Vec<ConfigSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("registration: {0}")]
pub struct RegistrationError(pub String);

fn fail<T>(message: impl Into<String>) -> Result<T, RegistrationError> {
    Err(RegistrationError(message.into()))
}

/// `host.toml` keys are snake_case (AGENTS.md).
fn is_snake(text: &str) -> bool {
    let bytes = text.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// An absolute path of unreserved URL characters, with no empty, `.` or `..` segment.
fn is_route_prefix(prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    let chars_ok = prefix.len() <= limits::PREFIX
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~'));
    chars_ok
        && prefix
            .strip_prefix('/')
            .is_some_and(|rest| rest.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != ".."))
}

fn is_media_type(text: &str) -> bool {
    let token = |t: &str| {
        !t.is_empty()
            && t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'+' | b'_'))
    };
    text.len() <= 128
        && text
            .split_once('/')
            .is_some_and(|(kind, sub)| token(kind) && token(sub))
}

fn unique<'a, T: Ord + 'a>(items: impl IntoIterator<Item = &'a T>, what: &str) -> Result<(), RegistrationError> {
    let mut seen = BTreeSet::new();
    if items.into_iter().all(|item| seen.insert(item)) {
        Ok(())
    } else {
        fail(format!("{what} given twice"))
    }
}

impl Registration {
    /// Checks every entry against the bounds and the manifest. The host runs it on every registration,
    /// compiled in or not: an external module's is untrusted input.
    pub fn check(&self, manifest: &ModuleManifest) -> Result<(), RegistrationError> {
        let port_of = |name: &str, transport: Transport| manifest.port(name).is_some_and(|p| p.transport == transport);
        if self.routes.len() > limits::ROUTES {
            return fail(format!("at most {} routes", limits::ROUTES));
        }
        for route in &self.routes {
            if !is_route_prefix(&route.prefix) {
                return fail(format!(
                    "route {:?}: an absolute path with no empty, . or .. segment",
                    route.prefix
                ));
            }
            if !port_of(&route.port, Transport::Tcp) {
                return fail(format!(
                    "route {}: {:?} is not a TCP port in the manifest",
                    route.prefix, route.port
                ));
            }
        }
        unique(self.routes.iter().map(|r| &r.prefix), "a route")?;
        for name in &self.udp_ports {
            if !port_of(name, Transport::Udp) {
                return fail(format!("{name:?} is not a UDP port in the manifest"));
            }
        }
        unique(&self.udp_ports, "a UDP port")?;
        if self.content.len() > limits::CONTENT_FILES {
            return fail(format!("at most {} content files", limits::CONTENT_FILES));
        }
        for file in &self.content {
            if !is_relative_path(&file.path) || !is_relative_path(&file.source) || !is_media_type(&file.media_type) {
                return fail(format!("content file {:?}: relative paths and a media type", file.path));
            }
        }
        unique(self.content.iter().map(|f| &f.path), "a content path")?;
        self.check_commands()?;
        if self.emits.len() > limits::EVENTS || self.subscribes.len() > limits::EVENTS {
            return fail(format!("at most {} events each way", limits::EVENTS));
        }
        unique(&self.emits, "an emitted event")?;
        unique(&self.subscribes, "a subscription")?;
        unique(&self.admin_actions, "an admin action")?;
        self.check_config()
    }

    fn check_commands(&self) -> Result<(), RegistrationError> {
        if self.commands.len() > limits::COMMANDS {
            return fail(format!("at most {} commands", limits::COMMANDS));
        }
        for command in &self.commands {
            let ok = check_name(&command.name, "").is_ok()
                && is_text(&command.summary, limits::HELP)
                && command.args.len() <= limits::ARGS
                && command
                    .args
                    .iter()
                    .all(|arg| check_name(&arg.name, "").is_ok() && is_text(&arg.help, limits::HELP));
            if !ok {
                return fail(format!(
                    "command {:?}: a name, a one-line summary, at most {} args",
                    command.name,
                    limits::ARGS
                ));
            }
            unique(command.args.iter().map(|a| &a.name), "an argument")?;
        }
        unique(self.commands.iter().map(|c| &c.name), "a command")
    }

    fn check_config(&self) -> Result<(), RegistrationError> {
        if self.config.len() > limits::SECTIONS {
            return fail(format!("at most {} config sections", limits::SECTIONS));
        }
        for section in &self.config {
            let ok = is_snake(&section.name)
                && section.keys.len() <= limits::KEYS
                && section
                    .keys
                    .iter()
                    .all(|key| is_snake(&key.name) && is_text(&key.help, limits::HELP));
            if !ok {
                return fail(format!(
                    "config section {:?}: snake_case names and one-line help",
                    section.name
                ));
            }
            unique(section.keys.iter().map(|k| &k.name), "a config key")?;
        }
        unique(self.config.iter().map(|s| &s.name), "a config section")
    }

    /// The core capabilities this registration needs served.
    pub fn needs(&self) -> BTreeSet<CoreCapability> {
        let pairs = [
            (CoreCapability::Routes, !self.routes.is_empty()),
            (CoreCapability::UdpPorts, !self.udp_ports.is_empty()),
            (CoreCapability::ContentFiles, !self.content.is_empty()),
            (CoreCapability::Commands, !self.commands.is_empty()),
            (
                CoreCapability::Events,
                !self.emits.is_empty() || !self.subscribes.is_empty(),
            ),
            (CoreCapability::AdminActions, !self.admin_actions.is_empty()),
            (CoreCapability::ConfigSections, !self.config.is_empty()),
        ];
        pairs
            .into_iter()
            .filter_map(|(cap, needed)| needed.then_some(cap))
            .collect()
    }
}

/// Collects a module's registration; [`Registrar::finish`] checks it against the manifest.
#[derive(Debug, Clone)]
pub struct Registrar {
    manifest: ModuleManifest,
    registration: Registration,
}

impl Registrar {
    pub fn new(manifest: ModuleManifest) -> Self {
        Self {
            manifest,
            registration: Registration::default(),
        }
    }

    pub fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    pub fn route(&mut self, prefix: impl Into<String>, port: impl Into<String>) -> &mut Self {
        self.registration.routes.push(RouteSpec {
            prefix: prefix.into(),
            port: port.into(),
            host_only: false,
        });
        self
    }

    /// A route the gateway serves only to the host's own tools, never from outside.
    pub fn host_only_route(&mut self, prefix: impl Into<String>, port: impl Into<String>) -> &mut Self {
        self.registration.routes.push(RouteSpec {
            prefix: prefix.into(),
            port: port.into(),
            host_only: true,
        });
        self
    }

    pub fn udp_ports(&mut self, port: impl Into<String>) -> &mut Self {
        self.registration.udp_ports.push(port.into());
        self
    }

    pub fn content_file(
        &mut self,
        path: impl Into<String>,
        source: impl Into<String>,
        media_type: impl Into<String>,
    ) -> &mut Self {
        self.registration.content.push(ContentFile {
            path: path.into(),
            source: source.into(),
            media_type: media_type.into(),
        });
        self
    }

    pub fn command(&mut self, command: CommandSpec) -> &mut Self {
        self.registration.commands.push(command);
        self
    }

    pub fn emits(&mut self, event: EventName) -> &mut Self {
        self.registration.emits.push(event);
        self
    }

    pub fn subscribes(&mut self, event: EventName) -> &mut Self {
        self.registration.subscribes.push(event);
        self
    }

    pub fn admin_action(&mut self, action: AdminAction) -> &mut Self {
        self.registration.admin_actions.push(action);
        self
    }

    pub fn config_section(&mut self, section: ConfigSection) -> &mut Self {
        self.registration.config.push(section);
        self
    }

    pub fn finish(self) -> Result<Registration, RegistrationError> {
        self.registration.check(&self.manifest)?;
        Ok(self.registration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::StubModule;
    use crate::{GameModule, ModuleConfig};

    type Case<T> = (&'static str, Box<dyn Fn(&mut T)>);

    fn registrar() -> Registrar {
        Registrar::new(StubModule::new().manifest())
    }

    #[test]
    fn the_stub_registers_cleanly_and_its_defaults_follow() {
        let stub = StubModule::new();
        let mut registrar = Registrar::new(stub.manifest());
        stub.register(&mut registrar).unwrap();
        let registration = registrar.finish().unwrap();
        assert!(registration.needs().contains(&CoreCapability::Routes));
        let config = ModuleConfig::defaults(&stub.manifest(), &registration);
        assert_eq!(config.get("stub", "greeting"), Some(&serde_json::json!("hello")));
        assert!(config.ports.contains_key("http"));
        let json = serde_json::to_string(&registration).unwrap();
        assert_eq!(serde_json::from_str::<Registration>(&json).unwrap(), registration);
    }

    #[test]
    fn bad_entries_are_refused() {
        let cases: Vec<Case<Registrar>> = vec![
            ("relative route", Box::new(|r| _ = r.route("stub", "http"))),
            ("dot route", Box::new(|r| _ = r.route("/a/../b", "http"))),
            ("empty segment", Box::new(|r| _ = r.route("/a//b", "http"))),
            ("trailing slash", Box::new(|r| _ = r.route("/a/", "http"))),
            ("query", Box::new(|r| _ = r.route("/a?b", "http"))),
            ("unknown port", Box::new(|r| _ = r.route("/a", "nope"))),
            ("udp as tcp", Box::new(|r| _ = r.route("/a", "game"))),
            ("tcp as udp", Box::new(|r| _ = r.udp_ports("http"))),
            (
                "route twice",
                Box::new(|r| _ = r.route("/a", "http").route("/a", "http")),
            ),
            (
                "escaping file",
                Box::new(|r| _ = r.content_file("../x", "x", "text/plain")),
            ),
            (
                "escaping source",
                Box::new(|r| _ = r.content_file("x", "/etc/passwd", "text/plain")),
            ),
            ("bad media type", Box::new(|r| _ = r.content_file("x", "x", "text"))),
            (
                "bad command",
                Box::new(|r| _ = r.command(CommandSpec::new("Echo", "x"))),
            ),
            (
                "multiline summary",
                Box::new(|r| _ = r.command(CommandSpec::new("echo", "a\nb"))),
            ),
            (
                "arg twice",
                Box::new(|r| _ = r.command(CommandSpec::new("echo", "x").arg("a", true, "a").arg("a", false, "a"))),
            ),
            (
                "kebab section",
                Box::new(|r| _ = r.config_section(ConfigSection::new("my-section"))),
            ),
            (
                "action twice",
                Box::new(|r| _ = r.admin_action(AdminAction::Kick).admin_action(AdminAction::Kick)),
            ),
        ];
        for (what, change) in cases {
            let mut registrar = registrar();
            change(&mut registrar);
            assert!(registrar.finish().is_err(), "{what}");
        }
        let mut registrar = registrar();
        registrar.route("/", "http").route("/a/b.c~d", "http").udp_ports("game");
        registrar.content_file("profiles/client.json", "data/client.json", "application/json");
        assert!(registrar.finish().is_ok());
    }
}
