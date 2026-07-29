//! Conversions from the canonical stateless input to the ethrex structures.

use core::cmp::Ordering;

use ethrex_common::{
    Bytes, H160,
    types::{
        ForkBlobSchedule,
        block_execution_witness::{
            self, RpcExecutionWitness, decode_witness_headers, validate_witness_headers_chain,
        },
        eip8025_ssz,
    },
};
use ethrex_crypto::Crypto;
use ethrex_guest_program::l1::{
    CanonicalChainConfig, CanonicalExecutionWitness, CanonicalForkActivation, CanonicalForkConfig,
    CanonicalStatelessInput, DecodedEip8025,
};
use hex_literal::hex;
use stateless_validator_common::{
    SszList, SszVector,
    guest::{
        Error as CommonError,
        input::{
            ChainConfig, ExecutionWitness, ProtocolFork, StatelessInput,
            new_payload_request::{
                ExecutionRequestsGloas, MAX_WITHDRAWALS_PER_PAYLOAD, NewPayloadRequest,
                NewPayloadRequestElectraFulu, NewPayloadRequestGloas, Withdrawals,
            },
        },
    },
};

use crate::error::Error;

/// Mainnet beacon chain deposit contract address.
const MAINNET_DEPOSIT_CONTRACT_ADDRESS: [u8; 20] = hex!("00000000219ab540356cbb839cbe05303d7705fa");

/// Converts the decoded canonical stateless input into the ethrex program
/// input consumed by `execute_decoded`.
pub(crate) fn to_ethrex_input(
    fork: ProtocolFork,
    input: StatelessInput,
    crypto: &dyn Crypto,
) -> Result<DecodedEip8025, Error> {
    let chain_config = to_ethrex_chain_config(fork, &input.chain_config)?;
    Ok(match input.new_payload_request {
        NewPayloadRequest::Gloas(request) => DecodedEip8025::Canonical {
            stateless_input: CanonicalStatelessInput {
                new_payload_request: to_ethrex_new_payload_request(request),
                witness: to_ethrex_witness(input.witness),
                chain_config: to_ethrex_canonical_chain_config(&input.chain_config),
                public_keys: map_ssz_list(input.public_keys, array_to_ssz_vec),
            },
            chain_config,
        },
        NewPayloadRequest::ElectraFulu(request) => {
            let first_block_number = request.execution_payload.block_number;
            DecodedEip8025::Legacy {
                new_payload_request: to_ethrex_legacy_new_payload_request(request),
                execution_witness: to_ethrex_legacy_witness(
                    input.witness,
                    chain_config,
                    first_block_number,
                    crypto,
                )?,
            }
        }
        _ => return Err(Error::UnsupportedPayload),
    })
}

/// Converts the new payload request into the ethrex container.
fn to_ethrex_new_payload_request(
    request: NewPayloadRequestGloas,
) -> eip8025_ssz::NewPayloadRequestAmsterdam {
    let payload = request.execution_payload;
    let execution_payload = eip8025_ssz::ExecutionPayloadV4 {
        parent_hash: payload.parent_hash,
        fee_recipient: payload.fee_recipient.into(),
        state_root: payload.state_root,
        receipts_root: payload.receipts_root,
        logs_bloom: array_to_ssz_vec(payload.logs_bloom),
        prev_randao: payload.prev_randao,
        block_number: payload.block_number,
        gas_limit: payload.gas_limit,
        gas_used: payload.gas_used,
        timestamp: payload.timestamp,
        extra_data: payload.extra_data,
        base_fee_per_gas: payload.base_fee_per_gas,
        block_hash: payload.block_hash,
        transactions: payload.transactions,
        withdrawals: to_ethrex_withdrawals(payload.withdrawals),
        blob_gas_used: payload.blob_gas_used,
        excess_blob_gas: payload.excess_blob_gas,
        block_access_list: payload.block_access_list,
        slot_number: payload.slot_number,
    };
    eip8025_ssz::NewPayloadRequestAmsterdam {
        execution_payload,
        versioned_hashes: request.versioned_hashes,
        parent_beacon_block_root: request.parent_beacon_block_root,
        execution_requests: to_ethrex_execution_requests(request.execution_requests),
    }
}

/// Converts canonical withdrawals into the ethrex list.
fn to_ethrex_withdrawals(
    withdrawals: Withdrawals,
) -> SszList<eip8025_ssz::Withdrawal, MAX_WITHDRAWALS_PER_PAYLOAD> {
    map_ssz_list(withdrawals, |withdrawal| eip8025_ssz::Withdrawal {
        index: withdrawal.index,
        validator_index: withdrawal.validator_index,
        address: withdrawal.address.into(),
        amount: withdrawal.amount,
    })
}

/// Converts the canonical execution requests into the ethrex container.
fn to_ethrex_execution_requests(
    requests: ExecutionRequestsGloas,
) -> eip8025_ssz::ExecutionRequests {
    eip8025_ssz::ExecutionRequests {
        deposits: map_ssz_list(requests.deposits, |deposit| eip8025_ssz::DepositRequest {
            pubkey: deposit.pubkey,
            withdrawal_credentials: deposit.withdrawal_credentials,
            amount: deposit.amount,
            signature: deposit.signature,
            index: deposit.index,
        }),
        withdrawals: map_ssz_list(requests.withdrawals, |withdrawal| {
            eip8025_ssz::WithdrawalRequest {
                source_address: withdrawal.source_address.into(),
                validator_pubkey: withdrawal.validator_pubkey,
                amount: withdrawal.amount,
            }
        }),
        consolidations: map_ssz_list(requests.consolidations, |consolidation| {
            eip8025_ssz::ConsolidationRequest {
                source_address: consolidation.source_address.into(),
                source_pubkey: consolidation.source_pubkey,
                target_pubkey: consolidation.target_pubkey,
            }
        }),
        builder_deposits: map_ssz_list(requests.builder_deposits, |builder_deposit| {
            eip8025_ssz::BuilderDepositRequest {
                pubkey: builder_deposit.pubkey,
                withdrawal_credentials: builder_deposit.withdrawal_credentials,
                amount: builder_deposit.amount,
                signature: builder_deposit.signature,
            }
        }),
        builder_exits: map_ssz_list(requests.builder_exits, |builder_exit| {
            eip8025_ssz::BuilderExitRequest {
                source_address: builder_exit.source_address.into(),
                pubkey: builder_exit.pubkey,
            }
        }),
    }
}

/// Converts the execution witness into the ethrex container.
fn to_ethrex_witness(witness: ExecutionWitness) -> CanonicalExecutionWitness {
    CanonicalExecutionWitness {
        state: witness.state,
        codes: witness.codes,
        headers: witness.headers,
    }
}

/// Converts the chain configuration into the ethrex canonical chain
/// configuration container.
fn to_ethrex_canonical_chain_config(config: &ChainConfig) -> CanonicalChainConfig {
    CanonicalChainConfig {
        chain_id: config.chain_id,
        active_fork: CanonicalForkConfig {
            activation: CanonicalForkActivation {
                block_number: config.active_fork.activation.block_number.clone(),
                timestamp: config.active_fork.activation.timestamp.clone(),
            },
        },
    }
}

/// Converts a chain configuration into a [`ethrex_common::types::ChainConfig`].
fn to_ethrex_chain_config(
    fork: ProtocolFork,
    config: &ChainConfig,
) -> Result<ethrex_common::types::ChainConfig, Error> {
    let (activation_block_number, activation_timestamp) = if fork >= ProtocolFork::Shanghai {
        let timestamp = config
            .active_fork
            .activation
            .timestamp()
            .ok_or(CommonError::InvalidForkActivation)?;
        (0, timestamp)
    } else {
        let block_number = config
            .active_fork
            .activation
            .block_number()
            .ok_or(CommonError::InvalidForkActivation)?;
        (block_number, 0)
    };
    let block_at = |target| match fork.cmp(&target) {
        Ordering::Greater => Some(0),
        Ordering::Equal => Some(activation_block_number),
        Ordering::Less => None,
    };
    let time_at = |target| match fork.cmp(&target) {
        Ordering::Greater => Some(0),
        Ordering::Equal => Some(activation_timestamp),
        Ordering::Less => None,
    };

    Ok(ethrex_common::types::ChainConfig {
        chain_id: config.chain_id,
        homestead_block: block_at(ProtocolFork::Homestead),
        dao_fork_block: block_at(ProtocolFork::DAOFork),
        dao_fork_support: fork >= ProtocolFork::DAOFork,
        eip150_block: block_at(ProtocolFork::TangerineWhistle),
        eip155_block: block_at(ProtocolFork::SpuriousDragon),
        eip158_block: block_at(ProtocolFork::SpuriousDragon),
        byzantium_block: block_at(ProtocolFork::Byzantium),
        constantinople_block: block_at(ProtocolFork::StPetersburg),
        petersburg_block: block_at(ProtocolFork::StPetersburg),
        istanbul_block: block_at(ProtocolFork::Istanbul),
        muir_glacier_block: block_at(ProtocolFork::MuirGlacier),
        berlin_block: block_at(ProtocolFork::Berlin),
        london_block: block_at(ProtocolFork::London),
        arrow_glacier_block: block_at(ProtocolFork::ArrowGlacier),
        gray_glacier_block: block_at(ProtocolFork::GrayGlacier),
        merge_netsplit_block: block_at(ProtocolFork::Paris),
        shanghai_time: time_at(ProtocolFork::Shanghai),
        cancun_time: time_at(ProtocolFork::Cancun),
        prague_time: time_at(ProtocolFork::Prague),
        verkle_time: None,
        osaka_time: time_at(ProtocolFork::Osaka),
        bpo1_time: time_at(ProtocolFork::BPO1),
        bpo2_time: time_at(ProtocolFork::BPO2),
        bpo3_time: None,
        bpo4_time: None,
        bpo5_time: None,
        amsterdam_time: time_at(ProtocolFork::Amsterdam),
        hegota_time: None,
        terminal_total_difficulty: (fork >= ProtocolFork::Paris).then_some(0),
        terminal_total_difficulty_passed: fork >= ProtocolFork::Paris,
        blob_schedule: active_fork_blob_schedule(fork)?,
        deposit_contract_address: H160(MAINNET_DEPOSIT_CONTRACT_ADDRESS),
        enable_verkle_at_genesis: false,
    })
}

/// Per-fork blob schedule `(target, max, base_fee_update_fraction)`, or `None` before Cancun.
fn blob_schedule(fork: ProtocolFork) -> Option<(u64, u64, u64)> {
    Some(match fork {
        ProtocolFork::Cancun => (3, 6, 3_338_477),
        ProtocolFork::Prague | ProtocolFork::Osaka => (6, 9, 5_007_716),
        ProtocolFork::BPO1 => (10, 15, 8_346_193),
        ProtocolFork::BPO2 | ProtocolFork::Amsterdam => (14, 21, 11_684_671),
        _ => return None,
    })
}

/// Builds an ethrex blob schedule for the active fork.
fn active_fork_blob_schedule(
    fork: ProtocolFork,
) -> Result<ethrex_common::types::BlobSchedule, Error> {
    let mut schedule = ethrex_common::types::BlobSchedule::default();
    let Some((target, max, base_fee_update_fraction)) = blob_schedule(fork) else {
        return Ok(schedule);
    };
    let entry = ForkBlobSchedule {
        base_fee_update_fraction,
        target: u32::try_from(target).map_err(|_| Error::BlobTargetOutOfBounds)?,
        max: u32::try_from(max).map_err(|_| Error::BlobMaxOutOfBounds)?,
    };
    match fork {
        ProtocolFork::Cancun => schedule.cancun = entry,
        ProtocolFork::Prague => schedule.prague = entry,
        ProtocolFork::Osaka => schedule.osaka = entry,
        ProtocolFork::BPO1 => schedule.bpo1 = entry,
        ProtocolFork::BPO2 => schedule.bpo2 = entry,
        ProtocolFork::Amsterdam => schedule.amsterdam = Some(entry),
        _ => unreachable!("forks before Cancun return None above"),
    }
    Ok(schedule)
}

/// Converts the canonical Electra/Fulu payload request into the ethrex
/// legacy container.
fn to_ethrex_legacy_new_payload_request(
    request: NewPayloadRequestElectraFulu,
) -> eip8025_ssz::NewPayloadRequest {
    let payload = request.execution_payload;
    eip8025_ssz::NewPayloadRequest {
        execution_payload: eip8025_ssz::ExecutionPayload {
            parent_hash: payload.parent_hash,
            fee_recipient: payload.fee_recipient.into(),
            state_root: payload.state_root,
            receipts_root: payload.receipts_root,
            logs_bloom: array_to_ssz_vec(payload.logs_bloom),
            prev_randao: payload.prev_randao,
            block_number: payload.block_number,
            gas_limit: payload.gas_limit,
            gas_used: payload.gas_used,
            timestamp: payload.timestamp,
            extra_data: payload.extra_data,
            base_fee_per_gas: payload.base_fee_per_gas,
            block_hash: payload.block_hash,
            transactions: payload.transactions,
            withdrawals: to_ethrex_withdrawals(payload.withdrawals),
            blob_gas_used: payload.blob_gas_used,
            excess_blob_gas: payload.excess_blob_gas,
        },
        versioned_hashes: request.versioned_hashes,
        parent_beacon_block_root: request.parent_beacon_block_root,
        execution_requests: to_ethrex_execution_requests(ExecutionRequestsGloas {
            deposits: request.execution_requests.deposits,
            withdrawals: request.execution_requests.withdrawals,
            consolidations: request.execution_requests.consolidations,
            ..Default::default()
        }),
    }
}

/// Converts the execution witness into the ethrex witness consumed by the
/// legacy execution.
fn to_ethrex_legacy_witness(
    witness: ExecutionWitness,
    chain_config: ethrex_common::types::ChainConfig,
    first_block_number: u64,
    crypto: &dyn Crypto,
) -> Result<block_execution_witness::ExecutionWitness, Error> {
    let rpc_witness = RpcExecutionWitness {
        state: to_bytes_vec(witness.state),
        keys: Vec::new(),
        codes: to_bytes_vec(witness.codes),
        headers: to_bytes_vec(witness.headers),
    };
    let decoded_headers = decode_witness_headers(&rpc_witness.headers)?;
    validate_witness_headers_chain(&decoded_headers, crypto)?;
    Ok(rpc_witness.into_execution_witness(
        chain_config,
        first_block_number,
        &decoded_headers,
        crypto,
    )?)
}

fn array_to_ssz_vec<T: Clone, const N: usize>(array: [T; N]) -> SszVector<T, N> {
    array.to_vec().try_into().expect("infallible")
}

fn map_ssz_list<T, U, const N: usize>(list: SszList<T, N>, f: impl Fn(T) -> U) -> SszList<U, N> {
    list.into_iter()
        .map(f)
        .collect::<Vec<_>>()
        .try_into()
        .expect("infallible")
}

fn to_bytes_vec<const M: usize, const N: usize>(items: SszList<SszList<u8, M>, N>) -> Vec<Bytes> {
    items
        .into_iter()
        .map(|item| Bytes::from(item.into_inner()))
        .collect()
}
