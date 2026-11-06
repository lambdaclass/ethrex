//! TEMPORARY: memory held by a trie after overwriting K existing keys. Not for commit.
//! `cargo test --release -p ethrex-trie --test mem_probe_tmp -- --ignored --nocapture`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use ethrex_crypto::NativeCrypto;
use ethrex_trie::{InMemoryTrieDB, Trie};

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, layout, new_size) };
        if !q.is_null() {
            LIVE.fetch_add(new_size as isize - layout.size() as isize, Ordering::Relaxed);
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A uniformly spread 32-byte key, like a hashed address or slot.
fn key(i: u64) -> Vec<u8> {
    let mut k = Vec::with_capacity(32);
    for lane in 0..4 {
        k.extend_from_slice(&splitmix(i.wrapping_mul(4).wrapping_add(lane)).to_be_bytes());
    }
    k
}

#[test]
#[ignore = "manual measurement"]
fn mem_probe() {
    // (keys in the trie, value length): 33 ~ a storage slot, 80 ~ an account.
    for (n, value_len) in [(100_000_u64, 33_usize), (1_000_000, 33), (1_000_000, 80)] {
        let map = ethrex_trie::db::NodeMap::default();
        let mut trie = Trie::new(Box::new(InMemoryTrieDB::new(map.clone())));
        for i in 0..n {
            trie.insert(key(i), vec![1u8; value_len]).unwrap();
        }
        let root = trie.hash(&NativeCrypto).unwrap();
        drop(trie);

        for k in [1_000_u64, 12_000, 60_000] {
            let before = LIVE.load(Ordering::Relaxed);
            let mut t = Trie::open(Box::new(InMemoryTrieDB::new(map.clone())), root);
            let stride = n / k;
            for j in 0..k {
                t.insert(key(j * stride), vec![2u8; value_len]).unwrap();
            }
            let _ = t.hash_no_commit(&NativeCrypto);
            let after = LIVE.load(Ordering::Relaxed);
            let bytes = after - before;
            println!(
                "PROBE n={n} value_len={value_len} overwrites={k} held={:.1} MiB per_key={} B",
                bytes as f64 / (1024.0 * 1024.0),
                bytes / k as isize
            );
            drop(t);
        }
    }
}
