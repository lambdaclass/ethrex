use bytes::Bytes;
use c_kzg::{Blob, BYTES_PER_BLOB};
use ethereum_rust_core::types::{EIP1559Transaction, EIP4844Transaction, PrivilegedL2Transaction};
use ethereum_rust_rlp::structs::Encoder;

pub fn blob_from_bytes(bytes: Bytes) -> Result<Blob, c_kzg::Error> {
    // We set the first byte of every 32-bytes chunk to 0x00
    // so it's always under the field module.
    if bytes.len() > BYTES_PER_BLOB * 31 / 32 {
        return Err(c_kzg::Error::InvalidBytesLength(format!(
            "Bytes too long for a Blob ({})",
            bytes.len()
        )));
    }

    let mut buf = [0u8; BYTES_PER_BLOB];
    buf[..(bytes.len() * 32).div_ceil(31)].copy_from_slice(
        &bytes
            .chunks(31)
            .map(|x| [&[0x00], x].concat())
            .collect::<Vec<_>>()
            .concat(),
    );

    Blob::from_bytes(&buf)
}

pub trait PayloadRLPEncode {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut);
    fn encode_payload_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_payload(&mut buf);
        buf
    }
}

impl PayloadRLPEncode for EIP1559Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce)
            .encode_field(&self.max_priority_fee_per_gas)
            .encode_field(&self.max_fee_per_gas)
            .encode_field(&self.gas_limit)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.access_list)
            .finish();
    }
}

impl PayloadRLPEncode for EIP4844Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce)
            .encode_field(&self.max_priority_fee_per_gas)
            .encode_field(&self.max_fee_per_gas)
            .encode_field(&self.gas)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.access_list)
            .encode_field(&self.max_fee_per_blob_gas)
            .encode_field(&self.blob_versioned_hashes)
            .finish();
    }
}

impl PayloadRLPEncode for PrivilegedL2Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce)
            .encode_field(&self.max_priority_fee_per_gas)
            .encode_field(&self.max_fee_per_gas)
            .encode_field(&self.gas_limit)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.access_list)
            .encode_field(&self.tx_type)
            .finish();
    }
}
