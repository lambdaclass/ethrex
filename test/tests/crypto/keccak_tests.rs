use ethrex_crypto::keccak::{Keccak256, keccak_hash};
use std::array;

const BLOCK_SIZE: usize = 136;

#[test]
fn keccak_empty() {
    assert_eq!(
        keccak_hash(b"")
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>(),
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
    );
}

#[test]
fn keccak_half_block() {
    let buf: [u8; BLOCK_SIZE >> 1] =
        array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);

    assert_eq!(
        keccak_hash(buf)
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>(),
        "337bf14237b641240bd3204e9991c8b96a5349613735ade90a5c2b8806355c11",
    );
}

#[test]
fn keccak_full_block() {
    let buf: [u8; BLOCK_SIZE] = array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);

    assert_eq!(
        keccak_hash(buf)
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>(),
        "3f7424fa94a2f8c5a733b86dac312d85685f9af3dea919694cc6a8abfc075460",
    );
}

#[test]
fn keccak_almost_full_block() {
    let buf: [u8; BLOCK_SIZE - 1] =
        array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);

    assert_eq!(
        keccak_hash(buf)
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>(),
        "3e4916729e2522af4937548f5848a5b49067eec910a0a6a890b0c71dde08854e",
    );
}

#[test]
fn keccak_asm_empty() {
    let keccak = Keccak256::new();
    assert_eq!(
        keccak
            .finalize()
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>(),
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
    );
}

#[test]
fn keccak_asm_half_block() {
    let mut keccak = Keccak256::new();
    let buf: [u8; BLOCK_SIZE >> 1] =
        array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);
    keccak.update(buf);

    assert_eq!(
        keccak
            .finalize()
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>(),
        "337bf14237b641240bd3204e9991c8b96a5349613735ade90a5c2b8806355c11",
    );
}

#[test]
fn keccak_asm_full_block() {
    let mut keccak = Keccak256::new();
    let buf: [u8; BLOCK_SIZE] = array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);
    keccak.update(buf);

    assert_eq!(
        keccak
            .finalize()
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>(),
        "3f7424fa94a2f8c5a733b86dac312d85685f9af3dea919694cc6a8abfc075460",
    );
}

#[test]
fn keccak_asm_almost_full_block() {
    let mut keccak = Keccak256::new();
    let buf: [u8; BLOCK_SIZE - 1] =
        array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);
    keccak.update(buf);

    assert_eq!(
        keccak
            .finalize()
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>(),
        "3e4916729e2522af4937548f5848a5b49067eec910a0a6a890b0c71dde08854e",
    );
}

#[test]
fn keccak_asm_two_half_updates() {
    let mut keccak = Keccak256::new();

    let full: [u8; BLOCK_SIZE] = array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);

    let half = BLOCK_SIZE / 2;

    keccak.update(&full[..half]);
    keccak.update(&full[half..]);

    let buf = keccak
        .finalize()
        .into_iter()
        .map(|x| format!("{x:02x}"))
        .collect::<String>();

    assert_eq!(
        buf,
        "3f7424fa94a2f8c5a733b86dac312d85685f9af3dea919694cc6a8abfc075460"
    );
}

#[test]
fn keccak_compare_one_shot_vs_two_updates() {
    let full: Vec<u8> = (0..BLOCK_SIZE)
        .map(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8)
        .collect();

    let mut k1 = Keccak256::new();
    let mut k2 = Keccak256::new();

    k1.update(&full);

    k2.update(&full[..BLOCK_SIZE / 2]);
    k2.update(&full[BLOCK_SIZE / 2..]);

    let h1 = k1.finalize();

    let h2 = k2.finalize();

    assert_eq!(h1, h2);
}

#[test]
fn keccac_compare_small_than_block() {
    let mut one = Keccak256::new();
    let mut two = Keccak256::new();

    let a = vec![1u8; 30];
    let b = vec![1u8; 40];

    one.update(&a);
    one.update(&b);

    two.update([1u8; 70]);

    assert_eq!(one.finalize(), two.finalize());
}
