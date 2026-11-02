//! IP address classification for SSRF protection.
//!
//! std's IpAddr::is_global is still unstable, so the deny tables below are
//! explicit and conservative. IPv6 is only accepted inside 2000::/3.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

use ipnet::{Ipv4Net, Ipv6Net};

/// IPv4 ranges that are never acceptable as an outbound destination.
const DENY_V4: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.2.0/24",
    "192.88.99.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/4",
    "240.0.0.0/4",
];

/// IPv6 ranges that are never acceptable. Applied after the 2000::/3 allow test.
const DENY_V6: &[&str] = &[
    "::/128",
    "::1/128",
    "::ffff:0:0/96",
    "64:ff9b::/96",
    "64:ff9b:1::/48",
    "100::/64",
    "2001::/32",
    "2001:2::/48",
    "2001:10::/28",
    "2001:20::/28",
    "2001:db8::/32",
    "2002::/16",
    "3fff::/20",
    "5f00::/16",
    "fc00::/7",
    "fe80::/10",
    "fec0::/10",
    "ff00::/8",
];

const GLOBAL_V6: &str = "2000::/3";

fn deny_v4() -> &'static [Ipv4Net] {
    static CELL: OnceLock<Vec<Ipv4Net>> = OnceLock::new();
    CELL.get_or_init(|| {
        DENY_V4
            .iter()
            .map(|cidr| cidr.parse().expect("static IPv4 CIDR must be valid"))
            .collect()
    })
}

fn deny_v6() -> &'static [Ipv6Net] {
    static CELL: OnceLock<Vec<Ipv6Net>> = OnceLock::new();
    CELL.get_or_init(|| {
        DENY_V6
            .iter()
            .map(|cidr| cidr.parse().expect("static IPv6 CIDR must be valid"))
            .collect()
    })
}

fn global_v6() -> &'static Ipv6Net {
    static CELL: OnceLock<Ipv6Net> = OnceLock::new();
    CELL.get_or_init(|| {
        GLOBAL_V6
            .parse()
            .expect("static global IPv6 CIDR must be valid")
    })
}

/// Address policy. The production constructor is strict; a loopback-only
/// variant exists for in-process integration tests behind the test-util feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpPolicy {
    allow_loopback_for_tests: bool,
}

impl IpPolicy {
    pub const fn strict() -> Self {
        Self {
            allow_loopback_for_tests: false,
        }
    }

    #[cfg(feature = "test-util")]
    pub const fn permissive_for_tests() -> Self {
        Self {
            allow_loopback_for_tests: true,
        }
    }

    pub fn is_permitted(&self, ip: IpAddr) -> bool {
        if self.allow_loopback_for_tests {
            return ip.is_loopback();
        }
        match ip {
            IpAddr::V4(v4) => is_permitted_v4(v4),
            IpAddr::V6(v6) => is_permitted_v6(v6),
        }
    }
}

fn is_permitted_v4(addr: Ipv4Addr) -> bool {
    !deny_v4().iter().any(|net| net.contains(&addr))
}

fn is_permitted_v6(addr: Ipv6Addr) -> bool {
    if !global_v6().contains(&addr) {
        return false;
    }
    !deny_v6().iter().any(|net| net.contains(&addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(value: &str) -> bool {
        IpPolicy::strict().is_permitted(value.parse().unwrap())
    }

    #[test]
    fn permits_public_ipv4() {
        for value in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "151.101.1.69"] {
            assert!(allowed(value), "{value} should be permitted");
        }
    }

    #[test]
    fn denies_special_ipv4() {
        for value in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.0.0.1",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!allowed(value), "{value} should be denied");
        }
    }

    #[test]
    fn permits_public_ipv6() {
        for value in [
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "2a00:1450:4001::1",
        ] {
            assert!(allowed(value), "{value} should be permitted");
        }
    }

    #[test]
    fn denies_special_ipv6() {
        for value in [
            "::",
            "::1",
            "::ffff:8.8.8.8",
            "64:ff9b::8.8.8.8",
            "64:ff9b:1::1",
            "100::1",
            "2001::1",
            "2001:2::1",
            "2001:10::1",
            "2001:20::1",
            "2001:db8::1",
            "2002:808:808::1",
            "3fff::1",
            "fc00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
        ] {
            assert!(!allowed(value), "{value} should be denied");
        }
    }

    #[test]
    fn ipv4_mapped_is_never_permitted() {
        let mapped: Ipv6Addr = "::ffff:1.1.1.1".parse().unwrap();
        assert!(!IpPolicy::strict().is_permitted(IpAddr::V6(mapped)));
    }
}
