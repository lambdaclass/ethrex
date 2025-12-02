use crate::H256;
use ethereum_types::U256;
use ethrex_crypto::keccak::keccak_hash;
use hex::FromHexError;

pub const ZERO_U256: U256 = U256([0, 0, 0, 0]);

/// Converts a big endian slice to a u256, faster than `u256::from_big_endian`.
pub fn u256_from_big_endian(slice: &[u8]) -> U256 {
    let mut padded = [0u8; 32];
    padded[32 - slice.len()..32].copy_from_slice(slice);

    let mut ret = [0; 4];

    let mut u64_bytes = [0u8; 8];
    for i in 0..4 {
        u64_bytes.copy_from_slice(&padded[8 * i..(8 * i + 8)]);
        ret[4 - i - 1] = u64::from_be_bytes(u64_bytes);
    }

    U256(ret)
}

/// Converts a constant big endian slice to a u256, faster than `u256::from_big_endian` and `u256_from_big_endian`.
///
/// Note: N should not exceed 32.
pub fn u256_from_big_endian_const<const N: usize>(slice: [u8; N]) -> U256 {
    const { assert!(N <= 32, "N must be less or equal to 32") };

    let mut padded = [0u8; 32];
    padded[32 - N..32].copy_from_slice(&slice);

    let mut ret = [0u64; 4];

    let mut u64_bytes = [0u8; 8];
    for i in 0..4 {
        u64_bytes.copy_from_slice(&padded[8 * i..(8 * i + 8)]);
        ret[4 - i - 1] = u64::from_be_bytes(u64_bytes);
    }

    U256(ret)
}

/// Converts a U256 to a big endian slice.
#[inline(always)]
pub fn u256_to_big_endian(value: U256) -> [u8; 32] {
    let mut bytes = [0u8; 32];

    for i in 0..4 {
        let u64_be = value.0[4 - i - 1].to_be_bytes();
        bytes[8 * i..(8 * i + 8)].copy_from_slice(&u64_be);
    }

    bytes
}

#[inline(always)]
pub fn u256_to_h256(value: U256) -> H256 {
    H256(u256_to_big_endian(value))
}

pub fn decode_hex(hex: &str) -> Result<Vec<u8>, FromHexError> {
    let trimmed = hex.strip_prefix("0x").unwrap_or(hex);
    hex::decode(trimmed)
}

pub fn keccak(data: impl AsRef<[u8]>) -> H256 {
    H256(keccak_hash(data))
}

// Allocation-free operations on arrays.
///
/// Truncates an array of size N to size M.
/// Fails compilation if N < M.
pub fn truncate_array<const N: usize, const M: usize>(data: [u8; N]) -> [u8; M] {
    const { assert!(M <= N) };
    let mut res = [0u8; M];
    res.copy_from_slice(&data[..M]);
    res
}

// Profiling tools
#[cfg(feature = "profiling")]
pub mod profiling {
    use tracing::warn;
    // TODO: convert to message passing with a dedicated thread
    // so the rest of the program doesn't need to block.
    pub struct ProfilingGuard {
        // Keep the result so we don't pollute the callers with error checking
        guard: pprof::Result<pprof::ProfilerGuard<'static>>,
        name: String,
        thread_prefixes: Vec<String>,
    }
    impl ProfilingGuard {
        pub fn stop(mut self) {
            use pprof::protos::Message;

            let Ok(guard) = self.guard else {
                warn!("Building profiler guard failed, no profile will be created");
                return;
            };
            let prefixes = std::mem::take(&mut self.thread_prefixes);
            let Ok(mut report) = guard.report().build() else {
                warn!("Building profiler report failed, no profile will be created");
                return;
            };
            report
                .data
                .retain(|k, _| prefixes.iter().any(|p| k.thread_name.starts_with(p)));
            let Ok(mut file) = std::fs::File::create(format!("profile-{}.pb", &self.name)) else {
                warn!("Failed to create files, no profile will be created");
                return;
            };
            let Ok(profile) = report.pprof() else {
                warn!("Failed to create pprof report, no profile will be created");
                return;
            };
            _ = profile
                .write_to_writer(&mut file)
                .inspect_err(|e| warn!("Profile writing failed: {e}"));
        }
        pub fn start_profiling(
            freq: i32,
            name: impl FnOnce() -> String,
            thread_prefixes: &[&str],
        ) -> ProfilingGuard {
            let guard = pprof::ProfilerGuardBuilder::default()
                .frequency(freq)
                .blocklist(&["libc", "libgcc", "pthread", "vdso"])
                .build();
            ProfilingGuard {
                guard,
                name: name(),
                thread_prefixes: thread_prefixes.iter().map(|tp| tp.to_string()).collect(),
            }
        }
    }
}

#[cfg(not(feature = "profiling"))]
pub mod profiling {
    pub struct ProfilingGuard();
    impl ProfilingGuard {
        #[inline(always)]
        pub fn stop(self) {}
        #[inline(always)]
        pub fn start_profiling(
            _freq: i32,
            _name: impl FnOnce() -> String,
            _thread_prefixes: &[&str],
        ) -> ProfilingGuard {
            ProfilingGuard()
        }
    }
}

pub use profiling::*;

#[cfg(test)]
mod test {
    use ethereum_types::U256;

    use crate::utils::u256_to_big_endian;

    #[test]
    fn u256_to_big_endian_test() {
        let a = u256_to_big_endian(U256::one());
        let b = U256::one().to_big_endian();
        assert_eq!(a, b);

        let a = u256_to_big_endian(U256::max_value());
        let b = U256::max_value().to_big_endian();
        assert_eq!(a, b);
    }
}
