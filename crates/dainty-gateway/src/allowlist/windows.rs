// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Windows Firewall through COM (`INetFwPolicy2`), never PowerShell. The only `unsafe` in the crate: COM calls
//! have no safe binding. Each call runs on a blocking thread inside a multithreaded apartment it sets up and
//! tears down. Testable only on an elevated Windows machine (the maintainer's live check).

use windows::Win32::Foundation::{CloseHandle, HANDLE, VARIANT_BOOL, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, INetFwRules, NET_FW_ACTION_ALLOW, NET_FW_IP_PROTOCOL_UDP, NET_FW_PROFILE2_ALL,
    NET_FW_RULE_DIR_IN, NetFwPolicy2, NetFwRule,
};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Ole::IEnumVARIANT;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Variant::{VARIANT, VT_DISPATCH, VariantClear};
use windows::core::{BSTR, Interface};

use super::firewall::WindowsChange;

/// What a rule with the helper's name says, read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleState {
    /// Whether the rule is enabled.
    pub enabled: bool,
    /// Its local ports.
    pub local_ports: String,
    /// Its remote addresses.
    pub remote_addresses: String,
    /// Its protocol number (17 for UDP).
    pub protocol: i32,
    /// Its direction (1 for in).
    pub direction: i32,
    /// Its action (1 for allow).
    pub action: i32,
}

fn com(e: windows::core::Error) -> String {
    format!("COM: {e}")
}

fn fw(e: windows::core::Error) -> String {
    format!("Windows Firewall: {e}")
}

/// A multithreaded COM apartment for this thread, left when dropped.
struct Apartment;

impl Apartment {
    #[allow(unsafe_code, reason = "CoInitializeEx; see the SAFETY note")]
    fn enter() -> Result<Self, String> {
        // SAFETY: no reserved pointer; paired with CoUninitialize in Drop on the same thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(com)?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    #[allow(unsafe_code, reason = "CoUninitialize; see the SAFETY note")]
    fn drop(&mut self) {
        // SAFETY: this thread entered the apartment in `enter`, and every COM object made in it is gone.
        unsafe { CoUninitialize() };
    }
}

#[allow(unsafe_code, reason = "COM: creates the firewall policy object")]
fn rules() -> Result<INetFwRules, String> {
    // SAFETY: a registered in-process class id and its interface; no outer object.
    let policy: INetFwPolicy2 = unsafe { CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER) }.map_err(com)?;
    // SAFETY: a live policy object.
    unsafe { policy.Rules() }.map_err(fw)
}

/// Every rule whose display name is `name`.
#[allow(unsafe_code, reason = "COM: walks the rules enumerator")]
fn named(rules: &INetFwRules, name: &str) -> Result<Vec<INetFwRule>, String> {
    // SAFETY: a live rules collection; its enumerator is an IEnumVARIANT by contract.
    let all: IEnumVARIANT = unsafe { rules._NewEnum() }.map_err(fw)?.cast().map_err(fw)?;
    let mut found = Vec::new();
    loop {
        let mut slot = [VARIANT::default()];
        let mut fetched = 0u32;
        // SAFETY: one slot to fill and a count to write, both live for the call.
        let hr = unsafe { all.Next(&mut slot, &mut fetched) };
        if hr.is_err() || fetched == 0 {
            break;
        }
        // SAFETY: the variant was just filled; its dispatch pointer is read only when its type says it holds one.
        let rule = unsafe {
            let inner = &slot[0].Anonymous.Anonymous;
            if inner.vt == VT_DISPATCH {
                inner
                    .Anonymous
                    .pdispVal
                    .as_ref()
                    .and_then(|d| d.cast::<INetFwRule>().ok())
            } else {
                None
            }
        };
        // SAFETY: releases what Next put in the slot; the cast above holds its own reference.
        _ = unsafe { VariantClear(&mut slot[0]) };
        if let Some(rule) = rule
            // SAFETY: a live rule object.
            && unsafe { rule.Name() }.is_ok_and(|n| n == name)
        {
            found.push(rule);
        }
    }
    Ok(found)
}

#[allow(unsafe_code, reason = "COM: the rule's setters")]
fn set_open(rule: &INetFwRule, open: &super::firewall::WindowsRule) -> Result<(), String> {
    // SAFETY: a live rule object; each setter copies the BSTR it is given. The protocol goes before the ports,
    // which need one.
    unsafe {
        rule.SetDescription(&BSTR::from(open.description.as_str()))
            .map_err(fw)?;
        rule.SetGrouping(&BSTR::from(open.grouping.as_str())).map_err(fw)?;
        rule.SetDirection(NET_FW_RULE_DIR_IN).map_err(fw)?;
        rule.SetAction(NET_FW_ACTION_ALLOW).map_err(fw)?;
        rule.SetProtocol(NET_FW_IP_PROTOCOL_UDP.0).map_err(fw)?;
        rule.SetLocalPorts(&BSTR::from(open.local_ports.as_str())).map_err(fw)?;
        rule.SetRemoteAddresses(&BSTR::from(open.remote_addresses.as_str()))
            .map_err(fw)?;
        rule.SetProfiles(NET_FW_PROFILE2_ALL.0).map_err(fw)?;
        rule.SetEnabled(VARIANT_TRUE).map_err(fw)
    }
}

#[allow(unsafe_code, reason = "COM: the rule's setter")]
fn disable(rule: &INetFwRule) -> Result<(), String> {
    // SAFETY: a live rule object.
    unsafe { rule.SetEnabled(VARIANT_FALSE) }.map_err(fw)
}

/// Brings the rules named `name` to `change`: close disables them all; open sets the first (or makes one) and
/// disables the rest, so exactly one is in effect. A rule an older helper made with the name is taken over.
#[allow(unsafe_code, reason = "COM calls on the firewall rules")]
pub fn apply_rule(change: &WindowsChange, name: &str) -> Result<(), String> {
    let _apartment = Apartment::enter()?;
    let rules = rules()?;
    let found = named(&rules, name)?;
    match change {
        WindowsChange::Close => found.iter().try_for_each(disable),
        WindowsChange::Open(open) => {
            match found.first() {
                Some(first) => set_open(first, open)?,
                None => {
                    // SAFETY: a registered in-process class id and its interface; no outer object.
                    let rule: INetFwRule =
                        unsafe { CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER) }.map_err(com)?;
                    // SAFETY: a live rule object; the name is copied.
                    unsafe { rule.SetName(&BSTR::from(name)) }.map_err(fw)?;
                    set_open(&rule, open)?;
                    // SAFETY: a live collection and a complete rule.
                    unsafe { rules.Add(&rule) }.map_err(fw)?;
                }
            }
            found.iter().skip(1).try_for_each(disable)
        }
    }
}

/// Every rule named `name`, read-only.
#[allow(unsafe_code, reason = "COM calls on the firewall rules")]
pub fn read_rules(name: &str) -> Result<Vec<RuleState>, String> {
    let _apartment = Apartment::enter()?;
    let rules = rules()?;
    named(&rules, name)?
        .iter()
        .map(|rule| {
            // SAFETY: a live rule object; getters return owned values.
            unsafe {
                Ok(RuleState {
                    enabled: rule.Enabled().map_err(fw)? != VARIANT_BOOL(0),
                    local_ports: rule.LocalPorts().map_err(fw)?.to_string(),
                    remote_addresses: rule.RemoteAddresses().map_err(fw)?.to_string(),
                    protocol: rule.Protocol().map_err(fw)?,
                    direction: rule.Direction().map_err(fw)?.0,
                    action: rule.Action().map_err(fw)?.0,
                })
            }
        })
        .collect()
}

/// Removes every rule named `name`, one at a time (only the live test uses it).
#[allow(unsafe_code, reason = "COM calls on the firewall rules")]
pub fn remove_rules(name: &str) -> Result<usize, String> {
    let _apartment = Apartment::enter()?;
    let rules = rules()?;
    let count = named(&rules, name)?.len();
    for _ in 0..count {
        // SAFETY: a live collection; Remove takes the first rule with the name.
        unsafe { rules.Remove(&BSTR::from(name)) }.map_err(fw)?;
    }
    Ok(count)
}

/// Whether this process runs elevated: a read-only query of its token.
#[allow(unsafe_code, reason = "the Win32 token query is an FFI call")]
pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    // SAFETY: the current process's pseudo handle, and a handle slot that outlives the call.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut written = 0u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION of the size given, live for the call.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut written,
        )
    }
    .is_ok();
    // SAFETY: the token handle opened above, closed once.
    _ = unsafe { CloseHandle(token) };
    ok && elevation.TokenIsElevated != 0
}
