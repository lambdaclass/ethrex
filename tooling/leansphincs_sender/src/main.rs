//! Sends EIP-8504 dependency transactions signed with leanSPHINCS keys.
//!
//! `alloc` prints the genesis accounts the tool sends from: each one is
//! controlled by a leanSPHINCS key through its validation code. `send` signs
//! `[VERIFY, PROOF, SENDER]` transactions from those accounts and submits each
//! one together with its signature through `eth_sendRawTransaction`.

use std::time::{Duration, Instant};

use bytes::Bytes;
use clap::{Parser, Subcommand};
use ethrex_common::{
    Address, H256, U256,
    types::{
        Frame, FrameMode, FrameTransaction, LEANSPHINCS_SCHEME, LEANSPHINCS_VERIFICATION_GAS,
        PROOF_FRAME_MODE, ProofWrapper,
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use eyre::{Context, OptionExt, eyre};
use serde_json::{Value, json};

/// Gas for the account's validation frame. Its code reads one triple and
/// approves, a few hundred gas.
const VERIFY_FRAME_GAS: u64 = 50_000;
/// Gas for the call the transaction makes once it is approved.
const SENDER_FRAME_GAS: u64 = 50_000;

#[derive(Parser)]
#[command(about = "Sends EIP-8504 dependency transactions signed with leanSPHINCS keys")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Prints the genesis `alloc` entries of the accounts, as a JSON object.
    Alloc {
        #[command(flatten)]
        accounts: AccountsArgs,
        /// Balance of each account, in ether.
        #[arg(long, default_value_t = 1_000)]
        balance_eth: u64,
    },
    /// Sends transactions from the accounts at a fixed rate.
    Send {
        #[command(flatten)]
        accounts: AccountsArgs,
        /// URL of the node's JSON-RPC endpoint.
        #[arg(long, default_value = "http://localhost:8545")]
        rpc_url: String,
        /// Transactions per second. An account sends its next transaction only
        /// once the previous one is included, so the rate is also bounded by the
        /// number of accounts per block.
        #[arg(long, default_value_t = 1.0)]
        rate: f64,
        /// Number of submissions to make, counting the ones the node rejects and
        /// the transactions sent again. Runs until interrupted when omitted.
        #[arg(long)]
        count: Option<u64>,
        /// Address the transactions call.
        #[arg(long, default_value = "0x000000000000000000000000000000000000dEaD")]
        recipient: Address,
        /// Fee per gas the transactions are willing to pay, in gwei.
        #[arg(long, default_value_t = 10)]
        max_fee_per_gas_gwei: u64,
        /// Priority fee per gas, in gwei.
        #[arg(long, default_value_t = 1)]
        max_priority_fee_per_gas_gwei: u64,
        /// Seconds after which a transaction that was not included is sent again.
        #[arg(long, default_value_t = 120)]
        resend_after: u64,
    },
}

#[derive(clap::Args)]
struct AccountsArgs {
    /// Number of accounts.
    #[arg(long, default_value_t = 16)]
    accounts: usize,
    /// Seed the accounts' keys are derived from. `alloc` and `send` must agree.
    #[arg(long, default_value = "pq-el-devnet-0")]
    seed: String,
}

/// An account controlled by a leanSPHINCS key.
struct Account {
    secret: [u8; 32],
    public: H256,
    address: Address,
    /// The nonce of the transaction that was sent and not yet seen included,
    /// with the time it was sent.
    pending: Option<(u64, Instant)>,
}

impl AccountsArgs {
    /// Derives the accounts: the key of account `i` from the seed and `i`, and
    /// its address from its public key.
    fn derive(&self) -> Vec<Account> {
        (0..self.accounts as u64)
            .map(|index| {
                let secret = keccak([self.seed.as_bytes(), &index.to_be_bytes()].concat()).0;
                let public = H256(lean_sphincs::key_gen(&secret).to_bytes());
                Account {
                    secret,
                    public,
                    address: Address::from_slice(&keccak(public.as_bytes()).as_bytes()[12..]),
                    pending: None,
                }
            })
            .collect()
    }
}

/// Validation code of an account controlled by the leanSPHINCS key `public_key`.
///
/// The frame's data is two words: the index of a proof frame and the byte offset
/// of a triple in it. The code approves execution and payment when that frame is
/// a proof frame and the triple is `(LEANSPHINCS_SCHEME, 0, public_key)`, that
/// is, when the transaction depends on a signature of its own signature hash
/// under the account's key. It reverts otherwise.
///
/// The same code is exercised by ethrex's EIP-8504 tests
/// (`test/tests/levm/eip8504_tests.rs`).
fn account_code(public_key: H256) -> Vec<u8> {
    let mut code = vec![
        // FRAMEPARAM(mode, index) == PROOF_FRAME_MODE
        0x60,
        0x02, // PUSH1 0x02 (param: mode)
        0x5f,
        0x35, // PUSH0, CALLDATALOAD (frame index)
        0xb3, // FRAMEPARAM
        0x60,
        PROOF_FRAME_MODE, // PUSH1 4
        0x14,             // EQ
        // FRAMEDATACOPY(memOffset 0, dataOffset, length 96, frame index)
        0x5f,
        0x35, // PUSH0, CALLDATALOAD (frame index)
        0x60,
        0x60, // PUSH1 96 (length)
        0x60,
        0x20,
        0x35, // PUSH1 32, CALLDATALOAD (offset of the triple)
        0x5f, // PUSH0 (memOffset)
        0xb2, // FRAMEDATACOPY
        // scheme word == LEANSPHINCS_SCHEME
        0x5f,
        0x51, // PUSH0, MLOAD
        0x60,
        LEANSPHINCS_SCHEME, // PUSH1 0x10
        0x14,
        0x16, // EQ, AND
        // data_hash == 0: the message is this transaction's signature hash
        0x60,
        0x20,
        0x51, // PUSH1 32, MLOAD
        0x15,
        0x16, // ISZERO, AND
        // verification_key_hash == public_key
        0x60,
        0x40,
        0x51, // PUSH1 64, MLOAD
        0x7f, // PUSH32 public_key
    ];
    code.extend_from_slice(public_key.as_bytes());
    code.extend_from_slice(&[0x14, 0x16]); // EQ, AND
    // JUMPI to the approval, 6 bytes ahead: PUSH1 dest, JUMPI, PUSH0, PUSH0, REVERT.
    let approve_at = (code.len() + 6) as u8;
    code.extend_from_slice(&[0x60, approve_at, 0x57, 0x5f, 0x5f, 0xfd]);
    // JUMPDEST; APPROVE(offset 0, length 0, scope 3)
    code.extend_from_slice(&[0x5b, 0x60, 0x03, 0x5f, 0x5f, 0xaa]);
    code
}

/// `[VERIFY, PROOF, SENDER]`: the account's validation frame pointed at the only
/// triple of the proof frame, the proof frame declaring a signature of the
/// transaction's signature hash under the account's key, and a call to
/// `recipient`.
fn transaction(
    chain_id: u64,
    account: &Account,
    nonce: u64,
    recipient: Address,
    max_fee_per_gas: U256,
    max_priority_fee_per_gas: U256,
) -> FrameTransaction {
    // The validation frame's data: the proof frame is frame 1, the triple is at
    // offset 0.
    let mut verify_data = [0u8; 64];
    verify_data[31] = 1;
    // The triple: the scheme in a 32-byte word, a zero message, the key.
    let mut triple = [0u8; 96];
    triple[31] = LEANSPHINCS_SCHEME;
    triple[64..].copy_from_slice(account.public.as_bytes());

    let mut tx = FrameTransaction {
        chain_id,
        nonce_keys: vec![],
        nonce_seq: nonce,
        sender: account.address,
        frames: vec![
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: Some(account.address),
                gas_limit: VERIFY_FRAME_GAS,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::copy_from_slice(&verify_data),
            },
            Frame {
                mode: PROOF_FRAME_MODE,
                flags: 0,
                target: None,
                gas_limit: LEANSPHINCS_VERIFICATION_GAS,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::copy_from_slice(&triple),
            },
            Frame {
                mode: u8::from(FrameMode::Sender),
                flags: 0,
                target: Some(recipient),
                gas_limit: SENDER_FRAME_GAS,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ],
        signatures: Vec::new(),
        max_priority_fee_per_gas,
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    };
    tx.max_fee = max_fee_per_gas * U256::from(tx.max_gas());
    tx
}

/// The form a dependency transaction is submitted in: the type byte followed by
/// `rlp([tx_payload_body, witnesses])`, with the leanSPHINCS signature of the
/// transaction's signature hash as its only witness.
fn signed_submission(tx: FrameTransaction, secret: &[u8; 32]) -> Vec<u8> {
    let signature = lean_sphincs::sign(secret, tx.compute_sig_hash().as_fixed_bytes());
    let wrapper = ProofWrapper::new(
        tx,
        None,
        vec![Bytes::copy_from_slice(&signature.to_bytes())],
    );
    let mut submission = Vec::new();
    wrapper.encode_submission(&mut submission);
    submission
}

struct RpcClient {
    http: reqwest::Client,
    url: String,
}

impl RpcClient {
    async fn call(&self, method: &str, params: Value) -> eyre::Result<Value> {
        let response: Value = self
            .http
            .post(&self.url)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .send()
            .await
            .wrap_err_with(|| format!("{method}: request failed"))?
            .json()
            .await
            .wrap_err_with(|| format!("{method}: response is not JSON"))?;
        if let Some(error) = response.get("error") {
            return Err(eyre!("{method}: {error}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_eyre(format!("{method}: response has no result"))
    }

    async fn quantity(&self, method: &str, params: Value) -> eyre::Result<u64> {
        let result = self.call(method, params).await?;
        let hex = result
            .as_str()
            .ok_or_eyre(format!("{method}: result is not a string"))?;
        u64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .wrap_err_with(|| format!("{method}: result {hex} is not a quantity"))
    }

    async fn nonce(&self, address: Address) -> eyre::Result<u64> {
        self.quantity(
            "eth_getTransactionCount",
            json!([format!("{address:#x}"), "latest"]),
        )
        .await
    }
}

fn print_alloc(accounts: &[Account], balance_eth: u64) -> eyre::Result<()> {
    let balance = U256::from(balance_eth) * U256::exp10(18);
    let alloc: serde_json::Map<String, Value> = accounts
        .iter()
        .map(|account| {
            (
                format!("{:#x}", account.address),
                json!({
                    "balance": format!("{balance:#x}"),
                    "code": format!("0x{}", hex::encode(account_code(account.public))),
                    "nonce": "0x0",
                    "storage": {},
                }),
            )
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&alloc)?);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send(
    mut accounts: Vec<Account>,
    client: RpcClient,
    rate: f64,
    count: Option<u64>,
    recipient: Address,
    max_fee_per_gas: U256,
    max_priority_fee_per_gas: U256,
    resend_after: Duration,
) -> eyre::Result<()> {
    if accounts.is_empty() {
        return Err(eyre!("no accounts to send from"));
    }
    if !(rate.is_finite() && rate > 0.0) {
        return Err(eyre!("the rate must be a positive number"));
    }
    let chain_id = client.quantity("eth_chainId", json!([])).await?;
    println!(
        "Sending from {} accounts on chain {chain_id} at {rate} tx/s",
        accounts.len()
    );

    let started = Instant::now();
    let mut ticks = tokio::time::interval(Duration::from_secs_f64(1.0 / rate));
    let mut next = 0;
    let (mut sent, mut resent, mut included, mut failed) = (0u64, 0u64, 0u64, 0u64);
    while count.is_none_or(|count| sent + resent + failed < count) {
        ticks.tick().await;

        // The next account, in turn, that has no transaction waiting for a block.
        let mut ready = None;
        for offset in 0..accounts.len() {
            let index = (next + offset) % accounts.len();
            let account = &mut accounts[index];
            let nonce = client.nonce(account.address).await?;
            match account.pending {
                Some((pending_nonce, _)) if nonce > pending_nonce => {
                    included += 1;
                    account.pending = None;
                }
                // Not included for too long: it may have left the pool. Sending
                // it again is harmless if it is still there.
                Some((_, sent_at)) if sent_at.elapsed() < resend_after => continue,
                _ => {}
            }
            ready = Some((index, nonce));
            break;
        }
        let Some((index, nonce)) = ready else {
            continue;
        };
        next = index + 1;

        let account = &accounts[index];
        // The same nonce as the pending transaction: this is that transaction
        // again, not a new one.
        let is_resend = account
            .pending
            .is_some_and(|(pending_nonce, _)| pending_nonce == nonce);
        let tx = transaction(
            chain_id,
            account,
            nonce,
            recipient,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        );
        let tx_hash = ProofWrapper::new(tx.clone(), None, Vec::new()).tx_hash(&NativeCrypto);
        // Signing takes tens of milliseconds of CPU: keep it off the runtime.
        let secret = account.secret;
        let submission =
            tokio::task::spawn_blocking(move || signed_submission(tx, &secret)).await?;
        let raw = format!("0x{}", hex::encode(&submission));
        match client.call("eth_sendRawTransaction", json!([raw])).await {
            Ok(_) => {
                if is_resend {
                    resent += 1;
                } else {
                    sent += 1;
                }
                accounts[index].pending = Some((nonce, Instant::now()));
                println!(
                    "{} {tx_hash:#x} from {:#x} nonce {nonce} ({} bytes)",
                    if is_resend { "sent again" } else { "sent" },
                    accounts[index].address,
                    submission.len()
                );
            }
            Err(error) => {
                failed += 1;
                eprintln!(
                    "{tx_hash:#x} from {:#x} nonce {nonce} was rejected: {error}",
                    accounts[index].address
                );
            }
        }
    }
    println!(
        "Done in {:.1}s: {sent} sent, {resent} sent again, {included} seen included, {failed} rejected",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    match Cli::parse().command {
        Command::Alloc {
            accounts,
            balance_eth,
        } => print_alloc(&accounts.derive(), balance_eth),
        Command::Send {
            accounts,
            rpc_url,
            rate,
            count,
            recipient,
            max_fee_per_gas_gwei,
            max_priority_fee_per_gas_gwei,
            resend_after,
        } => {
            let gwei = U256::exp10(9);
            send(
                accounts.derive(),
                RpcClient {
                    http: reqwest::Client::new(),
                    url: rpc_url,
                },
                rate,
                count,
                recipient,
                U256::from(max_fee_per_gas_gwei) * gwei,
                U256::from(max_priority_fee_per_gas_gwei) * gwei,
                Duration::from_secs(resend_after),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::Transaction;
    use ethrex_rlp::encode::RLPEncode;

    fn test_account() -> Account {
        AccountsArgs {
            accounts: 1,
            seed: "test".to_string(),
        }
        .derive()
        .remove(0)
    }

    #[test]
    fn accounts_are_derived_from_the_seed() {
        let args = |seed: &str| AccountsArgs {
            accounts: 2,
            seed: seed.to_string(),
        };
        let accounts = args("a").derive();
        let again = args("a").derive();
        let other = args("b").derive();
        assert_eq!(accounts[0].address, again[0].address);
        assert_ne!(accounts[0].address, accounts[1].address);
        assert_ne!(accounts[0].address, other[0].address);
        // The validation code embeds the account's own key.
        let code = account_code(accounts[0].public);
        assert!(
            code.windows(32)
                .any(|window| window == accounts[0].public.as_bytes())
        );
    }

    #[test]
    fn submission_carries_the_transaction_and_a_signature_of_its_signature_hash() {
        let account = test_account();
        let tx = transaction(
            9,
            &account,
            3,
            Address::repeat_byte(0xDE),
            U256::exp10(10),
            U256::exp10(9),
        );
        tx.validate_static_constraints()
            .expect("the transaction satisfies the static constraints");
        // The fee budget covers every unit of gas the transaction may use.
        assert_eq!(tx.max_fee, U256::exp10(10) * U256::from(tx.max_gas()));

        let submission = signed_submission(tx.clone(), &account.secret);
        assert_eq!(submission[0], 0x06);
        assert!(ProofWrapper::is_submission(&submission[1..]).unwrap());
        let wrapper = ProofWrapper::decode_submission(&submission[1..]).unwrap();
        assert_eq!(wrapper.tx.encode_to_vec(), tx.encode_to_vec());
        assert!(wrapper.states_its_dependencies());

        // One dependency: the transaction's signature hash under the account's
        // key, satisfied by the witness.
        let dependencies = wrapper.tx.dependencies();
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].data_hash, tx.compute_sig_hash());
        assert_eq!(dependencies[0].verification_key_hash, account.public);
        assert!(ethrex_crypto::leansphincs::leansphincs_verify(
            account.public.as_fixed_bytes(),
            dependencies[0].data_hash.as_fixed_bytes(),
            &wrapper.witnesses[0],
        ));
        assert_eq!(
            wrapper.tx_hash(&NativeCrypto),
            Transaction::FrameTransaction(tx).hash(&NativeCrypto)
        );
    }
}
