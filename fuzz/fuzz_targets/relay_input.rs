//! What the launcher's local relay reads from whoever connects to it on loopback (RD27 spec §2.6.4, §2.6.5, RX3):
//! the headers its local-request rule judges, the target the QoS answer matches, the port override, and the
//! `netstat` table the owner check reads. See docs/security/launcher-relay.md.
#![no_main]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4};

use http::header::{HOST, HeaderMap, HeaderName, HeaderValue, ORIGIN};
use dainty_launcher_kit::net::relay::local::{self, Refusal};
use dainty_launcher_kit::net::relay::port_override;
use dainty_launcher_kit::net::relay_owner::table::{Owner, owner};
use libfuzzer_sys::fuzz_target;

const CLIENT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 52000);
const SERVER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 61100);
const REMOTE: IpAddr = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));

/// Line one is the Host, line two the Origin, then `name:value` lines.
fn headers(text: &str) -> HeaderMap {
    let mut map = HeaderMap::new();
    let mut lines = text.split('\n');
    if let Some(host) = lines.next().and_then(|v| HeaderValue::from_str(v).ok()) {
        map.append(HOST, host);
    }
    if let Some(origin) = lines.next().and_then(|v| HeaderValue::from_str(v).ok()) {
        map.append(ORIGIN, origin);
    }
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value))
        {
            map.append(name, value);
        }
    }
    map
}

fn client_rows(table: &str) -> usize {
    table
        .lines()
        .filter(|row| {
            let fields: Vec<&str> = row.split_whitespace().collect();
            fields.len() == 5 && fields[..3] == ["TCP", "127.0.0.1:52000", "127.0.0.1:61100"]
        })
        .count()
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let map = headers(&text);
    for upgrade in [false, true] {
        // A pass needs exactly one loopback Host, and no Origin or one loopback Origin
        if local::check(IpAddr::V4(Ipv4Addr::LOCALHOST), &map, upgrade).is_ok() {
            let mut hosts = map.get_all(HOST).iter();
            assert!(
                hosts
                    .next()
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(local::host_is_local)
            );
            assert!(hosts.next().is_none());
            let mut origins = map.get_all(ORIGIN).iter();
            assert!(
                origins
                    .next()
                    .is_none_or(|v| v.to_str().is_ok_and(local::origin_is_local))
            );
            assert!(origins.next().is_none());
        }
        // Nobody off loopback passes, whatever the headers say
        assert_eq!(local::check(REMOTE, &map, upgrade), Err(Refusal::NotLocal));
    }
    for line in text.lines() {
        if let Some(host) = local::authority_host(line) {
            let _ = local::is_loopback_name(host);
        }
        let _ = local::origin_is_local(line);
        if local::is_qos_target(line) {
            assert!(line.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("/qos")));
        }
        if let Some(port) = port_override(line) {
            assert!(port >= 1024);
        }
    }
    // A pid only from exactly one client row
    if let Owner::Pid(_) = owner(&text, CLIENT, SERVER) {
        assert_eq!(client_rows(&text), 1);
    }
});
