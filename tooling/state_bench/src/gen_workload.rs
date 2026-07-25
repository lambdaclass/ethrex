//! `gen-workload`: produce a workload of real blocks + captured BALs on top of a
//! `gen-state` fixture.
//!
//! Every block is built through ethrex's real payload path (`create_payload` +
//! `Blockchain::build_payload`) so all header fields (state/receipts roots, gas,
//! base fee, and the EIP-7928 `block_access_list_hash`) are computed by the same
//! code that a live builder runs. The Block Access List captured for each block
//! is the exact one whose `compute_hash` was committed to the header, i.e. the
//! canonical BAL that the parallel import path validates against.
//!
//! Generation runs against a THROWAWAY CHECKPOINT of the `gen-state` datadir so
//! the pristine benchmark fixture (the one that gets passed to `run`) is never
//! mutated. The checkpoint hardlinks the immutable SSTs (near-instant, no extra
//! space even for a huge fixture) and is deleted when generation finishes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use secp256k1::{Message, Secp256k1, SecretKey};
use serde::Serialize;
use tracing::{info, warn};

use ethrex_blockchain::Blockchain;
use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::{
    Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
    Genesis, Transaction, TxKind,
};
use ethrex_common::{Address, H160, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::encode::{PayloadRLPEncode, RLPEncode};
use ethrex_storage::{EngineType, Store, StoreConfig};

use crate::gen_state::{derive_address, derive_slot_key, digest, mega_slot_count};
use crate::manifest::{MANIFEST_FILENAME, Manifest};

/// Advisory floor below which a workload is only a smoke test. The store commits
/// trie/flat-KV layers to disk on a rolling basis once the in-memory chain is
/// `commit_threshold` (128) deep, so a workload well above 128 blocks flushes
/// (and thus measures) the bulk of its cold writes; the most recent ~128 blocks
/// stay in memory as an unmeasured tail, so larger workloads give more
/// representative numbers.
const MIN_BLOCKS_FOR_WRITES: u64 = 150;

/// EIP-1559 transaction type byte, prepended to the RLP payload before hashing
/// for the signature (matches `Signable::sign_inplace` for EIP1559).
const EIP1559_TX_TYPE: u8 = 0x02;

/// Per-tx gas limit. Comfortably covers the accessor loop over a single 64-byte
/// record (base + calldata + one cold SLOAD or SSTORE + loop overhead) while
/// staying well under the fixture genesis gas limit.
const TX_GAS_LIMIT: u64 = 300_000;
/// Fee cap, set high enough to clear any fixture genesis base fee.
const TX_MAX_FEE_PER_GAS: u64 = 1_000_000_000_000; // 1000 gwei
const TX_MAX_PRIORITY_FEE_PER_GAS: u64 = 1_000_000_000; // 1 gwei

/// Deterministic derivation tags for workload targeting (distinct from the tags
/// `gen-state` used to seed state, so target picks never alias seeded slots).
const TAG_MEGA_PICK: &str = "csb-workload-mega";
const TAG_ACCT_PICK: &str = "csb-workload-acct";
const TAG_SLOT_PICK: &str = "csb-workload-slot";
const TAG_TARGET_PICK: &str = "csb-workload-target";
/// Fresh (never-seeded) slot keys used for cold WRITES. Derived with a tag that
/// `gen-state` never used, so every write creates a brand-new storage entry
/// (true cold-write path: new trie node + new flat-KV row) rather than
/// overwriting a pre-seeded value.
const TAG_WRITE_SLOT: &str = "csb-workload-write";

/// Parameters parsed from the `gen-workload` CLI.
pub struct GenWorkloadArgs {
    pub datadir: PathBuf,
    pub out_chain: PathBuf,
    pub out_bals: PathBuf,
    pub num_blocks: u64,
    pub reads_per_block: u64,
    pub writes_per_block: u64,
    pub mega_fraction: f64,
    /// If > 0, mega-account READS are drawn from a hot working set of this many
    /// seeded slots (the first `hot_slots`) instead of uniformly across all
    /// seeded slots. A small hot set re-accessed across blocks gives the read
    /// temporal locality needed to exercise read-through / value caches; the
    /// default (0) keeps the uniform, near-zero-reaccess cold pattern.
    pub hot_slots: u64,
    /// Zipf exponent for mega-account reads. When > 0, reads follow a `r^-s`
    /// power law over the read range (mainnet-like skew: hot slots + long cold
    /// tail) instead of picking uniformly. 0 (default) = uniform. Composes with
    /// `hot_slots` (which bounds the range the skew applies over).
    pub zipf_s: f64,
    pub genesis: PathBuf,
    pub verify_reimport: bool,
    pub jobs: usize,
}

/// One (target account, slot, mode) touch, recorded to the sidecar so Phase 5's
/// coldness self-check can reference exactly what the workload touches.
#[derive(Serialize)]
struct TouchedRecord {
    block_number: u64,
    target_account: String,
    slot: String,
    /// "SLOAD" (read) or "SSTORE" (write).
    mode: &'static str,
    /// True when the slot was seeded by `gen-state` (reads); false for the fresh
    /// slots used by writes.
    seeded: bool,
}

/// Whether a single touch reads a seeded slot or writes a fresh one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Read,
    Write,
}

/// A resolved touch: which account, which slot, and read vs write.
struct Touch {
    target: Address,
    slot: H256,
    mode: Mode,
    seeded: bool,
}

pub async fn run(args: GenWorkloadArgs) -> Result<()> {
    let GenWorkloadArgs {
        datadir,
        out_chain,
        out_bals,
        num_blocks,
        reads_per_block,
        writes_per_block,
        mega_fraction,
        hot_slots,
        zipf_s,
        genesis,
        verify_reimport,
        jobs,
    } = args;

    if !(0.0..=1.0).contains(&mega_fraction) {
        bail!("--mega-fraction must be in [0.0, 1.0], got {mega_fraction}");
    }
    if num_blocks == 0 {
        bail!("--num-blocks must be > 0");
    }

    // --- Load the fixture manifest ------------------------------------------
    let manifest_path = datadir.join(MANIFEST_FILENAME);
    let manifest_bytes = std::fs::read(&manifest_path).with_context(|| {
        format!(
            "reading gen-state manifest {} (is --datadir a gen-state output?)",
            manifest_path.display()
        )
    })?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("parsing gen-state manifest")?;

    // --- Chain config: the store does NOT reload chain config on reopen, so we
    //     re-apply it from the base genesis (same file used by gen-state). This
    //     is what activates Amsterdam and thus BAL recording. ------------------
    let genesis_bytes = std::fs::read(&genesis)
        .with_context(|| format!("reading base genesis {}", genesis.display()))?;
    let genesis: Genesis =
        serde_json::from_slice(&genesis_bytes).context("parsing base genesis JSON")?;

    // --- Small-workload advisory (do NOT hard-fail: small B is for smoke tests).
    // The store commits trie/flat-KV layers to disk on a rolling basis once the
    // in-memory chain is `commit_threshold` (128) deep, so a workload well above
    // 128 blocks flushes (and measures) the bulk of its cold writes during the
    // import loop. Below that, few/no rolling commits fire, so cold writes are
    // barely exercised. A larger workload also grows the on-disk working set and
    // amortizes fixed per-run warmup.
    if num_blocks < MIN_BLOCKS_FOR_WRITES {
        warn!(
            num_blocks,
            "small workload ({num_blocks} blocks): below ~128 blocks few rolling commits fire so \
             prefer >= 1000 blocks for representative cold-state numbers; this length is only \
             suitable for smoke tests."
        );
    }

    // --- Deterministic identities re-derived from the manifest --------------
    let seed = manifest.seed;
    let num_small_accounts = manifest.num_small_accounts;
    let slots_per_account = manifest.slots_per_account;

    // Guard against degenerate fixtures that would make small-account touches
    // silently meaningless: with `--mega-fraction < 1.0` some touches target
    // small accounts, but if the fixture seeded no small accounts (or no slots
    // per account) those touches hit unseeded addresses/slots — the SLOAD reads
    // a zero value / empty-code account, a no-op that would still be recorded as
    // a valid cold read. Fail loudly instead.
    if mega_fraction < 1.0 {
        if num_small_accounts == 0 {
            bail!(
                "fixture has num_small_accounts=0 but --mega-fraction {mega_fraction} < 1.0 routes \
                 touches to small accounts; use --mega-fraction 1.0 or a fixture with small accounts"
            );
        }
        if slots_per_account == 0 {
            bail!(
                "fixture has slots_per_account=0 but --mega-fraction {mega_fraction} < 1.0 routes \
                 touches to small-account slots; use --mega-fraction 1.0 or a fixture with slots"
            );
        }
    }
    let mega_slots = mega_slot_count(manifest.mega_target_bytes);
    let mega_address = derive_address(seed, "csb-mega-acct", 0);
    assert_addr_matches(
        &mega_address,
        &manifest.mega_account_address,
        "mega account",
    );

    let signer_sk = parse_secret_key(&manifest.funded_signer_private_key)
        .context("parsing funded_signer_private_key from manifest")?;

    // Hot-set reads need seeded slots to re-access; clamp to the seeded range.
    let hot_slots = if hot_slots > 0 {
        hot_slots.min(mega_slots)
    } else {
        0
    };
    if zipf_s < 0.0 {
        bail!("--zipf-s must be >= 0 (0 = uniform), got {zipf_s}");
    }

    info!(
        datadir = %datadir.display(),
        num_blocks,
        reads_per_block,
        writes_per_block,
        mega_fraction,
        hot_slots,
        zipf_s,
        num_small_accounts,
        slots_per_account,
        mega_slots,
        "gen-workload: starting"
    );

    // --- Throwaway checkpoint of the pristine datadir -----------------------
    // All generation writes land here; the pristine --datadir is never touched.
    // A RocksDB checkpoint hardlinks the immutable SSTs, so this is near-instant
    // and space-free even for a 50GB+ fixture (a recursive copy would be neither).
    let throwaway = throwaway_path("gen");
    crate::run::make_checkpoint(&datadir, &throwaway)?;

    let generation = generate_on_copy(
        &throwaway,
        &genesis,
        &signer_sk,
        num_blocks,
        reads_per_block,
        writes_per_block,
        mega_fraction,
        seed,
        num_small_accounts,
        slots_per_account,
        mega_slots,
        hot_slots,
        zipf_s,
        mega_address,
    )
    .await;

    // Always remove the throwaway, even on failure.
    let _ = std::fs::remove_dir_all(&throwaway);

    let Generation {
        blocks,
        bals,
        touched,
    } = generation?;

    // --- Serialize artifacts -------------------------------------------------
    write_chain_file(&out_chain, &blocks)?;
    write_bals_file(&out_bals, &bals)?;
    write_touched_sidecar(&out_chain, &touched)?;

    // Mirror the cli.rs:1050 assertion: one BAL per Amsterdam+ block.
    let amsterdam_blocks = blocks
        .iter()
        .filter(|b| b.header.block_access_list_hash.is_some())
        .count();
    if bals.len() != amsterdam_blocks {
        bail!(
            "captured {} BALs but {} Amsterdam+ blocks; the BAL file would mismatch on import",
            bals.len(),
            amsterdam_blocks
        );
    }

    info!(
        out_chain = %out_chain.display(),
        out_bals = %out_bals.display(),
        blocks = blocks.len(),
        bals = bals.len(),
        touched = touched.len(),
        "gen-workload: wrote chain + BAL artifacts (pristine --datadir untouched; pass it to `run`)"
    );

    if verify_reimport {
        verify_reimport_artifacts(&datadir, &genesis, &out_chain, &out_bals, jobs).await?;
    }

    Ok(())
}

/// Blocks + BALs + touched records produced by a single generation pass.
struct Generation {
    blocks: Vec<Block>,
    bals: Vec<BlockAccessList>,
    touched: Vec<TouchedRecord>,
}

#[allow(clippy::too_many_arguments)]
async fn generate_on_copy(
    throwaway: &Path,
    genesis: &Genesis,
    signer_sk: &SecretKey,
    num_blocks: u64,
    reads_per_block: u64,
    writes_per_block: u64,
    mega_fraction: f64,
    seed: u64,
    num_small_accounts: u64,
    slots_per_account: u64,
    mega_slots: u64,
    hot_slots: u64,
    zipf_s: f64,
    mega_address: Address,
) -> Result<Generation> {
    let mut store = Store::new_with_config(throwaway, EngineType::RocksDB, StoreConfig::default())
        .context("opening throwaway datadir copy")?;
    // Reopen does not reload chain config; re-apply it so Amsterdam (and thus BAL
    // recording) is active for the workload blocks.
    store
        .set_chain_config(&genesis.config)
        .await
        .context("applying chain config to throwaway store")?;
    store
        .load_initial_state()
        .await
        .context("anchoring throwaway store head to the durable genesis")?;

    let chain_config = store.get_chain_config();
    let chain_id = chain_config.chain_id;

    // Detect whether the fixture's fork records BALs at the workload timestamps.
    // Amsterdam+ (l1-bal.json) exercises the BAL parallel import path; pre-Amsterdam
    // (l1.json) is the realistic mainnet path (no BAL anywhere, streaming merkleizer).
    let genesis_header = store
        .get_block_header(0)
        .context("reading genesis header")?
        .context("genesis header (block 0) missing from datadir")?;
    let first_block_ts = genesis_header.timestamp + 12;
    let with_bal = chain_config.is_amsterdam_activated(first_block_ts);
    if with_bal {
        info!("gen-workload: Amsterdam active; capturing one BAL per block (BAL parallel path)");
    } else {
        info!("gen-workload: pre-Amsterdam fixture; blocks carry no BAL (realistic mainnet path)");
    }

    let blockchain = Blockchain::default_with_store(store.clone());

    let mut blocks: Vec<Block> = Vec::with_capacity(num_blocks as usize);
    let mut bals: Vec<BlockAccessList> = Vec::with_capacity(num_blocks as usize);
    let mut touched: Vec<TouchedRecord> = Vec::new();

    let mut parent_header: BlockHeader = genesis_header;
    let mut nonce: u64 = 0; // signer EOA starts at nonce 0 (gen-state funds it fresh)
    let mut touch_index: u64 = 0; // global counter for deterministic target picks
    let mut write_counter: u64 = 0; // global counter for fresh write-slot derivation

    for block_number in 1..=num_blocks {
        // Resolve this block's touches and craft one tx per touch.
        let mut block_touches: Vec<Touch> = Vec::new();
        for _ in 0..reads_per_block {
            block_touches.push(resolve_touch(
                Mode::Read,
                touch_index,
                &mut write_counter,
                mega_fraction,
                seed,
                num_small_accounts,
                slots_per_account,
                mega_slots,
                hot_slots,
                zipf_s,
                mega_address,
            ));
            touch_index += 1;
        }
        for _ in 0..writes_per_block {
            block_touches.push(resolve_touch(
                Mode::Write,
                touch_index,
                &mut write_counter,
                mega_fraction,
                seed,
                num_small_accounts,
                slots_per_account,
                mega_slots,
                hot_slots,
                zipf_s,
                mega_address,
            ));
            touch_index += 1;
        }

        for touch in &block_touches {
            let tx = build_signed_tx(chain_id, nonce, touch, signer_sk)?;
            blockchain
                .add_transaction_to_pool(tx)
                .await
                .with_context(|| format!("adding workload tx (nonce {nonce}) to the mempool"))?;
            nonce += 1;
        }

        let expected_txs = block_touches.len();

        // Build the block through the real payload path.
        let args = BuildPayloadArgs {
            parent: parent_header.hash(),
            timestamp: parent_header.timestamp + 12,
            fee_recipient: H160::zero(),
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            slot_number: None,
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };
        let empty_payload = create_payload(&args, &store, Bytes::new())
            .with_context(|| format!("create_payload for block {block_number}"))?;
        let result = blockchain
            .build_payload(empty_payload)
            .with_context(|| format!("build_payload for block {block_number}"))?;

        let block = result.payload;
        let included = block.body.transactions.len();
        if included != expected_txs {
            bail!(
                "block {block_number}: builder included {included} txs but {expected_txs} were \
                 submitted; workload targeting would be inconsistent (check gas limits / base fee)"
            );
        }

        // Capture the canonical BAL on Amsterdam+: `result.block_access_list` is
        // exactly the BAL whose `compute_hash` was written into
        // `header.block_access_list_hash` by `finalize_payload`. Assert that binding
        // so we know the artifact we emit is the one the parallel import path
        // validates. Pre-Amsterdam blocks produce no BAL, so there is nothing to
        // capture and the import path passes `None`.
        if with_bal {
            let bal = result.block_access_list.ok_or_else(|| {
                anyhow::anyhow!(
                    "block {block_number} produced no BAL despite Amsterdam being active; \
                     payload build did not record a block access list"
                )
            })?;
            let commitment = block.header.block_access_list_hash;
            if commitment != Some(bal.compute_hash(&NativeCrypto)) {
                bail!(
                    "block {block_number}: captured BAL hash does not match header \
                     block_access_list_hash; captured BAL is not the canonical one"
                );
            }
            bals.push(bal);
        }

        // Persist + advance the head-state so the next block parents on this one.
        // `add_block` re-executes and validates the state root against the header
        // that the builder produced, so a bad build fails loudly here.
        blockchain
            .add_block(block.clone())
            .with_context(|| format!("persisting built block {block_number} via add_block"))?;
        blockchain
            .remove_block_transactions_from_pool(&block)
            .with_context(|| format!("removing block {block_number} txs from the pool"))?;

        for touch in &block_touches {
            touched.push(TouchedRecord {
                block_number,
                target_account: format!("{:#x}", touch.target),
                slot: format!("{:#x}", touch.slot),
                mode: match touch.mode {
                    Mode::Read => "SLOAD",
                    Mode::Write => "SSTORE",
                },
                seeded: touch.seeded,
            });
        }

        parent_header = block.header.clone();
        blocks.push(block);

        if block_number % 100 == 0 || block_number == num_blocks {
            info!(
                built = block_number,
                total = num_blocks,
                "gen-workload progress"
            );
        }
    }

    // Generation is inherently sequential: block N's state is the parent of
    // block N+1, so `--jobs` does not influence it.
    drop(blockchain);
    drop(store);
    Ok(Generation {
        blocks,
        bals,
        touched,
    })
}

/// Resolve one touch to a concrete (account, slot, mode). Reads target a seeded
/// slot that actually exists in the pre-built tries; writes target a fresh
/// (never-seeded) slot on the same class of account.
#[allow(clippy::too_many_arguments)]
fn resolve_touch(
    mode: Mode,
    touch_index: u64,
    write_counter: &mut u64,
    mega_fraction: f64,
    seed: u64,
    num_small_accounts: u64,
    slots_per_account: u64,
    mega_slots: u64,
    hot_slots: u64,
    zipf_s: f64,
    mega_address: Address,
) -> Touch {
    // Deterministic mega-vs-small choice. Fall back to small if the mega account
    // has no seeded slots (only relevant for reads on a zero-size mega account).
    let want_mega = fraction_pick(seed, TAG_TARGET_PICK, touch_index) < mega_fraction;
    let use_mega = want_mega && !(mode == Mode::Read && mega_slots == 0);

    if use_mega {
        let (slot, seeded) = match mode {
            Mode::Read => {
                // Read universe: a bounded hot window (`hot_slots`) or all seeded
                // slots. Within it, Zipf-skew the pick when `zipf_s > 0`
                // (mainnet-like hot/cold) else pick uniformly (cold, ~no reaccess).
                let read_range = if hot_slots > 0 { hot_slots } else { mega_slots };
                let k = if zipf_s > 0.0 {
                    zipf_pick(seed, TAG_MEGA_PICK, touch_index, read_range.max(1), zipf_s)
                } else {
                    index_pick(seed, TAG_MEGA_PICK, touch_index, read_range.max(1))
                };
                (derive_slot_key(seed, "csb-mega-slot", k), true)
            }
            Mode::Write => {
                let slot = derive_slot_key(seed, TAG_WRITE_SLOT, *write_counter);
                *write_counter += 1;
                (slot, false)
            }
        };
        Touch {
            target: mega_address,
            slot,
            mode,
            seeded,
        }
    } else {
        let acct_idx = index_pick(seed, TAG_ACCT_PICK, touch_index, num_small_accounts.max(1));
        let target = derive_address(seed, "csb-small-acct", acct_idx);
        let (slot, seeded) = match mode {
            Mode::Read => {
                let slot_idx =
                    index_pick(seed, TAG_SLOT_PICK, touch_index, slots_per_account.max(1));
                let global_index = acct_idx * slots_per_account + slot_idx;
                (derive_slot_key(seed, "csb-small-slot", global_index), true)
            }
            Mode::Write => {
                let slot = derive_slot_key(seed, TAG_WRITE_SLOT, *write_counter);
                *write_counter += 1;
                (slot, false)
            }
        };
        Touch {
            target,
            slot,
            mode,
            seeded,
        }
    }
}

/// Deterministic pseudo-random fraction in [0, 1) from the seed derivation.
fn fraction_pick(seed: u64, tag: &str, index: u64) -> f64 {
    let d = digest(seed, tag, index);
    let hi = u64::from_be_bytes(d[0..8].try_into().expect("8-byte slice"));
    hi as f64 / (u64::MAX as f64 + 1.0)
}

/// Deterministic index in [0, modulus) from the seed derivation.
fn index_pick(seed: u64, tag: &str, index: u64, modulus: u64) -> u64 {
    let d = digest(seed, tag, index);
    let v = u64::from_be_bytes(d[0..8].try_into().expect("8-byte slice"));
    v % modulus
}

/// Zipf-like slot pick over `[0, n)`: rank `r` in `[1, n]` is drawn with density
/// proportional to `r^-s` (a few low-index slots dominate, long cold tail),
/// using the closed-form inverse-CDF of the continuous power law. `s ~ 1.0`
/// mimics mainnet-ish access skew; `s -> 0` approaches uniform. Low indices are
/// the hot ones, but since slot keys are keccak-hashed they scatter across the
/// trie (like mainnet's hot contracts). Deterministic in `(seed, tag, index)`.
fn zipf_pick(seed: u64, tag: &str, index: u64, n: u64, s: f64) -> u64 {
    if n <= 1 {
        return 0;
    }
    let u = fraction_pick(seed, tag, index); // uniform [0, 1)
    let nf = n as f64;
    let rank = if (s - 1.0).abs() < 1e-9 {
        nf.powf(u)
    } else {
        (1.0 + u * (nf.powf(1.0 - s) - 1.0)).powf(1.0 / (1.0 - s))
    };
    (rank as u64).saturating_sub(1).min(n - 1)
}

/// Build the accessor calldata for a single (mode, slot) record: 64 bytes,
/// `[mode: 32B BE][slot: 32B BE]`, mode 0 = SLOAD, 1 = SSTORE.
fn accessor_calldata(touch: &Touch) -> Bytes {
    let mut buf = Vec::with_capacity(64);
    let mut mode_word = [0u8; 32];
    if touch.mode == Mode::Write {
        mode_word[31] = 1;
    }
    buf.extend_from_slice(&mode_word);
    buf.extend_from_slice(touch.slot.as_bytes());
    Bytes::from(buf)
}

/// Construct and sign an EIP-1559 tx whose `to` is the target account and whose
/// calldata drives the accessor to touch `touch.slot`.
fn build_signed_tx(
    chain_id: u64,
    nonce: u64,
    touch: &Touch,
    signer_sk: &SecretKey,
) -> Result<Transaction> {
    let mut tx = EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: TX_MAX_PRIORITY_FEE_PER_GAS,
        max_fee_per_gas: TX_MAX_FEE_PER_GAS,
        gas_limit: TX_GAS_LIMIT,
        to: TxKind::Call(touch.target),
        value: U256::zero(),
        data: accessor_calldata(touch),
        ..Default::default()
    };

    // Signing hash = keccak(0x02 || rlp(payload)), matching Signable for EIP1559.
    let mut payload = Vec::with_capacity(1 + 128);
    payload.push(EIP1559_TX_TYPE);
    payload.extend_from_slice(&tx.encode_payload_to_vec());
    let hash = keccak_hash(&payload);

    let secp = Secp256k1::signing_only();
    let msg = Message::from_digest(hash);
    let (recovery_id, sig) = secp
        .sign_ecdsa_recoverable(&msg, signer_sk)
        .serialize_compact();

    tx.signature_r = U256::from_big_endian(&sig[..32]);
    tx.signature_s = U256::from_big_endian(&sig[32..64]);
    // For non-EIP155 typed txs recovery_id is 0 or 1 and maps directly to yParity.
    tx.signature_y_parity = i32::from(recovery_id) != 0;

    Ok(Transaction::EIP1559Transaction(tx))
}

/// Chain file framing matches `cmd/ethrex/decode.rs::chain_file` (read by
/// `utils::read_chain_file`): a bare concatenation of RLP-encoded `Block`s.
fn write_chain_file(path: &Path, blocks: &[Block]) -> Result<()> {
    let mut buf = Vec::new();
    for block in blocks {
        block.encode(&mut buf);
    }
    std::fs::write(path, &buf).with_context(|| format!("writing chain file {}", path.display()))?;
    Ok(())
}

/// BAL file framing matches the `--with-bal` decoder loop at cli.rs:1030-1045: a
/// bare concatenation of RLP-encoded `BlockAccessList`s.
fn write_bals_file(path: &Path, bals: &[BlockAccessList]) -> Result<()> {
    let mut buf = Vec::new();
    for bal in bals {
        bal.encode(&mut buf);
    }
    std::fs::write(path, &buf).with_context(|| format!("writing BAL file {}", path.display()))?;
    Ok(())
}

/// Write the `<out-chain>.touched.json` sidecar of every (account, slot, mode)
/// touched, for Phase 5's coldness self-check.
fn write_touched_sidecar(out_chain: &Path, touched: &[TouchedRecord]) -> Result<()> {
    let sidecar = sidecar_path(out_chain);
    let json = serde_json::to_string_pretty(touched).context("serializing touched sidecar")?;
    std::fs::write(&sidecar, json)
        .with_context(|| format!("writing touched sidecar {}", sidecar.display()))?;
    info!(sidecar = %sidecar.display(), records = touched.len(), "wrote touched-slot sidecar");
    Ok(())
}

fn sidecar_path(out_chain: &Path) -> PathBuf {
    let mut name = out_chain
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("chain.rlp"));
    name.push(".touched.json");
    out_chain.with_file_name(name)
}

/// Re-import self-check: import the emitted chain.rlp + bals.rlp onto a FRESH
/// COPY of the pristine gen-state datadir via the parallel BAL path, which
/// validates every block's state root AND its `block_access_list_hash` against
/// the provided BAL. Proves the artifacts are valid and re-importable.
async fn verify_reimport_artifacts(
    datadir: &Path,
    genesis: &Genesis,
    out_chain: &Path,
    out_bals: &Path,
    _jobs: usize,
) -> Result<()> {
    info!("gen-workload: verifying re-import on a fresh checkpoint of the pristine datadir");
    let fresh = throwaway_path("verify");
    crate::run::make_checkpoint(datadir, &fresh)?;

    let result = reimport_on_copy(&fresh, genesis, out_chain, out_bals).await;
    let _ = std::fs::remove_dir_all(&fresh);
    result
}

async fn reimport_on_copy(
    fresh: &Path,
    genesis: &Genesis,
    out_chain: &Path,
    out_bals: &Path,
) -> Result<()> {
    use std::sync::Arc;

    let mut store = Store::new_with_config(fresh, EngineType::RocksDB, StoreConfig::default())
        .context("opening fresh datadir copy for re-import")?;
    store
        .set_chain_config(&genesis.config)
        .await
        .context("applying chain config to re-import store")?;
    store
        .load_initial_state()
        .await
        .context("anchoring re-import store head to durable genesis")?;
    let blockchain = Blockchain::default_with_store(store.clone());

    let blocks = read_chain_file(out_chain)?;
    let bals = read_bals_file(out_bals)?;

    let amsterdam_blocks = blocks
        .iter()
        .filter(|b| b.header.block_access_list_hash.is_some())
        .count();
    if bals.len() != amsterdam_blocks {
        bail!(
            "re-import: BAL file has {} entries but chain has {} Amsterdam+ blocks",
            bals.len(),
            amsterdam_blocks
        );
    }

    let mut canonical: Vec<(u64, H256)> = Vec::with_capacity(blocks.len());
    let mut bal_index = 0usize;
    for block in &blocks {
        let number = block.header.number;
        let bal = if block.header.block_access_list_hash.is_some() {
            let b = bals.get(bal_index).cloned().map(Arc::new);
            bal_index += 1;
            b
        } else {
            None
        };
        blockchain
            .add_block_pipeline(block.clone(), bal)
            .with_context(|| format!("re-import: add_block_pipeline for block {number}"))?;
        canonical.push((number, block.hash()));
    }

    let (head_number, head_hash) = canonical
        .last()
        .copied()
        .context("re-import: no blocks imported")?;
    // Canonicalize exactly like cli.rs import does (single FCU at the end).
    store
        .forkchoice_update(
            canonical,
            head_number,
            head_hash,
            Some(head_number),
            Some(head_number),
        )
        .await
        .context("re-import: forkchoice_update")?;
    store
        .wait_for_persistence_idle()
        .await
        .context("re-import: wait_for_persistence_idle")?;

    let latest = store
        .get_latest_block_number()
        .await
        .context("re-import: reading latest block number")?;
    if latest != head_number {
        bail!("re-import: head advanced to {latest}, expected {head_number}");
    }

    info!(
        blocks = blocks.len(),
        bals = bals.len(),
        head = head_number,
        "gen-workload: re-import self-check PASSED (all blocks + BALs validated)"
    );
    drop(blockchain);
    drop(store);
    Ok(())
}

/// Read a chain file, mirroring `cmd/ethrex/decode.rs::chain_file`.
fn read_chain_file(path: &Path) -> Result<Vec<Block>> {
    use ethrex_rlp::decode::RLPDecode;
    let data =
        std::fs::read(path).with_context(|| format!("reading chain file {}", path.display()))?;
    let mut rest = data.as_slice();
    let mut blocks = Vec::new();
    while !rest.is_empty() {
        let (block, tail) =
            Block::decode_unfinished(rest).context("decoding block from chain file")?;
        blocks.push(block);
        rest = tail;
    }
    Ok(blocks)
}

/// Read a BAL file, mirroring the `--with-bal` decoder loop at cli.rs:1030-1045.
fn read_bals_file(path: &Path) -> Result<Vec<BlockAccessList>> {
    use ethrex_rlp::decode::RLPDecode;
    let data =
        std::fs::read(path).with_context(|| format!("reading BAL file {}", path.display()))?;
    let mut rest = data.as_slice();
    let mut bals = Vec::new();
    while !rest.is_empty() {
        let (bal, tail) =
            BlockAccessList::decode_unfinished(rest).context("decoding BAL from BAL file")?;
        bals.push(bal);
        rest = tail;
    }
    Ok(bals)
}

/// Unique throwaway path under the system temp dir (respects `TMPDIR`).
fn throwaway_path(kind: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("state-bench-{kind}-{}-{nanos}", std::process::id()))
}

fn parse_secret_key(hex_key: &str) -> Result<SecretKey> {
    let stripped = hex_key.strip_prefix("0x").unwrap_or(hex_key);
    let bytes = decode_hex(stripped).context("decoding hex private key")?;
    SecretKey::from_slice(&bytes).context("private key is not a valid secp256k1 scalar")
}

fn decode_hex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        bail!("hex string has odd length");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("invalid hex byte"))
        .collect()
}

fn assert_addr_matches(derived: &Address, manifest_hex: &str, what: &str) {
    let expected = format!("{derived:#x}");
    if !manifest_hex.eq_ignore_ascii_case(&expected) {
        warn!(
            what,
            derived = %expected,
            manifest = manifest_hex,
            "re-derived address differs from the manifest; seed or derivation may have changed"
        );
    }
}
