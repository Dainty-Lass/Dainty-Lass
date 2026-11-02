// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The game's UDP ports as the firewall rule takes them: a port or a range, and lists of those.

use crate::json_string;

/// The most ranges one rule holds.
pub const MAX_RANGES: usize = 32;

fn port(text: &str) -> Option<u32> {
    ((1..=5).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// `<port>` or `<low>-<high>`, normalised (no leading zeros, a single port kept single).
pub fn check_ports(text: &str) -> Result<String, String> {
    let shape = || {
        format!(
            "ALLOWLIST_PORTS must be a port or a range like 8770-8777 (got {})",
            json_string(text)
        )
    };
    let (low, high) = match text.split_once('-') {
        Some((low, high)) => (port(low).ok_or_else(shape)?, Some(port(high).ok_or_else(shape)?)),
        None => (port(text).ok_or_else(shape)?, None),
    };
    let in_range = |p: u32| (1..=65535).contains(&p);
    if !in_range(low) || high.is_some_and(|h| !in_range(h) || h < low) {
        return Err("ALLOWLIST_PORTS must be within 1-65535, low to high".to_owned());
    }
    Ok(match high {
        Some(high) => format!("{low}-{high}"),
        None => low.to_string(),
    })
}

/// A comma-separated list of [`check_ports`] items, at most [`MAX_RANGES`].
pub fn check_port_list(text: &str) -> Result<String, String> {
    let items: Vec<&str> = text.split(',').collect();
    if items.len() > MAX_RANGES {
        return Err(format!("ALLOWLIST_PORTS holds at most {MAX_RANGES} ranges"));
    }
    Ok(items
        .into_iter()
        .map(check_ports)
        .collect::<Result<Vec<_>, _>>()?
        .join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_items() {
        assert_eq!(check_ports("08770-08777").unwrap(), "8770-8777");
        assert_eq!(check_ports("8777").unwrap(), "8777");
        assert_eq!(check_ports("1-65535").unwrap(), "1-65535");
        for bad in [
            "8770,8771",
            "any",
            "",
            "-",
            "8770-",
            "-8777",
            "123456",
            " 8777",
            "8770-8777 accept",
            "+8777",
        ] {
            assert_eq!(
                check_ports(bad).unwrap_err(),
                format!(
                    "ALLOWLIST_PORTS must be a port or a range like 8770-8777 (got {})",
                    json_string(bad)
                ),
                "{bad}"
            );
        }
        for bad in ["8777-8770", "0-5", "0", "8770-70000"] {
            assert_eq!(
                check_ports(bad).unwrap_err(),
                "ALLOWLIST_PORTS must be within 1-65535, low to high",
                "{bad}"
            );
        }
    }

    #[test]
    fn lists() {
        assert_eq!(
            check_port_list("08770-8777,64000,64010-64019").unwrap(),
            "8770-8777,64000,64010-64019"
        );
        for bad in [",", "8770,", "8770,,8771", "8770, 8771"] {
            assert!(check_port_list(bad).is_err(), "{bad}");
        }
        let many = vec!["1"; 33].join(",");
        assert_eq!(
            check_port_list(&many).unwrap_err(),
            "ALLOWLIST_PORTS holds at most 32 ranges"
        );
        assert!(check_port_list(&vec!["1"; 32].join(",")).is_ok());
    }
}
