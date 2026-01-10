//! Fuzz BN254 (alt_bn128) curve precompiles
//!
//! BN254 precompiles:
//! - ecadd (0x06): Point addition. Input: P1 (64 bytes) || P2 (64 bytes) = 128 bytes
//! - ecmul (0x07): Scalar multiplication. Input: P (64 bytes) || scalar (32 bytes) = 96 bytes
//! - ecpairing (0x08): Pairing check. Input: pairs of (G1 point, G2 point) = N * 192 bytes
//!
//! G1 point format: x (32 bytes) || y (32 bytes)
//! G2 point format: x_imag (32 bytes) || x_real (32 bytes) || y_imag (32 bytes) || y_real (32 bytes)
//!
//! Security-critical: Invalid curve points, points not on curve, point at infinity handling

#![no_main]

use arbitrary::Arbitrary;
use bytes::Bytes;
use ethrex_common::types::Fork;
use ethrex_common::H160;
use ethrex_levm::precompiles::execute_precompile;
use libfuzzer_sys::fuzz_target;

/// BN254 precompile addresses
const ECADD_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06,
]);

const ECMUL_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07,
]);

const ECPAIRING_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08,
]);

/// G1 point (64 bytes)
#[derive(Arbitrary, Debug, Clone)]
struct G1Point {
    x: [u8; 32],
    y: [u8; 32],
}

impl G1Point {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(&self.x);
        bytes.extend_from_slice(&self.y);
        bytes
    }

    /// Point at infinity (all zeros)
    fn infinity() -> Self {
        Self {
            x: [0u8; 32],
            y: [0u8; 32],
        }
    }
}

/// G2 point (128 bytes)
#[derive(Arbitrary, Debug, Clone)]
struct G2Point {
    x_imag: [u8; 32],
    x_real: [u8; 32],
    y_imag: [u8; 32],
    y_real: [u8; 32],
}

impl G2Point {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(&self.x_imag);
        bytes.extend_from_slice(&self.x_real);
        bytes.extend_from_slice(&self.y_imag);
        bytes.extend_from_slice(&self.y_real);
        bytes
    }

    /// Point at infinity
    fn infinity() -> Self {
        Self {
            x_imag: [0u8; 32],
            x_real: [0u8; 32],
            y_imag: [0u8; 32],
            y_real: [0u8; 32],
        }
    }
}

/// Structured input for ecadd
#[derive(Arbitrary, Debug)]
struct EcaddInput {
    p1: G1Point,
    p2: G1Point,
    /// Use point at infinity for p1
    p1_infinity: bool,
    /// Use point at infinity for p2
    p2_infinity: bool,
    /// Extra bytes to append
    extra: Vec<u8>,
    /// Truncate input
    truncate_at: Option<u8>,
}

/// Structured input for ecmul
#[derive(Arbitrary, Debug)]
struct EcmulInput {
    point: G1Point,
    scalar: [u8; 32],
    /// Use point at infinity
    point_infinity: bool,
    /// Use zero scalar
    zero_scalar: bool,
    /// Use max scalar (curve order - 1)
    max_scalar: bool,
    /// Extra bytes
    extra: Vec<u8>,
    /// Truncate
    truncate_at: Option<u8>,
}

/// Structured input for ecpairing
#[derive(Arbitrary, Debug)]
struct EcpairingInput {
    /// Number of pairs (0-4 for reasonable fuzzing)
    num_pairs: u8,
    /// G1 points
    g1_points: Vec<G1Point>,
    /// G2 points
    g2_points: Vec<G2Point>,
    /// Use infinity points
    use_infinity: bool,
    /// Extra bytes
    extra: Vec<u8>,
}

#[derive(Arbitrary, Debug)]
enum Bn254Input {
    Ecadd(EcaddInput),
    Ecmul(EcmulInput),
    Ecpairing(EcpairingInput),
    /// Raw bytes to any BN254 precompile
    RawEcadd(Vec<u8>),
    RawEcmul(Vec<u8>),
    RawEcpairing(Vec<u8>),
}

fuzz_target!(|input: Bn254Input| {
    let mut gas_remaining: u64 = 100_000_000;

    match input {
        Bn254Input::Ecadd(ecadd) => {
            let mut data = Vec::new();

            let p1 = if ecadd.p1_infinity { G1Point::infinity() } else { ecadd.p1 };
            let p2 = if ecadd.p2_infinity { G1Point::infinity() } else { ecadd.p2 };

            data.extend_from_slice(&p1.to_bytes());
            data.extend_from_slice(&p2.to_bytes());
            data.extend_from_slice(&ecadd.extra);

            if let Some(truncate) = ecadd.truncate_at {
                let len = (truncate as usize) % (data.len() + 1);
                data.truncate(len);
            }

            let _ = execute_precompile(ECADD_ADDRESS, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bn254Input::Ecmul(ecmul) => {
            let mut data = Vec::new();

            let point = if ecmul.point_infinity { G1Point::infinity() } else { ecmul.point };
            data.extend_from_slice(&point.to_bytes());

            let scalar = if ecmul.zero_scalar {
                [0u8; 32]
            } else if ecmul.max_scalar {
                // BN254 curve order - 1 (roughly)
                let mut s = [0xffu8; 32];
                s[0] = 0x30; // Make it less than curve order
                s
            } else {
                ecmul.scalar
            };
            data.extend_from_slice(&scalar);
            data.extend_from_slice(&ecmul.extra);

            if let Some(truncate) = ecmul.truncate_at {
                let len = (truncate as usize) % (data.len() + 1);
                data.truncate(len);
            }

            let _ = execute_precompile(ECMUL_ADDRESS, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bn254Input::Ecpairing(pairing) => {
            let mut data = Vec::new();

            let num_pairs = (pairing.num_pairs % 5) as usize; // 0-4 pairs

            for i in 0..num_pairs {
                let g1 = if pairing.use_infinity {
                    G1Point::infinity()
                } else {
                    pairing.g1_points.get(i).cloned().unwrap_or_else(G1Point::infinity)
                };

                let g2 = if pairing.use_infinity {
                    G2Point::infinity()
                } else {
                    pairing.g2_points.get(i).cloned().unwrap_or_else(G2Point::infinity)
                };

                data.extend_from_slice(&g1.to_bytes());
                data.extend_from_slice(&g2.to_bytes());
            }

            data.extend_from_slice(&pairing.extra);

            let _ = execute_precompile(ECPAIRING_ADDRESS, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bn254Input::RawEcadd(data) => {
            let _ = execute_precompile(ECADD_ADDRESS, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bn254Input::RawEcmul(data) => {
            let _ = execute_precompile(ECMUL_ADDRESS, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bn254Input::RawEcpairing(data) => {
            // Limit pairing data to prevent very slow operations
            let data = if data.len() > 192 * 4 { data[..192 * 4].to_vec() } else { data };
            let _ = execute_precompile(ECPAIRING_ADDRESS, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
    }
});
