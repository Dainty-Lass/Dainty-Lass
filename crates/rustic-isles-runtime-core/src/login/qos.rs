//! CL-8, CL-9 (R4 6): the retired latency-test hosts resolve to loopback, where the backend answers the pings.

/// The domain of the official region list's `regionUrl`s (matched case-sensitively, as the AGPL).
pub const LATENCY_DOMAIN: &str = "gce.latency-test.steelyard.ca";
/// Units (or bytes) read from a node name; a name with no NUL among them is passed through.
pub const MAX_NODE_UNITS: usize = 256;
/// The first rewrites logged, both detours together.
pub const LOGGED: u32 = 3;

/// The name before its NUL, when the NUL is among the first 256 units.
pub fn node_name<T: Copy + Into<u16>>(units: &[T]) -> Option<&[T]> {
    let window = &units[..units.len().min(MAX_NODE_UNITS)];
    let end = window.iter().position(|&unit| unit.into() == 0)?;
    Some(&window[..end])
}

/// Whether the node name is a latency-test host.
pub fn rewrites<T: Copy + Into<u16>>(name: &[T]) -> bool {
    let needle: Vec<u16> = LATENCY_DOMAIN.encode_utf16().collect();
    let name: Vec<u16> = name.iter().map(|&unit| unit.into()).collect();
    name.windows(needle.len()).any(|window| window == needle)
}

/// The rewrite line, naming the host when it is ASCII (R4 departure 3).
pub fn rewrite_line<T: Copy + Into<u16>>(name: &[T]) -> String {
    let units: Vec<u16> = name.iter().map(|&unit| unit.into()).collect();
    let host = if units.iter().all(|&unit| (0x20..0x7F).contains(&unit)) {
        String::from_utf16_lossy(&units)
    } else {
        "a latency-test host".to_owned()
    };
    format!("[MitRuntime] [QoS] {host} resolves to 127.0.0.1 (the backend answers the latency test)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_test() {
        let wide: Vec<u16> = "euw11.gce.latency-test.steelyard.ca".encode_utf16().collect();
        assert!(rewrites(&wide));
        assert!(rewrites("euw11.gce.latency-test.steelyard.ca".as_bytes()));
        assert!(!rewrites("GCE.LATENCY-TEST.STEELYARD.CA".as_bytes()));
        assert!(!rewrites("steelyard.ca".as_bytes()));
        assert!(!rewrites(&[] as &[u8]));
        assert_eq!(
            rewrite_line("euw11.gce.latency-test.steelyard.ca".as_bytes()),
            "[MitRuntime] [QoS] euw11.gce.latency-test.steelyard.ca resolves to 127.0.0.1 (the backend answers the \
             latency test)"
        );
        assert!(rewrite_line(&[0x00E9u16]).contains("a latency-test host"));
    }

    #[test]
    fn node_names() {
        assert_eq!(node_name(b"abc\0def"), Some(&b"abc"[..]));
        assert_eq!(node_name(b"abc"), None);
        let mut long = vec![b'a'; MAX_NODE_UNITS];
        assert_eq!(node_name(&long), None, "a longer name is passed through");
        long[MAX_NODE_UNITS - 1] = 0;
        assert_eq!(node_name(&long).map(<[u8]>::len), Some(MAX_NODE_UNITS - 1));
    }
}
