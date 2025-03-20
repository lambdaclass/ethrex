use std::collections::HashMap;

use ethrex_common::{
    types::{AccountInfo, BlockHeader, ChainConfig, Fork, Transaction, TxKind, INITIAL_BASE_FEE},
    H160, H256, U256,
};
use ethrex_storage::{error::StoreError, AccountUpdate};
use pevm::EvmAccount;
use revm_pevm::primitives::{
    AccessListItem, Address, BlobExcessGasAndPrice, BlockEnv, FixedBytes, SignedAuthorization,
    SpecId, TxEnv, TxKind as RevmTxKind, B256, U256 as RevmU256,
};

use crate::db::StoreWrapper;

/// Returns the spec id according to the block timestamp and the stored chain config
/// WARNING: Assumes at least Merge fork is active
pub fn spec_id(chain_config: &ChainConfig, block_timestamp: u64) -> SpecId {
    fork_to_spec_id(chain_config.get_fork(block_timestamp))
}

pub fn fork_to_spec_id(fork: Fork) -> SpecId {
    match fork {
        Fork::Frontier => SpecId::FRONTIER,
        Fork::FrontierThawing => SpecId::FRONTIER_THAWING,
        Fork::Homestead => SpecId::HOMESTEAD,
        Fork::DaoFork => SpecId::DAO_FORK,
        Fork::Tangerine => SpecId::TANGERINE,
        Fork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        Fork::Byzantium => SpecId::BYZANTIUM,
        Fork::Constantinople => SpecId::CONSTANTINOPLE,
        Fork::Petersburg => SpecId::PETERSBURG,
        Fork::Istanbul => SpecId::ISTANBUL,
        Fork::MuirGlacier => SpecId::MUIR_GLACIER,
        Fork::Berlin => SpecId::BERLIN,
        Fork::London => SpecId::LONDON,
        Fork::ArrowGlacier => SpecId::ARROW_GLACIER,
        Fork::GrayGlacier => SpecId::GRAY_GLACIER,
        Fork::Paris => SpecId::MERGE,
        Fork::Shanghai => SpecId::SHANGHAI,
        Fork::Cancun => SpecId::CANCUN,
        Fork::Prague => SpecId::PRAGUE,
        Fork::Osaka => SpecId::OSAKA,
    }
}

pub fn block_env(header: &BlockHeader, spec_id: SpecId) -> BlockEnv {
    BlockEnv {
        number: RevmU256::from(header.number),
        coinbase: Address(header.coinbase.0.into()),
        timestamp: RevmU256::from(header.timestamp),
        gas_limit: RevmU256::from(header.gas_limit),
        basefee: RevmU256::from(header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE)),
        difficulty: RevmU256::from_limbs(header.difficulty.0),
        prevrandao: Some(header.prev_randao.as_fixed_bytes().into()),
        blob_excess_gas_and_price: Some(BlobExcessGasAndPrice::new(
            header.excess_blob_gas.unwrap_or_default(),
            spec_id >= SpecId::PRAGUE,
        )),
    }
}

// Used for the L2
pub const DEPOSIT_MAGIC_DATA: &[u8] = b"mint";
pub fn tx_env(tx: &Transaction, sender: H160) -> TxEnv {
    let max_fee_per_blob_gas = tx
        .max_fee_per_blob_gas()
        .map(|x| RevmU256::from_be_bytes(x.to_big_endian()));
    TxEnv {
        caller: match tx {
            Transaction::PrivilegedL2Transaction(_tx) => Address::ZERO,
            _ => Address(sender.0.into()),
        },
        gas_limit: tx.gas_limit(),
        gas_price: RevmU256::from(tx.gas_price()),
        transact_to: match tx.to() {
            TxKind::Call(address) => RevmTxKind::Call(address.0.into()),
            TxKind::Create => RevmTxKind::Create,
        },
        value: RevmU256::from_limbs(tx.value().0),
        data: match tx {
            Transaction::PrivilegedL2Transaction(_tx) => DEPOSIT_MAGIC_DATA.into(),
            _ => tx.data().clone().into(),
        },
        nonce: Some(tx.nonce()),
        chain_id: tx.chain_id(),
        access_list: tx
            .access_list()
            .into_iter()
            .map(|(addr, list)| {
                let (address, storage_keys) = (
                    Address(addr.0.into()),
                    list.into_iter()
                        .map(|a| FixedBytes::from_slice(a.as_bytes()))
                        .collect(),
                );
                AccessListItem {
                    address,
                    storage_keys,
                }
            })
            .collect(),
        gas_priority_fee: tx.max_priority_fee().map(RevmU256::from),
        blob_hashes: tx
            .blob_versioned_hashes()
            .into_iter()
            .map(|hash| B256::from(hash.0))
            .collect(),
        max_fee_per_blob_gas,
        // EIP7702
        // https://eips.ethereum.org/EIPS/eip-7702
        // The latest version of revm(19.3.0) is needed to run with the latest changes.
        // NOTE:
        // - rust 1.82.X is needed
        // - rust-toolchain 1.82.X is needed (this can be found in ethrex/crates/vm/levm/rust-toolchain.toml)
        authorization_list: tx.authorization_list().map(|list| {
            list.into_iter()
                .map(|auth_t| {
                    SignedAuthorization::new_unchecked(
                        revm_primitives::Authorization {
                            chain_id: RevmU256::from_limbs(auth_t.chain_id.0),
                            address: Address(auth_t.address.0.into()),
                            nonce: auth_t.nonce,
                        },
                        auth_t.y_parity.as_u32() as u8,
                        RevmU256::from_le_bytes(auth_t.r_signature.to_little_endian()),
                        RevmU256::from_le_bytes(auth_t.s_signature.to_little_endian()),
                    )
                })
                .collect::<Vec<SignedAuthorization>>()
                .into()
        }),
    }
}

pub fn map_account_update_to_ethrex_type(
    address: Address,
    account: Option<EvmAccount>,
) -> AccountUpdate {
    let address = H160::from(address.0 .0);
    let Some(account) = account else {
        return AccountUpdate::removed(address);
    };

    let code = if let Some(code) = account.code {
        let bytecode: Result<revm_pevm::primitives::Bytecode, _> = code.try_into();
        if let Ok(bytecode) = bytecode {
            Some(bytecode.bytes().0)
        } else {
            None
        }
    } else {
        None
    };

    let mut storage_changes: HashMap<H256, U256> = HashMap::new();

    for (k, v) in account.storage.iter() {
        let bytes: [u8; 32] = k.to_be_bytes();
        storage_changes.insert(H256::from(bytes), U256(v.as_limbs().clone()));
    }

    return AccountUpdate {
        address,
        info: Some(AccountInfo {
            balance: U256(account.balance.as_limbs().clone()),
            nonce: account.nonce,
            code_hash: H256::from(account.code_hash.unwrap_or_default().0),
        }),
        code,
        removed: false,
        added_storage: storage_changes,
    };
}

impl pevm::Storage for StoreWrapper {
    type Error = StoreError;
}
