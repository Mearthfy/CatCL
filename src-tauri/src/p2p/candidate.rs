use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use super::invite::{Candidate, CandidateKind};

pub fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || address.to_ipv4_mapped().is_some()
        || (address.segments()[0] & 0xffc0) == 0xfe80
        || (address.segments()[0] & 0xfe00) == 0xfc00
        || address.segments()[0..2] == [0x2001, 0x0db8]
    {
        return false;
    }
    true
}

pub fn direct_candidates(addresses: impl IntoIterator<Item = IpAddr>, port: u16) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = addresses
        .into_iter()
        .filter_map(|address| match address {
            IpAddr::V6(value) if is_public_ipv6(value) => Some(Candidate {
                address: SocketAddr::new(IpAddr::V6(value), port).to_string(),
                kind: CandidateKind::Ipv6Direct,
            }),
            IpAddr::V4(value) if !value.is_loopback() && !value.is_unspecified() => {
                Some(Candidate {
                    address: SocketAddr::new(IpAddr::V4(value), port).to_string(),
                    kind: CandidateKind::Local,
                })
            }
            _ => None,
        })
        .collect();
    candidates.sort_by_key(|candidate| match candidate.kind {
        CandidateKind::Ipv6Direct => 0,
        CandidateKind::Ipv4Direct => 1,
        CandidateKind::Mapped => 2,
        CandidateKind::Local => 3,
        CandidateKind::Loopback => 4,
    });
    candidates.dedup_by(|left, right| left.address == right.address);
    candidates
}

pub fn preferred_direct(candidates: &[Candidate]) -> Option<SocketAddr> {
    candidates
        .iter()
        .filter_map(|candidate| {
            candidate
                .socket_addr()
                .ok()
                .map(|address| (candidate, address))
        })
        .min_by_key(|(candidate, _)| match candidate.kind {
            CandidateKind::Ipv6Direct => 0,
            CandidateKind::Ipv4Direct => 1,
            CandidateKind::Mapped => 2,
            CandidateKind::Local => 3,
            CandidateKind::Loopback => 4,
        })
        .map(|(_, address)| address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_non_public_ipv6_ranges() {
        assert!(!is_public_ipv6("::1".parse().unwrap()));
        assert!(!is_public_ipv6("fe80::1".parse().unwrap()));
        assert!(!is_public_ipv6("fd00::1".parse().unwrap()));
        assert!(!is_public_ipv6("ff02::1".parse().unwrap()));
        assert!(is_public_ipv6("2408:8000::1".parse().unwrap()));
    }

    #[test]
    fn ipv6_direct_is_preferred() {
        let candidates = vec![
            Candidate {
                address: "192.168.1.2:4000".into(),
                kind: CandidateKind::Local,
            },
            Candidate {
                address: "[2408:8000::1]:4000".into(),
                kind: CandidateKind::Ipv6Direct,
            },
        ];
        assert!(preferred_direct(&candidates).unwrap().is_ipv6());
    }
}
