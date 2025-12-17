use bytes::Bytes;
use ethereum_types::Address;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

use crate::rkyv_utils::{H160Wrapper, OptionH160Wrapper};

#[derive(
    Serialize, Deserialize, RDeserialize, RSerialize, Archive, Clone, Copy, Debug, Default,
)]
pub struct FeeConfig {
    /// If set, the base fee is sent to this address instead of being burned.
    #[rkyv(with=OptionH160Wrapper)]
    pub base_fee_vault: Option<Address>,
    pub operator_fee_config: Option<OperatorFeeConfig>,
    pub l1_fee_config: Option<L1FeeConfig>,
}

/// Configuration for operator fees on L2
/// The operator fee is an additional fee on top of the base fee
/// that is sent to the operator fee vault.
/// This is used to pay for the cost of running the L2 network.
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive, Clone, Copy, Debug)]
pub struct OperatorFeeConfig {
    #[rkyv(with=H160Wrapper)]
    pub operator_fee_vault: Address,
    pub operator_fee_per_gas: u64,
}

/// L1 Fee is used to pay for the cost of
/// posting data to L1 (e.g. blob data).
#[derive(Serialize, Deserialize, RDeserialize, RSerialize, Archive, Clone, Copy, Debug)]
pub struct L1FeeConfig {
    #[rkyv(with=H160Wrapper)]
    pub l1_fee_vault: Address,
    pub l1_fee_per_blob_gas: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum FeeConfigError {
    #[error("Encoding error: {0}")]
    EncodingError(String),
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("Invalid fee config type: {0}")]
    InvalidFeeConfigType(u8),
    #[error("DecoderError error: {0}")]
    DecoderError(#[from] DecoderError),
}

#[derive(Debug, Clone, Copy)]
pub enum FeeConfigType {
    BaseFeeVault = 1,
    OperatorFee = 2,
    L1Fee = 4,
}

impl TryFrom<u8> for FeeConfigType {
    type Error = FeeConfigError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(FeeConfigType::BaseFeeVault),
            2 => Ok(FeeConfigType::OperatorFee),
            4 => Ok(FeeConfigType::L1Fee),
            _ => Err(FeeConfigError::InvalidFeeConfigType(value)),
        }
    }
}

impl From<FeeConfigType> for u8 {
    fn from(value: FeeConfigType) -> Self {
        match value {
            FeeConfigType::BaseFeeVault => 1,
            FeeConfigType::OperatorFee => 2,
            FeeConfigType::L1Fee => 4,
        }
    }
}

impl FeeConfigType {
    // Checks if the type is present in the given value
    pub fn is_in(&self, value: u8) -> bool {
        value & u8::from(*self) == u8::from(*self)
    }
}

impl FeeConfig {
    pub fn to_vec(&self) -> Vec<u8> {
        let version = 0u8;
        let mut encoded: Vec<u8> = Vec::new();

        let mut fee_config_type = 0;

        if let Some(base_fee_vault) = self.base_fee_vault {
            // base fee vault is set
            let base_fee_vault_type: u8 = FeeConfigType::BaseFeeVault.into();
            fee_config_type += base_fee_vault_type;
            encoded.extend_from_slice(&base_fee_vault.0);
        }

        if let Some(operator_fee_config) = self.operator_fee_config {
            // operator fee vault is set
            let base_fee_vault_type: u8 = FeeConfigType::OperatorFee.into();
            fee_config_type += base_fee_vault_type;
            encoded.extend_from_slice(&operator_fee_config.operator_fee_vault.0);
            encoded.extend(operator_fee_config.operator_fee_per_gas.to_be_bytes());
        }

        if let Some(l1_fee_config) = self.l1_fee_config {
            // l1 fee vault is set
            let l1_fee_type: u8 = FeeConfigType::L1Fee.into();
            fee_config_type += l1_fee_type;
            encoded.extend_from_slice(&l1_fee_config.l1_fee_vault.0);
            encoded.extend(l1_fee_config.l1_fee_per_blob_gas.to_be_bytes());
        }

        let mut result = Vec::with_capacity(1 + 1 + encoded.len());
        result.extend(version.to_be_bytes());
        result.extend(fee_config_type.to_be_bytes());
        result.extend(encoded);

        result
    }

    pub fn decode(bytes: &[u8]) -> Result<(usize, Self), FeeConfigError> {
        let mut decoder = Decoder::new(bytes);

        // Read version
        let version = decoder.get_u8()?;
        if version != 0 {
            return Err(FeeConfigError::UnsupportedVersion(version));
        }

        // Read fee config type
        let fee_config_type = decoder.get_u8()?;

        // Read base fee vault if present
        let base_fee_vault = if FeeConfigType::BaseFeeVault.is_in(fee_config_type) {
            let address = decoder.get_address()?;
            Some(address)
        } else {
            None
        };

        // Read operator fee config if present
        let operator_fee_config = if FeeConfigType::OperatorFee.is_in(fee_config_type) {
            let operator_fee_vault = decoder.get_address()?;
            let operator_fee_per_gas = decoder.get_u64()?;
            Some(OperatorFeeConfig {
                operator_fee_vault,
                operator_fee_per_gas,
            })
        } else {
            None
        };

        // Read L1 fee config if present
        let l1_fee_config = if FeeConfigType::L1Fee.is_in(fee_config_type) {
            let l1_fee_vault = decoder.get_address()?;
            let l1_fee_per_blob_gas = decoder.get_u64()?;
            Some(L1FeeConfig {
                l1_fee_vault,
                l1_fee_per_blob_gas,
            })
        } else {
            None
        };

        Ok((
            decoder.consumed(),
            FeeConfig {
                base_fee_vault,
                operator_fee_config,
                l1_fee_config,
            },
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecoderError {
    #[error("Decoder failed to deserialize: {0}")]
    FailedToDeserialize(String),
    #[error("StateDiff failed to deserialize: {0}")]
    FailedToDeserializeStateDiff(String),
}

pub struct Decoder {
    bytes: Bytes,
    offset: usize,
}

impl Decoder {
    pub fn new(bytes: &[u8]) -> Self {
        Decoder {
            bytes: Bytes::copy_from_slice(bytes),
            offset: 0,
        }
    }

    pub fn consumed(&self) -> usize {
        self.offset
    }

    pub fn advance(&mut self, size: usize) {
        self.offset += size;
    }

    pub fn get_address(&mut self) -> Result<Address, DecoderError> {
        let res = Address::from_slice(self.bytes.get(self.offset..self.offset + 20).ok_or(
            DecoderError::FailedToDeserializeStateDiff("Not enough bytes".to_string()),
        )?);
        self.offset += 20;

        Ok(res)
    }

    pub fn get_u8(&mut self) -> Result<u8, DecoderError> {
        let res = self
            .bytes
            .get(self.offset)
            .ok_or(DecoderError::FailedToDeserializeStateDiff(
                "Not enough bytes".to_string(),
            ))?;
        self.offset += 1;

        Ok(*res)
    }

    pub fn get_u64(&mut self) -> Result<u64, DecoderError> {
        let res = u64::from_be_bytes(
            self.bytes
                .get(self.offset..self.offset + 8)
                .ok_or(DecoderError::FailedToDeserializeStateDiff(
                    "Not enough bytes".to_string(),
                ))?
                .try_into()
                .map_err(|_| {
                    DecoderError::FailedToDeserializeStateDiff("Cannot parse u64".to_string())
                })?,
        );
        self.offset += 8;

        Ok(res)
    }
}
