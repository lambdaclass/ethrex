//! Fuzz BLS12-381 curve precompiles (Prague fork)
//!
//! BLS12-381 precompiles (EIP-2537):
//! - bls12_g1add (0x0b): G1 point addition. Input: P1 (128 bytes) || P2 (128 bytes) = 256 bytes
//! - bls12_g1msm (0x0c): G1 multi-scalar multiplication. Input: pairs of (point, scalar) = N * 160 bytes
//! - bls12_g2add (0x0d): G2 point addition. Input: P1 (256 bytes) || P2 (256 bytes) = 512 bytes
//! - bls12_g2msm (0x0e): G2 multi-scalar multiplication. Input: pairs of (point, scalar) = N * 288 bytes
//! - bls12_pairing_check (0x0f): Pairing check. Input: pairs of (G1, G2) = N * 384 bytes
//! - bls12_map_fp_to_g1 (0x10): Map field element to G1. Input: 64 bytes
//! - bls12_map_fp2_to_g2 (0x11): Map Fp2 element to G2. Input: 128 bytes
//!
//! G1 point: 128 bytes (padded 48-byte coordinates)
//! G2 point: 256 bytes (padded Fp2 coordinates)
//! Scalar: 32 bytes

#![no_main]

use arbitrary::Arbitrary;
use bytes::Bytes;
use ethrex_common::types::Fork;
use ethrex_common::H160;
use ethrex_levm::precompiles::execute_precompile;
use libfuzzer_sys::fuzz_target;

// BLS12-381 precompile addresses
const BLS12_G1ADD: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0b]);
const BLS12_G1MSM: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0c]);
const BLS12_G2ADD: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0d]);
const BLS12_G2MSM: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0e]);
const BLS12_PAIRING: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0f]);
const BLS12_MAP_FP_TO_G1: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10]);
const BLS12_MAP_FP2_TO_G2: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11]);

/// BLS12-381 G1 point (128 bytes - padded format)
#[derive(Arbitrary, Debug, Clone)]
struct Bls12G1Point {
    /// x coordinate (64 bytes: 16 padding + 48 actual)
    x: [u8; 64],
    /// y coordinate (64 bytes: 16 padding + 48 actual)
    y: [u8; 64],
}

impl Bls12G1Point {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(&self.x);
        bytes.extend_from_slice(&self.y);
        bytes
    }

    fn infinity() -> Self {
        Self {
            x: [0u8; 64],
            y: [0u8; 64],
        }
    }
}

/// BLS12-381 G2 point (256 bytes - padded format)
#[derive(Arbitrary, Debug, Clone)]
struct Bls12G2Point {
    /// x coordinate (Fp2 = 2 * 64 bytes)
    x_c0: [u8; 64],
    x_c1: [u8; 64],
    /// y coordinate (Fp2 = 2 * 64 bytes)
    y_c0: [u8; 64],
    y_c1: [u8; 64],
}

impl Bls12G2Point {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(&self.x_c0);
        bytes.extend_from_slice(&self.x_c1);
        bytes.extend_from_slice(&self.y_c0);
        bytes.extend_from_slice(&self.y_c1);
        bytes
    }

    fn infinity() -> Self {
        Self {
            x_c0: [0u8; 64],
            x_c1: [0u8; 64],
            y_c0: [0u8; 64],
            y_c1: [0u8; 64],
        }
    }
}

/// Scalar for BLS12-381 (32 bytes)
#[derive(Arbitrary, Debug, Clone)]
struct Bls12Scalar {
    value: [u8; 32],
}

#[derive(Arbitrary, Debug)]
struct G1AddInput {
    p1: Bls12G1Point,
    p2: Bls12G1Point,
    use_infinity_p1: bool,
    use_infinity_p2: bool,
    extra: Vec<u8>,
    truncate_at: Option<u16>,
}

#[derive(Arbitrary, Debug)]
struct G1MsmInput {
    /// Number of pairs (1-4)
    num_pairs: u8,
    points: Vec<Bls12G1Point>,
    scalars: Vec<Bls12Scalar>,
    use_zero_scalars: bool,
    extra: Vec<u8>,
}

#[derive(Arbitrary, Debug)]
struct G2AddInput {
    p1: Bls12G2Point,
    p2: Bls12G2Point,
    use_infinity_p1: bool,
    use_infinity_p2: bool,
    extra: Vec<u8>,
    truncate_at: Option<u16>,
}

#[derive(Arbitrary, Debug)]
struct G2MsmInput {
    /// Number of pairs (1-3)
    num_pairs: u8,
    points: Vec<Bls12G2Point>,
    scalars: Vec<Bls12Scalar>,
    use_zero_scalars: bool,
    extra: Vec<u8>,
}

#[derive(Arbitrary, Debug)]
struct PairingInput {
    /// Number of pairs (0-3)
    num_pairs: u8,
    g1_points: Vec<Bls12G1Point>,
    g2_points: Vec<Bls12G2Point>,
    use_infinity: bool,
    extra: Vec<u8>,
}

#[derive(Arbitrary, Debug)]
struct MapFpToG1Input {
    /// Field element (64 bytes)
    fp: [u8; 64],
    extra: Vec<u8>,
    truncate_at: Option<u8>,
}

#[derive(Arbitrary, Debug)]
struct MapFp2ToG2Input {
    /// Fp2 element (128 bytes)
    fp2_c0: [u8; 64],
    fp2_c1: [u8; 64],
    extra: Vec<u8>,
    truncate_at: Option<u8>,
}

#[derive(Arbitrary, Debug)]
enum Bls12Input {
    G1Add(G1AddInput),
    G1Msm(G1MsmInput),
    G2Add(G2AddInput),
    G2Msm(G2MsmInput),
    Pairing(PairingInput),
    MapFpToG1(MapFpToG1Input),
    MapFp2ToG2(MapFp2ToG2Input),
    // Raw inputs for each precompile
    RawG1Add(Vec<u8>),
    RawG1Msm(Vec<u8>),
    RawG2Add(Vec<u8>),
    RawG2Msm(Vec<u8>),
    RawPairing(Vec<u8>),
    RawMapFpToG1(Vec<u8>),
    RawMapFp2ToG2(Vec<u8>),
}

fuzz_target!(|input: Bls12Input| {
    let mut gas_remaining: u64 = 100_000_000;

    match input {
        Bls12Input::G1Add(add) => {
            let mut data = Vec::new();
            let p1 = if add.use_infinity_p1 { Bls12G1Point::infinity() } else { add.p1 };
            let p2 = if add.use_infinity_p2 { Bls12G1Point::infinity() } else { add.p2 };
            data.extend_from_slice(&p1.to_bytes());
            data.extend_from_slice(&p2.to_bytes());
            data.extend_from_slice(&add.extra);

            if let Some(truncate) = add.truncate_at {
                let len = (truncate as usize) % (data.len() + 1);
                data.truncate(len);
            }

            let _ = execute_precompile(BLS12_G1ADD, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::G1Msm(msm) => {
            let mut data = Vec::new();
            let num_pairs = ((msm.num_pairs % 4) + 1) as usize; // 1-4 pairs

            for i in 0..num_pairs {
                let point = msm.points.get(i).cloned().unwrap_or_else(Bls12G1Point::infinity);
                data.extend_from_slice(&point.to_bytes());

                let scalar = if msm.use_zero_scalars {
                    [0u8; 32]
                } else {
                    msm.scalars.get(i).map(|s| s.value).unwrap_or([0u8; 32])
                };
                data.extend_from_slice(&scalar);
            }
            data.extend_from_slice(&msm.extra);

            let _ = execute_precompile(BLS12_G1MSM, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::G2Add(add) => {
            let mut data = Vec::new();
            let p1 = if add.use_infinity_p1 { Bls12G2Point::infinity() } else { add.p1 };
            let p2 = if add.use_infinity_p2 { Bls12G2Point::infinity() } else { add.p2 };
            data.extend_from_slice(&p1.to_bytes());
            data.extend_from_slice(&p2.to_bytes());
            data.extend_from_slice(&add.extra);

            if let Some(truncate) = add.truncate_at {
                let len = (truncate as usize) % (data.len() + 1);
                data.truncate(len);
            }

            let _ = execute_precompile(BLS12_G2ADD, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::G2Msm(msm) => {
            let mut data = Vec::new();
            let num_pairs = ((msm.num_pairs % 3) + 1) as usize; // 1-3 pairs (G2 is larger)

            for i in 0..num_pairs {
                let point = msm.points.get(i).cloned().unwrap_or_else(Bls12G2Point::infinity);
                data.extend_from_slice(&point.to_bytes());

                let scalar = if msm.use_zero_scalars {
                    [0u8; 32]
                } else {
                    msm.scalars.get(i).map(|s| s.value).unwrap_or([0u8; 32])
                };
                data.extend_from_slice(&scalar);
            }
            data.extend_from_slice(&msm.extra);

            let _ = execute_precompile(BLS12_G2MSM, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::Pairing(pairing) => {
            let mut data = Vec::new();
            let num_pairs = (pairing.num_pairs % 4) as usize; // 0-3 pairs

            for i in 0..num_pairs {
                let g1 = if pairing.use_infinity {
                    Bls12G1Point::infinity()
                } else {
                    pairing.g1_points.get(i).cloned().unwrap_or_else(Bls12G1Point::infinity)
                };

                let g2 = if pairing.use_infinity {
                    Bls12G2Point::infinity()
                } else {
                    pairing.g2_points.get(i).cloned().unwrap_or_else(Bls12G2Point::infinity)
                };

                data.extend_from_slice(&g1.to_bytes());
                data.extend_from_slice(&g2.to_bytes());
            }
            data.extend_from_slice(&pairing.extra);

            let _ = execute_precompile(BLS12_PAIRING, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::MapFpToG1(map) => {
            let mut data = Vec::new();
            data.extend_from_slice(&map.fp);
            data.extend_from_slice(&map.extra);

            if let Some(truncate) = map.truncate_at {
                let len = (truncate as usize) % (data.len() + 1);
                data.truncate(len);
            }

            let _ = execute_precompile(BLS12_MAP_FP_TO_G1, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::MapFp2ToG2(map) => {
            let mut data = Vec::new();
            data.extend_from_slice(&map.fp2_c0);
            data.extend_from_slice(&map.fp2_c1);
            data.extend_from_slice(&map.extra);

            if let Some(truncate) = map.truncate_at {
                let len = (truncate as usize) % (data.len() + 1);
                data.truncate(len);
            }

            let _ = execute_precompile(BLS12_MAP_FP2_TO_G2, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        // Raw inputs - limit sizes to prevent slowness
        Bls12Input::RawG1Add(data) => {
            let data = if data.len() > 512 { data[..512].to_vec() } else { data };
            let _ = execute_precompile(BLS12_G1ADD, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::RawG1Msm(data) => {
            let data = if data.len() > 160 * 4 { data[..160 * 4].to_vec() } else { data };
            let _ = execute_precompile(BLS12_G1MSM, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::RawG2Add(data) => {
            let data = if data.len() > 1024 { data[..1024].to_vec() } else { data };
            let _ = execute_precompile(BLS12_G2ADD, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::RawG2Msm(data) => {
            let data = if data.len() > 288 * 3 { data[..288 * 3].to_vec() } else { data };
            let _ = execute_precompile(BLS12_G2MSM, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::RawPairing(data) => {
            let data = if data.len() > 384 * 3 { data[..384 * 3].to_vec() } else { data };
            let _ = execute_precompile(BLS12_PAIRING, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::RawMapFpToG1(data) => {
            let data = if data.len() > 128 { data[..128].to_vec() } else { data };
            let _ = execute_precompile(BLS12_MAP_FP_TO_G1, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
        Bls12Input::RawMapFp2ToG2(data) => {
            let data = if data.len() > 256 { data[..256].to_vec() } else { data };
            let _ = execute_precompile(BLS12_MAP_FP2_TO_G2, &Bytes::from(data), &mut gas_remaining, Fork::Prague);
        }
    }
});
