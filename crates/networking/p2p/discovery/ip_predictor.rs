use ethrex_common::H256;
use rustc_hash::{FxHashMap, FxHashSet};
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

/// Time window for collecting IP votes from PONG recipient_addr.
const IP_VOTE_WINDOW: Duration = Duration::from_secs(300);
/// Minimum number of agreeing votes required to update external IP.
const IP_VOTE_THRESHOLD: usize = 3;

/// Tracks PONG-observed external IPs from multiple peers and returns a
/// winning IP once quorum is reached, shared across discv4 and discv5.
#[derive(Debug, Default)]
pub struct IpPredictor {
    /// Collects reported IPs from PONGs, keyed by IP, value is set of voter node-ids.
    pub ip_votes: FxHashMap<IpAddr, FxHashSet<H256>>,
    /// When the current IP voting period started. None if no votes received yet.
    pub ip_vote_period_start: Option<Instant>,
    /// Whether the first (fast) voting round has completed.
    pub first_ip_vote_round_completed: bool,
}

impl IpPredictor {
    /// Records an IP vote from a PONG-observed address.
    /// Returns `Some(ip)` if the voting round ended with a winning IP to apply.
    pub fn record_ip_vote(&mut self, reported_ip: IpAddr, voter_id: H256) -> Option<IpAddr> {
        // Discard only never-routable addresses (loopback/link-local/unspecified). RFC1918
        // private IPs are kept as candidates: on a flat private network (e.g. a local or
        // kurtosis enclave) the private IP is the address peers actually reach us at, and no
        // public IP is ever observed. `finalize_ip_vote_round` still prefers a public winner
        // when one reaches quorum, so a NAT'd node (whose peers observe its public source IP)
        // converges on the public IP and never advertises a private one.
        if Self::is_unroutable_ip(reported_ip) {
            return None;
        }

        let now = Instant::now();

        if self.ip_vote_period_start.is_none() {
            self.ip_vote_period_start = Some(now);
        }

        self.ip_votes
            .entry(reported_ip)
            .or_default()
            .insert(voter_id);

        let total_votes: usize = self.ip_votes.values().map(|v| v.len()).sum();
        let round_ended = if !self.first_ip_vote_round_completed {
            total_votes >= IP_VOTE_THRESHOLD
        } else {
            self.ip_vote_period_start
                .is_some_and(|start| now.duration_since(start) >= IP_VOTE_WINDOW)
        };

        if round_ended {
            return self.finalize_ip_vote_round();
        }
        None
    }

    /// Checks whether the current voting period has timed out and finalizes it.
    /// Returns `Some(ip)` if a timed-out round produced a winning IP to apply.
    pub fn check_timeout(&mut self) -> Option<IpAddr> {
        let now = Instant::now();
        if let Some(start) = self.ip_vote_period_start
            && now.duration_since(start) >= IP_VOTE_WINDOW
        {
            return self.finalize_ip_vote_round();
        }
        None
    }

    /// Finalizes the current voting round.
    /// Returns `Some(winning_ip)` if a winner reached the threshold and should be applied.
    fn finalize_ip_vote_round(&mut self) -> Option<IpAddr> {
        // Among IPs that reached quorum, prefer a public (routable) one; fall back to a
        // private winner only if no public IP reached quorum. A NAT'd/SNAT'd node's peers
        // observe and vote its routable public source IP, so it converges on public; a node
        // on a flat private network only ever sees private votes, so it converges on the
        // reachable private IP instead of advertising nothing forever.
        let mut best_public: Option<(IpAddr, usize)> = None;
        let mut best_private: Option<(IpAddr, usize)> = None;
        for (ip, voters) in &self.ip_votes {
            let count = voters.len();
            if count < IP_VOTE_THRESHOLD {
                continue;
            }
            let slot = if Self::is_private_ip(*ip) {
                &mut best_private
            } else {
                &mut best_public
            };
            if slot.is_none_or(|(_, best)| count > best) {
                *slot = Some((*ip, count));
            }
        }
        let result = best_public.or(best_private).map(|(ip, _)| ip);

        self.ip_votes.clear();
        self.ip_vote_period_start = Some(Instant::now());
        self.first_ip_vote_round_completed = true;

        result
    }

    /// Returns true for addresses that can never be a valid externally-advertised endpoint
    /// (loopback, link-local, unspecified). Unlike [`is_private_ip`](Self::is_private_ip),
    /// RFC1918 / unique-local private addresses are NOT included: on a flat private network
    /// they are the reachable address, so they remain valid vote candidates (preferred only
    /// when no public IP reaches quorum).
    fn is_unroutable_ip(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => v4.is_loopback() || v4.is_link_local() || v4.is_unspecified(),
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    // link-local (fe80::/10)
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        }
    }

    /// Returns true if the IP is private/local (not useful for external connectivity).
    pub fn is_private_ip(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => {
                v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
            }
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    // unique local (fc00::/7)
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
                    // link-local (fe80::/10)
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        }
    }
}
