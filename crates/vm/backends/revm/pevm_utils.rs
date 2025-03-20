use std::collections::HashMap;

use ethrex_common::{
    types::{
        AccountInfo, AccountState, BlockHeader, ChainConfig, Fork, Transaction, TxKind,
        INITIAL_BASE_FEE,
    },
    H160, H256, U256,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_storage::{error::StoreError, hash_address, AccountUpdate};
use pevm::EvmAccount;
use revm_pevm::primitives::{
    AccessListItem, Address, BlobExcessGasAndPrice, BlockEnv, Bytecode, Bytes as RevmBytes,
    FixedBytes, SignedAuthorization, SpecId, TxEnv, TxKind as RevmTxKind, B256, U256 as RevmU256,
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
        let bytecode: Result<Bytecode, _> = code.try_into();
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

    fn basic(&self, address: &Address) -> Result<Option<pevm::AccountBasic>, Self::Error> {
        let acc_info = match self
            .store
            .get_account_info_by_hash(self.block_hash, H160::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };

        Ok(Some(pevm::AccountBasic {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
        }))
    }

    fn code_hash(&self, address: &Address) -> Result<Option<B256>, Self::Error> {
        let acc_info = match self
            .store
            .get_account_info_by_hash(self.block_hash, H160::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };

        let code = self
            .store
            .get_account_code(acc_info.code_hash)?
            .map(|b| Bytecode::new_raw(RevmBytes(b)));

        match code {
            Some(code) => Ok(Some(B256::from_slice(code.bytes_slice()))),
            None => Ok(None),
        }
    }

    fn code_by_hash(&self, code_hash: &B256) -> Result<Option<pevm::EvmCode>, Self::Error> {
        let code = self
            .store
            .get_account_code(H256::from(code_hash.as_ref()))?
            .map(|b| Bytecode::new_raw(RevmBytes(b)));

        match code {
            Some(code) => {
                let evm_code: pevm::EvmCode =
                    code.try_into().map_err(|_| StoreError::DecodeError)?;
                Ok(Some(evm_code))
            }
            None => Ok(None),
        }
    }

    fn has_storage(&self, address: &Address) -> Result<bool, Self::Error> {
        let trie = self.store.open_state_trie(self.block_hash);
        let account = match trie
            .get(&hash_address(&H160::from(address.0.as_ref())))
            .unwrap()
        {
            Some(encoded) => AccountState::decode(&encoded)?,
            None => AccountState::default(),
        };

        Ok(!account.storage_root.is_zero())
    }

    fn storage(&self, address: &Address, index: &RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(self
            .store
            .get_storage_at_hash(
                self.block_hash,
                H160::from(address.0.as_ref()),
                H256::from(index.to_be_bytes()),
            )?
            .map(|value| RevmU256::from_limbs(value.0))
            .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash(&self, number: &u64) -> Result<B256, Self::Error> {
        self.store
            .get_block_header(*number)?
            .map(|header| B256::from_slice(&header.compute_block_hash().0))
            .ok_or_else(|| StoreError::Custom(format!("Block {number} not found")))
    }
}
