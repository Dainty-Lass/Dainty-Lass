// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The allowlist as the core plugin that guards game modules' UDP ranges (DD3, DF4d). A module that
//! registers a UDP port gets its range into the firewall rule before it starts; unloading it takes the range
//! out again. The ranges are the ones the controller assigned (it moves a clash, DD4), checked against what
//! the manifest declared, so a module cannot ask for a range of its own choosing.
//!
//! Without a helper in this process (the allowlist runs as a process of its own, or not at all), a module
//! that registers a UDP port is refused: its ports would be open to everyone.

use std::sync::Arc;

use module_api::manifest::Transport;
use module_api::{
    CoreCapability, CorePlugin, ErrorKind, ModuleError, ModuleId, ModuleManifest, PortRange, Registration,
};

use super::guard::Guard;

/// What the plugin needs from the controller's registry: the range a module was given for a port name,
/// known from the moment the controller assigns it, before the module starts.
pub trait PortView: Send + Sync {
    fn assigned(&self, module: &ModuleId, port: &str) -> Option<PortRange>;
}

pub struct AllowlistPlugin {
    guard: Option<Arc<Guard>>,
    external_base: Option<PortRange>,
    view: Arc<dyn PortView>,
}

impl AllowlistPlugin {
    /// Guards through the helper in this process.
    pub fn new(guard: Arc<Guard>, view: Arc<dyn PortView>) -> Self {
        Self {
            guard: Some(guard),
            external_base: None,
            view,
        }
    }

    /// For a server whose allowlist this process does not run: refuses every UDP port.
    pub fn absent(view: Arc<dyn PortView>) -> Self {
        Self {
            guard: None,
            external_base: None,
            view,
        }
    }

    /// For a public server whose allowlist runs in an external process (`reforged-allowlist.service`,
    /// which guards `base`): admits a module whose assigned UDP ranges lie inside `base`, and refuses
    /// any range outside `base`.
    pub fn external(base: PortRange, view: Arc<dyn PortView>) -> Self {
        Self {
            guard: None,
            external_base: Some(base),
            view,
        }
    }

    /// The ranges `registration` asks the firewall to guard, as assigned, each checked against the manifest.
    fn ranges(&self, module: &ModuleManifest, registration: &Registration) -> Result<Vec<PortRange>, ModuleError> {
        let invalid = |message: String| ModuleError::new(ErrorKind::Invalid, message);
        let mut ranges = Vec::with_capacity(registration.udp_ports.len());
        for name in &registration.udp_ports {
            let declared = module
                .port(name)
                .filter(|port| port.transport == Transport::Udp)
                .ok_or_else(|| invalid(format!("{name} is not a UDP port its manifest declares")))?;
            let assigned = self
                .view
                .assigned(&module.id, name)
                .ok_or_else(|| ModuleError::new(ErrorKind::Unavailable, format!("no range was assigned for {name}")))?;
            if assigned.width() != declared.range.width() {
                return Err(invalid(format!(
                    "UDP port {name} was assigned {} ports, and its manifest declares {}",
                    assigned.width(),
                    declared.range.width()
                )));
            }
            ranges.push(assigned);
        }
        Ok(ranges)
    }
}

impl CorePlugin for AllowlistPlugin {
    fn name(&self) -> &'static str {
        "allowlist"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::UdpPorts]
    }

    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        if registration.udp_ports.is_empty() {
            return Ok(());
        }
        let ranges = self.ranges(module, registration)?;
        if let Some(guard) = &self.guard {
            return guard
                .attach(&module.id, &ranges)
                .map_err(|message| ModuleError::new(ErrorKind::Invalid, message));
        }
        if let Some(base) = self.external_base
            && ranges
                .iter()
                .all(|r| r.is_valid() && r.first >= base.first && r.last <= base.last)
        {
            return Ok(());
        }
        Err(ModuleError::new(
            ErrorKind::Unavailable,
            "the allowlist runs outside this process and guards only the server's own game ports; \
             the module's UDP ports would be open to everyone",
        ))
    }

    fn detach(&self, module: &ModuleId) {
        if let Some(guard) = &self.guard {
            guard.detach(module);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use module_api::stub::StubModule;
    use module_api::{GameModule, Registrar};

    use super::*;

    struct Assigned(BTreeMap<String, PortRange>);

    impl PortView for Assigned {
        fn assigned(&self, _module: &ModuleId, port: &str) -> Option<PortRange> {
            self.0.get(port).copied()
        }
    }

    fn stub() -> (ModuleManifest, Registration) {
        let module = StubModule::new();
        let mut registrar = Registrar::new(module.manifest());
        module.register(&mut registrar).unwrap();
        (module.manifest(), registrar.finish().unwrap())
    }

    fn plugin(assigned: Option<(u16, u16)>) -> (AllowlistPlugin, Arc<Guard>) {
        let guard = Arc::new(Guard::new("8770-8777").unwrap());
        let view = Assigned(
            assigned
                .map(|(first, last)| ("game".to_owned(), PortRange { first, last }))
                .into_iter()
                .collect(),
        );
        (AllowlistPlugin::new(guard.clone(), Arc::new(view)), guard)
    }

    fn refusal(plugin: &AllowlistPlugin, manifest: &ModuleManifest, registration: &Registration) -> String {
        plugin.attach(manifest, registration).unwrap_err().to_string()
    }

    #[test]
    fn the_stub_s_declared_range_is_guarded_while_it_is_loaded() {
        let (manifest, registration) = stub();
        let (plugin, guard) = plugin(Some((65010, 65019)));
        assert_eq!(plugin.serves(), [CoreCapability::UdpPorts]);
        plugin.attach(&manifest, &registration).unwrap();
        assert_eq!(guard.text(), "8770-8777,65010-65019");
        plugin.detach(&manifest.id);
        assert_eq!(guard.text(), "8770-8777");
    }

    #[test]
    fn a_range_the_controller_moved_is_the_one_guarded() {
        let (manifest, registration) = stub();
        let (plugin, guard) = plugin(Some((65110, 65119)));
        plugin.attach(&manifest, &registration).unwrap();
        assert_eq!(guard.text(), "8770-8777,65110-65119");
    }

    #[test]
    fn a_range_of_another_width_a_missing_one_and_an_undeclared_port_are_refused() {
        let (manifest, registration) = stub();
        let (wide, guard) = plugin(Some((65010, 65030)));
        assert!(refusal(&wide, &manifest, &registration).contains("assigned 21"));
        let (none, _) = plugin(None);
        assert!(refusal(&none, &manifest, &registration).contains("no range"));
        // A TCP port asked for as UDP, and a name the manifest lacks
        let (plugin, _) = plugin(Some((65010, 65019)));
        for name in ["http", "nothing"] {
            let mut bad = registration.clone();
            bad.udp_ports = vec![name.to_owned()];
            assert!(refusal(&plugin, &manifest, &bad).contains("not a UDP port"), "{name}");
        }
        assert_eq!(guard.text(), "8770-8777");
    }

    #[test]
    fn privileged_and_overlapping_ranges_are_refused() {
        let (manifest, registration) = stub();
        for assigned in [(1010, 1019), (8770, 8779), (8768, 8777)] {
            let (plugin, guard) = plugin(Some(assigned));
            plugin.attach(&manifest, &registration).unwrap_err();
            assert_eq!(guard.text(), "8770-8777");
        }
    }

    #[test]
    fn a_module_without_udp_ports_costs_nothing_and_an_absent_helper_refuses_one_with() {
        let (manifest, mut registration) = stub();
        let absent = AllowlistPlugin::absent(Arc::new(Assigned(
            [(
                "game".to_owned(),
                PortRange {
                    first: 65010,
                    last: 65019,
                },
            )]
            .into_iter()
            .collect(),
        )));
        assert!(refusal(&absent, &manifest, &registration).contains("everyone"));
        registration.udp_ports.clear();
        absent.attach(&manifest, &registration).unwrap();
        absent.detach(&manifest.id);
    }

    #[test]
    fn external_helper_admits_the_servers_own_game_ports_and_refuses_other_ranges() {
        let (manifest, registration) = stub();
        let base = PortRange {
            first: 65010,
            last: 65019,
        };
        let inside = AllowlistPlugin::external(
            base,
            Arc::new(Assigned([("game".to_owned(), base)].into_iter().collect())),
        );
        inside.attach(&manifest, &registration).unwrap();
        inside.detach(&manifest.id);

        let outside = AllowlistPlugin::external(
            PortRange {
                first: 8770,
                last: 8777,
            },
            Arc::new(Assigned([("game".to_owned(), base)].into_iter().collect())),
        );
        assert!(refusal(&outside, &manifest, &registration).contains("everyone"));
    }
}
