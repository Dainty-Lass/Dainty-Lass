// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The controller's part of `/metrics` (`dainty_service_kit::metrics`): each component's phase, restarts and uptime,
//! and the CPU and memory of this process and of every process it started, found by parentage: the game
//! servers, and turn-rs when the kit runs it. Read at each scrape, never in between.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use dainty_service_kit::context::{ComponentPhase, StatusRegistry};
use dainty_service_kit::metrics as m;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const PHASES: [ComponentPhase; 6] = [
    ComponentPhase::Stopped,
    ComponentPhase::Starting,
    ComponentPhase::Ready,
    ComponentPhase::Stopping,
    ComponentPhase::Failed,
    ComponentPhase::Disabled,
];

/// Registers the collector for this controller (one per process: a second controller replaces the first).
pub fn install(status: StatusRegistry) {
    install_as("controller", status);
}

fn install_as(key: &str, status: StatusRegistry) {
    // CPU use is measured between two refreshes, so the system is kept: the first scrape reads 0
    let system = Arc::new(Mutex::new(System::new()));
    m::collector(key, move || {
        components(&status);
        processes(&mut system.lock().unwrap_or_else(PoisonError::into_inner));
    });
}

fn components(status: &StatusRegistry) {
    let snapshot = status.snapshot();
    for name in [
        "dainty_component_phase",
        "dainty_component_restarts",
        "dainty_component_uptime_seconds",
    ] {
        m::clear(name);
    }
    for c in &snapshot.components {
        for phase in PHASES {
            m::gauge(
                "dainty_component_phase",
                "1 for the phase each component is in (stopped, starting, ready, stopping, failed, disabled)",
                &[("component", &c.name), ("phase", phase.as_str())],
                if c.phase == phase { 1.0 } else { 0.0 },
            );
        }
        let labels = [("component", c.name.as_str())];
        m::gauge(
            "dainty_component_restarts",
            "Times a component was restarted",
            &labels,
            f64::from(c.restarts),
        );
        m::gauge(
            "dainty_component_uptime_seconds",
            "Seconds since a component last became ready",
            &labels,
            c.uptime_secs as f64,
        );
    }
    m::gauge(
        "dainty_players_online",
        "Players the server counts as online",
        &[],
        f64::from(snapshot.online_players),
    );
    m::gauge(
        "dainty_hunts_active",
        "Hunts running",
        &[],
        f64::from(snapshot.active_hunts),
    );
    m::gauge(
        "dainty_hunts_max",
        "Hunts allowed at once",
        &[],
        f64::from(snapshot.max_hunts),
    );
}

/// This process and its descendants, by parent pid.
fn family(system: &System, root: Pid) -> BTreeSet<Pid> {
    let mut children: BTreeMap<Pid, Vec<Pid>> = BTreeMap::new();
    for (pid, process) in system.processes() {
        if let Some(parent) = process.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    let mut found = BTreeSet::from([root]);
    let mut next = vec![root];
    while let Some(pid) = next.pop() {
        for child in children.get(&pid).into_iter().flatten() {
            if found.insert(*child) {
                next.push(*child);
            }
        }
    }
    found
}

fn processes(system: &mut System) {
    for name in [
        "dainty_process_cpu_percent",
        "dainty_process_memory_bytes",
        "dainty_process_uptime_seconds",
    ] {
        m::clear(name);
    }
    let Ok(me) = sysinfo::get_current_pid() else {
        return;
    };
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
    for pid in family(system, me) {
        let Some(process) = system.process(pid) else {
            continue;
        };
        let pid_text = pid.as_u32().to_string();
        let name = process.name().to_string_lossy();
        let role = if pid == me { "host" } else { "child" };
        let labels = [("pid", pid_text.as_str()), ("name", name.as_ref()), ("role", role)];
        m::gauge(
            "dainty_process_cpu_percent",
            "CPU use of the host and every process it started, in percent of one core",
            &labels,
            f64::from(process.cpu_usage()),
        );
        m::gauge(
            "dainty_process_memory_bytes",
            "Resident memory of the host and every process it started",
            &labels,
            process.memory() as f64,
        );
        m::gauge(
            "dainty_process_uptime_seconds",
            "Seconds each process has run",
            &labels,
            process.run_time() as f64,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scrape_has_the_components_and_this_process() {
        let status = StatusRegistry::new("private", Vec::new());
        status.register_component("gateway", None);
        // Other tests' controllers replace the "controller" collector at any time. Collectors run in key order,
        // so a key of its own that sorts after it makes this registry the one the scrape reports.
        install_as("controller-metrics-test", status);
        let text = m::render();
        m::remove_collector("controller-metrics-test");
        assert!(
            text.contains("dainty_component_phase{component=\"gateway\",phase=\"stopped\"} 1\n"),
            "{text}"
        );
        assert!(
            text.contains("dainty_component_phase{component=\"gateway\",phase=\"ready\"} 0\n"),
            "{text}"
        );
        let me = std::process::id().to_string();
        let memory = text
            .lines()
            .find(|l| l.starts_with("dainty_process_memory_bytes{") && l.contains(&format!("pid=\"{me}\"")))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(memory.contains("role=\"host\""), "{memory}");
        let bytes: f64 = memory.rsplit(' ').next().unwrap().parse().unwrap();
        assert!(bytes > 1_000_000.0, "{memory}");
    }
}
