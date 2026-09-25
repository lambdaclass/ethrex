//! `--p2p.netrestrict`: confine every peer interaction to a set of IP networks.
//!
//! A node on a private devnet has no business talking to the public internet,
//! and an operator behind a carrier-grade NAT can get rate-limited by their
//! provider if it does. With a restriction in place, discovered nodes outside
//! the allowed networks are never stored, pinged or dialed, and inbound TCP and
//! UDP from outside is dropped before any handshake. Mirrors geth's
//! `--netrestrict`.

use std::{fmt, net::IpAddr, sync::Arc};

pub use ipnet::IpNet;

/// IP networks this node may talk to. Empty means unrestricted.
///
/// Consulted on every inbound packet and every discovered node, from several
/// actors, so the list is shared rather than copied on clone.
#[derive(Clone, Debug, Default)]
pub struct NetRestrict(Arc<[IpNet]>);

impl NetRestrict {
    pub fn new(nets: Vec<IpNet>) -> Self {
        Self(nets.into())
    }

    /// Whether `ip` may be contacted, or accepted as a source. Always true when
    /// no restriction is configured.
    pub fn allows(&self, ip: IpAddr) -> bool {
        self.0.is_empty() || self.0.iter().any(|net| net.contains(&ip))
    }

    pub fn is_unrestricted(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<IpNet>> for NetRestrict {
    fn from(nets: Vec<IpNet>) -> Self {
        Self::new(nets)
    }
}

impl fmt::Display for NetRestrict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("unrestricted");
        }
        for (i, net) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{net}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn restrict(nets: &[&str]) -> NetRestrict {
        NetRestrict::new(nets.iter().map(|n| n.parse().unwrap()).collect())
    }

    #[test]
    fn unrestricted_allows_everything() {
        let unrestricted = NetRestrict::default();
        assert!(unrestricted.is_unrestricted());
        assert!(unrestricted.allows(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))));
        assert!(unrestricted.allows(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert_eq!(unrestricted.to_string(), "unrestricted");
    }

    #[test]
    fn only_addresses_inside_a_listed_network_are_allowed() {
        let devnet = restrict(&["10.0.0.0/8", "172.16.0.0/12"]);
        assert!(!devnet.is_unrestricted());
        assert!(devnet.allows(IpAddr::V4(Ipv4Addr::new(10, 200, 1, 1))));
        assert!(devnet.allows(IpAddr::V4(Ipv4Addr::new(172, 31, 255, 254))));
        assert!(!devnet.allows(IpAddr::V4(Ipv4Addr::new(172, 32, 0, 1))));
        assert!(!devnet.allows(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        // A v4-only list says nothing about v6, so v6 sources are rejected too.
        assert!(!devnet.allows(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert_eq!(devnet.to_string(), "10.0.0.0/8,172.16.0.0/12");
    }

    #[test]
    fn v6_networks_are_matched_as_well() {
        let local = restrict(&["fd00::/8", "127.0.0.0/8"]);
        assert!(local.allows("fd12::1".parse().unwrap()));
        assert!(!local.allows("2001:db8::1".parse().unwrap()));
        assert!(local.allows(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }
}
